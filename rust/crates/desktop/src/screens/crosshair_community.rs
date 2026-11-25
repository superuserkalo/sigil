//! The Community shelf of the Crosshair page: the community crosshair library (ADR 0019) as the
//! same live mini reticles the presets use.
//!
//! Nothing here touches the network before the player opens the Community view. The list is
//! fetched on a worker, kept 5 minutes per sort, and every row has passed
//! `deadlock_core::crosshair_library` checks before it is drawn. Pressing a tile only loads it
//! into the editor; Apply stays the only write to the game, and an Apply of a library crosshair
//! left unchanged sends one `use` (fire and forget). Publishing and removing act on the
//! player's own saved presets; the delete tokens live in `crosshair-library.json` in the app
//! data directory, owner-only on Unix, and never enter a log line or the UI.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use deadlock_core::crosshair::Crosshair;
use deadlock_core::crosshair_library::{
    self as library, Client, LibraryCrosshair, LibraryError, Page, Published, Sort, Store,
};
use rune::{Color, El, Key, KeyEvent, TextEdit, col, el, icon, row, spacer, text};

use super::{Chosen, PresetDialog, TILE, oklab_dark, tr};
use crate::app::State;
use crate::icons;
use crate::jobs::Job;
use crate::ui::{ActionKind, BODY, SMALL, Tone, Ui};

/// How long a fetched list stays fresh.
pub const CACHE_FOR: Duration = Duration::from_secs(5 * 60);
/// A community tile: the 64 point art and room for a name and a "by Sigil, 12 uses" line.
pub const TILE_W: f32 = 120.0;
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Which set the strip under the stage shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shelf {
    #[default]
    Presets,
    Community,
}

/// One sort's rows, as fetched.
#[derive(Clone, Debug, Default)]
pub struct Listing {
    pub rows: Vec<LibraryCrosshair>,
    pub next_cursor: Option<String>,
    /// When the first page arrived; None for a listing a scene filled.
    pub at: Option<Instant>,
}

impl Listing {
    fn fresh(&self) -> bool {
        self.at.is_none_or(|at| at.elapsed() < CACHE_FOR)
    }
}

/// A page fetch: its sort, whether it extends the listing, and the answer.
pub struct Fetched {
    sort: Sort,
    append: bool,
    page: Result<Page, LibraryError>,
}

#[derive(Default)]
pub struct Community {
    pub sort: Sort,
    pub cache: HashMap<Sort, Listing>,
    pub fetch: Job<Fetched>,
    /// The last failed fetch and its sort: the strip shows it and nothing retries on its own.
    pub failed: Option<(Sort, LibraryError)>,
    /// Published presets and reported ids, read once.
    pub store: Store,
    pub store_loaded: bool,
    /// The library crosshair last loaded into the editor: its id and look.
    pub loaded: Option<(String, Crosshair)>,
    /// The library id a duplicate publish pointed at, ringed in the strip.
    pub found: Option<String>,
    /// The tile whose right-click menu is open.
    pub menu: Option<String>,
    /// The Report confirm: id and name.
    pub report: Option<(String, String)>,
    pub publish: Job<(String, Result<Published, LibraryError>)>,
    /// The preset being removed from the library.
    pub remove: Job<(String, Result<(), LibraryError>)>,
    /// The last `use` this app sent (or would send in a headless run), for tests.
    pub last_use: Option<String>,
}

fn store_path() -> std::path::PathBuf {
    crate::data::data_dir().join(library::STORE_FILE)
}

/// Demo and headless runs never touch the network or the store file.
/// Unit tests count as local too: they never reach the Worker or the real data directory.
fn local_only(st: &State) -> bool {
    cfg!(test) || st.demo || rune::is_headless()
}

fn load_store(st: &mut State) {
    if st.crosshair.community.store_loaded {
        return;
    }
    st.crosshair.community.store_loaded = true;
    if !local_only(st) {
        st.crosshair.community.store = library::load_store(&store_path());
    }
}

fn save_store(st: &State) -> Result<(), String> {
    if local_only(st) {
        return Ok(());
    }
    library::save_store(&store_path(), &st.crosshair.community.store)
        .map_err(|_| tr("Could not save the library file.").to_string())
}

/// The published entry for a saved preset, if any.
pub fn published<'a>(st: &'a State, preset: &str) -> Option<&'a Published> {
    st.crosshair.community.store.published.get(preset)
}

// --- the list ------------------------------------------------------------------------------

/// The player opened the Community view: read the store and fetch the list unless the cache
/// for this sort is fresh.
pub fn open(st: &mut State) {
    st.crosshair.shelf = Shelf::Community;
    load_store(st);
    st.crosshair.community.failed = None;
    ensure(st);
}

fn set_sort(st: &mut State, sort: Sort) {
    if st.crosshair.community.sort == sort {
        return;
    }
    st.crosshair.community.sort = sort;
    st.crosshair.community.failed = None;
    ensure(st);
}

