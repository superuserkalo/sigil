//! Weapon skins, voice lines and weapon sounds per hero (C14), and sorting them by hero.
//!
//! - Deadlock Mod Manager: buckets sounds into ability, voice and weapon and lists weapon skins
//!   and voice lines under their hero, with a switch for the hero guess.
//! - Grimoire: skins and ability sounds per hero only.
//! - Sigil: every mod's VPK paths are read with the Locker (on its worker) into
//!   `locker_parts::hero_parts`; a hero page has a "Weapons and voice" section listing the
//!   weapon skins, voice lines and weapon sounds for that hero, each with its switch. A skin that
//!   ships its own gun stays a skin.

use std::collections::BTreeMap;
use std::path::Path;

use deadlock_core::locker_parts::{self, HeroPart};
use rune::{El, col, row, text};

use super::{HeroCard, ModRow, Parts, View};
use crate::app::State;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{BODY, BODY_MED, SMALL, Ui};

/// The parts one slot file changes, by hero. Empty when the file cannot be read.
pub fn read(addons_dir: &Path, slot: u16, file: &str) -> Parts {
    let path = deadlock_core::addons::slot_folder(addons_dir, slot).join(file);
    let options = deadlock_core::vpk::ReadVpkOptions { keep_preload: false };
    match deadlock_core::vpk::read_vpk_directory_with(&path, &options) {
        Ok(directory) => {
            let paths: Vec<String> = directory.entries.into_iter().map(|entry| entry.path).collect();
            locker_parts::hero_parts(&paths)
        }
        Err(_) => Parts::new(),
    }
}

/// The parts this section lists, in order.
pub const LISTED: [HeroPart; 3] = [HeroPart::Weapon, HeroPart::Voice, HeroPart::WeaponSound];

/// Mods by hero for one part, slots in load order: `locker_parts::by_hero`'s answer from the
/// parts kept on each row.
pub fn by_hero(mods: &[ModRow], part: HeroPart) -> BTreeMap<String, Vec<u16>> {
    let mut out: BTreeMap<String, Vec<u16>> = BTreeMap::new();
    for m in mods {
        for (hero, parts) in &m.parts {
            if parts.contains(&part) {
                out.entry(hero.clone()).or_default().push(m.slot);
            }
        }
    }
    for slots in out.values_mut() {
        slots.sort_unstable();
    }
    out
}

/// The rows for one hero and part.
pub fn rows<'a>(v: &'a View, hero: &str, part: HeroPart) -> Vec<&'a ModRow> {
    let mut list: Vec<&ModRow> = v
        .mods
        .iter()
        .filter(|m| m.parts.get(hero).is_some_and(|parts| parts.contains(&part)))
        .collect();
    list.sort_by_key(|m| m.slot);
    list
}

/// How many mods the section holds for a hero, for the rail count.
pub fn count(v: &View, hero: &str) -> usize {
    let mut slots: Vec<u16> = LISTED
        .iter()
        .filter_map(|part| by_hero(&v.mods, *part).remove(hero))
        .flatten()
        .collect();
    slots.sort_unstable();
    slots.dedup();
    slots.len()
}

fn label(part: HeroPart) -> &'static str {
    match part {
        HeroPart::Weapon => tr("Weapon skins"),
        HeroPart::Voice => tr("Voice lines"),
        HeroPart::WeaponSound => tr("Weapon sounds"),
        other => tr(other.label()),
    }
}

/// The hero page section: one group per part, each mod with its switch.
pub fn section(s: &State, u: &Ui, v: &View, hero: &HeroCard) -> El {
    let mut out = col().w_full().gap(16.0);
    let mut any = false;
    for part in LISTED {
        let list = rows(v, &hero.display_name, part);
        if list.is_empty() {
            continue;
        }
        any = true;
        let mut group = col().w_full().child(
            row()
                .w_full()
                .h(28.0)
                .gap(8.0)
                .center_y()
                .child(text(label(part)).font(BODY_MED).color(u.muted).grow(1.0).min_w(0.0))
                .child(
                    text(if list.len() == 1 { tr("1 mod").to_string() } else { fill(tr("{} mods"), &[&list.len().to_string()]) })
                        .font(SMALL)
                        .color(u.faint),
                ),
        );
        for m in list {
            // The row carries its own switch.
            group = group.child(super::mod_row(s, u, m, None));
        }
        out = out.child(group);
    }
    if !any {
        out = out.child(
            text(fill(tr("No installed mod changes {}'s weapon, voice lines or weapon sounds yet."), &[&hero.display_name]))
                .font(BODY)
                .color(u.muted)
                .wrap(),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(slot: u16, hero: &str, parts: &[HeroPart]) -> ModRow {
        let mut map = Parts::new();
        map.insert(hero.to_string(), parts.iter().copied().collect());
        ModRow {
            slot,
            file: format!("pak{slot:02}_dir.vpk"),
            enabled: true,
            key: format!("k{slot}"),
            label: format!("m{slot}"),
            heroes: Vec::new(),
            general: None,
            sounds: Vec::new(),
            parts: map,
        }
    }

    #[test]
    fn mods_sort_by_hero_per_part() {
        let mods = vec![
            row(4, "Haze", &[HeroPart::Weapon]),
            row(2, "Haze", &[HeroPart::Weapon, HeroPart::Voice]),
            row(3, "Wraith", &[HeroPart::Voice]),
        ];
        let weapons = by_hero(&mods, HeroPart::Weapon);
        assert_eq!(weapons.get("Haze"), Some(&vec![2, 4]));
        assert!(!weapons.contains_key("Wraith"));
        let voice = by_hero(&mods, HeroPart::Voice);
        assert_eq!(voice.len(), 2);
        let mut v = View::default();
        v.mods = mods;
        assert_eq!(count(&v, "Haze"), 2);
        assert_eq!(rows(&v, "Wraith", HeroPart::Voice).len(), 1);
    }
}
