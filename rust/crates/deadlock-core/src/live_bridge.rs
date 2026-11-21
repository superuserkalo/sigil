//! The Sigil live bridge: an opt-in HUD mod that prints the in-game top bar (both teams'
//! heroes, souls, levels, K/D/A, objective damage and the match clock) into the console log,
//! where [`crate::live_match`] reads it.
//!
//! The bridge is two files in one `pakNN_dir.vpk`:
//!
//! - `panorama/layout/citadel_hud_top_bar_team.vxml_c`: the game's own top bar team layout with
//!   one script include added. It is a copy of the installed game's file, so it is built on the
//!   player's computer from their `pak01_dir.vpk` and rebuilt after a game update.
//! - `panorama/scripts/sigil_live_bridge.vjs_c`: `live_bridge.js` in a compiled script
//!   container. The container is one of the game's own scripts with its `DATA` block (the script
//!   text) swapped, so no Valve compiler is needed and nothing from the game is shipped.
//!
//! Deadlock Mod Manager (GPL-3.0) has a live bridge built the same way; this is written from its
//! behaviour, not its code. The compiled resource handling is morphic's (vpkmerge, MIT).
//!
//! Building writes only to a folder the caller picks. GAME WRITE: [`rebuild_installed`] replaces
//! the installed bridge's VPK and refuses without `approve`; the install itself goes through the
//! normal install pipeline from an app action.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use morphic::kv3::{self, Seg, Value};
use morphic::resource::Resource;

use crate::addons::{InstalledAddon, list_installed};
use crate::deploy::addons_dir;
use crate::error::{Error, Result};
use crate::vpk::{ReadVpkOptions, VpkDirectory, read_vpk_directory, read_vpk_directory_with, read_vpk_entry_data};
use crate::vpk_write::{PackOptions, pack_vpk_single};

/// The game's top bar team panel layout. One copy loads per team.
pub const TEAM_LAYOUT: &str = "panorama/layout/citadel_hud_top_bar_team.vxml_c";
/// The bridge script inside the mod.
pub const SCRIPT: &str = "panorama/scripts/sigil_live_bridge.vjs_c";
/// The script text.
pub const SOURCE: &str = include_str!("live_bridge.js");
/// The name the library shows for the installed bridge.
pub const MOD_NAME: &str = "Sigil live bridge";
/// `ModMeta::source` of the installed bridge.
pub const MOD_SOURCE: &str = "sigil";
/// Script containers looked at before giving up.
const CONTAINERS_TRIED: usize = 64;

fn bad(what: impl Into<String>) -> Error {
    Error::msg(what.into())
}

fn is_node(value: &Value, kind: &str) -> bool {
    value.get("eType").and_then(Value::as_str) == Some(kind)
}

