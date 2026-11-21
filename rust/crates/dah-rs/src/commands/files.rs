//! `dah files`: browse, preview, search and compare the files of a VPK or a project folder, and
//! export them into a mod project. Backs the agent's file tools.
//!
//! A source is `game` (citadel's `pak01_dir.vpk`), `core`, an installed addon's file name
//! (`pak03_dir.vpk`) or a path inside the workspace. Everything here reads, except `export`,
//! which writes into a mod project only (`deadlock_core::explorer::export` refuses any other
//! destination). Nothing writes to a game install.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use deadlock_core::explorer::{self, Listing, compare, export, preview, search};

use crate::error::{CliError, Result};
use crate::paths;
use crate::util::print_json;

/// Rows a listing prints before it says how many more there are.
const LIST_CAP: usize = 400;
/// Preview text handed to the model, in bytes.
const PREVIEW_CAP: usize = 24 * 1024;
const REFERENCE_CAP: usize = 60;
const COMPARE_ROWS_CAP: usize = 300;

pub struct SourceArgs {
    pub source: String,
    pub workspace: Option<PathBuf>,
}

fn game_root() -> Option<PathBuf> {
    deadlock_core::discovery::read_game_status(None)
        .ok()
        .and_then(|status| status.install)
        .map(|install| install.game_root)
}

/// Resolve a source name to a path the explorer opens.
pub fn resolve_source(args: &SourceArgs) -> Result<PathBuf> {
    let workspace = paths::workspace_root(args.workspace.as_deref())?;
    resolve_source_in(&args.source, &workspace, game_root().as_deref())
}

pub fn resolve_source_in(source: &str, workspace: &Path, game_root: Option<&Path>) -> Result<PathBuf> {
    let name = source.trim();
    if name.is_empty() {
        return Err(CliError::new("files_source", "Name a source: game, core, an addon file or a workspace path."));
    }
    let need_game = || {
        game_root.map(Path::to_path_buf).ok_or_else(|| {
            CliError::new("files_game", "No Deadlock install was found. Set DAH_GAME_DIR to the Deadlock folder.")
        })
    };
    match name.to_ascii_lowercase().as_str() {
        "game" | "citadel" => return Ok(need_game()?.join("game/citadel/pak01_dir.vpk")),
        "core" => return Ok(need_game()?.join("game/core/pak01_dir.vpk")),
        _ => {}
    }
    if !name.contains(['/', '\\']) && name.to_ascii_lowercase().ends_with(".vpk") {
        let root = need_game()?;
        for source in explorer::game_sources(&root) {
            if source
                .path
                .file_name()
                .is_some_and(|file| file.to_string_lossy().eq_ignore_ascii_case(name))
                && source.label.starts_with("Addon")
            {
                return Ok(source.path);
            }
        }
        return Err(CliError::new("files_source", format!("{name} is not an installed addon.")));
    }
    let candidate = if Path::new(name).is_absolute() {
        PathBuf::from(name)
    } else {
        workspace.join(name)
    };
    let candidate = paths::normalize_lexical(&candidate);
    if !paths::is_inside(workspace, &candidate) {
        return Err(CliError::new(
            "files_source",
            format!("{name} is outside the workspace. Use game, core, an addon file name or a workspace path."),
        ));
    }
    // A link inside the workspace must not lead out of it.
    if let (Ok(real), Ok(real_workspace)) = (candidate.canonicalize(), workspace.canonicalize())
        && !real.starts_with(&real_workspace)
    {
        return Err(CliError::new("files_source", format!("{name} leads outside the workspace.")));
    }
    Ok(candidate)
}

fn open(args: &SourceArgs) -> Result<Listing> {
    let path = resolve_source(args)?;
    explorer::open(&path).map_err(|error| CliError::new("files_open", error.to_string()))
}

