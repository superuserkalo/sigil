//! Whole-library tools: full addons backups (create, restore by replacing or merging,
//! delete), Clear all mods (to the trash, with a recovery report), resetting Sigil's mod
//! records, wiping Sigil's mod data, the unknown-file alert, copying mods to a developer
//! folder and imprinting installed mods. The writes live in `deadlock_core::mod_data`.

use std::path::{Path, PathBuf};

use deadlock_core::mod_data::{self, RecoveryReport, RestoreMode, Snapshot};
use rune::{El, FileDialogOptions, Tx, col, row, spacer, text};

use super::i18n::{fill, tr};
use super::options;
use super::{button, detail, disabled, say, section, setting_row, toggle_row, trailing};
use crate::app::State;
use crate::icons;
use crate::jobs::Job;
use crate::notify::{self, Notice};
use crate::ui::{ActionKind, BODY, MONO, Ui};

#[derive(Default)]
pub struct Tools {
    pub snapshots: Option<Vec<Snapshot>>,
    pub job: Job<String>,
    pub busy: Option<&'static str>,
    pub recovery: Option<RecoveryReport>,
    pub recovery_job: Job<RecoveryReport>,
    pub unknown: Option<Vec<String>>,
    pub warned: bool,
}

fn snapshots_root() -> PathBuf {
    mod_data::snapshots_root(&crate::data::workspace_dir())
}

fn trash_root() -> PathBuf {
    deadlock_core::trash::trash_root(&crate::data::data_dir())
}

pub fn read(st: &mut State) {
    st.settings.extra.tools.snapshots = Some(if st.demo { demo_snapshots() } else { mod_data::list_snapshots(&snapshots_root()) });
    st.settings.extra.tools.unknown = st.game_root.as_deref().map(mod_data::unknown_files);
}

pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.settings.extra.tools.job.try_take() {
        st.settings.extra.tools.busy = None;
        match result {
            Ok(line) => say(st, line, false),
            Err(error) => say(st, error, true),
        }
        read(st);
        crate::data::rescan_game(st);
        changed = true;
    }
    if let Some(result) = st.settings.extra.tools.recovery_job.try_take() {
        st.settings.extra.tools.busy = None;
        match result {
            Ok(report) => {
                super::log::info(format!("Cleared all mods: {} moved, {} failed", report.moved.len(), report.failed.len()));
                st.settings.extra.tools.recovery = Some(report);
                st.settings.extra.dialog = Some(super::extra::Dialog::Recovery);
            }
            Err(error) => say(st, error, true),
        }
        crate::data::rescan_game(st);
        changed = true;
    }
    changed
}

/// At startup: tell the user once about files in addons the game ignores.
pub fn warn_unknown(st: &mut State) {
    if st.settings.extra.tools.warned || st.demo || rune::is_headless() || options::get().unknown_alert_off {
        return;
    }
    st.settings.extra.tools.warned = true;
    let Some(root) = st.game_root.clone() else { return };
    let files = mod_data::unknown_files(&root);
    if files.is_empty() {
        return;
    }
    let line = files.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
    notify::push(
        st,
        Notice::warning(fill(tr("{} files in addons the game ignores"), &[&files.len().to_string()]))
            .detail(line)
            .action(tr("Don't warn again"), |st: &mut State| {
                options::edit(st, |o| o.unknown_alert_off = true);
            }),
    );
}

fn start(st: &mut State, busy: &'static str, work: impl FnOnce() -> Result<String, String> + Send + 'static) {
    if st.demo || rune::is_headless() {
        st.settings.extra.dialog = None;
        return say(st, tr("Demo data: nothing was changed."), false);
    }
    if st.settings.extra.tools.job.is_running() {
        return;
    }
    st.settings.extra.tools.busy = Some(busy);
    st.settings.extra.dialog = None;
    let wake = st.wake.clone();
    st.settings.extra.tools.job.start(wake, work);
}

