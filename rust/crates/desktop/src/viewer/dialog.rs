//! Preview in 3D for one mod: the dialog Installed (card menu, file picker), a mod's details
//! and the Locker's skin rows open. It shows the hero with that mod's slot included, switched
//! on or not, against the hero as it is without it; a mod that changes several heroes gets a
//! hero picker in the header.
//!
//! Which heroes a mod changes comes from `deadlock_core::viewer::heroes_in_vpk` (the VPK's
//! directory tree, read only), read for every installed slot on a worker after each addon scan
//! and kept by slot, file and size, so a menu knows at once whether to offer the preview.

use std::collections::HashMap;
use std::path::PathBuf;

use rune::{El, Key, KeyEvent, col, el, row, spacer, text};

use super::prepare::{self, Ask, DIALOG_KEY, Owner};
use crate::app::State;
use crate::icons;
use crate::jobs::Job;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{ActionKind, H2, SMALL, Ui};

/// The open dialog.
pub struct Dialog {
    pub slot: u16,
    pub name: String,
    /// Display names of the heroes the mod changes, and the one shown.
    pub heroes: Vec<String>,
    pub hero: usize,
}

impl Dialog {
    pub fn key(&self) -> &'static str {
        DIALOG_KEY
    }
}

type Found = Vec<(u16, String, u64, Vec<String>)>;

/// Heroes per installed slot.
#[derive(Default)]
pub struct ModHeroes {
    map: HashMap<u16, (String, u64, Vec<String>)>,
    sig: Vec<(u16, String, u64)>,
    job: Job<Found>,
    /// Set by demo scenes: heroes per slot without reading a VPK.
    pub fixed: HashMap<u16, Vec<String>>,
}

/// Display names of the heroes the mod in `slot` changes, as far as known.
pub fn heroes_of(s: &State, slot: u16) -> Vec<String> {
    if let Some(fixed) = s.viewers.mod_heroes.fixed.get(&slot) {
        return fixed.clone();
    }
    if let Some(addon) = s.addons.addons.iter().find(|a| a.slot == slot)
        && let Some((file, size, heroes)) = s.viewers.mod_heroes.map.get(&slot)
        && *file == addon.file
        && *size == addon.size_bytes
    {
        return heroes.clone();
    }
    if s.demo {
        // Demo addons are not on disk: the Locker's fixture knows their heroes.
        if let Some(m) = s.locker.mods.iter().find(|m| m.slot == slot) {
            return m.heroes.clone();
        }
    }
    Vec::new()
}

/// Reads the heroes of slots not seen yet, after an addon scan. True when a view must run.
pub fn poll(s: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = s.viewers.mod_heroes.job.try_take() {
        if let Ok(found) = result {
            for (slot, file, size, heroes) in found {
                s.viewers.mod_heroes.map.insert(slot, (file, size, heroes));
            }
        }
        changed = true;
    }
    if s.viewers.mod_heroes.job.is_running() || prepare::stand_in(s) || rune::is_headless() {
        return changed;
    }
    let Some(dir) = s.addons_dir.clone().map(PathBuf::from) else { return changed };
    let sig: Vec<(u16, String, u64)> = s.addons.addons.iter().map(|a| (a.slot, a.file.clone(), a.size_bytes)).collect();
    if sig == s.viewers.mod_heroes.sig {
        return changed;
    }
    let known = &s.viewers.mod_heroes.map;
    let todo: Vec<(u16, String, u64, PathBuf)> = s
        .addons
        .addons
        .iter()
        .filter(|a| known.get(&a.slot).is_none_or(|(file, size, _)| *file != a.file || *size != a.size_bytes))
        .map(|a| (a.slot, a.file.clone(), a.size_bytes, a.path(&dir)))
        .collect();
    s.viewers.mod_heroes.map.retain(|slot, _| sig.iter().any(|(s, _, _)| s == slot));
    s.viewers.mod_heroes.sig = sig;
    if todo.is_empty() {
        return changed;
    }
    s.viewers.mod_heroes.job.start(s.wake.clone(), move || {
        Ok(todo
            .into_iter()
            .map(|(slot, file, size, path)| {
                let heroes = deadlock_core::viewer::heroes_in_vpk(&path).unwrap_or_default();
                (slot, file, size, heroes.iter().map(|h| prepare::hero_name(h)).collect())
            })
            .collect())
    });
    changed
}

