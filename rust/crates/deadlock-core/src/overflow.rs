//! More enabled mods than one addon folder holds: extra folders, and the `gameinfo.gi` block
//! that mounts them.
//!
//! One folder loads `pak01_dir.vpk` through `pak99_dir.vpk`, so 99 mods are the most it can
//! carry. Deadlock Mod Manager and Grimoire both lift that ceiling by mounting sibling folders
//! under `citadel/`, each with its own `pak01` to `pak99`, listed after `citadel/addons` in
//! `SearchPaths`. Folder order decides first and the slot number second, so a mod in
//! `citadel/addons` still wins a shared path over one in an extra folder, and an earlier extra
//! folder wins over a later one.
//!
//! Sigil names its extra folders `sigil-addons2` to `sigil-addons10`, beside `addons`:
//!
//! ```text
//! // sigil:overflow-begin
//! Game citadel/sigil-addons2
//! Game citadel/sigil-addons3
//! // sigil:overflow-end
//! ```
//!
//! The plain folder counts as folder one, so the first extra folder is number two. The `sigil-`
//! prefix keeps these folders out of the `addonsN` names Deadlock Mod Manager and Grimoire own
//! and scan, so neither manager claims Sigil's files or mints slots into them. Sitting outside
//! `citadel/addons` also keeps them clear of the profile block in `profiles/isolation.rs`, which
//! rewrites every `citadel/addons/<sigil-...>` mount into its own block.
//!
//! [`plan`] is pure: it places the enabled mods, in load order, into a folder and a slot each.
//! [`gameinfo_with_overflow`] returns the text that mounts the extra folders and
//! [`overflow_folders_in`] reads the block back. Nothing here moves a file or writes a game
//! folder: the app creates the folders, moves the slot files and writes `gameinfo.gi` after the
//! user approves.

use serde::{Deserialize, Serialize};

use crate::deploy::GAMEINFO_ADDONS_LINE;
use crate::error::{Error, Result};

/// Slots one addon folder holds: `pak01_dir.vpk` through `pak99_dir.vpk`.
pub const SLOTS_PER_FOLDER: usize = 99;

/// Folders a plan uses at most, counting `citadel/addons` as the first: 990 slots.
///
/// Both reference managers stop at ten folders as well, so Sigil never mounts a folder the
/// others would refuse to make.
pub const MAX_FOLDERS: usize = 10;

/// The plain addon folder. It always holds the first mods and has the highest priority.
pub const BASE_FOLDER: &str = "addons";

/// Extra folder names this build writes start with this. A folder without it is never mounted.
pub const FOLDER_PREFIX: &str = "sigil-addons";

/// Marker the extra folder mounts sit between, in the style of the profile block.
pub const BLOCK_BEGIN: &str = "// sigil:overflow-begin";
pub const BLOCK_END: &str = "// sigil:overflow-end";

/// Marker family the profile block in `profiles/isolation.rs` is written with.
///
/// The overflow block is placed after the profile block because a profile switch replaces that
/// whole block, and anything placed inside it would go with it.
const PROFILE_MARKER_PREFIX: &str = "// sigil:profile-";

/// The name of extra folder `number`, counting `citadel/addons` as folder one.
///
/// `2` gives `sigil-addons2`, the first folder past the plain one. A number outside 2 to
/// [`MAX_FOLDERS`] is refused, so no caller can invent a folder the game never loads.
pub fn folder_name(number: usize) -> Result<String> {
    if !(2..=MAX_FOLDERS).contains(&number) {
        return Err(Error::msg(format!(
            "Addon folder {number} is outside 2 to {MAX_FOLDERS}."
        )));
    }
    Ok(extra_folder_name(number))
}

/// The unchecked form of [`folder_name`], for numbers [`plan`] has already bounded.
fn extra_folder_name(number: usize) -> String {
    format!("{FOLDER_PREFIX}{number}")
}

