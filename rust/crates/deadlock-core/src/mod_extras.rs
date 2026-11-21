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
    pub slot: u16,
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
    /// The player saved this mod. Installed and the Locker read the same flag, and saved mods
    /// sort first in both (Grimoire's shared mod favorites).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub favorite: bool,
    /// More pictures the player added to a local mod, copied into Sigil's data folder. `image`
    /// stays the card thumbnail; "Make mod thumbnail" swaps one of these into it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub gallery: Vec<PathBuf>,
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
    /// One member is turned on at each launch, picked by [`random_picks`]. A store written
    /// before this flag existed reads as off.
    #[serde(default)]
    pub random_each_launch: bool,
    /// The member the last launch's roll turned on, by `ModExtra::uid`, so the next roll
    /// passes over it. Saved by [`record_picks`] once the picks are applied. A store written
    /// before this field existed reads as none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_pick: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Extras {
    pub schema: u32,
    pub mods: Vec<ModExtra>,
    pub lists: Vec<ModList>,
    /// The player's display priority for tags of every kind, highest first, by
    /// [`tag_order_key`]: `custom:<tag>`, `base:<category>`, `hero:<name>`. Tags not listed
    /// follow in the default order. Empty is the default order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tag_order: Vec<String>,
}

impl Default for Extras {
    fn default() -> Self {
        Extras {
            schema: EXTRAS_SCHEMA,
            mods: Vec::new(),
            lists: Vec::new(),
            tag_order: Vec::new(),
        }
    }
}

/// The three kinds of tag the Installed tag menu shows in one list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TagKind {
    /// A tag the player typed.
    Custom,
    /// A GameBanana category (Skins, HUD, Faithful/Vanilla+).
    Base,
    /// A hero.
    Hero,
}

impl TagKind {
    fn prefix(self) -> &'static str {
        match self {
            TagKind::Custom => "custom",
            TagKind::Base => "base",
            TagKind::Hero => "hero",
        }
    }
}

/// The key a tag has in [`Extras::tag_order`]: kind and lowercase label.
pub fn tag_order_key(kind: TagKind, label: &str) -> String {
    format!("{}:{}", kind.prefix(), label.trim().to_lowercase())
}

/// Sort tags by the player's display priority: listed keys first in their saved order, then
/// the rest in the default order (custom tags, then categories, then heroes, each by name).
pub fn sort_tags(order: &[String], tags: &mut [(TagKind, String)]) {
    let rank = |kind: TagKind, label: &str| {
        let key = tag_order_key(kind, label);
        order.iter().position(|known| *known == key).unwrap_or(usize::MAX)
    };
    tags.sort_by(|(ka, la), (kb, lb)| {
        rank(*ka, la)
            .cmp(&rank(*kb, lb))
            .then(ka.cmp(kb))
            .then(la.to_lowercase().cmp(&lb.to_lowercase()))
    });
}

/// Move one tag up (`-1`) or down (`1`) in the shown order and save that whole order as the
/// priority. `shown` is the tags in the order the menu shows them now. Returns the new order
/// keys, or `None` when the tag cannot move that way.
pub fn move_tag(shown: &[(TagKind, String)], tag: (TagKind, &str), step: isize) -> Option<Vec<String>> {
    let at = shown
        .iter()
        .position(|(kind, label)| *kind == tag.0 && label.eq_ignore_ascii_case(tag.1))?;
    let to = at.checked_add_signed(step)?;
    if to >= shown.len() {
        return None;
    }
    let mut keys: Vec<String> = shown.iter().map(|(kind, label)| tag_order_key(*kind, label)).collect();
    keys.swap(at, to);
    Some(keys)
}

/// One slot on disk, as [`Extras::sync`] needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotKey {
    pub slot: u16,
    pub size_bytes: u64,
    pub gamebanana_id: Option<u64>,
}

impl Extras {
    pub fn find(&self, slot: u16, size_bytes: u64) -> Option<&ModExtra> {
        self.mods
            .iter()
            .rev()
            .find(|m| m.slot == slot && m.size_bytes == size_bytes)
    }

