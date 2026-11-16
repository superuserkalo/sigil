//! Real reads from `deadlock-core` at startup.
//!
//! Only reads happen here: game status, the cached Steam news feed, an optional workspace
//! project and the latest release for that project. A missing install, cache or release leaves
//! the matching screen empty instead of inventing data. The review queue loads the game catalog,
//! so it only runs from a local scene or an explicit action, never on every frame.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use deadlock_core::{
    addons, autoexec, deploy, discovery, game_catalog, gamebanana, locker, migrate, patch_notes,
    profiles, project, release,
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
    state.paths.data = data_dir().display().to_string();
    let game_root = apply_game_status(&mut state);
    load_cached_patches(&mut state);
    load_workspace_project(&mut state);
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
                state.status =
                    "No Deadlock install found in the Steam roots. Open Settings to see the paths searched."
                        .to_string();
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
            if let Some(info) = &status.gameinfo
                && !info.addons_search_path
            {
                state.status = "Game found, but gameinfo.gi does not list citadel/addons. The game will ignore every addon until SearchPaths is repaired."
                    .to_string();
            }
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

const BROWSE_PER_PAGE: usize = 20;
const THUMB_TIMEOUT_SECONDS: u64 = 20;

/// Thumbnail bytes arrive one image at a time so the first card can paint while
/// the rest of the page is still downloading.
#[derive(Default)]
pub struct ThumbCrew {
    rx: Option<Receiver<(String, Option<Vec<u8>>)>>,
    remaining: usize,
}

impl ThumbCrew {
    pub fn start(&mut self, urls: Vec<String>, wake: Option<rune::Waker>) {
        self.clear();
        if urls.is_empty() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        self.remaining = urls.len();
        let spawned = std::thread::Builder::new()
            .name("dah-desktop-thumbs".to_string())
            .spawn(move || {
                for url in urls {
                    let bytes = fetch_thumb_bytes(&url);
                    let _ = tx.send((url, bytes));
                    if let Some(wake) = &wake {
                        wake();
                    }
                }
            });
        if spawned.is_err() {
            self.rx = None;
            self.remaining = 0;
        }
    }

    pub fn try_next(&mut self) -> Option<(String, Option<Vec<u8>>)> {
        let rx = self.rx.as_ref()?;
        match rx.try_recv() {
            Ok(item) => {
                self.remaining = self.remaining.saturating_sub(1);
                Some(item)
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.remaining = 0;
                None
            }
        }
    }

    pub fn is_loading(&self) -> bool {
        self.remaining > 0
    }

    pub fn clear(&mut self) {
        self.rx = None;
        self.remaining = 0;
    }
}

fn fetch_thumb_bytes(url: &str) -> Option<Vec<u8>> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(THUMB_TIMEOUT_SECONDS)))
        .build()
        .new_agent();
    let mut response = agent
        .get(url)
        .header("User-Agent", gamebanana::GAMEBANANA_USER_AGENT)
        .header("Accept", "image/*")
        .call()
        .ok()?;
    response.body_mut().read_to_vec().ok()
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
            if library.conflicts.len() == 1 { "" } else { "s" }
        )
    };
    state.addons = library;
}

fn map_browse_page(page: &gamebanana::BrowsePage) -> BrowsePageView {
    BrowsePageView {
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
            })
            .collect(),
    }
}

fn fetch_browse_page(page: usize) -> Result<BrowsePageView, String> {
    let page = gamebanana::fetch_deadlock_mods_page(page, BROWSE_PER_PAGE)
        .map_err(|error| error.to_string())?;
    Ok(map_browse_page(&page))
}

/// Kick a page read on a worker. The current page stays on screen while the
/// next one loads; the header shows that a read is in flight.
pub fn start_browse_fetch(state: &mut State, page: usize) {
    if state.browse_job.is_running() {
        return;
    }
    let page = page.max(1);
    let wake = state.wake.clone();
    state.browse.loading = true;
    state.browse.error = None;
    state.thumbnails.clear();
    state.browse_job.start(wake, move || fetch_browse_page(page));
}