/// The game's top bar layout with an include of [`SCRIPT`] added: into its `SCRIPTS` section
/// when it has one, else as a new `SCRIPTS` section after the first child, shaped like the
/// layout's own `STYLES` section so the new nodes carry the keys the game's compiler writes.
pub fn patch_layout(base: &[u8]) -> Result<Vec<u8>> {
    let unexpected = |what: &str| bad(format!("The game's top bar layout has a shape Sigil does not know: {what}."));
    let resource = Resource::parse(base).map_err(|_| unexpected("not a compiled resource"))?;
    let index = resource
        .blocks()
        .iter()
        .position(|block| &block.kind == b"LaCo")
        .ok_or_else(|| unexpected("no layout block"))?;
    let payload = resource.get_block_by_index(index).ok_or_else(|| unexpected("layout block out of range"))?;
    let tree = kv3::decode(payload).map_err(|_| unexpected("the layout block is not KV3"))?;
    let children = tree
        .get("m_AST")
        .and_then(|ast| ast.get("m_pRoot"))
        .and_then(|root| root.get("vecChildren"))
        .and_then(Value::as_array)
        .ok_or_else(|| unexpected("no root panel"))?;
    let root_path = vec![
        Seg::Key("m_AST".into()),
        Seg::Key("m_pRoot".into()),
        Seg::Key("vecChildren".into()),
    ];

    let named = |include: &Value| -> Result<Value> {
        let mut include = include.clone();
        *include
            .get_mut("child")
            .and_then(|reference| reference.get_mut("name"))
            .ok_or_else(|| unexpected("an include without a named reference"))? = Value::String(SCRIPT.into());
        Ok(include)
    };

    let patched = if let Some(at) = children.iter().position(|node| is_node(node, "SCRIPTS")) {
        let scripts = &children[at];
        let includes = scripts.get("vecChildren").and_then(Value::as_array).unwrap_or(&[]);
        if includes.iter().any(|include| {
            include.get("child").and_then(|c| c.get("name")).and_then(Value::as_str) == Some(SCRIPT)
        }) {
            return Err(bad("The top bar layout already loads the live bridge."));
        }
        let template = includes.first().ok_or_else(|| unexpected("an empty scripts section"))?;
        let mut path = root_path.clone();
        path.push(Seg::Index(at));
        path.push(Seg::Key("vecChildren".into()));
        kv3::insert_array_element_adding(payload, &path, includes.len(), &named(template)?)
    } else {
        let styles = children
            .iter()
            .find(|node| is_node(node, "STYLES"))
            .ok_or_else(|| unexpected("no styles section"))?;
        let template = styles
            .get("vecChildren")
            .and_then(Value::as_array)
            .and_then(<[Value]>::first)
            .ok_or_else(|| unexpected("a styles section without includes"))?;
        let mut scripts = styles.clone();
        *scripts.get_mut("eType").ok_or_else(|| unexpected("a section without a type"))? = Value::String("SCRIPTS".into());
        *scripts.get_mut("vecChildren").ok_or_else(|| unexpected("a section without children"))? =
            Value::Array(vec![named(template)?]);
        kv3::insert_array_element_adding(payload, &root_path, 1.min(children.len()), &scripts)
    }
    .map_err(|error| bad(format!("Sigil could not add the bridge to the top bar layout: {error}")))?;
    resource
        .rebuild_with_block(index, &patched)
        .map_err(|error| bad(format!("Sigil could not rebuild the top bar layout: {error}")))
}

/// A compiled script whose `DATA` block is plain text and whose other blocks are only edit
/// info: the kind the game loads the text of.
fn usable_container(bytes: &[u8]) -> bool {
    let Ok(resource) = Resource::parse(bytes) else { return false };
    let only_info = resource
        .blocks()
        .iter()
        .all(|block| matches!(&block.kind, b"DATA" | b"RED2" | b"REDI"));
    let Ok(data) = resource.data_block() else { return false };
    only_info && !data.is_empty() && !data.contains(&0) && std::str::from_utf8(data).is_ok()
}

/// The bridge script: `container` (a compiled game script) with [`SOURCE`] as its text.
pub fn build_script(container: &[u8]) -> Result<Vec<u8>> {
    let resource = Resource::parse(container).map_err(|_| bad("The script container is not a compiled resource."))?;
    resource
        .rebuild_with_data(SOURCE.as_bytes())
        .map_err(|error| bad(format!("Sigil could not build the bridge script: {error}")))
}

/// The smallest usable script of the game, by size then path, so the choice is the same on
/// every build for one game version.
fn pick_container(dir: &VpkDirectory) -> Result<Vec<u8>> {
    let mut scripts: Vec<_> = dir
        .entries
        .iter()
        .filter(|entry| {
            let path = entry.path.to_ascii_lowercase();
            path.starts_with("panorama/scripts/") && path.ends_with(".vjs_c")
        })
        .collect();
    scripts.sort_by(|a, b| a.total_length.cmp(&b.total_length).then_with(|| a.path.cmp(&b.path)));
    for entry in scripts.into_iter().take(CONTAINERS_TRIED) {
        if let Ok(bytes) = read_vpk_entry_data(dir, entry)
            && usable_container(&bytes)
        {
            return Ok(bytes);
        }
    }
    Err(bad("This game version has no compiled script Sigil can carry the bridge in."))
}

