//! The shell around the pages: files dropped anywhere, the keyboard, zoom, the shortcuts
//! sheet, "What's new" after an update, the always-on-top switch and the pre-blurred
//! Personalize background.
//!
//! Drop targets are innermost first in Rune, so a page with its own target (Installed) keeps
//! it; every other page falls through to the root handler here. A drop only opens the import
//! review in `screens::import`; nothing is copied until the user confirms.
//!
//! "What's new" is shown once per version, from the release notes the updater already fetched
//! while offering that version (`updates/whats-new.json`). No extra network call, no notes for
//! a version that Sigil did not offer.

use std::path::{Path, PathBuf};

use rune::{
    El, Key, KeyEvent, Pixels, ScrollSpec, Tx, col, el, icon, row, spacer, text,
};
use serde_json::{Value, json};

use crate::app::State;
use crate::icons;
use crate::jobs::Job as Work;
use crate::model::Screen;
use crate::prefs::ZOOMS;
use crate::screens;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{ActionKind, BODY, BODY_MED, H1, Ui};

/// Files Sigil can import, the same list Installed accepts.
pub const MODS: [&str; 4] = ["vpk", "zip", "7z", "rar"];

/// Release notes are shown from plain text; a damaged or huge file is cut here.
pub const NOTES_CAP: usize = 16 * 1024;
/// Most lines shown in the dialog.
pub const NOTES_LINES: usize = 60;

/// The shell's own state, kept apart from the pages.
#[derive(Default)]
pub struct Shell {
    /// A file drag is over the window and no page took it.
    pub drop_hover: bool,
    /// The keyboard shortcuts sheet is open.
    pub shortcuts: bool,
    /// Release notes waiting to be read.
    pub whats_new: Option<Notes>,
    /// The window is pinned above other windows.
    pub on_top: bool,
    /// The notes file was read once this run.
    notes_loaded: bool,
    /// The version whose notes are on disk, so an offer writes them once.
    saved: Option<String>,
    /// Background blur jobs, newest first: (path and blur, pixels).
    pub blurred: Vec<(String, Pixels)>,
    pub blur_job: Work<(String, u32, u32, Vec<u8>)>,
    /// The key whose blur last failed, so a damaged file is not retried every frame.
    pub blur_failed: Option<String>,
    /// The key of the blur on the worker, to know what failed.
    pub blur_running: Option<String>,
    /// Wave 4: collapsed sidebar, New marks, hidden banners.
    pub w4: crate::shell_w4::W4,
    /// Wave 4: feature tips, the experimental tour, easter eggs and seasons.
    pub tips: crate::shell_tips::Tips,
    /// Pages visited before the current one, newest last: the mouse Back button walks them.
    pub back: Vec<Screen>,
    /// Pages Back left behind, newest last: the mouse Forward button returns to them.
    pub forward: Vec<Screen>,
    /// The mouse buttons are moving through history, so `State::go` must not record the change.
    pub(crate) navigating: bool,
}

/// Most pages remembered by the mouse buttons. Older ones drop off the front.
const NAV_CAP: usize = 64;

/// Mouse Back: close the nearest thing that was opened over the page (Settings, a mod's
/// details, a creator's page), else return to the page shown before this one.
pub fn back(st: &mut State) {
    if st.settings.open {
        st.settings.open = false;
        return;
    }
    if st.screen == Screen::Browse && st.browse_view.details.is_some() {
        crate::screens::browse::close_details(st);
        return;
    }
    if st.screen == Screen::Browse && st.browse_view.filters.creator.is_some() {
        st.browse_view.filters.creator = None;
        return;
    }
    let Some(previous) = st.shell.back.pop() else {
        if st.screen != Screen::Browse {
            go_without_history(st, Screen::Browse);
        }
        return;
    };
    if previous == st.screen {
        back(st);
        return;
    }
    st.shell.forward.push(st.screen);
    cap(&mut st.shell.forward);
    go_without_history(st, previous);
}

