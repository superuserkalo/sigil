//! Folder checks (C11): Analyze addons, the HUD check, missing files before launch and
//! read-only files.
//!
//! - Deadlock Mod Manager: an addons analysis (what each folder holds, unknown files, a
//!   GameBanana lookup), a HUD layout check with console evidence, a missing-VPK check before
//!   launch and a prompt when files are read-only.
//! - Grimoire: none of the four as such.
//! - Sigil: Installed > More > Analyze the addons folder reads `addon_checks::analyze_addons` on a
//!   worker: each addon folder's mods, chunks, leftovers and size, slots with no library entry
//!   (each with Find which mod this is), doubled slots, read-only files and missing files (each
//!   with Reinstall). Check HUD mods runs `addon_checks::hud_check` on a worker. Before a modded
//!   launch, `missing_before_launch` warns once when an enabled mod lacks files. Clearing a
//!   read-only flag is a game write Sigil does not make yet: the list says where they are.

use std::path::PathBuf;

use deadlock_core::addon_checks::{self, AddonsAnalysis, HudCheck, MissingFile};
use rune::{El, col, row, spacer, text};

use crate::app::State;
use crate::jobs::Job;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{ActionKind, BODY, BODY_MED, H1, H2, MONO, SMALL, Ui};

#[derive(Default)]
pub struct Checks {
    pub open: bool,
    pub job: Job<AddonsAnalysis>,
    pub analysis: Option<Result<AddonsAnalysis, String>>,
    pub hud_job: Job<Vec<HudCheck>>,
    pub hud: Option<Result<Vec<HudCheck>, String>>,
    /// "Remove N leftover files" was pressed once and asks before moving them.
    pub confirm_remove: bool,
}

fn library() -> Vec<(u16, String)> {
    deadlock_core::import::read_library(&super::library_file())
        .map(|library| library.mods.into_iter().map(|m| (m.slot, m.name)).collect())
        .unwrap_or_default()
}

/// Open the dialog and read the folder on a worker.
pub fn open(st: &mut State) {
    st.installed.menu = None;
    st.installed.checks.open = true;
    start(st);
}

fn start(st: &mut State) {
    if st.demo || rune::is_headless() || st.installed.checks.job.is_running() {
        return;
    }
    let dir = super::addons_folder(st);
    st.installed.checks.analysis = None;
    st.installed.checks.confirm_remove = false;
    st.installed.checks.job.start(st.wake.clone(), move || {
        addon_checks::analyze_addons(&dir, &library()).map_err(|e| e.to_string())
    });
}

fn start_hud(st: &mut State) {
    if st.demo || rune::is_headless() || st.installed.checks.hud_job.is_running() {
        return;
    }
    let Some(game_root) = st.game_root.clone() else { return };
    let dir = super::addons_folder(st);
    let enabled: Vec<(u16, PathBuf)> = st.addons.addons.iter().filter(|a| a.enabled).map(|a| (a.slot, a.path(&dir))).collect();
    let citadel = game_root.join("game").join("citadel");
    let scratch = crate::data::data_dir().join("hud-check");
    st.installed.checks.hud = None;
    st.installed.checks.hud_job.start(st.wake.clone(), move || {
        addon_checks::hud_check(&enabled, &citadel.join("pak01_dir.vpk"), &citadel.join("console.log"), &scratch)
            .map_err(|e| e.to_string())
    });
}

/// Take finished reads. Called from Installed's poll.
pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.installed.checks.job.try_take() {
        st.installed.checks.analysis = Some(result);
        changed = true;
    }
    if let Some(result) = st.installed.checks.hud_job.try_take() {
        st.installed.checks.hud = Some(result);
        changed = true;
    }
    changed
}