/// A folder name this build is willing to mount: `sigil-addons2` to `sigil-addons10`.
fn valid_folder(folder: &str) -> bool {
    let Some(digits) = folder.strip_prefix(FOLDER_PREFIX) else {
        return false;
    };
    let Ok(number) = digits.parse::<usize>() else {
        return false;
    };
    (2..=MAX_FOLDERS).contains(&number) && digits == number.to_string()
}

// ---------------------------------------------------------------------------
// Plan (pure)
// ---------------------------------------------------------------------------

/// One enabled mod's place: the folder it goes in and the slot file it takes there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverflowSlot {
    /// Position in the caller's load order, starting at 1.
    pub position: usize,
    /// The label the caller passed for the mod, unchanged.
    pub label: String,
    /// Folder inside `citadel/`: `addons` or `sigil-addonsN`.
    pub folder: String,
    /// Slot number, 1 to 99, local to `folder`.
    pub slot: u8,
    /// The file the slot takes in `folder`: `pakNN_dir.vpk`.
    pub file: String,
}

/// Where every enabled mod goes. Built by [`plan`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverflowPlan {
    /// Every mod that fits, in load order.
    pub slots: Vec<OverflowSlot>,
    /// Extra folders to mount, in mount order. Empty when everything fits in `citadel/addons`,
    /// which is the case for every install that has not crossed 99 enabled mods.
    pub folders: Vec<String>,
    /// Mods past the last folder, in load order. They cannot load until others are switched off.
    pub unplaced: Vec<String>,
}

impl OverflowPlan {
    /// Whether any mod needs an extra folder.
    pub fn needs_overflow(&self) -> bool {
        !self.folders.is_empty()
    }

    /// Whether the caller has more enabled mods than [`MAX_FOLDERS`] folders hold.
    pub fn over_capacity(&self) -> bool {
        !self.unplaced.is_empty()
    }
}

/// Where each enabled mod goes, in load order: the first 99 to `citadel/addons`, the rest to
/// extra folders of up to 99 each, keeping overall priority (earlier wins) across folders.
///
/// One entry is one addon slot, so a mod that ships more than one directory VPK is passed once
/// per slot. The label is not read or cleaned: it goes back out on [`OverflowSlot::label`] so the
/// caller can match it to its own list. Mods past the last folder are named in
/// [`OverflowPlan::unplaced`] instead of being given a slot the game would ignore.
pub fn plan(enabled_in_order: &[String]) -> OverflowPlan {
    let mut plan = OverflowPlan::default();
    for (index, label) in enabled_in_order.iter().enumerate() {
        let position = index + 1;
        let folder_index = (position - 1) / SLOTS_PER_FOLDER;
        if folder_index >= MAX_FOLDERS {
            plan.unplaced.push(label.clone());
            continue;
        }
        let folder = if folder_index == 0 {
            BASE_FOLDER.to_string()
        } else {
            extra_folder_name(folder_index + 1)
        };
        if folder_index > 0 && !plan.folders.contains(&folder) {
            plan.folders.push(folder.clone());
        }
        let slot = ((position - 1) % SLOTS_PER_FOLDER) + 1;
        plan.slots.push(OverflowSlot {
            position,
            label: label.clone(),
            folder,
            slot: slot as u8,
            file: format!("pak{slot:02}_dir.vpk"),
        });
    }
    plan
}

// ---------------------------------------------------------------------------
// gameinfo.gi text (pure)
// ---------------------------------------------------------------------------

/// The line text without its line ending.
fn content(line: &str) -> &str {
    line.trim_end_matches(['\r', '\n'])
}

