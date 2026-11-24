//! The Game index's Files tab: any game VPK, installed addon or project folder as a tree, with
//! previews (text, decoded KV3 data, Panorama, text DMX, textures), content search with pause
//! and stop, a comparison against a second source, export into a mod project and a plain text
//! editor for a project's own files.
//!
//! - Deadlock Mod Manager (`feature/modding-workbench`, behaviour only): a VPK browser with
//!   previews and export, content search with progress, Source 2 data previews and a comparison
//!   workspace.
//! - Sigil: every read runs on a worker through `deadlock_core::explorer`. The writes are export
//!   and Save, both into a mod project; nothing here touches the game install.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use deadlock_core::explorer::{
    self, Listing, SourceRef,
    compare::{self, Change, DiffLine, LineOp, TreeDiff},
    export::{self, ExportMode, ExportReport},
    preview::{self, Preview, PreviewKind},
    search::{SearchControl, SearchOptions, SearchReport},
};
use rune::{Color, El, Key, KeyEvent, Pixels, ScrollSpec, SurfaceFit, TextEdit, Tx, col, el, icon, row, text};

use crate::app::State;
use crate::icons;
use crate::jobs::Job;
use crate::screens::subpage;
use crate::ui::{ActionKind, BODY, BODY_MED, MONO, SMALL, Ui};

#[path = "explorer_de.rs"]
mod de;

fn tr(text: &str) -> &str {
    use crate::screens::settings::i18n::{self, Lang};
    if i18n::current() == Lang::German
        && let Some((_, de)) = de::DE.iter().find(|(en, _)| *en == text)
    {
        return de;
    }
    i18n::tr(text)
}

use crate::screens::settings::i18n::fill;

/// Rows a folder level draws before it says how many more there are.
const ROW_CAP: usize = 400;
/// Preview text drawn at once. The full decoded text stays in the export.
const DRAW_TEXT_CAP: usize = 96 * 1024;
const TREE_W: f32 = 360.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Paths,
    Files,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Browse,
    Search,
    Compare,
}

/// One file on show.
pub struct Shown {
    pub path: String,
    pub bytes: usize,
    pub preview: Preview,
}

type ShowResult = (String, usize, Preview, Option<(u32, u32, Vec<u8>)>);

pub struct Explorer {
    pub tab: Tab,
    pub mode: Mode,
    pub sources: Vec<SourceRef>,
    pub listing: Option<Arc<Listing>>,
    pub source_path: Option<PathBuf>,
    open_job: Job<Arc<Listing>>,
    pub folder: String,
    pub selected: Option<String>,
    pub shown: Option<Shown>,
    pub image: Option<Pixels>,
    show_job: Job<ShowResult>,
    pub query: TextEdit,
    pub case_sensitive: bool,
    pub control: Option<Arc<SearchControl>>,
    search_job: Job<SearchReport>,
    /// Progress and hit count the window last drew during a search.
    search_shown: (usize, usize, usize),
    pub report: Option<SearchReport>,
    pub right: Option<Arc<Listing>>,
    pub right_path: Option<PathBuf>,
    right_job: Job<(Arc<Listing>, TreeDiff)>,
    pub diff: Option<TreeDiff>,
    pub line_diff: Option<(String, Vec<DiffLine>)>,
    line_job: Job<(String, Vec<DiffLine>)>,
    export_job: Job<ExportReport>,
    /// A project file open for editing: its path and the text.
    pub editor: Option<(String, TextEdit)>,
    pub error: Option<String>,
}

impl Default for Explorer {
    fn default() -> Self {
        Explorer {
            tab: Tab::Paths,
            mode: Mode::Browse,
            sources: Vec::new(),
            listing: None,
            source_path: None,
            open_job: Job::default(),
            folder: String::new(),
            selected: None,
            shown: None,
            image: None,
            show_job: Job::default(),
            query: TextEdit::new(""),
            case_sensitive: false,
            control: None,
            search_job: Job::default(),
            search_shown: (0, 0, 0),
            report: None,
            right: None,
            right_path: None,
            right_job: Job::default(),
            diff: None,
            line_diff: None,
            line_job: Job::default(),
            export_job: Job::default(),
            editor: None,
            error: None,
        }
    }
}

// --- sources and projects --------------------------------------------------------------------

/// Mod project folders in the workspace, in the order the Projects screen lists them.
pub fn project_dirs() -> Vec<PathBuf> {
    let root = crate::data::workspace_dir();
    let mut dirs = Vec::new();
    if root.join("project.json").is_file() {
        dirs.push(root.clone());
    }
    if let Ok(read) = std::fs::read_dir(root.join("mods")) {
        let mut children: Vec<PathBuf> = read
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.join("project.json").is_file())
            .collect();
        children.sort();
        dirs.extend(children);
    }
    // The Projects screen skips a project it cannot read; stay aligned with its selection.
    dirs.retain(|dir| deadlock_core::project::read_project(dir).is_ok());
    dirs
}

