//! Locker data both reference apps show beyond one skin per hero: what part of a hero a mod
//! replaces (skin, weapon skin, voice lines, weapon sounds, ability sounds), several skins on at
//! once, and a shuffle pool that never re-rolls the skin already on.
//!
//! - Parts per hero follow Deadlock Mod Manager's sound buckets (`sounds/abilities/`,
//!   `sounds/vo/` or `sounds/voice/`, `sounds/weapons/`) with the hero read from the next folder,
//!   and add weapon skins: `models/weapons/...` (and its materials) whose folder or file name
//!   starts with a hero codename (`models/weapons/hero/nano_gun.vmdl_c` is Calico's gun).
//! - Several skins: Grimoire lets a hero keep more than one skin on (a body and a separate hat
//!   or weapon mod). [`equip_wanted`] plans either "replace" (the one skin, others off, the
//!   existing behaviour) or "add" (turn this one on, leave the rest).
//! - Shuffle pool: Grimoire's randomizer draws only from skins the player opted in and avoids
//!   the current pick when another exists (`avoidCurrent`); DMM's random-on-launch does the same
//!   for its lists. [`shuffle_wanted`] does both.
//!
//! Every function here plans; the app applies a plan with the existing approved
//! [`crate::addons::set_many`]. Nothing here writes to the game install.

use std::collections::{BTreeMap, BTreeSet};

use crate::error::{Error, Result};
use crate::locker::{LockerMod, hero_asset_key, hero_for_codename, hero_mods};

/// What part of a hero a path replaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HeroPart {
    /// Body model, materials or animations (`models/heroes*`, `materials/models/heroes*`).
    Skin,
    /// The hero's gun model or its materials.
    Weapon,
    /// Voice lines (`sounds/vo/<hero>/`).
    Voice,
    /// Gun sounds (`sounds/weapons/<hero>/`).
    WeaponSound,
    /// Ability sounds (`sounds/abilities/<hero>/`).
    AbilitySound,
}

impl HeroPart {
    pub fn label(self) -> &'static str {
        match self {
            HeroPart::Skin => "Skin",
            HeroPart::Weapon => "Weapon skin",
            HeroPart::Voice => "Voice lines",
            HeroPart::WeaponSound => "Weapon sounds",
            HeroPart::AbilitySound => "Ability sounds",
        }
    }

    pub const ALL: [HeroPart; 5] =
        [HeroPart::Skin, HeroPart::Weapon, HeroPart::Voice, HeroPart::WeaponSound, HeroPart::AbilitySound];
}

/// The hero and part one game path belongs to.
pub fn part_of_path(path: &str) -> Option<(String, HeroPart)> {
    let path = path.to_ascii_lowercase().replace('\\', "/");
    let hero_in = |rest: &str| rest.split('/').next().and_then(hero_for_codename).map(|h| h.name.to_string());
    if let Some(rest) = path.strip_prefix("sounds/abilities/") {
        return hero_in(rest).map(|hero| (hero, HeroPart::AbilitySound));
    }
    if let Some(rest) = path.strip_prefix("sounds/vo/").or_else(|| path.strip_prefix("sounds/voice/")) {
        return hero_in(rest).map(|hero| (hero, HeroPart::Voice));
    }
    if let Some(rest) = path.strip_prefix("sounds/weapons/") {
        return hero_in(rest).map(|hero| (hero, HeroPart::WeaponSound));
    }
    let model = path.strip_prefix("materials/").unwrap_or(&path);
    if let Some(rest) = model.strip_prefix("models/weapons/") {
        // A hero folder anywhere below, or a file named after the hero (`nano_gun.vmdl_c`).
        let segments: Vec<&str> = rest.split('/').collect();
        let (file, folders) = segments.split_last()?;
        let stem = file.split('.').next().unwrap_or(file);
        return folders
            .iter()
            .rev()
            .find_map(|folder| hero_for_codename(folder))
            .or_else(|| hero_for_codename(stem))
            .map(|hero| (hero.name.to_string(), HeroPart::Weapon));
    }
    if model.starts_with("models/heroes") {
        let hero = crate::locker::hero_of_path(&path)?;
        let tail = path.rsplit('/').next().unwrap_or("");
        let part = if tail.contains("weapon") || tail.contains("_gun") { HeroPart::Weapon } else { HeroPart::Skin };
        return Some((hero.name.to_string(), part));
    }
    None
}