    pub fn find_uid(&self, uid: &str) -> Option<&ModExtra> {
        self.mods.iter().find(|m| m.uid == uid)
    }

    /// The newest entry for a slot, whatever its size. Used where the size is not at hand.
    pub fn find_slot(&self, slot: u16) -> Option<&ModExtra> {
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
    pub fn slot_for_uid(&self, uid: &str) -> Option<u16> {
        self.find_uid(uid).map(|entry| entry.slot)
    }

    /// The uid of the entry for a slot, if it has one.
    pub fn uid_of(&self, slot: u16, size_bytes: u64) -> Option<String> {
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
        let map: BTreeMap<u16, u16> = moves.iter().map(|m| (m.from, m.to)).collect();
        for entry in &mut self.mods {
            if let Some(&to) = map.get(&entry.slot) {
                entry.slot = to;
            }
        }
    }

    /// Move one entry to another key (an update wrote a new file into the slot).
    pub fn rekey(&mut self, slot: u16, old_size: u64, new_size: u64) {
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

    /// `ModExtra::uid` of every saved mod, sorted.
    pub fn favorites(&self) -> Vec<String> {
        let mut uids: Vec<String> = self
            .mods
            .iter()
            .filter(|entry| entry.favorite && !entry.uid.is_empty())
            .map(|entry| entry.uid.clone())
            .collect();
        uids.sort();
        uids.dedup();
        uids
    }

    /// Save exactly these mods, by `ModExtra::uid`, and no others. Returns whether anything
    /// changed. A profile switch uses this to bring back its own favorites.
    pub fn set_favorites(&mut self, uids: &[String]) -> bool {
        let wanted: BTreeSet<&str> = uids.iter().map(String::as_str).collect();
        let mut changed = false;
        for entry in &mut self.mods {
            let on = wanted.contains(entry.uid.as_str());
            if entry.favorite != on {
                entry.favorite = on;
                changed = true;
            }
        }
        changed
    }

    /// Is the mod with this uid saved?
    pub fn is_favorite(&self, uid: &str) -> bool {
        !uid.is_empty() && self.find_uid(uid).is_some_and(|entry| entry.favorite)
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
            random_each_launch: false,
            last_pick: None,
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

/// One member to turn on for every list the player marked random.
///
/// Each pair is the list id and the slot key of the picked member. The choice is deterministic
/// for `seed`: the same store, the same slots and the same seed always return the same members,
/// so a test can pin the result. Lists are independent of each other, because the seed is mixed
/// with the list id.
///
/// A list with fewer than two members that have an entry in this store has nothing to choose
/// between and is left out, the same way Deadlock Mod Manager leaves out a pool of one. The roll
/// leaves out the member `is_on` says is switched on now and the member `ModList::last_pick`
/// says was picked last launch, so two launches in a row do not land on the same member, while
/// at least one candidate remains after both: a pool of two where one is on picks the other. When
/// every member is on or was picked last time, the roll uses the pool without `last_pick`, then
/// the whole pool, so a launch always gets a pick.
///
/// Pass the result to [`record_picks`] after the picks are applied and save the store, so the
/// next launch knows what was picked. Entries for slots that were removed stay in the store by
/// design, so the caller checks the pick against the slots on disk before it turns anything on.
pub fn random_picks(
    extras: &Extras,
    seed: u64,
    is_on: impl Fn(&SlotKey) -> bool,
) -> Vec<(String, SlotKey)> {
    let mut picks = Vec::new();
    for list in &extras.lists {
        if !list.random_each_launch {
            continue;
        }
        let mut seen = BTreeSet::new();
        let pool: Vec<(String, SlotKey)> = list
            .members
            .iter()
            .filter(|uid| seen.insert(uid.as_str()))
            .filter_map(|uid| extras.find_uid(uid).map(|entry| (uid.clone(), slot_key(entry))))
            .collect();
        if pool.len() < 2 {
            continue;
        }
        let last_pick = list.last_pick.as_deref();
        let mut candidates: Vec<usize> = (0..pool.len())
            .filter(|&i| !is_on(&pool[i].1) && Some(pool[i].0.as_str()) != last_pick)
            .collect();
        if candidates.is_empty() {
            candidates = (0..pool.len())
                .filter(|&i| Some(pool[i].0.as_str()) != last_pick)
                .collect();
        }
        // A safety net: a pool of two or more always keeps a member after dropping last_pick.
        if candidates.is_empty() {
            candidates = (0..pool.len()).collect();
        }
        let index = pick_index(seed, &list.id, candidates.len());
        let member = &pool[candidates[index]];
        picks.push((list.id.clone(), member.1));
    }
    picks
}

/// Remember the member each list picked, by `ModExtra::uid`, so the next launch passes over
/// it. The caller saves the store after applying the picks. A pick for a list that is gone, or
/// for a slot with no entry in the store, is ignored.
pub fn record_picks(extras: &mut Extras, picks: &[(String, SlotKey)]) {
    for (list_id, key) in picks {
        let Some(uid) = extras.find(key.slot, key.size_bytes).map(|entry| entry.uid.clone())
        else {
            continue;
        };
        if let Some(list) = extras.lists.iter_mut().find(|list| &list.id == list_id) {
            list.last_pick = Some(uid);
        }
    }
}

/// The key a store entry points at.
fn slot_key(entry: &ModExtra) -> SlotKey {
    SlotKey {
        slot: entry.slot,
        size_bytes: entry.size_bytes,
        gamebanana_id: entry.gamebanana_id,
    }
}

/// Which of `count` candidates a list picks for a seed. `count` is at least one at every call.
fn pick_index(seed: u64, group: &str, count: usize) -> usize {
    (split_mix(seed ^ fnv1a(group.as_bytes())) % count as u64) as usize
}

/// FNV-1a: a stable hash for a list id, independent of the standard library's hasher.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// SplitMix64: turns a seed into a well spread number, so neighbouring seeds do not walk
/// through a group in lockstep.
fn split_mix(value: u64) -> u64 {
    let value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = value;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
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
pub fn variant_label(extra: Option<&ModExtra>, file_name: &str, slot: u16) -> String {
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

/// GameBanana's category label with its two known misspellings fixed, as Grimoire's
/// `normalizeCategoryLabel`: `Faithul/Vanilla+` is `Faithful/Vanilla+` and `RatKing Flag` is
/// `Rat King Flag`. Runs of spaces collapse. The category's identity does not change.
pub fn normalize_category_label(label: &str) -> String {
    let cleaned = label.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.eq_ignore_ascii_case("faithul/vanilla+") {
        return "Faithful/Vanilla+".to_string();
    }
    if cleaned.eq_ignore_ascii_case("ratking flag") {
        return "Rat King Flag".to_string();
    }
    cleaned
}

/// The roster name a GameBanana category names, whatever its spelling: `RatKing` is Rat King,
/// `The Doorman` is Doorman, `Mo and Krill` is Mo & Krill. `None` for a category that is not
/// a hero.
pub fn category_hero(label: &str) -> Option<&'static str> {
    let key = crate::locker::hero_asset_key(label);
    if key.is_empty() {
        return None;
    }
    let joined = key.replace('_', "");
    crate::locker::ROSTER
        .iter()
        .find(|hero| {
            let name = crate::locker::hero_asset_key(hero.name);
            name == key || name.replace('_', "") == joined
        })
        .map(|hero| hero.name)
}

/// What GameBanana's categories mean for the filters. `category` is the leaf (`_aCategory`),
/// `super_category` its parent (`_aSuperCategory`). Returns `(hero, category, root)`.
///
/// GameBanana's current schema files hero skins at any depth (Grimoire's
/// `fix/gb-category-drift`): `Skins > Rat King` gives a leaf hero under Skins, and
/// `Skins > Rat King > Faithful/Vanilla+` gives a leaf that is not a hero under a parent that
/// is. Both are a Skins mod for that hero. Hero names follow the roster's spelling, and the
/// misspelled `Faithul/Vanilla+` reads as `Faithful/Vanilla+`. A sound or model under a hero
/// name keeps the hero when the roster or `heroes` (names seen under Skins) has it.
pub fn classify(
    category: &str,
    super_category: &str,
    heroes: &BTreeSet<String>,
) -> (Option<String>, Option<String>, Option<String>) {
    let leaf = normalize_category_label(category);
    let parent = normalize_category_label(super_category);
    let some = |s: &str| (!s.is_empty()).then(|| s.to_string());
    let hero_named = |label: &str| {
        category_hero(label).map(str::to_string).or_else(|| {
            heroes
                .iter()
                .find(|known| known.eq_ignore_ascii_case(label))
                .cloned()
        })
    };
    if parent.eq_ignore_ascii_case("skins") || parent.eq_ignore_ascii_case("hero skins") {
        let hero = hero_named(&leaf).or_else(|| some(&leaf));
        return (hero.clone(), hero.or_else(|| some(&leaf)), some(&parent));
    }
    if let Some(hero) = category_hero(&parent) {
        // A child of a hero category: Faithful/Vanilla+, Rat King Flag, a costume set.
        return (Some(hero.to_string()), some(&leaf), Some("Skins".to_string()));
    }
    let hero = hero_named(&leaf);
    if parent.is_empty() {
        (hero, some(&leaf), some(&leaf))
    } else {
        (hero, some(&leaf), some(&parent))
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

    fn key(slot: u16, size: u64, id: Option<u64>) -> SlotKey {
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
    fn a_random_list_picks_one_installed_member_for_a_seed() {
        let mut x = Extras::default();
        let a = x.entry(key(1, 10, Some(1))).uid.clone();
        let b = x.entry(key(2, 20, Some(2))).uid.clone();
        let c = x.entry(key(3, 30, Some(3))).uid.clone();
        let list = x.add_list("Skins", vec![a.clone(), b.clone(), c.clone()]);
        assert!(!x.list(&list).unwrap().random_each_launch);

        x.lists[0].random_each_launch = true;
        let picks = random_picks(&x, 7, |_| false);
        assert_eq!(picks.len(), 1);
        assert_eq!(picks[0].0, list);
        let all = [key(1, 10, Some(1)), key(2, 20, Some(2)), key(3, 30, Some(3))];
        assert!(all.contains(&picks[0].1));

        // The same seed gives the same pick, every seed gives one of the three members.
        assert_eq!(random_picks(&x, 7, |_| false), picks);
        let slots: BTreeSet<u16> = (0..64)
            .map(|seed| random_picks(&x, seed, |_| false)[0].1.slot)
            .collect();
        assert!(slots.len() > 1, "the seed has to change the pick");
    }

    #[test]
    fn a_pool_of_two_picks_the_member_that_is_off() {
        let mut x = Extras::default();
        let a = x.entry(key(1, 10, None)).uid.clone();
        let b = x.entry(key(2, 20, None)).uid.clone();
        x.add_list("Skins", vec![a.clone(), b.clone()]);
        x.lists[0].random_each_launch = true;
        let on = key(1, 10, None);
        for seed in 0..64 {
            let picks = random_picks(&x, seed, |member| *member == on);
            assert_eq!(picks.len(), 1, "seed {seed}");
            assert_eq!(picks[0].1, key(2, 20, None), "seed {seed}");
        }
        // What is on does not matter: the other member is the pick either way.
        let on = key(2, 20, None);
        for seed in 0..64 {
            let picks = random_picks(&x, seed, |member| *member == on);
            assert_eq!(picks[0].1, key(1, 10, None), "seed {seed}");
        }
    }

    #[test]
    fn the_member_picked_last_launch_is_left_out() {
        let mut x = Extras::default();
        let a = x.entry(key(1, 10, None)).uid.clone();
        let b = x.entry(key(2, 20, None)).uid.clone();
        let c = x.entry(key(3, 30, None)).uid.clone();
        x.add_list("Skins", vec![a.clone(), b.clone(), c.clone()]);
        x.lists[0].random_each_launch = true;
        x.lists[0].last_pick = Some(a);
        let mut slots = BTreeSet::new();
        for seed in 0..64 {
            let picks = random_picks(&x, seed, |_| false);
            assert_eq!(picks.len(), 1, "seed {seed}");
            assert_ne!(picks[0].1, key(1, 10, None), "seed {seed}");
            slots.insert(picks[0].1.slot);
        }
        assert_eq!(slots, BTreeSet::from([2, 3]));
    }

    #[test]
    fn a_pool_with_every_member_on_still_picks_one() {
        let mut x = Extras::default();
        let a = x.entry(key(1, 10, None)).uid.clone();
        let b = x.entry(key(2, 20, None)).uid.clone();
        let c = x.entry(key(3, 30, None)).uid.clone();
        x.add_list("Skins", vec![a.clone(), b.clone(), c.clone()]);
        x.lists[0].random_each_launch = true;
        x.lists[0].last_pick = Some(c);
        let all = [key(1, 10, None), key(2, 20, None), key(3, 30, None)];
        for seed in 0..64 {
            let picks = random_picks(&x, seed, |_| true);
            assert_eq!(picks.len(), 1, "seed {seed}");
            assert!(all.contains(&picks[0].1), "seed {seed}: {:?}", picks[0].1);
            // Everything is on, so the last pick is the only member left out.
            assert_ne!(picks[0].1, key(3, 30, None), "seed {seed}");
        }
    }

    #[test]
    fn record_picks_remembers_the_member_for_the_next_launch() {
        let mut x = Extras::default();
        let a = x.entry(key(1, 10, None)).uid.clone();
        let b = x.entry(key(2, 20, None)).uid.clone();
        x.add_list("Skins", vec![a.clone(), b.clone()]);
        x.lists[0].random_each_launch = true;
        assert_eq!(x.lists[0].last_pick, None);
        let picks = random_picks(&x, 3, |_| false);
        record_picks(&mut x, &picks);
        let picked = x.lists[0].last_pick.clone().unwrap();
        assert!(picked == a || picked == b);
        assert_eq!(picked, x.uid_of(picks[0].1.slot, picks[0].1.size_bytes).unwrap());
        // The same seed now gives the other member.
        let next = random_picks(&x, 3, |_| false);
        assert_ne!(next[0].1, picks[0].1);
        // A pick for a list that is gone, or for a slot with no entry, changes nothing.
        let gone = ("missing".to_string(), key(9, 90, None));
        record_picks(&mut x, &[gone]);
        assert_eq!(x.lists[0].last_pick, Some(picked));
    }

    #[test]
    fn a_random_list_needs_two_entries_and_leaves_other_lists_alone() {
        let mut x = Extras::default();
        let a = x.entry(key(1, 10, None)).uid.clone();
        let r = x.add_list("Rolls", vec![a.clone(), a.clone(), "gone".into()]);
        let plain = x.add_list("Fixed", vec![a.clone()]);
        x.lists[0].random_each_launch = true;
        // Two members listed, one installed: nothing to choose between.
        assert!(random_picks(&x, 1, |_| false).is_empty());

        let b = x.entry(key(2, 20, None)).uid.clone();
        x.lists[0].members.push(b.clone());
        let picks = random_picks(&x, 1, |_| false);
        assert_eq!(picks.len(), 1);
        assert_eq!(picks[0].0, r);
        assert_ne!(picks[0].0, plain);

        // A list that does not roll is never picked.
        x.lists[0].random_each_launch = false;
        assert!(random_picks(&x, 1, |_| false).is_empty());
    }

    #[test]
    fn a_store_without_the_random_flag_reads_it_as_off() {
        let old = r#"{"schema":1,"mods":[],"lists":[{"id":"l1","name":"Skins","members":[]}]}"#;
        let read: Extras = serde_json::from_str(old).unwrap();
        assert!(!read.lists[0].random_each_launch);

        let mut written = read;
        written.lists[0].random_each_launch = true;
        let text = serde_json::to_string(&written).unwrap();
        assert!(text.contains(r#""randomEachLaunch":true"#), "{text}");
        let back: Extras = serde_json::from_str(&text).unwrap();
        assert!(back.lists[0].random_each_launch);
    }

    #[test]
    fn a_last_pick_round_trips_and_an_old_store_reads_none() {
        let old = r#"{"schema":1,"mods":[],"lists":[{"id":"l1","name":"Skins","members":[],"randomEachLaunch":true}]}"#;
        let read: Extras = serde_json::from_str(old).unwrap();
        assert_eq!(read.lists[0].last_pick, None);

        let mut written = read;
        written.lists[0].last_pick = Some("u1".into());
        let text = serde_json::to_string(&written).unwrap();
        assert!(text.contains(r#""lastPick":"u1""#), "{text}");
        let back: Extras = serde_json::from_str(&text).unwrap();
        assert_eq!(back.lists[0].last_pick.as_deref(), Some("u1"));
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
    fn classify_follows_gamebananas_current_schema() {
        let none = BTreeSet::new();
        // A child of a hero category is that hero's skin.
        assert_eq!(
            classify("Faithul/Vanilla+", "Rat King", &none),
            (Some("Rat King".into()), Some("Faithful/Vanilla+".into()), Some("Skins".into()))
        );
        assert_eq!(
            classify("RatKing Flag", "RatKing", &none),
            (Some("Rat King".into()), Some("Rat King Flag".into()), Some("Skins".into()))
        );
        // A hero leaf under Skins takes the roster spelling.
        assert_eq!(classify("RatKing", "Skins", &none).0, Some("Rat King".into()));
        assert_eq!(classify("The Doorman", "Skins", &none).0, Some("Doorman".into()));
        // A roster hero under a non-skin parent keeps the hero without a known list.
        assert_eq!(classify("Haze", "Sounds", &none).0, Some("Haze".into()));
        // A root-filed skin has no hero from its category.
        assert_eq!(classify("Skins", "", &none), (None, Some("Skins".into()), Some("Skins".into())));
        assert_eq!(category_hero("Mo and Krill"), Some("Mo & Krill"));
        assert_eq!(category_hero("Gameplay Modifications"), None);
    }

    #[test]
    fn favorites_are_one_flag_set_in_bulk() {
        let mut x = Extras::default();
        let a = x.entry(key(1, 10, None)).uid.clone();
        let b = x.entry(key(2, 20, None)).uid.clone();
        x.entry(key(1, 10, None)).favorite = true;
        assert_eq!(x.favorites(), vec![a.clone()]);
        assert!(x.is_favorite(&a));
        assert!(x.set_favorites(std::slice::from_ref(&b)));
        assert_eq!(x.favorites(), vec![b.clone()]);
        assert!(!x.set_favorites(std::slice::from_ref(&b)));
        // A store written before the flag existed reads as not saved, and false is not written.
        let text = serde_json::to_string(&x.mods[0]).unwrap();
        assert!(!text.contains("favorite"));
        let old: ModExtra = serde_json::from_str(r#"{"uid":"u","slot":1}"#).unwrap();
        assert!(!old.favorite && old.gallery.is_empty());
    }

    #[test]
    fn tags_sort_by_the_players_priority_then_kind_and_name() {
        let mut tags = vec![
            (TagKind::Hero, "Haze".to_string()),
            (TagKind::Base, "Skins".to_string()),
            (TagKind::Custom, "ranked".to_string()),
            (TagKind::Custom, "comp".to_string()),
        ];
        sort_tags(&[], &mut tags);
        let labels: Vec<&str> = tags.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(labels, ["comp", "ranked", "Skins", "Haze"]);
        // Move Haze up twice: it is saved above Skins and ranked.
        let order = move_tag(&tags, (TagKind::Hero, "haze"), -1).unwrap();
        sort_tags(&order, &mut tags);
        let order = move_tag(&tags, (TagKind::Hero, "Haze"), -1).unwrap();
        sort_tags(&order, &mut tags);
        let labels: Vec<&str> = tags.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(labels, ["comp", "Haze", "ranked", "Skins"]);
        assert!(move_tag(&tags, (TagKind::Custom, "comp"), -1).is_none());
        assert!(move_tag(&tags, (TagKind::Base, "Skins"), 1).is_none());
        // A tag the order does not know yet follows the listed ones.
        tags.push((TagKind::Custom, "aaa".to_string()));
        sort_tags(&order, &mut tags);
        assert_eq!(tags.last().unwrap().1, "aaa");
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
