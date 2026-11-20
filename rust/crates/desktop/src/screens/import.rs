//! Import: bring mods in from local files, Grimoire or Deadlock Mod Manager.
//!
//! Detection and planning run on a worker and write nothing (`deadlock_core::import`). The
//! review dialog lists what was found with names and previews; pressing Import is the user's
//! confirm, and only then `import::apply_plan` copies the VPKs into free addon slots, switched
//! off, and records their names for the Installed cards.

use std::path::PathBuf;

use deadlock_core::import::{
    self, ApplyContext, DetectRoots, DetectedSource, ForeignMod, ImportAction, ImportPlan,
    ImportReport, SourceKind,
};
use rune::{Color, El, FileDialogOptions, ScrollSpec, SurfaceFit, Tx, col, el, icon, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::jobs::Job;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, SMALL, Ui};

#[derive(Default)]
pub struct Flow {
    /// What detection found. `None` until it ran once.
    pub sources: Option<Vec<DetectedSource>>,
    pub detect_job: Job<Vec<DetectedSource>>,
    pub dialog: Option<Dialog>,
    pub plan_job: Job<ImportPlan>,
    pub apply_job: Job<ImportReport>,
}

pub struct Dialog {
    pub kind: SourceKind,
    pub stage: Stage,
    pub plan: Option<ImportPlan>,
    /// One per plan item: whether it is part of the import.
    pub selected: Vec<bool>,
    pub report: Option<ImportReport>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Planning,
    Review,
    Applying,
    Done,
}

impl Dialog {
    fn new(kind: SourceKind) -> Self {
        Dialog {
            kind,
            stage: Stage::Planning,
            plan: None,
            selected: Vec::new(),
            report: None,
            error: None,
        }
    }

    fn chosen(&self) -> usize {
        self.plan
            .as_ref()
            .map(|plan| {
                plan.items
                    .iter()
                    .zip(&self.selected)
                    .filter(|(item, on)| **on && !matches!(item.action, ImportAction::Skip { .. }))
                    .count()
            })
            .unwrap_or(0)
    }
}

// --- detection ---------------------------------------------------------------

/// Look for other managers on a worker, once. Reads their data files and the game folder.
pub fn start_detect(st: &mut State) {
    if st.installed.import.sources.is_some()
        || st.installed.import.detect_job.is_running()
        || st.demo
        || rune::is_headless()
    {
        return;
    }
    let game_root = st.game_root.clone();
    let wake = st.wake.clone();
    st.installed.import.detect_job.start(wake, move || {
        Ok(import::detect_sources(
            &DetectRoots::from_environment(),
            game_root.as_deref(),
        ))
    });
}

pub fn source(st: &State, kind: SourceKind) -> Option<&DetectedSource> {
    st.installed
        .import
        .sources
        .as_ref()?
        .iter()
        .find(|source| source.kind == kind)
}

/// The detail on a manager's menu row: how many mods, or that it was not found.
pub fn source_detail(st: &State, kind: SourceKind) -> (String, bool) {
    if st.installed.import.sources.is_none() {
        return ("Looking".to_string(), false);
    }
    match source(st, kind) {
        Some(found) if found.mods.is_empty() => ("No mods".to_string(), false),
        Some(found) => (
            format!(
                "{} mod{}",
                found.mods.len(),
                if found.mods.len() == 1 { "" } else { "s" }
            ),
            true,
        ),
        None => ("Not found".to_string(), false),
    }
}

// --- planning ----------------------------------------------------------------

/// Open the review for a detected manager and plan it on a worker.
pub fn open_source(st: &mut State, kind: SourceKind) {
    st.installed.menu = None;
    let Some(mods) = source(st, kind).map(|found| found.mods.clone()) else {
        return;
    };
    start_plan(st, kind, mods);
}

/// Ask for a file, then review it. `.vpk` files are copied; archives go through the install
/// pipeline when the user confirms.
pub fn pick_local(st: &mut State, tx: &mut Tx) {
    st.installed.menu = None;
    let options = FileDialogOptions {
        title: "Import local mods".to_string(),
        ..FileDialogOptions::default()
    }
    .filter("Mods", &["vpk", "zip", "7z", "rar"]);
    tx.open_file(options, |st: &mut State, result, _tx| match result {
        Ok(Some(path)) => open_local(st, vec![path]),
        Ok(None) => {}
        Err(error) => st.status = format!("The file picker could not open: {error}"),
    });
}

pub fn open_local(st: &mut State, paths: Vec<PathBuf>) {
    let mods = import::local_mods(&paths);
    start_plan(st, SourceKind::Local, mods);
}