/// Every hero part a mod's paths replace: hero to parts.
pub fn hero_parts<S: AsRef<str>>(paths: &[S]) -> BTreeMap<String, BTreeSet<HeroPart>> {
    let mut out: BTreeMap<String, BTreeSet<HeroPart>> = BTreeMap::new();
    for path in paths {
        if let Some((hero, part)) = part_of_path(path.as_ref()) {
            out.entry(hero).or_default().insert(part);
        }
    }
    // A skin that ships its own gun materials is still a skin; only a mod without body files is
    // a weapon skin for that hero.
    for parts in out.values_mut() {
        if parts.contains(&HeroPart::Skin) {
            parts.remove(&HeroPart::Weapon);
        }
    }
    out
}

/// Slots grouped by hero for one part, heroes alphabetical and slots in load order: "sort
/// sounds and weapon skins by hero". `mods` is `(slot, paths)`.
pub fn by_hero(mods: &[(u16, Vec<String>)], part: HeroPart) -> BTreeMap<String, Vec<u16>> {
    let mut out: BTreeMap<String, Vec<u16>> = BTreeMap::new();
    for (slot, paths) in mods {
        for (hero, parts) in hero_parts(paths) {
            if parts.contains(&part) {
                out.entry(hero).or_default().push(*slot);
            }
        }
    }
    for slots in out.values_mut() {
        slots.sort_unstable();
        slots.dedup();
    }
    out
}

/// How equipping a skin treats the hero's other skins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EquipMode {
    /// One skin: this one on, every other mod of the hero off (the Locker's default).
    #[default]
    Replace,
    /// Several skins: turn this one on and leave the others as they are.
    Add,
}

/// The on/off plan for equipping `slot` as one of `hero`'s skins. Apply with
/// [`crate::addons::set_many`] (approved). Refuses a slot that is not one of the hero's mods.
pub fn equip_wanted(mods: &[LockerMod], hero: &str, slot: u16, mode: EquipMode) -> Result<BTreeMap<u16, bool>> {
    let of = hero_mods(mods, hero);
    if !of.iter().any(|item| item.slot == slot) {
        return Err(Error::msg(format!("Slot {slot:02} is not one of {hero}'s installed mods.")));
    }
    Ok(match mode {
        EquipMode::Replace => of.iter().map(|item| (item.slot, item.slot == slot)).collect(),
        EquipMode::Add => BTreeMap::from([(slot, true)]),
    })
}

/// The skins of `hero` that are on now.
pub fn equipped(mods: &[LockerMod], hero: &str) -> Vec<u16> {
    hero_mods(mods, hero).iter().filter(|item| item.enabled).map(|item| item.slot).collect()
}

