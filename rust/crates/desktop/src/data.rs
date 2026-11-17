//! Real reads from `deadlock-core` at startup.
//!
//! Only reads happen here: game status, the cached Steam news feed, an optional workspace
//! project and the latest release for that project. A missing install, cache or release leaves
//! the matching screen empty instead of inventing data. The review queue loads the game catalog,
//! so it only runs from a local scene or an explicit action, never on every frame.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use deadlock_core::{
    addons, autoexec, deploy, discovery, game_catalog, gamebanana, locker, migrate, patch_notes,
    project, release,
};

use rune::TextEdit;

use crate::app::State;
use crate::model::{
    self, AddonLibrary, AddonRow, BrowseCard, BrowsePageView, Compat, ConflictRow,
    DeployConflictView, DeployPlanView, HistoryEntry, PathMove, Project, ProjectState, PublishInfo,
    Reference, ReleaseInfo, ReviewItem, ReviewQueue, Target, UploadInfo,
};

pub fn init_state() -> State {
    let mut state = State::default();
    state.prefs = crate::prefs::Prefs::load();
    crate::prefs::apply_game_dir(&state.prefs);
    crate::ui::set_look(state.prefs.theme, state.prefs.accent_color());
    state.screen = state.prefs.startup;
    state.paths.data = data_dir().display().to_string();
    let game_root = apply_game_status(&mut state);
    load_cached_patches(&mut state);
    load_workspace_project(&mut state);
    state.watch = read_watch_state();
    state.capabilities = build_capabilities();
    refresh_subscriptions(&mut state);
    if let Ok(Some(host)) = deadlock_core::subscription::load_host_config() {
        state.host_model = TextEdit::new(host.model);
    }
    refresh_release_with(&mut state, game_root.as_deref());
    state
}

/// The app data directory. `DAH_DATA_DIR` overrides the platform default.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("DAH_DATA_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        home.map(|h| h.join("Library/Application Support/DeadlockAgentHarness"))
            .unwrap_or_else(|| PathBuf::from(".dah"))
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA")
            .map(|h| PathBuf::from(h).join("DeadlockAgentHarness"))
            .unwrap_or_else(|| PathBuf::from(".dah"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(|h| PathBuf::from(h).join("DeadlockAgentHarness"))
            .or_else(|| home.map(|h| h.join(".local/share/deadlock-harness")))
            .unwrap_or_else(|| PathBuf::from(".dah"))
    }
}

pub fn workspace_dir() -> PathBuf {
    if let Some(dir) =
        std::env::var_os("DAH_WORKSPACE_DIR").or_else(|| std::env::var_os("DAH_WORKSPACE"))
    {
        return PathBuf::from(dir);
    }
    data_dir().join("workspace")
}

fn apply_game_status(state: &mut State) -> Option<PathBuf> {
    match discovery::read_game_status(None) {
        Ok(status) => {
            let Some(install) = &status.install else {
                state.game_detected = false;
                state.game_label = "Game not detected".to_string();
                // A lasting condition: the banner shows it (`banner::Gameinfo::NoGame`), not a
                // toast that comes back on every launch.
                return None;
            };
            state.game_detected = true;
            state.paths.game = install.game_root.display().to_string();
            state.game_root = Some(install.game_root.clone());
            state.addons_dir = Some(install.addons_dir.display().to_string());
            state.game_label = match &status.addons {
                Some(addons) => match addons.next_free_slot {
                    Some(slot) => format!("citadel · slot {slot:02} free"),
                    None => "citadel · no free slot".to_string(),
                },
                None => "citadel · addons missing".to_string(),
            };
            // A gameinfo.gi without the addons path shows as the shell's banner (`crate::banner`).
            Some(install.game_root.clone())
        }
        Err(error) => {
            state.game_detected = false;
            state.game_label = "Game not detected".to_string();
            state.status = format!("Game discovery failed: {error}");
            None
        }
    }
}

/// Find the game again (after the folder changed in Settings, a repair, or Rescan) and reread
/// what depends on it: the addon slots, the launch check and autoexec.cfg.
pub fn rescan_game(state: &mut State) {
    if state.demo {
        return;
    }
    state.game_root = None;
    state.addons_dir = None;
    state.paths.game = model::empty_paths().game;
    apply_game_status(state);
    state.addons = AddonLibrary::default();
    if state.game_root.is_some() {
        start_addon_scan(state);
    }
    state.launch.loaded = false;
    state.launch.read = None;
    if !state.launch_job.is_running() {
        start_launch_read(state);
    }
    state.autoexec.loaded = false;
    crate::banner::recheck(state);
}

fn load_cached_patches(state: &mut State) {
    let workspace = workspace_dir();
    let path = patch_notes::patch_note_cache_path(&workspace);
    let Ok(cache) = patch_notes::load_patch_note_cache(&path) else {
        return;
    };
    let notes: Vec<model::PatchNote> = cache
        .notes
        .iter()
        .filter(|note| note.is_patch)
        .take(40)
        .map(|note| {
            let body: Vec<String> = patch_notes::strip_patch_note_html(&note.contents)
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .take(14)
                .map(str::to_string)
                .collect();
            let date: String = note.published_at.chars().take(10).collect();
            model::PatchNote {
                title: note.title.trim().to_string(),
                date,
                build: String::new(),
                body,
                changed: Vec::new(),
                targets: Vec::new(),
                is_new: false,
                source: "Steam news · cached".to_string(),
            }
        })
        .collect();
    if !notes.is_empty() {
        state.patches = notes;
        state.selected_patch = 0;
    }
}

fn load_workspace_project(state: &mut State) {
    let root = workspace_dir();
    let mut dirs = Vec::new();

    if root.join("project.json").is_file() {
        dirs.push(root.clone());
    }

    let mods = root.join("mods");
    if let Ok(read_dir) = fs::read_dir(&mods) {
        let mut children: Vec<PathBuf> = read_dir
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.join("project.json").is_file())
            .collect();
        children.sort();
        dirs.extend(children);
    }

    let mut projects = Vec::new();
    let mut first_dir: Option<PathBuf> = None;

    for dir in dirs {
        let Ok(project) = project::read_project(&dir) else {
            continue;
        };
        if first_dir.is_none() {
            first_dir = Some(dir.clone());
        }
        projects.push(map_project(&project));
    }

    state.projects = projects;
    state.selected_project = 0;

    if let Some(dir) = first_dir {
        state.paths.workspace = dir.display().to_string();
        state.paths.staging = dir.join("staging").display().to_string();
        if state.status.is_empty()
            && let Some(project) = state.projects.first()
        {
            state.status = format!("Opened {} from {}.", project.name, dir.display());
        }
    }
}

fn map_project(project: &project::ModProject) -> Project {
    let history = project
        .history
        .iter()
        .rev()
        .take(12)
        .map(|entry| HistoryEntry {
            version: entry.action.clone(),
            date: entry.timestamp.chars().take(10).collect(),
            note: entry.note.clone().unwrap_or_else(|| entry.action.clone()),
        })
        .collect();
    Project {
        name: project.name.clone(),
        version: project.version.clone(),
        slot: project.slot.unwrap_or(0),
        state: ProjectState::Draft,
        updated: project
            .updated_at
            .chars()
            .take(16)
            .collect::<String>()
            .replace('T', " "),
        summary: format!(
            "Read from {}. {} target{}, {} reference{}.",
            project.slug,
            project.targets.len(),
            if project.targets.len() == 1 { "" } else { "s" },
            project.references.len(),
            if project.references.len() == 1 {
                ""
            } else {
                "s"
            }
        ),
        targets: project
            .targets
            .iter()
            .map(|path| Target {
                path: path.clone(),
                kind: kind_for(path),
                status: Compat::Unchanged,
            })
            .collect(),
        references: project
            .references
            .iter()
            .map(|vpk| Reference {
                vpk: vpk.clone(),
                mode: "read".to_string(),
            })
            .collect(),
        history,
    }
}

fn kind_for(path: &str) -> String {
    if path.ends_with(".vmat_c") {
        "material"
    } else if path.ends_with(".vtex_c") {
        "texture"
    } else if path.ends_with(".vmdl_c") {
        "model"
    } else if path.ends_with(".vpcf_c") {
        "particle"
    } else if path.ends_with(".vsnd_c") {
        "sound"
    } else if path.ends_with(".vxml_c") {
        "layout"
    } else if path.ends_with(".vcss_c") {
        "style"
    } else {
        "file"
    }
    .to_string()
}

/// Start the Release scene with the real workspace: project, game status, latest
/// release and deploy plan. The review queue is loaded here too because a scene
/// load is an explicit moment, not a render frame.
pub fn init_release_state() -> State {
    let mut state = init_state();
    state.screen = model::Screen::Release;
    load_review_queue(&mut state);
    state
}

/// Re-read the latest release and its deploy plan. Small files only: the release
/// manifest, its VPK directory tree and the addon VPK trees in a game install.
pub fn refresh_release(state: &mut State) {
    let root = selected_workspace(state);
    if !root.join("project.json").is_file() {
        refresh_release_with(state, None);
        return;
    }
    let game_root = discovery::read_game_status(None)
        .ok()
        .and_then(|status| status.install.map(|install| install.game_root));
    refresh_release_with(state, game_root.as_deref());
}

fn refresh_release_with(state: &mut State, game_root: Option<&Path>) {
    let root = selected_workspace(state);
    state.release.info = None;
    state.release.plan = None;
    state.release.plan_error = None;
    state.release.publish = None;
    state.release.upload = None;
    state.release.upload_error = None;
    state.release.lookup_error = None;

    let manifest_path = root.join("project.json");
    if !manifest_path.is_file() {
        state.release.lookup_error = Some(format!(
            "No project manifest at {}. A release is read from the open workspace.",
            manifest_path.display()
        ));
        return;
    }

    let manifest = match release::latest_release(&root) {
        Ok(Some(manifest)) => manifest,
        Ok(None) => return,
        Err(error) => {
            state.release.lookup_error = Some(format!(
                "Could not read builds/ in {}: {error}",
                root.display()
            ));
            return;
        }
    };

    let release_dir = release::release_dir(&root, &manifest.version);
    let vpk_present = release_dir.join(&manifest.vpk_file).is_file();

    state.release.info = Some(ReleaseInfo {
        version: manifest.version.clone(),
        slot: manifest.slot,
        vpk_file: manifest.vpk_file.clone(),
        chunk_files: manifest.chunk_files.clone(),
        entries: manifest.entries,
        total_bytes: manifest.total_bytes,
        created_at: manifest.created_at.clone(),
        baseline_captured_at: manifest.baseline_captured_at.clone(),
        notes: manifest.notes.clone(),
        dir: release_dir.display().to_string(),
        vpk_present,
    });

    state.release.publish = Some(read_publish(&root, &manifest.version));
    match read_upload(&root.join("publish").join(&manifest.version)) {
        Ok(upload) => state.release.upload = upload,
        Err(error) => state.release.upload_error = Some(error),
    }

    let Some(game_root) = game_root else {
        return;
    };

    match deploy::plan_deploy(game_root, &release_dir, &manifest, None) {
        Ok(plan) => {
            state.release.plan = Some(DeployPlanView {
                slot: plan.slot,
                target: plan.target.display().to_string(),
                chunk_targets: plan
                    .chunk_targets
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect(),
                replace_required: plan.replace_required,
                conflicts: plan
                    .conflicts
                    .into_iter()
                    .map(|conflict| DeployConflictView {
                        slot: conflict.slot,
                        file: conflict.file,
                        paths: conflict.paths,
                    })
                    .collect(),
                gameinfo_repair: plan.gameinfo_repair,
                warnings: plan.warnings,
                entry_count: plan.entries.len(),
                entry_preview: plan.entries.into_iter().take(8).collect(),
            });
        }
        Err(error) => state.release.plan_error = Some(error.to_string()),
    }
}

/// The workspace the screen should read. The default `Paths` labels an
/// unselected workspace "Not selected"; that is not a real path.
fn selected_workspace(state: &State) -> PathBuf {
    let raw = state.paths.workspace.trim();
    if raw.is_empty() || raw == "Not selected" {
        workspace_dir()
    } else {
        PathBuf::from(raw)
    }
}

fn read_publish(project_dir: &Path, version: &str) -> PublishInfo {
    let dir = project_dir.join("publish").join(version);
    let metadata_path = dir.join("metadata.json");
    let metadata_present = metadata_path.is_file();
    let mut archive_file = None;
    let mut archive_sha256 = None;
    let mut archive_bytes = None;

    if metadata_present
        && let Ok(text) = fs::read_to_string(&metadata_path)
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
    {
        let archive = value.get("archive");
        archive_file = archive
            .and_then(|archive| archive.get("file"))
            .and_then(|file| file.as_str())
            .map(str::to_string);
        archive_sha256 = archive
            .and_then(|archive| archive.get("sha256"))
            .and_then(|sha| sha.as_str())
            .map(str::to_string);
        archive_bytes = archive
            .and_then(|archive| archive.get("bytes"))
            .and_then(|bytes| bytes.as_u64());
    }

    PublishInfo {
        dir: dir.display().to_string(),
        archive_file,
        archive_sha256,
        archive_bytes,
    }
}

/// Read the durable handoff record next to the prepared release. A missing file
/// means no handoff has been approved yet, which is the `none` state.
fn read_upload(folder: &Path) -> Result<Option<UploadInfo>, String> {
    let path = folder.join("upload.json");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Could not read {}: {error}", path.display())),
    };
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|error| format!("{} is not valid JSON: {error}", path.display()))?;

    let string = |key: &str| {
        value
            .get(key)
            .and_then(|item| item.as_str())
            .map(str::to_string)
    };
    let state = string("state").unwrap_or_else(|| "unknown".to_string());
    let archive_file = string("archiveFile").unwrap_or_default();
    let archive_sha256 = string("archiveSha256").unwrap_or_default();
    let archive_bytes = value
        .get("archiveBytes")
        .and_then(|bytes| bytes.as_u64())
        .unwrap_or(0);
    let submit_url = string("submitUrl").unwrap_or_default();
    let submission_url = string("submissionUrl").filter(|url| !url.is_empty());
    let verified = value
        .get("verified")
        .and_then(|verified| verified.as_bool())
        .unwrap_or(false);

    Ok(Some(UploadInfo {
        state,
        archive_file,
        archive_sha256,
        archive_bytes,
        submit_url,
        submission_url,
        verified,
        created_at: string("createdAt").map(|stamp| short_stamp(&stamp)),
        confirmed_at: string("confirmedAt").map(|stamp| short_stamp(&stamp)),
        record_path: path.display().to_string(),
    }))
}

/// Re-read only `<publish>/<version>/upload.json`. This is the cheap refresh the
/// Release screen runs after an upload or confirm command, so it never touches
/// the game install or the release VPK.
pub fn refresh_upload(state: &mut State) {
    state.release.upload = None;
    state.release.upload_error = None;

    let folder = match &state.release.publish {
        Some(publish) => PathBuf::from(&publish.dir),
        None => {
            let root = selected_workspace(state);
            match &state.release.info {
                Some(info) => root.join("publish").join(&info.version),
                None => return,
            }
        }
    };

    if !folder.is_dir() {
        return;
    }

    match read_upload(&folder) {
        Ok(upload) => state.release.upload = upload,
        Err(error) => state.release.upload_error = Some(error),
    }
}

/// Run the review queue: one read-only pass over every project in the workspace.
/// This loads the game catalog, so call it from a worker or an explicit scan.
pub fn load_review_queue(state: &mut State) {
    state.release.review = Some(build_review_queue());
}

/// The queue itself, with no UI state touched. Safe to run on a worker.
pub fn build_review_queue() -> ReviewQueue {
    let root = workspace_dir();
    let mut queue = ReviewQueue {
        workspace: root.display().to_string(),
        catalog_label: "no catalog loaded".to_string(),
        items: Vec::new(),
        error: None,
    };

    let mut loaded: Option<(discovery::GameStatus, game_catalog::GameCatalog)> = None;

    match discovery::read_game_status(None) {
        Ok(status) => match status.install.clone() {
            Some(install) => {
                let options = game_catalog::LoadGameCatalogOptions {
                    game_root: install.game_root,
                    cache_dir: root.join(".dah").join("index"),
                    force: false,
                };
                match game_catalog::load_game_catalog(options) {
                    Ok(result) => {
                        queue.catalog_label = format!(
                            "{} paths from {} game source{}",
                            result.catalog.entries.len(),
                            result.catalog.sources.len(),
                            if result.catalog.sources.len() == 1 {
                                ""
                            } else {
                                "s"
                            }
                        );
                        loaded = Some((status, result.catalog));
                    }
                    Err(error) => {
                        queue.error =
                            Some(format!("The game catalog could not be loaded: {error}"));
                    }
                }
            }
            None => {
                queue.error = Some(
                    "No Deadlock install was found, so project checks did not run.".to_string(),
                );
            }
        },
        Err(error) => {
            queue.error = Some(format!("Game discovery failed: {error}"));
        }
    }

    for dir in collect_project_dirs(&root) {
        queue.items.push(review_item(&dir, &loaded));
    }

    queue
}

fn collect_project_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if root.join("project.json").is_file() {
        dirs.push(root.to_path_buf());
    }

    if let Ok(entries) = fs::read_dir(root.join("mods")) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("project.json").is_file() {
                dirs.push(path);
            }
        }
    }

    dirs.sort();
    dirs
}

