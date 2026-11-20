//! The shapes a downloaded mod comes in, and what the install makes of each.
//!
//! `install.rs` downloads, unpacks, validates and deploys. This module holds the rules that turn
//! whatever a creator uploaded into one installable mod, gathered from Grimoire (MIT) and
//! Deadlock Mod Manager (behaviour only) and from a corpus of real GameBanana uploads
//! (`tests/install_corpus.rs`, `docs/design/install-robustness.md`):
//!
//! - a bare `.vpk` is the mod, with its numbered parts when it is a `_dir.vpk`;
//! - an archive inside the archive is unpacked in place, a few levels deep;
//! - macOS and Windows junk (`__MACOSX`, `._name`, `.DS_Store`, `Thumbs.db`) is dropped before
//!   anything is read, so an AppleDouble `._pak01_dir.vpk` is never taken for a mod;
//! - read-only flags from an archive never stop a re-extract or a later uninstall;
//! - loose files map to game paths from the first known game folder down, whatever folders
//!   (`game/citadel/`, `My Mod v2/`, `addons/`) sit above it;
//! - an archive Sigil cannot install gets a message that says what was in it and what to do.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use super::{
    ExtractLimits, ToolPaths, check_cancel, detect_archive_format, extract_one, human_bytes, walk_tree,
    ArchiveFormat,
};
use crate::error::{Error, Result};

/// How deep an archive inside an archive is followed. Real uploads nest once (a `.zip` of
/// variant `.zip`s); three levels leave room without inviting a recursion bomb.
pub(super) const MAX_NESTING: usize = 3;

/// The top folders of Deadlock's game content. A loose file's game path starts at the first of
/// these in its archive path; everything above it is the creator's packaging.
pub(super) const GAME_ROOT_FOLDERS: [&str; 15] = [
    "materials",
    "models",
    "sounds",
    "soundevents",
    "particles",
    "panorama",
    "animgraphs",
    "animgraph2",
    "scripts",
    "resource",
    "maps",
    "cfg",
    "shaders",
    "postprocess",
    "lighting",
];

// ---------------------------------------------------------------------------
// Files on disk
// ---------------------------------------------------------------------------

/// Clear the read-only flag. An archive made on Windows can mark its files read-only, and the
/// flag survives the extract: without this the next extract, the cache prune and the
/// uninstall all fail with "Access is denied".
pub(super) fn make_writable(path: &Path) {
    if let Ok(metadata) = fs::metadata(path) {
        let mut permissions = metadata.permissions();
        if permissions.readonly() {
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            let _ = fs::set_permissions(path, permissions);
        }
    }
}

fn make_tree_writable(root: &Path) {
    make_writable(root);
    // A folder also needs to be searchable to be emptied.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = fs::metadata(root)
            && metadata.is_dir()
            && metadata.permissions().mode() & 0o700 != 0o700
        {
            let _ = fs::set_permissions(root, fs::Permissions::from_mode(metadata.permissions().mode() | 0o700));
        }
    }
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => make_tree_writable(&path),
                Ok(kind) if kind.is_file() => make_writable(&path),
                _ => {}
            }
        }
    }
}

/// `remove_dir_all` that also removes read-only files.
pub(super) fn remove_tree(path: &Path) -> Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => {
            make_tree_writable(path);
            fs::remove_dir_all(path).map_err(|error| {
                Error::msg(format!(
                    "Could not clear {}: {error}. Close any program that has a file in it open (an archive tool, an antivirus scan) and try again.",
                    path.display()
                ))
            })
        }
    }
}

/// Operating system clutter that never belongs to a mod.
pub(super) fn is_junk_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == ".ds_store"
        || lower == "thumbs.db"
        || lower == "desktop.ini"
        || lower == "__macosx"
        || lower == ".git"
        || (lower.starts_with("._") && lower.len() > 2)
}

