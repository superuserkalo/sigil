//! Sigil's own notes about installed mods, kept beside the library store (`library.json`).
//!
//! The library store remembers a name, author, preview and GameBanana id per slot. This file
//! remembers the rest the Installed page needs, under the same key (slot number plus the size of
//! the directory VPK):
//!
//! - what GameBanana says about the mod: hero, category, root category, whether it is mature
//!   (filled after install and backfilled for older entries, see [`classify`]);
//! - what the player set: tags, a pin, "always load first", an ignored update, a local preview
//!   image, an NSFW flag, a variant group and label;
//! - named mod lists, which hold mods by a stable `uid` that survives a slot change.
//!
//! App data only: nothing here touches the game folder. Writes are atomic.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::load_order::SlotMove;
use crate::util::write_atomic;

pub const EXTRAS_FILE_NAME: &str = "library-extra.json";
pub const EXTRAS_SCHEMA: u32 = 1;

/// Notes about one installed slot.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModExtra {
    /// Stable id: mod lists point at this, so a mod keeps its lists when its slot changes.
    pub uid: String,
    pub slot: u8,
    /// Size of the directory VPK when this was written. Together with `slot` it is the key.
    pub size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gamebanana_id: Option<u64>,
    /// GameBanana's metadata was read for this entry (even when it had none).
    pub fetched: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hero: Option<String>,
    /// Leaf category, for example `Paradox` or `HUD`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Top category, for example `Skins`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_category: Option<String>,
    /// GameBanana marks the mod mature.
    pub remote_nsfw: bool,
    /// File names on the mod page when the metadata was read, newest first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub remote_files: Vec<String>,
    /// The player's NSFW flag. `None` follows GameBanana.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nsfw: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// A preview image the player picked, copied into Sigil's data folder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<PathBuf>,
    pub pinned: bool,
    /// Kept at the lowest slot numbers, so it wins every overlap.
    pub always_first: bool,
    /// A GameBanana file id the player chose not to update to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignored_update: Option<u64>,
    /// Cards with the same group show as one card with a variant picker. Slots of one
    /// GameBanana mod group by themselves; this joins other mods by hand.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant_group: Option<String>,
    /// The name of this file in its variant picker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant_label: Option<String>,
    /// The archive file name it was installed from, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
}

impl ModExtra {
    /// The NSFW flag the player sees: their own, else GameBanana's.
    pub fn is_nsfw(&self) -> bool {
        self.nsfw.unwrap_or(self.remote_nsfw)
    }
}

/// A named set of mods the player turns on or off together.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModList {
    pub id: String,
    pub name: String,
    /// `ModExtra::uid` of each member.
    pub members: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Extras {
    pub schema: u32,
    pub mods: Vec<ModExtra>,
    pub lists: Vec<ModList>,
}

impl Default for Extras {
    fn default() -> Self {
        Extras {
            schema: EXTRAS_SCHEMA,
            mods: Vec::new(),
            lists: Vec::new(),
        }
    }
}

/// One slot on disk, as [`Extras::sync`] needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotKey {
    pub slot: u8,
    pub size_bytes: u64,
    pub gamebanana_id: Option<u64>,
}

impl Extras {
    pub fn find(&self, slot: u8, size_bytes: u64) -> Option<&ModExtra> {
        self.mods
            .iter()
            .rev()
            .find(|m| m.slot == slot && m.size_bytes == size_bytes)
    }

    pub fn find_uid(&self, uid: &str) -> Option<&ModExtra> {
        self.mods.iter().find(|m| m.uid == uid)
    }

    /// The newest entry for a slot, whatever its size. Used where the size is not at hand.
    pub fn find_slot(&self, slot: u8) -> Option<&ModExtra> {
        self.mods.iter().rev().find(|m| m.slot == slot)
    }

    /// The entry a slot has, or a blank one. Reads, for drawing a card.
    pub fn notes(&self, key: SlotKey) -> ModExtra {
        self.find(key.slot, key.size_bytes).cloned().unwrap_or(ModExtra {
            slot: key.slot,
            size_bytes: key.size_bytes,
            gamebanana_id: key.gamebanana_id,
            ..ModExtra::default()
        })
    }

    /// The slot a uid sits in now, after reorders and updates.
    pub fn slot_for_uid(&self, uid: &str) -> Option<u8> {
        self.find_uid(uid).map(|entry| entry.slot)
    }