/// Mouse Forward: the page Back left, when nothing newer replaced it.
pub fn forward(st: &mut State) {
    let Some(next) = st.shell.forward.pop() else {
        return;
    };
    st.shell.back.push(st.screen);
    cap(&mut st.shell.back);
    go_without_history(st, next);
}

fn go_without_history(st: &mut State, screen: Screen) {
    st.shell.navigating = true;
    st.go(screen);
    st.shell.navigating = false;
}

fn cap(history: &mut Vec<Screen>) {
    if history.len() > NAV_CAP {
        history.remove(0);
    }
}

pub struct Notes {
    pub version: String,
    pub body: String,
}

// --- files dropped anywhere ---------------------------------------------------------

fn droppable(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| MODS.iter().any(|known| ext.eq_ignore_ascii_case(known)))
}

/// A drop on a page without its own target: the import review, which asks first.
pub fn dropped(st: &mut State, files: &[PathBuf]) {
    st.shell.drop_hover = false;
    let mods: Vec<PathBuf> = files.iter().filter(|path| droppable(path)).cloned().collect();
    if mods.is_empty() {
        st.status = tr("Drop a .vpk, .zip, .7z or .rar file to import it.").to_string();
        return;
    }
    // The review is the surface that matters now; Settings would cover it, and the mods
    // land on Installed afterwards.
    st.settings.open = false;
    if st.screen != Screen::Installed {
        st.go(Screen::Installed);
    }
    screens::import::open_local(st, mods);
}

/// The full-window drop target: a scrim and the same wording as Installed's page veil.
pub fn drop_veil(u: &Ui) -> El {
    col()
        .key("shell-drop-veil")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .padding(16.0)
        .child(
            col()
                .w_full()
                .h_full()
                .center()
                .gap(8.0)
                .radius(crate::ui::r(12.0))
                .fill(u.bg.alpha(0.85))
                .border(1.0, u.accent.alpha(0.6))
                .child(icon(&icons::DOWNLOAD, 24.0).color(u.ink).a11y_hidden())
                .child(text(tr("Drop to import")).font(H1).color(u.ink))
                .child(
                    text(tr("You review the mods before anything is copied."))
                        .font(BODY)
                        .color(u.muted),
                ),
        )
}

// --- keyboard ----------------------------------------------------------------------

/// A text key is the `?` of Shift and the slash key, on layouts that send either.
fn is_help_key(ev: &KeyEvent) -> bool {
    match &ev.key {
        Key::Text(t) => t.as_str() == "?" || (t.as_str() == "/" && ev.mods.shift),
        _ => false,
    }
}

/// Alt+Enter and F11 take the window full screen and bring it back (DMM issue #1414). The
/// window is borderless over its display, and leaving full screen restores the frame it had.
fn is_fullscreen_key(ev: &KeyEvent) -> bool {
    match &ev.key {
        Key::Function(11) => !ev.mods.cmd && !ev.mods.ctrl && !ev.mods.shift,
        Key::Enter => ev.mods.alt && !ev.mods.cmd && !ev.mods.ctrl,
        _ => false,
    }
}

/// Chords the shell owns. Runs before the screen shortcuts so zoom and the sheet work
/// everywhere; false lets the rest of `on_key` run.
pub fn on_key(st: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if is_fullscreen_key(ev) {
        tx.toggle_full_screen();
        return true;
    }
    if (st.shell.shortcuts || st.shell.whats_new.is_some())
        && (ev.key == Key::Escape || is_help_key(ev))
    {
        st.shell.shortcuts = false;
        st.shell.whats_new = None;
        return true;
    }
    if is_help_key(ev) {
        st.shell.shortcuts = !st.shell.shortcuts;
        st.shell.whats_new = None;
        return true;
    }
    let Key::Command(c) = &ev.key else {
        return false;
    };
    match *c {
        '=' | '+' => {
            zoom(st, 1);
            true
        }
        '-' | '_' => {
            zoom(st, -1);
            true
        }
        '0' => {
            zoom(st, 0);
            true
        }
        'm' if ev.mods.shift => {
            open_mods_folder(st);
            true
        }
        // Collapse or expand the sidebar (DMM's shortcut).
        'b' if !ev.mods.shift => {
            crate::shell_w4::toggle_sidebar(st);
            true
        }
        _ => false,
    }
}