/// Before a modded launch: one warning when an enabled mod is missing files (DMM's check). The
/// launch goes on; the toast opens the analysis.
pub fn missing_before_launch(st: &mut State) {
    if st.demo || rune::is_headless() || st.game_root.is_none() {
        return;
    }
    let dir = super::addons_folder(st);
    let Ok(missing) = addon_checks::missing_files(&dir, &library()) else { return };
    let enabled: Vec<u16> = st.addons.addons.iter().filter(|a| a.enabled).map(|a| a.slot).collect();
    let broken: Vec<&MissingFile> = missing
        .iter()
        .filter(|m| matches!(m, MissingFile::SlotGone { .. }) || enabled.contains(&m.slot()))
        .collect();
    if broken.is_empty() {
        return;
    }
    let names: Vec<String> = broken.iter().map(|m| describe(st, m)).collect();
    let title = if broken.len() == 1 {
        tr("A mod is missing files").to_string()
    } else {
        fill(tr("{} mods are missing files"), &[&broken.len().to_string()])
    };
    crate::notify::push(
        st,
        crate::notify::Notice::warning(title)
            .detail(names.join("; "))
            .action(tr("Review"), |st: &mut State| {
                st.go(crate::model::Screen::Installed);
                open(st);
            }),
    );
}

fn describe(s: &State, missing: &MissingFile) -> String {
    match missing {
        MissingFile::SlotGone { slot, name } => {
            fill(tr("{} (slot {}): its file is gone"), &[name, &deadlock_core::addons::slot_label(*slot)])
        }
        MissingFile::ChunksGone { slot, chunks, .. } => {
            let name = s
                .addons
                .addons
                .iter()
                .find(|a| a.slot == *slot)
                .map(|a| super::display_name(s, a))
                .unwrap_or_else(|| deadlock_core::addons::slot_label(*slot));
            fill(tr("{}: {} chunk files are gone"), &[&name, &chunks.len().to_string()])
        }
    }
}

/// GAME WRITE, from the Make writable press: clear the read-only flag on the listed slot files
/// (`game_apply::make_writable`), then read the folder again.
fn make_writable(st: &mut State, paths: Vec<PathBuf>) {
    use crate::notify::{self, Notice};
    if st.demo {
        st.status = tr("Demo library: nothing was changed.").to_string();
        return;
    }
    let dir = super::addons_folder(st);
    match deadlock_core::game_apply::make_writable(&dir, &paths, true) {
        Ok(n) => {
            notify::push(st, Notice::success(fill(tr("Made {} files writable"), &[&n.to_string()])));
            crate::data::start_addon_scan(st);
            start(st);
        }
        Err(error) => notify::push(st, Notice::error(tr("Could not make the files writable")).detail(error.to_string())),
    }
}

fn close(st: &mut State) {
    st.installed.checks.open = false;
}

/// Move every leftover Analyze listed to Sigil's trash, as one undoable change. Each addon
/// folder gets its own entry, so a failure in one never stops another.
fn remove_leftovers(st: &mut State) {
    st.installed.checks.confirm_remove = false;
    if st.demo {
        st.status = tr("Demo library: nothing was changed.").to_string();
        return;
    }
    if crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    let groups: std::collections::BTreeMap<PathBuf, Vec<String>> = match &st.installed.checks.analysis {
        Some(Ok(analysis)) => {
            let mut groups: std::collections::BTreeMap<PathBuf, Vec<String>> = std::collections::BTreeMap::new();
            for leftover in &analysis.leftovers {
                groups.entry(leftover.folder.clone()).or_default().push(leftover.file.clone());
            }
            groups
        }
        _ => return,
    };
    let trash_root = super::trash_dir();
    let mut entries = Vec::new();
    let mut errors = Vec::new();
    let mut failed = 0usize;
    for (folder, names) in groups {
        match deadlock_core::trash::trash_loose(&folder, &trash_root, &names, true) {
            Ok(entry) => entries.push(entry),
            Err(error) => {
                failed += names.len();
                errors.push(error.to_string());
            }
        }
    }
    if entries.is_empty() {
        if !errors.is_empty() {
            crate::notify::push(
                st,
                crate::notify::Notice::error(tr("The leftover files could not be removed")).detail(errors.join(" ")),
            );
        }
        return;
    }
    let files: usize = entries.iter().map(|entry| entry.manifest.files.len()).sum();
    st.installed.last_undo = Some(super::Undo::Delete(entries.iter().map(|entry| entry.dir.clone()).collect()));
    st.installed.undo_fresh = true;
    st.status = fill(tr("Removed {} leftover files."), &[&files.to_string()]);
    if !errors.is_empty() {
        crate::notify::push(
            st,
            crate::notify::Notice::warning(fill(
                tr("{} leftover files could not be removed"),
                &[&failed.to_string()],
            ))
            .detail(errors.join(" ")),
        );
    }
    crate::data::start_addon_scan(st);
    start(st);
}