/// The project exports land in: the one selected on the Projects screen.
fn export_project(st: &State) -> Option<PathBuf> {
    let dirs = project_dirs();
    dirs.get(st.selected_project).or(dirs.first()).cloned()
}

fn refresh_sources(st: &mut State) {
    let mut sources = match &st.game_root {
        Some(root) => explorer::game_sources(root),
        None => Vec::new(),
    };
    for dir in project_dirs() {
        let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        for (part, label) in [("staging", "staging"), ("sources", "sources")] {
            let path = dir.join(part);
            if path.is_dir() {
                sources.push(SourceRef { label: format!("{name} {label}"), path });
            }
        }
    }
    st.explorer.sources = sources;
}

/// Open the Files tab: list the sources and open the game's own VPK the first time.
pub fn open_tab(st: &mut State) {
    st.explorer.tab = Tab::Files;
    refresh_sources(st);
    if st.explorer.listing.is_none()
        && !st.explorer.open_job.is_running()
        && let Some(first) = st.explorer.sources.first().map(|s| s.path.clone())
    {
        open_source(st, first);
    }
}

pub fn open_source(st: &mut State, path: PathBuf) {
    let ex = &mut st.explorer;
    ex.source_path = Some(path.clone());
    ex.listing = None;
    ex.folder.clear();
    ex.selected = None;
    ex.shown = None;
    ex.image = None;
    ex.editor = None;
    ex.report = None;
    ex.right = None;
    ex.right_path = None;
    ex.right_job = Job::default();
    ex.diff = None;
    ex.line_diff = None;
    ex.error = None;
    if let Some(control) = ex.control.take() {
        control.stop();
    }
    ex.open_job = Job::default();
    ex.open_job.start(st.wake.clone(), move || {
        explorer::open(&path).map(Arc::new).map_err(|error| error.to_string())
    });
}

fn open_right(st: &mut State, path: PathBuf) {
    let Some(left) = st.explorer.listing.clone() else { return };
    let ex = &mut st.explorer;
    ex.right_path = Some(path.clone());
    ex.right = None;
    ex.diff = None;
    ex.line_diff = None;
    ex.right_job = Job::default();
    // A folder side is hashed file by file, so the comparison stays on the worker too.
    ex.right_job.start(st.wake.clone(), move || {
        let right = explorer::open(&path).map(Arc::new).map_err(|error| error.to_string())?;
        let diff = compare::compare_trees(&left, &right);
        Ok((right, diff))
    });
}

fn show_file(st: &mut State, path: String) {
    let Some(listing) = st.explorer.listing.clone() else { return };
    st.explorer.selected = Some(path.clone());
    st.explorer.editor = None;
    st.explorer.show_job = Job::default();
    st.explorer.show_job.start(st.wake.clone(), move || {
        let bytes = listing.read(&path).map_err(|error| error.to_string())?;
        let shown = preview::preview(&path, &bytes);
        let image = match (&shown.image, shown.kind) {
            (Some(thumb), _) => Some((thumb.width, thumb.height, thumb.rgba.clone())),
            (None, PreviewKind::Image) => decode_raster(&bytes),
            _ => None,
        };
        Ok((path, bytes.len(), shown, image))
    });
}

/// A PNG, JPEG or other raster file, scaled to at most the preview edge.
fn decode_raster(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let image = image::load_from_memory(bytes).ok()?;
    let edge = preview::PREVIEW_IMAGE_EDGE;
    let image = if image.width() > edge || image.height() > edge { image.thumbnail(edge, edge) } else { image };
    let rgba = image.to_rgba8();
    Some((rgba.width(), rgba.height(), rgba.into_raw()))
}

fn start_search(st: &mut State) {
    let Some(listing) = st.explorer.listing.clone() else { return };
    if let Some(control) = st.explorer.control.take() {
        control.stop();
    }
    let mut options = SearchOptions::new(st.explorer.query.text.clone());
    options.case_sensitive = st.explorer.case_sensitive;
    if !st.explorer.folder.is_empty() {
        options.path_filter = Some(format!("{}/", st.explorer.folder));
    }
    let control = Arc::new(SearchControl::default());
    st.explorer.control = Some(control.clone());
    st.explorer.report = None;
    st.explorer.error = None;
    let wake = st.wake.clone();
    st.explorer.search_job = Job::default();
    st.explorer.search_job.start(st.wake.clone(), move || {
        let ping = || {
            if let Some(wake) = &wake {
                wake();
            }
        };
        explorer::search::search(&listing, &options, &control, &ping)
    });
}

