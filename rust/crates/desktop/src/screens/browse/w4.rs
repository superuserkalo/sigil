//! Browse additions from parity wave 4 (UI B), DMM and Grimoire side by side:
//!
//! - Random hero on the "Browse by hero" rail (DMM's dice).
//! - Featured mod per category on the landing (DMM's weekly pick, from the saved catalog).
//! - Creator links in the details (both apps read the member's `_aContactInfo`).
//! - The rate-limit notice (Grimoire's toast and its load-failure text).
//! - Image menu on previews: copy the picture, copy its address, open it (Grimoire).
//! - Installed copies in the details, each with Delete (Grimoire's per-file trash).
//! - A resizable docked details panel (Grimoire: drag its left edge, kept across launches).

use std::collections::HashMap;
use std::time::Instant;

use deadlock_core::catalog_mirror::MirrorFilter;
use deadlock_core::gamebanana::BrowseMod;
use deadlock_core::library_facts::{self, AuthorProfile};

use super::*;
use crate::screens::settings::i18n::{fill, tr};
use crate::screens::settings::options;

/// Lucide `dices` (ISC), for the random hero button.
pub static DICE: Icon = Icon {
    name: "sigil-dices",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><rect width="12" height="12" x="2" y="10" rx="2" ry="2"/><path d="m17.92 14 3.5-3.5a2.24 2.24 0 0 0 0-3l-5-4.92a2.24 2.24 0 0 0-3 0L10 6"/><path d="M6 18h.01"/><path d="M10 14h.01"/><path d="M15 6h.01"/><path d="M18 9h.01"/></g>"#,
};

/// The docked panel's width range (Grimoire: 320 at least, the grid keeps 360).
pub const DOCK_MIN: f32 = 320.0;
pub const DOCK_MAX: f32 = 900.0;
/// Grimoire shows its rate-limit toast at most once per 10 seconds.
const RATE_TOAST_GAP: f32 = 10.0;

#[derive(Default)]
pub struct W4 {
    /// The docked panel's width; 0 keeps the default.
    pub dock_w: f32,
    /// The width when a resize drag began.
    dock_from: f32,
    /// The last random hero, so the next roll picks another.
    pub last_random: Option<String>,
    /// This week's featured mods, per category, from the saved catalog.
    pub featured: Vec<(String, BrowseCard)>,
    featured_key: Option<((usize, usize, i64), (i32, u32))>,
    /// Featured picks checked against their mod page this run (DMM `6bdb2970`): the saved
    /// catalog keeps mods GameBanana has since withheld, so a pick is confirmed before it
    /// stays. `true` when the page said the mod is hidden.
    featured_checked: HashMap<u64, bool>,
    featured_job: Job<Vec<(u64, bool)>>,
    /// Creator links by member id; a failed read is kept so it is not retried every frame.
    pub authors: HashMap<u64, Result<AuthorProfile, String>>,
    pub author_job: Job<(u64, AuthorProfile)>,
    author_asked: Option<u64>,
    /// The image menu: the element it hangs from and the picture's address.
    pub image_menu: Option<(String, String)>,
    /// Creator pages read this run (`creator_page.rs`).
    pub creator: super::creator_page::CreatorState,
    pub copy_job: Job<Vec<u8>>,
    /// The rate-limit toast was last shown.
    rate_toast: Option<Instant>,
    rate_seen: Option<String>,
    loaded: bool,
    /// Full-size view zoom (1 to 4) and pan in points, and the picture box size the last drag
    /// measured, to keep the pan inside the picture.
    pub zoom: f32,
    pub pan: (f32, f32),
    pan_from: (f32, f32),
    frame: (f32, f32),
}

// --- docked panel width ----------------------------------------------------------------------

pub fn dock_width(s: &State, default: f32) -> f32 {
    let w = s.browse_view.w4.dock_w;
    if w > 0.0 { w.clamp(DOCK_MIN, DOCK_MAX) } else { default }
}