/// Start the first page for the current sort when there is no fresh listing.
fn ensure(st: &mut State) {
    let c = &st.crosshair.community;
    let sort = c.sort;
    if c.cache.get(&sort).is_some_and(Listing::fresh) || c.fetch.is_running() {
        return;
    }
    if c.failed.as_ref().is_some_and(|(failed, _)| *failed == sort) {
        return;
    }
    start(st, sort, None);
}

fn retry(st: &mut State) {
    st.crosshair.community.failed = None;
    let sort = st.crosshair.community.sort;
    let cursor = st
        .crosshair
        .community
        .cache
        .get(&sort)
        .filter(|listing| listing.fresh() && !listing.rows.is_empty())
        .and_then(|listing| listing.next_cursor.clone());
    start(st, sort, cursor);
}

fn load_more(st: &mut State) {
    let sort = st.crosshair.community.sort;
    let Some(cursor) = st
        .crosshair
        .community
        .cache
        .get(&sort)
        .and_then(|listing| listing.next_cursor.clone())
    else {
        return;
    };
    st.crosshair.community.failed = None;
    start(st, sort, Some(cursor));
}

fn start(st: &mut State, sort: Sort, cursor: Option<String>) {
    if st.crosshair.community.fetch.is_running() {
        return;
    }
    if local_only(st) {
        // Snapshots fill the cache themselves; a bare headless run shows the offline line.
        if !st.crosshair.community.cache.contains_key(&sort) {
            st.crosshair.community.failed = Some((sort, LibraryError::Offline));
        }
        return;
    }
    let append = cursor.is_some();
    let wake = st.wake.clone();
    st.crosshair.community.fetch.start(wake, move || {
        let page = Client::new(VERSION).list(sort, cursor.as_deref());
        Ok(Fetched { sort, append, page })
    });
}

/// Take a finished fetch: a good page replaces or extends the listing; a failure keeps
/// whatever was cached and shows its line.
pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.crosshair.community.fetch.try_take() {
        changed = true;
        let c = &mut st.crosshair.community;
        match result {
            Ok(Fetched { sort, append, page: Ok(page) }) => {
                let listing = c.cache.entry(sort).or_default();
                if append {
                    for row in page.rows {
                        if !listing.rows.iter().any(|kept| kept.id == row.id) {
                            listing.rows.push(row);
                        }
                    }
                } else {
                    listing.rows = page.rows;
                    listing.at = Some(Instant::now());
                }
                listing.next_cursor = page.next_cursor;
                if c.failed.as_ref().is_some_and(|(failed, _)| *failed == sort) {
                    c.failed = None;
                }
            }
            Ok(Fetched { sort, page: Err(error), .. }) => c.failed = Some((sort, error)),
            Err(error) => c.failed = Some((c.sort, LibraryError::Failed(error))),
        }
        // The player may have switched sort while the other one loaded.
        if st.crosshair.shelf == Shelf::Community && st.screen == crate::model::Screen::Crosshair {
            ensure(st);
        }
    }
    changed |= poll_publish(st);
    changed |= poll_remove(st);
    changed
}

/// The rows the strip shows: the current sort's listing without what this player reported.
fn visible(st: &State) -> Vec<&LibraryCrosshair> {
    let c = &st.crosshair.community;
    c.cache
        .get(&c.sort)
        .map(|listing| {
            listing
                .rows
                .iter()
                .filter(|row| !c.store.is_reported(&row.id))
                .collect()
        })
        .unwrap_or_default()
}

fn find(st: &State, id: &str) -> Option<LibraryCrosshair> {
    st.crosshair
        .community
        .cache
        .values()
        .flat_map(|listing| listing.rows.iter())
        .find(|row| row.id == id)
        .cloned()
}

// --- load and apply ------------------------------------------------------------------------

/// Load a library crosshair into the editor, exactly as a preset loads: nothing is written to
/// the game. The player's hero-specific toggle stays as it is.
pub fn load(st: &mut State, id: &str) {
    st.crosshair.community.menu = None;
    let Some(row) = find(st, id) else {
        return;
    };
    let keep = st.crosshair.settings.disable_hero_specific_crosshairs;
    st.crosshair.settings = Crosshair {
        disable_hero_specific_crosshairs: keep,
        ..row.settings.clone()
    };
    st.crosshair.source = row.name.clone();
    st.crosshair.chosen = None;
    st.crosshair.log = None;
    st.crosshair.tone = Tone::Quiet;
    st.crosshair.community.loaded = Some((row.id, row.settings));
}

/// The library id to count for this Apply: the crosshair last loaded from the library, when
/// the editor still has its look.
pub fn use_to_send(st: &State) -> Option<String> {
    let (id, look) = st.crosshair.community.loaded.as_ref()?;
    library::same_look(look, &st.crosshair.settings).then(|| id.clone())
}

