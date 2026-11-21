//! One skin per hero (DMM's unreleased `feature/duplicate-mod-flag`, behaviour only): when an
//! install adds a skin for a hero who already has another skin on, ask whether to keep both or
//! only the new one. The question is a notice with an action, so nothing waits on it; "Keep
//! only the new one" turns the hero's other skins off through the same rename the Locker's
//! Equip uses. With "several skins per hero" on in the Locker the question is not asked.

use std::collections::BTreeMap;

use deadlock_core::import::parts::read_part;
use deadlock_core::locker::hero_asset_key;
use deadlock_core::locker_parts::HeroPart;

use crate::app::State;
use crate::notify::{self, Notice};
use crate::screens::settings::i18n::{fill, tr};

/// The other enabled skins of `hero`: (slot, name), by load order. Read from the Locker.
fn other_skins(st: &State, hero: &str, new_slot: u16) -> Vec<(u16, String)> {
    let key = hero_asset_key(hero);
    let mut found: Vec<(u16, String)> = st
        .locker
        .mods
        .iter()
        .filter(|m| m.enabled && m.slot != new_slot)
        .filter(|m| {
            m.parts
                .iter()
                .any(|(name, parts)| hero_asset_key(name) == key && parts.contains(&HeroPart::Skin))
        })
        .map(|m| (m.slot, m.label.clone()))
        .collect();
    found.sort_by_key(|(slot, _)| *slot);
    found
}

/// After an install landed in `slot`: ask when it is a second skin for a hero.
pub fn after_install(st: &mut State, slot: u16, name: &str) {
    if st.locker.prefs.multi_skin {
        return;
    }
    let Some(game_root) = st.game_root.clone() else { return };
    let addons = deadlock_core::deploy::addons_dir(&game_root);
    let file = deadlock_core::addons::slot_path(&addons, slot, "_dir.vpk");
    let part = read_part(&file);
    // One hero only: a pack for several heroes is not a second look for one of them.
    let [hero] = part.skin_heroes.as_slice() else { return };
    let others = other_skins(st, hero, slot);
    ask(st, hero, slot, name, &others);
}

fn ask(st: &mut State, hero: &str, slot: u16, name: &str, others: &[(u16, String)]) {
    if others.is_empty() {
        return;
    }
    let names: Vec<&str> = others.iter().map(|(_, label)| label.as_str()).collect();
    let off: Vec<u16> = others.iter().map(|(slot, _)| *slot).collect();
    let hero_name = hero.to_string();
    let new_name = name.to_string();
    let notice = Notice::info(fill(tr("{} now has two skins on"), &[hero]))
        .detail(fill(
            tr("{} is on as well as {}. The lower slot wins where they replace the same files."),
            &[name, &names.join(", ")],
        ))
        .action(tr("Keep only the new one"), move |st: &mut State| {
            keep_only(st, &hero_name, slot, &new_name, &off)
        });
    notify::push(st, notice);
}

/// The player's yes: turn the hero's other skins off. Game write, player-triggered.
fn keep_only(st: &mut State, hero: &str, slot: u16, name: &str, off: &[u16]) {
    if st.demo || crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    let Some(game_root) = st.game_root.clone() else {
        notify::push(st, Notice::error(tr("No Deadlock install was found")).detail(tr("Nothing was renamed.")));
        return;
    };
    let wanted: BTreeMap<u16, bool> = off.iter().map(|other| (*other, false)).chain([(slot, true)]).collect();
    match deadlock_core::addons::set_many(&game_root, &wanted, true) {
        Ok(_) => {
            notify::push(st, Notice::success(fill(tr("Only {} is on for {}"), &[name, hero])));
            crate::data::scan_addons_now(st);
            crate::data::start_addon_scan(st);
            crate::data::start_locker_load(st);
        }
        Err(error) => notify::push(st, Notice::error(tr("The other skins were not turned off")).detail(error.to_string())),
    }
}

/// Headless scene: the question after installing a second Haze skin.
pub fn scene(st: &mut State) {
    crate::app::scene_demo_installed(st);
    ask(st, "Haze", 7, "Neon Haze", &[(3, "Midnight Haze".to_string())]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_question_names_the_other_skin() {
        let mut st = State::default();
        ask(&mut st, "Haze", 7, "Neon Haze", &[(3, "Midnight Haze".to_string())]);
        assert_eq!(st.notices.len(), 1);
        let mut none = State::default();
        ask(&mut none, "Haze", 7, "Neon Haze", &[]);
        assert!(none.notices.is_empty(), "a first skin asks nothing");
    }
}