pub fn create_backup(st: &mut State) {
    let Some(root) = st.game_root.clone() else {
        return say(st, tr("Find the game folder first."), true);
    };
    let library = deadlock_core::import::library_path(&crate::data::data_dir());
    start(st, "backup", move || {
        let snap = mod_data::create_snapshot(&root, &snapshots_root(), Some(&library)).map_err(|e| e.to_string())?;
        Ok(fill(
            tr("Backed up {} files, {}."),
            &[&snap.manifest.files.len().to_string(), &super::size_label(snap.manifest.bytes)],
        ))
    });
}

pub fn restore(st: &mut State, dir: PathBuf, mode: RestoreMode) {
    if super::watch::refuse_while_running(st) {
        return;
    }
    let Some(root) = st.game_root.clone() else {
        return say(st, tr("Find the game folder first."), true);
    };
    start(st, "restore", move || {
        let r = mod_data::restore_snapshot(&dir, &root, &trash_root(), mode, true).map_err(|e| e.to_string())?;
        Ok(fill(
            tr("Restored {} files. {} skipped, {} slots moved to Recently deleted."),
            &[&r.copied.len().to_string(), &r.skipped.len().to_string(), &r.trashed.len().to_string()],
        ))
    });
}

pub fn delete_snapshot(st: &mut State, dir: PathBuf) {
    start(st, "delete", move || {
        std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
        Ok(tr("Deleted.").to_string())
    });
}

pub fn clear_all(st: &mut State) {
    if super::watch::refuse_while_running(st) {
        return;
    }
    let Some(root) = st.game_root.clone() else {
        return say(st, tr("Find the game folder first."), true);
    };
    if st.demo || rune::is_headless() {
        st.settings.extra.dialog = None;
        return say(st, tr("Demo data: nothing was changed."), false);
    }
    st.settings.extra.tools.busy = Some("clear");
    st.settings.extra.dialog = None;
    let wake = st.wake.clone();
    st.settings.extra.tools.recovery_job.start(wake, move || {
        mod_data::clear_all_mods(&root, &trash_root(), true).map_err(|e| e.to_string())
    });
}

pub fn reset_records(st: &mut State) {
    let data = crate::data::data_dir();
    let ws = crate::data::workspace_dir();
    start(st, "records", move || {
        let r = mod_data::reset_mod_records(&data, &ws).map_err(|e| e.to_string())?;
        Ok(match r.kept_in {
            Some(dir) => fill(tr("Sigil's mod records were moved to {}."), &[&dir.display().to_string()]),
            None => tr("There were no records to reset.").to_string(),
        })
    });
}

pub fn wipe_data(st: &mut State) {
    if crate::screens::browse::quick::installs_busy(st) {
        return say(st, tr("Wait for the current download to finish."), true);
    }
    let data = crate::data::data_dir();
    let ws = crate::data::workspace_dir();
    start(st, "wipe", move || {
        let r = mod_data::wipe_mod_data(&data, &ws, &trash_root()).map_err(|e| e.to_string())?;
        Ok(fill(tr("Freed {}. Mods in the game stay."), &[&super::size_label(r.freed)]))
    });
}

// --- Backups tab --------------------------------------------------------------------------

