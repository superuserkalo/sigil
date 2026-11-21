//! What local mod files say about themselves before they are imported: the Add mods dialog.
//!
//! - Deadlock Mod Manager (behaviour only, GPL): adding custom mods reads the dropped VPKs to
//!   suggest a category, says what each file holds, finds a map's console name and shows a
//!   cover; several loose VPKs become one custom mod whose parts switch in Manage files, and
//!   unrelated files can still be added as separate mods.
//! - Grimoire: names each member of a pack from its own imprint (already in [`super::local_mods`]).
//! - Sigil: [`read_part`] reads one directory VPK's tree (no file bodies) into a
//!   [`PartContents`]; [`suggest_category`], [`map_name`] and [`find_cover`] answer for the
//!   whole pick; [`default_as_one`], [`group_name`] and [`part_labels`] decide how several files
//!   become one mod. The parts model is the Installed page's own variant group: every part gets
//!   one `variant_group` and its own `variant_label` ([`write_parts`]), so the card shows one mod
//!   and Manage files switches each part.
//!
//! Read only, except [`write_parts`], which writes Sigil's own notes file in app data. Nothing
//! here writes to a game install or contacts the network.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::game_assets::{AssetThumbnail, TextureKind, classify_game_texture, decode_thumbnail};
use crate::locker::{LockerAssetPaths, LockerHero, hero_for_codename, hero_of_path, resolve_render, roster_hero};
use crate::mod_extras::{SlotKey, edit_extras};
use crate::vpk::{ReadVpkOptions, read_vpk_directory_with, read_vpk_entry_data};

use super::ForeignMod;

/// What one directory VPK changes, counted from its file list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartContents {
    pub file: PathBuf,
    /// False when the file could not be read as a VPK.
    pub readable: bool,
    /// Files in the VPK.
    pub entries: usize,
    /// Roster names of the heroes whose model or material folders the part replaces: a skin.
    pub skin_heroes: Vec<String>,
    /// Roster names of every hero any path names (sounds and particles included).
    pub heroes: Vec<String>,
    pub models: u32,
    pub particles: u32,
    pub textures: u32,
    pub ability_sounds: u32,
    pub voice_lines: u32,
    pub weapon_sounds: u32,
    pub other_sounds: u32,
    pub hud_files: u32,
    /// Console names of the maps the part ships (`maps/<name>.vpk`), never a hero-select scene.
    pub map_names: Vec<String>,
    /// The best hero card texture inside the part, for a cover.
    pub card_art: Option<String>,
}

impl PartContents {
    /// Count one game path. Case does not matter and either slash works.
    pub fn count(&mut self, path: &str) {
        let lower = path.to_ascii_lowercase().replace('\\', "/");
        self.entries += 1;
        if lower.ends_with(".vmdl_c") || lower.ends_with(".vmdl") {
            self.models += 1;
        } else if lower.ends_with(".vpcf_c") || lower.ends_with(".vpcf") {
            self.particles += 1;
        } else if lower.ends_with(".vtex_c") || lower.ends_with(".vtex") {
            self.textures += 1;
            let texture = classify_game_texture(&lower);
            if texture.kind == TextureKind::HeroCard && better_card(self.card_art.as_deref(), &lower) {
                self.card_art = Some(path.replace('\\', "/"));
            }
        } else if lower.ends_with(".vsnd_c") || lower.ends_with(".vsnd") {
            let bucket = if lower.starts_with("sounds/abilities/") {
                &mut self.ability_sounds
            } else if lower.starts_with("sounds/vo/") || lower.starts_with("sounds/voice/") {
                &mut self.voice_lines
            } else if lower.starts_with("sounds/weapons/") {
                &mut self.weapon_sounds
            } else {
                &mut self.other_sounds
            };
            *bucket += 1;
        }
        if lower.starts_with("panorama/") {
            self.hud_files += 1;
        }
        // `sounds/vo/<codename>/` and `sounds/abilities/<codename>/` name a hero without a
        // `heroes` folder.
        let sound_hero = lower
            .strip_prefix("sounds/")
            .and_then(|rest| rest.split('/').nth(1).filter(|_| rest.matches('/').count() >= 2))
            .and_then(hero_for_codename);
        if let Some(hero) = hero_of_path(&lower).or(sound_hero).map(|hero| hero.name.to_string())
            && !self.heroes.contains(&hero)
        {
            self.heroes.push(hero);
        }
        for name in crate::maps::map_names_in_paths([path]) {
            if !self.map_names.iter().any(|have| have.eq_ignore_ascii_case(&name)) {
                self.map_names.push(name);
            }
        }
    }