/// Why the expected bridge files could not be made.
#[derive(Debug)]
pub enum BuildError {
    /// The game's files could not be read.
    Unreadable(Error),
    /// The game's files no longer have the shape the bridge patches.
    Incompatible(Error),
}

impl From<BuildError> for Error {
    fn from(error: BuildError) -> Error {
        match error {
            BuildError::Unreadable(error) | BuildError::Incompatible(error) => error,
        }
    }
}

fn pak01(game_root: &Path) -> PathBuf {
    game_root.join("game").join("citadel").join("pak01_dir.vpk")
}

/// The two files a bridge built right now would hold: `(TEAM_LAYOUT, SCRIPT)`.
pub fn expected_files(game_root: &Path) -> std::result::Result<[(String, Vec<u8>); 2], BuildError> {
    let dir = read_vpk_directory(&pak01(game_root)).map_err(BuildError::Unreadable)?;
    let layout_entry = dir
        .by_path
        .get(TEAM_LAYOUT)
        .ok_or_else(|| BuildError::Incompatible(bad(format!("The game has no {TEAM_LAYOUT}."))))?;
    let base = read_vpk_entry_data(&dir, layout_entry).map_err(BuildError::Unreadable)?;
    let layout = patch_layout(&base).map_err(BuildError::Incompatible)?;
    let script = build_script(&pick_container(&dir).map_err(BuildError::Incompatible)?).map_err(BuildError::Incompatible)?;
    Ok([(TEAM_LAYOUT.to_string(), layout), (SCRIPT.to_string(), script)])
}

/// Pack `files` as one self-contained `pak01_dir.vpk` in `out_dir`.
fn pack(files: &[(String, Vec<u8>)], out_dir: &Path) -> Result<PathBuf> {
    let staging = out_dir.join("staging");
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    for (path, bytes) in files {
        let target = staging.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(target, bytes)?;
    }
    let out = out_dir.join("pak01_dir.vpk");
    if out.exists() {
        fs::remove_file(&out)?;
    }
    let packed = pack_vpk_single(&PackOptions::new(staging.clone(), out_dir.to_path_buf(), "pak01"));
    let _ = fs::remove_dir_all(&staging);
    Ok(packed?.dir_path)
}

/// Build the bridge from the installed game into `out_dir/pak01_dir.vpk`. Writes nothing else.
pub fn build(game_root: &Path, out_dir: &Path) -> Result<PathBuf> {
    let files = expected_files(game_root)?;
    fs::create_dir_all(out_dir)?;
    pack(&files, out_dir)
}

/// The entry paths of a VPK, lowercased.
fn entry_set(dir: &VpkDirectory) -> BTreeSet<String> {
    dir.entries.iter().map(|entry| entry.path.to_ascii_lowercase()).collect()
}

/// True when the VPK holds the bridge and nothing else.
pub fn is_bridge(dir: &VpkDirectory) -> bool {
    entry_set(dir) == BTreeSet::from([TEAM_LAYOUT.to_string(), SCRIPT.to_string()])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeState {
    /// No addon holds the bridge.
    NotInstalled,
    /// Installed but switched off, so the game does not load it.
    Off,
    /// Installed, and what a build right now would make.
    Current,
    /// Built from an older game layout or an older bridge script: rebuild it.
    Outdated,
    /// The game's top bar changed in a way this Sigil cannot patch. The installed copy holds an
    /// old layout and should be switched off.
    Incompatible,
    /// The game's files could not be read, so the bridge could not be compared.
    Unknown,
}

/// What the addons folder says about the bridge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeCheck {
    pub state: BridgeState,
    /// The bridge's slot.
    pub slot: Option<u16>,
    /// The bridge's directory VPK.
    pub file: Option<PathBuf>,
    /// Other switched-on mods that also replace the top bar team layout, by slot. The lower
    /// slot wins, so one below the bridge silences it.
    pub others: Vec<u16>,
    /// Why the state is `Incompatible` or `Unknown`.
    pub note: Option<String>,
}