/// The `citadel/addons` value of a `Game` line: `Some("")` for the plain folder,
/// `Some(folder)` for a subfolder, `None` for any other line.
///
/// `citadel/addons2`, the sibling root Deadlock Mod Manager and Grimoire use, is not a match:
/// the prefix needs its slash.
fn mount_value(line: &str) -> Option<String> {
    let code = line.split("//").next().unwrap_or(line).trim();
    let mut parts = code.split_whitespace();
    if !parts.next()?.eq_ignore_ascii_case("Game") {
        return None;
    }
    let value = parts.next()?.trim_matches('"');
    let lower = value.trim_end_matches('/').to_ascii_lowercase();
    let rest = lower.strip_prefix("citadel/addons")?;
    if rest.is_empty() {
        return Some(String::new());
    }
    let rest = rest.strip_prefix('/')?;
    Some(rest.trim_end_matches('/').to_string())
}

/// The extra folder a `Game citadel/sigil-addonsN` line mounts.
fn overflow_mount_value(line: &str) -> Option<String> {
    let code = line.split("//").next().unwrap_or(line).trim();
    let mut parts = code.split_whitespace();
    if !parts.next()?.eq_ignore_ascii_case("Game") {
        return None;
    }
    let value = parts.next()?.trim_matches('"');
    let lower = value.trim_end_matches('/').to_ascii_lowercase();
    let rest = lower.strip_prefix("citadel/")?;
    valid_folder(rest).then(|| rest.to_string())
}

/// The first and last line of the Sigil profile block, when the file has one.
///
/// The block is a begin marker, one mount line and an end marker, so the first other marker
/// after the begin is the end. A begin without an end is refused: the overflow block would have
/// no safe place to go, and `profiles/isolation.rs` refuses the same file.
fn profile_block(lines: &[&str]) -> Result<Option<(usize, usize)>> {
    let Some(start) = lines
        .iter()
        .position(|line| content(line).trim() == crate::profiles::PROFILE_BLOCK_BEGIN)
    else {
        return Ok(None);
    };
    for (index, line) in lines.iter().enumerate().skip(start + 1) {
        if content(line).trim().starts_with(PROFILE_MARKER_PREFIX) {
            return Ok(Some((start, index)));
        }
    }
    Err(Error::msg(
        "gameinfo.gi has half a Sigil profile block. Fix or remove its markers.",
    ))
}