/// Called after a successful Apply: count one use for an unchanged library crosshair, fire and
/// forget. The result is never shown.
pub fn after_apply(st: &mut State) {
    let id = use_to_send(st);
    st.crosshair.community.loaded = None;
    let Some(id) = id else {
        return;
    };
    st.crosshair.community.last_use = Some(id.clone());
    if local_only(st) {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("dah-crosshair-use".to_string())
        .spawn(move || {
            let _ = Client::new(VERSION).send_use(&id);
        });
}

// --- report --------------------------------------------------------------------------------

fn open_report(st: &mut State, id: String, name: String) {
    st.crosshair.community.menu = None;
    st.crosshair.community.report = Some((id, name));
}

/// The player confirmed a report: hide the tile at once, keep the id, send the report in the
/// background and say thanks. A failed send is not shown.
fn confirm_report(st: &mut State) {
    let Some((id, _)) = st.crosshair.community.report.take() else {
        return;
    };
    st.crosshair.community.store.add_reported(&id);
    let _ = save_store(st);
    crate::notify::push(st, crate::notify::Notice::info(tr("Thanks")).detail(tr("The crosshair is hidden from your list.")));
    if local_only(st) {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("dah-crosshair-report".to_string())
        .spawn(move || {
            let _ = Client::new(VERSION).report(&id);
        });
}

// --- publish and remove --------------------------------------------------------------------

/// Open the Publish dialog for the saved preset `preset`, its name prefilled.
pub fn open_publish(st: &mut State, preset: &str) {
    load_store(st);
    let Some(saved) = st.crosshair.presets.iter().find(|p| p.id == preset) else {
        return;
    };
    st.crosshair.preset_dialog = Some(PresetDialog::Publish {
        preset: preset.to_string(),
        name: TextEdit::new(&saved.name),
        error: None,
    });
}

fn set_error(st: &mut State, message: String) {
    if let Some(PresetDialog::Publish { error, .. }) = st.crosshair.preset_dialog.as_mut() {
        *error = Some(message);
    }
}

fn submit_publish(st: &mut State) {
    if st.crosshair.community.publish.is_running() {
        return;
    }
    let Some(PresetDialog::Publish { preset, name, .. }) = st.crosshair.preset_dialog.as_ref() else {
        return;
    };
    let preset = preset.clone();
    let name = match library::clean_name(&name.text) {
        Ok(name) => name,
        Err(error) => return set_error(st, tr(&error.message()).to_string()),
    };
    let Some(settings) = st
        .crosshair
        .presets
        .iter()
        .find(|p| p.id == preset)
        .map(|p| p.settings.clone())
    else {
        return set_error(st, tr("That preset no longer exists.").to_string());
    };
    if deadlock_core::net::is_offline() || local_only(st) {
        return set_error(st, tr(&LibraryError::Offline.message()).to_string());
    }
    if let Some(PresetDialog::Publish { error, .. }) = st.crosshair.preset_dialog.as_mut() {
        *error = None;
    }
    let wake = st.wake.clone();
    st.crosshair.community.publish.start(wake, move || {
        Ok((preset, Client::new(VERSION).publish(&name, &settings)))
    });
}

/// What a publish answer does to the page. Split from the worker so tests can drive it.
pub fn finish_publish(st: &mut State, preset: String, result: Result<Published, LibraryError>) {
    match result {
        Ok(entry) => {
            let id = entry.id.clone();
            st.crosshair.community.store.published.insert(preset, entry);
            st.crosshair.preset_dialog = None;
            // The new crosshair belongs at the top of Newest: drop that listing.
            st.crosshair.community.cache.remove(&Sort::New);
            st.crosshair.community.found = Some(id);
            match save_store(st) {
                Ok(()) => crate::notify::push(
                    st,
                    crate::notify::Notice::success(tr("Published to the library"))
                        .detail(tr("Everyone using Sigil can find it under Community.")),
                ),
                Err(error) => crate::notify::push(
                    st,
                    crate::notify::Notice::warning(tr("Published, but Sigil could not keep the removal key"))
                        .detail(error),
                ),
            }
        }
        Err(LibraryError::Duplicate(id)) => {
            // Already there: treat the existing one as found.
            st.crosshair.preset_dialog = None;
            st.crosshair.community.found = (!id.is_empty()).then_some(id);
            crate::notify::push(
                st,
                crate::notify::Notice::info(tr("This crosshair is already in the library")),
            );
        }
        Err(error) => set_error(st, tr(&error.message()).to_string()),
    }
}

fn poll_publish(st: &mut State) -> bool {
    let Some(result) = st.crosshair.community.publish.try_take() else {
        return false;
    };
    match result {
        Ok((preset, result)) => finish_publish(st, preset, result),
        Err(error) => set_error(st, error),
    }
    true
}

/// Remove the saved preset's crosshair from the library with its delete token.
pub fn remove(st: &mut State, preset: &str) {
    if st.crosshair.community.remove.is_running() {
        return;
    }
    let Some(entry) = published(st, preset).cloned() else {
        return;
    };
    let preset = preset.to_string();
    if local_only(st) {
        return finish_remove(st, preset, Ok(()));
    }
    let wake = st.wake.clone();
    st.crosshair.community.remove.start(wake, move || {
        Ok((preset, Client::new(VERSION).delete(&entry)))
    });
}

pub fn finish_remove(st: &mut State, preset: String, result: Result<(), LibraryError>) {
    match result {
        Ok(()) => {
            if let Some(entry) = st.crosshair.community.store.published.remove(&preset) {
                for listing in st.crosshair.community.cache.values_mut() {
                    listing.rows.retain(|row| row.id != entry.id);
                }
            }
            let _ = save_store(st);
            crate::notify::push(st, crate::notify::Notice::success(tr("Removed from the library")));
        }
        Err(error) => crate::notify::push(
            st,
            crate::notify::Notice::error(tr("Could not remove it from the library"))
                .detail(tr(&error.message()).to_string()),
        ),
    }
}

fn poll_remove(st: &mut State) -> bool {
    let Some(result) = st.crosshair.community.remove.try_take() else {
        return false;
    };
    match result {
        Ok((preset, result)) => finish_remove(st, preset, result),
        Err(error) => finish_remove(st, String::new(), Err(LibraryError::Failed(error))),
    }
    true
}

// --- view ----------------------------------------------------------------------------------

/// Most used and Newest, in the page's sliding segmented control.
pub fn sort_control(st: &State, u: &Ui) -> El {
    let selected = match st.crosshair.community.sort {
        Sort::Used => 0,
        Sort::New => 1,
    };
    crate::ui::Segmented::new("crosshair-community-sort", "Sort the library")
        .width(crate::ui::SegWidth::Even(196.0))
        .choices(&[tr("Most used"), tr("Newest")], selected, |st: &mut State, index| {
            set_sort(st, if index == 0 { Sort::Used } else { Sort::New })
        })
        .build(u)
}

/// "by Sigil, 12 uses" or "12 uses".
/// A use count that fits a tile: 377, 1.3k, 12k, 1.2M.
pub fn count(n: u64) -> String {
    let short = |value: f64, unit: &str| {
        let text = if value < 10.0 { format!("{value:.1}") } else { format!("{value:.0}") };
        format!("{}{unit}", text.trim_end_matches(".0"))
    };
    match n {
        0..1000 => n.to_string(),
        1000..999_500 => short(n as f64 / 1000.0, "k"),
        _ => short(n as f64 / 1_000_000.0, "M"),
    }
}

fn meta(row: &LibraryCrosshair) -> String {
    let uses = match row.uses {
        1 => tr("1 use").to_string(),
        n => super::fill(tr("{} uses"), &[&count(n)]),
    };
    if row.official {
        format!("{}, {uses}", tr("by Sigil"))
    } else {
        uses
    }
}

fn library_tile(u: &Ui, row: &LibraryCrosshair, on: bool) -> El {
    let id = row.id.clone();
    let menu_id = row.id.clone();
    let meta = meta(row);
    col()
        .key(format!("crosshair-community-{}", row.id))
        .w(TILE_W)
        .gap(4.0)
        .items_center()
        .focusable(true)
        .role("button")
        .label(format!("Load {}, {meta}", row.name))
        .checked(on)
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| load(st, &id))
        .on_secondary_press(move |st: &mut State, _press, _tx: &mut rune::Tx| {
            st.crosshair.community.menu = Some(menu_id.clone());
        })
        .child(super::tile_art(u, &row.settings, on))
        .child(
            col()
                .w(TILE_W)
                .items_center()
                .child(
                    // Drawn as plain text, never markup; the name already passed the Worker's rules.
                    text(row.name.clone())
                        .font(SMALL)
                        .color(if on { u.ink } else { u.muted })
                        .w(TILE_W)
                        .align(rune::Align::Center)
                        .truncate()
                        .help_when_truncated(row.name.clone()),
                )
                .child(
                    text(meta.clone())
                        .font(SMALL)
                        .color(u.faint)
                        .w(TILE_W)
                        .align(rune::Align::Center)
                        .truncate()
                        .help_when_truncated(meta),
                ),
        )
}

/// A placeholder tile while the first page loads.
fn ghost_tile(u: &Ui, index: usize) -> El {
    col()
        .key(format!("crosshair-community-ghost-{index}"))
        .w(TILE_W)
        .gap(6.0)
        .items_center()
        .a11y_hidden()
        .child(el().size(TILE, TILE).radius(crate::ui::r(4.0)).fill(oklab_dark()).border(1.0, u.hair))
        .child(el().w(56.0).h(8.0).radius(crate::ui::r(2.0)).fill(u.hair))
}

/// The Load more tile at the end of the strip.
fn more_tile(u: &Ui, busy: bool, failed: bool) -> El {
    let label = if busy {
        tr("Loading")
    } else if failed {
        tr("Try again")
    } else {
        tr("Load more")
    };
    let mut tile = col()
        .key("crosshair-community-more")
        .w(TILE_W)
        .gap(6.0)
        .items_center()
        .focusable(!busy)
        .role("button")
        .label(label.to_string())
        .child(
            el().size(TILE, TILE)
                .radius(crate::ui::r(4.0))
                .border(1.0, u.hair)
                .hover(Color::WHITE.alpha(0.05))
                .center()
                .child(
                    icon(if failed { &icons::REFRESH_CW } else { &icons::CHEVRON_RIGHT }, 16.0)
                        .color(u.muted)
                        .a11y_hidden(),
                ),
        )
        .child(text(label.to_string()).font(SMALL).color(u.muted).a11y_hidden());
    if !busy {
        tile = tile
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| if failed { retry(st) } else { load_more(st) });
    }
    tile
}