fn start_compare_file(st: &mut State, path: String) {
    let (Some(left), Some(right)) = (st.explorer.listing.clone(), st.explorer.right.clone()) else { return };
    st.explorer.line_job = Job::default();
    st.explorer.line_job.start(st.wake.clone(), move || {
        let text_of = |listing: &Listing| -> String {
            match listing.read(&path) {
                Ok(bytes) => preview::preview(&path, &bytes).text,
                Err(_) => String::new(),
            }
        };
        Ok((path.clone(), compare::diff_lines(&text_of(&left), &text_of(&right))))
    });
}

fn start_export(st: &mut State, paths: Vec<String>, mode: ExportMode) {
    let Some(listing) = st.explorer.listing.clone() else { return };
    let Some(project) = export_project(st) else {
        st.status = tr("Create a mod project first. Export writes into a project only.").to_string();
        return;
    };
    st.explorer.export_job.start(st.wake.clone(), move || {
        export::export(&listing, &paths, &project, mode, false).map_err(|error| error.to_string())
    });
}

/// Save the open editor into its project file. A project's own folder only.
fn save_editor(st: &mut State) {
    let (Some((path, edit)), Some(root)) = (st.explorer.editor.clone(), st.explorer.source_path.clone()) else {
        return;
    };
    match write_project_file(&root, &path, &edit.text) {
        Ok(()) => {
            st.status = fill(tr("Saved {}."), &[&path]);
            show_file(st, path);
        }
        Err(error) => st.status = error,
    }
}

/// Write one text file inside a project folder source. Refuses a path that leaves the folder
/// and a folder that is not in a mod project.
pub fn write_project_file(root: &Path, path: &str, text: &str) -> Result<(), String> {
    if !deadlock_core::vpk::is_safe_vpk_path(path) {
        return Err(fill(tr("{} is not a safe path."), &[path]));
    }
    if export::project_root(root).is_none() {
        return Err(tr("Only a mod project's own files can be edited.").to_string());
    }
    std::fs::write(root.join(path), text).map_err(|error| error.to_string())
}

fn editable_here(st: &State, shown: &Shown) -> bool {
    shown.preview.kind == PreviewKind::Text
        && !shown.preview.truncated
        && st.explorer.listing.as_ref().is_some_and(|l| l.kind == explorer::SourceKind::Folder)
        && st.explorer.source_path.as_deref().and_then(export::project_root).is_some()
}

pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    let ex = &mut st.explorer;
    if let Some(result) = ex.open_job.try_take() {
        match result {
            Ok(listing) => ex.listing = Some(listing),
            Err(error) => ex.error = Some(error),
        }
        changed = true;
    }
    if let Some(result) = ex.right_job.try_take() {
        match result {
            Ok((right, diff)) => {
                ex.diff = Some(diff);
                ex.right = Some(right);
            }
            Err(error) => ex.error = Some(error),
        }
        changed = true;
    }
    if let Some(result) = ex.show_job.try_take() {
        match result {
            Ok((path, bytes, shown, image)) if ex.selected.as_deref() == Some(path.as_str()) => {
                ex.image = image.map(|(w, h, rgba)| {
                    let pixels = Pixels::new();
                    pixels.set(w, h, rgba);
                    pixels
                });
                ex.shown = Some(Shown { path, bytes, preview: shown });
            }
            Ok(_) => {}
            Err(error) => ex.error = Some(error),
        }
        changed = true;
    }
    if let Some(result) = ex.search_job.try_take() {
        match result {
            Ok(report) => ex.report = Some(report),
            Err(error) => ex.error = Some(error),
        }
        ex.control = None;
        changed = true;
    }
    if let Some(control) = ex.control.as_ref().filter(|_| ex.search_job.is_running()) {
        // Live hits and progress: the worker pings the waker now and then. Commit when they
        // moved; true on every poll redrew the whole window at display rate.
        let (done, total) = control.progress();
        let now = (done, total, control.hit_count());
        if now != ex.search_shown {
            ex.search_shown = now;
            changed = true;
        }
    }
    if let Some(result) = ex.line_job.try_take() {
        if let Ok(diff) = result {
            ex.line_diff = Some(diff);
        }
        changed = true;
    }
    if let Some(result) = ex.export_job.try_take() {
        st.status = match result {
            Ok(report) => {
                let mut line = fill(tr("Exported {} file(s) into the project."), &[&report.written.len().to_string()]);
                if !report.skipped.is_empty() {
                    line.push(' ');
                    line.push_str(&fill(tr("{} left out."), &[&report.skipped.len().to_string()]));
                }
                line
            }
            Err(error) => error,
        };
        changed = true;
    }
    changed
}

// --- view ------------------------------------------------------------------------------------

/// The tab strip above the Game index body.
pub fn tabs(s: &State, u: &Ui) -> El {
    row().w_full().child(subpage::segmented(
        u,
        vec![
            subpage::segment(u, tr("Paths"), s.explorer.tab == Tab::Paths)
                .key("index-tab-paths")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| st.explorer.tab = Tab::Paths),
            subpage::segment(u, tr("Files"), s.explorer.tab == Tab::Files)
                .key("index-tab-files")
                .cursor(rune::Cursor::Pointer)
                .on_press(open_tab),
        ],
    ))
}