/// The 6 point handle along the docked panel's left edge.
pub fn dock_handle(s: &State, u: &Ui, default: f32) -> El {
    let current = dock_width(s, default);
    el().key("browse-dock-resize")
        .w(6.0)
        .h_full()
        .pin(Some(-3.0), Some(0.0), None, Some(0.0))
        .hover(u.accent.alpha(0.5))
        .role("separator")
        .label(tr("Resize details sidebar"))
        .help(tr("Drag to resize"))
        .cursor(rune::Cursor::ResizeColumn)
        .on_drag(move |st: &mut State, d: &mut rune::Drag, _tx: &mut Tx| {
            match d.phase {
                rune::Phase::Down => st.browse_view.w4.dock_from = current,
                rune::Phase::Move => {
                    st.browse_view.w4.dock_w = (st.browse_view.w4.dock_from - d.dx).clamp(DOCK_MIN, DOCK_MAX);
                }
                rune::Phase::Up => {
                    let w = st.browse_view.w4.dock_w.round() as u64;
                    options::edit(st, |o| {
                        o.counters.insert("browse_dock_w".into(), w);
                    });
                }
            }
        })
}

/// Read the saved width once.
fn load(st: &mut State) {
    if st.browse_view.w4.loaded {
        return;
    }
    st.browse_view.w4.loaded = true;
    if let Some(w) = options::get().counters.get("browse_dock_w") {
        st.browse_view.w4.dock_w = *w as f32;
    }
}

// --- zoom and pan in the full-size view -----------------------------------------------------

/// The furthest zoom (Grimoire and DMM zoom with the wheel; Grimoire pans while zoomed).
pub const ZOOM_MAX: f32 = 4.0;

/// The zoom in effect, 1 when unset.
pub fn zoom(s: &State) -> f32 {
    let z = s.browse_view.w4.zoom;
    if z >= 1.0 { z.min(ZOOM_MAX) } else { 1.0 }
}

/// A pan kept inside the zoomed picture: it may move at most half the extra size each way.
pub fn clamp_pan(pan: (f32, f32), zoom: f32, frame: (f32, f32)) -> (f32, f32) {
    let max_x = frame.0 * (zoom - 1.0) / 2.0;
    let max_y = frame.1 * (zoom - 1.0) / 2.0;
    (pan.0.clamp(-max_x, max_x), pan.1.clamp(-max_y, max_y))
}

pub fn set_zoom(st: &mut State, zoom: f32) {
    let w = &mut st.browse_view.w4;
    w.zoom = zoom.clamp(1.0, ZOOM_MAX);
    w.pan = if w.zoom <= 1.0 { (0.0, 0.0) } else { clamp_pan(w.pan, w.zoom, w.frame) };
}

/// A new picture or a closed view starts unzoomed.
pub fn reset_zoom(st: &mut State) {
    st.browse_view.w4.zoom = 1.0;
    st.browse_view.w4.pan = (0.0, 0.0);
}

/// Wheel zooms (DMM's scroll to zoom), a pinch zooms, a drag pans while zoomed, a double click
/// switches between 1x and 2x.
pub fn zoomable(frame: El) -> El {
    frame
        .on_wheel(|st: &mut State, e: rune::WheelEvent, _tx: &mut Tx| {
            let z = zoom(st);
            set_zoom(st, z * (-e.dy * 0.004).exp());
            true
        })
        .on_magnify(|st: &mut State, e: rune::MagnifyEvent, _tx: &mut Tx| {
            let z = zoom(st);
            set_zoom(st, z * (1.0 + e.delta));
            true
        })
        .on_drag(|st: &mut State, d: &mut rune::Drag, _tx: &mut Tx| {
            let w = &mut st.browse_view.w4;
            w.frame = (d.w, d.h);
            match d.phase {
                rune::Phase::Down => {
                    w.pan_from = w.pan;
                    if d.clicks == 2 {
                        let next = if w.zoom > 1.0 { 1.0 } else { 2.0 };
                        set_zoom(st, next);
                    }
                }
                rune::Phase::Move if w.zoom > 1.0 => {
                    w.pan = clamp_pan((w.pan_from.0 + d.dx, w.pan_from.1 + d.dy), w.zoom, w.frame);
                }
                _ => {}
            }
        })
}