pub fn backups_section(s: &State, u: &Ui) -> El {
    let t = &s.settings.extra.tools;
    let snaps = t.snapshots.as_deref().unwrap_or(&[]);
    let busy = t.busy == Some("backup");
    let mut rows = vec![setting_row(
        u,
        "settings-snapshot-create",
        "Back up every mod",
        detail(u, "Copies the whole addons folder and your mod names, to put back later."),
        disabled(
            button(u, "settings-snapshot-create-run", if busy { "Backing up" } else { "Create backup" }, ActionKind::Secondary)
                .on_press(create_backup),
            t.busy.is_some() || s.game_root.is_none(),
        ),
    )];
    for (index, snap) in snaps.iter().enumerate() {
        let restore_dir = snap.dir.clone();
        let reveal = snap.dir.display().to_string();
        let delete_dir = snap.dir.clone();
        rows.push(setting_row(
            u,
            &format!("settings-snapshot-{index}"),
            &snap.manifest.created_at.replace('T', " ").chars().take(16).collect::<String>(),
            detail(
                u,
                &fill(
                    tr("{} files, {}"),
                    &[&snap.manifest.files.len().to_string(), &super::size_label(snap.manifest.bytes)],
                ),
            ),
            trailing(vec![
                disabled(
                    button(u, &format!("settings-snapshot-restore-{index}"), "Restore", ActionKind::Secondary)
                        .on_press(move |st: &mut State| {
                            st.settings.extra.dialog = Some(super::extra::Dialog::RestoreSnapshot(restore_dir.clone()))
                        }),
                    t.busy.is_some(),
                ),
                u.icon_button(&icons::FOLDER, tr("Reveal"))
                    .key(format!("settings-snapshot-reveal-{index}"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| crate::data::reveal_path(st, reveal.clone())),
                u.icon_button(&icons::TRASH_2, tr("Delete this backup"))
                    .key(format!("settings-snapshot-delete-{index}"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        st.settings.extra.dialog = Some(super::extra::Dialog::DeleteSnapshot(delete_dir.clone()))
                    }),
            ]),
        ));
    }
    section(u, "Full backups", rows)
}

pub fn restore_dialog(u: &Ui, dir: &Path) -> El {
    let replace = dir.to_path_buf();
    let merge = dir.to_path_buf();
    super::extra::shell(u, "settings-snapshot-restore-dialog", tr("Restore this backup"))
        .child(
            text(tr("Replace makes the addons folder exactly as it was; current mods go to Recently deleted first. Merge only fills slots that are free."))
                .font(BODY)
                .color(u.muted)
                .wrap()
                .w_full(),
        )
        .child(
            row()
                .w_full()
                .gap(8.0)
                .pt(8.0)
                .child(spacer())
                .child(button(u, "settings-dialog-cancel", "Cancel", ActionKind::Ghost).on_press(super::extra::close))
                .child(
                    button(u, "settings-snapshot-merge", "Merge", ActionKind::Secondary)
                        .on_press(move |st: &mut State| restore(st, merge.clone(), RestoreMode::Merge)),
                )
                .child(
                    button(u, "settings-snapshot-replace", "Replace", ActionKind::Primary)
                        .on_press(move |st: &mut State| restore(st, replace.clone(), RestoreMode::Replace)),
                ),
        )
}

pub fn recovery_dialog(s: &State, u: &Ui) -> El {
    let report = s.settings.extra.tools.recovery.clone().unwrap_or_default();
    let copy = report.text();
    let mut list = col().w_full().gap(4.0);
    for (slot, dir) in report.moved.iter().take(40) {
        list = list.child(
            text(format!("pak{slot:02} · {}", dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()))
                .font(MONO)
                .color(u.prose)
                .truncate(),
        );
    }
    for (slot, error) in &report.failed {
        list = list.child(crate::notify::feedback::error_line(u, &format!("tools-failed-{slot}"), &format!("pak{slot:02} · {error}")));
    }
    super::extra::shell(u, "settings-recovery-dialog", tr("Mod recovery report"))
        .w(520.0)
        .child(
            text(fill(
                tr("Moved {} mods ({}) to Recently deleted. {} could not be moved. Restore any of them from Installed."),
                &[&report.moved.len().to_string(), &super::size_label(report.bytes), &report.failed.len().to_string()],
            ))
            .font(BODY)
            .color(u.muted)
            .wrap()
            .w_full(),
        )
        .child(col().w_full().max_h(220.0).scroll(rune::ScrollSpec { stick_bottom: false }).child(list))
        .child(
            row()
                .w_full()
                .gap(8.0)
                .pt(8.0)
                .child(spacer())
                .child(
                    button(u, "settings-recovery-copy", "Copy report", ActionKind::Secondary)
                        .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                            tx.copy(copy.clone());
                            say(st, tr("Copied."), false);
                        }),
                )
                .child(button(u, "settings-recovery-done", "Done", ActionKind::Primary).on_press(super::extra::close)),
        )
}

// --- Storage tab ---------------------------------------------------------------------------