pub fn view(s: &State, u: &Ui) -> El {
    let ex = &s.explorer;
    let modes = subpage::segmented(
        u,
        [(Mode::Browse, tr("Browse")), (Mode::Search, tr("Search contents")), (Mode::Compare, tr("Compare"))]
            .into_iter()
            .map(|(mode, label)| {
                subpage::segment(u, label, ex.mode == mode)
                    .key(format!("explorer-mode-{mode:?}"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| st.explorer.mode = mode)
            })
            .collect(),
    );
    let mut body = col().w_full().gap(12.0).child(source_bar(s, u)).child(row().w_full().child(modes));
    if let Some(error) = &ex.error {
        body = body.child(text(error.clone()).font(SMALL).color(u.rose).wrap());
    }
    if ex.sources.is_empty() {
        return body.child(u.empty_state(
            "",
            tr("Nothing to browse yet"),
            tr("Set the game folder in Settings or create a mod project."),
            None,
        ));
    }
    let Some(listing) = &ex.listing else {
        return body.child(text(tr("Opening the archive...")).font(SMALL).color(u.faint));
    };
    let left = match ex.mode {
        Mode::Browse => tree(s, u, listing),
        Mode::Search => search_pane(s, u),
        Mode::Compare => compare_pane(s, u, listing),
    };
    let right = if ex.mode == Mode::Compare && ex.line_diff.is_some() {
        line_diff_pane(s, u)
    } else {
        preview_pane(s, u)
    };
    body.child(
        row()
            .key("explorer-split")
            .w_full()
            .gap(12.0)
            .child(left.w(TREE_W).min_w(0.0))
            .child(right.grow(1.0).min_w(0.0)),
    )
}

fn source_label(s: &State, path: &Path) -> String {
    s.explorer
        .sources
        .iter()
        .find(|src| src.path == path)
        .map(|src| src.label.clone())
        .unwrap_or_else(|| path.display().to_string())
}

fn source_bar(s: &State, u: &Ui) -> El {
    let ex = &s.explorer;
    let mut chips = row().w_full().gap(6.0).wrap();
    for (i, src) in ex.sources.iter().enumerate() {
        let on = ex.source_path.as_deref() == Some(src.path.as_path());
        let path = src.path.clone();
        chips = chips.child(
            subpage::segment(u, &src.label, on)
                .key(format!("explorer-source-{i}"))
                .border(1.0, u.hair)
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| open_source(st, path.clone())),
        );
    }
    chips = chips.child(
        u.icon_action(&icons::FOLDER_OPEN, tr("Open a VPK"), ActionKind::Ghost)
            .key("explorer-pick")
            .cursor(rune::Cursor::Pointer)
            .on_press_tx(|_st: &mut State, tx: &mut Tx| {
                let options = rune::FileDialogOptions { title: tr("Open a VPK").to_string(), ..Default::default() }
                    .filter("VPK", &["vpk"]);
                tx.open_file(options, |st: &mut State, result, _tx| {
                    if let Ok(Some(path)) = result {
                        add_picked(st, path);
                    }
                });
            }),
    );
    let mut summary = String::new();
    if let Some(listing) = &ex.listing {
        summary = fill(
            tr("{} files, {}"),
            &[&listing.files.len().to_string(), &crate::model::format_size(listing.total_bytes())],
        );
    }
    col()
        .key("explorer-sources")
        .w_full()
        .gap(6.0)
        .child(chips)
        .child(text(summary).font(SMALL).color(u.faint))
}

fn add_picked(st: &mut State, path: PathBuf) {
    if !st.explorer.sources.iter().any(|src| src.path == path) {
        let label = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        st.explorer.sources.push(SourceRef { label, path: path.clone() });
    }
    open_source(st, path);
}

fn crumbs(s: &State, u: &Ui) -> El {
    let ex = &s.explorer;
    let mut line = row().w_full().gap(4.0).center_y().wrap();
    line = line.child(
        text(tr("Root"))
            .font(SMALL)
            .color(if ex.folder.is_empty() { u.ink } else { u.accent })
            .key("explorer-crumb-root")
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| st.explorer.folder.clear()),
    );
    let mut at = String::new();
    for (i, part) in ex.folder.split('/').filter(|p| !p.is_empty()).enumerate() {
        if !at.is_empty() {
            at.push('/');
        }
        at.push_str(part);
        let target = at.clone();
        line = line.child(text("/").font(SMALL).color(u.faint)).child(
            text(part.to_string())
                .font(SMALL)
                .color(if target == ex.folder { u.ink } else { u.accent })
                .key(format!("explorer-crumb-{i}"))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| st.explorer.folder = target.clone()),
        );
    }
    line
}

