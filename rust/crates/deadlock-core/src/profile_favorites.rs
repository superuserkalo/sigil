//! Favourites per profile (DMM's unreleased `feature/293/favourite-mods`, behaviour only).
//!
//! The saved flag lives on each mod in Sigil's notes file (`mod_extras::ModExtra::favorite`), so
//! Installed and the Locker read one set. A profile keeps its own copy of that set under this
//! app's extension namespace, by `ModExtra::uid`: applying the profile brings its favourites
//! back, and saving or unsaving a mod while a profile is active updates that profile. A profile
//! saved before it kept favourites has none stored and leaves the current set alone.
//!
//! Writes the workspace only, never a game install.

use std::path::Path;

use serde_json::{Map, Value};

use crate::error::Result;
use crate::profiles::{EXTENSION_KEY, Profile, read_profile, write_profile};

const FAVORITES_KEY: &str = "favorites";

/// The favourites stored with `profile`, sorted. `None` for a profile saved before profiles
/// kept them.
pub fn favorites_of(profile: &Profile) -> Option<Vec<String>> {
    let value = profile.extensions.get(EXTENSION_KEY)?.get(FAVORITES_KEY)?;
    let mut uids: Vec<String> = serde_json::from_value(value.clone()).ok()?;
    uids.sort();
    uids.dedup();
    Some(uids)
}

/// Store `uids` as the profile's favourites, keeping anything else in the namespace.
pub fn set_favorites(profile: &mut Profile, uids: &[String]) {
    let entry = profile
        .extensions
        .entry(EXTENSION_KEY.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        *entry = Value::Object(Map::new());
    }
    let mut uids = uids.to_vec();
    uids.sort();
    uids.dedup();
    if let Some(map) = entry.as_object_mut() {
        map.insert(FAVORITES_KEY.to_string(), Value::from(uids));
    }
}

/// Save `uids` into the profile `slug`. Returns false when it already held exactly this set,
/// so nothing was written.
pub fn store_favorites(workspace: &Path, slug: &str, uids: &[String]) -> Result<bool> {
    let mut profile = read_profile(workspace, slug)?;
    let mut wanted = uids.to_vec();
    wanted.sort();
    wanted.dedup();
    if favorites_of(&profile).as_ref() == Some(&wanted) {
        return Ok(false);
    }
    set_favorites(&mut profile, &wanted);
    write_profile(workspace, &profile)?;
    Ok(true)
}

/// Read the favourites stored with the profile `slug`. `None` when the profile cannot be read
/// or keeps none.
pub fn stored_favorites(workspace: &Path, slug: &str) -> Option<Vec<String>> {
    favorites_of(&read_profile(workspace, slug).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mod_extras::{Extras, ModExtra};
    use crate::profiles::PROFILE_SCHEMA;

    fn profile(slug: &str) -> Profile {
        Profile {
            schema: PROFILE_SCHEMA,
            name: slug.to_string(),
            slug: slug.to_string(),
            created_at: String::new(),
            updated_at: String::new(),
            slots: Vec::new(),
            crosshair: None,
            autoexec: None,
            author: None,
            extensions: Map::new(),
            isolated: false,
        }
    }

    #[test]
    fn each_profile_keeps_its_own_favourites() {
        let workspace = std::env::temp_dir().join(format!("sigil-profile-favs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&workspace);
        write_profile(&workspace, &profile("ranked")).unwrap();
        write_profile(&workspace, &profile("fun")).unwrap();
        assert_eq!(stored_favorites(&workspace, "ranked"), None, "an old profile keeps none");

        assert!(store_favorites(&workspace, "ranked", &["b".into(), "a".into()]).unwrap());
        assert!(!store_favorites(&workspace, "ranked", &["a".into(), "b".into()]).unwrap());
        assert!(store_favorites(&workspace, "fun", &[]).unwrap());
        assert_eq!(stored_favorites(&workspace, "ranked"), Some(vec!["a".into(), "b".into()]));
        assert_eq!(stored_favorites(&workspace, "fun"), Some(Vec::new()));

        // Applying "fun" then "ranked" puts each set back on the shared flag.
        let mut extras = Extras::default();
        for uid in ["a", "b", "c"] {
            extras.mods.push(ModExtra { uid: uid.into(), ..ModExtra::default() });
        }
        extras.set_favorites(&stored_favorites(&workspace, "ranked").unwrap());
        assert_eq!(extras.favorites(), vec!["a".to_string(), "b".to_string()]);
        extras.set_favorites(&stored_favorites(&workspace, "fun").unwrap());
        assert!(extras.favorites().is_empty());
        let _ = std::fs::remove_dir_all(&workspace);
    }
}