/// Opens the dialog for the mod in `slot` on its first hero.
pub fn open(s: &mut State, slot: u16, name: &str) {
    let heroes = heroes_of(s, slot);
    if heroes.is_empty() {
        s.status = fill(tr("{} does not change a hero model."), &[name]);
        return;
    }
    s.viewers.dialog = Some(Dialog { slot, name: name.to_string(), heroes, hero: 0 });
    show(s);
}

/// The same, on a given hero (the Locker's skin rows).
pub fn open_on(s: &mut State, slot: u16, name: &str, hero: &str) {
    let mut heroes = heroes_of(s, slot);
    if !heroes.iter().any(|h| h == hero) {
        heroes.insert(0, hero.to_string());
    }
    let at = heroes.iter().position(|h| h == hero).unwrap_or(0);
    s.viewers.dialog = Some(Dialog { slot, name: name.to_string(), heroes, hero: at });
    show(s);
}

fn show(s: &mut State) {
    let Some(d) = &s.viewers.dialog else { return };
    let ask = Ask::Mod { hero: d.heroes[d.hero].clone(), slot: d.slot, name: d.name.clone() };
    prepare::request(s, DIALOG_KEY, ask, Owner::Dialog, None);
}

fn pick(s: &mut State, at: usize) {
    if let Some(d) = &mut s.viewers.dialog
        && d.hero != at
        && at < d.heroes.len()
    {
        d.hero = at;
        show(s);
    }
}

pub fn close(s: &mut State) {
    s.viewers.dialog = None;
    super::close(s, DIALOG_KEY);
}

/// The menu item and button label, for the screens that offer it.
pub const LABEL: &str = "Preview in 3D";

/// The dialog over the page, when open.
pub fn view(s: &State, u: &Ui) -> Option<El> {
    let d = s.viewers.dialog.as_ref()?;
    let enabled = s.addons.addons.iter().find(|a| a.slot == d.slot).map(|a| a.enabled);
    let mut head = row().w_full().gap(12.0).center_y().child(
        col()
            .grow(1.0)
            .min_w(0.0)
            .gap(2.0)
            .child(text(tr(LABEL).to_string()).font(SMALL).color(u.faint))
            .child(text(d.name.clone()).font(H2).color(u.ink).truncate()),
    );
    if enabled == Some(false) {
        let slot = d.slot;
        head = head.child(text(tr("Off")).font(SMALL).color(u.faint)).child(
            u.action(tr("Turn on"), ActionKind::Secondary)
                .key("viewer-dialog-enable")
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| crate::screens::installed::enable::switch(st, slot, true)),
        );
    }
    head = head.child(
        u.icon_button(&icons::X, tr("Close"))
            .key("viewer-dialog-close")
            .cursor(rune::Cursor::Pointer)
            .on_press(close),
    );
    let mut body = col().w_full().grow(1.0).min_h(0.0).gap(12.0).child(head);
    if d.heroes.len() > 1 {
        let mut tabs = crate::ui::Segmented::new("viewer-dialog-heroes", tr("Hero")).tabs();
        for (i, hero) in d.heroes.iter().enumerate() {
            tabs = tabs.part(
                crate::ui::SegPart::new(format!("viewer-dialog-hero-{i}"), hero.clone())
                    .on(i == d.hero)
                    .with(move |e| e.on_press(move |st: &mut State| pick(st, i))),
            );
        }
        let tabs = tabs.build(u);
        body = body.child(row().w_full().gap(10.0).center_y().child(text(tr("Changes")).font(SMALL).color(u.faint)).child(tabs).child(spacer()));
    }
    body = body.child(el().w_full().grow(1.0).min_h(0.0).child(super::view(s, u, DIALOG_KEY)));
    Some(
        u.dialog_shell()
            .key("viewer-dialog")
            .role("dialog")
            .label(fill(tr("{} on {}"), &[&d.name, &d.heroes[d.hero]]))
            .w_full()
            .max_w(1120.0)
            .h_full()
            .max_h(820.0)
            .padding(16.0)
            .modal(true, close)
            .on_key(|st: &mut State, ev: &KeyEvent, _| {
                if ev.key == Key::Escape && st.viewers.fullscreen.is_none() {
                    close(st);
                    true
                } else {
                    false
                }
            })
            .child(body),
    )
}