/// One plain line in place of the tiles (offline, failed, empty), with an optional Try again.
fn note(u: &Ui, key: &str, line: &str, again: bool) -> El {
    let mut r = row()
        .key(key.to_string())
        .h(TILE)
        .gap(12.0)
        .center_y()
        .child(text(line.to_string()).font(BODY).color(u.muted).truncate().min_w(0.0).shrink());
    if again {
        r = r.child(
            u.action(tr("Try again"), ActionKind::Secondary)
                .key("crosshair-community-retry")
                .cursor(rune::Cursor::Pointer)
                .on_press(retry),
        );
    }
    r
}

/// The Community strip: tiles, or one finished state when there are none.
pub fn strip(st: &State, u: &Ui) -> El {
    let c = &st.crosshair.community;
    let rows = visible(st);
    let busy = c.fetch.is_running();
    let failed = c
        .failed
        .as_ref()
        .filter(|(sort, _)| *sort == c.sort)
        .map(|(_, error)| error);
    let mut grid = row()
        .key("crosshair-community")
        // Not pulled left like the presets: a wide name under the first tile would be clipped.
        .gap(4.0)
        .role("list")
        .label("Community crosshairs");
    if rows.is_empty() {
        return match (busy, failed) {
            (true, _) => {
                for index in 0..6 {
                    grid = grid.child(ghost_tile(u, index));
                }
                grid.label(tr("Loading the library"))
            }
            (false, Some(LibraryError::Offline)) => note(
                u,
                "crosshair-community-offline",
                tr("The crosshair library needs a connection. Your presets still work."),
                !deadlock_core::net::is_offline(),
            ),
            (false, Some(error)) => note(u, "crosshair-community-failed", tr(&error.message()), true),
            (false, None) => note(u, "crosshair-community-empty", tr("No crosshairs in the library yet."), false),
        };
    }
    for row in &rows {
        let on = library::same_look(&row.settings, &st.crosshair.settings)
            || c.found.as_deref() == Some(row.id.as_str());
        grid = grid.child(library_tile(u, row, on));
    }
    let more = c.cache.get(&c.sort).is_some_and(|listing| listing.next_cursor.is_some());
    if more || busy || failed.is_some() {
        grid = grid.child(more_tile(u, busy, failed.is_some()));
    }
    grid
}