/// The gameinfo.gi text with the marked block for the extra folders added, replaced or removed
/// (no extra folders = block removed). Same marker style and placement rules as isolation.rs.
///
/// The block goes after the addon mount, which is the plain `Game citadel/addons` line or the
/// end of the Sigil profile block, and so above `Game citadel`: the extra folders load after the
/// main folder and before the game's own files. Every mount line carries the indentation of the
/// line it follows and the file's own line ending, so a CRLF file stays CRLF.
///
/// A `Game citadel/sigil-addonsN` line outside the block is folded back in, the way a stray
/// profile mount is. The profile block itself is never touched. Text that already mounts exactly
/// these folders comes back unchanged, so calling twice is safe.
pub fn gameinfo_with_overflow(text: &str, folders: &[String]) -> Result<String> {
    let mut wanted: Vec<&str> = Vec::new();
    for folder in folders {
        if !valid_folder(folder) {
            return Err(Error::msg(format!("{folder:?} is not an extra addon folder.")));
        }
        if wanted.contains(&folder.as_str()) {
            return Err(Error::msg(format!("{folder:?} is mounted twice.")));
        }
        wanted.push(folder);
    }

    let ending = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let lines: Vec<&str> = text.split_inclusive('\n').collect();

    let mut block_start = None;
    let mut block_end = None;
    for (index, line) in lines.iter().enumerate() {
        let trimmed = content(line).trim();
        if trimmed == BLOCK_BEGIN {
            block_start = Some(index);
        } else if trimmed == BLOCK_END {
            block_end = Some(index);
            break;
        }
    }
    if block_start.is_some() != block_end.is_some()
        || block_start.is_some_and(|start| block_end.is_some_and(|end| end < start))
    {
        return Err(Error::msg(
            "gameinfo.gi has half a Sigil overflow block. Fix or remove its markers.",
        ));
    }

    let profile = profile_block(&lines)?;
    let owned = |index: usize| {
        (block_start.is_some_and(|start| index >= start) && block_end.is_some_and(|end| index <= end))
            || profile.is_some_and(|(start, end)| (start..=end).contains(&index))
    };

    // The addon mount the block follows, and stale mounts of our own folders outside both blocks.
    let mut plain_at = None;
    let mut stale: Vec<usize> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if owned(index) {
            continue;
        }
        if let Some(value) = mount_value(content(line)) {
            if value.is_empty() {
                plain_at = Some(index);
            }
        } else if overflow_mount_value(content(line)).is_some() {
            stale.push(index);
        }
    }

    let anchor = match (profile, plain_at) {
        (Some((_, end)), _) => end,
        (None, Some(at)) => at,
        (None, None) => {
            return Err(Error::msg(format!(
                "gameinfo.gi has no `{GAMEINFO_ADDONS_LINE}` SearchPath line to put the extra folders after."
            )));
        }
    };
    // An existing block keeps its place, so a profile switch or a removed folder never moves it.
    // A new block goes after the addon mount it follows, never before it.
    let indent_from = block_start.unwrap_or(anchor);
    let at = match block_start {
        Some(start) => start,
        None => anchor + 1,
    };
    let indent = {
        let source = content(lines[indent_from]);
        let trimmed = source.trim_start();
        source[..source.len() - trimmed.len()].to_string()
    };

    let mut block = String::new();
    if !wanted.is_empty() {
        block.push_str(&indent);
        block.push_str(BLOCK_BEGIN);
        block.push_str(ending);
        for folder in &wanted {
            block.push_str(&indent);
            block.push_str("Game citadel/");
            block.push_str(folder);
            block.push_str(ending);
        }
        block.push_str(&indent);
        block.push_str(BLOCK_END);
        block.push_str(ending);
    }

    let mut removed: Vec<usize> = Vec::new();
    if let (Some(start), Some(end)) = (block_start, block_end) {
        removed.extend(start..=end);
    }
    removed.extend(stale);
    removed.sort_unstable();
    removed.dedup();

    let mut output = String::with_capacity(text.len() + block.len());
    let mut inserted = false;
    for (index, line) in lines.iter().enumerate() {
        if index == at {
            output.push_str(&block);
            inserted = true;
        }
        if removed.contains(&index) {
            continue;
        }
        output.push_str(line);
    }
    if !inserted {
        output.push_str(&block);
    }
    Ok(output)
}