/// The zoom controls on the picture: out, the level (press to reset), in.
pub fn zoom_bar(s: &State, u: &Ui) -> El {
    let z = zoom(s);
    let round = |key: &str, glyph: &'static Icon, label: &str| {
        el().key(key.to_string())
            .size(28.0, 28.0)
            .center()
            .radius(14.0)
            .hover(Color::WHITE.alpha(0.12))
            .focusable(true)
            .role("button")
            .label(tr(label).to_string())
            .cursor(rune::Cursor::Pointer)
            .child(icon(glyph, 14.0).color(u.ink).a11y_hidden())
    };
    row()
        .key("lightbox-zoom")
        .h(32.0)
        .px(2.0)
        .gap(2.0)
        .center_y()
        .radius(crate::ui::r(16.0))
        .fill(Color::BLACK.alpha(0.6))
        .border(1.0, Color::WHITE.alpha(0.15))
        .child(round("lightbox-zoom-out", &icons::MINUS, "Zoom out").on_press(|st: &mut State| {
            let z = zoom(st);
            set_zoom(st, z / 1.5)
        }))
        .child(
            el().key("lightbox-zoom-level")
                .h(28.0)
                .px(6.0)
                .center()
                .radius(crate::ui::r(14.0))
                .hover(Color::WHITE.alpha(0.12))
                .role("button")
                .label(tr("Actual size"))
                .cursor(rune::Cursor::Pointer)
                .on_press(reset_zoom)
                .child(text(format!("{:.0}%", z * 100.0)).font(SMALL).color(u.prose).a11y_hidden()),
        )
        .child(round("lightbox-zoom-in", &icons::PLUS, "Zoom in").on_press(|st: &mut State| {
            let z = zoom(st);
            set_zoom(st, z * 1.5)
        }))
}

// --- random hero ----------------------------------------------------------------------------

/// A hero from the rail at random, never the last one picked (DMM's dice, without the spin).
pub fn random_hero(heroes: &[String], last: Option<&str>, seed: u64) -> Option<String> {
    let pool: Vec<&String> = heroes.iter().filter(|h| Some(h.as_str()) != last).collect();
    let pool = if pool.is_empty() { heroes.iter().collect() } else { pool };
    if pool.is_empty() {
        return None;
    }
    Some(pool[(seed % pool.len() as u64) as usize].clone())
}

pub fn random_button(u: &Ui) -> El {
    row()
        .key("browse-hero-random")
        .h(24.0)
        .px(8.0)
        .gap(6.0)
        .center_y()
        .radius(crate::ui::r(4.0))
        .hover(u.hover)
        .press(u.over.alpha(0.08))
        .focusable(true)
        .role("button")
        .label(tr("Random"))
        .help(tr("Open a random hero's skins"))
        .cursor(rune::Cursor::Pointer)
        .child(icon(&DICE, 14.0).color(u.muted).a11y_hidden())
        .child(text(tr("Random")).font(crate::ui::LABEL).color(u.muted).a11y_hidden())
        .on_press_tx(|st: &mut State, tx: &mut Tx| {
            let heroes: Vec<String> = hero_list(st).into_iter().map(|h| h.name).collect();
            let last = st.browse_view.w4.last_random.clone();
            if let Some(hero) = random_hero(&heroes, last.as_deref(), library_facts::random_seed()) {
                st.browse_view.w4.last_random = Some(hero.clone());
                open_skins_for(st, tx, &hero);
            }
            // Pressed in the Filters menu: the grid it opened is the answer, so the menu closes.
            st.browse_view.menu = None;
            st.browse_view.from_filters = false;
        })
}

// --- featured per category ------------------------------------------------------------------