/// Step the page zoom through the sizes Settings offers: 0 resets to 100%.
fn zoom(st: &mut State, direction: i8) {
    let current = st
        .prefs
        .zoom
        .clamp(ZOOMS[0], ZOOMS[ZOOMS.len() - 1]);
    let at = ZOOMS
        .iter()
        .position(|size| (size - current).abs() < 0.001)
        .unwrap_or(1);
    let next = if direction == 0 {
        1.0
    } else if direction > 0 {
        ZOOMS[(at + 1).min(ZOOMS.len() - 1)]
    } else {
        ZOOMS[at.saturating_sub(1)]
    };
    if (next - st.prefs.zoom).abs() < 0.001 {
        return;
    }
    crate::prefs::edit(st, |p| p.zoom = next);
    let percent = (next * 100.0).round() as i32;
    crate::notify::push(
        st,
        crate::notify::Notice::info(fill(tr("Zoom {}%"), &[&percent.to_string()])),
    );
}

/// Open the addons folder in the file manager.
pub fn open_mods_folder(st: &mut State) {
    let Some(dir) = st
        .addons_dir
        .clone()
        .or_else(|| st.game_root.as_ref().map(|root| {
            deadlock_core::deploy::addons_dir(root).display().to_string()
        }))
    else {
        st.status = tr("Sigil has not found the game folder yet. Set it in Settings.").to_string();
        return;
    };
    let path = PathBuf::from(&dir);
    if !path.exists() {
        st.status = fill(
            tr("The mods folder is not there yet: {}"),
            &[&path.display().to_string()],
        );
        return;
    }
    if rune::is_headless() {
        st.status = fill(tr("Headless run: the mods folder was not opened ({})."), &[&dir]);
        return;
    }
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(&path).spawn().map(|_| ());
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("explorer").arg(&path).spawn().map(|_| ());
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(&path).spawn().map(|_| ());
    match result {
        Ok(()) => {
            crate::notify::push(
                st,
                crate::notify::Notice::success(tr("Mods folder open")),
            );
        }
        Err(error) => {
            st.status = fill(tr("The mods folder could not be opened: {}"), &[&error.to_string()]);
        }
    }
}

// --- always on top -----------------------------------------------------------------

thread_local! {
    /// The main window, handed over by `main.rs`. UI thread only.
    static WINDOW: std::cell::RefCell<Option<rune::WindowHandle>> =
        const { std::cell::RefCell::new(None) };
}

/// Hook for `main.rs`: `.on_window(|w| shell::remember_window(w))`.
pub fn remember_window(window: &rune::WindowHandle) {
    WINDOW.with(|slot| *slot.borrow_mut() = Some(window.clone()));
}

/// Keep Sigil above other windows, or let it go back to normal.
pub fn set_always_on_top(st: &mut State, on: bool) {
    st.shell.on_top = on;
    if rune::is_headless() {
        return;
    }
    let done = WINDOW.with(|slot| match slot.borrow().as_ref() {
        Some(window) => {
            window.set_always_on_top(on);
            true
        }
        None => false,
    });
    if !done {
        st.status = tr("This build cannot pin the window yet.").to_string();
    }
}

pub fn always_on_top(st: &State) -> bool {
    st.shell.on_top
}

// --- what's new --------------------------------------------------------------------

fn notes_path(dir: &Path) -> PathBuf {
    dir.join("whats-new.json")
}

fn shown_path(dir: &Path) -> PathBuf {
    dir.join("whats-new-shown.json")
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let io = |e: std::io::Error| format!("{}: {e}", path.display());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(value).unwrap_or_default()).map_err(io)?;
    std::fs::rename(&tmp, path).map_err(io)
}