/// The extra folders the marked block mounts, in block order. Empty when the file has no closed
/// block, so a half-written block reads as no overflow rather than as a plan.
pub fn overflow_folders_in(text: &str) -> Vec<String> {
    let mut folders = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == BLOCK_BEGIN {
            if inside {
                return Vec::new();
            }
            inside = true;
            continue;
        }
        if trimmed == BLOCK_END {
            return if inside { folders } else { Vec::new() };
        }
        if inside
            && let Some(folder) = overflow_mount_value(line).filter(|folder| valid_folder(folder))
        {
            folders.push(folder);
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GAMEINFO: &str = "\"GameInfo\"\n{\n\tFileSystem\n\t{\n\t\tSearchPaths\n\t\t{\n\t\t\tGame_Language \"citadel_*LANGUAGE*\"\n\n\t\t\tGame citadel/addons\n\t\t\tGame \"citadel\"\n\t\t\tGame \"core\"\n\t\t}\n\t}\n}\n";

    fn labels(count: usize) -> Vec<String> {
        (1..=count).map(|number| format!("mod-{number}")).collect()
    }

    fn folders(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn nothing_to_place_plans_nothing() {
        let plan = plan(&[]);
        assert!(plan.slots.is_empty());
        assert!(plan.folders.is_empty());
        assert!(!plan.needs_overflow());
        assert!(!plan.over_capacity());
    }

    #[test]
    fn ninety_nine_mods_all_fit_in_the_plain_folder() {
        let plan = plan(&labels(99));
        assert_eq!(plan.slots.len(), 99);
        assert!(plan.folders.is_empty());
        assert!(plan.slots.iter().all(|slot| slot.folder == BASE_FOLDER));
        let first = &plan.slots[0];
        assert_eq!((first.position, first.slot, first.file.as_str()), (1, 1, "pak01_dir.vpk"));
        let last = &plan.slots[98];
        assert_eq!((last.position, last.slot, last.file.as_str()), (99, 99, "pak99_dir.vpk"));
        assert_eq!(last.label, "mod-99");
    }

    #[test]
    fn the_hundredth_mod_opens_the_first_extra_folder() {
        let plan = plan(&labels(100));
        assert!(plan.needs_overflow());
        assert_eq!(plan.folders, vec!["sigil-addons2".to_string()]);
        let extra = &plan.slots[99];
        assert_eq!(extra.position, 100);
        assert_eq!(extra.label, "mod-100");
        assert_eq!(extra.folder, "sigil-addons2");
        assert_eq!(extra.slot, 1);
        assert_eq!(extra.file, "pak01_dir.vpk");
    }

    #[test]
    fn two_hundred_and_fifty_mods_fill_two_extra_folders() {
        let plan = plan(&labels(250));
        assert_eq!(plan.slots.len(), 250);
        assert_eq!(
            plan.folders,
            vec!["sigil-addons2".to_string(), "sigil-addons3".to_string()]
        );
        let at = |position: usize| &plan.slots[position - 1];
        assert_eq!((at(198).folder.as_str(), at(198).slot), ("sigil-addons2", 99));
        assert_eq!((at(199).folder.as_str(), at(199).slot), ("sigil-addons3", 1));
        assert_eq!((at(250).folder.as_str(), at(250).slot), ("sigil-addons3", 52));
        assert_eq!(at(250).file, "pak52_dir.vpk");
        assert!(!plan.over_capacity());
    }

    #[test]
    fn mods_past_the_last_folder_are_named_not_placed() {
        let count = MAX_FOLDERS * SLOTS_PER_FOLDER;
        let plan = plan(&labels(count + 3));
        assert_eq!(plan.slots.len(), count);
        assert_eq!(plan.folders.len(), MAX_FOLDERS - 1);
        assert_eq!(plan.folders.last().map(String::as_str), Some("sigil-addons10"));
        let last = plan.slots.last().unwrap();
        assert_eq!((last.folder.as_str(), last.slot), ("sigil-addons10", 99));
        assert!(plan.over_capacity());
        let first_unplaced = plan.slots.len() + 1;
        assert_eq!(plan.unplaced, labels(count + 3)[count..].to_vec());
        assert_eq!(plan.unplaced[0], format!("mod-{first_unplaced}"));
    }

    #[test]
    fn folder_names_count_the_plain_folder_as_one() {
        assert_eq!(folder_name(2).unwrap(), "sigil-addons2");
        assert_eq!(folder_name(MAX_FOLDERS).unwrap(), "sigil-addons10");
        assert!(folder_name(1).is_err());
        assert!(folder_name(MAX_FOLDERS + 1).is_err());
        assert!(!valid_folder("sigil-addons1"));
        assert!(!valid_folder("sigil-addons02"));
        assert!(!valid_folder("addons2"));
        assert!(!valid_folder("sigil-addons2/extra"));
    }

    #[test]
    fn the_block_is_added_replaced_and_removed_without_touching_the_rest() {
        let added =
            gameinfo_with_overflow(GAMEINFO, &folders(&["sigil-addons2", "sigil-addons3"])).unwrap();
        let begin = added.find(BLOCK_BEGIN).unwrap();
        let plain = added.find("Game citadel/addons\n").unwrap();
        let extra = added.find("Game citadel/sigil-addons2").unwrap();
        let citadel = added.find("Game \"citadel\"\n").unwrap();
        assert!(plain < begin && begin < extra && extra < citadel, "{added}");
        assert!(added.contains("\t\t\t// sigil:overflow-end\n"));
        assert!(added.contains("Game \"citadel\"\n"));
        assert!(added.contains("Game \"core\"\n"));
        assert!(added.contains("Game_Language"));
        assert_eq!(
            overflow_folders_in(&added),
            folders(&["sigil-addons2", "sigil-addons3"])
        );

        // Idempotent: the same folders produce the same text.
        assert_eq!(
            gameinfo_with_overflow(&added, &folders(&["sigil-addons2", "sigil-addons3"])).unwrap(),
            added
        );

        // Replaced: one folder left, no trace of the one dropped.
        let replaced = gameinfo_with_overflow(&added, &folders(&["sigil-addons2"])).unwrap();
        assert_eq!(replaced.matches(BLOCK_BEGIN).count(), 1);
        assert!(!replaced.contains("sigil-addons3"));
        assert_eq!(overflow_folders_in(&replaced), folders(&["sigil-addons2"]));

        // Removed: back to the stock text, byte for byte.
        let removed = gameinfo_with_overflow(&replaced, &[]).unwrap();
        assert_eq!(removed, GAMEINFO);
        assert_eq!(gameinfo_with_overflow(GAMEINFO, &[]).unwrap(), GAMEINFO);

        // A file without a final line ending comes back as it went in.
        let no_ending = GAMEINFO.trim_end_matches('\n');
        let mounted = gameinfo_with_overflow(no_ending, &folders(&["sigil-addons2"])).unwrap();
        assert!(!mounted.ends_with('\n'));
        assert_eq!(gameinfo_with_overflow(&mounted, &[]).unwrap(), no_ending);
    }

    #[test]
    fn a_stray_overflow_mount_outside_the_block_is_folded_back_in() {
        let stray = GAMEINFO.replace(
            "Game citadel/addons",
            "Game citadel/addons\n\t\t\tGame citadel/sigil-addons2",
        );
        let mounted = gameinfo_with_overflow(&stray, &folders(&["sigil-addons3"])).unwrap();
        assert_eq!(mounted.matches(BLOCK_BEGIN).count(), 1);
        assert!(!mounted.contains("sigil-addons2"));
        assert_eq!(overflow_folders_in(&mounted), folders(&["sigil-addons3"]));
    }

    #[test]
    fn crlf_and_tabs_survive() {
        let crlf = GAMEINFO.replace('\n', "\r\n");
        let mounted = gameinfo_with_overflow(&crlf, &folders(&["sigil-addons2"])).unwrap();
        assert!(mounted.contains("\r\n\t\t\t// sigil:overflow-begin\r\n"));
        assert!(mounted.contains("\r\n\t\t\tGame citadel/sigil-addons2\r\n"));
        assert!(mounted.contains("\r\n\t\t\t// sigil:overflow-end\r\n"));
        assert!(!mounted.replace("\r\n", "").contains('\n'));
        assert_eq!(overflow_folders_in(&mounted), folders(&["sigil-addons2"]));
        assert_eq!(gameinfo_with_overflow(&mounted, &[]).unwrap(), crlf);
    }

    #[test]
    fn the_profile_block_and_the_overflow_block_live_together() {
        // Overflow first, then a profile mount: the profile block goes above the overflow.
        let with_overflow = gameinfo_with_overflow(GAMEINFO, &folders(&["sigil-addons2"])).unwrap();
        let mounted = crate::profiles::plan_mount(&with_overflow, Some("sigil-ranked"))
            .unwrap()
            .unwrap();
        let profile = mounted.find("Game citadel/addons/sigil-ranked").unwrap();
        let overflow = mounted.find("Game citadel/sigil-addons2").unwrap();
        let citadel = mounted.find("Game \"citadel\"\n").unwrap();
        assert!(profile < overflow && overflow < citadel, "{mounted}");
        assert_eq!(overflow_folders_in(&mounted), folders(&["sigil-addons2"]));

        // A profile switch keeps the overflow block where it is.
        let switched = crate::profiles::plan_mount(&mounted, Some("sigil-casual"))
            .unwrap()
            .unwrap();
        assert!(!switched.contains("sigil-ranked"));
        assert_eq!(overflow_folders_in(&switched), folders(&["sigil-addons2"]));
        let profile_at = switched.find("Game citadel/addons/sigil-casual").unwrap();
        let overflow_at = switched.find("Game citadel/sigil-addons2").unwrap();
        assert!(profile_at < overflow_at);
        assert_eq!(
            gameinfo_with_overflow(&switched, &folders(&["sigil-addons2"])).unwrap(),
            switched
        );

        // Dropping the profile puts the plain line back and leaves the overflow alone.
        let plain = crate::profiles::plan_mount(&switched, None).unwrap().unwrap();
        assert!(plain.contains("\t\t\tGame citadel/addons\n"));
        assert_eq!(overflow_folders_in(&plain), folders(&["sigil-addons2"]));
        let addons_at = plain.find("Game citadel/addons\n").unwrap();
        let overflow_at = plain.find("Game citadel/sigil-addons2").unwrap();
        assert!(addons_at < overflow_at);

        // Profile first, then overflow: the overflow block lands after the profile block.
        let profile_first = crate::profiles::plan_mount(GAMEINFO, Some("sigil-ranked"))
            .unwrap()
            .unwrap();
        let both = gameinfo_with_overflow(&profile_first, &folders(&["sigil-addons2"])).unwrap();
        assert!(both.find("// sigil:profile-end").unwrap() < both.find(BLOCK_BEGIN).unwrap());
        assert!(both.find(BLOCK_BEGIN).unwrap() < both.find("Game citadel/sigil-addons2").unwrap());
        assert!(both.contains("Game citadel/addons/sigil-ranked"));
    }

    #[test]
    fn bad_folder_names_and_half_blocks_are_refused() {
        for folder in [
            "addons2",
            "sigil-addons1",
            "sigil-addons11",
            "sigil-addons02",
            "sigil-addons2/../x",
            "../sigil-addons2",
            "",
        ] {
            assert!(
                gameinfo_with_overflow(GAMEINFO, &folders(&[folder])).is_err(),
                "{folder:?}"
            );
        }
        assert!(
            gameinfo_with_overflow(GAMEINFO, &folders(&["sigil-addons2", "sigil-addons2"])).is_err()
        );

        let half = format!("{GAMEINFO}\n{BLOCK_BEGIN}\n");
        assert!(gameinfo_with_overflow(&half, &folders(&["sigil-addons2"])).is_err());
        let broken_profile = format!(
            "{GAMEINFO}\n{}\n",
            crate::profiles::PROFILE_BLOCK_BEGIN
        );
        assert!(gameinfo_with_overflow(&broken_profile, &folders(&["sigil-addons2"])).is_err());

        // No addon mount at all: refuse rather than put the folders below the game's own files.
        let stock = GAMEINFO.replace("\t\t\tGame citadel/addons\n", "");
        assert!(gameinfo_with_overflow(&stock, &folders(&["sigil-addons2"])).is_err());
    }

    #[test]
    fn reading_requires_a_closed_block() {
        assert!(overflow_folders_in("").is_empty());
        assert!(overflow_folders_in(GAMEINFO).is_empty());
        assert!(overflow_folders_in(&format!("{BLOCK_BEGIN}\nGame citadel/sigil-addons2\n")).is_empty());
        assert!(overflow_folders_in(&format!("{BLOCK_END}\n")).is_empty());
        // A closed block reports only the folder lines in it, in order, ignoring other mounts.
        let block = format!(
            "{BLOCK_BEGIN}\nGame citadel/sigil-addons3\nGame citadel/addons2\nGame citadel/sigil-addons2\n{BLOCK_END}\n"
        );
        assert_eq!(overflow_folders_in(&block), folders(&["sigil-addons3", "sigil-addons2"]));
    }
}
