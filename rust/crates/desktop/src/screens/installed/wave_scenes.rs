//! Headless scenes for the parity wave's Installed states: the hero, requirement and dependent
//! questions, the hidden-model mark, a drag, several files of one mod on at once, the selection's
//! update and copy, the changelog in the update confirm, the load order dialog, the overlap
//! badge and its list, and the hero question raised by model paths alone.

use deadlock_core::requirements::{MissingRequirement, RequirementState};

use crate::app::State;
use crate::model::ConflictRow;

use super::enable::Gate;
use super::{Confirm, Menu, drag, layout, order, overlap};

fn base(s: &mut State) {
    crate::app::scene_demo_installed(s);
}

/// Slot 11 (Paradox, off) is turned on while slot 05 is a Paradox skin too.
pub fn hero(s: &mut State) {
    base(s);
    s.installed.gates.open = Some(Gate::Hero { slot: 11, hero: "Paradox".to_string(), others: vec![5] });
}

/// Slot 12 needs one installed mod that is off and one that is not installed.
pub fn needs(s: &mut State) {
    base(s);
    let missing = vec![
        MissingRequirement {
            gamebanana_id: Some(600_009),
            name: "Taokaka over Drifter".to_string(),
            url: Some("https://gamebanana.com/mods/600009".to_string()),
            state: RequirementState::InstalledOff,
        },
        MissingRequirement {
            gamebanana_id: Some(552_211),
            name: "Soul Container base pack".to_string(),
            url: Some("https://gamebanana.com/mods/552211".to_string()),
            state: RequirementState::NotInstalled,
        },
    ];
    s.installed.gates.open = Some(Gate::Needs { slot: 12, also_off: Vec::new(), missing, with: [9].into_iter().collect() });
}

/// Turning slot 01 off while two mods list it as a requirement.
pub fn breaks(s: &mut State) {
    base(s);
    s.installed.gates.open = Some(Gate::Breaks { slot: 1, dependents: vec![3, 7] });
}

/// Slot 04 ships a model slot 02 also ships; 02 loads first, so 04's model is hidden.
pub fn hidden(s: &mut State) {
    base(s);
    s.addons.conflicts.push(ConflictRow {
        path: "models/heroes_wip/paige/paige.vmdl_c".to_string(),
        slots: vec![2, 4],
        winner: 2,
    });
    super::enable::refresh_hidden(s);
}

/// Card 05 dragged two places left, over card 02.
pub fn dragging(s: &mut State) {
    base(s);
    s.installed.hovered = Some(5);
    let order = drag::order(s, true);
    let from = order.iter().position(|slot| *slot == 5).unwrap_or(0);
    let to = order.iter().position(|slot| *slot == 2).unwrap_or(0);
    s.installed.drag = Some(drag::Dragging { slot: 5, enabled: true, from, to });
}

/// The list look with row 07 dragged up over row 03.
pub fn dragging_list(s: &mut State) {
    base(s);
    s.installed.layout = layout::Layout::List;
    s.installed.hovered = Some(7);
    let order = drag::order(s, true);
    let from = order.iter().position(|slot| *slot == 7).unwrap_or(0);
    let to = order.iter().position(|slot| *slot == 3).unwrap_or(0);
    s.installed.drag = Some(drag::Dragging { slot: 7, enabled: true, from, to });
}

/// Two files of one mod, both on, with the picker open.
pub fn variants_on(s: &mut State) {
    super::scene_variants(s);
    for addon in &mut s.addons.addons {
        if addon.slot == 11 || addon.slot == 18 {
            addon.enabled = true;
            addon.file = format!("pak{:02}_dir.vpk", addon.slot);
        }
    }
}

/// Three picked mods, two with an update: the bar offers Update 2 and Copy.
pub fn selection(s: &mut State) {
    base(s);
    super::stage_updates(s);
    s.installed.selected = [2, 3, 4].into_iter().collect();
    s.installed.anchor = Some(4);
}

/// The Update all confirm with each mod's newest changelog entry.
pub fn changelog(s: &mut State) {
    base(s);
    super::stage_updates(s);
    let slots = super::available_updates(s);
    for (n, slot) in slots.iter().enumerate() {
        let Some(id) = s
            .installed
            .library
            .mods
            .iter()
            .find(|meta| meta.slot == *slot)
            .and_then(|meta| meta.gamebanana_id)
        else {
            continue;
        };
        let entry = deadlock_core::gamebanana::ModUpdate {
            id: 90 + n as u64,
            title: ["Colour fixes", "Patch-day update", "New glow"][n % 3].to_string(),
            version: Some(format!("1.{}", n + 2)),
            text: String::new(),
            changes: vec![
                ("Bugfix".to_string(), "Arms no longer stretch in the hideout".to_string()),
                ("Adjustment".to_string(), "Paths updated for the November patch".to_string()),
            ],
            added_ts: 0,
        };
        s.installed.updates.changelog.insert(id, Ok(vec![entry]));
    }
    s.installed.confirm = Some(Confirm::UpdateAll(slots));
}

/// The load order dialog with slot 07 moved to the top and slot 04 picked up.
pub fn load_order(s: &mut State) {
    base(s);
    order::open(s);
    if let Some(dialog) = s.installed.order.as_mut() {
        dialog.order = order::moved(&dialog.order, 7, order::Move::First);
        dialog.picked = Some(4);
        dialog.held = true;
    }
}

/// The load order dialog as it opens: nothing moved, the first row picked.
pub fn load_order_open(s: &mut State) {
    base(s);
    order::open(s);
}

/// The overlap badges on the cards, from the demo's conflict rows.
pub fn overlap_badges(s: &mut State) {
    base(s);
    overlap::refresh(s);
}

/// Slot 07's overlap badge pressed: who it overlaps and who wins.
pub fn overlap_list(s: &mut State) {
    base(s);
    overlap::refresh(s);
    s.installed.hovered = Some(7);
    s.installed.menu = Some(Menu::Overlap(7));
}

/// Slot 12 (off, no hero tag) replaces Paradox's model, and slot 05 is a Paradox skin: turning
/// 12 on asks, from the model paths alone.
pub fn hero_from_models(s: &mut State) {
    base(s);
    for slot in [5u8, 12] {
        if let Some(addon) = s.addons.addons.iter().find(|a| a.slot == slot) {
            let key = (addon.slot, addon.size_bytes);
            s.installed.gates.model_heroes.insert(key, vec!["Paradox".to_string()]);
        }
    }
    super::enable::switch(s, 12, true);
}

/// A crawling download raises its one toast.
pub fn slow_toast(s: &mut State) {
    crate::screens::launch::scene_downloads_slow(s);
    crate::screens::downloads::poll(s);
}