    /// Sounds of every kind.
    pub fn sounds(&self) -> u32 {
        self.ability_sounds + self.voice_lines + self.weapon_sounds + self.other_sounds
    }

    /// The one hero the part is about, when it names exactly one: a skin's hero first.
    pub fn hero(&self) -> Option<&str> {
        match (self.skin_heroes.as_slice(), self.heroes.as_slice()) {
            ([one], _) => Some(one),
            ([], [one]) => Some(one),
            _ => None,
        }
    }
}

/// `card` beats `vertical` beats the critical and gloat cards; the first of a kind stays.
fn better_card(current: Option<&str>, candidate: &str) -> bool {
    let rank = |path: &str| {
        let texture = classify_game_texture(path);
        match texture.variant.as_deref() {
            Some("card") => 0,
            Some("vertical") => 1,
            _ => 2,
        }
    };
    match current {
        None => true,
        Some(current) => rank(candidate) < rank(&current.to_ascii_lowercase()),
    }
}

/// Read one directory VPK's tree. A file that is not a VPK comes back with `readable` false.
pub fn read_part(path: &Path) -> PartContents {
    let mut part = PartContents {
        file: path.to_path_buf(),
        ..PartContents::default()
    };
    let Ok(directory) = read_vpk_directory_with(path, &ReadVpkOptions { keep_preload: false }) else {
        return part;
    };
    part.readable = true;
    for entry in &directory.entries {
        part.count(&entry.path);
    }
    part.skin_heroes = crate::mod_heroes::model_heroes(directory.entries.iter().map(|e| e.path.as_str()));
    part.heroes.sort();
    part
}

// ---------------------------------------------------------------------------
// Category
// ---------------------------------------------------------------------------

/// The kinds a local mod can be filed as. Each maps onto the GameBanana names the Installed
/// filters already read (`root_category`, `category`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LocalCategory {
    Skin,
    Map,
    Hud,
    VoiceLines,
    AbilitySounds,
    WeaponSounds,
    Sounds,
    Other,
}

impl LocalCategory {
    pub const ALL: [LocalCategory; 8] = [
        LocalCategory::Skin,
        LocalCategory::Map,
        LocalCategory::Hud,
        LocalCategory::VoiceLines,
        LocalCategory::AbilitySounds,
        LocalCategory::WeaponSounds,
        LocalCategory::Sounds,
        LocalCategory::Other,
    ];

    /// English label; the UI passes it through its translation table.
    pub fn label(self) -> &'static str {
        match self {
            LocalCategory::Skin => "Skin",
            LocalCategory::Map => "Map",
            LocalCategory::Hud => "HUD",
            LocalCategory::VoiceLines => "Voice lines",
            LocalCategory::AbilitySounds => "Ability sounds",
            LocalCategory::WeaponSounds => "Weapon sounds",
            LocalCategory::Sounds => "Other sounds",
            LocalCategory::Other => "Other",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            LocalCategory::Skin => "skin",
            LocalCategory::Map => "map",
            LocalCategory::Hud => "hud",
            LocalCategory::VoiceLines => "voice",
            LocalCategory::AbilitySounds => "ability-sounds",
            LocalCategory::WeaponSounds => "weapon-sounds",
            LocalCategory::Sounds => "sounds",
            LocalCategory::Other => "other",
        }
    }

    /// `(root_category, category)` as GameBanana names them. A skin's leaf is its hero.
    pub fn gamebanana(self, hero: Option<&str>) -> (&'static str, Option<String>) {
        match self {
            LocalCategory::Skin => ("Skins", hero.map(str::to_string)),
            LocalCategory::Map => ("Maps", None),
            LocalCategory::Hud => ("HUD", None),
            LocalCategory::VoiceLines => ("Sound", Some("VOs".to_string())),
            LocalCategory::AbilitySounds => ("Sound", Some("Abilities".to_string())),
            LocalCategory::WeaponSounds => ("Sound", Some("Weapons".to_string())),
            LocalCategory::Sounds => ("Sound", Some("Other/Misc".to_string())),
            LocalCategory::Other => ("Other/Misc", None),
        }
    }
}