/// The synchronous form for a headless scene. Page and thumbnails are fetched
/// before the first frame so a snapshot shows what the feed actually returned.
pub fn browse_now(state: &mut State, page: usize) {
    state.browse.loading = true;
    state.browse.error = None;
    state.thumbnails.clear();
    state.pending_thumbs.clear();
    match fetch_browse_page(page.max(1)) {
        Ok(view) => {
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
            state.browse.page = Some(view);
            state.browse.loading = false;
            state.status = format!(
                "GameBanana read: {} mods on {}.",
                state
                    .browse
                    .page
                    .as_ref()
                    .map(|page| page.mods.len())
                    .unwrap_or(0),
                state
                    .browse
                    .page
                    .as_ref()
                    .map(BrowsePageView::page_label)
                    .unwrap_or_else(|| "page 1".to_string())
            );
        }
        Err(error) => {
            state.browse.loading = false;
            state.browse.error = Some(error.clone());
            state.status = format!("GameBanana read failed: {error}");
        }
    }
}

pub fn apply_browse_page(state: &mut State, result: Result<BrowsePageView, String>) {
    state.browse.loading = false;
    match result {
        Ok(view) => {
            let urls: Vec<String> = view
                .mods
                .iter()
                .filter_map(|card| card.thumbnail_url.clone())
                .collect();
            state.browse.error = None;
            state.browse.page = Some(view);
            state.thumbnails.clear();
            state.thumb_crew.start(urls, state.wake.clone());
            state.status = format!(
                "GameBanana read: {} mods on {}.",
                state
                    .browse
                    .page
                    .as_ref()
                    .map(|page| page.mods.len())
                    .unwrap_or(0),
                state
                    .browse
                    .page
                    .as_ref()
                    .map(BrowsePageView::page_label)
                    .unwrap_or_else(|| "page 1".to_string())
            );
        }
        Err(error) => {
            state.browse.error = Some(error.clone());
            state.status = format!("GameBanana read failed: {error}");
        }
    }
}

/// Load the queued thumbnails into the image cache. Called from `poll` on the UI
/// thread; decode still happens on Rune's image workers.
pub fn drain_thumbnails(state: &mut State) -> bool {
    let mut changed = false;
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
            let image = cache.load(bytes);
            state.thumbnails.insert(url, image.pixels().clone());
            changed = true;
        }
    }
    changed
}