/// The right-click menu on a community tile: Load, and Report for a community row.
pub fn menu(st: &State, u: &Ui) -> Option<El> {
    let id = st.crosshair.community.menu.clone()?;
    let row = find(st, &id)?;
    let load_id = id.clone();
    let mut menu = u
        .menu("crosshair-community-menu", &format!("crosshair-community-{id}"), 200.0)
        .child(
            u.menu_item("crosshair-community-menu-load", &icons::DOWNLOAD, tr("Load into the editor"), false)
                .on_press(move |st: &mut State| load(st, &load_id)),
        );
    if !row.official {
        let name = row.name.clone();
        menu = menu.child(
            u.menu_item("crosshair-community-menu-report", &icons::TRIANGLE_ALERT, tr("Report"), true)
                .on_press(move |st: &mut State| open_report(st, id.clone(), name.clone())),
        );
    }
    Some(u.popover_layer(
        "crosshair-community-menu-layer",
        el().on_press(|st: &mut State| st.crosshair.community.menu = None),
        menu,
    ))
}

/// The Report confirm.
pub fn report_dialog(st: &State, u: &Ui) -> Option<El> {
    let (_, name) = st.crosshair.community.report.as_ref()?;
    let close = |st: &mut State| st.crosshair.community.report = None;
    Some(
        u.dialog_shell()
            .key("crosshair-report-dialog")
            .role("dialog")
            .label(tr("Report this crosshair?"))
            .w(440.0)
            .padding(24.0)
            .gap(16.0)
            .modal(true, close)
            .on_key(move |st: &mut State, ev: &KeyEvent, _| {
                if ev.key == Key::Escape {
                    close(st);
                    true
                } else {
                    false
                }
            })
            .child(
                col()
                    .w_full()
                    .gap(6.0)
                    .child(text(tr("Report this crosshair?")).font(crate::ui::H1).color(u.ink))
                    .child(
                        text(super::fill(
                            tr("{} is hidden from your list at once. Crosshairs that several players report are taken down for review."),
                            &[name],
                        ))
                        .font(BODY)
                        .color(u.muted)
                        .wrap(),
                    ),
            )
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .pt(8.0)
                    .child(spacer())
                    .child(
                        u.action(tr("Cancel"), ActionKind::Ghost)
                            .key("crosshair-report-cancel")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(close),
                    )
                    .child(
                        u.action(tr("Report"), ActionKind::Danger)
                            .key("crosshair-report-confirm")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(confirm_report),
                    ),
            ),
    )
}