fn browse_row(entry: &deadlock_core::catalog_mirror::MirrorEntry) -> BrowseMod {
    BrowseMod {
        id: entry.id,
        name: entry.name.clone(),
        profile_url: entry.profile_url.clone(),
        model: entry.model.clone(),
        author: entry.author.clone(),
        category: entry.category.clone(),
        subcategory: entry.subcategory.clone(),
        sub_category_id: entry.sub_category_id,
        version: None,
        updated_at: String::new(),
        likes: entry.likes,
        views: entry.views,
        has_files: true,
        thumbnail_url: entry.thumbnail_url.clone(),
        added_ts: entry.added_ts,
        updated_ts: entry.updated_ts,
        nsfw: entry.nsfw,
        audio_url: entry.audio_url.clone(),
        featured: entry.featured,
        comments: entry.comments,
        author_id: entry.author_id,
        obsolete: entry.obsolete,
    }
}

/// Recompute this week's picks when the saved catalog or the week changed, then confirm each
/// new pick against its GameBanana page (DMM: withheld mods leave Featured).
pub fn refresh_featured(st: &mut State) -> bool {
    load(st);
    let mut changed = take_featured_checks(st);
    changed |= recompute_featured(st);
    confirm_featured(st);
    changed
}

/// Picks checked per run at most, so a catalog full of withheld mods cannot keep Sigil asking.
const FEATURED_CHECKS: usize = 24;

/// Recompute the picks on the next frame: a mod left the saved catalog.
pub fn forget_featured(st: &mut State) {
    st.browse_view.w4.featured_key = None;
}

/// Drop the picks GameBanana no longer shows from the saved catalog; the category's next
/// candidate takes the place on the next recompute. A failed check keeps the pick: offline
/// says nothing about the mod.
fn take_featured_checks(st: &mut State) -> bool {
    let Some(result) = st.browse_view.w4.featured_job.try_take() else {
        return false;
    };
    let Ok(checked) = result else { return false };
    let mut dropped = false;
    for (id, hidden) in checked {
        st.browse_view.w4.featured_checked.insert(id, hidden);
        if hidden && let Some(mirror) = st.browse_view.mirror.as_mut() && mirror.remove("Mod", id) {
            dropped = true;
        }
    }
    if dropped {
        st.browse_view.mirror_dirty = true;
        forget_featured(st);
        super::store::start_mirror_save(st);
    }
    dropped
}

/// Read the mod page of each pick not checked yet, on a worker. GameBanana only.
fn confirm_featured(st: &mut State) {
    if st.demo || rune::is_headless() || st.prefs.offline || st.browse_view.w4.featured_job.is_running() {
        return;
    }
    let checked = &st.browse_view.w4.featured_checked;
    if checked.len() >= FEATURED_CHECKS {
        return;
    }
    let ids: Vec<u64> = st
        .browse_view
        .w4
        .featured
        .iter()
        .map(|(_, card)| card.id)
        .filter(|id| !checked.contains_key(id))
        .take(FEATURED_CHECKS - checked.len())
        .collect();
    if ids.is_empty() {
        return;
    }
    // Mark them now so a failed read is not asked again every frame.
    for id in &ids {
        st.browse_view.w4.featured_checked.insert(*id, false);
    }
    let wake = st.wake.clone();
    st.browse_view.w4.featured_job.start(wake, move || {
        Ok(ids
            .into_iter()
            .filter_map(|id| {
                gamebanana::fetch_profile(BrowseSection::Mods, id)
                    .ok()
                    .map(|profile| (id, profile.is_hidden()))
            })
            .collect())
    });
}

/// This week's picks from the saved catalog, when it or the week changed.
fn recompute_featured(st: &mut State) -> bool {
    let Some(mirror) = st.browse_view.mirror.as_ref() else {
        return false;
    };
    let now = chrono_now();
    // This runs on every poll: key on what is cheap to read, and sort the catalog only when
    // it or the week changed (sorting and copying up to 4000 entries cost milliseconds a frame).
    let catalog = (mirror.entries.len(), mirror.meta.withheld.len(), mirror.meta.synced_at);
    let key = (catalog, library_facts::iso_week(now));
    if st.browse_view.w4.featured_key == Some(key) {
        return false;
    }
    let page = mirror.page(&MirrorFilter::section(BrowseSection::Mods), BrowseOrder::MostLiked, 1, 4000);
    let rows: Vec<BrowseMod> = page.entries.iter().map(browse_row).collect();
    let picks = library_facts::featured(&rows, now, 0);
    st.browse_view.w4.featured = picks
        .into_iter()
        .map(|(category, row)| (category, card_from_browse(&row)))
        .collect();
    st.browse_view.w4.featured_key = Some(key);
    true
}