    /// The uid of the entry for a slot, if it has one.
    pub fn uid_of(&self, slot: u8, size_bytes: u64) -> Option<String> {
        self.find(slot, size_bytes).map(|entry| entry.uid.clone())
    }

    /// The entry for a slot, made when there is none.
    pub fn entry(&mut self, key: SlotKey) -> &mut ModExtra {
        let at = self
            .mods
            .iter()
            .rposition(|m| m.slot == key.slot && m.size_bytes == key.size_bytes);
        let at = match at {
            Some(at) => at,
            None => {
                self.mods.push(ModExtra {
                    uid: new_uid(),
                    slot: key.slot,
                    size_bytes: key.size_bytes,
                    gamebanana_id: key.gamebanana_id,
                    ..Default::default()
                });
                self.mods.len() - 1
            }
        };
        let entry = &mut self.mods[at];
        if entry.gamebanana_id.is_none() {
            entry.gamebanana_id = key.gamebanana_id;
        }
        entry
    }

    /// Follow the slots on disk. An entry whose slot holds a file of another size but the same
    /// GameBanana mod (an update, or another tool rewrote it) moves to the new size. Entries for
    /// slots that are gone stay, so a restore from the trash finds them again. Returns whether
    /// anything changed.
    pub fn sync(&mut self, on_disk: &[SlotKey]) -> bool {
        let mut changed = false;
        for key in on_disk {
            if self.find(key.slot, key.size_bytes).is_some() {
                continue;
            }
            let stale = self.mods.iter().rposition(|m| {
                m.slot == key.slot
                    && key.gamebanana_id.is_some()
                    && m.gamebanana_id == key.gamebanana_id
            });
            if let Some(at) = stale {
                self.mods[at].size_bytes = key.size_bytes;
                changed = true;
            }
        }
        changed
    }

    /// Make sure every slot on disk has an entry with a uid, so tags, pins and lists can point
    /// at it. Returns whether anything changed.
    pub fn ensure(&mut self, on_disk: &[SlotKey]) -> bool {
        let before = self.mods.len();
        for key in on_disk {
            if self.find(key.slot, key.size_bytes).is_none()
                && self
                    .mods
                    .iter()
                    .rev()
                    .find(|m| m.slot == key.slot)
                    .is_none()
            {
                self.entry(*key);
            }
        }
        self.mods.len() != before
    }

    /// Entries follow their slot to its new number after a reorder.
    pub fn remap(&mut self, moves: &[SlotMove]) {
        let map: BTreeMap<u8, u8> = moves.iter().map(|m| (m.from, m.to)).collect();
        for entry in &mut self.mods {
            if let Some(&to) = map.get(&entry.slot) {
                entry.slot = to;
            }
        }
    }

    /// Move one entry to another key (an update wrote a new file into the slot).
    pub fn rekey(&mut self, slot: u8, old_size: u64, new_size: u64) {
        if let Some(entry) = self
            .mods
            .iter_mut()
            .rev()
            .find(|m| m.slot == slot && m.size_bytes == old_size)
        {
            entry.size_bytes = new_size;
        }
    }

    /// Every tag in use, sorted, without duplicates (case kept from the first use).
    pub fn all_tags(&self) -> Vec<String> {
        let mut seen = BTreeSet::new();
        let mut tags = Vec::new();
        for tag in self.mods.iter().flat_map(|m| m.tags.iter()) {
            if seen.insert(tag.to_lowercase()) {
                tags.push(tag.clone());
            }
        }
        tags.sort_by_key(|t| t.to_lowercase());
        tags
    }

    pub fn list(&self, id: &str) -> Option<&ModList> {
        self.lists.iter().find(|l| l.id == id)
    }

    /// Make a list. A name in use gets a number after it.
    pub fn add_list(&mut self, name: &str, members: Vec<String>) -> String {
        let base = name.trim();
        let base = if base.is_empty() { "List" } else { base };
        let mut label = base.to_string();
        let mut n = 2;
        while self.lists.iter().any(|l| l.name.eq_ignore_ascii_case(&label)) {
            label = format!("{base} {n}");
            n += 1;
        }
        let id = new_uid();
        let mut members = members;
        dedup_keep_order(&mut members);
        self.lists.push(ModList {
            id: id.clone(),
            name: label,
            members,
        });
        id
    }

    pub fn remove_list(&mut self, id: &str) {
        self.lists.retain(|l| l.id != id);
    }