fn bind_publish(st: &mut State) -> &mut TextEdit {
    match st.crosshair.preset_dialog.as_mut() {
        Some(PresetDialog::Publish { name, .. }) => name,
        _ => &mut st.crosshair.blank,
    }
}

/// The Publish dialog: the name, the rule, and one sentence on what becomes public.
pub fn publish_dialog(st: &State, u: &Ui, name: &TextEdit, error: Option<&str>) -> El {
    let busy = st.crosshair.community.publish.is_running();
    let ready = !name.text.trim().is_empty() && !busy;
    let mut body = col()
        .w_full()
        .gap(8.0)
        .child(
            text(tr("Everyone using Sigil will see this crosshair and its name. You can remove it from the library later."))
                .font(BODY)
                .color(u.muted)
                .wrap(),
        )
        .child(
            row()
                .w_full()
                .h(32.0)
                .px(10.0)
                .center_y()
                .radius(crate::ui::r(8.0))
                .fill(u.well)
                .border(1.0, u.hair)
                .child(
                    rune::text_input(name, BODY)
                        .key("crosshair-publish-name")
                        .label("Name in the library")
                        .single_line()
                        .autofocus()
                        .grow(1.0)
                        .min_w(0.0)
                        .color(u.ink)
                        .placeholder(tr("Name").to_string(), u.faint)
                        .caret(u.accent, u.accent.alpha(0.20))
                        .on_edit(bind_publish)
                        .on_submit(|st: &mut State, _| submit_publish(st)),
                ),
        )
        .child(text(tr(library::name_rule())).font(SMALL).color(u.faint).wrap());
    if let Some(error) = error {
        body = body.child(crate::notify::feedback::error_line(u, "crosshair-publish-error", error));
    }
    let confirm = u
        .action(if busy { tr("Publishing") } else { tr("Publish") }, ActionKind::Primary)
        .key("crosshair-publish-confirm")
        .disabled(!ready)
        .opacity(if ready { 1.0 } else { 0.45 })
        .cursor(rune::Cursor::Pointer)
        .on_press(submit_publish);
    super::preset_shell(
        u,
        "crosshair-publish-dialog",
        tr("Publish to the library"),
        col().w_full().gap(16.0).child(body).child(super::buttons(u, None, confirm)),
    )
}

/// The Publish or Remove from library control beside Rename and Delete, for the saved preset
/// last loaded. Disabled (and Publish) for a built-in or nothing loaded.
pub fn publish_control(st: &State, u: &Ui) -> El {
    let saved = match &st.crosshair.chosen {
        Some(Chosen::Saved(id)) if st.crosshair.presets.iter().any(|p| &p.id == id) => Some(id.clone()),
        _ => None,
    };
    if let Some(id) = saved.clone()
        && published(st, &id).is_some()
    {
        let busy = st.crosshair.community.remove.is_running();
        return u
            .icon_action(&icons::X, if busy { tr("Removing") } else { tr("Remove from library") }, ActionKind::Ghost)
            .h(24.0)
            .pl(6.0)
            .pr(8.0)
            .key("crosshair-preset-unpublish")
            .disabled(busy)
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| remove(st, &id));
    }
    let off = saved.is_none();
    let id = saved.unwrap_or_default();
    u.icon_button(&icons::UPLOAD, tr("Publish to the library"))
        .key("crosshair-preset-publish")
        .size(24.0, 24.0)
        .disabled(off)
        .opacity(if off { 0.45 } else { 1.0 })
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            if !id.is_empty() {
                open_publish(st, &id)
            }
        })
}

/// The small globe on a saved preset's art that is in the library.
pub fn published_mark(u: &Ui) -> El {
    el().pin(Some(3.0), Some(3.0), None, None)
        .size(14.0, 14.0)
        .center()
        .radius(crate::ui::r(2.0))
        .fill(Color::BLACK.alpha(0.55))
        .child(icon(&icons::GLOBE, 10.0).color(u.ink))
        .help(tr("Published to the library"))
}

// --- scenes --------------------------------------------------------------------------------