/// Store the release notes an offer carries, once per version. Nothing for an offer this copy
/// cannot install, so a hand-made release without a manifest leaves nothing behind.
pub fn save_notes(dir: &Path, version: &str, body: &str) -> Result<(), String> {
    let body: String = body.chars().filter(|c| *c != '\0').take(NOTES_CAP).collect();
    write_json(&notes_path(dir), &json!({"version": version, "notes": body}))
}

/// The notes on disk, if any.
pub fn read_notes(dir: &Path) -> Option<(String, String)> {
    let v = read_json(&notes_path(dir))?;
    let version = v.get("version")?.as_str()?.trim();
    let notes = v.get("notes")?.as_str()?;
    if version.is_empty() || notes.trim().is_empty() {
        return None;
    }
    Some((version.to_string(), notes.to_string()))
}

fn shown_version(dir: &Path) -> Option<String> {
    read_json(&shown_path(dir))?
        .get("version")?
        .as_str()
        .map(str::to_string)
}

fn mark_shown(dir: &Path, version: &str) -> Result<(), String> {
    write_json(&shown_path(dir), &json!({"version": version}))
}

/// Show the notes of the version this copy just started on, once.
pub fn on_startup(st: &mut State) {
    if st.demo || rune::is_headless() {
        return;
    }
    let dir = crate::updater::updates_dir();
    let Some((version, body)) = read_notes(&dir) else {
        return;
    };
    if version != crate::updater::VERSION {
        return;
    }
    if shown_version(&dir).as_deref() == Some(version.as_str()) {
        return;
    }
    if mark_shown(&dir, &version).is_ok() {
        st.shell.whats_new = Some(Notes { version, body });
    }
}

/// Keep the notes of an offered update so the next start can show them. Called from `poll`.
fn keep_offer(st: &mut State) {
    if st.demo || rune::is_headless() {
        return;
    }
    if !st.shell.notes_loaded {
        st.shell.notes_loaded = true;
        st.shell.saved = read_notes(&crate::updater::updates_dir()).map(|(version, _)| version);
    }
    let offer = match &st.settings.update {
        Some(Ok(check)) if check.newer => check.latest.as_ref(),
        _ => None,
    };
    let Some(offer) = offer else {
        return;
    };
    if offer.install.is_err() || st.shell.saved.as_deref() == Some(offer.version.as_str()) {
        return;
    }
    let dir = crate::updater::updates_dir();
    if save_notes(&dir, &offer.version, &offer.notes).is_ok() {
        st.shell.saved = Some(offer.version.clone());
    }
}

/// A line of release notes, styled by what it starts with.
fn note_line(u: &Ui, line: &str) -> El {
    let (body, color) = if line.starts_with("- ") || line.starts_with("* ") {
        (&line[2..], u.muted)
    } else {
        (line, u.ink)
    };
    text(body.to_string())
        .font(if line.starts_with('-') { BODY } else { BODY_MED })
        .color(color)
        .wrap()
        .w_full()
        .a11y_hidden()
}