    /// Add or remove one mod in a list.
    pub fn toggle_member(&mut self, list: &str, uid: &str) {
        if let Some(l) = self.lists.iter_mut().find(|l| l.id == list) {
            if let Some(at) = l.members.iter().position(|m| m == uid) {
                l.members.remove(at);
            } else {
                l.members.push(uid.to_string());
            }
        }
    }
}

fn dedup_keep_order(items: &mut Vec<String>) {
    let mut seen = BTreeSet::new();
    items.retain(|item| seen.insert(item.clone()));
}

/// The card group a slot belongs to in the variant picker: the player's own group name when
/// they set one, else the GameBanana mod, which is how several files of one mod group by
/// themselves. `None` means the slot is a card of its own.
pub fn variant_key(extra: Option<&ModExtra>, gamebanana_id: Option<u64>) -> Option<String> {
    if let Some(group) = extra.and_then(|entry| entry.variant_group.as_ref())
        && !group.trim().is_empty()
    {
        return Some(format!("local:{}", group.trim().to_lowercase()));
    }
    gamebanana_id
        .or_else(|| extra.and_then(|entry| entry.gamebanana_id))
        .map(|id| format!("gb:{id}"))
}

/// The label one file gets in its variant picker: the player's label, else the archive file
/// name, else the slot.
pub fn variant_label(extra: Option<&ModExtra>, file_name: &str, slot: u8) -> String {
    extra
        .and_then(|entry| entry.variant_label.clone())
        .filter(|label| !label.trim().is_empty())
        .or_else(|| {
            extra
                .and_then(|entry| entry.file_name.clone())
                .filter(|name| !name.trim().is_empty())
        })
        .map(|label| label.trim().to_string())
        .unwrap_or_else(|| {
            let stem = file_name
                .strip_suffix(crate::addons::DISABLED_SUFFIX)
                .unwrap_or(file_name);
            if stem.eq_ignore_ascii_case(&format!("pak{slot:02}_dir.vpk")) {
                format!("File {slot:02}")
            } else {
                stem.to_string()
            }
        })
}

/// Add a tag (trimmed, at most 32 characters) unless the entry has it in any case.
pub fn add_tag(entry: &mut ModExtra, tag: &str) -> bool {
    let tag: String = tag.trim().chars().take(32).collect();
    if tag.is_empty() || entry.tags.iter().any(|t| t.eq_ignore_ascii_case(&tag)) {
        return false;
    }
    entry.tags.push(tag);
    true
}

/// Tags typed as one line: split on commas.
pub fn parse_tags(line: &str) -> Vec<String> {
    let mut entry = ModExtra::default();
    for part in line.split(',') {
        add_tag(&mut entry, part);
    }
    entry.tags
}

pub fn extras_path(data_dir: &Path) -> PathBuf {
    data_dir.join(EXTRAS_FILE_NAME)
}

/// Read the store. A missing file is an empty store.
pub fn read_extras(path: &Path) -> Result<Extras> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|error| Error::msg(format!("{} is not a Sigil extras file: {error}", path.display()))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Extras::default()),
        Err(error) => Err(error.into()),
    }
}

pub fn write_extras(path: &Path, extras: &Extras) -> Result<()> {
    let text = serde_json::to_vec_pretty(extras)?;
    write_atomic(path, &text)
}

/// Read, change and write the store in one step.
pub fn edit_extras<T>(path: &Path, f: impl FnOnce(&mut Extras) -> T) -> Result<T> {
    let mut extras = read_extras(path).unwrap_or_default();
    let out = f(&mut extras);
    write_extras(path, &extras)?;
    Ok(out)
}