/// Drop junk and clear read-only flags across an extracted tree.
pub(super) fn tidy_tree(root: &Path) -> Result<()> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            continue; // walk_tree refuses it with a clear message
        }
        if is_junk_name(&name) {
            if kind.is_dir() {
                remove_tree(&path)?;
            } else {
                make_writable(&path);
                fs::remove_file(&path)?;
            }
            continue;
        }
        if kind.is_dir() {
            make_writable(&path);
            tidy_tree(&path)?;
        } else if kind.is_file() {
            make_writable(&path);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Bare VPKs and nested archives
// ---------------------------------------------------------------------------

/// A browser or a creator's copy names `pak98_dir (1).vpk`, `pak98_dir(2).vpk` or
/// `pak98_dir - Copy.vpk`. The game reads none of those, but they are the same VPK: this is the
/// lower-case name with that decoration removed.
pub(super) fn canonical_vpk_name(name: &str) -> String {
    let lower = name.to_lowercase();
    let Some(stem) = lower.strip_suffix(".vpk") else {
        return lower;
    };
    let mut stem = stem.trim_end().to_string();
    loop {
        let before = stem.clone();
        for decoration in [" - copy", " copy", "-copy", "_copy"] {
            if let Some(rest) = stem.strip_suffix(decoration) {
                stem = rest.trim_end().to_string();
            }
        }
        if stem.ends_with(')')
            && let Some(open) = stem.rfind('(')
            && stem[open + 1..stem.len() - 1].chars().all(|c| c.is_ascii_digit())
            && open + 2 < stem.len()
        {
            stem = stem[..open].trim_end().to_string();
        }
        if stem == before {
            break;
        }
    }
    format!("{stem}.vpk")
}

/// Treat a bare `.vpk` download or file as an archive holding just that VPK. A `<name>_dir.vpk`
/// brings its `<name>_NNN.vpk` parts from the same folder.
pub(super) fn copy_bare_vpk(
    vpk: &Path,
    destination: &Path,
    limits: &ExtractLimits,
) -> Result<(Vec<String>, u64)> {
    let name = vpk
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .ok_or_else(|| Error::msg(format!("{} has no file name.", vpk.display())))?;
    let mut sources = vec![(vpk.to_path_buf(), name.clone())];
    let canonical = canonical_vpk_name(&name);
    if let Some(base) = canonical.strip_suffix("_dir.vpk")
        && let Some(folder) = vpk.parent()
        && let Ok(entries) = fs::read_dir(folder)
    {
        for entry in entries.flatten() {
            let sibling = entry.file_name().to_string_lossy().into_owned();
            let lower = sibling.to_lowercase();
            if super::vpk_chunk_base(&lower) == Some(base) && entry.path().is_file() {
                sources.push((entry.path(), sibling));
            }
        }
    }
    let mut files = Vec::new();
    let mut total: u64 = 0;
    for (source, file_name) in sources {
        let size = fs::metadata(&source)?.len();
        total += size;
        if total > limits.max_total_bytes {
            return Err(Error::msg(format!(
                "{} is larger than Sigil installs ({} at most).",
                vpk.display(),
                human_bytes(limits.max_total_bytes)
            )));
        }
        let target = destination.join(&file_name);
        fs::copy(&source, &target)?;
        make_writable(&target);
        files.push(file_name);
    }
    files.sort();
    Ok((files, total))
}

fn is_archive_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".zip") || lower.ends_with(".7z") || lower.ends_with(".rar")
}

/// `name.part2.rar` and later volumes of a split RAR: the first volume reads them.
fn is_later_rar_volume(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let Some(stem) = lower.strip_suffix(".rar") else {
        return false;
    };
    let Some((_, number)) = stem.rsplit_once(".part") else {
        return false;
    };
    number.parse::<u32>().is_ok_and(|value| value > 1)
}

/// The password message every reader uses, so a nested archive and the caller see one wording.
pub(super) fn password_message(archive: &Path) -> String {
    format!(
        "{} is password protected, so Sigil cannot open it. Deadlock mod managers never ask for archive passwords: if the mod page gives one, unpack the archive with 7-Zip using it and import the .vpk inside.",
        archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    )
}

/// True for a file that is empty or starts like a web page: never an archive, whatever its name.
pub(super) fn not_an_archive(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 256];
    let read = fs::File::open(path).and_then(|mut file| file.read(&mut head)).unwrap_or(0);
    if read == 0 {
        return true;
    }
    let text = String::from_utf8_lossy(&head[..read]).to_ascii_lowercase();
    let text = text.trim_start();
    text.starts_with("<!doctype html") || text.starts_with("<html") || text.starts_with("<?xml") || text.starts_with("{\"")
}

pub(super) fn is_password_error(error: &Error) -> bool {
    error.to_string().contains("password protected")
}