impl BridgeCheck {
    pub fn installed(&self) -> bool {
        self.slot.is_some()
    }

    /// A switched-on mod in a lower slot replaces the same layout, so the game loads that one.
    pub fn shadowed(&self) -> Option<u16> {
        let slot = self.slot?;
        self.others.iter().copied().filter(|other| *other < slot).min()
    }
}

/// Find the bridge among the installed addons and compare it with a build from the game.
pub fn check(game_root: &Path) -> Result<BridgeCheck> {
    let addons = addons_dir(game_root);
    let installed = list_installed(game_root)?;
    let mut bridge: Option<InstalledAddon> = None;
    let mut others = Vec::new();
    let quick = ReadVpkOptions { keep_preload: false };
    for addon in installed {
        let Ok(dir) = read_vpk_directory_with(&addon.path(&addons), &quick) else { continue };
        if bridge.is_none() && is_bridge(&dir) {
            bridge = Some(addon);
        } else if addon.enabled && dir.by_path.contains_key(TEAM_LAYOUT) {
            others.push(addon.slot);
        }
    }
    let Some(addon) = bridge else {
        return Ok(BridgeCheck { state: BridgeState::NotInstalled, slot: None, file: None, others, note: None });
    };
    let file = addon.path(&addons);
    let mut out = BridgeCheck {
        state: BridgeState::Current,
        slot: Some(addon.slot),
        file: Some(file.clone()),
        others,
        note: None,
    };
    match expected_files(game_root) {
        Err(BuildError::Incompatible(error)) => {
            out.state = BridgeState::Incompatible;
            out.note = Some(error.to_string());
        }
        Err(BuildError::Unreadable(error)) => {
            out.state = BridgeState::Unknown;
            out.note = Some(error.to_string());
        }
        Ok(expected) => {
            let dir = read_vpk_directory(&file)?;
            let same = expected.iter().all(|(path, bytes)| {
                dir.by_path
                    .get(path.as_str())
                    .and_then(|entry| read_vpk_entry_data(&dir, entry).ok())
                    .is_some_and(|have| &have == bytes)
            });
            if !same {
                out.state = BridgeState::Outdated;
            }
        }
    }
    if !addon.enabled && out.state == BridgeState::Current {
        out.state = BridgeState::Off;
    }
    Ok(out)
}

/// GAME WRITE. Rebuild the installed bridge at `file` from the current game, replacing it in
/// place. Refuses without `approve` and when `file` is not the bridge alone (a merged pack or
/// someone else's mod is never overwritten). `work` is a scratch folder outside the game.
pub fn rebuild_installed(game_root: &Path, file: &Path, work: &Path, approve: bool) -> Result<()> {
    if !approve {
        return Err(bad(format!("Not approved: rebuild {}. Nothing was written.", file.display())));
    }
    let dir = read_vpk_directory(file)?;
    if !is_bridge(&dir) {
        return Err(bad(format!("{} is not the live bridge alone. Nothing was written.", file.display())));
    }
    let built = build(game_root, work)?;
    let bytes = fs::read(&built)?;
    crate::util::write_atomic(file, &bytes)?;
    let _ = fs::remove_file(built);
    Ok(())
}

/// The game build as Sigil tells updates apart: `ClientVersion` from `steam.inf` and the size
/// and modified time of `pak01_dir.vpk` (a hotfix can keep the version). Two stats and one small
/// read; None when the game is not there. The live bridge is checked again when this changes.
pub fn game_build(game_root: &Path) -> Option<String> {
    let pak = fs::metadata(pak01(game_root)).ok()?;
    let modified = pak
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_secs());
    let version = crate::host::read_build(game_root).unwrap_or_default();
    Some(format!("{version}:{:x}-{modified:x}", pak.len()))
}