fn chrono_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// One tile per category with this week's pick; a sound pick has its play button.
// --- creator links --------------------------------------------------------------------------

/// Read the open mod's creator links once per member, on a worker.
pub fn poll_author(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.browse_view.w4.author_job.try_take() {
        let asked = st.browse_view.w4.author_asked.take();
        match result {
            Ok((id, profile)) => {
                st.browse_view.w4.authors.insert(id, Ok(profile));
            }
            Err(error) => {
                if let Some(id) = asked {
                    st.browse_view.w4.authors.insert(id, Err(error));
                }
            }
        }
        changed = true;
    }
    let member = st
        .browse_view
        .details
        .as_ref()
        .and_then(|d| d.profile.as_ref())
        .and_then(|p| p.submitter.as_ref())
        .map(|m| m.id)
        .filter(|id| *id > 0);
    if let Some(id) = member
        && !st.browse_view.w4.authors.contains_key(&id)
        && !st.browse_view.w4.author_job.is_running()
        && !st.prefs.offline
        && !rune::is_headless()
    {
        st.browse_view.w4.author_asked = Some(id);
        let wake = st.wake.clone();
        st.browse_view.w4.author_job.start(wake, move || {
            library_facts::fetch_author_profile(id).map(|p| (id, p)).map_err(|e| e.to_string())
        });
    }
    changed |= super::creator_page::poll(st);
    changed
}

/// Chips for the creator's contacts under their name: a link opens, a Discord user name copies.
pub fn author_links(s: &State, u: &Ui, member_id: u64) -> Option<El> {
    let Some(Ok(profile)) = s.browse_view.w4.authors.get(&member_id) else {
        return None;
    };
    if profile.contacts.is_empty() {
        return None;
    }
    let mut r = row().key("details-author-links").w_full().gap(6.0).mt(4.0).flex_wrap();
    for (index, contact) in profile.contacts.iter().enumerate() {
        let label = contact.kind.label();
        let value = contact.value.clone();
        let chip = row()
            .key(format!("details-author-link-{index}"))
            .h(26.0)
            .px(10.0)
            .gap(6.0)
            .center_y()
            .radius(crate::ui::r(13.0))
            .fill(u.over.alpha(0.05))
            .border(1.0, u.hair_strong)
            .hover(u.hover)
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .child(text(label).font(SMALL).color(u.prose));
        r = r.child(match contact.url.clone() {
            Some(url) => chip
                .role("link")
                .label(fill(tr("{} on {}"), &[&profile.name, label]))
                .help(value.clone())
                .child(icon(&icons::EXTERNAL_LINK, 11.0).color(u.faint).a11y_hidden())
                .on_press_tx(move |st: &mut State, tx: &mut Tx| open_url(st, tx, &url)),
            None => chip
                .role("button")
                .label(fill(tr("Copy the Discord user name {}"), &[&value]))
                .help(tr("Copy the Discord user name"))
                .child(icon(&icons::COPY, 11.0).color(u.faint).a11y_hidden())
                .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                    tx.copy(value.clone());
                    crate::notify::push(st, crate::notify::Notice::info(fill(tr("Copied {} to the clipboard"), &[&value])));
                }),
        });
    }
    Some(r)
}

// --- rate limit -----------------------------------------------------------------------------

/// GameBanana answered 429 (the core retries once, then reports the status).
pub fn is_rate_limit(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("http 429") || lower.contains("status: 429") || lower.contains("too many requests")
}