/// The category the files point to, or `None` when they could be several. A map wins (a map
/// mod carries models and sounds of its own), then a skin; models or particles that name no
/// hero (items, effects, props) fit too many categories to guess. Among sounds, one kind must
/// outnumber the uncategorised ones; a part with only Panorama files is a HUD.
pub fn suggest_category(parts: &[PartContents]) -> Option<LocalCategory> {
    let readable: Vec<&PartContents> = parts.iter().filter(|part| part.readable).collect();
    if readable.is_empty() {
        return None;
    }
    if readable.iter().any(|part| !part.map_names.is_empty()) {
        return Some(LocalCategory::Map);
    }
    if readable.iter().any(|part| !part.skin_heroes.is_empty()) {
        return Some(LocalCategory::Skin);
    }
    let total = |count: fn(&PartContents) -> u32| readable.iter().map(|part| count(part)).sum::<u32>();
    if total(|p| p.models + p.particles) > 0 {
        return None;
    }
    let kinds = [
        (LocalCategory::VoiceLines, total(|p| p.voice_lines)),
        (LocalCategory::AbilitySounds, total(|p| p.ability_sounds)),
        (LocalCategory::WeaponSounds, total(|p| p.weapon_sounds)),
    ];
    let other = total(|p| p.other_sounds);
    let (best, count) = kinds
        .iter()
        .copied()
        .fold((LocalCategory::Sounds, 0), |best, kind| if kind.1 > best.1 { kind } else { best });
    if count == 0 && other == 0 {
        return (total(|p| p.hud_files) > 0).then_some(LocalCategory::Hud);
    }
    if count > other {
        Some(best)
    } else if count == 0 {
        Some(LocalCategory::Sounds)
    } else {
        None
    }
}

/// The hero every readable part agrees on, when there is one.
pub fn common_hero(parts: &[PartContents]) -> Option<String> {
    let heroes: BTreeSet<&str> = parts
        .iter()
        .filter(|part| part.readable)
        .filter_map(PartContents::hero)
        .collect();
    (heroes.len() == 1).then(|| heroes.into_iter().next().unwrap_or_default().to_string())
}

/// The first map name any part ships: what `map <name>` loads.
pub fn map_name(parts: &[PartContents]) -> Option<String> {
    parts.iter().flat_map(|part| part.map_names.iter()).next().cloned()
}

// ---------------------------------------------------------------------------
// Cover
// ---------------------------------------------------------------------------

/// Where a cover can come from, best first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoverSource {
    /// A picture shipped beside the VPK, as GameBanana archives often carry.
    File(PathBuf),
    /// A hero card the part itself replaces: the skin's own card art.
    Texture { vpk: PathBuf, entry: String },
    /// The hero's render that ships with Sigil, for a skin with no art of its own.
    Render(PathBuf),
}

const IMAGE_EXTENSIONS: [&str; 4] = ["png", "jpg", "jpeg", "webp"];
const COVER_WORDS: [&str; 7] = ["cover", "preview", "thumbnail", "thumb", "screenshot", "banner", "icon"];

/// A picture beside `vpk`: one named like the VPK (`neon_haze_dir.png`, `neon_haze.jpg`), one
/// named like a cover (`preview.png`), or the only picture in the folder. A folder with many
/// unrelated pictures (Downloads) gives none.
pub fn image_beside(vpk: &Path) -> Option<PathBuf> {
    let folder = vpk.parent()?;
    let stem = vpk
        .file_name()?
        .to_string_lossy()
        .to_ascii_lowercase()
        .trim_end_matches(".disabled")
        .trim_end_matches(".vpk")
        .to_string();
    let bare = stem.trim_end_matches("_dir").to_string();
    let mut pictures: Vec<PathBuf> = fs::read_dir(folder)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| IMAGE_EXTENSIONS.iter().any(|known| ext.eq_ignore_ascii_case(known)))
        })
        .collect();
    pictures.sort();
    let stem_of = |path: &Path| {
        path.file_stem()
            .map(|stem| stem.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default()
    };
    if let Some(named) = pictures
        .iter()
        .find(|path| {
            let picture = stem_of(path);
            picture == stem || picture == bare
        })
    {
        return Some(named.clone());
    }
    if let Some(cover) = pictures
        .iter()
        .find(|path| COVER_WORDS.iter().any(|word| stem_of(path).contains(word)))
    {
        return Some(cover.clone());
    }
    (pictures.len() == 1).then(|| pictures.remove(0))
}