/// "What's new in Sigil X", from the update's own release notes.
pub fn whats_new_dialog(s: &State, u: &Ui) -> Option<El> {
    let notes = s.shell.whats_new.as_ref()?;
    let title = fill(tr("What's new in {}"), &[&notes.version]);
    let lines: Vec<&str> = notes
        .body
        .lines()
        .map(str::trim_end)
        .take(NOTES_LINES)
        .collect();
    let mut list = col().key("whats-new-notes").w_full().gap(4.0);
    for (index, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            list = list.child(el().h(8.0));
            continue;
        }
        let heading = index == 0 || line.trim_start().starts_with('#');
        let clean = line.trim_start().trim_start_matches('#').trim();
        list = list.child(if heading {
            text(clean.to_string())
                .font(BODY_MED)
                .color(u.ink)
                .wrap()
                .w_full()
                .a11y_hidden()
        } else if clean.starts_with("- ") || clean.starts_with("* ") {
            note_line(u, clean)
        } else {
            text(clean.to_string())
                .font(BODY)
                .color(u.muted)
                .wrap()
                .w_full()
                .a11y_hidden()
        });
    }
    let scrolling = u.scroll_area(col()
        .key("whats-new-scroll")
        .w_full()
        .max_h(320.0)
        .scroll(ScrollSpec { stick_bottom: false })
        .child(list));
    let body = col()
        .w_full()
        .gap(16.0)
        .child(text(title.clone()).font(H1).color(u.ink))
        .child(
            text(tr("Release notes for the version you are running."))
                .font(BODY)
                .color(u.muted),
        )
        .child(scrolling)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .child(spacer())
                .child(
                    u.action(tr("Got it"), ActionKind::Primary)
                        .key("whats-new-close")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| st.shell.whats_new = None),
                ),
        );
    Some(
        u.dialog_shell()
            .key("whats-new-dialog")
            .role("dialog")
            .label(title)
            .w(520.0)
            .padding(24.0)
            .modal(true, |st: &mut State| st.shell.whats_new = None)
            .on_key(|st: &mut State, ev: &KeyEvent, _| {
                if ev.key == Key::Escape {
                    st.shell.whats_new = None;
                    true
                } else {
                    false
                }
            })
            .child(body),
    )
}

// --- keyboard shortcuts sheet ------------------------------------------------------

/// The shortcuts sheet, grouped. Keyboard-first players find every chord here.
pub fn shortcuts_dialog(s: &State, u: &Ui) -> Option<El> {
    if !s.shell.shortcuts {
        return None;
    }
    let groups: [(&str, &[(&str, &str)]); 4] = [
        (
            "Pages",
            &[
                ("Browse", "⌘1"),
                ("Installed", "⌘2"),
                ("Overlap", "⌘3"),
                ("Updates", "⌘4"),
                ("Stats", "⌘5"),
                ("Locker", "⌘6"),
                ("Agent", "⌘7"),
                ("Profiles", "⌘P"),
                ("Autoexec", "⌘E"),
                ("Crosshair", "⌘X"),
                ("Game", "⌘L"),
                ("Downloads", "⌘J"),
                ("Settings", "⌘,"),
            ],
        ),
        (
            "Window",
            &[
                ("Zoom in", "⌘="),
                ("Zoom out", "⌘-"),
                ("Reset zoom", "⌘0"),
                ("Open mods folder", "⇧⌘M"),
                ("Toggle fullscreen", "F11, Alt+Enter"),
                ("Shortcuts", "?"),
                ("Close window", "⌘W"),
                ("Quit Sigil", "⌘Q"),
            ],
        ),
        (
            "Installed",
            &[
                ("Select a mod", "Click"),
                ("Select a range", "Shift click"),
                ("Open a mod's menu", "Right click"),
            ],
        ),
        (
            "Lists",
            &[
                ("Move between rows", "↑ ↓"),
                ("Pick a row", "Enter"),
                ("Search the page", "⌘F, /"),
                ("Close a dialog", "Esc"),
            ],
        ),
    ];
    let mut columns = [
        col().grow(1.0).min_w(0.0).gap(16.0),
        col().grow(1.0).min_w(0.0).gap(16.0),
        col().grow(1.0).min_w(0.0).gap(16.0),
    ];
    for (index, (title, rows)) in groups.into_iter().enumerate() {
        // A build without the agent has no Agent shortcut.
        let rows: Vec<(&str, &str)> =
            rows.iter().copied().filter(|(action, _)| crate::prefs::AGENT || *action != "Agent").collect();
        let group = shortcut_group(u, title, &rows);
        // Pages fill the first column; Window the second; Installed and Lists share the third.
        let column = match index {
            1 => 1,
            2 | 3 => 2,
            _ => 0,
        };
        columns[column] = std::mem::replace(&mut columns[column], col()).child(group);
    }
    let mut body = col()
        .w_full()
        .gap(16.0)
        .child(text(tr("Keyboard shortcuts")).font(H1).color(u.ink))
        .child(
            row()
                .w_full()
                .gap(24.0)
                .items_start()
                .children(columns),
        );
    body = body.child(
        row()
            .w_full()
            .gap(8.0)
            .child(spacer())
            .child(
                u.action(tr("Close"), ActionKind::Secondary)
                    .key("shortcuts-close")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| st.shell.shortcuts = false),
            ),
    );
    Some(
        u.dialog_shell()
            .key("shortcuts-dialog")
            .role("dialog")
            .label(tr("Keyboard shortcuts"))
            .w(720.0)
            .padding(24.0)
            .modal(true, |st: &mut State| st.shell.shortcuts = false)
            .on_key(|st: &mut State, ev: &KeyEvent, _| {
                if ev.key == Key::Escape {
                    st.shell.shortcuts = false;
                    true
                } else {
                    false
                }
            })
            .child(body),
    )
}