/// Grimoire's toast, at most once per 10 seconds, when a Browse read hits the limit.
pub fn poll_rate_limit(st: &mut State) -> bool {
    let error = st.browse.error.clone().filter(|e| is_rate_limit(e));
    if error == st.browse_view.w4.rate_seen {
        return false;
    }
    st.browse_view.w4.rate_seen = error.clone();
    if error.is_none() {
        return false;
    }
    if st.browse_view.w4.rate_toast.is_some_and(|at| at.elapsed().as_secs_f32() < RATE_TOAST_GAP) {
        return false;
    }
    st.browse_view.w4.rate_toast = Some(Instant::now());
    crate::notify::push(
        st,
        crate::notify::Notice::warning(tr("GameBanana is rate-limiting Sigil"))
            .detail(tr("Pause a moment before retrying.")),
    );
    true
}

// --- image menu -----------------------------------------------------------------------------

/// Right-click on a preview: open the image menu under it.
pub fn open_image_menu(st: &mut State, anchor: String, url: String) {
    st.browse_view.w4.image_menu = Some((anchor, url));
}

/// Decode the cached picture and hand the clipboard a PNG, on a worker.
fn copy_image(st: &mut State, url: String) {
    let wake = st.wake.clone();
    st.browse_view.w4.copy_job.start(wake, move || {
        let bytes = crate::data::fetch_thumb_bytes(&url).ok_or_else(|| tr("The picture is not downloaded yet.").to_string())?;
        let decoded = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
        let mut png = std::io::Cursor::new(Vec::new());
        decoded.write_to(&mut png, image::ImageFormat::Png).map_err(|e| e.to_string())?;
        Ok(png.into_inner())
    });
}

/// Take a finished copy and put it on the clipboard.
pub fn poll_copy(st: &mut State, tx: &mut Tx) -> bool {
    let Some(result) = st.browse_view.w4.copy_job.try_take() else {
        return false;
    };
    match result.and_then(|png| rune::ClipboardImage::from_png(png).ok_or_else(|| tr("The picture is too large to copy.").to_string())) {
        Ok(image) => {
            tx.copy_rich(rune::ClipboardContent::default().with_image(image));
            crate::notify::push(st, crate::notify::Notice::info(tr("Image copied")));
        }
        Err(error) => crate::notify::push(st, crate::notify::Notice::warning(tr("Copy image failed")).detail(error)),
    }
    true
}

pub fn image_menu(s: &State, u: &Ui) -> Option<El> {
    let (anchor, url) = s.browse_view.w4.image_menu.clone()?;
    let close = |st: &mut State| st.browse_view.w4.image_menu = None;
    let item = |key: &str, glyph: &'static Icon, label: &str| {
        row()
            .key(key.to_string())
            .w_full()
            .h(30.0)
            .px(10.0)
            .gap(10.0)
            .center_y()
            .radius(crate::ui::r(6.0))
            .hover(u.hover)
            .focusable(true)
            .role("menuitem")
            .label(tr(label).to_string())
            .cursor(rune::Cursor::Pointer)
            .child(icon(glyph, 14.0).color(u.muted).a11y_hidden())
            .child(text(tr(label).to_string()).font(BODY).color(u.ink).a11y_hidden())
    };
    let (copy_url, open_url_value, copy_address) = (url.clone(), url.clone(), url);
    Some(
        col()
            .key("browse-image-menu")
            .role("menu")
            .w(220.0)
            .padding(4.0)
            .fill(u.raised)
            .radius(crate::ui::r(10.0))
            .border(1.0, u.hair_strong)
            .shadow(12.0, 32.0, crate::ui::soft(Color::BLACK.alpha(0.5)))
            .anchored(anchor, Side::Bottom, Align::Start, 6.0)
            .transition(crate::motion::pop(false))
            .modal(true, close)
            .on_key(|st: &mut State, ev: &KeyEvent, _| {
                if ev.key == Key::Escape {
                    st.browse_view.w4.image_menu = None;
                    true
                } else {
                    false
                }
            })
            .child(
                item("image-menu-copy", &icons::COPY, "Copy image").on_press(move |st: &mut State| {
                    st.browse_view.w4.image_menu = None;
                    copy_image(st, copy_url.clone());
                }),
            )
            .child(
                item("image-menu-address", &icons::COPY, "Copy image address").on_press_tx(
                    move |st: &mut State, tx: &mut Tx| {
                        st.browse_view.w4.image_menu = None;
                        tx.copy(copy_address.clone());
                        crate::notify::push(st, crate::notify::Notice::info(tr("Address copied")));
                    },
                ),
            )
            .child(el().w_full().h(1.0).my(4.0).fill(u.hair))
            .child(
                item("image-menu-open", &icons::EXTERNAL_LINK, "Open image").on_press_tx(
                    move |st: &mut State, tx: &mut Tx| {
                        st.browse_view.w4.image_menu = None;
                        open_url(st, tx, &open_url_value);
                    },
                ),
            ),
    )
}