fn heading(u: &Ui, words: &str) -> El {
    text(words.to_string()).font(H2).color(u.ink).pt(6.0)
}

fn line(u: &Ui, words: String) -> El {
    text(words).font(SMALL).color(u.muted).wrap()
}

pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    let c = &s.installed.checks;
    if !c.open {
        return None;
    }
    let mut body = col().w_full().gap(8.0);
    match &c.analysis {
        None => body = body.child(line(u, tr("Reading the addon folders").to_string())),
        Some(Err(error)) => body = body.child(crate::notify::feedback::error_box(u, "checks-error", error)),
        Some(Ok(a)) => {
            body = body.child(
                text(fill(
                    tr("{} mods, {} in all."),
                    &[&a.total_mods().to_string(), &crate::model::format_size(a.total_bytes())],
                ))
                .font(BODY_MED)
                .color(u.ink),
            );
            if a.is_clean() {
                body = body.child(line(u, tr("Nothing to fix: every slot is known, nothing is doubled, read-only or missing.").to_string()));
            }
            for f in &a.folders {
                let name = f.folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let mut words = fill(
                    tr("{}: {} on, {} off, {} chunk files, {}."),
                    &[&name, &f.enabled.to_string(), &f.disabled.to_string(), &f.chunks.to_string(), &crate::model::format_size(f.bytes)],
                );
                if !f.orphan_chunks.is_empty() {
                    words.push(' ');
                    words.push_str(&fill(tr("{} leftover chunk files no mod uses."), &[&f.orphan_chunks.len().to_string()]));
                }
                if !f.ignored.is_empty() {
                    words.push(' ');
                    words.push_str(&fill(tr("The game ignores: {}."), &[&f.ignored.join(", ")]));
                }
                if !f.temp_files.is_empty() {
                    words.push(' ');
                    words.push_str(&fill(tr("Left by a stopped reorder: {}."), &[&f.temp_files.join(", ")]));
                }
                body = body.child(line(u, words));
            }
            if !a.leftovers.is_empty() {
                let count = a.leftovers.len();
                let remove_label = if count == 1 {
                    tr("Remove 1 leftover file").to_string()
                } else {
                    fill(tr("Remove {} leftover files"), &[&count.to_string()])
                };
                body = body.child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .center_y()
                        .child(heading(u, tr("Leftover files")))
                        .child(spacer())
                        .child(
                            u.action(&remove_label, ActionKind::Secondary)
                                .key("checks-remove-leftovers")
                                .cursor(rune::Cursor::Pointer)
                                .on_press(|st: &mut State| st.installed.checks.confirm_remove = true),
                        ),
                );
                body = body.child(line(u, tr("No mod uses these. They go to the trash, and you can undo this.").to_string()));
                for leftover in a.leftovers.iter().take(8) {
                    let folder = leftover
                        .folder
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    body = body.child(line(u, format!("{}  ·  {}", leftover.file, folder)));
                }
                if count > 8 {
                    body = body.child(line(u, fill(tr("and {} more"), &[&(count - 8).to_string()])));
                }
                if c.confirm_remove {
                    body = body.child(
                        row()
                            .key("checks-remove-ask")
                            .w_full()
                            .padding(10.0)
                            .gap(8.0)
                            .center_y()
                            .radius(crate::ui::r(8.0))
                            .border(1.0, u.amber.alpha(0.45))
                            .child(
                                text(tr("Remove these files? They go to the trash, and you can put them back."))
                                    .font(SMALL)
                                    .color(u.prose)
                                    .wrap()
                                    .grow(1.0)
                                    .min_w(0.0),
                            )
                            .child(
                                u.action(tr("Cancel"), ActionKind::Ghost)
                                    .key("checks-remove-cancel")
                                    .cursor(rune::Cursor::Pointer)
                                    .on_press(|st: &mut State| st.installed.checks.confirm_remove = false),
                            )
                            .child(
                                u.action(tr("Remove"), ActionKind::Primary)
                                    .key("checks-remove-yes")
                                    .cursor(rune::Cursor::Pointer)
                                    .on_press(remove_leftovers),
                            ),
                    );
                }
            }
            if !a.unknown_slots.is_empty() {
                body = body.child(heading(u, tr("Not in your library")));
                for slot in &a.unknown_slots {
                    let slot = *slot;
                    body = body.child(
                        row()
                            .w_full()
                            .gap(8.0)
                            .center_y()
                            .child(text(deadlock_core::addons::slot_label(slot)).font(MONO).color(u.muted).w(40.0))
                            .child(spacer())
                            .child(
                                u.action(tr("Find which mod this is"), ActionKind::Ghost)
                                    .key(format!("checks-identify-{slot}"))
                                    .cursor(rune::Cursor::Pointer)
                                    .on_press(move |st: &mut State| {
                                        close(st);
                                        super::tools::open_identify(st, slot);
                                    }),
                            ),
                    );
                }
            }
            if !a.doubled_slots.is_empty() {
                body = body.child(heading(u, tr("Doubled slots")));
                let labels: Vec<String> = a.doubled_slots.iter().map(|s| deadlock_core::addons::slot_label(*s)).collect();
                body = body.child(line(u, fill(tr("Two files claim slot {}: the game reads one of them."), &[&labels.join(", ")])));
            }
            if !a.read_only.is_empty() {
                let paths = a.read_only.clone();
                body = body.child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .center_y()
                        .child(heading(u, tr("Read-only files")))
                        .child(spacer())
                        .child(
                            u.action(tr("Make writable"), ActionKind::Secondary)
                                .key("checks-make-writable")
                                .cursor(rune::Cursor::Pointer)
                                .on_press(move |st: &mut State| make_writable(st, paths.clone())),
                        ),
                );
                body = body.child(line(u, tr("Sigil cannot switch, move or update these while they are read-only. Make writable clears the flag on these files only.").to_string()));
                for path in &a.read_only {
                    let p = path.clone();
                    body = body.child(
                        row()
                            .w_full()
                            .gap(8.0)
                            .center_y()
                            .child(text(path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).font(MONO).color(u.prose).grow(1.0).min_w(0.0).truncate())
                            .child(
                                u.action(tr("Show in folder"), ActionKind::Ghost)
                                    .cursor(rune::Cursor::Pointer)
                                    .on_press(move |st: &mut State| super::reveal(&mut st.status, p.clone())),
                            ),
                    );
                }
            }
            if !a.missing.is_empty() {
                body = body.child(heading(u, tr("Missing files")));
                for missing in &a.missing {
                    let slot = missing.slot();
                    let mut entry = row().w_full().gap(8.0).center_y().child(line(u, describe(s, missing)).grow(1.0).shrink().min_w(0.0));
                    if matches!(missing, MissingFile::ChunksGone { .. }) {
                        entry = entry.child(
                            u.action(tr("Reinstall"), ActionKind::Secondary)
                                .key(format!("checks-reinstall-{slot}"))
                                .cursor(rune::Cursor::Pointer)
                                .on_press(move |st: &mut State| {
                                    close(st);
                                    super::reinstall(st, slot);
                                }),
                        );
                    }
                    body = body.child(entry);
                }
            }
        }
    }
    // HUD check.
    body = body.child(heading(u, tr("HUD mods")));
    match &c.hud {
        None if c.hud_job.is_running() => body = body.child(line(u, tr("Comparing HUD layouts with the game's").to_string())),
        None => body = body.child(line(u, tr("Compares each HUD mod's layouts with the game's, and reads the game's console log for missing panels.").to_string())),
        Some(Err(error)) => body = body.child(crate::notify::feedback::error_box(u, "checks-hud-error", error)),
        Some(Ok(checks)) => {
            let outdated: Vec<&HudCheck> = checks.iter().filter(|c| c.is_outdated()).collect();
            if outdated.is_empty() {
                body = body.child(line(u, fill(tr("{} layouts compared. None is missing a panel."), &[&checks.len().to_string()])));
            }
            for check in outdated {
                let name = s
                    .addons
                    .addons
                    .iter()
                    .find(|a| a.slot == check.slot)
                    .map(|a| super::display_name(s, a))
                    .unwrap_or_else(|| deadlock_core::addons::slot_label(check.slot));
                body = body.child(
                    col()
                        .w_full()
                        .gap(2.0)
                        .child(text(fill(tr("{} is out of date"), &[&name])).font(BODY_MED).color(u.amber))
                        .child(line(u, fill(tr("{} lacks {}"), &[&check.layout, &check.missing_panels.join(", ")]))),
                );
            }
        }
    }
    let busy = c.job.is_running() || c.hud_job.is_running();
    Some(
        u.dialog_shell()
            .key("installed-checks")
            .role("dialog")
            .label(tr("Analyze the addons folder"))
            .w(640.0)
            .max_h(680.0)
            .padding(24.0)
            .gap(14.0)
            .modal(true, close)
            .child(text(tr("Analyze the addons folder")).font(H1).color(u.ink))
            .child(text(tr("What each addon folder holds, and what needs a look. Nothing here changes a file.")).font(BODY).color(u.muted).wrap())
            .child(u.scroll_area(col().w_full().shrink().min_h(0.0).scroll(rune::ScrollSpec { stick_bottom: false }).child(body)))
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(
                        u.action(tr("Check HUD mods"), ActionKind::Secondary)
                            .key("checks-hud")
                            .disabled(c.hud_job.is_running())
                            .cursor(rune::Cursor::Pointer)
                            .on_press(start_hud),
                    )
                    .child(
                        u.action(tr("Read again"), ActionKind::Ghost)
                            .key("checks-again")
                            .disabled(busy)
                            .cursor(rune::Cursor::Pointer)
                            .on_press(start),
                    )
                    .child(spacer())
                    .child(
                        u.action(tr("Close"), ActionKind::Primary)
                            .key("checks-close")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(close),
                    ),
            ),
    )
}