/// Unpack every archive found inside `root`, in a folder named after it, then look again, up to
/// [`MAX_NESTING`] levels. An inner archive that cannot be opened stays as it is with a warning:
/// the classifier reports it if nothing else is installable.
pub(super) fn expand_nested(
    root: &Path,
    limits: &ExtractLimits,
    tools: &ToolPaths,
    cancel: Option<&AtomicBool>,
    warnings: &mut Vec<String>,
) -> Result<()> {
    let mut failed: Vec<String> = Vec::new();
    for _ in 0..MAX_NESTING {
        let inner: Vec<String> = walk_tree(root)?
            .into_iter()
            .map(|(path, _)| path)
            .filter(|path| is_archive_name(path) && !failed.contains(path))
            .collect();
        if inner.is_empty() {
            return Ok(());
        }
        for relative in inner {
            check_cancel(cancel)?;
            let file_name = relative.rsplit('/').next().unwrap_or(&relative).to_string();
            let path = root.join(&relative);
            if is_later_rar_volume(&file_name) {
                if !path.exists() {
                    continue;
                }
                // Read through part 1; on its own it is not an archive Sigil can open.
                failed.push(relative);
                continue;
            }
            let format = detect_archive_format(&path);
            if !matches!(format, ArchiveFormat::Zip | ArchiveFormat::SevenZip | ArchiveFormat::Rar) {
                failed.push(relative);
                continue;
            }
            let stem = file_name
                .rsplit_once('.')
                .map(|(stem, _)| stem.to_string())
                .unwrap_or_else(|| file_name.clone());
            let stem = stem.strip_suffix(".part1").or_else(|| stem.strip_suffix(".part01")).unwrap_or(&stem).to_string();
            let parent = path.parent().unwrap_or(root).to_path_buf();
            let mut target = parent.join(&stem);
            let mut counter = 2;
            while target.exists() {
                target = parent.join(format!("{stem} ({counter})"));
                counter += 1;
            }
            fs::create_dir_all(&target)?;
            match extract_one(format, &path, &target, limits, tools, cancel) {
                Ok(_) => {
                    make_writable(&path);
                    fs::remove_file(&path)?;
                    tidy_tree(&target)?;
                }
                Err(error) if super::is_cancelled(&error) => return Err(error),
                Err(error) => {
                    let _ = remove_tree(&target);
                    warnings.push(format!("The archive inside it, {relative}, could not be opened: {error}"));
                    failed.push(relative);
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Loose files
// ---------------------------------------------------------------------------

/// Split an archive path into the creator's packaging and the game path, at the first known game
/// folder. `My Mod/game/citadel/materials/x.vtex_c` gives `("My Mod/game/citadel/",
/// "materials/x.vtex_c")`. `None` for a file under no game folder (a readme, a preview).
pub(super) fn split_at_game_root(relative: &str) -> Option<(String, String)> {
    let segments: Vec<&str> = relative.split('/').collect();
    let at = segments[..segments.len().saturating_sub(1)]
        .iter()
        .position(|segment| GAME_ROOT_FOLDERS.contains(&segment.to_ascii_lowercase().as_str()))?;
    let prefix: String = segments[..at].iter().map(|segment| format!("{segment}/")).collect();
    Some((prefix, segments[at..].join("/")))
}

/// The game path of `relative` in the mod folder `prefix`: the path from its game folder down,
/// or for a font outside any game folder (`fonts/Inter.ttf`, as font packs ship them for a
/// script to copy) its place under `panorama/fonts/`, where Sigil's font install finds it.
pub(super) fn game_path_under(prefix: &str, relative: &str) -> Option<String> {
    let rest = relative.strip_prefix(prefix)?;
    let first = rest.split('/').next()?.to_ascii_lowercase();
    if rest.contains('/') && GAME_ROOT_FOLDERS.contains(&first.as_str()) {
        return Some(rest.to_string());
    }
    loose_font_path(relative)
}

/// `panorama/fonts/<name>` for a `.ttf` or `.otf` outside every game folder.
pub(super) fn loose_font_path(relative: &str) -> Option<String> {
    if split_at_game_root(relative).is_some() {
        return None;
    }
    let name = relative.rsplit('/').next().unwrap_or(relative);
    let lower = name.to_ascii_lowercase();
    (lower.ends_with(".ttf") || lower.ends_with(".otf")).then(|| format!("panorama/fonts/{name}"))
}

/// A loose file the game reads as it is, without compiling: compiled `_c` files, map VPKs,
/// fonts, localization text and Panorama movies. Everything else at a game path is a source
/// file or a note and is left out with a warning.
pub(super) fn is_loose_payload(game_path: &str) -> bool {
    let lower = game_path.to_ascii_lowercase();
    if lower.ends_with("_c") {
        return true;
    }
    let first = lower.split('/').next().unwrap_or("");
    match first {
        "maps" => lower.ends_with(".vpk"),
        "resource" => lower.ends_with(".txt") && lower.contains("localization/"),
        "panorama" => {
            lower.ends_with(".ttf")
                || lower.ends_with(".otf")
                || lower.ends_with(".webm")
                || (lower.starts_with("panorama/fonts/") && lower.ends_with(".conf"))
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

/// What a file in an archive is, for the "nothing to install" message.
fn kind_of(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    let extension = name.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("");
    if name == "gameinfo.gi" || extension == "gi" {
        return "gameinfo";
    }
    if crate::cursors::is_cursor_file_name(name) {
        return "cursor";
    }
    match extension {
        "cfg" | "vcfg" => "config",
        "vpk" => "vpk",
        "exe" | "bat" | "cmd" | "ps1" | "msi" | "py" | "sh" | "vbs" | "jar" | "lnk" | "reg" | "dll" => "program",
        "vmat" | "vmdl" | "vpcf" | "vsndevts" | "vtex" | "vxml" | "vcss" | "vjs" | "vanmgrph" | "fbx" | "obj" | "dmx" | "smd" | "blend" | "psd" | "tga" | "wav" | "mp3" | "vsnd" => "source",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" => "image",
        "txt" | "md" | "pdf" | "rtf" | "url" | "html" | "htm" | "docx" => "note",
        "zip" | "7z" | "rar" => "archive",
        "ttf" | "otf" => "font",
        _ => "other",
    }
}

/// The message for an archive with nothing Sigil can install: what it holds, then what to do.
pub(super) fn nothing_installable(files: &[(String, u64)]) -> String {
    if files.is_empty() {
        return "The archive is empty: it unpacked to no files at all. The upload on GameBanana is broken; tell the creator, or pick another file of the mod.".to_string();
    }
    let mut groups: BTreeMap<&'static str, Vec<&str>> = BTreeMap::new();
    for (path, _) in files {
        groups.entry(kind_of(path)).or_default().push(path.as_str());
    }
    let listing = |kind: &str| -> String {
        let names = &groups[kind];
        let shown: Vec<&str> = names.iter().take(4).copied().collect();
        let more = names.len().saturating_sub(shown.len());
        if more > 0 {
            format!("{} and {more} more", shown.join(", "))
        } else {
            shown.join(", ")
        }
    };
    let found: Vec<String> = groups
        .iter()
        .map(|(kind, names)| format!("{} {kind} file{}", names.len(), if names.len() == 1 { "" } else { "s" }))
        .collect();
    let mut message = format!(
        "Nothing in this archive is a mod Sigil can install. No compiled _c files and no .vpk were found. It holds {}.",
        found.join(", ")
    );
    let advice: &[(&str, &str)] = &[
        ("gameinfo", "It replaces gameinfo.gi ({}). Sigil manages that file itself and never installs a replacement: a copied gameinfo.gi drops the addons line and breaks every mod after the next game update. Performance configs like this one are applied from Sigil's Performance page, which merges the author's settings into your own gameinfo.gi and takes them out again exactly."),
        ("config", "It is a config mod ({}). Configs are not VPK mods: paste the commands into Sigil's Autoexec page, or copy the file into game/citadel/cfg yourself."),
        ("cursor", "It is a cursor pack ({}). Import it from Locker > Cursors: the game reads cursors as loose files, not from a VPK."),
        ("source", "It ships uncompiled source files ({}). The game reads only compiled _c files: the creator has to compile the mod with the Workshop Tools."),
        ("program", "It ships a program or script ({}). Sigil never runs programs from a mod; read the mod page for what it does."),
        ("archive", "It holds an archive Sigil could not open ({}). See the warning above for why."),
    ];
    for (kind, text) in advice {
        if groups.contains_key(kind) {
            message.push(' ');
            message.push_str(&text.replace("{}", &listing(kind)));
        }
    }
    if !groups.keys().any(|kind| advice.iter().any(|(known, _)| known == kind)) {
        message.push_str(" If the mod page describes manual steps, follow them; otherwise report the file to the creator.");
    }
    message
}