fn review_item(
    dir: &Path,
    loaded: &Option<(discovery::GameStatus, game_catalog::GameCatalog)>,
) -> ReviewItem {
    let mut item = ReviewItem {
        slug: dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "project".to_string()),
        name: String::new(),
        version: String::new(),
        path: dir.display().to_string(),
        slot: None,
        unchanged: 0,
        changed: 0,
        missing: 0,
        moved: 0,
        unbaselined: 0,
        migrations: Vec::new(),
        warnings: Vec::new(),
        error: None,
    };

    let project = match project::read_project(dir) {
        Ok(project) => project,
        Err(error) => {
            item.error = Some(error.to_string());
            return item;
        }
    };

    item.slug = project.slug.clone();
    item.name = project.name.clone();
    item.version = project.version.clone();
    item.slot = project.slot;

    let Some((status, catalog)) = loaded else {
        item.error = Some("No game catalog loaded, so the target check did not run.".to_string());
        return item;
    };

    item.migrations = migrate::plan_migrations(&project, catalog)
        .into_iter()
        .map(|migration| PathMove {
            from: migration.from,
            to: migration.to,
        })
        .collect();

    match project::check_project(dir, catalog, status) {
        Ok(check) => {
            item.unchanged = check.unchanged;
            item.changed = check.changed;
            item.missing = check.missing;
            item.moved = check.moved;
            item.unbaselined = check.unbaselined;
            item.warnings = review_warnings(&check);
        }
        Err(error) => item.error = Some(error.to_string()),
    }

    item
}

fn review_warnings(check: &project::ProjectCheck) -> Vec<String> {
    let mut warnings = Vec::new();

    if !check.gameinfo_ok {
        warnings.push(
            "gameinfo.gi has no `Game citadel/addons` SearchPaths line, mods will not load"
                .to_string(),
        );
    }

    if let Some(conflict) = &check.slot_conflict
        && let Some(slot) = check.slot
    {
        let file = discovery::format_slot_file_name(slot)
            .unwrap_or_else(|_| format!("pak{slot:02}_dir.vpk"));
        warnings.push(if conflict.file_name == file {
            format!("slot {file} already exists in the addon folder")
        } else {
            format!("slot {file} is already used by {}", conflict.file_name)
        });
    }

    warnings
}

fn short_stamp(value: &str) -> String {
    value.chars().take(16).collect::<String>().replace('T', " ")
}

// Library reads. Addon inventory, conflicts, the GameBanana page and the review
// queue all run on workers; `poll` applies their results on the UI thread.

const BROWSE_PER_PAGE: usize = 50;
const THUMB_TIMEOUT_SECONDS: u64 = 8;
/// Parallel image downloads. GameBanana's image host serves keep-alive HTTPS, so
/// a handful of workers on one shared agent fills a page in about one round trip.
const THUMB_WORKERS: usize = 6;

/// Device pixels per point in the window, as `f32` bits. `app::view` keeps it current.
static PIXEL_SCALE: AtomicU32 = AtomicU32::new(0x3f80_0000);
/// The Browse card width in points, as `f32` bits; 0 before the grid lays out.
static CARD_POINTS: AtomicU32 = AtomicU32::new(0);

pub fn set_pixel_scale(scale: f32) {
    if scale.is_finite() && scale > 0.0 {
        PIXEL_SCALE.store(scale.to_bits(), Ordering::Relaxed);
    }
}

pub fn set_card_points(width: f32) {
    if width.is_finite() && width > 0.0 {
        CARD_POINTS.store(width.to_bits(), Ordering::Relaxed);
    }
}

/// The pixel width a card thumbnail draws at, rounded up to 64 so a small resize keeps the
/// decoded copy. `None` before the grid has laid out.
fn card_thumb_px() -> Option<u32> {
    let points = f32::from_bits(CARD_POINTS.load(Ordering::Relaxed));
    let scale = f32::from_bits(PIXEL_SCALE.load(Ordering::Relaxed));
    (points > 0.0).then(|| ((points * scale / 64.0).ceil() as u32 * 64).max(128))
}

/// The card width the held card thumbnails were scaled for.
static SCALED_FOR: AtomicU32 = AtomicU32::new(0);

/// Cards grew (bigger card size, zoom, a sharper screen): fetch the held card thumbnails again
/// at the new width. The disk cache answers, and each one replaces the softer copy as it
/// arrives, so nothing blanks.
fn regrow_card_thumbs(state: &mut State) {
    let Some(target) = card_thumb_px() else { return };
    let before = SCALED_FOR.swap(target, Ordering::Relaxed);
    if before == 0 || target <= before {
        return;
    }
    let held: Vec<String> = state
        .thumbnails
        .keys()
        .filter(|key| key.rsplit('/').next().is_some_and(|name| name.starts_with("530-90_")))
        .cloned()
        .collect();
    let wake = state.wake.clone();
    state.thumb_crew.add(held, wake, false);
}

/// A fetched image, as the bytes or already scaled to the size it draws at.
pub enum ThumbBytes {
    Encoded(Vec<u8>),
    Rgba(u32, u32, Vec<u8>),
}

/// GameBanana's 530 px card thumbnails (`530-90_*`) are drawn at the card's width, often half
/// that or less. Decode and scale them on the fetch worker so the image cache keeps (with its
/// mips, and again on the GPU) only the pixels a card shows. Anything else stays encoded.
fn shrink_card_thumb(url: &str, bytes: Vec<u8>) -> ThumbBytes {
    let is_card = url.rsplit('/').next().is_some_and(|name| name.starts_with("530-90_"));
    let Some(target) = card_thumb_px().filter(|_| is_card) else {
        return ThumbBytes::Encoded(bytes);
    };
    let Ok(image) = image::load_from_memory(&bytes) else {
        return ThumbBytes::Encoded(bytes);
    };
    let (w, h) = (image.width(), image.height());
    // Within about 15% is not worth a second copy of the work.
    if w == 0 || h == 0 || w * 20 <= target * 23 {
        return ThumbBytes::Encoded(bytes);
    }
    let height = ((h as u64 * target as u64) / w as u64).max(1) as u32;
    let scaled = image::imageops::resize(
        &image.to_rgba8(),
        target,
        height,
        image::imageops::FilterType::Triangle,
    );
    ThumbBytes::Rgba(target, height, scaled.into_raw())
}

/// Thumbnail bytes arrive as each worker finishes, so the first card paints while
/// the rest of the page is still downloading. A disk cache under the data
/// directory answers repeat visits without touching the network.
///
/// The crew is incremental: `add` queues more images onto the running workers, so
/// a page appended by the endless catalog or a mod's gallery never cancels the
/// images already in flight. A URL already queued is not queued twice.
#[derive(Default)]
pub struct ThumbCrew {
    rx: Option<Receiver<(String, Option<ThumbBytes>)>>,
    tx: Option<mpsc::Sender<(String, Option<ThumbBytes>)>>,
    queue: std::sync::Arc<std::sync::Mutex<CrewQueue>>,
    inflight: std::collections::HashSet<String>,
    /// Scale Browse card thumbnails to the card on the worker (`shrink_card_thumb`).
    shrink: bool,
}

#[derive(Default)]
struct CrewQueue {
    urls: std::collections::VecDeque<String>,
    workers: usize,
}

impl ThumbCrew {
    /// The Browse crew: card thumbnails arrive scaled to the card they draw in.
    pub fn shrinking() -> Self {
        Self { shrink: true, ..Self::default() }
    }

    /// Drop everything queued and start over with `urls`.
    pub fn start(&mut self, urls: Vec<String>, wake: Option<rune::Waker>) {
        self.clear();
        self.add(urls, wake, false);
    }

    /// Queue more images. `urgent` puts them ahead of what is waiting, for the
    /// image the player is looking at right now.
    pub fn add(&mut self, urls: Vec<String>, wake: Option<rune::Waker>, urgent: bool) {
        let fresh: Vec<String> = urls
            .into_iter()
            .filter(|url| !url.is_empty() && self.inflight.insert(url.clone()))
            .collect();
        if fresh.is_empty() {
            return;
        }
        if self.tx.is_none() || self.rx.is_none() {
            let (tx, rx) = mpsc::channel();
            self.tx = Some(tx);
            self.rx = Some(rx);
        }
        let Some(tx) = self.tx.clone() else { return };
        let Ok(mut queue) = self.queue.lock() else {
            return;
        };
        if urgent {
            for url in fresh.into_iter().rev() {
                queue.urls.push_front(url);
            }
        } else {
            queue.urls.extend(fresh);
        }
        // Workers leave when the queue is empty, under the same lock that adds
        // to it, so a URL queued here always has a worker to take it.
        let wanted = THUMB_WORKERS.min(queue.workers + queue.urls.len());
        while queue.workers < wanted {
            let shared = self.queue.clone();
            let tx = tx.clone();
            let wake = wake.clone();
            let n = queue.workers;
            let shrink = self.shrink;
            let spawned = std::thread::Builder::new()
                .name(format!("sigil-thumbs-{n}"))
                .spawn(move || {
                    loop {
                        let next = match shared.lock() {
                            Ok(mut queue) => match queue.urls.pop_front() {
                                Some(url) => url,
                                None => {
                                    queue.workers = queue.workers.saturating_sub(1);
                                    break;
                                }
                            },
                            Err(_) => break,
                        };
                        let bytes = fetch_thumb_bytes(&next).map(|bytes| {
                            if shrink {
                                shrink_card_thumb(&next, bytes)
                            } else {
                                ThumbBytes::Encoded(bytes)
                            }
                        });
                        if tx.send((next, bytes)).is_err() {
                            if let Ok(mut queue) = shared.lock() {
                                queue.workers = queue.workers.saturating_sub(1);
                            }
                            break;
                        }
                        if let Some(wake) = &wake {
                            wake();
                        }
                    }
                });
            if spawned.is_err() {
                break;
            }
            queue.workers += 1;
        }
    }

    pub fn try_next(&mut self) -> Option<(String, Option<ThumbBytes>)> {
        let rx = self.rx.as_ref()?;
        match rx.try_recv() {
            Ok(item) => {
                self.inflight.remove(&item.0);
                Some(item)
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.inflight.clear();
                None
            }
        }
    }

    pub fn is_loading(&self) -> bool {
        !self.inflight.is_empty()
    }

    /// True while `url` is queued or downloading.
    pub fn is_queued(&self, url: &str) -> bool {
        self.inflight.contains(url)
    }

    pub fn clear(&mut self) {
        if let Ok(mut queue) = self.queue.lock() {
            queue.urls.clear();
        }
        // A fresh queue: workers of the old one see a closed channel and leave.
        self.queue = Default::default();
        self.rx = None;
        self.tx = None;
        self.inflight.clear();
    }
}

/// Queue images for the browse grid or the mod details view, skipping any the
/// image cache already holds.
pub fn queue_images(state: &mut State, urls: Vec<String>, urgent: bool) {
    let urls: Vec<String> = urls
        .into_iter()
        .filter(|url| !state.thumbnails.contains_key(url))
        .collect();
    let wake = state.wake.clone();
    state.thumb_crew.add(urls, wake, urgent);
}

/// One agent for every image request, so connections and TLS sessions are reused.
fn thumb_agent() -> &'static ureq::Agent {
    static AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(THUMB_TIMEOUT_SECONDS)))
            .build()
            .new_agent()
    })
}

fn thumb_cache_path(url: &str) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut hasher);
    let tail: String = url
        .rsplit('/')
        .next()
        .unwrap_or("img")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-' || *c == '_')
        .take(48)
        .collect();
    data_dir()
        .join("cache")
        .join("images")
        .join(format!("{:016x}-{tail}", hasher.finish()))
}

pub(crate) fn fetch_thumb_bytes(url: &str) -> Option<Vec<u8>> {
    let cached = thumb_cache_path(url);
    if let Ok(bytes) = fs::read(&cached)
        && !bytes.is_empty()
    {
        return Some(bytes);
    }
    let mut response = thumb_agent()
        .get(url)
        .header("User-Agent", gamebanana::GAMEBANANA_USER_AGENT)
        .header("Accept", "image/*")
        .call()
        .ok()?;
    let bytes = response.body_mut().read_to_vec().ok()?;
    if !bytes.is_empty()
        && let Some(dir) = cached.parent()
        && fs::create_dir_all(dir).is_ok()
    {
        let partial = cached.with_extension("part");
        if fs::write(&partial, &bytes).is_ok() {
            let _ = fs::rename(&partial, &cached);
        }
    }
    Some(bytes)
}

fn map_addon_library(report: addons::ConflictReport) -> AddonLibrary {
    let rows = report
        .addons
        .into_iter()
        .map(|addon| AddonRow {
            slot: addon.slot,
            file: addon.file,
            size_bytes: addon.size_bytes,
            chunk_files: addon.chunk_files,
            entry_count: addon.entry_count,
            enabled: addon.enabled,
        })
        .collect();
    let conflicts = report
        .conflicts
        .into_iter()
        .map(|conflict| ConflictRow {
            path: conflict.path,
            slots: conflict.slots,
            winner: conflict.winner,
        })
        .collect();
    AddonLibrary {
        addons: rows,
        conflicts,
        warnings: report.warnings,
        scanned: true,
        error: None,
    }
}

fn scan_addons(game_root: &Path) -> Result<AddonLibrary, String> {
    let report = addons::read_conflict_report(game_root)
        .map_err(|error| format!("The addon folder could not be read: {error}"))?;
    Ok(map_addon_library(report))
}

/// Kick one read-only addon and conflict scan on a worker.
pub fn start_addon_scan(state: &mut State) {
    if state.addon_job.is_running() {
        return;
    }
    let Some(game_root) = state.game_root.clone() else {
        state.addons = missing_install_library();
        return;
    };
    let wake = state.wake.clone();
    state.addons.error = None;
    state.addon_job.start(wake, move || scan_addons(&game_root));
}

/// The synchronous form for a headless scene, before `setup` has a waker.
pub fn scan_addons_now(state: &mut State) {
    let Some(game_root) = state.game_root.clone() else {
        state.addons = missing_install_library();
        return;
    };
    match scan_addons(&game_root) {
        Ok(library) => apply_addon_library(state, library),
        Err(error) => {
            state.addons = AddonLibrary {
                scanned: true,
                error: Some(error.clone()),
                ..AddonLibrary::default()
            };
            state.status = error;
        }
    }
}

fn missing_install_library() -> AddonLibrary {
    AddonLibrary {
        scanned: true,
        error: Some(
            "No Deadlock install was found, so the addon folder could not be read.".to_string(),
        ),
        ..AddonLibrary::default()
    }
}

pub fn apply_addon_library(state: &mut State, library: AddonLibrary) {
    state.status = if let Some(error) = &library.error {
        error.clone()
    } else {
        format!(
            "Addon folder read: {}. {} conflict{}.",
            library.summary(),
            library.conflicts.len(),
            if library.conflicts.len() == 1 {
                ""
            } else {
                "s"
            }
        )
    };
    state.addons = library;
}

fn map_browse_page(page: &gamebanana::BrowsePage, key: u64) -> BrowsePageView {
    BrowsePageView {
        key,
        page: page.page,
        per_page: page.per_page,
        total: page.total,
        complete: page.complete,
        page_count: page.page_count(),
        mods: page
            .mods
            .iter()
            .map(|item| BrowseCard {
                id: item.id,
                name: item.name.clone(),
                author: item.author.clone(),
                category: item.category_line(),
                updated_at: item.updated_at.clone(),
                likes: item.likes,
                views: item.views,
                profile_url: item.profile_url.clone(),
                thumbnail_url: item.thumbnail_url.clone(),
                version: item.version.clone(),
                added_ts: item.added_ts,
                updated_ts: item.updated_ts,
                nsfw: item.nsfw,
                model: item.model.clone(),
                audio_url: item.audio_url.clone(),
                featured: item.featured,
                comments: item.comments,
                author_id: item.author_id,
                obsolete: item.obsolete,
            })
            .collect(),
    }
}

/// Pages a bounded window reads at most, newest first, before it stops: 30 pages of 50.
const WINDOW_MAX_PAGES: usize = 30;

fn fetch_browse_page(
    query: &gamebanana::BrowseQuery,
    page: usize,
    key: u64,
) -> Result<BrowsePageView, String> {
    let now = gamebanana::now_unix();
    let start = query.window_start(now);
    if start <= 0 && query.date_to <= 0 {
        let page = gamebanana::fetch_browse(query, page, BROWSE_PER_PAGE)
            .map_err(|error| error.to_string())?;
        return Ok(map_browse_page(&page, key));
    }
    // GameBanana has no date filter: read newest first until the window's start, then order
    // the window here. The whole window arrives as one complete page.
    let mut window: Vec<gamebanana::BrowseMod> = Vec::new();
    let mut total = 0;
    for page in 1..=WINDOW_MAX_PAGES {
        let read = gamebanana::fetch_browse(query, page, gamebanana::GAMEBANANA_MAX_PERPAGE)
            .map_err(|error| error.to_string())?;
        total = read.total;
        let done = read.complete
            || read.mods.is_empty()
            || read
                .mods
                .iter()
                .any(|m| start > 0 && query.stamp(m) < start);
        window.extend(read.mods.into_iter().filter(|m| {
            let stamp = query.stamp(m);
            (start <= 0 || stamp == 0 || stamp >= start)
                && (query.date_to <= 0 || stamp <= query.date_to)
        }));
        if done {
            break;
        }
    }
    crate::screens::browse::order_window(&mut window, query.shown_order());
    let page = gamebanana::BrowsePage {
        page: 1,
        per_page: window.len().max(1),
        total: total.min(window.len()),
        complete: true,
        mods: window,
    };
    Ok(map_browse_page(&page, key))
}