/// What a game build change asks of the installed bridge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RebuildAction {
    /// The build is the one the caller last saw: nothing changed.
    Unchanged,
    /// A new build, with nothing for the bridge to do: none installed, switched off, shadowed
    /// by a lower slot, already current, or a game layout this Sigil cannot patch.
    Nothing,
    /// The bridge is outdated and the game is closed: rebuild it now.
    RebuildNow { slot: u16, file: PathBuf },
    /// The bridge is outdated but the game is running: wait until it closes, then ask again.
    WaitForGame { slot: u16, file: PathBuf },
}

/// The build read now, and what a change since the caller last looked asks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RebuildPlan {
    /// The build to remember. None when the game is not there.
    pub build: Option<String>,
    pub action: RebuildAction,
}

/// Decide what a game update means for the live bridge: an outdated bridge is rebuilt after a
/// build change, but never while Deadlock runs. Reads the game and the addon folder, writes
/// nothing; the caller runs [`rebuild_when_closed`] on a worker when the answer is
/// [`RebuildAction::RebuildNow`].
pub fn rebuild_plan(game_root: &Path, seen_build: Option<&str>, game_running: bool) -> Result<RebuildPlan> {
    let Some(build) = game_build(game_root) else {
        return Ok(RebuildPlan { build: None, action: RebuildAction::Nothing });
    };
    if seen_build == Some(build.as_str()) {
        return Ok(RebuildPlan { build: Some(build), action: RebuildAction::Unchanged });
    }
    let check = check(game_root)?;
    let action = if check.state == BridgeState::Outdated {
        match (check.slot, check.file) {
            (Some(slot), Some(file)) if game_running => RebuildAction::WaitForGame { slot, file },
            (Some(slot), Some(file)) => RebuildAction::RebuildNow { slot, file },
            _ => RebuildAction::Nothing,
        }
    } else {
        RebuildAction::Nothing
    };
    Ok(RebuildPlan { build: Some(build), action })
}