pub fn storage_rows(s: &State, u: &Ui) -> Vec<El> {
    let t = &s.settings.extra.tools;
    let busy = t.busy.is_some() || t.recovery_job.is_running();
    let o = options::get();
    let unknown = t.unknown.as_deref().unwrap_or(&[]);
    let unknown_line = if unknown.is_empty() {
        tr("Sigil warns at startup when the addons folder has files the game ignores.").to_string()
    } else {
        fill(tr("{} files the game ignores: {}"), &[&unknown.len().to_string(), &unknown.iter().take(3).cloned().collect::<Vec<_>>().join(", ")])
    };
    let mut rows = vec![
        toggle_row(
            u,
            "settings-unknown-alert",
            "Warn about unknown files",
            &unknown_line,
            !o.unknown_alert_off,
            |st| {
                options::edit(st, |o| o.unknown_alert_off = !o.unknown_alert_off);
                st.settings.extra.tools.warned = false;
            },
        ),
        setting_row(
            u,
            "settings-clear-mods",
            "Clear all mods",
            detail(u, "Moves every mod out of the addons folder to Recently deleted, with a recovery report."),
            disabled(
                button(u, "settings-clear-mods-run", "Clear", ActionKind::Danger)
                    .on_press(|st: &mut State| st.settings.extra.dialog = Some(super::extra::Dialog::ClearAllMods)),
                busy || s.game_root.is_none(),
            ),
        ),
        setting_row(
            u,
            "settings-reset-records",
            "Reset mod records",
            detail(u, "Forgets names, previews and install history. Mods stay in the game and show as unknown."),
            disabled(
                button(u, "settings-reset-records-run", "Reset", ActionKind::Secondary)
                    .on_press(|st: &mut State| st.settings.extra.dialog = Some(super::extra::Dialog::ResetRecords)),
                busy,
            ),
        ),
        setting_row(
            u,
            "settings-wipe-data",
            "Clear all mod data",
            detail(u, "Records, the download cache, install backups and Recently deleted. Mods in the game stay."),
            disabled(
                button(u, "settings-wipe-data-run", "Clear", ActionKind::Danger)
                    .on_press(|st: &mut State| st.settings.extra.dialog = Some(super::extra::Dialog::WipeData)),
                busy,
            ),
        ),
    ];
    if t.recovery.is_some() {
        rows.push(setting_row(
            u,
            "settings-recovery",
            "Mod recovery report",
            detail(u, "From the last Clear all mods."),
            button(u, "settings-recovery-open", "Open", ActionKind::Ghost)
                .on_press(|st: &mut State| st.settings.extra.dialog = Some(super::extra::Dialog::Recovery)),
        ));
    }
    rows
}

// --- Experimental ----------------------------------------------------------------------------

pub fn experimental_section(s: &State, u: &Ui) -> El {
    let t = &s.settings.extra.tools;
    let o = options::get();
    section(
        u,
        "Tools",
        vec![
            toggle_row(
                u,
                "settings-imprint",
                "Imprint installed mods",
                "Writes the mod's name, author and GameBanana id into its VPK, so it is recognised after a copy or rename.",
                o.imprint,
                |st| options::edit(st, |o| o.imprint = !o.imprint),
            ),
            setting_row(
                u,
                "settings-imprint-now",
                "Imprint now",
                detail(u, "Every installed mod Sigil knows the name of. Skips mods already imprinted."),
                disabled(
                    button(u, "settings-imprint-run", if t.busy == Some("imprint") { "Imprinting" } else { "Imprint" }, ActionKind::Secondary)
                        .on_press(imprint_all),
                    t.busy.is_some() || s.game_root.is_none() || super::watch::changes_blocked(),
                ),
            ),
            setting_row(
                u,
                "settings-copy-dev",
                "Copy mods to a developer folder",
                detail(u, "Makes another Deadlock folder's addons match this one. Its own mods go to Recently deleted."),
                disabled(
                    button(u, "settings-copy-dev-run", if t.busy == Some("copy") { "Copying" } else { "Choose folder" }, ActionKind::Secondary)
                        .on_press_tx(choose_dev_folder),
                    t.busy.is_some() || s.game_root.is_none(),
                ),
            ),
        ],
    )
}