/// One titled group of shortcuts: the action left, its keys right.
fn shortcut_group(u: &Ui, title: &str, rows: &[(&str, &str)]) -> El {
    let mut group = col().key(format!("shortcuts-{title}")).w_full().gap(0.0);
    group = group.child(u.section(tr(title)).mb(4.0));
    for (action, keys) in rows {
        group = group.child(
            row()
                .w_full()
                .h(crate::ui::CONTROL_M)
                .gap(12.0)
                .center_y()
                .child(text(tr(action)).font(BODY).color(u.ink).grow(1.0).min_w(0.0).truncate())
                .child(text(tr(keys).to_string()).font(crate::ui::SMALL_NUM).color(u.muted)),
        );
    }
    group
}

// --- pre-blurred background --------------------------------------------------------

fn images_dir() -> PathBuf {
    if rune::is_headless() {
        return std::env::temp_dir().join("sigil-headless").join(crate::ui::personalize::DIR);
    }
    crate::ui::personalize::images_dir(&crate::data::data_dir())
}

fn blur_key(path: &str, blur: f32) -> String {
    format!("{path}|{blur:.2}")
}

/// Decode, scale and blur once. `sigma` is in image pixels, sized for a window about the
/// default width, so the result matches what the live blur drew before.
fn blur_file(path: &Path, blur: f32) -> Result<(u32, u32, Vec<u8>), String> {
    let io = |e: image::ImageError| format!("{}: {e}", path.display());
    let mut decoded = image::open(path).map_err(io)?.to_rgba8();
    let (mut w, mut h) = decoded.dimensions();
    if w > 2000 {
        let scale = 2000.0 / w as f32;
        w = 2000;
        h = ((h as f32 * scale).round() as u32).max(1);
        decoded = image::imageops::resize(&decoded, w, h, image::imageops::FilterType::Triangle);
    }
    let sigma = blur * (w as f32 / 1440.0).max(0.5);
    let blurred = image::imageops::blur(&decoded, sigma);
    Ok((w, h, blurred.into_raw()))
}

/// Start the one-time blur when Personalize shows a background with blur. Called from poll;
/// the UI thread never decodes or blurs.
pub fn ensure_background(st: &mut State) -> bool {
    if rune::is_headless() {
        return false;
    }
    let p = &st.prefs.personal.background;
    let (Some(file), true, blur) = (&p.file, p.on, p.blur) else {
        return false;
    };
    if blur <= 0.0 {
        return false;
    }
    let path = images_dir().join(file).display().to_string();
    let key = blur_key(&path, blur);
    if st.shell.blurred.iter().any(|(k, _)| *k == key)
        || st.shell.blur_failed.as_deref() == Some(key.as_str())
    {
        return false;
    }
    if st.shell.blur_job.is_running() {
        return false;
    }
    let wake = st.wake.clone();
    let work = path.clone();
    st.shell.blur_running = Some(key.clone());
    st.shell.blur_job.start(wake, move || {
        blur_file(Path::new(&work), blur).map(|(w, h, rgba)| (key, w, h, rgba))
    });
    false
}