/// Kick a page read on a worker for the current browse query. Page 1 replaces the catalog
/// (a refresh or new filters); any later page is appended when it lands, so the grid reads as
/// one long catalog. The loaded cards stay on screen while the read is in flight. A read for
/// filters that changed meanwhile is dropped when it lands and page 1 is read again.
pub fn start_browse_fetch(state: &mut State, page: usize) {
    let query = crate::screens::browse::current_query(state);
    let key = crate::screens::browse::query_key(state, &query);
    if let Some(view) = crate::screens::browse::local_page(state, key) {
        state.browse.loading = false;
        state.browse.error = None;
        state.browse_view.from_mirror = false;
        state.browse.page = Some(view);
        return;
    }
    // Offline, and for a custom dated window the server cannot filter, the saved copy is the
    // fastest and only answer. Live reads still win when they can answer.
    let wants_mirror = state.prefs.offline
        || query.date_from > 0
        || query.date_to > 0
        || query.date_field == deadlock_core::gamebanana::DateField::Updated
        // A filtered mixed feed is one read the server cannot do; the copy can.
        || (query.all_models && !query.uses_subfeed() && state.browse_view.mirror.is_some());
    if wants_mirror {
        if let Some(view) = crate::screens::browse::mirror_page(state, page) {
            state.browse.loading = false;
            state.browse.error = None;
            state.browse_view.from_mirror = true;
            if page <= 1 {
                state.browse.page = Some(view);
            } else if let Some(current) = state.browse.page.as_mut()
                && current.key == view.key
            {
                let known: std::collections::HashSet<u64> =
                    current.mods.iter().map(|card| card.id).collect();
                current
                    .mods
                    .extend(view.mods.into_iter().filter(|card| !known.contains(&card.id)));
                current.page = view.page;
                current.page_count = view.page_count;
                current.total = view.total;
                current.complete = view.complete;
            }
            state.status = "Showing the saved catalog.".to_string();
            return;
        }
        if state.prefs.offline {
            state.browse.loading = false;
            state.browse.error = Some(
                "Sigil is offline and the saved catalog has nothing for these filters yet."
                    .to_string(),
            );
            return;
        }
    }
    state.browse_view.from_mirror = false;
    state.browse.loading = true;
    state.browse.error = None;
    if state.browse_job.is_running() {
        return;
    }
    let page = page.max(1);
    let wake = state.wake.clone();
    state.browse_view.in_flight = Some(key);
    state
        .browse_job
        .start(wake, move || fetch_browse_page(&query, page, key));
}

/// The synchronous form for a headless scene. Page and thumbnails are fetched
/// before the first frame so a snapshot shows what the feed actually returned.
pub fn browse_now(state: &mut State, page: usize) {
    state.browse.loading = true;
    state.browse.error = None;
    let query = crate::screens::browse::current_query(state);
    let key = crate::screens::browse::query_key(state, &query);
    match fetch_browse_page(&query, page.max(1), key) {
        Ok(view) => browse_view_now(state, view),
        Err(error) => {
            state.browse.loading = false;
            state.browse.error = Some(error.clone());
            state.status = format!("GameBanana read failed: {error}");
        }
    }
}

/// A headless scene from a stored subfeed body, so the grid renders offline.
/// Thumbnails come from the disk cache or the network when either answers.
pub fn browse_from_body(state: &mut State, body: &str) {
    let query = crate::screens::browse::current_query(state);
    let key = crate::screens::browse::query_key(state, &query);
    match gamebanana::parse_browse(body, 1, BROWSE_PER_PAGE) {
        Ok(page) => browse_view_now(state, map_browse_page(&page, key)),
        Err(error) => {
            state.browse.loading = false;
            state.browse.error = Some(error.to_string());
        }
    }
}

fn browse_view_now(state: &mut State, view: BrowsePageView) {
    let urls: Vec<String> = view
        .mods
        .iter()
        .filter_map(|card| card.thumbnail_url.clone())
        .collect();
    for url in urls {
        if let Some(bytes) = fetch_thumb_bytes(&url) {
            state.pending_thumbs.push((url, bytes));
        }
    }
    // A successful read shows in the grid itself; only failures become a notice.
    state.browse.page = Some(view);
    state.browse.loading = false;
}

/// Page 1 replaces the catalog. A later page is appended, skipping any mod the
/// catalog already shows (the feed shifts while it is read).
pub fn apply_browse_page(state: &mut State, result: Result<BrowsePageView, String>) {
    state.browse.loading = false;
    let query = crate::screens::browse::current_query(state);
    let key = crate::screens::browse::query_key(state, &query);
    // A read for filters that changed while it ran says nothing about the new ones.
    let stale = match &result {
        Ok(view) => view.key != key,
        Err(_) => state.browse_view.in_flight != Some(key),
    };
    state.browse_view.in_flight = None;
    if stale {
        start_browse_fetch(state, 1);
        return;
    }
    match result {
        Ok(view) => {
            state.browse.error = None;
            match state.browse.page.as_mut() {
                Some(current)
                    if !state.browse_view.paged
                        && view.page == current.page + 1
                        && current.key == view.key =>
                {
                    let known: std::collections::HashSet<u64> =
                        current.mods.iter().map(|card| card.id).collect();
                    current
                        .mods
                        .extend(view.mods.into_iter().filter(|card| !known.contains(&card.id)));
                    current.page = view.page;
                    current.page_count = view.page_count;
                    current.total = view.total;
                    current.complete = view.complete;
                }
                Some(_) if view.page > 1 => {}
                _ => {
                    state.thumb_crew.clear();
                    state.browse.page = Some(view);
                }
            }
            // Thumbnails load as cards scroll into view (`browse::store::track_visible`), not
            // the whole page at once: each decoded one holds about 1 MB.
            // The grid shows the result; no success notice.
            // A page of mostly non-mod records can leave the grid as long as it
            // was, so no new row scrolls into view to ask for the next page.
            crate::screens::browse::load_more_if_near_end(state);
        }
        Err(error) => {
            crate::screens::settings::log::error(format!("Browse read failed: {error}"));
            // A failed next page (or refresh) of the catalog already on screen keeps that grid;
            // scrolling to the end asks again.
            if state.browse.page.as_ref().is_some_and(|page| page.key == key) {
                state.status = "GameBanana did not answer. Scroll down to try again.".to_string();
                return;
            }
            // A saved copy keeps Browse usable when GameBanana does not answer.
            if let Some(view) = crate::screens::browse::mirror_page(state, 1) {
                state.browse.page = Some(view);
                state.browse.error = None;
                state.browse_view.from_mirror = true;
                state.status =
                    "GameBanana did not answer. Showing the saved catalog.".to_string();
            } else {
                state.browse.error = Some(error.clone());
                state.status = format!("GameBanana read failed: {error}");
            }
        }
    }
}

/// Load the queued thumbnails into the image cache. Called from `poll` on the UI
/// thread; decode still happens on Rune's image workers.
pub fn drain_thumbnails(state: &mut State) -> bool {
    let mut changed = false;
    regrow_card_thumbs(state);
    if let Some(cache) = state.images.as_ref() {
        let pending = std::mem::take(&mut state.pending_thumbs);
        for (key, bytes) in pending {
            let image = cache.load(bytes);
            state.thumbnails.insert(key, image.pixels().clone());
            changed = true;
        }
    }
    while let Some((url, bytes)) = state.thumb_crew.try_next() {
        if let (Some(cache), Some(bytes)) = (state.images.as_ref(), bytes) {
            let image = match bytes {
                ThumbBytes::Encoded(bytes) => cache.load(bytes),
                ThumbBytes::Rgba(width, height, pixels) => cache.rgba(width, height, pixels),
            };
            state.thumbnails.insert(url, image.pixels().clone());
            changed = true;
        }
    }
    changed
}

/// Kick one review queue pass on a worker. It loads the game catalog, so this
/// never runs from a view.
pub fn start_review_scan(state: &mut State) {
    if state.model_review_job.is_running() {
        return;
    }
    let wake = state.wake.clone();
    state.status = "Review queue started in the background.".to_string();
    state.review_job.start(wake, || Ok(build_review_queue()));
}

pub fn apply_review_queue(state: &mut State, result: Result<ReviewQueue, String>) {
    match result {
        Ok(queue) => {
            state.status = format!("Review queue: {}. Nothing was written.", queue.summary());
            state.release.review = Some(queue);
        }
        Err(error) => {
            state.status = format!("Review queue failed: {error}");
        }
    }
}

/// The synchronous review pass for a headless scene.
pub fn review_now(state: &mut State) {
    load_review_queue(state);
    let summary = state
        .release
        .review
        .as_ref()
        .map(ReviewQueue::summary)
        .unwrap_or_else(|| "nothing was scanned".to_string());
    state.status = format!("Review queue: {summary}. Nothing was written.");
}

// ---------------------------------------------------------------------------
// Screens added after the library: profiles, autoexec, stats, locker, launch,
// toolchain and character. Each one is a real core read or a gated app action.
// ---------------------------------------------------------------------------

/// Reveal a file or folder in the platform file manager. Read-only.
pub fn reveal_path(state: &mut State, path: String) {
    if path.is_empty() {
        state.status = "No path to reveal was resolved yet.".to_string();
        return;
    }
    let path = PathBuf::from(path);
    if !path.exists() {
        state.status = format!("{} does not exist on disk.", path.display());
        return;
    }
    if rune::is_headless() {
        state.status = format!("Headless run: reveal was not sent for {}.", path.display());
        return;
    }
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open")
        .arg("-R")
        .arg(&path)
        .spawn()
        .map(|_| ());
    #[cfg(target_os = "windows")]
    let result = {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("explorer")
            .raw_arg(deadlock_core::autoexec::windows_select_argument(&path))
            .spawn()
    }
        .map(|_| ());
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open")
        .arg(path.parent().unwrap_or(&path))
        .spawn()
        .map(|_| ());
    state.status = match result {
        Ok(()) => format!("Revealed {} in the file manager.", path.display()),
        Err(error) => format!(
            "The file manager could not open {}: {error}",
            path.display()
        ),
    };
}

// --- profiles --------------------------------------------------------------

/// Read the saved profiles and check them against the install on a worker.
pub fn start_profile_list(state: &mut State) {
    if state.profiles_job.is_running() || state.profiles.fixture {
        return;
    }
    let workspace = workspace_dir();
    let game_root = state.game_root.clone();
    let wake = state.wake.clone();
    state.profiles.error = None;
    state.profiles_job.start(wake, move || {
        crate::screens::profiles::load(&workspace, game_root.as_deref())
    });
}

pub fn apply_profile_list(
    state: &mut State,
    result: Result<crate::screens::profiles::Load, String>,
) {
    match result {
        Ok(load) => crate::screens::profiles::apply_load(state, load),
        Err(error) => {
            state.profiles.loaded = true;
            state.profiles.error = Some(error.clone());
            state.status = error;
        }
    }
}

// --- autoexec --------------------------------------------------------------

pub fn start_autoexec_read(state: &mut State) {
    if state.autoexec_job.is_running() {
        return;
    }
    let Some(game_root) = state.game_root.clone() else {
        state.autoexec.loaded = true;
        state.autoexec.error =
            Some("No Deadlock install was found, so there is no autoexec path.".to_string());
        return;
    };
    let wake = state.wake.clone();
    state.autoexec.error = None;
    state.autoexec_job.start(wake, move || {
        let path = autoexec::autoexec_path(&game_root);
        let text = autoexec::read_autoexec(&game_root).map_err(|error| error.to_string())?;
        Ok(crate::screens::autoexec::Read {
            path: path.display().to_string(),
            exists: text.is_some(),
            text: text.unwrap_or_default(),
        })
    });
}

pub fn load_autoexec_now(state: &mut State) {
    let Some(game_root) = state.game_root.clone() else {
        state.autoexec.loaded = true;
        state.autoexec.error =
            Some("No Deadlock install was found, so there is no autoexec path.".to_string());
        return;
    };
    let path = autoexec::autoexec_path(&game_root);
    match autoexec::read_autoexec(&game_root) {
        Ok(text) => {
            state.autoexec.exists = text.is_some();
            let text = text.unwrap_or_default();
            state.autoexec.path = path.display().to_string();
            state.autoexec.set_text(&text);
            state.autoexec_original = text;
            state.autoexec.loaded = true;
            state.autoexec.error = None;
        }
        Err(error) => {
            state.autoexec.loaded = true;
            state.autoexec.error = Some(error.to_string());
        }
    }
}

pub fn apply_autoexec(state: &mut State, result: Result<crate::screens::autoexec::Read, String>) {
    match result {
        Ok(read) => {
            state.autoexec.path = read.path;
            state.autoexec.exists = read.exists;
            state.autoexec.set_text(&read.text);
            state.autoexec_original = read.text;
            state.autoexec.loaded = true;
            state.autoexec.error = None;
        }
        Err(error) => {
            state.autoexec.loaded = true;
            state.autoexec.error = Some(error.clone());
            state.status = error;
        }
    }
}

// --- stats -----------------------------------------------------------------

/// Matches kept from the stored history. The API returns the whole history in
/// one response, so a larger window costs no extra request; 200 covers a few
/// weeks of rank movement and enough games for the form comparisons.
const STATS_WINDOW: usize = 200;
/// Tracked players kept for the account switcher.
const STATS_REMEMBERED: usize = 6;

fn stats_accounts_path() -> PathBuf {
    data_dir().join("stats-accounts.json")
}

/// Where the stats responses are kept between loads. Safe to delete: the next
/// load asks the API again.
pub fn stats_cache_dir() -> PathBuf {
    data_dir().join("cache").join("stats")
}

/// A client with the disk cache attached, so a load inside a TTL costs no
/// request and a failed one falls back to the copy on disk.
pub fn stats_client() -> deadlock_core::stats::StatsClient {
    deadlock_core::stats::StatsClient::with_cache(stats_cache_dir())
}

/// Accounts this machine has loaded before: the primary account first, then the
/// most recently loaded. A missing or unreadable file is an empty list.
pub fn stats_remembered() -> Vec<crate::screens::stats::Remembered> {
    let Ok(text) = fs::read_to_string(stats_accounts_path()) else {
        return Vec::new();
    };
    let Ok(rows) = serde_json::from_str::<Vec<serde_json::Value>>(&text) else {
        return Vec::new();
    };
    let mut out: Vec<crate::screens::stats::Remembered> = rows
        .iter()
        .filter_map(|row| {
            Some(crate::screens::stats::Remembered {
                account_id: row.get("accountId")?.as_u64()?,
                name: row.get("name")?.as_str()?.to_string(),
                primary: row.get("primary").and_then(|value| value.as_bool()).unwrap_or(false),
            })
        })
        .take(STATS_REMEMBERED)
        .collect();
    crate::screens::stats::sort_remembered(&mut out);
    out
}

fn save_stats_remembered(rows: &[crate::screens::stats::Remembered]) {
    let json: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "accountId": row.account_id,
                "name": row.name,
                "primary": row.primary,
            })
        })
        .collect();
    let path = stats_accounts_path();
    let Ok(text) = serde_json::to_string_pretty(&json) else {
        return;
    };
    if let Some(dir) = path.parent()
        && fs::create_dir_all(dir).is_ok()
    {
        let partial = path.with_extension("json.part");
        if fs::write(&partial, text).is_ok() {
            let _ = fs::rename(&partial, &path);
        }
    }
}

fn remember_stats_account(state: &mut State, account_id: u64, name: &str) {
    let rows = &mut state.stats.remembered;
    let had_primary = rows.iter().any(|row| row.primary);
    rows.retain(|row| row.account_id != account_id);
    rows.insert(
        0,
        crate::screens::stats::Remembered {
            account_id,
            name: name.to_string(),
            primary: !had_primary,
        },
    );
    rows.truncate(STATS_REMEMBERED);
    crate::screens::stats::sort_remembered(rows);
    if !rune::is_headless() {
        save_stats_remembered(rows);
    }
}

/// Make one tracked account the primary one: the page opens on it.
pub fn set_primary_account(state: &mut State, account_id: u64) {
    for row in &mut state.stats.remembered {
        row.primary = row.account_id == account_id;
    }
    crate::screens::stats::sort_remembered(&mut state.stats.remembered);
    if !rune::is_headless() {
        save_stats_remembered(&state.stats.remembered);
    }
}

/// Drop one tracked account. Only the local list changes; the primary moves to
/// the next account when the one removed held it.
pub fn forget_stats_account(state: &mut State, account_id: u64) {
    let was_primary = state
        .stats
        .remembered
        .iter()
        .any(|row| row.account_id == account_id && row.primary);
    state
        .stats
        .remembered
        .retain(|row| row.account_id != account_id);
    if was_primary && let Some(first) = state.stats.remembered.first_mut() {
        first.primary = true;
    }
    crate::screens::stats::sort_remembered(&mut state.stats.remembered);
    if !rune::is_headless() {
        save_stats_remembered(&state.stats.remembered);
    }
}