fn tree(s: &State, u: &Ui, listing: &Listing) -> El {
    let ex = &s.explorer;
    let children = listing.children(&ex.folder);
    let mut list = col().key("explorer-tree").w_full();
    for (i, node) in children.iter().take(ROW_CAP).enumerate() {
        let on = ex.selected.as_deref() == Some(node.path.as_str());
        let path = node.path.clone();
        let folder = node.folder;
        let meta = if node.folder {
            fill(tr("{} files"), &[&node.files.to_string()])
        } else {
            crate::model::format_size(node.size)
        };
        list = list.child(
            row()
                .key(format!("explorer-node-{i}"))
                .w_full()
                .h(30.0)
                .px(8.0)
                .gap(8.0)
                .center_y()
                .radius(crate::ui::r(6.0))
                .fill(if on { u.selected } else { Color::TRANSPARENT })
                .hover(u.hover)
                .focusable(true)
                .role("row")
                .label(node.name.clone())
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    if folder {
                        st.explorer.folder = path.clone();
                    } else {
                        show_file(st, path.clone());
                    }
                })
                .child(icon(if node.folder { &icons::FOLDER } else { &icons::FILE }, 14.0).color(u.faint))
                .child(text(node.name.clone()).font(MONO).color(u.ink).grow(1.0).min_w(0.0).truncate())
                .child(text(meta).font(SMALL).color(u.faint)),
        );
    }
    if children.len() > ROW_CAP {
        list = list.child(
            text(fill(tr("{} more. Search to narrow it down."), &[&(children.len() - ROW_CAP).to_string()]))
                .font(SMALL)
                .color(u.faint)
                .py(6.0),
        );
    }
    let folder = ex.folder.clone();
    let actions = row()
        .gap(6.0)
        .child(
            u.action(tr("Export folder"), ActionKind::Ghost)
                .key("explorer-export-folder")
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| start_export(st, vec![folder.clone()], ExportMode::Original)),
        );
    u.panel()
        .w_full()
        .padding(8.0)
        .gap(6.0)
        .child(crumbs(s, u))
        .child(if ex.folder.is_empty() { el() } else { actions })
        .child(
            u.scroll_area(col()
                .w_full()
                .h(520.0)
                .scroll(ScrollSpec { stick_bottom: false })
                .child(list)),
        )
}

fn preview_pane(s: &State, u: &Ui) -> El {
    let ex = &s.explorer;
    let Some(shown) = &ex.shown else {
        let line = if ex.show_job.is_running() { tr("Reading the file...") } else { tr("Pick a file to preview it.") };
        return u.panel().w_full().h(560.0).center().child(text(line).font(SMALL).color(u.faint));
    };
    let path = shown.path.clone();
    let mut head = row().w_full().gap(8.0).center_y().child(
        col()
            .grow(1.0)
            .min_w(0.0)
            .child(text(shown.path.clone()).font(MONO).color(u.ink).truncate())
            .child(
                text(format!("{} · {}", shown.preview.format, crate::model::format_size(shown.bytes as u64)))
                    .font(SMALL)
                    .color(u.faint),
            ),
    );
    let copy = path.clone();
    head = head.child(
        u.icon_button(&icons::COPY, tr("Copy path"))
            .key("explorer-copy")
            .cursor(rune::Cursor::Pointer)
            .on_press_tx(move |_st: &mut State, tx: &mut Tx| tx.copy(copy.clone())),
    );
    let original = path.clone();
    head = head.child(
        u.action(tr("Export original"), ActionKind::Secondary)
            .key("explorer-export")
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| start_export(st, vec![original.clone()], ExportMode::Original)),
    );
    if export::editable_path(&path).is_some() && shown.preview.kind != PreviewKind::Text {
        let editable = path.clone();
        head = head.child(
            u.action(tr("Export editable"), ActionKind::Ghost)
                .key("explorer-export-editable")
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| start_export(st, vec![editable.clone()], ExportMode::Editable)),
        );
    }
    if editable_here(s, shown) && ex.editor.is_none() {
        let start = shown.preview.text.clone();
        let at = path.clone();
        head = head.child(
            u.icon_action(&icons::SQUARE_PEN, tr("Edit"), ActionKind::Ghost)
                .key("explorer-edit")
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| st.explorer.editor = Some((at.clone(), TextEdit::new(&start)))),
        );
    }

    let mut body = col().w_full().gap(8.0);
    if let Some(pixels) = &ex.image {
        body = body.child(
            el().w_full().h(320.0).radius(crate::ui::r(6.0)).fill(u.well).child(
                el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).pixels_fit(pixels, SurfaceFit::Fit),
            ),
        );
    }
    if let Some(note) = &shown.preview.note {
        body = body.child(text(note.clone()).font(SMALL).color(u.muted).wrap());
    }
    if let Some((_, edit)) = &ex.editor {
        let editor = rune::text_input(edit, MONO)
            .key("explorer-editor")
            .label(tr("File text"))
            .color(u.ink)
            .caret(u.accent, u.accent.alpha(0.22))
            .on_edit(|st: &mut State| &mut st.explorer.editor.as_mut().expect("editor open").1)
            .on_input_key(|st: &mut State, ev: &KeyEvent, _| {
                if ev.key == Key::Escape {
                    st.explorer.editor = None;
                    true
                } else {
                    false
                }
            })
            .w_full()
            .min_h(360.0);
        body = body
            .child(
                row()
                    .gap(8.0)
                    .child(
                        u.action(tr("Save"), ActionKind::Primary)
                            .key("explorer-save")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(save_editor),
                    )
                    .child(
                        u.action(tr("Cancel"), ActionKind::Ghost)
                            .key("explorer-cancel")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(|st: &mut State| st.explorer.editor = None),
                    ),
            )
            .child(editor);
    } else if !shown.preview.text.is_empty() {
        let mut shown_text = shown.preview.text.as_str();
        let mut cut = shown_text.len().min(DRAW_TEXT_CAP);
        while !shown_text.is_char_boundary(cut) {
            cut -= 1;
        }
        let clipped = cut < shown_text.len() || shown.preview.truncated;
        shown_text = &shown_text[..cut];
        body = body.child(
            text(shown_text.to_string()).font(MONO).color(u.prose).wrap_anywhere().selectable(true),
        );
        if clipped {
            body = body.child(
                text(tr("The preview stops here. Export editable for the whole file."))
                    .font(SMALL)
                    .color(u.faint),
            );
        }
    }
    if !shown.preview.references.is_empty() {
        body = body.child(text(tr("References")).font(BODY_MED).color(u.ink));
        for reference in shown.preview.references.iter().take(80) {
            body = body.child(text(reference.clone()).font(MONO).color(u.muted).wrap_anywhere().selectable(true));
        }
    }
    u.panel()
        .key("explorer-preview")
        .w_full()
        .padding(12.0)
        .gap(10.0)
        .child(head)
        .child(
            u.scroll_area(col()
                .w_full()
                .h(500.0)
                .scroll(ScrollSpec { stick_bottom: false })
                .child(body)),
        )
}