/// GAME WRITE. [`rebuild_installed`], refused while Deadlock runs: the game keeps its addons
/// open, so a bridge is only ever replaced after it closes.
pub fn rebuild_when_closed(
    game_root: &Path,
    file: &Path,
    work: &Path,
    game_running: bool,
    approve: bool,
) -> Result<()> {
    if game_running {
        return Err(bad(
            "Close Deadlock first. The game keeps its mods open while it runs.",
        ));
    }
    rebuild_installed(game_root, file, work, approve)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_game_build_follows_steam_inf_and_pak01() {
        let root = std::env::temp_dir().join(format!("sigil-bridge-build-{}", std::process::id()));
        let citadel = root.join("game").join("citadel");
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(super::game_build(&root), None, "no game, no build");
        std::fs::create_dir_all(&citadel).unwrap();
        std::fs::write(citadel.join("pak01_dir.vpk"), b"1234").unwrap();
        std::fs::write(citadel.join("steam.inf"), "ClientVersion=6123\n").unwrap();
        let first = super::game_build(&root).unwrap();
        assert!(first.starts_with("6123:4-"), "{first}");
        std::fs::write(citadel.join("steam.inf"), "ClientVersion=6124\n").unwrap();
        assert_ne!(super::game_build(&root).unwrap(), first, "a new version is a new build");
        let _ = std::fs::remove_dir_all(&root);
    }

    use super::*;
    use morphic::kv3::Format;

    /// A resource container from `(kind, payload)` blocks, laid out the way morphic reads them.
    fn container(blocks: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let table = 16 + blocks.len() * 12;
        let align = |n: usize| n.div_ceil(16) * 16;
        let mut offsets = Vec::new();
        let mut cursor = align(table);
        for (_, payload) in blocks {
            offsets.push(cursor);
            cursor = align(cursor + payload.len());
        }
        let mut out = vec![0u8; cursor];
        out[0..4].copy_from_slice(&(cursor as u32).to_le_bytes());
        out[4..6].copy_from_slice(&12u16.to_le_bytes());
        out[6..8].copy_from_slice(&4u16.to_le_bytes());
        out[8..12].copy_from_slice(&8u32.to_le_bytes());
        out[12..16].copy_from_slice(&(blocks.len() as u32).to_le_bytes());
        for (i, ((kind, payload), offset)) in blocks.iter().zip(&offsets).enumerate() {
            let entry = 16 + i * 12;
            out[entry..entry + 4].copy_from_slice(*kind);
            out[entry + 4..entry + 8].copy_from_slice(&((offset - (entry + 4)) as u32).to_le_bytes());
            out[entry + 8..entry + 12].copy_from_slice(&(payload.len() as u32).to_le_bytes());
            out[*offset..offset + payload.len()].copy_from_slice(payload);
        }
        out
    }

    fn obj(pairs: Vec<(&str, Value)>) -> Value {
        Value::Object(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    fn include(name: &str) -> Value {
        obj(vec![
            ("eType", Value::String("INCLUDE".into())),
            ("child", obj(vec![("eType", Value::String("REFERENCE_COMPILED".into())), ("name", Value::String(name.into()))])),
        ])
    }

    /// A top bar team layout shaped like the game's: styles, then the panel tree.
    fn layout(with_scripts: bool) -> Vec<u8> {
        let mut root = vec![obj(vec![
            ("eType", Value::String("STYLES".into())),
            ("vecChildren", Value::Array(vec![include("panorama/styles/citadel_hud_top_bar.vcss_c")])),
        ])];
        if with_scripts {
            root.push(obj(vec![
                ("eType", Value::String("SCRIPTS".into())),
                ("vecChildren", Value::Array(vec![include("panorama/scripts/game.vjs_c")])),
            ]));
        }
        root.push(obj(vec![("eType", Value::String("PANEL".into())), ("vecChildren", Value::Array(vec![]))]));
        let tree = obj(vec![(
            "m_AST",
            obj(vec![("m_pRoot", obj(vec![("eType", Value::String("ROOT".into())), ("vecChildren", Value::Array(root))]))]),
        )]);
        let kv = kv3::encode(&tree, &Format([7; 16]));
        container(&[(b"RED2", b"edit info"), (b"LaCo", &kv)])
    }

    fn root_children(bytes: &[u8]) -> Vec<Value> {
        let resource = Resource::parse(bytes).unwrap();
        let index = resource.blocks().iter().position(|b| &b.kind == b"LaCo").unwrap();
        let tree = kv3::decode(resource.get_block_by_index(index).unwrap()).unwrap();
        tree.get("m_AST").unwrap().get("m_pRoot").unwrap().get("vecChildren").unwrap().as_array().unwrap().to_vec()
    }

    fn names(section: &Value) -> Vec<String> {
        section
            .get("vecChildren")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .map(|include| include.get("child").unwrap().get("name").unwrap().as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn adds_a_scripts_section_after_the_styles() {
        let patched = patch_layout(&layout(false)).unwrap();
        let children = root_children(&patched);
        assert_eq!(children.len(), 3);
        assert!(is_node(&children[1], "SCRIPTS"));
        assert_eq!(names(&children[1]), vec![SCRIPT.to_string()]);
        assert_eq!(
            children[1].get("vecChildren").unwrap().as_array().unwrap()[0].get("eType").unwrap().as_str(),
            Some("INCLUDE")
        );
        assert!(is_node(&children[2], "PANEL"));
        // The edit info block is carried over untouched.
        assert_eq!(Resource::parse(&patched).unwrap().find_block(*b"RED2"), Some(&b"edit info"[..]));
        // Patching twice is refused, not stacked.
        assert!(patch_layout(&patched).is_err());
    }

    #[test]
    fn joins_a_scripts_section_the_layout_already_has() {
        let patched = patch_layout(&layout(true)).unwrap();
        let children = root_children(&patched);
        assert_eq!(children.len(), 3);
        assert_eq!(names(&children[1]), vec!["panorama/scripts/game.vjs_c".to_string(), SCRIPT.to_string()]);
    }

    #[test]
    fn refuses_what_is_not_a_layout() {
        assert!(patch_layout(b"not a resource").is_err());
        assert!(patch_layout(&container(&[(b"DATA", b"text")])).is_err());
    }

    #[test]
    fn the_script_carries_the_bridge_text_in_a_game_container() {
        let game_script = container(&[(b"RED2", b"edit info"), (b"DATA", b"\"use strict\";\n$.Msg('game');\n")]);
        assert!(usable_container(&game_script));
        assert!(!usable_container(&container(&[(b"RED2", b"x"), (b"DATA", b"a\0b")])));
        assert!(!usable_container(&container(&[(b"SrMa", b"map"), (b"DATA", b"text")])));
        let built = build_script(&game_script).unwrap();
        let resource = Resource::parse(&built).unwrap();
        assert_eq!(resource.data_block().unwrap(), SOURCE.as_bytes());
        assert_eq!(resource.find_block(*b"RED2"), Some(&b"edit info"[..]));
    }

    #[test]
    fn the_script_prints_what_the_reader_reads() {
        use crate::live_match::{BRIDGE_TAG, BRIDGE_VERSION, ROSTER_TAG};
        assert!(SOURCE.contains(&format!("var VERSION = {BRIDGE_VERSION};")));
        assert!(SOURCE.contains(&format!("var TAG = \"{BRIDGE_TAG}\";")));
        assert!(SOURCE.contains(&format!("var ROSTER_TAG = \"{ROSTER_TAG}\";")));
        assert!(!SOURCE.contains('\u{2014}'));
    }

    fn fake_game(root: &Path) {
        let citadel = root.join("game").join("citadel");
        fs::create_dir_all(citadel.join("addons")).unwrap();
        let files = vec![
            (TEAM_LAYOUT.to_string(), layout(false)),
            ("panorama/scripts/tiny.vjs_c".to_string(), container(&[(b"RED2", b"i"), (b"DATA", b"1;")])),
            ("panorama/scripts/bigger.vjs_c".to_string(), container(&[(b"RED2", b"i"), (b"DATA", b"var a = 1; var b = 2;")])),
        ];
        let work = root.join("pak-work");
        let packed = pack(&files, &work).unwrap();
        fs::rename(packed, citadel.join("pak01_dir.vpk")).unwrap();
        let _ = fs::remove_dir_all(work);
    }

    #[test]
    fn builds_finds_and_rebuilds_the_bridge_in_a_game_folder() {
        let root = std::env::temp_dir().join(format!("sigil-live-bridge-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fake_game(&root);
        assert_eq!(check(&root).unwrap().state, BridgeState::NotInstalled);

        let built = build(&root, &root.join("work")).unwrap();
        let dir = read_vpk_directory(&built).unwrap();
        assert!(is_bridge(&dir));
        let script = read_vpk_entry_data(&dir, dir.by_path.get(SCRIPT).unwrap()).unwrap();
        assert_eq!(Resource::parse(&script).unwrap().data_block().unwrap(), SOURCE.as_bytes());

        let addons = root.join("game").join("citadel").join("addons");
        let slot = addons.join("pak07_dir.vpk");
        fs::copy(&built, &slot).unwrap();
        let found = check(&root).unwrap();
        assert_eq!(found.state, BridgeState::Current);
        assert_eq!(found.slot, Some(7));

        // Another top bar mod in a lower slot wins over the bridge.
        let other = pack(&[(TEAM_LAYOUT.to_string(), b"someone's layout".to_vec())], &root.join("other")).unwrap();
        fs::copy(&other, addons.join("pak03_dir.vpk")).unwrap();
        let found = check(&root).unwrap();
        assert_eq!(found.others, vec![3]);
        assert_eq!(found.shadowed(), Some(3));
        fs::remove_file(addons.join("pak03_dir.vpk")).unwrap();

        // An older bridge reads as outdated and a rebuild brings it back.
        let stale = pack(
            &[(TEAM_LAYOUT.to_string(), layout(false)), (SCRIPT.to_string(), b"old".to_vec())],
            &root.join("stale"),
        )
        .unwrap();
        fs::copy(&stale, &slot).unwrap();
        assert_eq!(check(&root).unwrap().state, BridgeState::Outdated);
        assert!(rebuild_installed(&root, &slot, &root.join("work"), false).is_err());
        rebuild_installed(&root, &slot, &root.join("work"), true).unwrap();
        assert_eq!(check(&root).unwrap().state, BridgeState::Current);

        // A pack that holds more than the bridge is never overwritten.
        let merged = pack(
            &[
                (TEAM_LAYOUT.to_string(), layout(false)),
                (SCRIPT.to_string(), b"old".to_vec()),
                ("panorama/images/other.vtex_c".to_string(), b"x".to_vec()),
            ],
            &root.join("merged"),
        )
        .unwrap();
        let before = fs::read(&merged).unwrap();
        assert!(rebuild_installed(&root, &merged, &root.join("work"), true).is_err());
        assert_eq!(fs::read(&merged).unwrap(), before);

        // Switched off.
        fs::rename(&slot, addons.join("pak07_dir.vpk.disabled")).unwrap();
        assert_eq!(check(&root).unwrap().state, BridgeState::Off);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_new_game_build_asks_for_a_rebuild_only_when_the_game_is_closed() {
        let root = std::env::temp_dir().join(format!("sigil-live-update-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        // No game, no build and no plan.
        let none = rebuild_plan(&root, None, false).unwrap();
        assert_eq!((none.build, none.action), (None, RebuildAction::Nothing));

        fake_game(&root);
        let build = game_build(&root).unwrap();
        // The same build is not looked at again, and a new one with no bridge has nothing to do.
        assert_eq!(rebuild_plan(&root, Some(&build), false).unwrap().action, RebuildAction::Unchanged);
        assert_eq!(rebuild_plan(&root, Some("older"), false).unwrap().action, RebuildAction::Nothing);

        // A stale bridge with the game closed is rebuilt now.
        let addons = root.join("game").join("citadel").join("addons");
        let socket = addons.join("pak07_dir.vpk");
        let stale = pack(
            &[
                (TEAM_LAYOUT.to_string(), layout(false)),
                (SCRIPT.to_string(), b"old".to_vec()),
            ],
            &root.join("stale"),
        )
        .unwrap();
        fs::copy(&stale, &socket).unwrap();
        let plan = rebuild_plan(&root, Some("older"), false).unwrap();
        assert_eq!(plan.build.as_deref(), Some(build.as_str()));
        assert!(
            matches!(&plan.action, RebuildAction::RebuildNow { slot: 7, file } if file == &socket),
            "{:?}",
            plan.action
        );
        // The game running turns it into a wait, and the guarded rebuild writes nothing.
        assert!(matches!(
            rebuild_plan(&root, Some("older"), true).unwrap().action,
            RebuildAction::WaitForGame { slot: 7, .. }
        ));
        let before = fs::read(&socket).unwrap();
        assert!(rebuild_when_closed(&root, &socket, &root.join("work"), true, true).is_err());
        assert_eq!(fs::read(&socket).unwrap(), before, "nothing is written under a running game");
        rebuild_when_closed(&root, &socket, &root.join("work"), false, true).unwrap();
        assert_eq!(check(&root).unwrap().state, BridgeState::Current);
        let _ = fs::remove_dir_all(&root);
    }
}