fn scene_rows() -> Vec<LibraryCrosshair> {
    let body = serde_json::json!({
        "crosshairs": [
            {"id": "sigil-default", "name": "Sigil Default", "config": library::config_json(&Crosshair::default()), "uses": 1284, "official": true, "created_day": "2026-11-25"},
            {"id": "5f2a9c01d4e7b3a8", "name": "Tiny Cyan", "config": library::config_json(&Crosshair { pip_gap: 2, pip_height: 5, color_r: 0, color_g: 230, color_b: 255, ..Crosshair::default() }), "uses": 412, "official": false, "created_day": "2026-11-25"},
            {"id": "sigil-red-dot", "name": "Red Dot", "config": library::config_json(&Crosshair { pip_opacity: 0.0, pip_outline_opacity: 0.0, dot_size: 4, dot_opacity: 1.0, dot_outline_border: 1, dot_outline_opacity: 1.0, color_r: 255, color_g: 40, color_b: 40, ..Crosshair::default() }), "uses": 377, "official": true, "created_day": "2026-11-25"},
            {"id": "8c1e04f7a29b6d53", "name": "Dr. Aim's Plus", "config": library::config_json(&Crosshair { pip_gap: 3, pip_height: 8, pip_width: 3, color_r: 255, color_g: 220, color_b: 0, ..Crosshair::default() }), "uses": 96, "official": false, "created_day": "2026-11-25"},
            {"id": "sigil-wide-gap", "name": "Wide Gap", "config": library::config_json(&Crosshair { pip_gap: 14, color_r: 255, color_g: 190, color_b: 0, ..Crosshair::default() }), "uses": 51, "official": true, "created_day": "2026-11-25"},
            {"id": "0b7d3e9a51c2f846", "name": "Grüne Linie", "config": library::config_json(&Crosshair { pip_gap: 6, pip_height: 14, pip_width: 1, pip_outline_border: 0, pip_outline_opacity: 0.0, ..Crosshair::default() }), "uses": 1, "official": false, "created_day": "2026-11-25"},
            {"id": "e40c6a1b93d75f28", "name": "Boxed Pink", "config": library::config_json(&Crosshair { pip_opacity: 0.0, pip_outline_opacity: 0.0, dot_size: 4, dot_opacity: 1.0, dot_outline_border: 1, dot_outline_gap: 3, dot_outline_opacity: 1.0, color_r: 255, color_g: 90, color_b: 200, outline_color_r: 255, outline_color_g: 90, outline_color_b: 200, ..Crosshair::default() }), "uses": 0, "official": false, "created_day": "2026-11-25"},
        ],
        "next_cursor": "eyJzIjoidXNlZCJ9",
    });
    library::parse_page(body.to_string().as_bytes()).map(|page| page.rows).unwrap_or_default()
}

/// `crosshair-community`: the Community view with a page of rows, one loaded into the editor.
pub fn scene_community(s: &mut State) {
    crate::app::scene_crosshair(s);
    s.crosshair.log = None;
    s.crosshair.presets_loaded = true;
    s.crosshair.shelf = Shelf::Community;
    s.crosshair.community.store_loaded = true;
    s.crosshair.community.cache.insert(
        Sort::Used,
        Listing {
            rows: scene_rows(),
            next_cursor: Some("eyJzIjoidXNlZCJ9".to_string()),
            at: None,
        },
    );
    load(s, "5f2a9c01d4e7b3a8");
}