fn search_pane(s: &State, u: &Ui) -> El {
    let ex = &s.explorer;
    let field = rune::text_input(&ex.query, BODY)
        .key("explorer-query")
        .label(tr("Search file contents"))
        .single_line()
        .color(u.ink)
        .placeholder(tr("Text inside files"), u.faint)
        .caret(u.accent, u.accent.alpha(0.22))
        .on_edit(|st: &mut State| &mut st.explorer.query)
        .on_submit(|st: &mut State, _| start_search(st));
    let mut controls = row().w_full().gap(6.0).center_y();
    if let Some(control) = ex.control.clone() {
        let paused = control.paused();
        let pause = control.clone();
        controls = controls
            .child(
                u.action(if paused { tr("Resume") } else { tr("Pause") }, ActionKind::Secondary)
                    .key("explorer-pause")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |_st: &mut State| pause.set_paused(!pause.paused())),
            )
            .child(
                u.action(tr("Stop"), ActionKind::Ghost)
                    .key("explorer-stop")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |_st: &mut State| control.stop()),
            );
    } else {
        controls = controls.child(
            u.action(tr("Search"), ActionKind::Primary)
                .key("explorer-search")
                .disabled(ex.query.text.trim().is_empty())
                .cursor(rune::Cursor::Pointer)
                .on_press(start_search),
        );
    }
    let case = ex.case_sensitive;
    controls = controls.child(
        subpage::segment(u, tr("Match case"), case)
            .key("explorer-case")
            .border(1.0, u.hair)
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| st.explorer.case_sensitive = !st.explorer.case_sensitive),
    );

    let (hits, status) = match (&ex.control, &ex.report) {
        (Some(control), _) => {
            let (done, total) = control.progress();
            (control.hits(), fill(tr("Searching {} of {} files..."), &[&done.to_string(), &total.to_string()]))
        }
        (None, Some(report)) => {
            let mut line = fill(
                tr("{} matches in {} of {} files."),
                &[&report.hits.len().to_string(), &report.files_matched.to_string(), &report.files_searched.to_string()],
            );
            if report.stopped {
                line.push(' ');
                line.push_str(tr("Stopped."));
            } else if report.capped {
                line.push(' ');
                line.push_str(tr("Capped: narrow the search."));
            }
            (report.hits.clone(), line)
        }
        _ => (Vec::new(), if s.explorer.folder.is_empty() {
            tr("Searches decoded data too. Open a folder in Browse to search only below it.").to_string()
        } else {
            fill(tr("Searches below {}."), &[&s.explorer.folder])
        }),
    };
    let mut list = col().key("explorer-hits").w_full().gap(2.0);
    for (i, hit) in hits.iter().enumerate().take(ROW_CAP) {
        let path = hit.path.clone();
        list = list.child(
            col()
                .key(format!("explorer-hit-{i}"))
                .w_full()
                .px(8.0)
                .py(4.0)
                .radius(crate::ui::r(6.0))
                .hover(u.hover)
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| show_file(st, path.clone()))
                .child(text(format!("{}:{}", hit.path, hit.line)).font(SMALL).color(u.accent).truncate())
                .child(text(hit.text.clone()).font(MONO).color(u.prose).truncate()),
        );
    }
    u.panel()
        .w_full()
        .padding(8.0)
        .gap(8.0)
        .child(u.search_shell(field))
        .child(controls)
        .child(text(status).font(SMALL).color(u.faint).wrap())
        .child(u.scroll_area(col().w_full().h(440.0).scroll(ScrollSpec { stick_bottom: false }).child(list)))
}