fn start_plan(st: &mut State, kind: SourceKind, mods: Vec<ForeignMod>) {
    let Some(game_root) = st.game_root.clone() else {
        st.status = "Set the game folder in Settings first.".to_string();
        return;
    };
    let mut dialog = Dialog::new(kind);
    if mods.is_empty() {
        dialog.stage = Stage::Done;
        dialog.error = Some("Nothing to import.".to_string());
        st.installed.import.dialog = Some(dialog);
        return;
    }
    st.installed.import.dialog = Some(dialog);
    let wake = st.wake.clone();
    let addons = deadlock_core::deploy::addons_dir(&game_root);
    st.installed.import.plan_job.start(wake, move || {
        import::plan_import(kind, &addons, mods).map_err(|error| error.to_string())
    });
}

fn set_plan(st: &mut State, plan: ImportPlan) {
    let Some(dialog) = st.installed.import.dialog.as_mut() else {
        return;
    };
    dialog.selected = plan
        .items
        .iter()
        .map(|item| !matches!(item.action, ImportAction::Skip { .. }))
        .collect();
    dialog.plan = Some(plan);
    dialog.stage = Stage::Review;
}

// --- apply -------------------------------------------------------------------

/// The user's confirm: copy the selected mods into the addon folder on a worker.
fn confirm(st: &mut State) {
    let Some(dialog) = st.installed.import.dialog.as_mut() else {
        return;
    };
    let Some(plan) = dialog.plan.as_ref() else {
        return;
    };
    let mut chosen = plan.clone();
    chosen.items = plan
        .items
        .iter()
        .zip(&dialog.selected)
        .filter(|(_, on)| **on)
        .map(|(item, _)| item.clone())
        .collect();
    dialog.stage = Stage::Applying;

    if st.demo || rune::is_headless() {
        let mut report = ImportReport::default();
        for item in &chosen.items {
            match item.action {
                ImportAction::Copy { slot } | ImportAction::Archive { slot } => {
                    report.imported.push((slot, item.source.name.clone()))
                }
                ImportAction::Adopt { slot } => report.adopted.push((slot, item.source.name.clone())),
                ImportAction::Skip { .. } => {}
            }
        }
        finish(st, Ok(report));
        return;
    }

    let Some(game_root) = st.game_root.clone() else {
        finish(st, Err("No Deadlock install was found. Nothing was copied.".to_string()));
        return;
    };
    let context = ApplyContext {
        game_root,
        workspace: crate::data::workspace_dir(),
        library: import::library_path(&crate::data::data_dir()),
    };
    let wake = st.wake.clone();
    st.installed.import.apply_job.start(wake, move || {
        import::apply_plan(&chosen, &context).map_err(|error| error.to_string())
    });
}

fn finish(st: &mut State, result: Result<ImportReport, String>) {
    if let Some(dialog) = st.installed.import.dialog.as_mut() {
        dialog.stage = Stage::Done;
        match result {
            Ok(report) => dialog.report = Some(report),
            Err(error) => dialog.error = Some(error),
        }
    }
    // The other managers' lists may now overlap with what is installed.
    st.installed.import.sources = None;
    st.installed.library_sig = None;
    if !st.demo && !rune::is_headless() {
        crate::data::start_addon_scan(st);
    }
}

fn close(st: &mut State) {
    if matches!(
        st.installed.import.dialog.as_ref().map(|d| d.stage),
        Some(Stage::Applying)
    ) {
        return;
    }
    st.installed.import.dialog = None;
}

pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.installed.import.detect_job.try_take() {
        st.installed.import.sources = Some(result.unwrap_or_default());
        changed = true;
    }
    if let Some(result) = st.installed.import.plan_job.try_take() {
        match result {
            Ok(plan) => set_plan(st, plan),
            Err(error) => {
                if let Some(dialog) = st.installed.import.dialog.as_mut() {
                    dialog.stage = Stage::Done;
                    dialog.error = Some(error);
                }
            }
        }
        changed = true;
    }
    if let Some(result) = st.installed.import.apply_job.try_take() {
        finish(st, result);
        changed = true;
    }
    changed
}

/// Thumbnail URLs the open review wants.
pub fn wanted_thumbs(st: &State) -> Vec<String> {
    st.installed
        .import
        .dialog
        .as_ref()
        .and_then(|dialog| dialog.plan.as_ref())
        .map(|plan| {
            plan.items
                .iter()
                .filter_map(|item| item.source.thumbnail_url.clone())
                .collect()
        })
        .unwrap_or_default()
}

// --- dialog ------------------------------------------------------------------

pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    let d = s.installed.import.dialog.as_ref()?;
    let title = match d.kind {
        SourceKind::Local => "Import local mods".to_string(),
        kind => format!("Import from {}", kind.label()),
    };
    let plan = d.plan.as_ref();
    let line = match (d.stage, plan, &d.report, &d.error) {
        (_, _, _, Some(_)) if d.report.is_none() => String::new(),
        (Stage::Planning, ..) => "Reading the mods and checking each one.".to_string(),
        (Stage::Done, _, Some(report), _) => done_line(report),
        (_, Some(plan), ..) => review_line(plan),
        _ => String::new(),
    };

    let mut head = col()
        .w_full()
        .gap(6.0)
        .child(text(title.clone()).font(H1).color(u.ink));
    if !line.is_empty() {
        head = head.child(text(line).font(BODY).color(u.muted).wrap());
    }
    if let Some(error) = &d.error {
        head = head.child(crate::notify::feedback::error_line(u, "import-error", error));
    }
    let mut body = col().w_full().gap(16.0).child(head);

    match (d.stage, plan) {
        (Stage::Planning, _) => body = body.child(loading_rows(u)),
        (Stage::Review | Stage::Applying, Some(plan)) => body = body.child(item_list(s, u, d, plan)),
        (Stage::Done, _) => {
            if let Some(report) = &d.report
                && !report.failed.is_empty()
            {
                let mut failed = col().w_full().gap(4.0);
                for (name, reason) in report.failed.iter().take(6) {
                    failed = failed.child(
                        text(format!("{name}: {reason}"))
                            .font(SMALL)
                            .color(u.amber)
                            .wrap(),
                    );
                }
                body = body.child(failed);
            }
        }
        _ => {}
    }

    let mut buttons = row().w_full().gap(8.0).center_y().child(spacer());
    match d.stage {
        Stage::Done => {
            buttons = buttons.child(
                u.action("Done", ActionKind::Primary)
                    .key("import-done")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| st.installed.import.dialog = None),
            );
        }
        stage => {
            let count = d.chosen();
            buttons = buttons.child(
                u.action("Cancel", ActionKind::Ghost)
                    .key("import-cancel")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(close),
            );
            let label = if stage == Stage::Applying {
                "Importing".to_string()
            } else if count == 0 {
                "Import".to_string()
            } else {
                format!("Import {count} mod{}", if count == 1 { "" } else { "s" })
            };
            let mut primary = u.action(&label, ActionKind::Primary).key("import-confirm");
            if stage == Stage::Review && count > 0 {
                primary = primary
                    .cursor(rune::Cursor::Pointer)
                    .on_press(confirm);
            } else {
                primary = primary.opacity(0.45).disabled(true);
            }
            buttons = buttons.child(primary);
        }
    }

    Some(
        u.dialog_shell()
            .key("import-dialog")
            .role("dialog")
            .label(title)
            .w(560.0)
            .padding(24.0)
            .gap(20.0)
            .modal(true, close)
            .child(body)
            .child(buttons),
    )
}

fn review_line(plan: &ImportPlan) -> String {
    let found = plan.items.len();
    let copies = plan.copies();
    let mut line = format!("Found {found} mod{}.", if found == 1 { "" } else { "s" });
    if copies > 0 {
        line.push_str(" They arrive switched off in free slots.");
    } else {
        line.push_str(" Nothing new to copy.");
    }
    line
}

fn done_line(report: &ImportReport) -> String {
    let imported = report.imported.len();
    let adopted = report.adopted.len();
    let mut parts = Vec::new();
    if imported > 0 {
        parts.push(format!(
            "Imported {imported} mod{}. Turn {} on in Installed.",
            if imported == 1 { "" } else { "s" },
            if imported == 1 { "it" } else { "them" }
        ));
    }
    if adopted > 0 {
        parts.push(format!(
            "Named {adopted} mod{} you already had.",
            if adopted == 1 { "" } else { "s" }
        ));
    }
    if parts.is_empty() {
        "Nothing was imported.".to_string()
    } else {
        parts.join(" ")
    }
}