/// The best cover for a pick: a picture beside one of the files, else a hero card one of the
/// parts replaces, else the bundled render of the one hero the skin is for.
pub fn find_cover(parts: &[PartContents], assets: &LockerAssetPaths) -> Option<CoverSource> {
    if let Some(file) = parts.iter().find_map(|part| image_beside(&part.file)) {
        return Some(CoverSource::File(file));
    }
    if let Some(part) = parts.iter().find(|part| part.card_art.is_some()) {
        return Some(CoverSource::Texture {
            vpk: part.file.clone(),
            entry: part.card_art.clone().unwrap_or_default(),
        });
    }
    let skins: BTreeSet<&str> = parts
        .iter()
        .flat_map(|part| part.skin_heroes.iter().map(String::as_str))
        .collect();
    let hero = (skins.len() == 1).then(|| skins.into_iter().next()).flatten()?;
    let roster = roster_hero(hero)?;
    let locker_hero = LockerHero {
        codename: roster.codenames.first().copied().unwrap_or_default().to_string(),
        display_name: roster.name.to_string(),
        render_path: None,
        name_art_path: None,
        mod_count: 0,
        asset_count: 0,
        in_catalog: true,
        recolor_paths: Vec::new(),
    };
    resolve_render(&locker_hero, assets).map(CoverSource::Render)
}

/// Decode a hero card texture inside a VPK to a preview no wider than `max_edge`.
pub fn decode_texture(vpk: &Path, entry: &str, max_edge: u32) -> Result<AssetThumbnail> {
    let directory = read_vpk_directory_with(vpk, &ReadVpkOptions { keep_preload: false })?;
    let found = directory
        .entries
        .iter()
        .find(|candidate| candidate.path.eq_ignore_ascii_case(entry))
        .ok_or_else(|| crate::error::Error::msg(format!("{entry} is not in {}.", vpk.display())))?;
    let bytes = read_vpk_entry_data(&directory, found)?;
    decode_thumbnail(&bytes, max_edge)
}

// ---------------------------------------------------------------------------
// One mod with parts
// ---------------------------------------------------------------------------

/// Folder names that say nothing about a mod.
const GENERIC_FOLDERS: [&str; 10] = [
    "downloads", "desktop", "documents", "addons", "citadel", "game", "mods", "files", "tmp", "temp",
];

/// Whether several picked files should start as one mod with parts: two or more loose VPKs that
/// sit in one folder and do not name two different GameBanana mods. The player can still split
/// them in the dialog.
pub fn default_as_one(mods: &[&ForeignMod]) -> bool {
    let loose: Vec<&&ForeignMod> = mods.iter().filter(|m| !m.archive).collect();
    if loose.len() < 2 || loose.len() != mods.len() {
        return false;
    }
    let folders: BTreeSet<Option<&Path>> = loose.iter().map(|m| m.file.parent()).collect();
    let ids: BTreeSet<u64> = loose.iter().filter_map(|m| m.gamebanana_id).collect();
    folders.len() == 1 && ids.len() <= 1
}

/// The name of a mod made of these files: the folder they share when it is not a generic one,
/// else a title they all carry, else the first file's name.
pub fn group_name(mods: &[&ForeignMod]) -> String {
    let folders: BTreeSet<Option<&Path>> = mods.iter().map(|m| m.file.parent()).collect();
    if folders.len() == 1
        && let Some(Some(folder)) = folders.into_iter().next()
        && let Some(name) = folder.file_name().map(|name| name.to_string_lossy().into_owned())
        && !GENERIC_FOLDERS.contains(&name.to_ascii_lowercase().as_str())
        && !name.trim().is_empty()
    {
        return tidy(&name);
    }
    let names: BTreeSet<&str> = mods.iter().map(|m| m.name.as_str()).collect();
    if names.len() == 1
        && let Some(name) = names.into_iter().next()
    {
        return name.to_string();
    }
    mods.first().map(|m| m.name.clone()).unwrap_or_default()
}