/// The scene: two folders, an unknown slot, a read-only file, a missing chunk and an outdated
/// HUD mod.
pub fn stage_scene(st: &mut State) {
    use deadlock_core::addon_checks::{FolderSummary, HudStatus, Leftover, LeftoverKind};
    let folder = |index: usize, name: &str, enabled, disabled, bytes| FolderSummary {
        index,
        folder: PathBuf::from(name),
        enabled,
        disabled,
        chunks: 3,
        orphan_chunks: Vec::new(),
        ignored: Vec::new(),
        temp_files: Vec::new(),
        bytes,
    };
    let mut first = folder(0, "addons", 6, 4, 812_000_000);
    first.orphan_chunks = vec!["pak12_000.vpk".to_string()];
    first.ignored = vec!["readme.txt".to_string()];
    st.installed.checks.open = true;
    st.installed.checks.analysis = Some(Ok(AddonsAnalysis {
        folders: vec![first, folder(1, "sigil-addons2", 2, 0, 64_000_000)],
        unknown_slots: vec![9],
        doubled_slots: Vec::new(),
        read_only: vec![PathBuf::from("pak04_dir.vpk")],
        missing: vec![MissingFile::ChunksGone { slot: 3, file: "pak03_dir.vpk".to_string(), chunks: vec!["pak03_001.vpk".to_string()] }],
        leftovers: vec![
            Leftover { folder: PathBuf::from("addons"), file: "pak12_000.vpk".to_string(), kind: LeftoverKind::OrphanChunk },
            Leftover { folder: PathBuf::from("addons"), file: "readme.txt".to_string(), kind: LeftoverKind::Ignored },
            Leftover { folder: PathBuf::from("addons"), file: "pak08_dir.vpk.tmp".to_string(), kind: LeftoverKind::TempFile },
        ],
    }));
    st.installed.checks.hud = Some(Ok(vec![HudCheck {
        slot: 2,
        layout: "panorama/layout/citadel_hud.vxml_c".to_string(),
        status: HudStatus::Compared,
        missing_panels: vec!["HudAbilityQueue".to_string(), "HudPingWheel".to_string()],
        detail: None,
        console: Vec::new(),
    }]));
}
