//! Heroes for a mod GameBanana did not file under a hero (Grimoire #593 `9967d3a2` and
//! `fix/gb-category-drift`).
//!
//! A skin filed directly under Skins or Model Replacement, a multi-file submission, a sound or
//! HUD pack and a local file have no hero category. Their files usually still say which hero
//! they are for: voice lines under `sounds/vo/<codename>/`, footsteps, ability effects, a
//! portrait. When the files do not, the archive file name and then the mod's name are read for
//! a hero's name, whole words only. A broad skin whose files or title name several heroes gets
//! none: one skin is one hero's.
//!
//! The files are read on a worker through `deadlock_core::mod_heroes::heroes_cached`, which
//! keeps its answers in `<data>/mod-heroes.json` by path, size and modification time. Local
//! reads only, so it runs offline too. The guess never overwrites what GameBanana said: it is
//! computed when a filter or a card asks and is not stored in the extras file.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use deadlock_core::mod_heroes::{self, CachedHeroes, Payload};

use crate::app::State;
use crate::jobs::Job;
use crate::model::AddonRow;

/// VPKs read per worker run.
const BATCH: usize = 64;

/// What the worker found per slot file, keyed by slot and size.
#[derive(Default)]
pub struct Guess {
    pub answers: HashMap<(u16, u64), CachedHeroes>,
    tried: HashSet<(u16, u64)>,
    job: Job<Vec<((u16, u64), CachedHeroes)>>,
}

/// GameBanana Mod categories that never hold a hero skin: fps configs, fixes and gameplay
/// tweaks (Grimoire's `HERO_LESS_MOD_CATEGORIES`). Lowercase.
const HERO_LESS: &[&str] = &["quality of life/fixes", "quality of life", "accessibility", "gameplay modifications"];

/// Roots a broad skin is filed under with no hero below it. Lowercase.
const BROAD_ROOTS: &[&str] = &["skins", "model replacement"];

/// Take the worker's answers and read a few more slot files while Installed is open.
pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.installed.notes.guess.job.try_take() {
        if let Ok(found) = result {
            st.installed.notes.guess.answers.extend(found);
        }
        changed = true;
    }
    if st.demo || rune::is_headless() || !st.addons.scanned {
        return changed;
    }
    if st.screen != crate::model::Screen::Installed || st.installed.notes.guess.job.is_running() {
        return changed;
    }
    let dir = super::super::addons_folder(st);
    let guess = &st.installed.notes.guess;
    let wanted: Vec<((u16, u64), PathBuf)> = st
        .addons
        .addons
        .iter()
        .map(|addon| ((addon.slot, addon.size_bytes), addon.path(&dir)))
        .filter(|(key, _)| !guess.answers.contains_key(key) && !guess.tried.contains(key))
        .take(BATCH)
        .collect();
    if wanted.is_empty() {
        return changed;
    }
    st.installed.notes.guess.tried.extend(wanted.iter().map(|(key, _)| *key));
    let cache = crate::data::data_dir().join(mod_heroes::HERO_CACHE_FILE);
    st.installed.notes.guess.job.start(st.wake.clone(), move || {
        let paths: Vec<PathBuf> = wanted.iter().map(|(_, path)| path.clone()).collect();
        let found = mod_heroes::heroes_cached(&cache, &paths);
        Ok(wanted
            .into_iter()
            .filter_map(|(key, path)| found.get(&path).cloned().map(|answer| (key, answer)))
            .collect())
    });
    changed
}

/// Every hero a mod is for: GameBanana's hero category when it has one, else the guess from
/// its files, its archive file name and its name. Sorted, without duplicates.
pub fn heroes_of(st: &State, addon: &AddonRow) -> Vec<String> {
    let entry = super::extra(st, addon);
    if let Some(hero) = entry.hero.as_ref().filter(|hero| !hero.trim().is_empty()) {
        return vec![hero.clone()];
    }
    let meta = super::meta_for(st, addon);
    let linked = entry.gamebanana_id.is_some() || meta.and_then(|meta| meta.gamebanana_id).is_some();
    let category = entry.category.as_deref().unwrap_or("").trim().to_lowercase();
    let root = entry.root_category.as_deref().unwrap_or("").trim().to_lowercase();
    if linked && !eligible(&category, &root) {
        return Vec::new();
    }
    let broad = linked && BROAD_ROOTS.contains(&category.as_str()) && (root.is_empty() || root == category);
    let answer = st.installed.notes.guess.answers.get(&(addon.slot, addon.size_bytes));
    let payload = answer.map(|answer| answer.payload.clone()).unwrap_or(Payload::Unknown);
    let files: Vec<String> = answer.map(|answer| answer.assets.clone()).unwrap_or_default();
    let file_name = entry.file_name.clone().unwrap_or_default();
    let name = meta.map(|meta| meta.name.clone()).unwrap_or_default();
    let levels: [&[&str]; 2] = [&[file_name.as_str()], &[name.as_str()]];
    mod_heroes::guess_heroes(&payload, &files, broad, &levels).0
}

/// Is a GameBanana mod in a category the guess applies to? Not a QoL or gameplay tweak, and
/// not a General destination like Hideout or Soul Containers (HUD and sounds still are: a HUD
/// or a voice pack can be for one hero).
fn eligible(category: &str, root: &str) -> bool {
    if HERO_LESS.contains(&category) || HERO_LESS.contains(&root) {
        return false;
    }
    let general = deadlock_core::locker::general_for_category(category)
        .or_else(|| deadlock_core::locker::general_for_category(root));
    matches!(general, None | Some(deadlock_core::locker::GeneralType::Hud))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qol_and_general_categories_get_no_guess() {
        assert!(!eligible("quality of life/fixes", ""));
        assert!(!eligible("accessibility", "quality of life/fixes"));
        assert!(!eligible("hideout", "model replacement"));
        assert!(eligible("hud", "hud"));
        assert!(eligible("skins", "skins"));
        assert!(eligible("gameplay events", ""));
    }
}
