//! Lookups the Installed page makes for every mod, made once per build.
//!
//! The library and the notes store are lists searched from the end (`Library::find`,
//! `Extras::find`), and an update check walks every slot of a GameBanana mod. Asked once per mod
//! from a loop over every mod, that cost grew with the square of the library: at 300 mods the
//! toolbar's update count alone took about 9 ms a build on an M-series Mac. An `Index` answers
//! each in constant time and keeps the update answers it worked out, so the toolbar, the cards
//! and the selection bar share them. It borrows nothing: build one from the state a view reads
//! and drop it with that build.

use std::cell::RefCell;
use std::collections::HashMap;

use deadlock_core::import::ModMeta;
use deadlock_core::mod_extras::ModExtra;
use deadlock_core::mod_updates::RemoteFile;

use crate::app::State;
use crate::model::AddonRow;

pub struct Index {
    /// `(slot, size)` to the library entry `Library::find` returns (the last that matches).
    meta: HashMap<(u16, u64), usize>,
    /// `(slot, size)` to the notes entry `Extras::find` returns (the last that matches).
    extra: HashMap<(u16, u64), usize>,
    /// Slot to the row on disk.
    slot: HashMap<u16, usize>,
    /// GameBanana id to the rows whose library entry names it, in row order.
    gamebanana: HashMap<u64, Vec<usize>>,
    /// `update_for` answers by slot, worked out on first ask.
    updates: RefCell<HashMap<u16, Option<RemoteFile>>>,
}

#[cfg(test)]
thread_local! {
    /// Indexes built on this thread, for tests that hold a quiet poll to building none.
    pub static BUILT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl Index {
    pub fn new(s: &State) -> Self {
        #[cfg(test)]
        BUILT.with(|n| n.set(n.get() + 1));
        let mut meta = HashMap::with_capacity(s.installed.library.mods.len());
        for (i, m) in s.installed.library.mods.iter().enumerate() {
            meta.insert((m.slot, m.size_bytes), i);
        }
        let mut extra = HashMap::with_capacity(s.installed.notes.extras.mods.len());
        for (i, m) in s.installed.notes.extras.mods.iter().enumerate() {
            extra.insert((m.slot, m.size_bytes), i);
        }
        let mut slot = HashMap::with_capacity(s.addons.addons.len());
        let mut gamebanana: HashMap<u64, Vec<usize>> = HashMap::new();
        for (i, addon) in s.addons.addons.iter().enumerate() {
            slot.entry(addon.slot).or_insert(i);
            let id = meta
                .get(&(addon.slot, addon.size_bytes))
                .and_then(|&m| s.installed.library.mods[m].gamebanana_id);
            if let Some(id) = id {
                gamebanana.entry(id).or_default().push(i);
            }
        }
        Index { meta, extra, slot, gamebanana, updates: RefCell::default() }
    }

    /// `meta_for`.
    pub fn meta<'a>(&self, s: &'a State, addon: &AddonRow) -> Option<&'a ModMeta> {
        self.meta.get(&(addon.slot, addon.size_bytes)).map(|&i| &s.installed.library.mods[i])
    }

    /// The stored notes entry for a row, without copying it; `None` where `notes::extra`
    /// would make a blank one.
    pub fn extra_ref<'a>(&self, s: &'a State, addon: &AddonRow) -> Option<&'a ModExtra> {
        self.extra.get(&(addon.slot, addon.size_bytes)).map(|&i| &s.installed.notes.extras.mods[i])
    }

    /// `notes::extra`: the entry, or a blank one carrying the slot, size and GameBanana id.
    pub fn extra(&self, s: &State, addon: &AddonRow) -> ModExtra {
        self.extra_ref(s, addon).cloned().unwrap_or_else(|| ModExtra {
            slot: addon.slot,
            size_bytes: addon.size_bytes,
            gamebanana_id: self.meta(s, addon).and_then(|m| m.gamebanana_id),
            ..ModExtra::default()
        })
    }

    /// The row on disk in `slot`.
    pub fn addon<'a>(&self, s: &'a State, slot: u16) -> Option<&'a AddonRow> {
        self.slot.get(&slot).map(|&i| &s.addons.addons[i])
    }

    /// `variant_slots`: the rows that hold GameBanana mod `id`, in row order.
    pub fn variant_slots<'a>(&self, s: &'a State, id: u64) -> Vec<&'a AddonRow> {
        self.gamebanana
            .get(&id)
            .map(|rows| rows.iter().map(|&i| &s.addons.addons[i]).collect())
            .unwrap_or_default()
    }

    /// A kept `update_for` answer, or `work` run once and kept.
    pub fn update(&self, slot: u16, work: impl FnOnce() -> Option<RemoteFile>) -> Option<RemoteFile> {
        if let Some(known) = self.updates.borrow().get(&slot) {
            return known.clone();
        }
        let answer = work();
        self.updates.borrow_mut().insert(slot, answer.clone());
        answer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Installed's poll runs on every animating frame. With nothing changed it must not walk
    /// the library again: no index, no order check, no preview list.
    #[test]
    fn a_quiet_poll_rebuilds_nothing() {
        let mut s = crate::data::init_state();
        crate::bench_scenes::installed_stress_of(&mut s, 120);
        super::super::poll(&mut s);
        super::super::poll(&mut s);
        let before = BUILT.with(|n| n.get());
        for _ in 0..5 {
            super::super::poll(&mut s);
        }
        assert_eq!(BUILT.with(|n| n.get()), before, "a poll with nothing changed built an index");
        // A change to the rows is picked up on the next poll.
        s.addons.addons[0].enabled = !s.addons.addons[0].enabled;
        super::super::poll(&mut s);
        assert!(BUILT.with(|n| n.get()) > before);
    }

    /// The index answers exactly what the linear lookups it replaces answer, for every row.
    #[test]
    fn answers_as_the_linear_lookups_do() {
        let mut s = crate::data::init_state();
        crate::bench_scenes::installed_stress(&mut s);
        // A second library entry for one slot at the same size: the last one wins, as in
        // `Library::find`.
        if let Some(first) = s.installed.library.mods.first().cloned() {
            let mut again = first.clone();
            again.name = "Later entry".into();
            s.installed.library.mods.push(again);
        }
        let index = Index::new(&s);
        for addon in &s.addons.addons {
            assert_eq!(
                index.meta(&s, addon).map(|m| &m.name),
                super::super::meta_for(&s, addon).map(|m| &m.name),
                "meta for slot {}",
                addon.slot
            );
            assert_eq!(index.extra(&s, addon), super::super::notes::extra(&s, addon), "extra for slot {}", addon.slot);
            assert_eq!(
                super::super::update_for_in(&s, &index, addon),
                super::super::update_for_linear(&s, addon),
                "update for slot {}",
                addon.slot
            );
        }
        assert!(s.addons.addons.iter().any(|addon| index.update(addon.slot, || None).is_some()));
    }
}