fn compare_pane(s: &State, u: &Ui, listing: &Listing) -> El {
    let ex = &s.explorer;
    let mut picks = row().w_full().gap(6.0).wrap();
    for (i, src) in ex.sources.iter().enumerate() {
        if ex.source_path.as_deref() == Some(src.path.as_path()) {
            continue;
        }
        let on = ex.right_path.as_deref() == Some(src.path.as_path());
        let path = src.path.clone();
        picks = picks.child(
            subpage::segment(u, &src.label, on)
                .key(format!("explorer-right-{i}"))
                .border(1.0, u.hair)
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| open_right(st, path.clone())),
        );
    }
    let mut pane = u
        .panel()
        .w_full()
        .padding(8.0)
        .gap(8.0)
        .child(text(fill(tr("Compare {} with"), &[&listing.label])).font(BODY_MED).color(u.ink))
        .child(picks);
    let Some(diff) = &ex.diff else {
        let line = if ex.right_job.is_running() { tr("Comparing...") } else { tr("Pick the second source.") };
        return pane.child(text(line).font(SMALL).color(u.faint));
    };
    let right_label = ex.right_path.as_deref().map(|p| source_label(s, p)).unwrap_or_default();
    pane = pane.child(
        text(fill(
            tr("{} only here, {} only in {}, {} changed, {} the same."),
            &[
                &diff.only_left.to_string(),
                &diff.only_right.to_string(),
                &right_label,
                &diff.changed.to_string(),
                &diff.same.to_string(),
            ],
        ))
        .font(SMALL)
        .color(u.muted)
        .wrap(),
    );
    let mut list = col().key("explorer-diff").w_full();
    for (i, entry) in diff.rows.iter().filter(|r| r.change != Change::Same).take(ROW_CAP).enumerate() {
        let (mark, tone) = match entry.change {
            Change::OnlyLeft => ("-", u.rose),
            Change::OnlyRight => ("+", u.teal),
            _ => ("~", u.amber),
        };
        let path = entry.path.clone();
        let changed = entry.change == Change::Changed;
        let open = ex.line_diff.as_ref().is_some_and(|(p, _)| *p == entry.path);
        list = list.child(
            row()
                .key(format!("explorer-diff-{i}"))
                .w_full()
                .h(28.0)
                .px(8.0)
                .gap(8.0)
                .center_y()
                .radius(crate::ui::r(6.0))
                .fill(if open { u.selected } else { Color::TRANSPARENT })
                .hover(u.hover)
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    if changed {
                        start_compare_file(st, path.clone());
                    } else {
                        st.explorer.line_diff = None;
                        show_file(st, path.clone());
                    }
                })
                .child(text(mark).font(MONO).color(tone).w(12.0))
                .child(text(entry.path.clone()).font(MONO).color(u.ink).grow(1.0).min_w(0.0).truncate()),
        );
    }
    pane.child(u.scroll_area(col().w_full().h(440.0).scroll(ScrollSpec { stick_bottom: false }).child(list)))
}

fn line_diff_pane(s: &State, u: &Ui) -> El {
    let Some((path, lines)) = &s.explorer.line_diff else { return el() };
    let mut body = col().w_full();
    for line in lines.iter().take(4000) {
        let (mark, tone, fill_color) = match line.op {
            LineOp::Same => (" ", u.muted, Color::TRANSPARENT),
            LineOp::Removed => ("-", u.rose, u.rose.alpha(0.10)),
            LineOp::Added => ("+", u.teal, u.teal.alpha(0.10)),
        };
        body = body.child(
            row()
                .w_full()
                .gap(6.0)
                .fill(fill_color)
                .child(text(mark).font(MONO).color(tone).w(12.0))
                .child(text(line.text.clone()).font(MONO).color(u.prose).grow(1.0).min_w(0.0).wrap_anywhere()),
        );
    }
    u.panel()
        .key("explorer-line-diff")
        .w_full()
        .padding(12.0)
        .gap(8.0)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(text(path.clone()).font(MONO).color(u.ink).grow(1.0).min_w(0.0).truncate())
                .child(
                    u.icon_button(&icons::X, tr("Close"))
                        .key("explorer-line-close")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| st.explorer.line_diff = None),
                ),
        )
        .child(u.scroll_area(col().w_full().h(500.0).scroll(ScrollSpec { stick_bottom: false }).child(body)))
}