/// Kick one review queue pass on a worker. It loads the game catalog, so this
/// never runs from a view.
pub fn start_review_scan(state: &mut State) {
    if state.review_job.is_running() {
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
    let result = std::process::Command::new("explorer")
        .arg(format!("/select,{}", path.display()))
        .spawn()
        .map(|_| ());
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open")
        .arg(path.parent().unwrap_or(&path))
        .spawn()
        .map(|_| ());
    state.status = match result {
        Ok(()) => format!("Revealed {} in the file manager.", path.display()),
        Err(error) => format!("The file manager could not open {}: {error}", path.display()),
    };
}

// --- profiles --------------------------------------------------------------

pub fn start_profile_list(state: &mut State) {
    if state.profiles_job.is_running() {
        return;
    }
    let workspace = workspace_dir();
    let wake = state.wake.clone();
    state.profiles.error = None;
    state.profiles_job.start(wake, move || {
        let list =
            profiles::list_profiles(&workspace).map_err(|error| error.to_string())?;
        let rows = list
            .iter()
            .map(|profile| crate::screens::profiles::Row {
                name: profile.name.clone(),
                slug: profile.slug.clone(),
                updated: profile
                    .updated_at
                    .chars()
                    .take(16)
                    .collect::<String>()
                    .replace('T', " "),
                enabled: profile.enabled_slots(),
                total: profile.slots.len(),
                slots: profile
                    .slots
                    .iter()
                    .map(|slot| {
                        format!(
                            "{:02}{}",
                            slot.slot,
                            if slot.enabled { "+" } else { "-" }
                        )
                    })
                    .collect(),
            })
            .collect();
        Ok(crate::screens::profiles::Load {
            rows,
            directory: profiles::profile_dir(&workspace).display().to_string(),
        })
    });
}

pub fn apply_profile_list(
    state: &mut State,
    result: Result<crate::screens::profiles::Load, String>,
) {
    match result {
        Ok(load) => {
            state.profiles.directory = load.directory;
            state.profiles.rows = load.rows;
            state.profiles.loaded = true;
            state.profiles.error = None;
        }
        Err(error) => {
            state.profiles.loaded = true;
            state.profiles.error = Some(error.clone());
            state.status = error;
        }
    }
}

pub fn capture_profile(state: &mut State) {
    let name = state.profiles.name.text.trim().to_string();
    if name.is_empty() {
        state.status = "A profile needs a name. Nothing was written.".to_string();
        return;
    }
    let Some(game_root) = state.game_root.clone() else {
        state.status = "No Deadlock install was found, so no slot was recorded.".to_string();
        return;
    };
    let workspace = workspace_dir();
    match profiles::capture_profile(&game_root, &workspace, &name) {
        Ok(profile) => {
            state.profiles.last = Some(format!(
                "Captured {} with {} enabled slot(s) of {} recorded.",
                profile.name,
                profile.enabled_slots(),
                profile.slots.len()
            ));
            state.profiles.dialog = crate::screens::profiles::Dialog::None;
            start_profile_list(state);
        }
        Err(error) => {
            state.profiles.last = Some(error.to_string());
        }
    }
}

pub fn apply_profile(state: &mut State, index: usize) {
    let Some(row) = state.profiles.rows.get(index).cloned() else {
        return;
    };
    let Some(game_root) = state.game_root.clone() else {
        state.profiles.last =
            Some("No Deadlock install was found. Apply refuses. Nothing was renamed.".to_string());
        return;
    };
    let workspace = workspace_dir();
    state.profiles.busy = true;
    let result = profiles::apply_profile(&game_root, &workspace, &row.slug, true);
    state.profiles.busy = false;
    state.profiles.dialog = crate::screens::profiles::Dialog::None;
    match result {
        Ok(applied) => {
            let mut line = format!(
                "Applied {}: {} slot rename(s).",
                row.name,
                applied.changed.len()
            );
            if !applied.warnings.is_empty() {
                line.push_str(&format!(" {} warning(s): {}", applied.warnings.len(), applied.warnings.join(" ")));
            }
            state.profiles.last = Some(line);
            scan_addons_now(state);
            start_profile_list(state);
        }
        Err(error) => {
            state.profiles.last = Some(error.to_string());
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
            let text = text.unwrap_or_default();
            state.autoexec.path = path.display().to_string();
            state.autoexec.exists = !text.is_empty() || path.is_file();
            state.autoexec_original = text.clone();
            state.autoexec.editor = TextEdit::new(text);
            state.autoexec.loaded = true;
            state.autoexec.error = None;
        }
        Err(error) => {
            state.autoexec.loaded = true;
            state.autoexec.error = Some(error.to_string());
        }
    }
}

pub fn apply_autoexec(
    state: &mut State,
    result: Result<crate::screens::autoexec::Read, String>,
) {
    match result {
        Ok(read) => {
            state.autoexec.path = read.path;
            state.autoexec.exists = read.exists;
            state.autoexec_original = read.text.clone();
            state.autoexec.editor = TextEdit::new(read.text);
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

pub fn save_autoexec(state: &mut State) {
    let Some(game_root) = state.game_root.clone() else {
        state.autoexec.last =
            Some("No Deadlock install was found. Nothing was written.".to_string());
        return;
    };
    state.autoexec.busy = true;
    let result = autoexec::write_autoexec(&game_root, &state.autoexec.editor.text, true);
    state.autoexec.busy = false;
    state.autoexec.confirm_save = false;
    match result {
        Ok(written) => {
            let text = state.autoexec.editor.text.clone();
            state.autoexec_original = autoexec::normalize_trailing_newline(&text);
            state.autoexec.exists = true;
            state.autoexec.path = written.path.display().to_string();
            state.autoexec.last = Some(match written.backup {
                Some(backup) => format!(
                    "Wrote {} and copied the previous file to {}.",
                    written.path.display(),
                    backup.display()
                ),
                None => format!("Wrote {}. There was no previous file to back up.", written.path.display()),
            });
        }
        Err(error) => {
            state.autoexec.last = Some(error.to_string());
        }
    }
}

// --- stats -----------------------------------------------------------------

fn stats_account(state: &State) -> Option<u64> {
    let text = state.stats.account.text.trim();
    if text.is_empty() {
        std::env::var("DAH_STEAM_ID")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
    } else {
        text.parse::<u64>().ok()
    }
}

pub fn start_stats_load(state: &mut State) {
    if state.stats_job.is_running() {
        return;
    }
    let Some(account_id) = stats_account(state) else {
        state.stats.error =
            Some("A Steam account id is required. It is the numeric id, not a profile name.".to_string());
        state.status = state.stats.error.clone().unwrap_or_default();
        return;
    };
    let wake = state.wake.clone();
    state.stats.error = None;
    state.stats_job.start(wake, move || {
        let client = deadlock_core::stats::StatsClient::new();
        let snapshot = client
            .fetch_snapshot(account_id, 20)
            .map_err(|error| error.to_string());
        let heatmap = client
            .fetch_kill_heatmap(account_id)
            .map(|heat| crate::screens::stats::bucket(&heat, 56, 56))
            .map_err(|error| error.to_string());
        Ok(crate::screens::stats::Load {
            account_id,
            snapshot,
            heatmap,
        })
    });
}

pub fn stats_now(state: &mut State) {
    let Some(account_id) = stats_account(state) else {
        state.stats.error =
            Some("A Steam account id is required. It is the numeric id, not a profile name.".to_string());
        return;
    };
    let client = deadlock_core::stats::StatsClient::new();
    let snapshot = client
        .fetch_snapshot(account_id, 20)
        .map_err(|error| error.to_string());
    let heatmap = client
        .fetch_kill_heatmap(account_id)
        .map(|heat| crate::screens::stats::bucket(&heat, 56, 56))
        .map_err(|error| error.to_string());
    apply_stats(
        state,
        Ok(crate::screens::stats::Load {
            account_id,
            snapshot,
            heatmap,
        }),
    );
}

pub fn apply_stats(
    state: &mut State,
    result: Result<crate::screens::stats::Load, String>,
) {
    match result {
        Ok(load) => {
            state.stats.loaded = true;
            state.stats.account = TextEdit::new(load.account_id.to_string());
            match load.snapshot {
                Ok(snapshot) => {
                    state.stats.snapshot = Some(snapshot);
                    state.stats.error = None;
                    state.status = format!(
                        "Loaded player stats for account {}.",
                        load.account_id
                    );
                }
                Err(error) => {
                    state.stats.snapshot = None;
                    state.stats.error = Some(error.clone());
                    state.status = error;
                }
            }
            match load.heatmap {
                Ok(heat) => {
                    state.stats.heat = Some(heat);
                    state.stats.heat_error = None;
                }
                Err(error) => {
                    state.stats.heat = None;
                    state.stats.heat_error = Some(error);
                }
            }
        }
        Err(error) => {
            state.stats.loaded = true;
            state.stats.error = Some(error.clone());
            state.status = error;
        }
    }
}

// --- locker ----------------------------------------------------------------

const ART_BYTE_CAP: u64 = 24 * 1024 * 1024;

fn read_art(path: &Path) -> Option<(String, Vec<u8>)> {
    let size = fs::metadata(path).ok()?.len();
    if size == 0 || size > ART_BYTE_CAP {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    Some((path.display().to_string(), bytes))
}

pub fn load_locker(workspace: &Path) -> Result<crate::screens::locker::Load, String> {
    let install = discovery::read_game_status(None)
        .ok()
        .and_then(|status| status.install);
    let Some(install) = install else {
        return Err(
            "No Deadlock install was found, so the game catalog has no hero folders to read."
                .to_string(),
        );
    };
    let options = game_catalog::LoadGameCatalogOptions {
        game_root: install.game_root.clone(),
        cache_dir: workspace.join(".dah").join("index"),
        force: false,
    };
    let catalog = game_catalog::load_game_catalog(options)
        .map_err(|error| format!("The game catalog could not be loaded: {error}"))?;
    let addons = addons::read_conflict_report(&install.game_root)
        .map(|report| report.addons)
        .unwrap_or_default();
    let assets = locker::LockerAssetPaths::discover();
    let heroes = locker::discover_heroes_with_assets(&catalog.catalog, &addons, &assets);
    let rows = heroes
        .iter()
        .map(|hero| crate::screens::locker::HeroCard {
            codename: hero.codename.clone(),
            display_name: hero.display_name.clone(),
            render: hero.render_path.as_deref().and_then(read_art),
            name_art: hero.name_art_path.as_deref().and_then(read_art),
            mod_count: hero.mod_count,
            asset_count: hero.asset_count,
            in_catalog: hero.in_catalog,
        })
        .collect();
    let assets_dir = assets
        .heroes_dir
        .as_ref()
        .map(|dir| dir.display().to_string())
        .unwrap_or_default();
    Ok(crate::screens::locker::Load {
        heroes: rows,
        assets_dir,
        catalog_label: format!(
            "{} paths from {} source{}",
            catalog.catalog.entries.len(),
            catalog.catalog.sources.len(),
            if catalog.catalog.sources.len() == 1 { "" } else { "s" }
        ),
        error: None,
    })
}

pub fn start_locker_load(state: &mut State) {
    if state.locker_job.is_running() {
        return;
    }
    let workspace = workspace_dir();
    let wake = state.wake.clone();
    state.locker.error = None;
    state.locker_job
        .start(wake, move || load_locker(&workspace));
}

pub fn locker_now(state: &mut State) {
    let workspace = workspace_dir();
    apply_locker(state, load_locker(&workspace));
}

pub fn apply_locker(
    state: &mut State,
    result: Result<crate::screens::locker::Load, String>,
) {
    match result {
        Ok(load) => {
            state.locker.heroes = load.heroes;
            state.locker.assets_dir = load.assets_dir;
            state.locker.catalog_label = load.catalog_label;
            state.locker.error = load.error;
            state.locker.loaded = true;
            for hero in &state.locker.heroes {
                if let Some((path, bytes)) = &hero.render {
                    state.pending_thumbs.push((path.clone(), bytes.clone()));
                }
                if let Some((path, bytes)) = &hero.name_art {
                    state.pending_thumbs.push((path.clone(), bytes.clone()));
                }
            }
            state.status = format!(
                "Locker resolved {} heroes from the game catalog.",
                state.locker.heroes.len()
            );
        }
        Err(error) => {
            state.locker.loaded = true;
            state.locker.error = Some(error.clone());
            state.status = error;
        }
    }
}

// --- launch ----------------------------------------------------------------

pub fn read_launch(data: &Path) -> Result<crate::screens::launch::Read, String> {
    let status = discovery::read_game_status(None);
    let (gameinfo_present, gameinfo_addons, install) = match status {
        Ok(status) => (
            status.gameinfo.as_ref().map(|info| info.present).unwrap_or(false),
            status
                .gameinfo
                .as_ref()
                .map(|info| info.addons_search_path)
                .unwrap_or(false),
            status.install,
        ),
        Err(error) => return Err(format!("Game discovery failed: {error}")),
    };
    let (addons_dir, addon_files, enabled) = match &install {
        Some(install) => {
            let report = addons::read_conflict_report(&install.game_root).ok();
            match report {
                Some(report) => (
                    Some(install.addons_dir.display().to_string()),
                    report.addons.len(),
                    report
                        .addons
                        .iter()
                        .filter(|addon| addon.enabled)
                        .count(),
                ),
                None => (Some(install.addons_dir.display().to_string()), 0, 0),
            }
        }
        None => (None, 0, 0),
    };
    Ok(crate::screens::launch::Read {
        stash: deadlock_core::launch::stash_status(data),
        gameinfo_present,
        gameinfo_addons,
        addons_dir,
        addon_files,
        enabled,
        data_dir: data.display().to_string(),
    })
}

pub fn start_launch_read(state: &mut State) {
    if state.launch_job.is_running() {
        return;
    }
    let data = data_dir();
    let wake = state.wake.clone();
    state.launch.error = None;
    state.launch_job.start(wake, move || read_launch(&data));
}

pub fn launch_now(state: &mut State) {
    let data = data_dir();
    apply_launch(state, read_launch(&data));
}

pub fn apply_launch(
    state: &mut State,
    result: Result<crate::screens::launch::Read, String>,
) {
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
                state.status = "A vanilla stash is pending. Restore it before editing addons.".to_string();
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
                format!("Stashed addons into {files} and handed {} to Steam.", launch.url)
            })
        }
        Dialog::Restore => deadlock_core::launch::restore_vanilla(&data, true).map(|restore| {
            format!(
                "Restored {} file(s) into {}.",
                restore.restored.len(),
                restore.addons_dir.display()
            )
        }),
        Dialog::None => Ok(String::new()),
    };
    match result {
        Ok(message) => {
            if rune::is_headless() {
                state.launch.last = Some(format!(
                    "Headless run: {message} Nothing was sent to the platform opener."
                ));
            } else {
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
        Stage::Decompile => deadlock_core::toolchain::plan_decompile(
            &deadlock_core::toolchain::DecompileRequest {
                input,
                output_dir: out,
            },
        )
        .map(|plan| {
            format!(
                "would run: {:?} {}\ninput: {}\noutput: {}",
                plan.program,
                plan.args.join(" "),
                plan.input.display(),
                plan.output_dir.display()
            )
        }),
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
        Stage::Decompile => deadlock_core::toolchain::decompile(
            &deadlock_core::toolchain::DecompileRequest {
                input,
                output_dir: out,
            },
        )
        .map(|result| {
            format!(
                "{} output file(s)\n{}",
                result.outputs.len(),
                result.log
            )
        }),
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
            .map(|result| {
                format!(
                    "{} output file(s)\n{}",
                    result.outputs.len(),
                    result.log
                )
            })
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
    let records = deadlock_core::character::character_status(workspace)
        .map_err(|error| error.to_string())?;
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
    state.character_job
        .start(wake, move || load_character(&workspace));
}

pub fn character_now(state: &mut State) {
    let workspace = workspace_dir();
    apply_character(state, load_character(&workspace));
}

pub fn apply_character(
    state: &mut State,
    result: Result<crate::screens::character::Load, String>,
) {
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
    request.rights_note = "created in the desktop app; the operator confirms rights to the reference".to_string();

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
    let workspace = workspace_dir();
    let result = profiles::list_profiles(&workspace).map(|list| crate::screens::profiles::Load {
        rows: list
            .iter()
            .map(|profile| crate::screens::profiles::Row {
                name: profile.name.clone(),
                slug: profile.slug.clone(),
                updated: profile
                    .updated_at
                    .chars()
                    .take(16)
                    .collect::<String>()
                    .replace('T', " "),
                enabled: profile.enabled_slots(),
                total: profile.slots.len(),
                slots: profile
                    .slots
                    .iter()
                    .map(|slot| format!("{:02}{}", slot.slot, if slot.enabled { "+" } else { "-" }))
                    .collect(),
            })
            .collect(),
        directory: profiles::profile_dir(&workspace).display().to_string(),
    });
    apply_profile_list(state, result.map_err(|error| error.to_string()));
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
        replaced: plan.replaced.iter().map(|path| path.display().to_string()).collect(),
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
    let workspace = workspace_dir();
    let game_root = state.game_root.clone();
    let wake = state.wake.clone();
    state.install.busy = true;
    state.install.log =
        Some("Downloading, extracting and validating on a worker.".to_string());
    state.install.tone = crate::ui::Tone::Info;
    state.install_job.start(wake, move || {
        let outcome = (|| {
            let request =
                deadlock_core::install::InstallRequest::new(target, workspace.clone())
                    .map_err(|error| error.to_string())?
                    .with_slot(slot)
                    .with_force(force);
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

pub fn start_install_apply(state: &mut State) {
    if state.install_job.is_running() {
        return;
    }
    let Some(summary) = state.install.plan.clone() else {
        return;
    };
    let workspace = workspace_dir();
    let wake = state.wake.clone();
    state.install.busy = true;
    state.install_job.start(wake, move || {
        let outcome = (|| {
            let plan = deadlock_core::install::load_plan(&workspace, &summary.id)
                .map_err(|error| error.to_string())?;
            let record = deadlock_core::install::apply_install(&plan, true)
                .map_err(|error| error.to_string())?;
            let bytes = std::fs::metadata(&record.target)
                .map(|meta| meta.len())
                .unwrap_or(0);
            Ok::<_, String>((
                plan_summary(&plan),
                format!(
                    "Installed slot {:02} into {} ({} byte VPK, {} backup(s)).",
                    plan.slot,
                    record.target.display(),
                    bytes,
                    record.backups.len()
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

pub fn apply_install_outcome(
    state: &mut State,
    outcome: crate::screens::browse::InstallOutcome,
) {
    state.install.busy = false;
    if let Some(plan) = outcome.plan {
        state.install.plan = Some(plan);
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
                    result: item.result.clone(),
                    players: item.player_count,
                    events: item.event_count,
                    heat_cells: item.heatmap_cells,
                    has_api: item.has_api,
                    updated: item.updated_at.chars().take(16).collect::<String>().replace('T', " "),
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
    state.demo_job.start(wake, move || Ok(load_demo_rows(&workspace)));
}

pub fn demos_now(state: &mut State) {
    let workspace = workspace_dir();
    apply_demo_list(state, Ok(load_demo_rows(&workspace)));
}

pub fn apply_demo_list(
    state: &mut State,
    result: Result<crate::screens::stats::DemoLoad, String>,
) {
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
    let Some(game_root) = state.game_root.clone() else {
        state.status = "No Deadlock install was found. Nothing was renamed.".to_string();
        state.addon_dialog = None;
        return;
    };
    state.addon_dialog = None;
    match addons::set_enabled(&game_root, slot, enable, true) {
        Ok(result) => {
            state.status = format!(
                "Slot {:02}: renamed {} to {}.",
                result.slot, result.from, result.to
            );
            scan_addons_now(state);
            start_addon_scan(state);
        }
        Err(error) => {
            state.status = error.to_string();
        }
    }
}