/// Take the worker's blurred image. Called from poll.
pub fn take_background(st: &mut State) -> bool {
    let Some(result) = st.shell.blur_job.try_take() else {
        return false;
    };
    match result {
        Ok((key, w, h, rgba)) => {
            st.shell.blur_failed = None;
            st.shell.blur_running = None;
            let pixels = Pixels::new();
            pixels.set(w, h, rgba);
            st.shell.blurred.retain(|(k, _)| *k != key);
            st.shell.blurred.insert(0, (key, pixels));
            st.shell.blurred.truncate(2);
            true
        }
        Err(_) => {
            // Keep the sharp image with its live blur rather than retrying a damaged file.
            st.shell.blur_failed = st.shell.blur_running.take();
            true
        }
    }
}

/// The background pixels to draw and how much live blur is still needed. A ready pre-blur
/// needs none; while the worker runs the sharp image keeps the live blur, so nothing flashes.
pub fn background_pixels<'a>(
    s: &'a State,
    path: Option<String>,
    blur: f32,
) -> Option<(&'a Pixels, f32)> {
    let path = path?;
    if blur <= 0.0 {
        return s.thumbnails.get(&path).map(|px| (px, 0.0));
    }
    let key = blur_key(&path, blur);
    if let Some((_, px)) = s.shell.blurred.iter().find(|(k, _)| *k == key) {
        return Some((px, 0.0));
    }
    s.thumbnails.get(&path).map(|px| (px, blur))
}

/// Called from the app's poll every frame.
pub fn poll(st: &mut State) -> bool {
    keep_offer(st);
    ensure_background(st);
    take_background(st)
}

// --- scenes -----------------------------------------------------------------------

/// A page with a blurred background image, for the frame measurement and a snapshot.
pub fn scene_background(s: &mut State) {
    screens::browse::scene_fixture(s);
    let dir = images_dir();
    let _ = std::fs::create_dir_all(&dir);
    let file = "background-shell-scene.png";
    let path = dir.join(file);
    if !path.exists() {
        let mut image = image::RgbaImage::new(1600, 1000);
        for (x, y, px) in image.enumerate_pixels_mut() {
            let u = x as f32 / 1600.0;
            let v = y as f32 / 1000.0;
            let warm = (1.0 - (u * 0.8 + v * 0.5)).clamp(0.0, 1.0);
            *px = image::Rgba([
                (40.0 + 150.0 * warm) as u8,
                (48.0 + 90.0 * warm) as u8,
                (70.0 + 40.0 * warm) as u8,
                255,
            ]);
        }
        let _ = image.save(&path);
    }
    let blur = 12.0;
    // `SIGIL_BLUR_LIVE=1` keeps the sharp image so the old live-blur draw can be measured
    // against the pre-blurred one (the perf script's A/B).
    #[cfg(debug_assertions)]
    let live = std::env::var_os("SIGIL_BLUR_LIVE").is_some();
    #[cfg(not(debug_assertions))]
    let live = false;
    let key = blur_key(&path.display().to_string(), blur);
    if !live
        && let Ok((w, h, rgba)) = blur_file(&path, blur)
    {
        let pixels = Pixels::new();
        pixels.set(w, h, rgba);
        s.shell.blurred.insert(0, (key, pixels));
        s.shell.blurred.truncate(2);
    }
    s.prefs.personal.background.file = Some(file.to_string());
    s.prefs.personal.background.on = true;
    s.prefs.personal.background.opacity = 0.4;
    s.prefs.personal.background.blur = blur;
    s.prefs.personal.background.sidebar = true;
}

/// The shortcuts sheet over Browse.
pub fn scene_shortcuts(s: &mut State) {
    screens::browse::scene_fixture(s);
    s.shell.shortcuts = true;
}