/// `crosshair-publish`: the Publish dialog for a saved preset, its name prefilled.
pub fn scene_publish(s: &mut State) {
    super::scene_presets(s);
    s.crosshair.community.store_loaded = true;
    if let Some(id) = s.crosshair.presets.first().map(|p| p.id.clone()) {
        open_publish(s, &id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> State {
        let mut st = State::default();
        st.screen = crate::model::Screen::Crosshair;
        st
    }

    #[test]
    fn counts_fit_a_tile() {
        assert_eq!(count(377), "377");
        assert_eq!(count(1284), "1.3k");
        assert_eq!(count(1000), "1k");
        assert_eq!(count(12_400), "12k");
        assert_eq!(count(2_000_000), "2M");
    }

    #[test]
    fn nothing_is_fetched_before_the_community_view_opens() {
        let mut st = state();
        poll(&mut st);
        assert!(!st.crosshair.community.fetch.is_running());
        assert!(st.crosshair.community.cache.is_empty());
        assert!(st.crosshair.community.failed.is_none());
    }

    #[test]
    fn loading_a_library_crosshair_does_not_write_to_the_game() {
        let root = std::env::temp_dir().join(format!("dah-xhair-community-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let mut st = state();
        st.game_root = Some(root.clone());
        scene_community(&mut st);
        st.game_root = Some(root.clone());
        st.crosshair.settings.disable_hero_specific_crosshairs = true;
        let before = st.crosshair.applied.clone();
        load(&mut st, "sigil-red-dot");
        assert_eq!(st.crosshair.settings.color_r, 255);
        assert!(st.crosshair.settings.disable_hero_specific_crosshairs, "the player's toggle stays");
        assert_eq!(st.crosshair.applied, before);
        assert!(!st.crosshair.pending);
        let written: Vec<_> = walk(&root);
        assert!(written.is_empty(), "{written:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walk(&path));
            } else {
                out.push(path);
            }
        }
        out
    }

    #[test]
    fn use_is_sent_only_for_an_unchanged_library_crosshair() {
        let mut st = state();
        scene_community(&mut st);
        load(&mut st, "sigil-red-dot");
        assert_eq!(use_to_send(&st).as_deref(), Some("sigil-red-dot"));
        after_apply(&mut st);
        assert_eq!(st.crosshair.community.last_use.as_deref(), Some("sigil-red-dot"));
        // A second Apply of the same load sends nothing more.
        st.crosshair.community.last_use = None;
        after_apply(&mut st);
        assert_eq!(st.crosshair.community.last_use, None);

        // Changed after loading: no use.
        load(&mut st, "sigil-wide-gap");
        st.crosshair.settings.pip_gap += 1;
        after_apply(&mut st);
        assert_eq!(st.crosshair.community.last_use, None);

        // A preset load in between: no use.
        load(&mut st, "sigil-wide-gap");
        super::super::load_builtin(&mut st, "dot");
        after_apply(&mut st);
        assert_eq!(st.crosshair.community.last_use, None);

        // The hero toggle is not part of the look.
        load(&mut st, "sigil-wide-gap");
        st.crosshair.settings.disable_hero_specific_crosshairs = !st.crosshair.settings.disable_hero_specific_crosshairs;
        after_apply(&mut st);
        assert_eq!(st.crosshair.community.last_use.as_deref(), Some("sigil-wide-gap"));
    }

    #[test]
    fn a_reported_tile_vanishes_at_once() {
        let mut st = state();
        scene_community(&mut st);
        assert!(visible(&st).iter().any(|row| row.id == "8c1e04f7a29b6d53"));
        open_report(&mut st, "8c1e04f7a29b6d53".to_string(), "Dr. Aim's Plus".to_string());
        confirm_report(&mut st);
        assert!(!visible(&st).iter().any(|row| row.id == "8c1e04f7a29b6d53"));
        assert!(st.crosshair.community.report.is_none());
    }

    #[test]
    fn publish_answers_update_the_page() {
        let mut st = state();
        scene_publish(&mut st);
        let preset = st.crosshair.presets[0].id.clone();
        // A refused name stays in the dialog with a plain message.
        finish_publish(&mut st, preset.clone(), Err(LibraryError::BlockedName));
        assert!(matches!(&st.crosshair.preset_dialog, Some(PresetDialog::Publish { error: Some(e), .. }) if e.contains("not allowed")));
        finish_publish(&mut st, preset.clone(), Err(LibraryError::PublishLimit));
        assert!(matches!(&st.crosshair.preset_dialog, Some(PresetDialog::Publish { error: Some(e), .. }) if e.contains("3 crosshairs a day")));
        // A duplicate closes the dialog and marks the existing one as found.
        finish_publish(&mut st, preset.clone(), Err(LibraryError::Duplicate("sigil-dot".to_string())));
        assert!(st.crosshair.preset_dialog.is_none());
        assert_eq!(st.crosshair.community.found.as_deref(), Some("sigil-dot"));
        assert!(published(&st, &preset).is_none());
        // Success keeps the token and offers Remove.
        let entry = Published {
            id: "00112233aabbccdd".to_string(),
            delete_token: library::DeleteToken::parse("AbCdEfGhIjKlMnOpQrStUvWxYz0123456789-_abcde").unwrap(),
        };
        open_publish(&mut st, &preset);
        finish_publish(&mut st, preset.clone(), Ok(entry));
        assert!(published(&st, &preset).is_some());
        assert!(st.crosshair.preset_dialog.is_none());
        remove(&mut st, &preset);
        assert!(published(&st, &preset).is_none());
    }

    #[test]
    fn a_bad_name_is_refused_before_sending() {
        let mut st = state();
        scene_publish(&mut st);
        if let Some(PresetDialog::Publish { name, .. }) = st.crosshair.preset_dialog.as_mut() {
            *name = TextEdit::new("visit x.com");
        }
        submit_publish(&mut st);
        assert!(!st.crosshair.community.publish.is_running());
        assert!(matches!(&st.crosshair.preset_dialog, Some(PresetDialog::Publish { error: Some(_), .. })));
    }

    #[test]
    fn a_failed_fetch_keeps_the_cached_list() {
        let mut st = state();
        scene_community(&mut st);
        let before = visible(&st).len();
        st.crosshair.community.fetch.start(None, || {
            Ok(Fetched { sort: Sort::Used, append: false, page: Err(LibraryError::Failed("bad body".to_string())) })
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while st.crosshair.community.fetch.is_running() && Instant::now() < deadline {
            if poll(&mut st) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(visible(&st).len(), before);
        assert!(st.crosshair.community.failed.is_some());
    }
}