// --- scenes ----------------------------------------------------------------------------------

/// A built-in listing for the headless scenes: no game install needed.
fn scene_listing(label: &str, extra: bool) -> Arc<Listing> {
    let mut paths = vec![
        ("scripts/abilities.vdata_c", 182_000),
        ("scripts/heroes.vdata_c", 96_000),
        ("panorama/layout/hud.vxml_c", 12_400),
        ("panorama/images/heroes/haze_card_psd.vtex_c", 410_000),
        ("soundevents/hero/haze.vsndevts_c", 22_000),
        ("models/heroes_staging/haze/haze.vmdl_c", 1_200_000),
    ];
    if extra {
        paths.push(("scripts/items.vdata_c", 64_000));
    }
    Arc::new(Listing::from_paths(label, &paths))
}

pub fn scene_files(st: &mut State) {
    st.load_demo();
    st.screen = crate::model::Screen::Index;
    st.explorer.tab = Tab::Files;
    st.explorer.sources = vec![
        SourceRef { label: "Game files (citadel)".into(), path: PathBuf::from("/scene/pak01_dir.vpk") },
        SourceRef { label: "Addon pak03_dir.vpk".into(), path: PathBuf::from("/scene/pak03_dir.vpk") },
    ];
    st.explorer.source_path = Some(PathBuf::from("/scene/pak01_dir.vpk"));
    st.explorer.listing = Some(scene_listing("pak01_dir.vpk", false));
    st.explorer.folder = "scripts".into();
    st.explorer.selected = Some("scripts/heroes.vdata_c".into());
    let text = "{\n\thero_haze =\n\t{\n\t\tm_strModelName = \"models/heroes_staging/haze/haze.vmdl\"\n\t\tm_mapStartingStats =\n\t\t{\n\t\t\tEMaxHealth = 550\n\t\t\tEBaseWeaponDamageIncrease = 0.0\n\t\t}\n\t}\n}\n";
    let mut shown = preview::preview("scripts/heroes.txt", text.as_bytes());
    shown.format = "KV3 data".into();
    shown.kind = PreviewKind::Data;
    shown.references = vec!["models/heroes_staging/haze/haze.vmdl".into()];
    st.explorer.shown = Some(Shown { path: "scripts/heroes.vdata_c".into(), bytes: 96_000, preview: shown });
}

pub fn scene_files_compare(st: &mut State) {
    scene_files(st);
    st.explorer.mode = Mode::Compare;
    st.explorer.right_path = Some(PathBuf::from("/scene/pak03_dir.vpk"));
    let right = scene_listing("pak03_dir.vpk", true);
    st.explorer.diff = st.explorer.listing.as_ref().map(|left| compare::compare_trees(left, &right));
    st.explorer.right = Some(right);
    st.explorer.line_diff = Some((
        "scripts/heroes.vdata_c".into(),
        compare::diff_lines("EMaxHealth = 550\nEBaseWeaponDamageIncrease = 0.0\n", "EMaxHealth = 700\nEBaseWeaponDamageIncrease = 0.0\n"),
    ));
}

type Scene = (&'static str, fn(&mut State));

pub const SCENES: &[Scene] = &[("index-files", scene_files), ("index-files-compare", scene_files_compare)];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing_refuses_paths_that_leave_the_project_and_non_projects() {
        let base = std::env::temp_dir().join(format!("sigil-explorer-edit-{}", std::process::id()));
        let project = base.join("proj");
        std::fs::create_dir_all(project.join("sources")).unwrap();
        std::fs::write(project.join("project.json"), "{}").unwrap();
        let sources = project.join("sources");
        assert!(write_project_file(&sources, "../escape.txt", "x").is_err());
        assert!(write_project_file(&sources, "a.txt", "hello").is_ok());
        assert_eq!(std::fs::read_to_string(sources.join("a.txt")).unwrap(), "hello");
        let loose = base.join("loose");
        std::fs::create_dir_all(&loose).unwrap();
        assert!(write_project_file(&loose, "a.txt", "x").is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn german_strings_have_no_em_dash() {
        for (en, de) in de::DE {
            assert!(!en.contains('\u{2014}') && !de.contains('\u{2014}'));
        }
    }
}