/// The What's new dialog after an update.
pub fn scene_whats_new(s: &mut State) {
    screens::browse::scene_fixture(s);
    s.shell.whats_new = Some(Notes {
        version: "0.3.0".to_string(),
        body: r#"What's new

- Mods drop anywhere in the window.
- Zoom with Cmd and plus or minus.
- A shortcuts sheet on the ? key.
- Sidebar counts for installed mods and overlaps.

Fixes

- Load order saves after a drag.
- The game folder opens from Cmd and M."#
            .to_string(),
    });
}

/// A file drag over a page without its own target.
pub fn scene_drop(s: &mut State) {
    screens::browse::scene_fixture(s);
    s.shell.drop_hover = true;
}

/// Compact density over Browse: the Appearance option at its smallest.
pub fn scene_density(s: &mut State) {
    screens::browse::scene_fixture(s);
    s.prefs.personal.density = crate::ui::personalize::Density::Compact;
}

// --- tests ------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sigil-shell-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn notes_round_trip_and_ignore_damage() {
        let dir = temp("notes");
        assert!(read_notes(&dir).is_none());
        save_notes(&dir, "0.3.0", "What's new\n\n- A thing").unwrap();
        assert_eq!(
            read_notes(&dir),
            Some(("0.3.0".to_string(), "What's new\n\n- A thing".to_string()))
        );
        std::fs::write(notes_path(&dir), "not json").unwrap();
        assert!(read_notes(&dir).is_none());
        save_notes(&dir, "", "notes").unwrap();
        assert!(read_notes(&dir).is_none(), "a version is required");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shown_marker_keeps_the_dialog_once() {
        let dir = temp("shown");
        assert_eq!(shown_version(&dir), None);
        mark_shown(&dir, "0.3.0").unwrap();
        assert_eq!(shown_version(&dir), Some("0.3.0".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn notes_are_cut_and_cleaned() {
        let dir = temp("cut");
        let body = format!("Start\0{}", "x".repeat(NOTES_CAP * 2));
        save_notes(&dir, "1.0.0", &body).unwrap();
        let (_, notes) = read_notes(&dir).unwrap();
        assert!(notes.len() <= NOTES_CAP);
        assert!(!notes.contains('\0'));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mouse_back_and_forward_walk_pages() {
        let mut st = State::default();
        st.screen = Screen::Browse;
        // Record history the way `State::go` does, without starting a page's reads.
        st.shell.back.push(Screen::Browse);
        st.screen = Screen::Crosshair;
        st.shell.back.push(Screen::Crosshair);
        st.screen = Screen::Autoexec;
        assert!(st.shell.forward.is_empty());

        back(&mut st);
        assert!(st.screen == Screen::Crosshair);
        back(&mut st);
        assert!(st.screen == Screen::Browse);
        forward(&mut st);
        assert!(st.screen == Screen::Crosshair);

        // A new page clears what Forward could have returned to.
        st.go(Screen::Profiles);
        assert!(st.shell.forward.is_empty());
    }

    #[test]
    fn mouse_back_closes_a_creator_page_before_the_page_behind_it() {
        let mut st = State::default();
        st.screen = Screen::Browse;
        st.browse_view.filters.creator = Some(crate::screens::browse::Pick {
            id: 7,
            name: "Ana".to_string(),
        });
        st.shell.back.push(Screen::Installed);
        back(&mut st);
        assert!(st.browse_view.filters.creator.is_none());
        assert!(st.screen == Screen::Browse);
        back(&mut st);
        assert!(st.screen == Screen::Installed);
    }

    #[test]
    fn zoom_steps_through_the_sizes() {
        assert_eq!(ZOOMS, [0.9, 1.0, 1.1, 1.25]);
        let step = |at: usize, direction: i8| {
            if direction > 0 {
                ZOOMS[(at + 1).min(ZOOMS.len() - 1)]
            } else {
                ZOOMS[at.saturating_sub(1)]
            }
        };
        assert_eq!(step(1, 1), 1.1);
        assert_eq!(step(1, -1), 0.9);
        assert_eq!(step(3, 1), 1.25);
        assert_eq!(step(0, -1), 0.9);
    }
}