/// The label each part gets in Manage files: its own name, or its file name when two parts
/// would read the same.
pub fn part_labels(mods: &[&ForeignMod]) -> Vec<String> {
    let names: Vec<&str> = mods.iter().map(|m| m.name.as_str()).collect();
    let unique = names.iter().collect::<BTreeSet<_>>().len() == names.len();
    mods.iter()
        .map(|m| {
            if unique {
                m.name.clone()
            } else {
                m.file
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .map(|name| tidy(name.trim_end_matches(".vpk").trim_end_matches("_dir")))
                    .unwrap_or_else(|| m.name.clone())
            }
        })
        .collect()
}

/// `neon_haze-pack` reads "Neon haze pack"; a name with capitals or spaces stays as typed.
fn tidy(name: &str) -> String {
    if name.contains(' ') || name.chars().any(|c| c.is_ascii_uppercase()) {
        return name.trim().to_string();
    }
    let spaced: String = name
        .chars()
        .map(|c| if c == '_' || c == '-' { ' ' } else { c })
        .collect();
    let words = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => name.to_string(),
    }
}

/// What the Add mods dialog records about one imported slot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PartWrite {
    pub slot: u16,
    /// Size of the directory VPK as it now sits in the slot.
    pub size_bytes: u64,
    pub gamebanana_id: Option<u64>,
    /// The part's name in Manage files, for a mod with parts.
    pub label: Option<String>,
    pub category: Option<LocalCategory>,
    pub hero: Option<String>,
    /// A cover already copied into Sigil's data folder.
    pub image: Option<PathBuf>,
}