pub fn sources(workspace: Option<PathBuf>, json: bool) -> Result<()> {
    let _ = paths::workspace_root(workspace.as_deref())?;
    let list: Vec<Value> = match game_root() {
        Some(root) => explorer::game_sources(&root)
            .into_iter()
            .map(|source| {
                let name = match source.label.as_str() {
                    "Game files (citadel)" => "game".to_string(),
                    "Engine files (core)" => "core".to_string(),
                    _ => source
                        .path
                        .file_name()
                        .map(|file| file.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                };
                json!({"source": name, "label": source.label})
            })
            .collect(),
        None => Vec::new(),
    };
    if json {
        print_json(&json!({"ok": true, "sources": list}));
    } else {
        for item in &list {
            println!("{}  {}", item["source"].as_str().unwrap_or(""), item["label"].as_str().unwrap_or(""));
        }
    }
    Ok(())
}

pub fn list(args: &SourceArgs, folder: &str, json: bool) -> Result<()> {
    let listing = open(args)?;
    let folder = folder.trim().trim_matches('/');
    let children = listing.children(folder);
    let shown: Vec<Value> = children
        .iter()
        .take(LIST_CAP)
        .map(|node| {
            if node.folder {
                json!({"name": node.name, "path": node.path, "folder": true, "files": node.files, "size": node.size})
            } else {
                json!({"name": node.name, "path": node.path, "size": node.size})
            }
        })
        .collect();
    if json {
        print_json(&json!({
            "ok": true,
            "source": listing.label,
            "folder": folder,
            "totalFiles": listing.files.len(),
            "rows": shown,
            "more": children.len().saturating_sub(LIST_CAP),
        }));
    } else {
        for node in children.iter().take(LIST_CAP) {
            println!("{}{}", node.path, if node.folder { "/" } else { "" });
        }
    }
    Ok(())
}

pub fn show(args: &SourceArgs, path: &str, json: bool) -> Result<()> {
    let listing = open(args)?;
    let bytes = listing
        .read(path)
        .map_err(|error| CliError::new("files_read", error.to_string()))?;
    let preview = preview::preview(path, &bytes);
    let mut text = preview.text.clone();
    let mut truncated = preview.truncated;
    if text.len() > PREVIEW_CAP {
        let mut cut = PREVIEW_CAP;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        truncated = true;
    }
    if json {
        print_json(&json!({
            "ok": true,
            "path": path,
            "bytes": bytes.len(),
            "kind": preview.kind,
            "format": preview.format,
            "text": text,
            "truncated": truncated,
            "references": preview.references.iter().take(REFERENCE_CAP).collect::<Vec<_>>(),
            "note": preview.note,
            "editableAs": export::editable_path(path),
        }));
    } else {
        println!("{} ({})", path, preview.format);
        println!("{text}");
    }
    Ok(())
}

pub struct SearchArgs {
    pub source: SourceArgs,
    pub query: String,
    pub under: Option<String>,
    pub case_sensitive: bool,
    pub max_hits: Option<usize>,
    pub json: bool,
}

pub fn find_text(args: &SearchArgs) -> Result<()> {
    let listing = open(&args.source)?;
    let mut options = search::SearchOptions::new(args.query.clone());
    options.case_sensitive = args.case_sensitive;
    options.path_filter = args.under.clone().filter(|under| !under.trim().is_empty());
    options.max_hits = args.max_hits.unwrap_or(60).clamp(1, search::DEFAULT_MAX_HITS);
    let control = search::SearchControl::default();
    let report = search::search(&listing, &options, &control, &|| {})
        .map_err(|error| CliError::new("files_search", error))?;
    if args.json {
        print_json(&json!({
            "ok": true,
            "source": listing.label,
            "filesSearched": report.files_searched,
            "filesMatched": report.files_matched,
            "filesSkipped": report.files_skipped,
            "capped": report.capped,
            "hits": report.hits.iter().map(|hit| json!({
                "path": hit.path, "line": hit.line, "text": hit.text,
            })).collect::<Vec<_>>(),
        }));
    } else {
        for hit in &report.hits {
            println!("{}:{}: {}", hit.path, hit.line, hit.text);
        }
    }
    Ok(())
}

pub fn diff(left: &SourceArgs, right: &SourceArgs, path: Option<&str>, json: bool) -> Result<()> {
    let left_listing = open(left)?;
    let right_listing = open(right)?;
    if let Some(path) = path {
        let text_of = |listing: &Listing| -> Result<String> {
            let bytes = listing
                .read(path)
                .map_err(|error| CliError::new("files_read", error.to_string()))?;
            Ok(preview::preview(path, &bytes).text)
        };
        let lines = compare::diff_lines(&text_of(&left_listing)?, &text_of(&right_listing)?);
        let changed: Vec<Value> = lines
            .iter()
            .filter(|line| line.op != compare::LineOp::Same)
            .take(COMPARE_ROWS_CAP)
            .map(|line| json!({"op": line.op, "left": line.left, "right": line.right, "text": line.text}))
            .collect();
        if json {
            print_json(&json!({"ok": true, "path": path, "changedLines": changed}));
        } else {
            for line in &changed {
                println!("{} {}", line["op"], line["text"].as_str().unwrap_or(""));
            }
        }
        return Ok(());
    }
    let tree = compare::compare_trees(&left_listing, &right_listing);
    let rows: Vec<Value> = tree
        .rows
        .iter()
        .filter(|row| row.change != compare::Change::Same)
        .take(COMPARE_ROWS_CAP)
        .map(|row| json!({"path": row.path, "change": row.change, "left": row.left_size, "right": row.right_size}))
        .collect();
    if json {
        print_json(&json!({
            "ok": true,
            "left": left_listing.label,
            "right": right_listing.label,
            "onlyLeft": tree.only_left,
            "onlyRight": tree.only_right,
            "changed": tree.changed,
            "same": tree.same,
            "rows": rows,
        }));
    } else {
        println!(
            "{} only left, {} only right, {} changed, {} same",
            tree.only_left, tree.only_right, tree.changed, tree.same
        );
    }
    Ok(())
}

pub struct ExportArgs {
    pub source: SourceArgs,
    pub paths: Vec<String>,
    pub editable: bool,
    pub overwrite: bool,
    pub json: bool,
}

pub fn export_files(args: &ExportArgs) -> Result<()> {
    let listing = open(&args.source)?;
    let project = paths::project_dir(args.source.workspace.as_deref())?;
    let mode = if args.editable {
        export::ExportMode::Editable
    } else {
        export::ExportMode::Original
    };
    let report = export::export(&listing, &args.paths, &project, mode, args.overwrite)
        .map_err(|error| CliError::new("files_export", error.to_string()))?;
    if args.json {
        print_json(&json!({
            "ok": true,
            "project": project.display().to_string(),
            "mode": mode,
            "written": report.written.iter().take(LIST_CAP).map(|file| json!({
                "path": file.path, "bytes": file.bytes,
            })).collect::<Vec<_>>(),
            "writtenCount": report.written.len(),
            "skipped": report.skipped.iter().take(LIST_CAP).map(|(path, why)| json!({"path": path, "reason": why})).collect::<Vec<_>>(),
            "newTargets": report.new_targets,
        }));
    } else {
        println!("Exported {} file(s) into {}", report.written.len(), project.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_outside_the_workspace_are_refused() {
        let workspace = Path::new("/tmp/dah-files-ws");
        assert!(resolve_source_in("/etc", workspace, None).is_err());
        assert!(resolve_source_in("../outside", workspace, None).is_err());
        assert!(resolve_source_in("", workspace, None).is_err());
        assert_eq!(
            resolve_source_in("mods/a/staging", workspace, None).unwrap(),
            workspace.join("mods/a/staging")
        );
        let game = Path::new("/games/Deadlock");
        assert_eq!(
            resolve_source_in("game", workspace, Some(game)).unwrap(),
            game.join("game/citadel/pak01_dir.vpk")
        );
        assert!(resolve_source_in("game", workspace, None).is_err());
    }
}