fn item_list(s: &State, u: &Ui, d: &Dialog, plan: &ImportPlan) -> El {
    let mut list = col()
        .w_full()
        .border_sides([1.0, 0.0, 0.0, 0.0], u.hair);
    for (index, item) in plan.items.iter().enumerate() {
        let on = d.selected.get(index).copied().unwrap_or(false);
        let skip = matches!(item.action, ImportAction::Skip { .. });
        let (detail, detail_color) = match &item.action {
            ImportAction::Copy { slot } | ImportAction::Archive { slot } => {
                let mut parts = Vec::new();
                if let Some(author) = &item.source.author {
                    parts.push(format!("by {author}"));
                }
                parts.push(format!("slot {slot:02}"));
                (parts.join("  ·  "), u.faint)
            }
            ImportAction::Adopt { slot } => (format!("Already installed in slot {slot:02}"), u.faint),
            ImportAction::Skip { reason } => (reason.clone(), u.amber),
        };
        let mut r = row()
            .key(format!("import-item-{index}"))
            .w_full()
            .h(60.0)
            .gap(12.0)
            .center_y()
            .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
            .child(checkbox(u, on, skip))
            .child(preview(s, u, item.source.thumbnail_url.as_deref(), 64.0, 36.0, 6.0))
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(2.0)
                    .child(
                        text(item.source.name.clone())
                            .font(BODY_MED)
                            .color(if skip { u.muted } else { u.ink })
                            .truncate(),
                    )
                    .child(text(detail).font(SMALL).color(detail_color).truncate()),
            );
        if !skip && d.stage == Stage::Review {
            r = r
                .focusable(true)
                .role("checkbox")
                .checked(on)
                .label(item.source.name.clone())
                .hover(u.hover)
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    if let Some(dialog) = st.installed.import.dialog.as_mut()
                        && let Some(value) = dialog.selected.get_mut(index)
                    {
                        *value = !*value;
                    }
                });
        }
        list = list.child(r);
    }
    col()
        .key("import-list")
        .w_full()
        .max_h(360.0)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(list)
}

fn checkbox(u: &Ui, on: bool, disabled: bool) -> El {
    let base = el().size(16.0, 16.0).radius(4.0).center().a11y_hidden();
    if disabled {
        base.border(1.0, u.hair)
    } else if on {
        base.fill(u.accent)
            .child(icon(&icons::CHECK, 12.0).color(u.accent_ink).a11y_hidden())
    } else {
        base.border(1.0, Color::WHITE.alpha(0.24))
    }
}

/// A mod preview: the cached image, or a calm placeholder with a package glyph.
pub fn preview(s: &State, u: &Ui, url: Option<&str>, w: f32, h: f32, radius: f32) -> El {
    let pixels = url.and_then(|url| s.thumbnails.get(url));
    let frame = el().size(w, h).radius(radius).a11y_hidden();
    match pixels {
        Some(pixels) => frame.pixels_fit(pixels, SurfaceFit::Fill),
        None => frame
            .fill(Color::WHITE.alpha(0.04))
            .center()
            .child(icon(&icons::PACKAGE, 14.0).color(u.faint.alpha(0.6))),
    }
}

fn loading_rows(u: &Ui) -> El {
    let mut list = col()
        .w_full()
        .border_sides([1.0, 0.0, 0.0, 0.0], u.hair);
    for n in 0..4 {
        list = list.child(
            row()
                .w_full()
                .h(60.0)
                .gap(12.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(el().size(16.0, 16.0).radius(4.0).fill(u.hover))
                .child(el().size(64.0, 36.0).radius(crate::ui::r(6.0)).fill(u.hover))
                .child(
                    el().h(10.0)
                        .w(140.0 + (n % 3) as f32 * 48.0)
                        .radius(3.0)
                        .fill(u.hover),
                ),
        );
    }
    list
}

// --- fixtures ----------------------------------------------------------------

/// A review dialog for the `installed-import` scene. No file is read or written.
pub fn stage_demo_review(st: &mut State, thumbs: &[String]) {
    let names = [
        ("Asa Mitaka over Paige", Some("toast068"), ImportAction::Copy { slot: 6 }),
        ("Violet recolors Pack 4", Some("achhille"), ImportAction::Copy { slot: 8 }),
        ("Random Respawn Dances", Some("Bailey31"), ImportAction::Copy { slot: 10 }),
        ("The Rebelcapin Collection", Some("dunder_duck"), ImportAction::Adopt { slot: 3 }),
        ("Kasane Teto skin", None, ImportAction::Copy { slot: 13 }),
        (
            "old_hud_tweaks",
            None,
            ImportAction::Skip {
                reason: "Fails the payload rules: scripts/*.vdata_c is gameplay data".to_string(),
            },
        ),
    ];
    let items = names
        .into_iter()
        .enumerate()
        .map(|(n, (name, author, action))| deadlock_core::import::PlannedImport {
            source: ForeignMod {
                name: name.to_string(),
                author: author.map(str::to_string),
                thumbnail_url: thumbs.get(n).cloned(),
                ..ForeignMod::default()
            },
            action,
        })
        .collect();
    let plan = ImportPlan {
        kind: SourceKind::Grimoire,
        addons_dir: PathBuf::new(),
        items,
    };
    st.installed.import.dialog = Some(Dialog::new(SourceKind::Grimoire));
    set_plan(st, plan);
}

/// Detection results for the empty-state scenes.
pub fn stage_demo_sources(st: &mut State) {
    st.installed.import.sources = Some(vec![DetectedSource {
        kind: SourceKind::Grimoire,
        root: PathBuf::new(),
        mods: vec![ForeignMod::default(); 12],
        warnings: Vec::new(),
    }]);
}