/// A random 16 character id.
pub fn new_uid() -> String {
    let mut bytes = [0u8; 8];
    if getrandom::fill(&mut bytes).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        bytes = (nanos as u64 ^ u64::from(std::process::id())).to_le_bytes();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// What GameBanana's categories mean for the filters. `category` is the leaf (`_aCategory`),
/// `super_category` its parent (`_aSuperCategory`). Under Skins the leaf is a hero; a sound or
/// model under a hero name keeps the hero when `heroes` (names seen under Skins) has it.
/// Returns `(hero, category, root)`.
pub fn classify(
    category: &str,
    super_category: &str,
    heroes: &BTreeSet<String>,
) -> (Option<String>, Option<String>, Option<String>) {
    let leaf = category.trim();
    let parent = super_category.trim();
    let some = |s: &str| (!s.is_empty()).then(|| s.to_string());
    if parent.eq_ignore_ascii_case("skins") || parent.eq_ignore_ascii_case("hero skins") {
        return (some(leaf), some(leaf), some(parent));
    }
    let hero = heroes
        .iter()
        .find(|h| h.eq_ignore_ascii_case(leaf))
        .cloned();
    if parent.is_empty() {
        (hero, some(leaf), some(leaf))
    } else {
        (hero, some(leaf), some(parent))
    }
}

/// Hero names the store already knows from Skins entries.
pub fn known_heroes(extras: &Extras) -> BTreeSet<String> {
    extras
        .mods
        .iter()
        .filter(|m| {
            m.root_category
                .as_deref()
                .is_some_and(|r| r.eq_ignore_ascii_case("skins"))
        })
        .filter_map(|m| m.hero.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../.scratch/extras-test")
            .join(format!("{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn key(slot: u8, size: u64, id: Option<u64>) -> SlotKey {
        SlotKey {
            slot,
            size_bytes: size,
            gamebanana_id: id,
        }
    }

    #[test]
    fn entries_round_trip_and_keep_their_uid() {
        let root = dir("roundtrip");
        let path = extras_path(&root);
        let uid = edit_extras(&path, |x| {
            let e = x.entry(key(3, 100, Some(7)));
            add_tag(e, "favourite");
            add_tag(e, "Favourite");
            e.pinned = true;
            e.uid.clone()
        })
        .unwrap();
        let back = read_extras(&path).unwrap();
        let e = back.find(3, 100).unwrap();
        assert_eq!(e.uid, uid);
        assert_eq!(e.tags, vec!["favourite".to_string()]);
        assert!(e.pinned);
        assert_eq!(e.gamebanana_id, Some(7));
        assert!(read_extras(&root.join("missing.json")).unwrap().mods.is_empty());
    }

    #[test]
    fn remap_and_sync_follow_the_slot() {
        let mut x = Extras::default();
        let uid = x.entry(key(1, 10, Some(5))).uid.clone();
        x.entry(key(2, 20, None));
        x.remap(&[SlotMove { from: 1, to: 4 }, SlotMove { from: 2, to: 1 }]);
        assert_eq!(x.find(4, 10).unwrap().uid, uid);
        assert!(x.find(1, 20).is_some());
        // An update changed the size of slot 4; the same GameBanana mod keeps its notes.
        assert!(x.sync(&[key(4, 99, Some(5)), key(1, 20, None)]));
        assert_eq!(x.find(4, 99).unwrap().uid, uid);
        // Another mod in the slot does not inherit them.
        assert!(!x.sync(&[key(1, 21, None)]));
        x.rekey(4, 99, 100);
        assert!(x.find(4, 100).is_some());
    }

    #[test]
    fn lists_get_unique_names_and_members_toggle() {
        let mut x = Extras::default();
        let a = x.add_list("Comp", vec!["u1".into(), "u1".into()]);
        let b = x.add_list("comp", vec![]);
        assert_eq!(x.list(&a).unwrap().members, vec!["u1".to_string()]);
        assert_eq!(x.list(&b).unwrap().name, "comp 2");
        x.toggle_member(&b, "u2");
        x.toggle_member(&a, "u1");
        assert!(x.list(&a).unwrap().members.is_empty());
        assert_eq!(x.list(&b).unwrap().members, vec!["u2".to_string()]);
        x.remove_list(&a);
        assert!(x.list(&a).is_none());
    }

    #[test]
    fn classify_reads_heroes_from_skins() {
        let none = BTreeSet::new();
        assert_eq!(
            classify("Paradox", "Skins", &none),
            (Some("Paradox".into()), Some("Paradox".into()), Some("Skins".into()))
        );
        assert_eq!(classify("HUD", "", &none), (None, Some("HUD".into()), Some("HUD".into())));
        let heroes: BTreeSet<String> = ["Paradox".to_string()].into();
        assert_eq!(classify("paradox", "Sounds", &heroes).0, Some("Paradox".into()));
        assert_eq!(classify("Music", "Sounds", &heroes).0, None);
    }

    #[test]
    fn tags_parse_from_a_line() {
        assert_eq!(parse_tags(" comp, ranked ,,Comp"), vec!["comp".to_string(), "ranked".to_string()]);
        let mut e = ModExtra::default();
        assert!(!add_tag(&mut e, "   "));
        e.nsfw = None;
        e.remote_nsfw = true;
        assert!(e.is_nsfw());
        e.nsfw = Some(false);
        assert!(!e.is_nsfw());
    }
}