/// The account to load: the field, then the primary tracked account, then the
/// last loaded one, then `DAH_STEAM_ID`.
fn stats_input(state: &State) -> deadlock_core::stats::AccountInput {
    use deadlock_core::stats::{AccountInput, parse_account_input};
    match parse_account_input(&state.stats.account.text) {
        AccountInput::Empty => {}
        found => return found,
    }
    if let Some(primary) = state.stats.primary() {
        return AccountInput::Account(primary.account_id);
    }
    std::env::var("DAH_STEAM_ID")
        .map(|value| parse_account_input(&value))
        .unwrap_or(AccountInput::Empty)
}

/// Chip icons for the heroes on screen, keyed `stats-hero:<id>`. Read from the
/// local Grimoire art (`hero_portraits/chip-icons`); a missing icon falls back
/// to a letter in the view.
pub fn stats_hero_icons(
    snapshot: &deadlock_core::stats::StatsSnapshot,
) -> Vec<(String, Vec<u8>)> {
    let assets = deadlock_core::locker::LockerAssetPaths::discover();
    let Some(dir) = assets.hero_portraits_dir.map(|dir| dir.join("chip-icons")) else {
        return Vec::new();
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    let names = snapshot
        .heroes
        .iter()
        .map(|hero| (hero.hero_id, hero.hero_name.clone()))
        .chain(
            snapshot
                .matches
                .iter()
                .map(|item| (item.hero_id, item.hero_name.clone())),
        );
    for (hero_id, name) in names {
        let Some(name) = name else { continue };
        if !seen.insert(hero_id) {
            continue;
        }
        let key = deadlock_core::locker::hero_asset_key(&name);
        for file in [
            format!("{key}.png"),
            format!("{key}.webp"),
            format!("the_{key}.png"),
            format!("the_{key}.webp"),
        ] {
            if let Ok(bytes) = fs::read(dir.join(&file)) {
                out.push((crate::screens::stats::icon_key(hero_id), bytes));
                break;
            }
        }
    }
    out
}

/// Steam accounts this computer has signed in with. Local file reads only.
fn steam_users() -> Vec<deadlock_core::steam_users::SteamUser> {
    deadlock_core::steam_users::detect(&deadlock_core::discovery::default_steam_roots())
}

fn fetch_stats(account_id: u64) -> crate::screens::stats::Load {
    let client = stats_client();
    let snapshot = client
        .fetch_snapshot(account_id, STATS_WINDOW)
        .map(|mut snapshot| {
            let tier = snapshot.rank.badge / 10;
            let tier_name = snapshot.tier_names.get(&tier).cloned();
            snapshot.peers = client
                .fetch_peer_benchmarks(snapshot.rank.badge, snapshot.generated_unix, tier_name)
                .unwrap_or(None);
            snapshot
        })
        .map_err(|error| error.to_string());
    let heatmap = client
        .fetch_kill_heatmap(account_id)
        .map_err(|error| error.to_string());
    let objectives = client.fetch_map_objectives().unwrap_or_default();
    let icons = snapshot.as_ref().map(stats_hero_icons).unwrap_or_default();
    crate::screens::stats::Load {
        account_id,
        snapshot,
        heatmap,
        objectives,
        icons,
        steam: steam_users(),
        age: client.age(),
    }
}

pub fn start_stats_load(state: &mut State) {
    load_stats(state, false);
}

/// The shared loader. `force` skips the 60 second refresh cooldown: switching to
/// another tracked player is a deliberate action, not a refresh.
fn load_stats(state: &mut State, force: bool) {
    use deadlock_core::stats::AccountInput;
    if state.stats_job.is_running() {
        return;
    }
    if !force && !state.stats.can_refresh() {
        let wait = state.stats.refresh_wait();
        state.status = format!("Stats refreshed a moment ago. Try again in {wait} seconds.");
        return;
    }
    if !state.prefs.allow_stats {
        state.stats.error = Some("Player stats are turned off in Settings, Privacy.".to_string());
        return;
    }
    let account_id = match stats_input(state) {
        AccountInput::Account(id) => id,
        AccountInput::Search(query) => {
            crate::screens::stats::start_search(state, query);
            return;
        }
        AccountInput::Empty => {
            state.stats.error = None;
            return;
        }
    };
    let wake = state.wake.clone();
    state.stats.error = None;
    state.stats.search.clear();
    state.stats.last_refresh = Some(std::time::Instant::now());
    state
        .stats_job
        .start(wake, move || Ok(fetch_stats(account_id)));
}

/// Load one account picked from a list.
/// Load what the field holds, now: the player pressed Load or Enter.
pub fn track_stats_account(state: &mut State) {
    load_stats(state, true);
}

pub fn load_stats_account(state: &mut State, account_id: u64) {
    state.stats.account = TextEdit::new(account_id.to_string());
    load_stats(state, true);
}

pub fn stats_now(state: &mut State) {
    use deadlock_core::stats::AccountInput;
    let AccountInput::Account(account_id) = stats_input(state) else {
        return;
    };
    state.stats.last_refresh = Some(std::time::Instant::now());
    apply_stats(state, Ok(fetch_stats(account_id)));
}

pub fn apply_stats(state: &mut State, result: Result<crate::screens::stats::Load, String>) {
    match result {
        Ok(load) => {
            state.stats.loaded = true;
            state.stats.age = load.age;
            state.stats.steam = load.steam;
            state.pending_thumbs.extend(load.icons);
            match load.snapshot {
                Ok(snapshot) => {
                    state.stats.account = TextEdit::new("");
                    state.stats.account_open = false;
                    remember_stats_account(
                        state,
                        load.account_id,
                        &snapshot.profile.persona_name.clone(),
                    );
                    state.status = format!("Loaded player stats for account {}.", load.account_id);
                    crate::screens::stats::set_snapshot(&mut state.stats, snapshot);
                    state.stats.error = None;
                }
                Err(error) => {
                    state.stats.error = Some(error.clone());
                    state.status = error;
                }
            }
            match load.heatmap {
                Ok(heat) => {
                    state.stats.heat = Some(crate::screens::stats::bucket(
                        &heat,
                        crate::screens::stats::HEAT_CELLS,
                        crate::screens::stats::HEAT_CELLS,
                    ));
                    state.stats.heat_error = None;
                }
                Err(error) => {
                    state.stats.heat = None;
                    state.stats.heat_error = Some(error);
                }
            }
            if !load.objectives.is_empty() {
                state.stats.objectives = load.objectives;
            }
        }
        Err(error) => {
            state.stats.loaded = true;
            state.stats.error = Some(error.clone());
            state.status = error;
        }
    }
}

/// Open one match's scoreboard: both teams, the final builds and the souls curve.
pub fn start_scoreboard_load(state: &mut State, match_id: u64) {
    use crate::screens::stats::Fetched;
    if state.stats.board_job.is_running() {
        return;
    }
    if !state.prefs.allow_stats {
        state.stats.board = Fetched::Failed(
            "Player stats are turned off in Settings, Privacy.".to_string(),
        );
        return;
    }
    let Some(account_id) = state.stats.snapshot.as_ref().map(|snap| snap.account_id) else {
        return;
    };
    if let Fetched::Ready(board) = &state.stats.board
        && board.match_id == match_id
    {
        state.stats.board_open = Some(match_id);
        return;
    }
    state.stats.board = Fetched::Loading;
    state.stats.board_open = Some(match_id);
    let wake = state.wake.clone();
    state.stats.board_job.start(wake, move || {
        stats_client()
            .fetch_match_scoreboard(match_id, account_id)
            .map_err(|error| error.to_string())
    });
}

pub fn close_scoreboard(state: &mut State) {
    state.stats.board_open = None;
}

/// Teammates and opponents. Three large responses, so only on a press.
pub fn start_social_load(state: &mut State) {
    use crate::screens::stats::Fetched;
    if state.stats.social_job.is_running() {
        return;
    }
    if !state.prefs.allow_stats {
        state.stats.social =
            Fetched::Failed("Player stats are turned off in Settings, Privacy.".to_string());
        return;
    }
    let Some(account_id) = state.stats.snapshot.as_ref().map(|snap| snap.account_id) else {
        return;
    };
    state.stats.social = Fetched::Loading;
    let wake = state.wake.clone();
    state.stats.social_job.start(wake, move || {
        stats_client()
            .fetch_social(account_id)
            .map_err(|error| error.to_string())
    });
}

/// One region's leaderboard for the current season, with the previous season as
/// the fallback when the current board is empty.
pub fn start_leaderboard_load(state: &mut State, region: impl Into<String>) {
    use crate::screens::stats::Fetched;
    let region = region.into();
    state.stats.region = Some(region.clone());
    state.stats.leaderboard_all = false;
    if state.stats.leaderboard_job.is_running() {
        return;
    }
    if !state.prefs.allow_stats {
        state.stats.leaderboard =
            Fetched::Failed("Player stats are turned off in Settings, Privacy.".to_string());
        return;
    }
    state.stats.leaderboard = Fetched::Loading;
    let wake = state.wake.clone();
    state.stats.leaderboard_job.start(wake, move || {
        let client = stats_client();
        let seasons = client.fetch_seasons().map_err(|error| error.to_string())?;
        let now = deadlock_core::stats::now_seconds();
        let season = deadlock_core::stats::leaderboard::current_season(&seasons, now)
            .ok_or_else(|| "The API lists no ranked season with a leaderboard yet.".to_string())?;
        let mut board = client
            .fetch_season_board(&region, season)
            .map_err(|error| error.to_string())?;
        if board.entries.is_empty()
            && let Some(previous) =
                deadlock_core::stats::leaderboard::previous_season(&seasons, season)
        {
            board = client
                .fetch_season_board(&region, previous)
                .map_err(|error| error.to_string())?;
        }
        Ok(board)
    });
}

// --- locker ----------------------------------------------------------------

const ART_BYTE_CAP: u64 = 24 * 1024 * 1024;

/// Bytes a PNG or WebP header needs for its pixel size (`screens::locker::image_size`).
const ART_HEADER_BYTES: usize = 32;

/// One art file by path, the image cache's key, with the pixel size from its header so cards
/// lay out before the decode lands. Only the header is read here: the image cache reads and
/// decodes the file on its own worker (`ImageCache::file`), so the encoded bytes never pass
/// through the UI thread.
fn read_art(path: &Path) -> Option<(String, Option<(u32, u32)>)> {
    use std::io::Read;
    let size = fs::metadata(path).ok()?.len();
    if size == 0 || size > ART_BYTE_CAP {
        return None;
    }
    let mut header = Vec::with_capacity(ART_HEADER_BYTES);
    fs::File::open(path)
        .ok()?
        .take(ART_HEADER_BYTES as u64)
        .read_to_end(&mut header)
        .ok()?;
    let dims = crate::screens::locker::image_size(&header);
    Some((path.display().to_string(), dims))
}

/// Width of the card copies of the Locker renders at 2x: about twice the widest grid card.
/// Decoding these instead of the 1100 to 1508 pixel sources is most of the Locker's load time
/// in a debug build. The hero page loads the full render on its own. `art_width` scales it to
/// the screen, so a 100% display decodes a quarter of the pixels.
const CARD_ART_W: u32 = 360;

/// Width of a name plate copy. A plate draws at most 70 percent of a card, so 320 pixels is
/// still over 2x on the largest card.
const NAME_ART_W: u32 = 320;

/// Where the card copies live: `<data>/cache/locker-art`. Safe to delete; the next load
/// makes them again.
fn locker_art_cache() -> PathBuf {
    data_dir().join("cache").join("locker-art")
}

/// A `width` pixel wide copy of one art file, made once and kept in `cache`, keyed by the
/// source path, size and modified time. The grid decodes these instead of the full renders:
/// a packed render is 1100 pixels wide and a development checkout's is 1508, and decoding 88
/// of those (plus their mip chains) cost about a minute of CPU and 400 MB per Locker open in
/// a debug build. Runs on the Locker's load worker, never the UI thread. Returns the source
/// itself when it is already that small, and `None` when it cannot be read.
/// A 2x copy width scaled to the window's pixels per point (at least 1x), in steps of 1/4 so a
/// fractional display scale reuses one copy.
fn art_width(at_2x: u32) -> u32 {
    let scale = f32::from_bits(PIXEL_SCALE.load(Ordering::Relaxed)).clamp(1.0, 2.0);
    let step = (scale * 4.0).ceil() / 4.0;
    ((at_2x as f32) * step / 2.0).ceil() as u32
}

fn card_copy(src: &Path, width: u32, cache: &Path) -> Option<PathBuf> {
    use image::ImageEncoder;
    let meta = fs::metadata(src).ok()?;
    if meta.len() == 0 || meta.len() > ART_BYTE_CAP {
        return None;
    }
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in src
        .to_string_lossy()
        .bytes()
        .chain(meta.len().to_le_bytes())
        .chain(mtime.to_le_bytes())
        .chain(width.to_le_bytes())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let stem: String = src
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();
    let out = cache.join(format!("{stem}-{width}-{hash:016x}.png"));
    if out.is_file() {
        return Some(out);
    }
    // Small enough already: the header says so without a decode.
    if let Some((_, Some((w, _)))) = read_art(src)
        && w <= width
    {
        return Some(src.to_path_buf());
    }
    let mut reader = image::ImageReader::open(src).ok()?.with_guessed_format().ok()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().ok()?;
    if img.width() <= width {
        return Some(src.to_path_buf());
    }
    let height = ((u64::from(img.height()) * u64::from(width)) / u64::from(img.width())).max(1) as u32;
    let small = img.thumbnail_exact(width, height).to_rgba8();
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new_with_quality(
        &mut bytes,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Sub,
    )
    .write_image(small.as_raw(), small.width(), small.height(), image::ExtendedColorType::Rgba8)
    .ok()?;
    fs::create_dir_all(cache).ok()?;
    let partial = cache.join(format!(".{stem}-{}.tmp", std::process::id()));
    fs::write(&partial, &bytes).ok()?;
    if fs::rename(&partial, &out).is_err() {
        let _ = fs::remove_file(&partial);
        return None;
    }
    Some(out)
}

/// [`card_copy`] for many files at once, on a few threads. Keys are the sources.
fn card_copies(sources: Vec<PathBuf>, width: u32) -> std::collections::HashMap<PathBuf, PathBuf> {
    let cache = locker_art_cache();
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .clamp(1, 6);
    let chunk = sources.len().div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        let handles: Vec<_> = sources
            .chunks(chunk)
            .map(|part| {
                let cache = &cache;
                scope.spawn(move || {
                    part.iter()
                        .filter_map(|src| card_copy(src, width, cache).map(|out| (src.clone(), out)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap_or_default())
            .collect()
    })
}

/// Ask the image cache for one Locker art file the grid did not preload: the full render
/// behind a hero page, the General backdrop. Read and decoded on the cache's worker.
pub fn request_locker_art(state: &mut State, path: &str) {
    if path.is_empty() || state.thumbnails.contains_key(path) {
        return;
    }
    match state.images.as_ref() {
        Some(cache) => {
            let image = cache.file(path, 1.0, 1.0);
            state.thumbnails.insert(path.to_string(), image.pixels().clone());
        }
        // A headless scene runs before `setup` makes the cache, and preloads.
        None => {
            if !state.pending_thumbs.iter().any(|(p, _)| p == path)
                && let Ok(bytes) = fs::read(path)
            {
                state.pending_thumbs.push((path.to_string(), bytes));
            }
        }
    }
}

fn locker_categories_path() -> PathBuf {
    data_dir().join(locker::LOCKER_CATEGORIES_FILE)
}

/// Store the Locker categories and show them. The file lives in the app data directory.
pub fn save_locker_categories(state: &mut State, next: Vec<locker::LockerCategory>) {
    if let Err(error) = locker::write_categories(&locker_categories_path(), &next) {
        state.status = format!("Locker categories could not be saved: {error}");
    }
    state.locker.categories = next;
}

pub fn load_locker(
    workspace: &Path,
    pinned: Option<String>,
) -> Result<crate::screens::locker::Load, String> {
    // The roster and its art do not wait for the game: without an install or a
    // catalog every hero still shows, with art.
    let install = discovery::read_game_status(None)
        .ok()
        .and_then(|status| status.install);
    let mut note = None;
    let (catalog, addons) = match &install {
        Some(install) => {
            let options = game_catalog::LoadGameCatalogOptions {
                game_root: install.game_root.clone(),
                cache_dir: workspace.join(".dah").join("index"),
                force: false,
            };
            let catalog = match game_catalog::load_game_catalog(options) {
                Ok(loaded) => loaded.catalog,
                Err(error) => {
                    note = Some(format!("The game files could not be read: {error}"));
                    empty_catalog()
                }
            };
            let addons = addons::read_conflict_report(&install.game_root)
                .map(|report| report.addons)
                .unwrap_or_default();
            (catalog, addons)
        }
        None => (empty_catalog(), Vec::new()),
    };
    let assets = locker::LockerAssetPaths::discover();
    let library = locker::discover_locker(
        &catalog,
        &addons,
        install.as_ref().map(|install| install.addons_dir.as_path()),
        &assets,
    );
    let prefs = locker::read_prefs(&data_dir().join(locker::LOCKER_PREFS_FILE));
    let cards_dir = data_dir().join(locker::LOCKER_CARDS_DIR);
    let briefs: Vec<crate::screens::locker::BriefRow> = deadlock_core::recolor::list_briefs(workspace)
        .unwrap_or_default()
        .into_iter()
        .map(|brief| crate::screens::locker::BriefRow {
            hero: brief.hero,
            display_name: brief.display_name,
            palette: brief.palette.name,
            note: brief.note,
        })
        .collect();
    let global = install
        .as_ref()
        .map(|install| {
            locker::read_global_path(&deadlock_core::deploy::gameinfo_path(&install.game_root))
                .unwrap_or_default()
        })
        .unwrap_or_default();

    // Names for slots Sigil installed: the latest journal entry per directory file.
    let mut labels: std::collections::BTreeMap<String, String> = Default::default();
    if let Ok(journal) = deadlock_core::install::read_journal(workspace) {
        for entry in journal {
            if !entry.source_label.trim().is_empty() {
                labels.insert(entry.file.clone(), entry.source_label.clone());
            }
        }
    }
    let mods = library
        .mods
        .iter()
        .map(|item| {
            let base = item.file.trim_end_matches(".disabled").to_string();
            crate::screens::locker::ModRow {
                slot: item.slot,
                file: item.file.clone(),
                enabled: item.enabled,
                key: item.key.clone(),
                label: labels
                    .get(&base)
                    .cloned()
                    .unwrap_or_else(|| format!("pak{:02}", item.slot)),
                heroes: item.heroes.clone(),
                general: item.general,
                sounds: item.sounds.clone(),
            }
        })
        .collect();

    // Card copies of every render and name plate, and the General backdrop: the grid only
    // ever decodes these. Renders and plates are separate passes because a plate needs less
    // width than a render.
    let mut render_sources: Vec<PathBuf> = Vec::new();
    let mut name_sources: Vec<PathBuf> = Vec::new();
    for hero in &library.heroes {
        render_sources.extend(hero.render_path.iter().cloned());
        name_sources.extend(hero.name_art_path.iter().cloned());
    }
    let mut copies = card_copies(render_sources, art_width(CARD_ART_W));
    copies.extend(card_copies(name_sources, art_width(NAME_ART_W)));
    copies.extend(card_copies(
        assets.general_backdrop.iter().cloned().collect(),
        art_width(CARD_ART_W),
    ));
    let copy_of = |src: Option<&Path>| src.and_then(|src| copies.get(src)).and_then(|p| read_art(p));

    let heroes = library
        .heroes
        .iter()
        .map(|hero| {
            let render = hero.render_path.as_deref().and_then(read_art);
            let card = copy_of(hero.render_path.as_deref());
            // The name plate never draws wider than the card copy, so the copy is the art.
            let name_art = copy_of(hero.name_art_path.as_deref())
                .or_else(|| hero.name_art_path.as_deref().and_then(read_art));
            let custom = prefs.card(&hero.display_name).and_then(|card| {
                read_art(&cards_dir.join(&card.file)).map(|(path, size)| (path, size, card.focus_x))
            });
            crate::screens::locker::HeroCard {
                codename: hero.codename.clone(),
                display_name: hero.display_name.clone(),
                render_size: render.as_ref().and_then(|art| art.1),
                name_size: name_art.as_ref().and_then(|art| art.1),
                card_size: card.as_ref().and_then(|art| art.1),
                card: card.map(|(path, _)| path),
                render: render.map(|(path, _)| path),
                name_art: name_art.map(|(path, _)| path),
                custom: custom.as_ref().map(|(path, _, _)| path.clone()),
                custom_size: custom.as_ref().and_then(|(_, size, _)| *size),
                custom_focus: custom.map(|(_, _, focus)| focus).unwrap_or(locker::DEFAULT_FACE_X),
                face_x: locker::face_position_x(&hero.display_name),
                mod_count: hero.mod_count,
                asset_count: hero.asset_count,
                in_catalog: hero.in_catalog,
                recolor_paths: hero.recolor_paths.clone(),
            }
        })
        .collect();
    let general_bg = assets.general_backdrop.as_deref().and_then(read_art);
    let general_card = copy_of(assets.general_backdrop.as_deref());
    let assets_dir = assets
        .heroes_dir
        .as_ref()
        .map(|dir| dir.display().to_string())
        .unwrap_or_default();
    Ok(crate::screens::locker::Load {
        heroes,
        mods,
        categories: locker::read_categories(&locker_categories_path()),
        prefs,
        briefs,
        global,
        general_bg_size: general_bg.as_ref().and_then(|art| art.1),
        general_bg: general_bg.map(|(path, _)| path),
        general_card_size: general_card.as_ref().and_then(|art| art.1),
        general_card: general_card.map(|(path, _)| path),
        assets_dir,
        catalog_label: format!(
            "{} paths from {} source{}",
            catalog.entries.len(),
            catalog.sources.len(),
            if catalog.sources.len() == 1 { "" } else { "s" }
        ),
        pinned,
        error: note,
    })
}

fn empty_catalog() -> game_catalog::GameCatalog {
    game_catalog::GameCatalog {
        schema: 0,
        built_at: String::new(),
        sources: Vec::new(),
        entries: Vec::new(),
    }
}

pub fn start_locker_load(state: &mut State) {
    if state.locker_job.is_running() {
        return;
    }
    let workspace = workspace_dir();
    let wake = state.wake.clone();
    let pinned = state.project().and_then(project_hero);
    state.locker.error = None;
    state
        .locker_job
        .start(wake, move || load_locker(&workspace, pinned));
}

pub fn locker_now(state: &mut State) {
    let workspace = workspace_dir();
    let pinned = state.project().and_then(project_hero);
    apply_locker(state, load_locker(&workspace, pinned));
}

/// The hero codename a project targets, from the first target under a
/// `heroes_*` folder. `materials/models/heroes_wip/billy/x.vmat_c` gives
/// `billy`; `models/heroes_wip/punkgoat/punkgoat.vmdl_c` gives `punkgoat`.
pub fn project_hero(project: &model::Project) -> Option<String> {
    let hero_of = |path: &str| -> Option<String> {
        let lowered = path.to_ascii_lowercase();
        let segments: Vec<&str> = lowered.split('/').collect();
        let index = segments
            .iter()
            .position(|segment| segment.starts_with("heroes"))?;
        segments.get(index + 1).map(|code| (*code).to_string())
    };
    // A material target is the recolor surface, so it names the hero the user
    // is actually editing. Model targets often point at a WIP variant folder.
    project
        .targets
        .iter()
        .filter(|target| target.path.starts_with("materials/"))
        .find_map(|target| hero_of(&target.path))
        .or_else(|| {
            project
                .targets
                .iter()
                .find_map(|target| hero_of(&target.path))
        })
}

/// The grid's art: card copies, name plates, custom cards and the General card.
fn locker_grid_art(state: &State) -> Vec<String> {
    let mut art: Vec<String> = Vec::new();
    for hero in &state.locker.heroes {
        art.extend(hero.card.iter().cloned());
        art.extend(hero.name_art.iter().cloned());
        art.extend(hero.custom.iter().cloned());
    }
    art.extend(state.locker.general_card.iter().cloned());
    art
}

fn decode_locker_art(state: &mut State, art: Vec<String>) {
    for path in art {
        if state.thumbnails.contains_key(&path) {
            continue;
        }
        match state.images.as_ref() {
            Some(cache) => {
                let image = cache.file(&path, 1.0, 1.0);
                state.thumbnails.insert(path, image.pixels().clone());
            }
            // A headless scene runs before `setup` makes the cache, and preloads.
            None => {
                if let Ok(bytes) = fs::read(&path) {
                    state.pending_thumbs.push((path, bytes));
                }
            }
        }
    }
}

/// Back on the Locker: decode the grid art again (released on leave). The card copies are on
/// disk, so this is a read and a decode on the image workers.
pub fn request_locker_grid(state: &mut State) {
    let art = locker_grid_art(state);
    decode_locker_art(state, art);
}

/// Leaving the Locker: let go of its decoded art (about 1 MB a card with its atlas copies), so
/// memory goes back down on the next page. Personalize art painted elsewhere stays.
pub fn release_locker_art(state: &mut State) {
    let keep = crate::screens::settings::personalize::in_use(&state.prefs.personal);
    let mut keys = locker_grid_art(state);
    for hero in &state.locker.heroes {
        keys.extend(hero.render.iter().cloned());
    }
    keys.extend(state.locker.general_bg.iter().cloned());
    for key in keys {
        if !keep.contains(&key) {
            state.thumbnails.remove(&key);
        }
    }
}

pub fn apply_locker(state: &mut State, result: Result<crate::screens::locker::Load, String>) {
    match result {
        Ok(load) => {
            let mut art: Vec<String> = Vec::new();
            // The grid's art only: card copies and name plates. A hero page asks for its full
            // render when it opens (`request_locker_art`).
            for hero in &load.heroes {
                art.extend(hero.card.iter().cloned());
                art.extend(hero.name_art.iter().cloned());
                art.extend(hero.custom.iter().cloned());
            }
            art.extend(load.general_card.iter().cloned());
            state.locker.heroes = load.heroes;
            state.locker.mods = load.mods;
            state.locker.categories = load.categories;
            state.locker.prefs = load.prefs;
            state.locker.briefs = load.briefs;
            state.locker.global = load.global;
            state.locker.general_bg = load.general_bg;
            state.locker.general_bg_size = load.general_bg_size;
            state.locker.general_card = load.general_card;
            state.locker.general_card_size = load.general_card_size;
            state.locker.pinned = load.pinned;
            state.locker.assets_dir = load.assets_dir;
            state.locker.catalog_label = load.catalog_label;
            state.locker.error = load.error;
            state.locker.loaded = true;
            // The Settings switch: a newly loaded list starts with every hero open.
            if crate::screens::settings::options::locker_expanded() {
                state.locker.expanded = state
                    .locker
                    .heroes
                    .iter()
                    .map(|hero| hero.codename.clone())
                    .collect();
            }
            // Start every decode now rather than on the next poll, so the grid fills as soon
            // as the workers finish. A rescan keeps what is already decoded. The cache reads
            // each file on its worker; the size argument only applies to SVG.
            decode_locker_art(state, art);
            // The grid shows the result; no success notice.
        }
        Err(error) => {
            state.locker.loaded = true;
            state.locker.error = Some(error.clone());
            state.status = error;
        }
    }
}

fn locker_prefs_path() -> PathBuf {
    data_dir().join(locker::LOCKER_PREFS_FILE)
}

fn locker_cards_dir() -> PathBuf {
    data_dir().join(locker::LOCKER_CARDS_DIR)
}

/// Store the favourites, shuffle and card choices and show them.
pub fn save_locker_prefs(state: &mut State) {
    if let Err(error) = locker::write_prefs(&locker_prefs_path(), &state.locker.prefs) {
        state.status = format!("Locker preferences could not be saved: {error}");
    }
}

pub fn toggle_locker_favorite(state: &mut State, hero: &str) {
    state.locker.prefs.toggle_favorite(hero);
    save_locker_prefs(state);
}

pub fn toggle_locker_shuffle(state: &mut State, hero: &str) {
    state.locker.prefs.toggle_shuffle(hero);
    save_locker_prefs(state);
}

/// The installed mods in the shape `deadlock_core::locker` writes with. The screen's rows
/// carry everything the writes need; the entry count is not one of them.
fn locker_mods(state: &State) -> Vec<locker::LockerMod> {
    state
        .locker
        .mods
        .iter()
        .map(|m| locker::LockerMod {
            slot: m.slot,
            file: m.file.clone(),
            enabled: m.enabled,
            key: m.key.clone(),
            entry_count: 0,
            heroes: m.heroes.clone(),
            general: m.general,
            sounds: m.sounds.clone(),
        })
        .collect()
}

/// Read the addon folder again after a Locker write, and the Locker with it.
fn after_locker_write(state: &mut State) {
    scan_addons_now(state);
    start_addon_scan(state);
    start_locker_load(state);
}

pub fn equip_locker_skin(state: &mut State, hero: &str, slot: u8) {
    use crate::notify::{self, Notice};
    let Some(game_root) = state.game_root.clone() else {
        notify::push(
            state,
            Notice::error("No Deadlock install was found").detail("Nothing was renamed."),
        );
        return;
    };
    match locker::equip_hero_skin(&game_root, &locker_mods(state), hero, slot, true) {
        Ok(_) => {
            notify::push(state, Notice::success(format!("Equipped {hero}'s slot {slot:02}")));
            after_locker_write(state);
        }
        Err(error) => notify::push(
            state,
            Notice::error("That skin was not equipped").detail(error.to_string()),
        ),
    }
}

pub fn move_locker_skin(state: &mut State, hero: &str, slot: u8, to_index: usize) {
    use crate::notify::{self, Notice};
    let Some(game_root) = state.game_root.clone() else {
        notify::push(
            state,
            Notice::error("No Deadlock install was found").detail("Nothing was renamed."),
        );
        return;
    };
    match locker::move_hero_skin(&game_root, &locker_mods(state), hero, slot, to_index, true) {
        Ok(report) if report.moves.is_empty() => {}
        Ok(report) => {
            // The installed library keeps a name per slot, so it moves with the files.
            let path = deadlock_core::import::library_path(&data_dir());
            if let Ok(mut library) = deadlock_core::import::read_library(&path) {
                deadlock_core::load_order::remap_library(&mut library, &report.moves);
                let _ = deadlock_core::import::write_library(&path, &library);
            }
            notify::push(state, Notice::success(format!("Changed {hero}'s skin order")));
            after_locker_write(state);
        }
        Err(error) => notify::push(
            state,
            Notice::error("That skin did not move").detail(error.to_string()),
        ),
    }
}

pub fn equip_locker_sound(state: &mut State, hero: &str, ability: u8, slot: u8) {
    use crate::notify::{self, Notice};
    let Some(game_root) = state.game_root.clone() else {
        notify::push(
            state,
            Notice::error("No Deadlock install was found").detail("Nothing was renamed."),
        );
        return;
    };
    match locker::equip_ability_sound(
        &game_root,
        &locker_mods(state),
        hero,
        ability,
        slot,
        true,
    ) {
        Ok(_) => {
            notify::push(state, Notice::success(format!("Equipped {hero}'s sound")));
            after_locker_write(state);
        }
        Err(error) => notify::push(
            state,
            Notice::error("That sound was not equipped").detail(error.to_string()),
        ),
    }
}

/// One random skin per shuffled hero, now. A launch with the same switch uses
/// `locker::shuffle_for_launch` with a fresh seed.
pub fn shuffle_now(state: &mut State) {    use crate::notify::{self, Notice};
    let Some(game_root) = state.game_root.clone() else {
        notify::push(
            state,
            Notice::error("No Deadlock install was found").detail("Nothing was renamed."),
        );
        return;
    };
    let heroes = state.locker.prefs.shuffle.clone();
    if heroes.is_empty() {
        notify::push(
            state,
            Notice::info("No hero shuffles yet").detail("Pick heroes in Shuffle on launch first."),
        );
        return;
    }
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    match locker::shuffle_for_launch(&game_root, &locker_mods(state), &heroes, seed, true) {
        Ok(report) if report.done.is_empty() && report.failed.is_empty() => {}
        Ok(report) => {
            let count = report.done.len();
            notify::push(
                state,
                Notice::success(plural_line(count, "skin shuffled", "skins shuffled")),
            );
            after_locker_write(state);
        }
        Err(error) => notify::push(
            state,
            Notice::error("The skins were not shuffled").detail(error.to_string()),
        ),
    }
}

fn plural_line(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The launch handler's hook: apply the launch shuffle for the heroes the user picked.
/// Silent when nothing shuffles, and a failure never blocks a launch. Returns whether a
/// slot changed. Safe to call with the Locker never opened this session: it rebuilds the
/// mod list from the installed VPKs.
#[allow(dead_code)] // The launch handler calls it; that line is in another owner's file.
pub fn shuffle_before_launch(state: &mut State) -> bool {
    let prefs = locker::read_prefs(&locker_prefs_path());
    if prefs.shuffle.is_empty() {
        return false;
    }
    let Some(game_root) = state.game_root.clone() else {
        return false;
    };
    let mods = if state.locker.mods.is_empty() {
        let addons: Vec<deadlock_core::addons::InstalledAddon> = state
            .addons
            .addons
            .iter()
            .map(|addon| deadlock_core::addons::InstalledAddon {
                slot: addon.slot,
                file: addon.file.clone(),
                size_bytes: addon.size_bytes,
                chunk_files: addon.chunk_files.clone(),
                entry_count: addon.entry_count,
                enabled: addon.enabled,
            })
            .collect();
        let addons_dir = deadlock_core::deploy::addons_dir(&game_root);
        locker::discover_locker(
            &empty_catalog(),
            &addons,
            Some(addons_dir.as_path()),
            &locker::LockerAssetPaths::default(),
        )
        .mods
    } else {
        locker_mods(state)
    };
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    match locker::shuffle_for_launch(&game_root, &mods, &prefs.shuffle, seed, true) {
        Ok(report) if report.done.is_empty() => false,
        Ok(_) => {
            scan_addons_now(state);
            start_addon_scan(state);
            true
        }
        Err(_) => false,
    }
}

/// Copy the chosen card image into the app data folder and point the hero at it.
pub fn save_locker_card(state: &mut State) {
    use crate::notify::{self, Notice};
    let Some(crate::screens::locker::Dialog::Card(dialog)) = state.locker.dialog.as_ref() else {
        return;
    };
    let hero = dialog.hero.clone();
    let path = dialog.path.clone();
    let focus = dialog.focus_x;
    let cards = locker_cards_dir();
    let current = state.locker.prefs.card(&hero).cloned();
    let previous = current.clone();
    let same_file = path.as_ref().is_some_and(|path| {
        current
            .as_ref()
            .is_some_and(|card| cards.join(&card.file).display().to_string() == *path)
    });
    let card = match (path, current, same_file) {
        (Some(_), Some(mut card), true) => {
            card.focus_x = focus.clamp(0.0, 100.0);
            card
        }
        (Some(path), _, _) => match locker::install_custom_card(&cards, &hero, Path::new(&path), focus)
        {
            Ok(card) => card,
            Err(error) => {
                if let Some(crate::screens::locker::Dialog::Card(dialog)) = state.locker.dialog.as_mut() {
                    dialog.error = Some(error.to_string());
                }
                return;
            }
        },
        (None, _, _) => return,
    };

    state.locker.prefs.set_card(&hero, card.clone());
    save_locker_prefs(state);
    if let Some(previous) = previous
        && previous.file != card.file
    {
        let _ = locker::remove_custom_card(&cards, &previous);
    }
    let file = cards.join(&card.file).display().to_string();
    let key = deadlock_core::locker::hero_asset_key(&hero);
    if let Some(found) = state
        .locker
        .heroes
        .iter_mut()
        .find(|found| deadlock_core::locker::hero_asset_key(&found.display_name) == key)
    {
        found.custom = Some(file.clone());
        found.custom_size = Some((card.width, card.height));
        found.custom_focus = card.focus_x;
    }
    request_locker_art(state, &file);
    state.locker.dialog = None;
    notify::push(state, Notice::success(format!("New card for {hero}")));
}

/// Forget a custom card and delete the copy. The hero's own render shows again.
pub fn remove_locker_card(state: &mut State, hero: &str) {
    use crate::notify::{self, Notice};
    if let Some(card) = state.locker.prefs.clear_card(hero)
        && let Err(error) = locker::remove_custom_card(&locker_cards_dir(), &card)
    {
        notify::push(
            state,
            Notice::error("The card file was not deleted").detail(error.to_string()),
        );
    }
    save_locker_prefs(state);
    let key = deadlock_core::locker::hero_asset_key(hero);
    if let Some(found) = state
        .locker
        .heroes
        .iter_mut()
        .find(|found| deadlock_core::locker::hero_asset_key(&found.display_name) == key)
    {
        found.custom = None;
        found.custom_size = None;
        found.custom_focus = found.face_x;
    }
    if matches!(
        state.locker.dialog,
        Some(crate::screens::locker::Dialog::Card(_))
    ) {
        state.locker.dialog = None;
    }
    notify::push(state, Notice::info(format!("Removed {hero}'s card")));
}

/// Add or remove the Global priority folder in gameinfo.gi.
pub fn set_locker_global(state: &mut State, on: bool) {
    use crate::notify::{self, Notice};
    let Some(game_root) = state.game_root.clone() else {
        notify::push(
            state,
            Notice::error("No Deadlock install was found").detail("Nothing was written."),
        );
        return;
    };
    let gameinfo = deadlock_core::deploy::gameinfo_path(&game_root);
    let result = if on {
        locker::ensure_global_path(&gameinfo, true)
    } else {
        locker::remove_global_path(&gameinfo, true)
    };
    match result {
        Ok(edit) => {
            state.locker.global.configured = on;
            if on {
                state.locker.global.folder = true;
            }
            let notice = if on {
                Notice::success("Global mods load first")
            } else {
                Notice::info("Global mods folder turned off")
            };
            let notice = if edit.changed {
                notice.undo(move |st: &mut State| set_locker_global(st, !on))
            } else {
                notice
            };
            notify::push(state, notice);
        }
        Err(error) => notify::push(
            state,
            Notice::error("The Global mods folder was not changed").detail(error.to_string()),
        ),
    }
}

/// Delete one recolor brief from the workspace.
pub fn remove_locker_brief(state: &mut State, hero: &str) {
    use crate::notify::{self, Notice};
    let Some(brief) = state
        .locker
        .briefs
        .iter()
        .find(|brief| brief.hero == hero)
        .cloned()
    else {
        return;
    };
    let workspace = workspace_dir();
    let target = locker::LockerHero {
        codename: brief.hero.clone(),
        display_name: brief.display_name.clone(),
        render_path: None,
        name_art_path: None,
        mod_count: 0,
        asset_count: 0,
        in_catalog: false,
        recolor_paths: Vec::new(),
    };
    match deadlock_core::recolor::brief_path(&workspace, &target) {
        Ok(path) => match fs::remove_file(&path) {
            Ok(()) => {
                state.locker.briefs.retain(|brief| brief.hero != hero);
                notify::push(state, Notice::info(format!("Removed {}'s brief", brief.display_name)));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                state.locker.briefs.retain(|brief| brief.hero != hero);
            }
            Err(error) => notify::push(
                state,
                Notice::error("The brief was not deleted").detail(error.to_string()),
            ),
        },
        Err(error) => notify::push(
            state,
            Notice::error("The brief was not deleted").detail(error.to_string()),
        ),
    }
}

// --- launch ----------------------------------------------------------------

pub fn read_launch(data: &Path) -> Result<crate::screens::launch::Read, String> {
    let workspace = workspace_dir();
    let status = discovery::read_game_status(None);
    let (gameinfo_present, gameinfo_addons, install) = match status {
        Ok(status) => (
            status
                .gameinfo
                .as_ref()
                .map(|info| info.present)
                .unwrap_or(false),
            status
                .gameinfo
                .as_ref()
                .map(|info| info.addons_search_path)
                .unwrap_or(false),
            status.install,
        ),
        Err(error) => return Err(format!("Game discovery failed: {error}")),
    };
    let (addon_files, enabled) = match &install {
        Some(install) => {
            let report = addons::read_conflict_report(&install.game_root).ok();
            match report {
                Some(report) => (
                    report.addons.len(),
                    report.addons.iter().filter(|addon| addon.enabled).count(),
                ),
                None => (0, 0),
            }
        }
        None => (0, 0),
    };
    // The launch marker tells a crash from this session apart from an old dump. It is closed
    // when the game is gone, and opened when the game was started outside Sigil.
    let process = deadlock_core::launch::read_game_process();
    let marker = match deadlock_core::launch::read_marker(data) {
        Some(marker) if marker.open() && !process.running => {
            deadlock_core::launch::record_exit(data).ok().flatten()
        }
        Some(marker) => Some(marker),
        None if process.running => {
            deadlock_core::launch::record_launch(data, deadlock_core::launch::MODE_UNKNOWN).ok()
        }
        None => None,
    };
    let crash = marker.as_ref().and_then(|marker| {
        crash_advisory(
            &workspace,
            install.as_ref().map(|install| install.addons_dir.as_path()),
            data,
            marker,
        )
    });

    Ok(crate::screens::launch::Read {
        stash: deadlock_core::launch::stash_status(data),
        gameinfo_present,
        gameinfo_addons,
        addon_files,
        enabled,
        marker,
        crash,
    })
}

/// The newest crash dump the last launch has not explained, with the mod that most likely
/// caused it. Read on the launch worker: it opens the enabled VPKs only when a dump is new.
fn crash_advisory(
    workspace: &Path,
    addons_dir: Option<&Path>,
    data: &Path,
    marker: &deadlock_core::launch::LaunchMarker,
) -> Option<crate::screens::launch::Crash> {
    use deadlock_core::game_session;
    use deadlock_core::impact;
    let mut roots = deadlock_core::discovery::default_steam_roots();
    if let Some(custom) = crate::screens::settings::options::get().steam_root {
        roots.insert(0, PathBuf::from(custom));
    }
    let game_root = addons_dir.and_then(Path::parent).and_then(Path::parent);
    let dumps = game_session::list_dumps(&game_session::dump_folders(game_root, &roots));
    let dump = impact::unacknowledged_dump(&dumps, marker.started_secs, marker.acknowledged_secs)?;
    let at_secs = impact::dump_seconds(dump).unwrap_or(0);
    let info = game_session::read_dump_info(&dump.path);
    let changes = impact::recent_changes(workspace, std::time::Duration::from_secs(7 * 86_400), 20);

    // Names and shipped paths for the enabled slots: the library first, the journal next.
    let mut names: std::collections::BTreeMap<u8, String> = std::collections::BTreeMap::new();
    if let Ok(library) = deadlock_core::import::read_library(&deadlock_core::import::library_path(data))
    {
        for meta in &library.mods {
            if !meta.name.trim().is_empty() {
                names.insert(meta.slot, meta.name.clone());
            }
        }
    }
    for change in &changes {
        names.entry(change.slot).or_insert_with(|| change.name.clone());
    }
    let mut shipped: std::collections::BTreeMap<u8, Vec<String>> = std::collections::BTreeMap::new();
    if let Some(addons) = addons_dir
        && info.comment.is_some()
        && let Ok(installed) = deadlock_core::addons::list_installed(addons)
    {
        for addon in installed.iter().filter(|addon| addon.enabled) {
            let path = addons.join(&addon.file);
            let options = deadlock_core::vpk::ReadVpkOptions { keep_preload: false };
            if let Ok(directory) = deadlock_core::vpk::read_vpk_directory_with(&path, &options) {
                let paths = directory
                    .entries
                    .iter()
                    .map(|entry| entry.path.clone())
                    .collect();
                shipped.insert(addon.slot, paths);
            }
        }
    }
    let facts = impact::DumpFacts {
        path: dump.path.clone(),
        modified_secs: impact::dump_seconds(dump),
        exception: info.exception,
        comment: info.comment.clone(),
    };
    let suspect = impact::suspect(&facts, &changes, &names, &shipped);
    Some(crate::screens::launch::Crash {
        path: dump.path.display().to_string(),
        file: dump
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        when: deadlock_core::launch::clock_label(at_secs),
        at_secs,
        exception: info.exception.map(game_session::exception_label),
        comment: info.comment,
        suspect,
    })
}

pub fn start_launch_read(state: &mut State) {
    // The process monitor and the download worker start with the first launch read and stay
    // for the session: the running row and the queue do not depend on this page being open.
    state.launch.monitor.start(state.wake.clone());
    state
        .launch
        .downloads
        .ensure(data_dir(), workspace_dir(), state.wake.clone());
    if state.launch_job.is_running() {
        return;
    }
    let data = data_dir();
    let wake = state.wake.clone();
    state.launch.error = None;
    state.launch_job.start(wake, move || read_launch(&data));
}

/// Queue a link or a mod id from anywhere. Browse's install dialog can call this instead of
/// downloading inside its own job; the row reports what the worker found.
pub fn queue_install(state: &mut State, target: &str) {
    state
        .launch
        .downloads
        .ensure(data_dir(), workspace_dir(), state.wake.clone());
    state
        .launch
        .downloads
        .send(deadlock_core::download_queue::Command::Add(target.to_string()));
    state.screen = crate::model::Screen::Launch;
    // The page needs its launch read before the first frame on it.
    start_launch_read(state);
}

pub fn launch_now(state: &mut State) {
    let data = data_dir();
    apply_launch(state, read_launch(&data));
}

pub fn apply_launch(state: &mut State, result: Result<crate::screens::launch::Read, String>) {
    match result {
        Ok(read) => {
            let stashed = read
                .stash
                .as_ref()
                .map(|stash| stash.present > 0)
                .unwrap_or(false);
            state.launch.read = Some(read);
            state.launch.loaded = true;
            state.launch.error = None;
            if stashed {
                state.status =
                    "A vanilla stash is pending. Restore it before editing addons.".to_string();
            }
        }
        Err(error) => {
            state.launch.loaded = true;
            state.launch.error = Some(error.clone());
            state.status = error;
        }
    }
}

pub fn run_launch(state: &mut State, dialog: crate::screens::launch::Dialog) {
    use crate::screens::launch::Dialog;
    state.launch.dialog = Dialog::None;
    let data = data_dir();
    let result = match dialog {
        Dialog::Modded => deadlock_core::launch::launch_modded(true)
            .map(|launch| format!("Steam opener {} accepted {}.", launch.opener, launch.url)),
        Dialog::Vanilla => {
            let Some(game_root) = state.game_root.clone() else {
                state.launch.last =
                    Some("No Deadlock install was found. Nothing was stashed.".to_string());
                return;
            };
            deadlock_core::launch::launch_vanilla(&game_root, &data, true).map(|launch| {
                let files = launch
                    .stash_dir
                    .as_ref()
                    .map(|dir| dir.display().to_string())
                    .unwrap_or_else(|| "the stash".to_string());
                format!(
                    "Stashed addons into {files} and handed {} to Steam.",
                    launch.url
                )
            })
        }
        Dialog::Restore => deadlock_core::launch::restore_vanilla(&data, true).map(|restore| {
            format!(
                "Restored {} file(s) into {}.",
                restore.restored.len(),
                restore.addons_dir.display()
            )
        }),
        // The checklist and the stop confirm have their own dialogs; neither launches.
        Dialog::Checklist | Dialog::StopGame | Dialog::None => Ok(String::new()),
    };
    match result {
        Ok(message) => {
            if rune::is_headless() {
                state.launch.last = Some(format!(
                    "Headless run: {message} Nothing was sent to the platform opener."
                ));
            } else {
                // The launch marker makes the running row and the crash advisory work; the
                // presence check is asked for at once so Discord switches to Deadlock now.
                let mode = match dialog {
                    Dialog::Modded => Some(deadlock_core::launch::MODE_MODDED),
                    Dialog::Vanilla => Some(deadlock_core::launch::MODE_VANILLA),
                    _ => None,
                };
                if let Some(mode) = mode {
                    let _ = deadlock_core::launch::record_launch(&data, mode);
                    state.settings.game_running = true;
                    state.settings.last_game_check = None;
                }
                state.launch.last = Some(message);
            }
        }
        Err(error) => {
            state.launch.last = Some(error.to_string());
        }
    }
    state.launch.busy = false;
    start_launch_read(state);
    scan_addons_now(state);
    crate::banner::recheck(state);
}

// --- toolchain -------------------------------------------------------------

pub fn start_toolchain_read(state: &mut State) {
    if state.toolchain_job.is_running() {
        return;
    }
    let wake = state.wake.clone();
    state.toolchain.error = None;
    state.toolchain_job.start(wake, move || {
        Ok(deadlock_core::toolchain::toolchain_status())
    });
}

pub fn apply_toolchain(
    state: &mut State,
    result: Result<deadlock_core::toolchain::ToolchainStatus, String>,
) {
    match result {
        Ok(status) => {
            let available = status.tools.iter().filter(|tool| tool.available).count();
            state.status = format!(
                "Toolchain probed on {}: {} of {} tools available.",
                status.platform,
                available,
                status.tools.len()
            );
            state.toolchain.status = Some(status);
            state.toolchain.loaded = true;
            state.toolchain.error = None;
        }
        Err(error) => {
            state.toolchain.loaded = true;
            state.toolchain.error = Some(error.clone());
            state.status = error;
        }
    }
}

fn tool_paths(state: &State, stage: crate::screens::toolchain::Stage) -> (PathBuf, PathBuf) {
    use crate::screens::toolchain::Stage;
    let workspace = workspace_dir();
    match stage {
        Stage::Decompile => {
            let input = PathBuf::from(state.toolchain.decompile_input.text.trim());
            let out = if state.toolchain.decompile_out.text.trim().is_empty() {
                workspace.join("sources")
            } else {
                PathBuf::from(state.toolchain.decompile_out.text.trim())
            };
            (input, out)
        }
        Stage::Compile => {
            let source = PathBuf::from(state.toolchain.compile_source.text.trim());
            let out = if state.toolchain.compile_out.text.trim().is_empty() {
                workspace.join("staging")
            } else {
                PathBuf::from(state.toolchain.compile_out.text.trim())
            };
            (source, out)
        }
    }
}

pub fn plan_tool(state: &mut State, stage: crate::screens::toolchain::Stage) {
    use crate::screens::toolchain::Stage;
    let (input, out) = tool_paths(state, stage);
    let result = match stage {
        Stage::Decompile => {
            deadlock_core::toolchain::plan_decompile(&deadlock_core::toolchain::DecompileRequest {
                input,
                output_dir: out,
            })
            .map(|plan| {
                format!(
                    "would run: {:?} {}\ninput: {}\noutput: {}",
                    plan.program,
                    plan.args.join(" "),
                    plan.input.display(),
                    plan.output_dir.display()
                )
            })
        }
        Stage::Compile => {
            let gameinfo = state
                .toolchain
                .compile_gameinfo()
                .unwrap_or_else(|| workspace_dir().join("gameinfo.gi"));
            deadlock_core::toolchain::plan_compile(&deadlock_core::toolchain::CompileRequest {
                source: input,
                gameinfo,
                output_dir: out,
            })
            .map(|plan| {
                format!(
                    "would run: {:?} {}\nsource: {}\noutput: {}",
                    plan.program,
                    plan.args.join(" "),
                    plan.source.display(),
                    plan.output_dir.display()
                )
            })
        }
    };
    match result {
        Ok(log) => {
            state.toolchain.log = Some(log);
            state.toolchain.log_tone = crate::ui::Tone::Info;
        }
        Err(error) => {
            state.toolchain.log = Some(error.to_string());
            state.toolchain.log_tone = crate::ui::Tone::Bad;
        }
    }
}

pub fn run_tool(state: &mut State, stage: crate::screens::toolchain::Stage) {
    use crate::screens::toolchain::Stage;
    state.toolchain.pending = None;
    state.toolchain.busy = true;
    let (input, out) = tool_paths(state, stage);
    let result = match stage {
        Stage::Decompile => {
            deadlock_core::toolchain::decompile(&deadlock_core::toolchain::DecompileRequest {
                input,
                output_dir: out,
            })
            .map(|result| format!("{} output file(s)\n{}", result.outputs.len(), result.log))
        }
        Stage::Compile => {
            let gameinfo = state
                .toolchain
                .compile_gameinfo()
                .unwrap_or_else(|| workspace_dir().join("gameinfo.gi"));
            deadlock_core::toolchain::compile(&deadlock_core::toolchain::CompileRequest {
                source: input,
                gameinfo,
                output_dir: out,
            })
            .map(|result| format!("{} output file(s)\n{}", result.outputs.len(), result.log))
        }
    };
    state.toolchain.busy = false;
    match result {
        Ok(log) => {
            state.toolchain.log = Some(log);
            state.toolchain.log_tone = crate::ui::Tone::Ok;
        }
        Err(error) => {
            state.toolchain.log = Some(error.to_string());
            state.toolchain.log_tone = crate::ui::Tone::Bad;
        }
    }
}

// --- character -------------------------------------------------------------

pub fn load_character(workspace: &Path) -> Result<crate::screens::character::Load, String> {
    let records =
        deadlock_core::character::character_status(workspace).map_err(|error| error.to_string())?;
    let rows = records
        .iter()
        .map(crate::screens::character::row_from_record)
        .collect();
    Ok(crate::screens::character::Load {
        rows,
        workspace: workspace.display().to_string(),
    })
}

pub fn start_character_read(state: &mut State) {
    if state.character_job.is_running() {
        return;
    }
    let workspace = workspace_dir();
    let wake = state.wake.clone();
    state.character.error = None;
    state
        .character_job
        .start(wake, move || load_character(&workspace));
}

pub fn character_now(state: &mut State) {
    let workspace = workspace_dir();
    apply_character(state, load_character(&workspace));
}

pub fn apply_character(state: &mut State, result: Result<crate::screens::character::Load, String>) {
    match result {
        Ok(load) => {
            state.character.rows = load.rows;
            state.character.workspace = load.workspace;
            state.character.loaded = true;
            state.character.error = None;
        }
        Err(error) => {
            state.character.loaded = true;
            state.character.error = Some(error.clone());
            state.status = error;
        }
    }
}

pub fn create_character_task(state: &mut State) {
    let workspace = workspace_dir();
    let mut request = deadlock_core::character::CharacterRequest::new(
        state.character.prompt.text.clone(),
        deadlock_core::character::CharacterProviderKind::Meshy,
    );
    let reference = state.character.reference.text.trim();
    request.reference_image = if reference.is_empty() {
        None
    } else {
        Some(PathBuf::from(reference))
    };
    let hero = state.character.hero.text.trim();
    request.target_hero = if hero.is_empty() {
        None
    } else {
        Some(hero.to_string())
    };
    if !state.character.style.text.trim().is_empty() {
        request.style = state.character.style.text.trim().to_string();
    }
    request.allow_reference_upload = state.character.consent;
    request.rights_note =
        "created in the desktop app; the operator confirms rights to the reference".to_string();

    state.character.busy = true;
    let result = deadlock_core::character::create_character(&workspace, &request);
    state.character.busy = false;
    match result {
        Ok(record) => {
            state.character.last = Some(format!(
                "Created {} for {}. source2Compile is required and inGameReady stays false.",
                record.slug,
                record.request.provider.as_str()
            ));
            state.character.create_open = false;
            start_character_read(state);
        }
        Err(error) => {
            state.character.last = Some(error.to_string());
        }
    }
}

pub fn advance_character(state: &mut State, index: usize) {
    let Some(row) = state.character.rows.get(index).cloned() else {
        return;
    };
    let workspace = workspace_dir();
    state.character.busy = true;
    let result = deadlock_core::character::refresh_character(&workspace, &row.slug);
    state.character.busy = false;
    match result {
        Ok(record) => {
            state.character.last = Some(format!(
                "{} is {} at {}%.",
                record.slug,
                record.task.status.as_str(),
                record.task.progress
            ));
            start_character_read(state);
        }
        Err(error) => {
            state.character.last = Some(error.to_string());
        }
    }
}

pub fn import_character_asset(state: &mut State, index: usize) {
    let Some(row) = state.character.rows.get(index).cloned() else {
        return;
    };
    let workspace = workspace_dir();
    state.character.busy = true;
    let result = deadlock_core::character::import_character(&workspace, &row.slug);
    state.character.busy = false;
    match result {
        Ok(import) => {
            state.character.last = Some(format!(
                "Imported {} ({} bytes). Compile requirements: {}.",
                import.asset.path,
                import.asset.bytes,
                import.compile_requirements.len()
            ));
            start_character_read(state);
        }
        Err(error) => {
            state.character.last = Some(error.to_string());
        }
    }
}

pub fn toolchain_now(state: &mut State) {
    apply_toolchain(state, Ok(deadlock_core::toolchain::toolchain_status()));
}

pub fn profiles_now(state: &mut State) {
    let result = crate::screens::profiles::load(&workspace_dir(), state.game_root.as_deref());
    apply_profile_list(state, result);
}

// --- install ---------------------------------------------------------------

fn plan_summary(plan: &deadlock_core::install::InstallPlan) -> crate::screens::browse::PlanSummary {
    crate::screens::browse::PlanSummary {
        id: plan.id.clone(),
        source_label: plan.source_label.clone(),
        slot: plan.slot,
        target: plan.target.display().to_string(),
        files: plan.files.len(),
        bytes: plan.bytes,
        conflicts: plan.conflicts.len(),
        replace_required: plan.replace_required,
        replaced: plan
            .replaced
            .iter()
            .map(|path| path.display().to_string())
            .collect(),
        gameinfo_repair: plan.gameinfo_repair,
        warnings: plan.warnings.clone(),
        unmapped: plan.unmapped.clone(),
        artifact: match &plan.artifact {
            deadlock_core::install::InstallArtifact::Vpk {
                dir_path,
                chunk_paths,
            } => format!(
                "VPK from the archive · {} · {} chunk(s)",
                dir_path.display(),
                chunk_paths.len()
            ),
            deadlock_core::install::InstallArtifact::Staging { path } => {
                format!("compiled tree packed into a VPK · {}", path.display())
            }
        },
    }
}

pub fn start_install_plan(state: &mut State) {
    if state.install_job.is_running() {
        return;
    }
    let target = state.install.target.trim().to_string();
    if target.is_empty() {
        state.install.log = Some("An install target is required.".to_string());
        state.install.tone = crate::ui::Tone::Bad;
        return;
    }
    let slot = state.install.slot_value();
    let force = state.install.force;
    let one_click = state.install.one_click;
    let vpk = state.install.vpk.clone();
    let workspace = workspace_dir();
    let game_root = state.game_root.clone();
    let wake = state.wake.clone();
    state.install.busy = true;
    state.install.vpks.clear();
    state.install.log = Some("Downloading, extracting and validating on a worker.".to_string());
    state.install.tone = crate::ui::Tone::Info;
    state.install_job.start(wake, move || {
        let outcome = (|| {
            let request = deadlock_core::install::InstallRequest::new(target, workspace.clone())
                .map_err(|error| error.to_string())?
                .with_slot(slot)
                .with_force(force)
                .with_one_click(one_click)
                .with_vpk(vpk);
            let options = deadlock_core::install::PlanOptions {
                game_root,
                ..Default::default()
            };
            let plan = deadlock_core::install::plan_install(&request, &options)
                .map_err(|error| error.to_string())?;
            deadlock_core::install::store_plan(&plan).map_err(|error| error.to_string())?;
            Ok::<_, String>(plan_summary(&plan))
        })();
        Ok(match outcome {
            Ok(plan) => crate::screens::browse::InstallOutcome {
                plan: Some(plan),
                applied: None,
                error: None,
            },
            Err(error) => crate::screens::browse::InstallOutcome {
                plan: None,
                applied: None,
                error: Some(error),
            },
        })
    });
}

/// Read the VPKs an archive holds after a plan refused to choose between them.
pub fn start_vpk_scan(state: &mut State) {
    if state.browse_view.vpk_job.is_running() || state.install.busy {
        return;
    }
    let target = state.install.target.trim().to_string();
    if target.is_empty() {
        return;
    }
    let slot = state.install.slot_value();
    let force = state.install.force;
    let one_click = state.install.one_click;
    let workspace = workspace_dir();
    let wake = state.wake.clone();
    state.install.busy = true;
    state.browse_view.vpk_job.start(wake, move || {
        let request = deadlock_core::install::InstallRequest::new(target, workspace)
            .map_err(|error| error.to_string())?
            .with_slot(slot)
            .with_force(force)
            .with_one_click(one_click);
        let options = deadlock_core::install::PlanOptions::default();
        deadlock_core::install::vpk_choices(&request, &options)
            .map_err(|error| error.to_string())
    });
}

pub fn start_install_apply(state: &mut State) {
    if state.install_job.is_running() {
        return;
    }
    // Settings > Game > Block changes while the game runs.
    if crate::screens::settings::watch::refuse_while_running(state) {
        return;
    }
    let after_root = state.game_root.clone();
    let switch_variants = crate::screens::settings::options::switch_variants();
    let Some(summary) = state.install.plan.clone() else {
        return;
    };
    let workspace = workspace_dir();
    let wake = state.wake.clone();
    // What the Browse card showed, remembered for the Installed card once the slot is written.
    let mod_id = state.install.target.trim().parse::<u64>().ok();
    let card = mod_id.and_then(|id| {
        state
            .browse
            .page
            .as_ref()
            .and_then(|page| page.mods.iter().find(|card| card.id == id))
    });
    let meta_name = card
        .map(|card| card.name.clone())
        .unwrap_or_else(|| state.install.label.clone());
    let meta_author = card.map(|card| card.author.clone()).filter(|a| !a.is_empty());
    let meta_thumb = card.and_then(|card| card.thumbnail_url.clone());
    let library = deadlock_core::import::library_path(&data_dir());
    let land_disabled = state.prefs.land_disabled;
    state.install.busy = true;
    state.install_job.start(wake, move || {
        let outcome = (|| {
            let plan = deadlock_core::install::load_plan(&workspace, &summary.id)
                .map_err(|error| error.to_string())?;
            // Browse installs land switched off unless Settings says otherwise; the user turns
            // them on in Installed.
            let record = deadlock_core::install::apply_install_with(
                &plan,
                true,
                deadlock_core::install::ApplyOptions {
                    land_disabled,
                },
            )
            .map_err(|error| error.to_string())?;
            let bytes = std::fs::metadata(&record.target)
                .map(|meta| meta.len())
                .unwrap_or(0);
            let meta = deadlock_core::import::ModMeta {
                slot: plan.slot,
                size_bytes: bytes,
                name: meta_name.trim().to_string(),
                author: meta_author,
                thumbnail_url: meta_thumb,
                gamebanana_id: mod_id,
                source: "browse".to_string(),
                recorded_at: String::new(),
            };
            if !meta_name.trim().is_empty() {
                let _ = deadlock_core::import::record_meta(&library, meta.clone());
            }
            if let Some(root) = &after_root {
                // Settings > Mods > Switch variants: other files of this mod turn off.
                if switch_variants
                    && let Some(id) = mod_id
                    && let Ok(lib) = deadlock_core::import::read_library(&library)
                {
                    let _ = deadlock_core::mod_data::switch_variants(root, &lib, id, plan.slot, true);
                }
                // Settings > Experimental > Imprint installed mods.
                crate::screens::settings::tools::imprint_after_install(root, plan.slot, &meta);
            }
            Ok::<_, String>((
                plan_summary(&plan),
                format!(
                    "Installed in slot {:02}, switched off. Turn it on in Installed.",
                    plan.slot
                ),
            ))
        })();
        Ok(match outcome {
            Ok((plan, applied)) => crate::screens::browse::InstallOutcome {
                plan: Some(plan),
                applied: Some(applied),
                error: None,
            },
            Err(error) => crate::screens::browse::InstallOutcome {
                plan: None,
                applied: None,
                error: Some(error),
            },
        })
    });
}

pub fn apply_install_outcome(state: &mut State, outcome: crate::screens::browse::InstallOutcome) {
    state.install.busy = false;
    let plan_landed = outcome.plan.is_some() && outcome.applied.is_none() && outcome.error.is_none();
    let applied_ok = outcome.applied.is_some() && outcome.error.is_none();
    let failed = outcome.error.is_some();
    let error = outcome.error.clone();
    if let Some(plan) = outcome.plan {
        state.install.plan = Some(plan);
        state.install.vpks.clear();
        if state.install.tone != crate::ui::Tone::Bad {
            state.install.log = None;
        }
    }
    if let Some(applied) = outcome.applied {
        state.install.log = Some(applied.clone());
        state.install.tone = crate::ui::Tone::Ok;
        state.status = applied;
        scan_addons_now(state);
        start_addon_scan(state);
    }
    if let Some(error) = outcome.error {
        state.install.log = Some(error.clone());
        state.install.tone = crate::ui::Tone::Bad;
        state.status = error;
    }
    // An archive with several VPKs is a choice: read them and show the picker.
    if let Some(error) = error
        && error.contains("more than one pakNN_dir.vpk")
    {
        start_vpk_scan(state);
    }
    crate::screens::browse::after_install(state, plan_landed, applied_ok, failed);
}

// --- demos -----------------------------------------------------------------

pub fn load_demo_rows(workspace: &Path) -> crate::screens::stats::DemoLoad {
    match deadlock_core::demo::list_matches(workspace) {
        Ok(list) => crate::screens::stats::DemoLoad {
            rows: list
                .matches
                .iter()
                .map(|item| crate::screens::stats::DemoRow {
                    match_id: item.match_id.clone(),
                    source: item.source.clone(),
                    map: item.map.clone(),
                    duration_seconds: item.duration_seconds,
                    players: item.player_count,
                    events: item.event_count,
                    heat_cells: item.heatmap_cells,
                    has_api: item.has_api,
                    updated: item
                        .updated_at
                        .chars()
                        .take(16)
                        .collect::<String>()
                        .replace('T', " "),
                })
                .collect(),
            error: None,
        },
        Err(error) => crate::screens::stats::DemoLoad {
            rows: Vec::new(),
            error: Some(error.to_string()),
        },
    }
}

pub fn start_demo_list(state: &mut State) {
    if state.demo_job.is_running() {
        return;
    }
    let workspace = workspace_dir();
    let wake = state.wake.clone();
    state
        .demo_job
        .start(wake, move || Ok(load_demo_rows(&workspace)));
}

pub fn demos_now(state: &mut State) {
    let workspace = workspace_dir();
    apply_demo_list(state, Ok(load_demo_rows(&workspace)));
}

pub fn apply_demo_list(state: &mut State, result: Result<crate::screens::stats::DemoLoad, String>) {
    state.stats.demo_busy = false;
    match result {
        Ok(load) => {
            state.stats.demos = load.rows;
            state.stats.demo_error = load.error;
        }
        Err(error) => {
            state.stats.demo_error = Some(error);
        }
    }
}

pub fn import_demo_file(state: &mut State) {
    if state.demo_job.is_running() {
        return;
    }
    let path = state.stats.demo_path.text.trim().to_string();
    if path.is_empty() {
        state.stats.demo_error = Some("A .dem file path is required.".to_string());
        return;
    }
    let workspace = workspace_dir();
    let wake = state.wake.clone();
    state.stats.demo_busy = true;
    state.stats.demo_error = None;
    state.demo_job.start(wake, move || {
        let demo = PathBuf::from(path);
        let mut load = load_demo_rows(&workspace);
        match deadlock_core::demo::import_demo(&workspace, &demo, None) {
            Ok(result) => {
                load = load_demo_rows(&workspace);
                if !result.provenance.missing_fields.is_empty() {
                    load.error = Some(format!(
                        "Imported {}. Fields the container extractor cannot read: {}.",
                        result.match_id,
                        result.provenance.missing_fields.join(", ")
                    ));
                }
            }
            Err(error) => {
                load.error = Some(error.to_string());
            }
        }
        Ok(load)
    });
}

pub fn set_addon_enabled(state: &mut State, slot: u8, enable: bool) {
    toggle_addon(state, slot, enable, true);
}

/// The slot switch's rename. The toast offers Undo (the opposite rename) unless this call is
/// itself the undo.
fn toggle_addon(state: &mut State, slot: u8, enable: bool, offer_undo: bool) {
    use crate::notify::{self, Notice};
    let Some(game_root) = state.game_root.clone() else {
        state.status = "No Deadlock install was found. Nothing was renamed.".to_string();
        state.addon_dialog = None;
        return;
    };
    state.addon_dialog = None;
    match addons::set_enabled(&game_root, slot, enable, true) {
        Ok(result) => {
            let what = state
                .installed
                .library
                .mods
                .iter()
                .find(|meta| meta.slot == result.slot && !meta.name.trim().is_empty())
                .map(|meta| meta.name.clone())
                .unwrap_or_else(|| format!("slot {:02}", result.slot));
            let title = format!("Turned {} {what}", if enable { "on" } else { "off" });
            let notice = Notice::success(title);
            let notice = if offer_undo {
                notice.undo(move |st: &mut State| toggle_addon(st, slot, !enable, false))
            } else {
                notice
            };
            notify::push(state, notice);
            scan_addons_now(state);
            start_addon_scan(state);
        }
        Err(error) => {
            state.status = error.to_string();
        }
    }
}

// --- review ----------------------------------------------------------------

pub fn open_review(state: &mut State, match_id: &str) {
    let workspace = workspace_dir();
    match deadlock_core::demo::read_summary(&workspace, match_id) {
        Ok(summary) => {
            let facts = deadlock_core::review::facts_for(&summary);
            state.stats.review = Some(crate::screens::stats::ReviewPanel {
                match_id: match_id.to_string(),
                facts,
                body: TextEdit::new(""),
                log: Some(
                    "The prompt and the allowed numbers are from the stored summary only. Nothing was sent anywhere."
                        .to_string(),
                ),
                tone: crate::ui::Tone::Info,
            });
        }
        Err(error) => {
            state.stats.demo_error = Some(error.to_string());
        }
    }
}

pub fn save_review(state: &mut State) {
    let workspace = workspace_dir();
    let Some(panel) = state.stats.review.as_ref() else {
        return;
    };
    if panel.body.text.trim().is_empty() {
        if let Some(panel) = state.stats.review.as_mut() {
            panel.log = Some("The reply is empty. Nothing was written.".to_string());
            panel.tone = crate::ui::Tone::Warn;
        }
        return;
    }
    let facts = panel.facts.clone();
    let body = panel.body.text.clone();
    let report = deadlock_core::review::lint_review(&body, &facts);
    if !report.ok() {
        let lines: Vec<String> = report
            .violations
            .iter()
            .take(6)
            .map(|violation| {
                format!(
                    "line {}: {} is not in the fact set",
                    violation.line, violation.number
                )
            })
            .collect();
        if let Some(panel) = state.stats.review.as_mut() {
            panel.log = Some(format!(
                "Refused: {} unsupported number(s). {}",
                report.violations.len(),
                lines.join(" · ")
            ));
            panel.tone = crate::ui::Tone::Bad;
        }
        return;
    }
    match deadlock_core::review::write_review(&workspace, &facts, &body) {
        Ok(stored) => {
            let path = deadlock_core::review::review_path(&workspace, &facts.match_id)
                .map(|path| path.display().to_string())
                .unwrap_or_default();
            if let Some(panel) = state.stats.review.as_mut() {
                panel.log = Some(format!(
                    "Accepted {} number(s) and wrote review.md: {}",
                    stored.checked_numbers, path
                ));
                panel.tone = crate::ui::Tone::Ok;
            }
            state.status = format!("Review saved for match {}.", facts.match_id);
        }
        Err(error) => {
            if let Some(panel) = state.stats.review.as_mut() {
                panel.log = Some(error.to_string());
                panel.tone = crate::ui::Tone::Bad;
            }
        }
    }
}

/// The durable state a `dah watch` loop writes, if one has run.
pub fn read_watch_state() -> Option<model::WatchState> {
    let path = workspace_dir().join(".dah").join("watch-state.json");
    let text = fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    Some(model::WatchState {
        updated_at: value
            .get("updatedAt")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown")
            .chars()
            .take(16)
            .collect::<String>()
            .replace('T', " "),
        interval_seconds: value
            .get("intervalSeconds")
            .and_then(|value| value.as_u64())
            .unwrap_or(0),
        newest_patch: value
            .get("newestPatch")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        total_projects: value
            .get("totalProjects")
            .and_then(|value| value.as_u64())
            .unwrap_or(0) as usize,
        needs_review: value
            .get("needsReview")
            .and_then(|value| value.as_u64())
            .unwrap_or(0) as usize,
    })
}

/// What this build can actually do on this machine, measured at startup.
pub fn build_capabilities() -> Vec<(String, String)> {
    let mut lines = Vec::new();
    lines.push((
        "Stats API".to_string(),
        "api.deadlock-api.com, keyless".to_string(),
    ));

    let demos = deadlock_core::demo::backend_status();
    lines.push((
        "Demo backend".to_string(),
        if demos.position_extraction_available {
            "positions and heatmaps available".to_string()
        } else {
            "container only, positions and heatmaps absent".to_string()
        },
    ));

    let tools = deadlock_core::install::ToolPaths::from_environment();
    lines.push((
        "Install archives".to_string(),
        format!(
            "zip in process, 7z {}, rar {}",
            if tools.seven_zip.is_some() {
                "available"
            } else {
                "missing"
            },
            if tools.unrar.is_some() {
                "available"
            } else {
                "missing"
            }
        ),
    ));

    let chain = deadlock_core::toolchain::toolchain_status();
    lines.push((
        "Decompile".to_string(),
        if chain.decompile_available {
            "ready".to_string()
        } else {
            "Source2Viewer-CLI not found".to_string()
        },
    ));
    lines.push((
        "Compile".to_string(),
        if chain.compile_available {
            "ready".to_string()
        } else if cfg!(target_os = "windows") {
            "resourcecompiler.exe not found".to_string()
        } else {
            "Windows and CSDK 12 only".to_string()
        },
    ));

    let assets = deadlock_core::locker::LockerAssetPaths::discover();
    lines.push((
        "Locker art".to_string(),
        assets
            .heroes_dir
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "no art root".to_string()),
    ));

    lines.push((
        "Character".to_string(),
        if std::env::var_os("DAH_MESHGEN_COMMAND").is_some() {
            "local command configured".to_string()
        } else if std::env::var_os("DAH_MESHGEN_URL").is_some() {
            "local HTTP model server configured".to_string()
        } else if std::env::var_os("MESHY_API_KEY").is_some() {
            "Meshy adapter configured".to_string()
        } else {
            "no generative backend".to_string()
        },
    ));

    lines
}

// --- crosshair -------------------------------------------------------------

pub fn import_crosshair(state: &mut State) {
    let Some(game_root) = state.game_root.clone() else {
        state.crosshair.log = Some(
            "No Deadlock install was found, so there is no machine_convars.vcfg to read."
                .to_string(),
        );
        state.crosshair.tone = crate::ui::Tone::Warn;
        return;
    };
    let vcfg = game_root
        .join("game")
        .join("citadel")
        .join("cfg")
        .join("machine_convars.vcfg");
    match fs::read_to_string(&vcfg) {
        Ok(text) => match deadlock_core::crosshair::parse_machine_convars(&text) {
            Some(crosshair) => {
                state.crosshair.settings = crosshair;
                state.crosshair.source = "in-game machine_convars.vcfg".to_string();
                state.crosshair.log =
                    Some(format!("Read the live crosshair from {}.", vcfg.display()));
                state.crosshair.tone = crate::ui::Tone::Ok;
            }
            None => {
                state.crosshair.log = Some(format!(
                    "{} has no citadel_crosshair_ convars yet. The game writes them after the first crosshair change.",
                    vcfg.display()
                ));
                state.crosshair.tone = crate::ui::Tone::Warn;
            }
        },
        Err(error) => {
            state.crosshair.log = Some(format!(
                "Could not read {}: {error}. The game may never have written it.",
                vcfg.display()
            ));
            state.crosshair.tone = crate::ui::Tone::Warn;
        }
    }
}

pub fn apply_crosshair(state: &mut State) {
    state.crosshair.pending = false;
    let Some(game_root) = state.game_root.clone() else {
        state.crosshair.log =
            Some("No Deadlock install was found. Nothing was written.".to_string());
        state.crosshair.tone = crate::ui::Tone::Bad;
        return;
    };
    let existing = match autoexec::read_autoexec(&game_root) {
        Ok(text) => text.unwrap_or_default(),
        Err(error) => {
            state.crosshair.log = Some(error.to_string());
            state.crosshair.tone = crate::ui::Tone::Bad;
            return;
        }
    };
    let merged =
        match deadlock_core::crosshair::apply_to_autoexec(&existing, &state.crosshair.settings) {
            Ok(text) => text,
            Err(error) => {
                state.crosshair.log = Some(error.to_string());
                state.crosshair.tone = crate::ui::Tone::Bad;
                return;
            }
        };
    match autoexec::write_autoexec(&game_root, &merged, true) {
        Ok(result) => {
            state.crosshair.log = Some(match &result.backup {
                Some(backup) => format!(
                    "Wrote the crosshair block into {} and copied the previous file to {}.",
                    result.path.display(),
                    backup.display()
                ),
                None => format!(
                    "Wrote the crosshair block into {}. There was no previous file to back up.",
                    result.path.display()
                ),
            });
            state.crosshair.tone = crate::ui::Tone::Ok;
            let notice = crate::notify::Notice::success("Crosshair applied")
                .detail("The game loads it on the next start.");
            // Undo puts back the autoexec.cfg this write copied aside. With no earlier file
            // there is nothing to put back, so no Undo.
            let notice = match result.backup {
                Some(backup) => notice.undo(move |st: &mut State| undo_crosshair(st, backup)),
                None => notice,
            };
            crate::notify::push(state, notice);
        }
        Err(error) => {
            state.crosshair.log = Some(error.to_string());
            state.crosshair.tone = crate::ui::Tone::Bad;
        }
    }
}

/// GAME WRITE, from a toast's Undo: restore the autoexec.cfg backup the crosshair apply wrote.
fn undo_crosshair(state: &mut State, backup: PathBuf) {
    use crate::notify::{self, Notice};
    match deadlock_core::backups::restore(&backup, true) {
        Ok(_) => {
            state.crosshair.log = Some("Put the previous autoexec.cfg back.".to_string());
            state.crosshair.tone = crate::ui::Tone::Ok;
            state.autoexec.loaded = false;
            notify::push(state, Notice::info("Put the previous autoexec.cfg back"));
        }
        Err(error) => notify::push(
            state,
            Notice::error("Could not undo the crosshair").detail(error.to_string()),
        ),
    }
}

// --- recolor ---------------------------------------------------------------

pub fn write_recolor_brief(state: &mut State) {
    let Some(panel) = state.locker.panel.as_ref() else {
        return;
    };
    let hero = deadlock_core::locker::LockerHero {
        codename: panel.codename.clone(),
        display_name: panel.display_name.clone(),
        render_path: None,
        name_art_path: None,
        mod_count: 0,
        asset_count: panel.targets.len(),
        in_catalog: panel.in_catalog,
        recolor_paths: panel.targets.clone(),
    };
    let palettes = deadlock_core::recolor::presets();
    let palette = palettes
        .get(panel.palette)
        .cloned()
        .unwrap_or_else(|| palettes[0].clone());
    let note = panel.note.text.clone();
    let workspace = workspace_dir();
    let result = deadlock_core::recolor::write_brief(&workspace, &hero, &palette, &note);
    if let Some(panel) = state.locker.panel.as_mut() {
        panel.log = Some(match result {
            Ok(path) => format!(
                "Brief written to {}. It lists {} target(s) and the steps still required; nothing is in game ready.",
                path.display(),
                hero.recolor_paths.len()
            ),
            Err(error) => error.to_string(),
        });
    }
}

// --- review generation -----------------------------------------------------

pub fn start_review_generation(state: &mut State) {
    if state.model_review_job.is_running() {
        return;
    }
    let Some(panel) = state.stats.review.as_ref() else {
        return;
    };
    let endpoint = state.local_endpoint.text.trim().to_string();
    let model = state.agent.model.clone().or_else(|| {
        if state.model_label.is_empty() || state.model_label == "No model" {
            None
        } else {
            Some(state.model_label.clone())
        }
    });
    let (Some(model), false) = (model, endpoint.is_empty()) else {
        if let Some(panel) = state.stats.review.as_mut() {
            panel.log = Some(
                "Set the local OpenAI-compatible endpoint in Settings and pick a model before generating."
                    .to_string(),
            );
            panel.tone = crate::ui::Tone::Warn;
        }
        return;
    };
    let facts = panel.facts.clone();
    let wake = state.wake.clone();
    if let Some(panel) = state.stats.review.as_mut() {
        panel.log = Some(format!("Calling {model} at {endpoint}. This stays local."));
        panel.tone = crate::ui::Tone::Info;
    }
    state.model_review_job.start(wake, move || {
        let config = deadlock_core::review::ModelConfig {
            base_url: endpoint,
            model,
            api_key: deadlock_core::review::key_from_environment(),
            timeout_seconds: 120,
        };
        Ok(deadlock_core::review::generate_review(&facts, &config)
            .map_err(|error| error.to_string()))
    });
}

pub fn apply_review_generation(state: &mut State, result: Result<String, String>) {
    let Some(panel) = state.stats.review.as_mut() else {
        return;
    };
    match result {
        Ok(text) => {
            panel.body = TextEdit::new(text);
            panel.log = Some(
                "Generated. Lint and save checks every number against the facts before writing."
                    .to_string(),
            );
            panel.tone = crate::ui::Tone::Ok;
        }
        Err(error) => {
            panel.log = Some(error);
            panel.tone = crate::ui::Tone::Bad;
        }
    }
}

// --- subscriptions ---------------------------------------------------------

pub fn refresh_subscriptions(state: &mut State) {
    let mut rows = Vec::new();
    for provider in deadlock_core::subscription::Provider::ALL {
        match deadlock_core::subscription::load(provider) {
            Ok(Some(tokens)) => rows.push(model::SubscriptionRow {
                id: provider.id().to_string(),
                signed_in: true,
                account: tokens.account_id.clone(),
                expires_in_seconds: tokens.expires_in_seconds(now_unix()),
                has_refresh: tokens.refresh_token.is_some(),
            }),
            Ok(None) => rows.push(model::SubscriptionRow {
                id: provider.id().to_string(),
                signed_in: false,
                account: None,
                expires_in_seconds: 0,
                has_refresh: false,
            }),
            Err(error) => {
                state.subscription_log = Some(error.to_string());
                rows.push(model::SubscriptionRow {
                    id: provider.id().to_string(),
                    signed_in: false,
                    account: None,
                    expires_in_seconds: 0,
                    has_refresh: false,
                });
            }
        }
    }
    state.subscriptions = rows;
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

/// The dah CLI: `DAH_RS_BIN`, else this executable as `cli` (`crate::helpers`).
fn dah_binary() -> Option<crate::helpers::Helper> {
    crate::helpers::cli().ok()
}

pub fn start_subscription_login(state: &mut State, provider: &str) {
    if rune::is_headless() {
        state.subscription_log =
            Some("Headless run: the browser sign-in was not started.".to_string());
        return;
    }
    let Some(provider) = deadlock_core::subscription::Provider::parse(provider) else {
        state.subscription_log = Some("Unknown subscription provider.".to_string());
        return;
    };
    let Some(binary) = dah_binary() else {
        state.subscription_log = Some(
            "Sigil could not locate its own executable to run the sign-in."
                .to_string(),
        );
        return;
    };
    let mut command = binary.command();
    command
        .args(["auth", "login", provider.id()])
        .env("DAH_DATA_DIR", &state.paths.data)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    match command.spawn() {
        Ok(_) => {
            state.subscription_log = Some(format!(
                "Opened the browser for {}. Finish sign-in, then press Refresh.",
                provider.id()
            ));
        }
        Err(error) => {
            state.subscription_log = Some(format!("Could not start {}: {error}", binary.label()));
        }
    }
}

pub fn use_subscription_for_agent(state: &mut State, provider: &str) {
    let Some(provider) = deadlock_core::subscription::Provider::parse(provider) else {
        state.subscription_log = Some("Unknown subscription provider.".to_string());
        return;
    };
    let logged_in = deadlock_core::subscription::load(provider)
        .ok()
        .flatten()
        .is_some();
    if !logged_in {
        state.subscription_log = Some(format!(
            "No {} login yet. Press Sign in and finish the browser flow first.",
            provider.id()
        ));
        return;
    }
    let model = state.host_model.text.trim().to_string();
    if model.is_empty() {
        state.subscription_log =
            Some("Enter the exact model id before using the subscription.".to_string());
        return;
    }
    match deadlock_core::subscription::save_host_config(provider, &model) {
        Ok(path) => {
            state.subscription_log = Some(format!(
                "The agent host will use {} with {}. Saved to {}.",
                provider.id(),
                model,
                path.display()
            ));
            // A running host picks up the new model now; one not started yet reads it later.
            if state.host_started {
                state.agent.shutdown();
                state.start_host();
            }
        }
        Err(error) => {
            state.subscription_log = Some(error.to_string());
        }
    }
}