/// Record the dialog's answers in Sigil's notes (`library-extra.json`). With `group`, every
/// slot joins one variant group, so the Installed page shows one card whose parts switch in
/// Manage files. A slot GameBanana already described keeps GameBanana's category. App data
/// only.
pub fn write_parts(extras: &Path, group: Option<&str>, writes: &[PartWrite]) -> Result<()> {
    edit_extras(extras, |store| {
        for write in writes {
            let entry = store.entry(SlotKey {
                slot: write.slot,
                size_bytes: write.size_bytes,
                gamebanana_id: write.gamebanana_id,
            });
            if let Some(group) = group {
                entry.variant_group = Some(group.to_string());
                entry.variant_label = write.label.clone();
            }
            if let Some(category) = write.category
                && entry.root_category.is_none()
            {
                let (root, leaf) = category.gamebanana(write.hero.as_deref());
                entry.root_category = Some(root.to_string());
                entry.category = leaf;
            }
            if entry.hero.is_none() {
                entry.hero = write.hero.clone();
            }
            if write.image.is_some() && entry.image.is_none() {
                entry.image = write.image.clone();
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(paths: &[&str]) -> PartContents {
        let mut part = PartContents {
            readable: true,
            ..PartContents::default()
        };
        for path in paths {
            part.count(path);
        }
        part.skin_heroes = crate::mod_heroes::model_heroes(paths.iter().copied());
        part
    }

    #[test]
    fn a_map_name_comes_from_the_map_vpk_not_a_skins_hero_scene() {
        let p = part(&[
            "maps/ui/hero_prefabs/necro.vpk",
            "maps/street_test.vpk",
            "maps/street_test/entities/lights.vmdl_c",
        ]);
        assert_eq!(p.map_names, ["street_test"]);
        assert_eq!(map_name(&[p.clone()]).as_deref(), Some("street_test"));
        assert_eq!(suggest_category(&[p]), Some(LocalCategory::Map));
    }

    #[test]
    fn a_skin_beats_the_sounds_it_ships_with() {
        let skin = part(&[
            "models/heroes_wip/haze/haze.vmdl_c",
            "materials/models/heroes_wip/haze/haze_body_color_psd.vtex_c",
            "sounds/vo/haze/haze_kill_01.vsnd_c",
        ]);
        assert_eq!(skin.skin_heroes, ["Haze"]);
        assert_eq!(skin.hero(), Some("Haze"));
        assert_eq!(suggest_category(&[skin.clone()]), Some(LocalCategory::Skin));
        assert_eq!(common_hero(&[skin]), Some("Haze".to_string()));
    }

    #[test]
    fn sounds_pick_the_kind_that_outnumbers_the_rest() {
        let voice = part(&["sounds/vo/haze/a.vsnd_c", "sounds/vo/haze/b.vsnd_c", "sounds/misc/c.vsnd_c"]);
        assert_eq!(voice.heroes, ["Haze"]);
        assert_eq!(voice.hero(), Some("Haze"));
        assert_eq!(suggest_category(&[voice]), Some(LocalCategory::VoiceLines));
        let mixed = part(&["sounds/vo/haze/a.vsnd_c", "sounds/misc/b.vsnd_c", "sounds/misc/c.vsnd_c"]);
        assert_eq!(suggest_category(&[mixed]), None);
        let other = part(&["sounds/music/a.vsnd_c"]);
        assert_eq!(suggest_category(&[other]), Some(LocalCategory::Sounds));
        let weapons = part(&["sounds/weapons/shotgun.vsnd_c"]);
        assert_eq!(suggest_category(&[weapons]), Some(LocalCategory::WeaponSounds));
    }

    #[test]
    fn panorama_alone_is_a_hud_and_props_are_not_guessed() {
        let hud = part(&["panorama/layout/hud/hud.vxml_c", "panorama/images/hud/x_png.vtex_c"]);
        assert_eq!(suggest_category(&[hud]), Some(LocalCategory::Hud));
        let props = part(&["models/props/crate.vmdl_c", "sounds/misc/a.vsnd_c"]);
        assert_eq!(suggest_category(&[props]), None);
        assert_eq!(suggest_category(&[]), None);
        assert_eq!(suggest_category(&[PartContents::default()]), None);
    }

    #[test]
    fn categories_map_onto_the_names_installed_filters_read() {
        assert_eq!(LocalCategory::Skin.gamebanana(Some("Haze")), ("Skins", Some("Haze".to_string())));
        assert_eq!(LocalCategory::Map.gamebanana(None), ("Maps", None));
        assert_eq!(LocalCategory::VoiceLines.gamebanana(None).0, "Sound");
        for category in LocalCategory::ALL {
            assert!(!category.label().contains('\u{2014}'));
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir()
            .join("sigil-import-parts-test")
            .join(format!("{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn a_picture_beside_the_vpk_is_found_by_name_then_by_word_then_alone() {
        let root = scratch("beside");
        let vpk = root.join("neon_haze_dir.vpk");
        fs::write(&vpk, b"x").unwrap();
        fs::write(root.join("readme.txt"), b"x").unwrap();
        fs::write(root.join("other.png"), b"x").unwrap();
        assert_eq!(image_beside(&vpk), Some(root.join("other.png")));
        fs::write(root.join("second.jpg"), b"x").unwrap();
        assert_eq!(image_beside(&vpk), None);
        fs::write(root.join("Preview.JPG"), b"x").unwrap();
        assert_eq!(image_beside(&vpk), Some(root.join("Preview.JPG")));
        fs::write(root.join("neon_haze.webp"), b"x").unwrap();
        assert_eq!(image_beside(&vpk), Some(root.join("neon_haze.webp")));
        let _ = fs::remove_dir_all(&root);
    }

    fn foreign(file: &str, name: &str, id: Option<u64>) -> ForeignMod {
        ForeignMod {
            name: name.to_string(),
            file: PathBuf::from(file),
            gamebanana_id: id,
            ..ForeignMod::default()
        }
    }

    #[test]
    fn several_loose_files_in_one_folder_start_as_one_mod() {
        let a = foreign("/m/Neon Haze/body_dir.vpk", "Body", None);
        let b = foreign("/m/Neon Haze/voice_dir.vpk", "Voice", None);
        let c = foreign("/m/Other/thing_dir.vpk", "Thing", None);
        let d = foreign("/m/Neon Haze/x_dir.vpk", "X", Some(9));
        let e = foreign("/m/Neon Haze/y_dir.vpk", "Y", Some(10));
        assert!(default_as_one(&[&a, &b]));
        assert!(!default_as_one(&[&a]));
        assert!(!default_as_one(&[&a, &c]));
        assert!(default_as_one(&[&a, &d]));
        assert!(!default_as_one(&[&d, &e]));
        let mut zip = b.clone();
        zip.archive = true;
        assert!(!default_as_one(&[&a, &zip]));
        assert_eq!(group_name(&[&a, &b]), "Neon Haze");
    }

    #[test]
    fn a_generic_folder_does_not_name_the_mod() {
        let a = foreign("/u/Downloads/neon_dir.vpk", "Neon", None);
        let b = foreign("/u/Downloads/haze_dir.vpk", "Neon", None);
        assert_eq!(group_name(&[&a, &b]), "Neon");
        let c = foreign("/u/Downloads/haze_dir.vpk", "Haze", None);
        assert_eq!(group_name(&[&a, &c]), "Neon");
        let d = foreign("/u/neon_pack/haze_dir.vpk", "Haze", None);
        assert_eq!(group_name(&[&d]), "Neon pack");
    }

    #[test]
    fn parts_with_one_name_are_labelled_by_their_files() {
        let a = foreign("/m/Neon Haze/pak01_dir.vpk", "Neon Haze", None);
        let b = foreign("/m/Neon Haze/pak02_dir.vpk", "Neon Haze", None);
        assert_eq!(part_labels(&[&a, &b]), ["Pak01", "Pak02"]);
        let c = foreign("/m/Neon Haze/voice_dir.vpk", "Voice", None);
        assert_eq!(part_labels(&[&a, &c]), ["Neon Haze", "Voice"]);
    }

    #[test]
    fn write_parts_groups_the_slots_and_keeps_gamebanana_categories() {
        let root = scratch("write");
        let path = root.join("library-extra.json");
        edit_extras(&path, |store| {
            store.entry(SlotKey { slot: 4, size_bytes: 20, gamebanana_id: None }).root_category =
                Some("Sound".to_string());
        })
        .unwrap();
        let writes = [
            PartWrite {
                slot: 3,
                size_bytes: 10,
                label: Some("Body".into()),
                category: Some(LocalCategory::Skin),
                hero: Some("Haze".into()),
                image: Some(root.join("cover.png")),
                ..PartWrite::default()
            },
            PartWrite {
                slot: 4,
                size_bytes: 20,
                label: Some("Voice".into()),
                category: Some(LocalCategory::Skin),
                hero: Some("Haze".into()),
                ..PartWrite::default()
            },
        ];
        write_parts(&path, Some("parts-abc"), &writes).unwrap();
        let store = crate::mod_extras::read_extras(&path).unwrap();
        let body = store.find(3, 10).unwrap();
        assert_eq!(body.variant_group.as_deref(), Some("parts-abc"));
        assert_eq!(body.variant_label.as_deref(), Some("Body"));
        assert_eq!(body.root_category.as_deref(), Some("Skins"));
        assert_eq!(body.category.as_deref(), Some("Haze"));
        assert_eq!(body.image, Some(root.join("cover.png")));
        let voice = store.find(4, 20).unwrap();
        assert_eq!(voice.root_category.as_deref(), Some("Sound"));
        assert_eq!(voice.variant_label.as_deref(), Some("Voice"));
        assert_eq!(
            crate::mod_extras::variant_key(Some(body), None),
            crate::mod_extras::variant_key(Some(voice), None)
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn reads_a_real_vpk_and_finds_its_card_art() {
        let root = scratch("real");
        let staging = root.join("staging");
        for path in [
            "models/heroes_wip/haze/haze.vmdl_c",
            "panorama/images/heroes/haze_card_psd.vtex_c",
            "panorama/images/heroes/haze_vertical_psd.vtex_c",
        ] {
            let target = staging.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, b"data").unwrap();
        }
        let packed = crate::vpk_write::pack_vpk(&crate::vpk_write::PackOptions::new(
            staging,
            root.join("out"),
            "haze",
        ))
        .unwrap();
        let read = read_part(&packed.dir_path);
        assert!(read.readable);
        assert_eq!(read.entries, 3);
        assert_eq!(read.models, 1);
        assert_eq!(read.skin_heroes, ["Haze"]);
        assert_eq!(read.card_art.as_deref(), Some("panorama/images/heroes/haze_card_psd.vtex_c"));
        let cover = find_cover(&[read.clone()], &LockerAssetPaths::default());
        assert_eq!(
            cover,
            Some(CoverSource::Texture {
                vpk: packed.dir_path.clone(),
                entry: "panorama/images/heroes/haze_card_psd.vtex_c".to_string()
            })
        );
        let unreadable = read_part(&root.join("missing_dir.vpk"));
        assert!(!unreadable.readable);
        let _ = fs::remove_dir_all(&root);
    }
}