fn choose_dev_folder(_st: &mut State, tx: &mut Tx) {
    let options = FileDialogOptions {
        title: tr("Choose the developer Deadlock folder").into(),
        directory: super::editor::dialog_dir(),
        ..Default::default()
    };
    tx.open_folder(options, |st: &mut State, result, _tx| match result {
        Ok(Some(path)) => {
            super::editor::remember(&path);
            st.settings.extra.dialog = Some(super::extra::Dialog::CopyDev(path));
        }
        Ok(None) => {}
        Err(error) => say(st, error.to_string(), true),
    });
}

pub fn copy_dev(st: &mut State, target: PathBuf) {
    let Some(root) = st.game_root.clone() else { return };
    start(st, "copy", move || {
        let r = mod_data::copy_mods_to(&root, &target, &trash_root(), true).map_err(|e| e.to_string())?;
        Ok(fill(tr("Copied {} files, {}."), &[&r.copied.to_string(), &super::size_label(r.bytes)]))
    });
}

/// Imprint every slot whose library entry has a name.
pub fn imprint_all(st: &mut State) {
    if super::watch::refuse_while_running(st) {
        return;
    }
    let Some(root) = st.game_root.clone() else { return };
    let library = deadlock_core::import::library_path(&crate::data::data_dir());
    start(st, "imprint", move || {
        let lib = deadlock_core::import::read_library(&library).map_err(|e| e.to_string())?;
        let addons = deadlock_core::deploy::addons_dir(&root);
        let files = deadlock_core::load_order::read_slot_files(&addons).map_err(|e| e.to_string())?;
        let mut done = 0;
        for (slot, names) in files {
            let Some(dir_file) = names.iter().find(|n| n.to_ascii_lowercase().contains("_dir.vpk")) else { continue };
            let Some(meta) = lib.mods.iter().rev().find(|m| m.slot == slot) else { continue };
            let info = mod_data::Imprint {
                title: meta.name.clone(),
                author: meta.author.clone().unwrap_or_default(),
                gamebanana_id: meta.gamebanana_id,
                source_url: meta.gamebanana_id.map(|id| format!("https://gamebanana.com/mods/{id}")).unwrap_or_default(),
            };
            if mod_data::imprint_vpk(&addons.join(dir_file), &info, true).unwrap_or(false) {
                done += 1;
            }
        }
        Ok(fill(tr("Imprinted {} mods."), &[&done.to_string()]))
    });
}

/// After an install: imprint the new slot when the setting is on. Worker thread.
pub fn imprint_after_install(root: &Path, slot: u16, meta: &deadlock_core::import::ModMeta) {
    if !options::get().imprint {
        return;
    }
    let addons = deadlock_core::deploy::addons_dir(root);
    let Ok(files) = deadlock_core::load_order::read_slot_files(&addons) else { return };
    let Some(dir_file) = files.get(&slot).and_then(|n| n.iter().find(|n| n.to_ascii_lowercase().contains("_dir.vpk")).cloned()) else {
        return;
    };
    let info = mod_data::Imprint {
        title: meta.name.clone(),
        author: meta.author.clone().unwrap_or_default(),
        gamebanana_id: meta.gamebanana_id,
        source_url: meta.gamebanana_id.map(|id| format!("https://gamebanana.com/mods/{id}")).unwrap_or_default(),
    };
    let _ = mod_data::imprint_vpk(&addons.join(dir_file), &info, true);
}

fn demo_snapshots() -> Vec<Snapshot> {
    vec![Snapshot {
        dir: PathBuf::from("~/Sigil/workspace/.dah/install/snapshots/2026-11-15T19-02-11-004Z"),
        manifest: mod_data::SnapshotManifest {
            schema: 1,
            created_at: "2026-11-15T19:02:11Z".into(),
            addons_dir: PathBuf::from("~/Steam/steamapps/common/Deadlock/game/citadel/addons"),
            files: vec!["pak01_dir.vpk".into(), "pak02_dir.vpk".into(), "pak03_dir.vpk.disabled".into()],
            bytes: 48_300_000,
            library: true,
        },
    }]
}