/// SplitMix64 over the seed and the hero key, so each hero rolls on its own.
fn roll(seed: u64, key: &str) -> u64 {
    let mut z = key.bytes().fold(seed ^ 0x9e37_79b9_7f4a_7c15, |acc, b| acc.rotate_left(5) ^ u64::from(b));
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// The shuffle plan for `heroes`: for each, one pick from its pool (the slots in `pool`, or
/// every mod of the hero when `pool` is None or holds none of them), avoiding the skin on now
/// when another candidate exists; the pick goes on and the hero's other pooled mods go off.
/// Mods outside the pool are left as they are (a hat the player keeps on). Deterministic for a
/// seed. Apply with [`crate::addons::set_many`].
pub fn shuffle_wanted(
    mods: &[LockerMod],
    heroes: &[String],
    pool: Option<&BTreeSet<u16>>,
    seed: u64,
) -> BTreeMap<u16, bool> {
    let mut wanted = BTreeMap::new();
    let keys: BTreeSet<String> = heroes.iter().map(|h| hero_asset_key(h)).filter(|k| !k.is_empty()).collect();
    for key in keys {
        let of = hero_mods(mods, &key);
        if of.is_empty() {
            continue;
        }
        let pooled: Vec<&&LockerMod> = match pool {
            Some(pool) if of.iter().any(|item| pool.contains(&item.slot)) => {
                of.iter().filter(|item| pool.contains(&item.slot)).collect()
            }
            _ => of.iter().collect(),
        };
        let fresh: Vec<&&LockerMod> = pooled.iter().copied().filter(|item| !item.enabled).collect();
        let candidates = if fresh.is_empty() { &pooled } else { &fresh };
        let pick = candidates[(roll(seed, &key) % candidates.len() as u64) as usize].slot;
        for item in &pooled {
            let on = item.slot == pick;
            // A mod two heroes share stays on when either picks it.
            let entry = wanted.entry(item.slot).or_insert(on);
            *entry |= on;
        }
    }
    wanted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skin(slot: u16, hero: &str, enabled: bool) -> LockerMod {
        LockerMod {
            slot,
            file: format!("pak{slot:02}_dir.vpk"),
            enabled,
            key: format!("k{slot}"),
            entry_count: 1,
            heroes: vec![hero.to_string()],
            general: None,
            sounds: Vec::new(),
        }
    }

    #[test]
    fn paths_name_their_hero_and_part() {
        assert_eq!(part_of_path("sounds/vo/haze/haze_kill_01.vsnd_c"), Some(("Haze".into(), HeroPart::Voice)));
        assert_eq!(part_of_path("sounds/weapons/haze/smg_fire.vsnd_c"), Some(("Haze".into(), HeroPart::WeaponSound)));
        assert_eq!(part_of_path("models/weapons/hero/nano_gun.vmdl_c"), Some(("Calico".into(), HeroPart::Weapon)));
        assert_eq!(
            part_of_path("materials/models/weapons/inferno/flame_gun.vmat_c"),
            Some(("Infernus".into(), HeroPart::Weapon))
        );
        assert_eq!(part_of_path("sounds/abilities/atlas/a1_cast.vsnd_c").map(|p| p.1), Some(HeroPart::AbilitySound));
        assert_eq!(part_of_path("sounds/vo/shopkeeper/hello.vsnd_c"), None, "not a hero");
        assert_eq!(part_of_path("models/heroes_wip/haze/haze.vmdl_c").map(|p| p.1), Some(HeroPart::Skin));
        assert_eq!(part_of_path("panorama/images/x.png"), None);
    }

    #[test]
    fn a_skin_with_its_own_gun_is_a_skin() {
        let parts = hero_parts(&["models/heroes_wip/haze/haze.vmdl_c", "models/weapons/haze/haze_gun.vmdl_c"]);
        assert_eq!(parts["Haze"], BTreeSet::from([HeroPart::Skin]));
        let gun = hero_parts(&["models/weapons/haze/haze_gun.vmdl_c", "sounds/vo/haze/a.vsnd_c"]);
        assert_eq!(gun["Haze"], BTreeSet::from([HeroPart::Weapon, HeroPart::Voice]));
        let grouped = by_hero(
            &[
                (3, vec!["sounds/vo/haze/a.vsnd_c".to_string()]),
                (1, vec!["sounds/vo/haze/b.vsnd_c".to_string(), "sounds/vo/atlas/c.vsnd_c".to_string()]),
            ],
            HeroPart::Voice,
        );
        assert_eq!(grouped["Haze"], vec![1, 3]);
        assert_eq!(grouped["Abrams"], vec![1]);
    }

    #[test]
    fn equip_replaces_or_adds() {
        let mods = vec![skin(1, "Haze", true), skin(2, "Haze", false), skin(3, "Abrams", true)];
        let replace = equip_wanted(&mods, "Haze", 2, EquipMode::Replace).unwrap();
        assert_eq!(replace, BTreeMap::from([(1, false), (2, true)]));
        let add = equip_wanted(&mods, "haze", 2, EquipMode::Add).unwrap();
        assert_eq!(add, BTreeMap::from([(2, true)]));
        assert!(equip_wanted(&mods, "Haze", 3, EquipMode::Add).is_err());
        assert_eq!(equipped(&mods, "Haze"), vec![1]);
    }

    #[test]
    fn shuffle_avoids_the_current_pick_and_keeps_to_the_pool() {
        let mods = vec![skin(1, "Haze", true), skin(2, "Haze", false), skin(3, "Haze", false), skin(4, "Haze", true)];
        let pool = BTreeSet::from([1, 2, 3]);
        for seed in 0..200 {
            let wanted = shuffle_wanted(&mods, &["Haze".to_string()], Some(&pool), seed);
            assert_eq!(wanted.get(&1), Some(&false), "the current pick is not rolled again");
            assert!(!wanted.contains_key(&4), "a mod outside the pool is left alone");
            assert_eq!(wanted.values().filter(|on| **on).count(), 1);
        }
        // One candidate only: it stays on.
        let single = vec![skin(1, "Haze", true)];
        assert_eq!(shuffle_wanted(&single, &["Haze".to_string()], None, 7), BTreeMap::from([(1, true)]));
        assert!(shuffle_wanted(&mods, &["Nobody".to_string()], None, 1).is_empty());

        let mut prefs = crate::locker::LockerPrefs::default();
        assert_eq!(prefs.pool_slots(&mods), None);
        assert!(prefs.toggle_pool("k2"));
        assert_eq!(prefs.pool_slots(&mods), Some(BTreeSet::from([2])));
        assert!(!prefs.toggle_pool("k2"));
        assert_eq!(prefs.equip_mode(), EquipMode::Replace);
        prefs.multi_skin = true;
        assert_eq!(prefs.equip_mode(), EquipMode::Add);
    }
}
