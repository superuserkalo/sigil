//! One skin per hero (DMM's unreleased `feature/duplicate-mod-flag`, behaviour only): when an
//! install adds a skin for a hero who already has another skin on, ask whether to keep both or
//! only the new one. The question is a notice with an action, so nothing waits on it; "Keep
//! only the new one" turns the hero's other skins off through the same rename the Locker's
//! Equip uses. With "several skins per hero" on in the Locker the question is not asked.

use std::collections::BTreeMap;
use std::path::PathBuf;

use deadlock_core::import::parts::read_part;
use deadlock_core::locker::hero_asset_key;

use crate::app::State;
use crate::jobs::Job;
use crate::notify::{self, Notice};
use crate::screens::settings::i18n::{fill, tr};

/// A finished check: hero, new slot, new name, and the hero's other enabled skins.
type Found = Option<(String, u16, String, Vec<(u16, String)>)>;

/// One check at a time; installs that land meanwhile wait their turn.
#[derive(Default)]
pub struct Check {
    job: Job<Found>,
    waiting: Vec<(u16, String)>,
}

/// Read the new VPK and every other enabled one (file lists only). A file that landed switched
/// off has no enabled path, reads as nothing and asks nothing.
fn find(addons: PathBuf, slot: u16, name: String, others: Vec<(u16, String)>) -> Found {
    let path = |slot| deadlock_core::addons::slot_path(&addons, slot, "_dir.vpk");
    let part = read_part(&path(slot));
    // One hero only: a pack for several heroes is not a second look for one of them.
    let [hero] = part.skin_heroes.as_slice() else { return None };
    let key = hero_asset_key(hero);
    let found: Vec<(u16, String)> = others
        .into_iter()
        .filter(|(other, _)| read_part(&path(*other)).skin_heroes.iter().any(|h| hero_asset_key(h) == key))
        .collect();
    (!found.is_empty()).then(|| (hero.clone(), slot, name, found))
}

fn start(st: &mut State, slot: u16, name: String) {
    let Some(game_root) = st.game_root.clone() else { return };
    let addons = deadlock_core::deploy::addons_dir(&game_root);
    let mut others: Vec<(u16, String)> = st
        .addons
        .addons
        .iter()
        .filter(|addon| addon.enabled && addon.slot != slot)
        .map(|addon| (addon.slot, crate::screens::installed::display_name(st, addon)))
        .collect();
    others.sort_by_key(|(slot, _)| *slot);
    let wake = st.wake.clone();
    st.installed.tools.one_skin.job.start(wake, move || Ok(find(addons, slot, name, others)));
}

/// After an install landed in `slot`: check on a worker whether it is a second skin.
pub fn after_install(st: &mut State, slot: u16, name: &str) {
    if st.locker.prefs.multi_skin || st.demo {
        return;
    }
    if st.installed.tools.one_skin.job.is_running() {
        st.installed.tools.one_skin.waiting.push((slot, name.to_string()));
    } else {
        start(st, slot, name.to_string());
    }
}

pub fn poll(st: &mut State) -> bool {
    let Some(result) = st.installed.tools.one_skin.job.try_take() else {
        return false;
    };
    if let Ok(Some((hero, slot, name, others))) = result {
        ask(st, &hero, slot, &name, &others);
    }
    if !st.installed.tools.one_skin.waiting.is_empty() {
        let (slot, name) = st.installed.tools.one_skin.waiting.remove(0);
        start(st, slot, name);
    }
    true
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
    if crate::screens::installed::waiting::refuse_held(st, crate::screens::installed::waiting::HELD_MIXED) {
        return;
    }
    if st.demo || crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    let Some(game_root) = st.game_root.clone() else {
        notify::push(st, Notice::error(tr("No Deadlock install was found")).detail(tr("Nothing was renamed.")));
        return;
    };
    let wanted: BTreeMap<u16, bool> = off.iter().map(|other| (*other, false)).chain([(slot, true)]).collect();
    match deadlock_core::addons::set_many(&game_root, &wanted, true) {
        Ok(report) => {
            crate::screens::settings::product::switched(st, report.done.iter().map(|done| done.slot), &wanted);
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
    fn a_second_skin_is_found_and_a_file_that_landed_off_asks_nothing() {
        use deadlock_core::vpk_write::{PackOptions, pack_vpk};
        let root = std::env::temp_dir().join(format!("sigil-one-skin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let addons = root.join("addons");
        std::fs::create_dir_all(&addons).unwrap();
        for (name, path) in [
            ("pak01", "models/heroes_staging/haze/haze.vmdl_c"),
            ("pak02", "models/heroes_staging/inferno/inferno.vmdl_c"),
            ("pak03", "models/heroes_staging/haze/haze_alt.vmdl_c"),
        ] {
            let stage = root.join(format!("stage-{name}"));
            let file = stage.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, b"x").unwrap();
            let packed = pack_vpk(&PackOptions::new(stage, root.join(format!("out-{name}")), name)).unwrap();
            std::fs::rename(&packed.dir_path, addons.join(format!("{name}_dir.vpk"))).unwrap();
        }
        let others = vec![(1, "Old Haze".to_string()), (2, "Infernus".to_string())];
        let (_, slot, _, found) = find(addons.clone(), 3, "New Haze".into(), others.clone()).expect("a second skin");
        assert_eq!((slot, found), (3, vec![(1, "Old Haze".to_string())]));
        assert!(find(addons, 9, "Off".into(), others).is_none(), "no enabled file, no question");
        let _ = std::fs::remove_dir_all(&root);
    }

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
