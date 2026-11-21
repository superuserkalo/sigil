//! Favourites per profile (DMM's `feature/293/favourite-mods`, behaviour only): each profile
//! keeps its own saved mods. Saving a profile stores the current set, applying it brings that
//! set back (Undo puts the earlier one back), and saving or unsaving a mod while a profile is
//! active updates that profile. Core: `deadlock_core::profile_favorites`. Sigil's data folder
//! and workspace only.

use deadlock_core::mod_extras;
use deadlock_core::profile_favorites as core;
use deadlock_core::profiles::active_profile;

use crate::app::State;

fn live(st: &State) -> bool {
    !st.demo && !st.profiles.fixture && !rune::is_headless()
}

/// The saved mods now, by notes uid.
pub fn current(st: &State) -> Vec<String> {
    st.installed.notes.extras.favorites()
}

/// Store the current favourites in the profile `slug` (a create or an Update).
pub fn remember(st: &State, slug: &str) {
    if live(st) {
        let _ = core::store_favorites(&crate::data::workspace_dir(), slug, &current(st));
    }
}

/// After a save or unsave: the active profile keeps the new set.
pub fn sync_active(st: &State) {
    if !live(st) {
        return;
    }
    let workspace = crate::data::workspace_dir();
    if let Some(slug) = active_profile(&workspace) {
        let _ = core::store_favorites(&workspace, &slug, &current(st));
    }
}

/// Put exactly `uids` on the shared saved flag and reload the notes.
pub fn set(st: &mut State, uids: &[String]) {
    if !live(st) || current(st) == uids {
        return;
    }
    let path = crate::screens::installed::notes::extras_file();
    match mod_extras::edit_extras(&path, |extras| extras.set_favorites(uids)) {
        Ok(_) => st.installed.notes.extras = mod_extras::read_extras(&path).unwrap_or_default(),
        Err(error) => st.installed.notes.error = Some(error.to_string()),
    }
}

/// After an apply: bring back the active profile's favourites. Returns the set before, for
/// Undo. A profile that keeps none leaves the current set alone.
pub fn bring_back(st: &mut State) -> Vec<String> {
    let before = current(st);
    let workspace = crate::data::workspace_dir();
    if let Some(uids) = active_profile(&workspace).and_then(|slug| core::stored_favorites(&workspace, &slug)) {
        set(st, &uids);
    }
    before
}