// --- installed copies in the details --------------------------------------------------------

/// "Installed" rows above the files: each slot that holds this mod, with Delete (Grimoire's
/// trash on the file row). Delete goes through Installed's own confirm and trash, with Undo.
pub fn installed_copies(s: &State, u: &Ui, d: &Details) -> Option<El> {
    if d.section != BrowseSection::Mods {
        return None;
    }
    let slots = super::replace::candidates(s, d.id);
    if slots.is_empty() {
        return None;
    }
    let dir = s.addons_dir.as_deref().map(std::path::Path::new);
    let mut c = col().key("details-installed-copies").w_full().gap(6.0).child(
        text(tr("Installed")).font(BODY_MED).color(u.muted),
    );
    for slot in slots {
        let Some(addon) = s.addons.addons.iter().find(|a| a.slot == slot) else {
            continue;
        };
        let file = dir
            .map(|dir| addon.path(dir))
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default();
        let name = d.name.clone();
        c = c.child(
            row()
                .key(format!("details-installed-{slot}"))
                .w_full()
                .h(40.0)
                .px(12.0)
                .gap(10.0)
                .center_y()
                .radius(crate::ui::r(8.0))
                .border(1.0, u.hair)
                .child(text(deadlock_core::addons::slot_label(slot)).font(LABEL).color(u.faint).w(36.0))
                .child(text(file).font(SMALL).color(u.prose).truncate().grow(1.0).min_w(0.0))
                .child(
                    text(tr(if addon.enabled { "On" } else { "Off" }))
                        .font(SMALL)
                        .color(if addon.enabled { u.teal } else { u.faint }),
                )
                .child(
                    u.icon_button(&icons::TRASH_2, tr("Delete"))
                        .key(format!("details-installed-delete-{slot}"))
                        .label(fill(tr("Delete {} from your installed mods"), &[&name]))
                        .help(tr("Delete installed file"))
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| {
                            crate::screens::installed::ask(st, crate::screens::installed::Confirm::Delete(vec![slot]))
                        }),
                ),
        );
    }
    Some(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_random_hero_is_never_the_last_one() {
        let heroes = vec!["Haze".to_string(), "Abrams".to_string(), "Paige".to_string()];
        for seed in 0..30 {
            let pick = random_hero(&heroes, Some("Haze"), seed).unwrap();
            assert_ne!(pick, "Haze");
        }
        assert_eq!(random_hero(&["Haze".to_string()], Some("Haze"), 4).as_deref(), Some("Haze"));
        assert_eq!(random_hero(&[], None, 1), None);
    }

    #[test]
    fn the_pan_stays_inside_the_zoomed_picture() {
        assert_eq!(clamp_pan((500.0, -500.0), 2.0, (400.0, 300.0)), (200.0, -150.0));
        assert_eq!(clamp_pan((30.0, 10.0), 1.0, (400.0, 300.0)), (0.0, 0.0));
        assert_eq!(clamp_pan((30.0, 10.0), 3.0, (400.0, 300.0)), (30.0, 10.0));
    }

    #[test]
    fn rate_limits_are_told_apart() {
        assert!(is_rate_limit("GameBanana request to https://x failed: http status: 429"));
        assert!(is_rate_limit("GameBanana returned HTTP 429 for https://x."));
        assert!(!is_rate_limit("GameBanana returned HTTP 500 for https://x."));
    }
}
