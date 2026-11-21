//! Locker, ported from Grimoire's `Locker.tsx`, `LockerHero.tsx` and the General drill-in.
//!
//! Three pages: the grid (a General tile, then one 3:4 card per roster hero with its render
//! anchored on the face and the name plate at the bottom), a hero page (the render full height
//! on the right under a layered fade, a left rail with the name plate and sections, and the
//! section's content beside it), and the General page (a rail of General types and the user's
//! own categories, and the mods in the chosen one).
//!
//! `deadlock_core::locker` owns the roster, the art lookup, mod classification and the category
//! store. Art is read at runtime from the app's asset folder and never copied into the repo.

use deadlock_core::locker::{GeneralType, LockerCategory, LockerPrefs, LockerSound};
use rune::{
    Color, El, Pixels, ScrollSpec, SurfaceFit, TextEdit, col, el, oklch, row, spacer, text,
};

use crate::app::State;
use crate::icons;
use crate::model::Screen;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, MONO, SMALL, SMALL_NUM, Ui};

mod cards;
pub mod art;
// Parity wave 4, UI C: the game's own art and sounds (`assets.rs` holds the shared state).
pub mod assets;
pub mod card_art;
mod catalog;
pub mod game_sounds;
pub mod cursor_create;
pub mod cursors;
pub mod parts;
pub mod sync;
mod variants;

/// One roster hero as the screen draws it. `asset_count` is kept for later views.
#[derive(Clone)]
#[allow(dead_code)]
pub struct HeroCard {
    pub codename: String,
    pub display_name: String,
    /// Render and name art paths, the image cache's keys (`State::thumbnails`).
    pub render: Option<String>,
    pub name_art: Option<String>,
    /// Pixel size of the render and the name art, read from the file header so layout never
    /// waits for a decode.
    pub render_size: Option<(u32, u32)>,
    pub name_size: Option<(u32, u32)>,
    /// The card-sized copy of the render the grid draws (`data::card_copy`), and its size.
    pub card: Option<String>,
    pub card_size: Option<(u32, u32)>,
    /// A custom card image the user picked, and where its face sits.
    pub custom: Option<String>,
    pub custom_size: Option<(u32, u32)>,
    pub custom_focus: f32,
    /// Face position across the render, percent.
    pub face_x: f32,
    pub mod_count: usize,
    pub asset_count: usize,
    pub in_catalog: bool,
    pub recolor_paths: Vec<String>,
}

impl HeroCard {
    /// The art the grid draws: the custom card when there is one.
    pub fn art(&self) -> (Option<String>, Option<(u32, u32)>, f32) {
        match &self.custom {
            Some(path) => (Some(path.clone()), self.custom_size, self.custom_focus),
            None => (self.card.clone(), self.card_size, self.face_x),
        }
    }
}

/// One installed addon slot.
#[derive(Clone)]
pub struct ModRow {
    pub slot: u16,
    pub file: String,
    pub enabled: bool,
    /// Content identity, the category membership key.
    pub key: String,
    /// The install's name when Sigil installed it, else the slot.
    pub label: String,
    pub heroes: Vec<String>,
    pub general: Option<GeneralType>,
    /// Ability sounds this mod replaces.
    pub sounds: Vec<LockerSound>,
    /// What it changes per hero (C14): skin, weapon skin, voice lines, weapon and ability
    /// sounds, from its VPK paths (`locker_parts::hero_parts`, read with the Locker).
    pub parts: Parts,
}

/// Hero name to the parts a mod changes for that hero.
pub type Parts = std::collections::BTreeMap<String, std::collections::BTreeSet<deadlock_core::locker_parts::HeroPart>>;

/// A recolor brief in the workspace, for the applied changes review.
#[derive(Clone)]
pub struct BriefRow {
    pub hero: String,
    pub display_name: String,
    pub palette: String,
    pub note: String,
}

pub struct Load {
    pub heroes: Vec<HeroCard>,
    pub mods: Vec<ModRow>,
    pub categories: Vec<LockerCategory>,
    pub prefs: LockerPrefs,
    pub briefs: Vec<BriefRow>,
    pub global: deadlock_core::locker::GlobalPathStatus,
    /// The General backdrop's path.
    pub general_bg: Option<String>,
    pub general_bg_size: Option<(u32, u32)>,
    /// The backdrop's card copy, for the General tile.
    pub general_card: Option<String>,
    pub general_card_size: Option<(u32, u32)>,
    pub assets_dir: String,
    pub catalog_label: String,
    pub pinned: Option<String>,
    pub error: Option<String>,
}

pub struct Panel {
    pub codename: String,
    pub display_name: String,
    pub in_catalog: bool,
    pub targets: Vec<String>,
    pub palette: usize,
    pub note: TextEdit,
    pub log: Option<String>,
}

#[derive(Clone, PartialEq)]
pub enum Page {
    Grid,
    Hero(String),
    General,
}

#[derive(Clone, Copy, PartialEq)]
pub enum HeroSection {
    Mods,
    Sounds,
    /// Weapon skins, voice lines and weapon sounds (`parts.rs`).
    Parts,
    Recolor,
    /// The game's own textures (`catalog.rs`).
    GameArt,
    /// The game's own sounds and swaps (`game_sounds.rs`).
    GameSounds,
}

#[derive(Clone, PartialEq)]
pub enum GeneralTab {
    Type(GeneralType),
    Custom(String),
    /// Cursor packs (`cursors.rs`): loose files over the game's cursor folder, not VPKs.
    Cursors,
}

/// The New category and Rename dialog.
pub struct NameDialog {
    /// The category being renamed, or `None` for a new one.
    pub editing: Option<String>,
    /// A mod to file into the new category, as Grimoire's per-card "New category...".
    pub file_key: Option<String>,
    pub name: TextEdit,
    pub error: Option<String>,
}

/// The custom card picker. `path` is the chosen source, held until Save copies it.
pub struct CardDialog {
    pub hero: String,
    pub display_name: String,
    pub path: Option<String>,
    pub size: Option<(u32, u32)>,
    pub focus_x: f32,
    pub existing: bool,
    pub error: Option<String>,
}

#[allow(clippy::large_enum_variant)]
pub enum Dialog {
    Name(NameDialog),
    /// Add mods to the category with this id.
    Picker(String),
    Manage,
    /// The per-hero launch shuffle switches.
    Shuffle,
    /// What the Locker applied: cards, shuffle, sounds, colors.
    Applied,
    Card(CardDialog),
}

pub struct View {
    pub loaded: bool,
    pub heroes: Vec<HeroCard>,
    pub mods: Vec<ModRow>,
    pub categories: Vec<LockerCategory>,
    pub prefs: LockerPrefs,
    pub briefs: Vec<BriefRow>,
    pub global: deadlock_core::locker::GlobalPathStatus,
    pub general_bg: Option<String>,
    pub general_bg_size: Option<(u32, u32)>,
    pub general_card: Option<String>,
    pub general_card_size: Option<(u32, u32)>,
    pub assets_dir: String,
    pub catalog_label: String,
    pub error: Option<String>,
    pub pinned: Option<String>,
    pub modded_only: bool,
    pub starred_only: bool,
    pub query: TextEdit,
    pub query_text: String,
    /// The list view: full width hero rows, each expandable to its skins.
    pub list_view: bool,
    /// Hero codenames whose list row is open.
    pub expanded: std::collections::BTreeSet<String>,
    /// The slot whose "..." menu is open on the hero page.
    pub more: Option<u16>,
    pub page: Page,
    pub hero_section: HeroSection,
    pub general_tab: Option<GeneralTab>,
    /// Kept for the recolor scene: the hero the recolor dialog was opened for.
    pub selected: Option<String>,
    pub panel: Option<Panel>,
    pub dialog: Option<Dialog>,
    /// The content area's laid-out size, so cards can crop the render on the face.
    pub page_size: (f32, f32),
    pub notes_placeholder: TextEdit,
    /// Cursor packs (`cursors.rs`).
    pub cursors: cursors::Cursors,
    /// The addon folder the list last matched (`sync.rs`, issue #1).
    pub addons_sig: Option<sync::Sig>,
    /// Mod pictures already asked for (`art.rs`).
    pub art_asked: std::collections::HashSet<String>,
    /// The game's own art and sounds (`assets.rs`).
    pub assets: assets::Assets,
    /// The hero page shows the hero in 3D beside its sections (Grimoire's 3D toggle). Kept
    /// across heroes, as Grimoire's panel is.
    pub view3d: bool,
}

impl Default for View {
    fn default() -> Self {
        View {
            loaded: false,
            heroes: Vec::new(),
            mods: Vec::new(),
            categories: Vec::new(),
            prefs: LockerPrefs::default(),
            briefs: Vec::new(),
            global: deadlock_core::locker::GlobalPathStatus::default(),
            general_bg: None,
            general_bg_size: None,
            general_card: None,
            general_card_size: None,
            assets_dir: String::new(),
            catalog_label: String::new(),
            error: None,
            pinned: None,
            modded_only: false,
            starred_only: false,
            query: TextEdit::new(""),
            query_text: String::new(),
            list_view: false,
            expanded: std::collections::BTreeSet::new(),
            more: None,
            page: Page::Grid,
            hero_section: HeroSection::Mods,
            general_tab: None,
            selected: None,
            panel: None,
            dialog: None,
            page_size: (
                1360.0 - crate::ui::SIDEBAR_W,
                860.0 - crate::ui::TITLEBAR_H,
            ),
            notes_placeholder: TextEdit::new(""),
            cursors: cursors::Cursors::default(),
            addons_sig: None,
            art_asked: Default::default(),
            assets: Default::default(),
            view3d: false,
        }
    }
}

const GRID_GAP: f32 = 12.0;
const CARD_RADIUS: f32 = 8.0;
const CARD_MIN_W: f32 = 128.0;
const RAIL_W: f32 = 240.0;
const PANE_W: f32 = 480.0;
/// With the 3D view on the pane gives up width down to this, so the view keeps about
/// `VIEW3D_W`; below `VIEW3D_MIN_W` the view replaces the pane.
const PANE_MIN_W: f32 = 380.0;
const VIEW3D_W: f32 = 440.0;
const VIEW3D_MIN_W: f32 = 300.0;
/// How wide the left fade runs over the art on the hero and General pages.
const FADE_W: f32 = 1040.0;

/// Pixel size of a PNG, WebP or JPEG from its header. `None` for anything else. The reader
/// lives in core so the card picker and the grid agree on what an image is.
pub fn image_size(bytes: &[u8]) -> Option<(u32, u32)> {
    deadlock_core::locker::art_dimensions(bytes)
}

pub fn view(s: &State, u: &Ui) -> El {
    let v = &s.locker;
    let body = match &v.page {
        Page::Hero(name) => match v.heroes.iter().find(|hero| &hero.display_name == name) {
            Some(hero) => return measured(hero_page(s, u, v, hero)),
            None => grid_page(s, u, v),
        },
        Page::General if v.loaded => return measured(general_page(s, u, v)),
        _ => grid_page(s, u, v),
    };
    measured(body)
}

/// The page's size. Inside a view build it is the laid-out size, which Rune settles in the same
/// commit, so the first frame is already right; `View::page_size` (stored by the layout
/// handler, a frame late) is for code outside a build.
fn page_size(v: &View) -> (f32, f32) {
    rune::memo::layout_size("locker-root").map_or(v.page_size, |[w, h]| (w, h))
}

/// Wraps a page so its laid-out size lands in `View::page_size`.
fn measured(page: El) -> El {
    el().key("locker-root")
        .w_full()
        .h_full()
        .min_h(0.0)
        .on_layout(|st: &mut State, [_, _, w, h]: [f32; 4]| {
            if (st.locker.page_size.0 - w).abs() > 0.5 || (st.locker.page_size.1 - h).abs() > 0.5
            {
                st.locker.page_size = (w, h);
            }
        })
        .child(page)
}

// --- grid --------------------------------------------------------------------

fn sorted_heroes(v: &View) -> Vec<&HeroCard> {
    let pinned = v.pinned.as_deref();
    let query = v.query_text.trim().to_ascii_lowercase();
    let mut heroes: Vec<&HeroCard> = v
        .heroes
        .iter()
        .filter(|hero| !v.modded_only || hero.mod_count > 0)
        .filter(|hero| !v.starred_only || v.prefs.is_favorite(&hero.display_name))
        .filter(|hero| hero_name_matches(&hero.display_name, &hero.codename, &query))
        .collect();
    // The project hero leads the grid, then starred heroes, then the roster. Stable.
    heroes.sort_by_key(|hero| {
        (
            !pinned.is_some_and(|key| same_hero(hero, key)),
            !v.prefs.is_favorite(&hero.display_name),
        )
    });
    heroes
}

/// True when the hero search reaches a hero: its display name, its current codename, or any
/// older codename the game and community packs still use (`orion` finds Grey Talon).
fn hero_name_matches(name: &str, codename: &str, query: &str) -> bool {
    query.is_empty()
        || name.to_ascii_lowercase().contains(query)
        || codename.to_ascii_lowercase().contains(query)
        || deadlock_core::locker::roster_hero(name).is_some_and(|roster| {
            roster.codenames.iter().any(|code| code.contains(query))
        })
}

fn grid_columns(v: &View) -> (usize, f32) {
    let avail = (page_size(v).0 - 2.0 * crate::ui::PAGE_X).max(CARD_MIN_W);
    let cols = (((avail + GRID_GAP) / (CARD_MIN_W + GRID_GAP)).floor() as usize).clamp(3, 10);
    let card_w = (avail - GRID_GAP * (cols as f32 - 1.0)) / cols as f32;
    (cols, card_w.floor())
}

fn grid_page(s: &State, u: &Ui, v: &View) -> El {
    let shown = sorted_heroes(v);

    let search = rune::text_input(&v.query, BODY)
        .key("locker-search")
        .label("Search heroes")
        .single_line()
        .color(u.ink)
        .placeholder(crate::screens::settings::i18n::tr("Search heroes"), u.faint)
        .caret(u.accent, u.accent.alpha(0.20))
        .on_edit(|st: &mut State| {
            let text = st.locker.query.text.clone();
            st.locker.query_text = text;
            &mut st.locker.query
        })
        .on_input_key(|st: &mut State, ev: &rune::KeyEvent, _| {
            if ev.key == rune::Key::Escape {
                st.locker.query = TextEdit::new("");
                st.locker.query_text.clear();
                true
            } else {
                false
            }
        });

    // Every control is drawn from the first frame (disabled until the heroes load), so
    // nothing shifts when they arrive.
    let ready = v.loaded && !v.heroes.is_empty();
    let off = |e: El, off: bool| e.disabled(off).opacity(if off { 0.45 } else { 1.0 });
    let art_missing = ready && !v.heroes.iter().any(|hero| hero.render.is_some());
    let mut rescan = u
        .icon_button(&icons::REFRESH_CW, crate::screens::settings::i18n::tr("Rescan"))
        .key("locker-rescan")
        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| crate::data::start_locker_load(st));
    if art_missing {
        // Missing hero art: a quiet amber mark on Rescan, the reason in its tooltip.
        rescan = rescan.help("Hero art was not found next to Sigil").child(
            el().pin(None, Some(6.0), Some(6.0), None)
                .size(6.0, 6.0)
                .radius(2.0)
                .fill(u.amber)
                .a11y_hidden(),
        );
    }
    let actions = vec![
        off(view_toggle(u, v.list_view), !v.loaded),
        off(
            u.icon_button(&icons::ROTATE_CCW, crate::screens::settings::i18n::tr("Shuffle on launch"))
                .key("locker-shuffle")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| st.locker.dialog = Some(Dialog::Shuffle)),
            !v.loaded,
        ),
        off(
            u.icon_button(&icons::SLIDERS_HORIZONTAL, crate::screens::settings::i18n::tr("Locker changes"))
                .key("locker-applied")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| st.locker.dialog = Some(Dialog::Applied)),
            !v.loaded,
        ),
        rescan,
    ];
    let toolbar = u.page_toolbar(
        u.page_title(crate::screens::settings::i18n::tr("Locker")),
        vec![off(filter(u, v.modded_only, v.starred_only), !ready).ml(8.0)],
        Some(u.page_search(search)),
        actions,
    );

    let (cols, card_w) = grid_columns(v);
    let body = if v.heroes.is_empty() {
        if !v.loaded {
            loading(cols)
        } else if let Some(error) = &v.error {
            u.empty_state(
                "",
                "Heroes could not be loaded",
                error,
                Some(
                    row()
                        .gap(8.0)
                        .center_y()
                        .child(retry(u))
                        .child(crate::notify::feedback::button(u, "locker-error-feedback", "Heroes could not be loaded", error)),
                ),
            )
        } else {
            u.empty_state("", "No heroes yet", "Try again in a moment.", Some(retry(u)))
        }
    } else if shown.is_empty() {
        let (title, line) = if v.starred_only && v.query_text.trim().is_empty() {
            ("No starred heroes", "Star a hero to keep it at the top.")
        } else if v.modded_only && v.query_text.trim().is_empty() {
            ("No modded heroes", "None of your installed mods change a hero.")
        } else {
            ("No heroes match", "Try another name or clear the search.")
        };
        u.empty_state(
            "",
            title,
            line,
            Some(
                u.action("Show all heroes", ActionKind::Secondary)
                    .key("locker-show-all")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| {
                        st.locker.modded_only = false;
                        st.locker.starred_only = false;
                        st.locker.query = TextEdit::new("");
                        st.locker.query_text.clear();
                    }),
            ),
        )
    } else if v.list_view {
        list_body(s, u, v, &shown)
    } else {
        let mut grid = el().w_full().equal_columns(cols).gap(GRID_GAP);
        if !v.modded_only && !v.starred_only && v.query_text.trim().is_empty() {
            grid = grid.child(general_card(s, u, v, card_w));
        }
        for hero in &shown {
            grid = grid.child(hero_card(s, u, v, hero, card_w));
        }
        grid
    };

    col()
        .key("locker-scroll")
        .w_full()
        .h_full()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            col()
                .key("locker-page")
                .w_full()
                .px(crate::ui::PAGE_X)
                .pt(crate::ui::PAGE_TOP)
                .pb(32.0)
                .gap(16.0)
                .child(toolbar)
                .child(body),
        )
}

/// Grid or list, as a two part segmented control.
fn view_toggle(u: &Ui, list: bool) -> El {
    let segment = |glyph: &'static rune::Icon, label: &'static str, on: bool, value: bool| {
        row()
            .key(format!("locker-view-{label}"))
            .size(28.0, 26.0)
            .center()
            .radius(crate::ui::r(4.0))
            .fill(if on { u.selected } else { Color::TRANSPARENT })
            .hover(u.hover)
            .focusable(true)
            .role("radio")
            .checked(on)
            .label(label)
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| {
                st.locker.list_view = value;
                if value && crate::screens::settings::options::locker_expanded() {
                    st.locker.expanded = st
                        .locker
                        .heroes
                        .iter()
                        .map(|hero| hero.codename.clone())
                        .collect();
                }
            })
            .child(rune::icon(glyph, 14.0).color(if on { u.ink } else { u.muted }).a11y_hidden())
    };
    row()
        .key("locker-view")
        .h(32.0)
        .padding(2.0)
        .gap(2.0)
        .center_y()
        .radius(crate::ui::r(4.0))
        .border(1.0, u.hair)
        .role("radiogroup")
        .label("View")
        .child(segment(&icons::LAYOUT_GRID, "Grid", !list, false))
        .child(segment(&icons::LIST, "List", list, true))
}

fn retry(u: &Ui) -> El {
    u.icon_action(&icons::REFRESH_CW, "Try again", ActionKind::Secondary)
        .key("locker-retry")
        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| crate::data::start_locker_load(st))
}

/// All, Modded or Starred, as a three part segmented control.
fn filter(u: &Ui, modded_only: bool, starred_only: bool) -> El {
    let segment = |label: &'static str, on: bool, modded: bool, starred: bool| {
        row()
            .key(format!("locker-filter-{}", label.to_lowercase()))
            .h(26.0)
            .px(10.0)
            .center()
            .radius(crate::ui::r(4.0))
            .fill(if on { u.selected } else { Color::TRANSPARENT })
            .hover(u.hover)
            .focusable(true)
            .role("radio")
            .checked(on)
            .label(label)
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| {
                st.locker.modded_only = modded;
                st.locker.starred_only = starred;
            })
            .child(
                text(label)
                    .font(BODY_MED)
                    .color(if on { u.ink } else { u.muted })
                    .a11y_hidden(),
            )
    };
    row()
        .key("locker-filter")
        .h(32.0)
        .padding(2.0)
        .gap(2.0)
        .center_y()
        .radius(crate::ui::r(4.0))
        .border(1.0, u.hair)
        .role("radiogroup")
        .label("Show heroes")
        .child(segment(crate::screens::settings::i18n::tr("All"), !modded_only && !starred_only, false, false))
        .child(segment(crate::screens::settings::i18n::tr("Modded"), modded_only, true, false))
        .child(segment(crate::screens::settings::i18n::tr("Starred"), starred_only, false, true))
}

pub(super) fn pixels_for(s: &State, path: Option<&str>) -> Option<Pixels> {
    s.thumbnails.get(path?).cloned()
}

/// Pixels that have finished decoding, so a larger image can stand in for a smaller one only
/// once it is there.
fn ready_pixels(s: &State, path: Option<&str>) -> Option<Pixels> {
    pixels_for(s, path).filter(|pixels| pixels.size().0 > 0)
}

/// An image cropped into a `box_w` by `box_h` box like CSS `object-fit: cover` with
/// `object-position: x% y%`: the side with slack slides so that point stays in view.
pub(super) fn cover_at(pixels: &Pixels, size: Option<(u32, u32)>, box_w: f32, box_h: f32, x: f32, y: f32) -> El {
    let ratio = size
        .filter(|(w, h)| *w > 0 && *h > 0)
        .map(|(w, h)| w as f32 / h as f32);
    let Some(ratio) = ratio else {
        return el()
            .absolute(0.0, 0.0)
            .size(box_w, box_h)
            .pixels_fit(pixels, SurfaceFit::Fill)
            .a11y_hidden();
    };
    let (w, h, left, top) = if ratio >= box_w / box_h {
        let w = box_h * ratio;
        (w, box_h, -(w - box_w) * x / 100.0, 0.0)
    } else {
        let h = box_w / ratio;
        (box_w, h, 0.0, -(h - box_h) * y / 100.0)
    };
    el().absolute(left.round(), top.round())
        .size(w.round(), h.round())
        .pixels_fit(pixels, SurfaceFit::Fill)
        .a11y_hidden()
}

/// The shared frame of a Locker card: 3:4, rounded, a dark bottom scrim, the hover wash.
fn card_frame(key: String, label: String, card_w: f32) -> El {
    el().key(key)
        .w_full()
        .min_w(0.0)
        .h((card_w * 4.0 / 3.0).round())
        .radius(crate::ui::r(CARD_RADIUS))
        .fill(Color::WHITE.alpha(0.03))
        .clip_rounded(crate::ui::r(CARD_RADIUS))
        .hover_group()
        .focusable(true)
        .role("button")
        .label(label)
        .cursor(rune::Cursor::Pointer)
}

fn card_chrome() -> [El; 2] {
    [
        // Grimoire's `from-black/70 via-black/10 to-transparent`, bottom up.
        el().absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .linear_gradient(
                std::f32::consts::PI,
                [
                    (0.0, Color::BLACK.alpha(0.0)),
                    (0.5, Color::BLACK.alpha(0.08)),
                    (1.0, Color::BLACK.alpha(0.62)),
                ],
            )
            .a11y_hidden(),
        el().absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .radius(crate::ui::r(CARD_RADIUS))
            .border(1.0, crate::ui::Ui::new().hair_strong)
            .group_opacity(0.0, 1.0)
            .a11y_hidden(),
    ]
}

fn hero_card(s: &State, u: &Ui, v: &View, hero: &HeroCard, card_w: f32) -> El {
    let card_h = (card_w * 4.0 / 3.0).round();
    let pinned_here = v
        .pinned
        .as_deref()
        .is_some_and(|pinned| same_hero(hero, pinned));
    let favorite = v.prefs.is_favorite(&hero.display_name);
    let shuffles = v.prefs.shuffles(&hero.display_name);
    let name = hero.display_name.clone();
    let mut frame = card_frame(
        format!("locker-{}", hero.codename),
        hero.display_name.clone(),
        card_w,
    )
    .on_press(move |st: &mut State| open_hero(st, &name));

    let (art, art_size, focus) = hero.art();
    if let Some(pixels) = pixels_for(s, art.as_deref()) {
        frame = frame.child(cover_at(&pixels, art_size, card_w, card_h, focus, 20.0));
    }
    let [scrim, wash] = card_chrome();
    frame = frame.child(scrim);

    // Mods on this hero, top left, as Grimoire's customization pill.
    if hero.mod_count > 0 || pinned_here || shuffles {
        let mut pill = row()
            .absolute(8.0, 8.0)
            .h(20.0)
            .px(6.0)
            .gap(4.0)
            .center_y()
            .radius(crate::ui::r(4.0))
            .fill(Color::BLACK.alpha(0.55))
            .a11y_hidden();
        if hero.mod_count > 0 {
            pill = pill.child(
                text(hero.mod_count.to_string())
                    .font(SMALL_NUM)
                    .color(Color::WHITE),
            );
        }
        if shuffles {
            pill = pill.child(rune::icon(&icons::ROTATE_CCW, 12.0).color(Color::WHITE.alpha(0.85)));
        }
        if pinned_here {
            pill = pill.child(rune::icon(&icons::PIN, 12.0).color(Color::WHITE.alpha(0.85)));
        }
        frame = frame.child(pill);
    }

    // The star is a sibling of the card press, so starring never opens the hero.
    let hero_name = hero.display_name.clone();
    frame = frame.child(
        el().key(format!("locker-star-{}", hero.codename))
            .pin(None, Some(8.0), Some(8.0), None)
            .size(24.0, 24.0)
            .center()
            .radius(crate::ui::r(4.0))
            .fill(Color::BLACK.alpha(0.55))
            .hover(Color::BLACK.alpha(0.70))
            .focusable(true)
            .role("button")
            .label(if favorite {
                format!("Unstar {}", hero.display_name)
            } else {
                format!("Star {}", hero.display_name)
            })
            .group_opacity(if favorite { 1.0 } else { 0.0 }, 1.0)
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| {
                crate::data::toggle_locker_favorite(st, &hero_name)
            })
            .child(
                rune::icon(&icons::HEART, 12.0)
                    .color(if favorite { u.accent } else { Color::WHITE })
                    .a11y_hidden(),
            ),
    );

    frame = frame.child(name_plate(s, u, hero, (card_w * 0.7).floor(), 24.0, true));
    frame.child(wash)
}

/// The name art, right aligned at the bottom of a card, or the name as text without art.
fn name_plate(s: &State, u: &Ui, hero: &HeroCard, max_w: f32, max_h: f32, on_card: bool) -> El {
    let pixels = pixels_for(s, hero.name_art.as_deref());
    // On a card the plate is a fixed box (70 percent by 24, bottom left 8/8): art or text
    // sits in the same box, so decoding the art never moves it.
    let place = |e: El| {
        if on_card {
            row()
                .pin(Some(8.0), None, None, Some(8.0))
                .size(max_w, max_h)
                .center_y()
                .child(e)
        } else {
            e
        }
    };
    match (pixels, hero.name_size) {
        (Some(pixels), Some((w, h))) if w > 0 && h > 0 => {
            let ratio = w as f32 / h as f32;
            let (pw, ph) = if max_w / ratio <= max_h {
                (max_w, max_w / ratio)
            } else {
                (max_h * ratio, max_h)
            };
            place(
                el().size(pw.round(), ph.round())
                    .pixels_fit(&pixels, SurfaceFit::Fit)
                    .label(hero.display_name.clone()),
            )
        }
        _ => place(
            text(hero.display_name.clone())
                .font(if on_card { BODY_MED } else { H1 })
                .color(if on_card { Color::WHITE } else { u.ink })
                .truncate(),
        ),
    }
}

fn general_counts(v: &View) -> (usize, usize) {
    let general: Vec<&ModRow> = v.mods.iter().filter(|m| m.general.is_some()).collect();
    let types: std::collections::BTreeSet<GeneralType> =
        general.iter().filter_map(|m| m.general).collect();
    (general.len(), types.len())
}

/// The General tile: the environment backdrop, the label bottom right, as Grimoire's
/// `GlobalGalleryCard`.
fn general_card(s: &State, _u: &Ui, v: &View, card_w: f32) -> El {
    let card_h = (card_w * 4.0 / 3.0).round();
    let (count, types) = general_counts(v);
    let mut frame = card_frame("locker-general".to_string(), "General".to_string(), card_w)
        .on_press(|st: &mut State| open_general(st));
    if let Some(pixels) = pixels_for(s, v.general_card.as_deref()) {
        frame = frame.child(cover_at(&pixels, v.general_card_size, card_w, card_h, 50.0, 50.0));
    }
    let [scrim, wash] = card_chrome();
    let detail = if count == 0 {
        crate::screens::settings::i18n::tr("HUD, icons, sounds and more").to_string()
    } else if v.categories.is_empty() {
        format!("{}, {}", plural(count, "mod", "mods"), plural(types, "type", "types"))
    } else {
        format!(
            "{}, {}",
            plural(count, "mod", "mods"),
            plural(v.categories.len(), "category", "categories")
        )
    };
    frame
        .child(scrim)
        .child(
            // Bottom left, 8/8, like every hero's name plate.
            col()
                .pin(Some(8.0), None, Some(8.0), Some(8.0))
                .gap(2.0)
                .items_start()
                .child(text(crate::screens::settings::i18n::tr("General")).font(BODY_MED).color(Color::WHITE))
                .child(
                    text(detail)
                        .font(SMALL)
                        .color(Color::WHITE.alpha(0.80))
                        .truncate(),
                ),
        )
        .child(wash)
}

fn loading(cols: usize) -> El {
    let mut grid = el().w_full().equal_columns(cols).gap(GRID_GAP);
    for _ in 0..cols * 2 {
        grid = grid.child(
            el().w_full()
                .aspect_ratio(3.0 / 4.0)
                .radius(crate::ui::r(CARD_RADIUS))
                .fill(Color::WHITE.alpha(0.04)),
        );
    }
    grid
}

/// Grimoire's list view: one row per hero, opened to its skins.
fn list_body(s: &State, u: &Ui, v: &View, shown: &[&HeroCard]) -> El {
    let mut rows = col().w_full();
    for hero in shown {
        rows = rows.child(list_row(s, u, v, hero));
    }
    rows
}

fn list_row(s: &State, u: &Ui, v: &View, hero: &HeroCard) -> El {
    let open = v.expanded.contains(&hero.codename);
    let favorite = v.prefs.is_favorite(&hero.display_name);
    let shuffles = v.prefs.shuffles(&hero.display_name);
    let pinned_here = v
        .pinned
        .as_deref()
        .is_some_and(|pinned| same_hero(hero, pinned));
    let codename = hero.codename.clone();

    let (art, art_size, focus) = hero.art();
    let thumb = el()
        .size(32.0, 32.0)
        .radius(crate::ui::r(4.0))
        .clip_rounded(crate::ui::r(4.0))
        .fill(Color::WHITE.alpha(0.04))
        .a11y_hidden();
    let thumb = match pixels_for(s, art.as_deref()) {
        Some(pixels) => thumb.child(cover_at(&pixels, art_size, 32.0, 32.0, focus, 20.0)),
        None => thumb,
    };

    // "0 mods" is not written: the line stays empty.
    let mut meta = Vec::new();
    if hero.mod_count > 0 {
        meta.push(plural(hero.mod_count, "mod", "mods"));
    }
    let sounds = v
        .mods
        .iter()
        .filter(|m| m.heroes.iter().any(|name| name == &hero.display_name))
        .filter(|m| m.sounds.iter().any(|s| s.hero == hero.display_name))
        .count();
    if sounds > 0 {
        meta.push(plural(sounds, "sound", "sounds"));
    }
    if pinned_here {
        meta.push("project".to_string());
    }
    if shuffles {
        meta.push("shuffle".to_string());
    }

    let header = row()
        .key(format!("locker-list-{}", hero.codename))
        .w_full()
        .h(48.0)
        .px(8.0)
        .gap(12.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .hover(u.hover)
        .focusable(true)
        .role("button")
        .label(format!("{} skins", hero.display_name))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            if !st.locker.expanded.remove(&codename) {
                st.locker.expanded.insert(codename.clone());
            }
        })
        .child(thumb)
        .child({
            let mut names = col().grow(1.0).min_w(0.0).child(
                text(hero.display_name.clone())
                    .font(BODY_MED)
                    .color(u.ink)
                    .truncate(),
            );
            if !meta.is_empty() {
                names = names.child(text(meta.join(" · ")).font(SMALL).color(u.muted).truncate());
            }
            names
        });

    let star_name = hero.display_name.clone();
    let star = el()
        .key(format!("locker-list-star-{}", hero.codename))
        .size(28.0, 28.0)
        .center()
        .radius(crate::ui::r(4.0))
        .hover(u.hover)
        .focusable(true)
        .role("button")
        .label(if favorite {
            format!("Unstar {}", hero.display_name)
        } else {
            format!("Star {}", hero.display_name)
        })
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            crate::data::toggle_locker_favorite(st, &star_name)
        })
        .child(
            rune::icon(&icons::HEART, 14.0)
                .color(if favorite { u.accent } else { u.muted })
                .a11y_hidden(),
        );

    let open_name = hero.display_name.clone();
    let header = header
        .child(star)
        .child(
            el().size(28.0, 28.0)
                .center()
                .radius(crate::ui::r(4.0))
                .hover(u.hover)
                .focusable(true)
                .role("button")
                .label(format!("Open {}", hero.display_name))
                .help(format!("Open {}", hero.display_name))
                .child(
                    rune::icon(&icons::ARROW_LEFT, 14.0)
                        .color(u.muted)
                        .rotate(std::f32::consts::PI)
                        .a11y_hidden(),
                )
                .key(format!("locker-list-open-{}", hero.codename))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| open_hero(st, &open_name)),
        )
        .child(
            el().size(28.0, 28.0).center().child(
                rune::icon(if open { &icons::CHEVRON_DOWN } else { &icons::CHEVRON_RIGHT }, 14.0)
                    .color(u.muted)
                    .a11y_hidden(),
            ),
        );

    let mut body = col().w_full();
    if open {
        // Indented 44 so the mod names line up with the hero name.
        body = body.child(col().w_full().pl(44.0).child(variants::skins_list(s, u, v, hero, false, false)));
    }

    col().key(format!("locker-list-block-{}", hero.codename)).w_full().child(header).child(body)
}

fn open_hero(st: &mut State, name: &str) {
    st.locker.page = Page::Hero(name.to_string());
    st.locker.hero_section = HeroSection::Mods;
    let render = st
        .locker
        .heroes
        .iter()
        .find(|hero| hero.display_name == name)
        .and_then(|hero| hero.render.clone());
    if let Some(render) = render {
        crate::data::request_locker_art(st, &render);
    }
    crate::viewer::prepare::sync_locker(st);
}

/// The 3D view on or off. On, it shows the hero with the enabled mods, against the game's own.
fn toggle_3d(st: &mut State) {
    st.locker.view3d = !st.locker.view3d;
    if st.locker.view3d {
        crate::viewer::prepare::sync_locker(st);
    } else {
        crate::viewer::close(st, crate::viewer::prepare::LOCKER_KEY);
    }
}

/// How wide the section pane is with the 3D view on, and whether the view has room beside
/// it. Without room the view takes the pane's place.
fn pane_with_3d(page_w: f32) -> (f32, bool) {
    let pane = (page_w - RAIL_W - VIEW3D_W).clamp(PANE_MIN_W, PANE_W);
    (pane, page_w - RAIL_W - pane >= VIEW3D_MIN_W)
}

fn open_general(st: &mut State) {
    st.locker.page = Page::General;
    if let Some(bg) = st.locker.general_bg.clone() {
        crate::data::request_locker_art(st, &bg);
    }
    if st.locker.general_tab.is_none() {
        // Land on the first populated type, as Grimoire does.
        let first = GeneralType::ALL
            .into_iter()
            .find(|kind| st.locker.mods.iter().any(|m| m.general == Some(*kind)))
            .unwrap_or(GeneralType::SoulContainer);
        st.locker.general_tab = Some(GeneralTab::Type(first));
    }
}

fn back(st: &mut State) {
    st.locker.page = Page::Grid;
}

// --- shared drill-in shell -----------------------------------------------------

/// The backdrop, the layered fade and the content row for the hero and General pages.
fn drill_shell(u: &Ui, key: &str, backdrop: Option<El>, dim: f32, content: El) -> El {
    let mut shell = el().key(key.to_string()).w_full().h_full().fill(u.bg).clip_rounded(0.0);
    if let Some(art) = backdrop {
        shell = shell.child(art);
    }
    if dim > 0.0 {
        shell = shell.child(
            el().absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .fill(u.bg.alpha(dim))
                .a11y_hidden(),
        );
    }
    let bg = u.bg;
    shell
        // Depth at the bottom, Grimoire's `from-black/50`.
        .child(
            el().pin(Some(0.0), None, Some(0.0), Some(0.0))
                .h(128.0)
                .gradient(Color::BLACK.alpha(0.0), Color::BLACK.alpha(0.5))
                .a11y_hidden(),
        )
        // The rail side stays dark and fades out across the art, as the frosted layers do.
        .child(
            el().absolute(0.0, 0.0)
                .w(FADE_W)
                .h_full()
                .linear_gradient(
                    std::f32::consts::FRAC_PI_2,
                    [
                        (0.0, bg),
                        (0.28, bg.alpha(0.95)),
                        (0.50, bg.alpha(0.75)),
                        (0.70, bg.alpha(0.40)),
                        (0.85, bg.alpha(0.15)),
                        (0.96, bg.alpha(0.0)),
                    ],
                )
                .a11y_hidden(),
        )
        .child(content)
}

fn back_row(u: &Ui, extra: Option<El>) -> El {
    let mut r = row().w_full().gap(8.0).center_y().child(
        row()
            .key("locker-back")
            .h(32.0)
            .pl(8.0)
            .pr(12.0)
            .gap(8.0)
            .center_y()
            .radius(crate::ui::r(8.0))
            .hover(u.hover)
            .focusable(true)
            .role("button")
            .label("Back to Locker")
            .cursor(rune::Cursor::Pointer)
            .on_press(back)
            .child(rune::icon(&icons::ARROW_LEFT, 16.0).color(u.muted).a11y_hidden())
            .child(text(crate::screens::settings::i18n::tr("Locker")).font(BODY_MED).color(u.muted).a11y_hidden()),
    );
    if let Some(extra) = extra {
        r = r.child(spacer()).child(extra);
    }
    r
}

/// A rail row: label, count on the right, neutral fill when active.
fn rail_row(u: &Ui, key: String, label: &str, count: usize, active: bool, _dim: bool) -> El {
    row()
        .key(key)
        .w_full()
        .h(32.0)
        .px(10.0)
        .gap(8.0)
        .center_y()
        .radius(crate::ui::r(4.0))
        .fill(if active { u.selected } else { Color::TRANSPARENT })
        .hover(u.hover)
        .focusable(true)
        .role("tab")
        .checked(active)
        .label(label.to_string())
        .cursor(rune::Cursor::Pointer)
        .child(
            text(label.to_string())
                .font(crate::ui::NAV)
                .color(if active { u.ink } else { u.prose })
                .grow(1.0)
                .min_w(0.0)
                .truncate()
                .a11y_hidden(),
        )
        // The count in a fixed 28 point slot, drawn only above zero.
        .child(row().w(28.0).justify_end().child(if count > 0 {
            text(count.to_string()).font(SMALL_NUM).color(u.muted).a11y_hidden()
        } else {
            el()
        }))
}

/// A rail action: an icon and a label, tinted while its state is on.
fn rail_action(u: &Ui, key: String, glyph: &'static rune::Icon, label: &str, active: bool) -> El {
    row()
        .key(key)
        .w_full()
        .h(32.0)
        .px(10.0)
        .gap(8.0)
        .center_y()
        .radius(crate::ui::r(4.0))
        .fill(if active { u.selected } else { Color::TRANSPARENT })
        .hover(u.hover)
        .focusable(true)
        .role("button")
        .label(label.to_string())
        .child(
            rune::icon(glyph, 16.0)
                .color(u.muted)
                .a11y_hidden(),
        )
        .child(
            text(label.to_string())
                .font(BODY)
                .color(if active { u.ink } else { u.prose })
                .grow(1.0)
                .min_w(0.0)
                .truncate()
                .a11y_hidden(),
        )
}

/// The pane's header: the shared section header. The count lives on the rail, so the
/// header has none; its action is a ghost 24 that is always in the header.
fn pane_head(u: &Ui, title: &str, _count: String, trailing: Option<El>) -> El {
    u.section_head(title, None, trailing).w_full()
}

/// One installed mod: name, what it is, and its on or off state.
pub(super) fn mod_row(s: &State, u: &Ui, m: &ModRow, trailing: Option<El>) -> El {
    // The Installed screen's library is fresher than this load after a toggle.
    let enabled = s
        .addons
        .addons
        .iter()
        .find(|addon| addon.slot == m.slot)
        .map(|addon| addon.enabled)
        .unwrap_or(m.enabled);
    let detail = {
        let mut parts = vec![format!("Slot {}", deadlock_core::addons::slot_label(m.slot))];
        if !m.heroes.is_empty() {
            parts.push(m.heroes.join(", "));
        } else if let Some(kind) = m.general {
            parts.push(kind.label().to_string());
        }
        parts.join(" · ")
    };
    let slot = m.slot;
    let mut r = row()
        .key(format!("locker-mod-{}", m.key))
        .label(format!("{}, {}", m.label, m.file))
        .w_full()
        .h(48.0)
        .gap(12.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(art::thumb(s, u, m.slot))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .child(
                    text(m.label.clone())
                        .font(BODY_MED)
                        .color(if enabled { u.ink } else { u.muted })
                        .truncate(),
                )
                .child(text(detail).font(SMALL).color(u.muted).truncate()),
        )
        .child(favorite_button(s, u, m));
    if let Some(t) = trailing {
        r = r.child(t);
    }
    // Move earlier or later, Preview in 3D and Change the picture live in this menu.
    r.child(
        u.more_button(&format!("More for {}", m.label))
            .key(format!("locker-more-{slot}"))
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| {
                st.locker.more = if st.locker.more == Some(slot) { None } else { Some(slot) }
            }),
    )
    .child(
        u.toggle(enabled)
            .key(format!("locker-mod-toggle-{}", m.slot))
            .label(if enabled {
                format!("Disable {}", m.label)
            } else {
                format!("Enable {}", m.label)
            })
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| st.addon_dialog = Some((slot, !enabled))),
    )
}

/// Save or unsave a mod. The flag is Installed's (`mod_extras`), so a mod saved on either
/// page is saved on both.
fn favorite_button(s: &State, u: &Ui, m: &ModRow) -> El {
    use crate::screens::settings::i18n::{fill, tr};
    let slot = m.slot;
    let saved = crate::screens::installed::notes::is_favorite(s, slot);
    let label = if saved {
        fill(tr("Remove {} from saved"), &[&m.label])
    } else {
        fill(tr("Save {}"), &[&m.label])
    };
    el().size(32.0, 32.0)
        .center()
        .radius(crate::ui::r(8.0))
        .hover(u.hover)
        .focusable(true)
        .role("button")
        .label(label)
        .key(format!("locker-favorite-{slot}"))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| crate::screens::installed::notes::toggle_favorite(st, slot))
        .child(rune::icon(&icons::HEART, 16.0).color(if saved { u.accent } else { u.faint }).a11y_hidden())
}

// --- hero page -------------------------------------------------------------------

fn hero_page(s: &State, u: &Ui, v: &View, hero: &HeroCard) -> El {
    let (page_w, page_h) = page_size(v);
    // The full render once it has decoded, the card copy until then. The 3D view takes the
    // art's place.
    let backdrop = ready_pixels(s, hero.render.as_deref())
        .or_else(|| pixels_for(s, hero.card.as_deref()))
        .filter(|_| !v.view3d)
        .map(|pixels| {
        // Full height, natural width, anchored right: Grimoire's `h-full w-auto right-0`.
        let ratio = hero
            .render_size
            .filter(|(w, h)| *w > 0 && *h > 0)
            .map(|(w, h)| w as f32 / h as f32)
            .unwrap_or(1.0);
        let w = (page_h * ratio).round().max(page_w * 0.5);
        el().pin(None, Some(0.0), Some(0.0), None)
            .size(w, page_h.round())
            .pixels_fit(&pixels, SurfaceFit::Fill)
            .a11y_hidden()
    });

    let list = sorted_heroes(v);
    let at = list.iter().position(|h| h.display_name == hero.display_name);
    let step = |delta: isize| -> Option<String> {
        let at = at? as isize;
        let n = list.len() as isize;
        if n < 2 {
            return None;
        }
        Some(list[((at + delta).rem_euclid(n)) as usize].display_name.clone())
    };
    let mut nav = row().gap(4.0);
    for (glyph, label, target, key) in [
        (&icons::CHEVRON_LEFT, "Previous hero", step(-1), "locker-prev"),
        (&icons::CHEVRON_RIGHT, "Next hero", step(1), "locker-next"),
    ] {
        if let Some(target) = target {
            nav = nav.child(
                u.icon_button(glyph, label)
                    .key(key)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| open_hero(st, &target)),
            );
        }
    }

    let mods: Vec<&ModRow> = v
        .mods
        .iter()
        .filter(|m| m.heroes.iter().any(|name| name == &hero.display_name))
        .collect();
    let sounds = hero_sounds(v, hero);
    let section = v.hero_section;
    let sections = col()
        .w_full()
        .gap(4.0)
        .role("tablist")
        .child(
            rail_row(u, "locker-section-mods".into(), crate::screens::settings::i18n::tr("Skins"), mods.len(), section == HeroSection::Mods, mods.is_empty())
                .on_press(|st: &mut State| st.locker.hero_section = HeroSection::Mods),
        )
        .child(
            rail_row(u, "locker-section-sounds".into(), crate::screens::settings::i18n::tr("Sounds"), sounds.len(), section == HeroSection::Sounds, sounds.is_empty())
                .on_press(|st: &mut State| st.locker.hero_section = HeroSection::Sounds),
        )
        .child({
            let n = parts::count(v, &hero.display_name);
            rail_row(u, "locker-section-parts".into(), crate::screens::settings::i18n::tr("Weapons and voice"), n, section == HeroSection::Parts, n == 0)
                .on_press(|st: &mut State| st.locker.hero_section = HeroSection::Parts)
        })
        .child(
            rail_row(
                u,
                "locker-section-recolor".into(),
                crate::screens::settings::i18n::tr("Recolor"),
                hero.recolor_paths.len(),
                section == HeroSection::Recolor,
                hero.recolor_paths.is_empty(),
            )
            .on_press(|st: &mut State| st.locker.hero_section = HeroSection::Recolor),
        )
        .child({
            let n = v.assets.textures.ready().map_or(0, Vec::len);
            rail_row(u, "locker-section-game-art".into(), crate::screens::settings::i18n::tr("Game art"), n, section == HeroSection::GameArt, false)
                .on_press(|st: &mut State| {
                    st.locker.hero_section = HeroSection::GameArt;
                    assets::ensure_loaded(st);
                })
        })
        .child({
            let n = v.assets.sounds.ready().map_or(0, Vec::len);
            rail_row(u, "locker-section-game-sounds".into(), crate::screens::settings::i18n::tr("Game sounds"), n, section == HeroSection::GameSounds, false)
                .on_press(|st: &mut State| {
                    st.locker.hero_section = HeroSection::GameSounds;
                    assets::ensure_loaded(st);
                })
        });

    let pinned_here = v
        .pinned
        .as_deref()
        .is_some_and(|pinned| same_hero(hero, pinned));
    // The name plate's box is fixed (240 by 56): art or text, so decoding never moves the rail.
    let mut identity = row()
        .w(RAIL_W)
        .h(56.0)
        .center_y()
        .pl(10.0)
        .child(name_plate(s, u, hero, 200.0, 40.0, false));
    if pinned_here {
        identity = identity.help("Your project hero");
    }

    // Star, shuffle and the card picker, as three rail rows. The star and the switch are
    // states, so only the switch uses the accent.
    let favorite = v.prefs.is_favorite(&hero.display_name);
    let shuffles = v.prefs.shuffles(&hero.display_name);
    let star_name = hero.display_name.clone();
    let shuffle_name = hero.display_name.clone();
    let card_hero = hero.clone();
    let hideout_hero = hero.display_name.clone();
    // The two switches first (pressed directly), then the four actions: one weight for all.
    let controls = col()
        .w_full()
        .gap(0.0)
        .child(el().pl(10.0).pb(8.0).child(u.section(crate::screens::settings::i18n::tr("Hero"))))
        .child(
            rail_action(
                u,
                format!("locker-hero-3d-{}", hero.codename),
                &icons::BOX,
                crate::screens::settings::i18n::tr("3D view"),
                v.view3d,
            )
            .role("switch")
            .checked(v.view3d)
            .cursor(rune::Cursor::Pointer)
            .on_press(toggle_3d)
            .child(u.toggle_mark(v.view3d)),
        )
        .child(
            row()
                .key(format!("locker-hero-shuffle-{}", hero.codename))
                .w_full()
                .h(32.0)
                .px(10.0)
                .gap(8.0)
                .center_y()
                .radius(crate::ui::r(4.0))
                .hover(u.hover)
                .focusable(true)
                .role("switch")
                .checked(shuffles)
                .label(format!("Shuffle {} skins at launch", hero.display_name))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| crate::data::toggle_locker_shuffle(st, &shuffle_name))
                .child(rune::icon(&icons::ROTATE_CCW, 16.0).color(u.muted).a11y_hidden())
                .child(
                    text(crate::screens::settings::i18n::tr("Shuffle at launch").to_string())
                        .font(BODY)
                        .color(u.prose)
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate(),
                )
                .child(u.toggle_mark(shuffles)),
        )
        .child(
            rail_action(
                u,
                format!("locker-hero-hideout-{}", hero.codename),
                &crate::viewer::icons::HOUSE,
                crate::screens::settings::i18n::tr("Hideout"),
                false,
            )
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| crate::viewer::prepare::open_hideout(st, Some(hideout_hero.clone()))),
        )
        .child(
            rail_action(
                u,
                format!("locker-hero-star-{}", hero.codename),
                &icons::HEART,
                if favorite { "Starred" } else { crate::screens::settings::i18n::tr("Star this hero") },
                favorite,
            )
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| {
                crate::data::toggle_locker_favorite(st, &star_name)
            }),
        )
        .child(
            rail_action(
                u,
                format!("locker-hero-card-{}", hero.codename),
                &icons::SQUARE_PEN,
                crate::screens::settings::i18n::tr("Custom card"),
                hero.custom.is_some(),
            )
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| cards::open_card_dialog(st, &card_hero)),
        )
        .child({
            let (code, display) = (hero.codename.clone(), hero.display_name.clone());
            rail_action(
                u,
                format!("locker-hero-ingame-{}", hero.codename),
                &icons::SPARKLES,
                crate::screens::settings::i18n::tr("In-game card"),
                false,
            )
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| card_art::open(st, &code, &display))
        });

    let rail = col()
        .key("locker-hero-rail")
        .w(RAIL_W + crate::ui::PAGE_X)
        .h_full()
        .pl(crate::ui::PAGE_X)
        .pt(crate::ui::PAGE_TOP)
        .pb(16.0)
        .gap(16.0)
        .child(back_row(u, Some(nav)))
        .child(identity)
        .child(sections)
        .child(controls);

    let pane_body = match section {
        HeroSection::Mods => {
            let browse_name = hero.display_name.clone();
            let browse = move |kind: ActionKind| {
                let name = browse_name.clone();
                let _ = kind;
                u.icon_action(&icons::COMPASS, crate::screens::settings::i18n::tr("Browse skins"), ActionKind::Ghost)
                    .h(24.0)
                    .pl(6.0)
                    .pr(8.0)
                    .key("locker-find-mods")
                    .label(format!("Browse skins for {name}"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        crate::screens::browse::open_for_hero(st, &name)
                    })
            };
            let empty = mods.is_empty();
            let mut c = col().w_full().gap(8.0).child(pane_head(
                u,
                crate::screens::settings::i18n::tr("Skins"),
                plural(mods.len(), "mod", "mods"),
                Some(browse(ActionKind::Ghost)),
            ));
            if empty {
                c = c.child(
                    text(crate::screens::settings::i18n::fill(crate::screens::settings::i18n::tr("No installed mod changes {} yet."), &[&hero.display_name]))
                        .font(SMALL)
                        .color(u.muted)
                        .wrap(),
                );
            } else {
                c = c.child(variants::skins_list(s, u, v, hero, !v.view3d, true));
            }
            c
        }
        HeroSection::Sounds => {
            let empty = sounds.is_empty();
            let mut c = col().w_full().gap(8.0).child(pane_head(
                u,
                crate::screens::settings::i18n::tr("Sounds"),
                plural(sounds.len(), "mod", "mods"),
                None,
            ));
            if empty {
                c = c.child(
                    text(format!(
                        "No installed mod changes {}'s ability sounds yet.",
                        hero.display_name
                    ))
                    .font(SMALL)
                    .color(u.muted)
                    .wrap(),
                );
            } else {
                c = c.child(variants::sounds_list(s, u, v, hero));
            }
            c
        }
        HeroSection::Parts => col()
            .w_full()
            .gap(8.0)
            .child(pane_head(
                u,
                crate::screens::settings::i18n::tr("Weapons and voice"),
                plural(parts::count(v, &hero.display_name), "mod", "mods"),
                None,
            ))
            .child(parts::section(s, u, v, hero)),
        HeroSection::GameArt => catalog::section(s, u, v, hero),
        HeroSection::GameSounds => game_sounds::section(s, u, v, hero),
        HeroSection::Recolor => {
            let recolor_hero = hero.clone();
            let can = !hero.recolor_paths.is_empty();
            let action = u
                .icon_action(&icons::SPARKLES, crate::screens::settings::i18n::tr("Recolor"), ActionKind::Ghost)
                .h(24.0)
                .pl(6.0)
                .pr(8.0)
                .disabled(!can)
                .opacity(if can { 1.0 } else { 0.45 })
                .key(format!("recolor-{}", hero.codename))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| open_recolor(st, &recolor_hero));
            let mut c = col().w_full().gap(8.0).child(pane_head(
                u,
                crate::screens::settings::i18n::tr("Recolor"),
                plural(hero.recolor_paths.len(), "file", "files"),
                Some(action),
            ));
            if hero.recolor_paths.is_empty() {
                c = c.child(
                    text(if hero.in_catalog {
                        "This hero has no materials or textures to recolor yet."
                    } else {
                        "Index the game files to find this hero's materials."
                    })
                    .font(SMALL)
                    .color(u.muted)
                    .wrap(),
                );
            } else {
                let mut rows = col().w_full();
                for target in &hero.recolor_paths {
                    let (folder, file) = target.rsplit_once('/').unwrap_or(("", target));
                    rows = rows.child(
                        col()
                            .w_full()
                            .h(48.0)
                            .justify_center()
                            .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                            .child(text(file.to_string()).font(MONO).color(u.ink).truncate().selectable(true))
                            .child(text(folder.to_string()).font(SMALL).color(u.faint).truncate()),
                    );
                }
                c = c.child(rows);
            }
            c
        }
    };

    let (pane_w, room) = if v.view3d { pane_with_3d(page_w) } else { (PANE_W, true) };
    // An opaque pane: text never sits on the art.
    let pane = col()
        .key("locker-hero-pane-frame")
        .w(pane_w)
        .self_stretch()
        .min_h(0.0)
        .ml(if page_w < 1100.0 { 16.0 } else { 24.0 })
        .mt(crate::ui::PAGE_TOP)
        .mb(16.0)
        .fill(u.bg)
        .border(1.0, u.hair)
        .radius(crate::ui::r(8.0))
        .clip_rounded(crate::ui::r(8.0))
        .child(
            col()
                .key("locker-hero-pane")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(col().w_full().padding(16.0).child(pane_body)),
        );

    drill_shell(
        u,
        &format!("locker-hero-{}", hero.codename),
        backdrop,
        0.0,
        if v.view3d {
            let stage = el()
                .key("locker-hero-3d")
                .grow(1.0)
                .min_w(0.0)
                .h_full()
                .pt(24.0)
                .pr(24.0)
                .pb(24.0)
                .pl(if room { 0.0 } else { 8.0 })
                .child(crate::viewer::view(s, u, crate::viewer::prepare::LOCKER_KEY));
            let r = row().w_full().h_full().child(rail);
            if room { r.child(pane).child(stage) } else { r.child(stage) }
        } else {
            row().w_full().h_full().child(rail).child(pane)
        },
    )
}

fn open_recolor(st: &mut State, hero: &HeroCard) {
    st.locker.selected = Some(hero.codename.clone());
    st.locker.panel = Some(Panel {
        codename: hero.codename.clone(),
        display_name: hero.display_name.clone(),
        in_catalog: hero.in_catalog,
        targets: hero.recolor_paths.clone(),
        palette: 0,
        note: TextEdit::new(""),
        log: None,
    });
}

pub(super) fn plural(n: usize, one: &str, many: &str) -> String {
    if one == "mod" {
        let template = crate::screens::settings::i18n::tr(if n == 1 { "{} mod" } else { "{} mods" });
        return crate::screens::settings::i18n::fill(template, &[&n.to_string()]);
    }
    format!("{n} {}", if n == 1 { one } else { many })
}

pub(super) fn hero_sounds<'a>(v: &'a View, hero: &HeroCard) -> Vec<&'a ModRow> {
    let mut rows: Vec<&ModRow> = v
        .mods
        .iter()
        .filter(|m| m.sounds.iter().any(|s| s.hero == hero.display_name))
        .collect();
    rows.sort_by(|a, b| {
        let ability = |m: &ModRow| {
            m.sounds
                .iter()
                .find(|s| s.hero == hero.display_name)
                .map(|s| s.ability)
                .unwrap_or(0)
        };
        ability(a)
            .cmp(&ability(b))
            .then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
    });
    rows
}

// --- General page ----------------------------------------------------------------

fn tab_mods<'a>(v: &'a View, tab: &GeneralTab) -> Vec<&'a ModRow> {
    let mut mods: Vec<&ModRow> = match tab {
        GeneralTab::Type(kind) => v.mods.iter().filter(|m| m.general == Some(*kind)).collect(),
        GeneralTab::Custom(id) => match v.categories.iter().find(|c| &c.id == id) {
            Some(category) => v
                .mods
                .iter()
                .filter(|m| category.keys.iter().any(|key| key == &m.key))
                .collect(),
            None => Vec::new(),
        },
        GeneralTab::Cursors => Vec::new(),
    };
    // Enabled first, then by name, as Grimoire's groups.
    mods.sort_by(|a, b| {
        b.enabled
            .cmp(&a.enabled)
            .then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
    });
    mods
}

fn general_page(s: &State, u: &Ui, v: &View) -> El {
    let (page_w, page_h) = page_size(v);
    let backdrop = match ready_pixels(s, v.general_bg.as_deref()) {
        Some(pixels) => Some(cover_at(&pixels, v.general_bg_size, page_w, page_h, 50.0, 50.0)),
        None => pixels_for(s, v.general_card.as_deref())
            .map(|pixels| cover_at(&pixels, v.general_card_size, page_w, page_h, 50.0, 50.0)),
    };
    let (total, _) = general_counts(v);
    let active = v
        .general_tab
        .clone()
        .unwrap_or(GeneralTab::Type(GeneralType::SoulContainer));

    let mut tabs = col().w_full().gap(4.0).role("tablist");
    for kind in GeneralType::ALL {
        let count = v.mods.iter().filter(|m| m.general == Some(kind)).count();
        tabs = tabs.child(
            rail_row(
                u,
                format!("locker-tab-{}", kind.id()),
                kind.label(),
                count,
                active == GeneralTab::Type(kind),
                count == 0,
            )
            .on_press(move |st: &mut State| st.locker.general_tab = Some(GeneralTab::Type(kind))),
        );
    }
    // Grimoire's Cursor tab: packs, not VPKs, so its count is the packs.
    let packs = v.cursors.state.as_ref().map_or(0, |state| state.packs.len());
    tabs = tabs.child(
        rail_row(u, "locker-tab-cursors".to_string(), "Cursors", packs, active == GeneralTab::Cursors, packs == 0)
            .on_press(|st: &mut State| st.locker.general_tab = Some(GeneralTab::Cursors)),
    );
    if !v.categories.is_empty() {
        tabs = tabs.child(el().w_full().h(1.0).my(4.0).fill(u.hair).a11y_hidden());
    }
    for category in &v.categories {
        let tab = GeneralTab::Custom(category.id.clone());
        let count = tab_mods(v, &tab).len();
        let id = category.id.clone();
        tabs = tabs.child(
            rail_row(
                u,
                format!("locker-tab-custom-{}", category.id),
                &category.name,
                count,
                active == tab,
                count == 0,
            )
            .on_press(move |st: &mut State| {
                st.locker.general_tab = Some(GeneralTab::Custom(id.clone()))
            }),
        );
    }
    let small_link = |key: &'static str, glyph: &'static rune::Icon, label: &'static str| {
        row()
            .key(key)
            .h(28.0)
            .pl(10.0)
            .pr(12.0)
            .gap(6.0)
            .center_y()
            .radius(crate::ui::r(6.0))
            .hover(u.hover)
            .focusable(true)
            .role("button")
            .label(label)
            .cursor(rune::Cursor::Pointer)
            .child(rune::icon(glyph, 14.0).color(u.muted).a11y_hidden())
            .child(text(label).font(SMALL).color(u.muted).a11y_hidden())
    };
    let mut links = col().pt(4.0).gap(2.0).child(
        small_link("locker-new-category", &icons::PLUS, "New category")
            .on_press(|st: &mut State| open_name_dialog(st, None, None)),
    );
    if !v.categories.is_empty() {
        links = links.child(
            small_link("locker-manage-categories", &icons::SLIDERS_HORIZONTAL, "Manage")
                .on_press(|st: &mut State| st.locker.dialog = Some(Dialog::Manage)),
        );
    }
    tabs = tabs.child(links);
    tabs = tabs
        .child(el().w_full().h(1.0).my(4.0).fill(u.hair).a11y_hidden())
        .child(
            row()
                .key("locker-global")
                .w_full()
                .h(40.0)
                .px(12.0)
                .gap(12.0)
                .center_y()
                .radius(crate::ui::r(8.0))
                .label("Load the Global mods folder before addons")
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .child(
                            text("Global mods folder")
                                .font(BODY_MED)
                                .color(u.ink)
                                .truncate(),
                        )
                        .child(
                            text(if v.global.configured {
                                "Loads before addons"
                            } else {
                                "Off"
                            })
                            .font(SMALL)
                            .color(u.faint)
                            .truncate(),
                        ),
                )
                .child(
                    u.toggle(v.global.configured)
                        .label(crate::screens::settings::i18n::tr("Global mods folder").to_string())
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| {
                            let on = !st.locker.global.configured;
                            crate::data::set_locker_global(st, on)
                        }),
                ),
        )
        .child(
            text("A mod in game/citadel/grimoire wins any file it shares with an addon.")
                .font(SMALL)
                .color(u.faint)
                .px(12.0)
                .wrap(),
        );

    let rail = col()
        .key("locker-general-rail")
        .w(RAIL_W)
        .h_full()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            col()
                .w_full()
                .padding(24.0)
                .gap(24.0)
                .child(back_row(u, None))
                .child(
                    row()
                        .px(12.0)
                        .gap(8.0)
                        .items_end()
                        .child(text("General").font(H1).color(u.ink))
                        .child(text(plural(total, "mod", "mods")).font(SMALL).color(u.faint).pb(2.0)),
                )
                .child(tabs),
        );

    let mut mods = tab_mods(v, &active);
    // Saved mods first (Grimoire #593: favorites shared with Installed), stable otherwise.
    mods.sort_by_key(|m| !crate::screens::installed::notes::is_favorite(s, m.slot));
    let (title, custom_id) = match &active {
        GeneralTab::Cursors => (String::new(), None),
        GeneralTab::Type(kind) => (kind.label().to_string(), None),
        GeneralTab::Custom(id) => (
            v.categories
                .iter()
                .find(|c| &c.id == id)
                .map(|c| c.name.clone())
                .unwrap_or_default(),
            Some(id.clone()),
        ),
    };
    let add = custom_id.clone().map(|id| {
        u.icon_action(&icons::PLUS, "Add mods", ActionKind::Secondary)
            .key("locker-add-mods")
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| st.locker.dialog = Some(Dialog::Picker(id.clone())))
    });
    let mut pane_body = col()
        .w_full()
        .gap(8.0)
        .child(pane_head(u, &title, plural(mods.len(), "mod", "mods"), add));
    if active == GeneralTab::Cursors {
        pane_body = cursors::pane(s, u);
    } else if mods.is_empty() {
        let (line, action) = match &custom_id {
            Some(_) => (
                format!("Nothing in {title} yet. Add mods to file them here."),
                None,
            ),
            None => (
                format!("No {title} installed yet. Find some in Browse and they show up here."),
                Some(
                    u.action("Browse mods", ActionKind::Secondary)
                        .key("locker-browse")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| st.go(Screen::Browse)),
                ),
            ),
        };
        let mut empty = col()
            .w_full()
            .pt(8.0)
            .gap(12.0)
            .items_start()
            .child(text(line).font(BODY).color(u.muted).wrap());
        if let Some(action) = action {
            empty = empty.child(action);
        }
        pane_body = pane_body.child(empty);
    } else {
        let mut rows = col().w_full();
        for m in mods {
            let trailing = custom_id.clone().map(|id| {
                let key = m.key.clone();
                u.icon_button(&icons::X, &format!("Remove {} from {title}", m.label))
                    .key(format!("locker-unfile-{}", m.key))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        let next = deadlock_core::locker::toggle_category_membership(
                            &st.locker.categories,
                            &id,
                            &key,
                        );
                        crate::data::save_locker_categories(st, next);
                    })
            });
            rows = rows.child(mod_row(s, u, m, trailing));
        }
        pane_body = pane_body.child(rows);
    }

    let pane = col()
        .key("locker-general-pane")
        .grow(1.0)
        .max_w(640.0)
        .min_w(0.0)
        .h_full()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(col().w_full().padding(24.0).child(pane_body));

    drill_shell(
        u,
        "locker-general-page",
        backdrop,
        0.55,
        row().w_full().h_full().child(rail).child(pane),
    )
}

fn open_name_dialog(st: &mut State, editing: Option<String>, file_key: Option<String>) {
    let current = editing
        .as_ref()
        .and_then(|id| st.locker.categories.iter().find(|c| &c.id == id))
        .map(|c| c.name.clone())
        .unwrap_or_default();
    st.locker.dialog = Some(Dialog::Name(NameDialog {
        editing,
        file_key,
        name: TextEdit::new(&current),
        error: None,
    }));
}

fn submit_name(st: &mut State) {
    let Some(Dialog::Name(dialog)) = st.locker.dialog.as_ref() else {
        return;
    };
    let name = dialog.name.text.clone();
    let editing = dialog.editing.clone();
    let file_key = dialog.file_key.clone();
    let categories = st.locker.categories.clone();
    match editing {
        Some(id) => {
            let (next, applied) = deadlock_core::locker::rename_category(&categories, &id, &name);
            if applied {
                crate::data::save_locker_categories(st, next);
                st.locker.dialog = Some(Dialog::Manage);
            } else if let Some(Dialog::Name(dialog)) = st.locker.dialog.as_mut() {
                dialog.error = Some(if name.trim().is_empty() {
                    "Give the category a name.".to_string()
                } else {
                    "Another category already uses that name.".to_string()
                });
            }
        }
        None => {
            let (mut next, id) = deadlock_core::locker::create_category(&categories, &name);
            let Some(id) = id else {
                if let Some(Dialog::Name(dialog)) = st.locker.dialog.as_mut() {
                    dialog.error = Some("Give the category a name.".to_string());
                }
                return;
            };
            if let Some(key) = file_key {
                next = deadlock_core::locker::add_category_membership(&next, &id, &key);
            }
            crate::data::save_locker_categories(st, next);
            st.locker.general_tab = Some(GeneralTab::Custom(id));
            st.locker.dialog = None;
        }
    }
}

// --- dialogs ---------------------------------------------------------------------

/// Mounted at the root so dialogs overlay the shell.
/// The row "..." menu, mounted at the root without a dimming layer.
pub fn overlay(s: &State, u: &Ui) -> Option<El> {
    if s.screen != crate::model::Screen::Locker {
        return None;
    }
    s.locker.more.map(|slot| more_menu(s, u, slot)).or_else(|| catalog::kind_menu(s, u))
}

pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    if let Some(dialog) = card_art::dialog(s, u)
        .or_else(|| catalog::dialog(s, u))
        .or_else(|| game_sounds::swaps_dialog(s, u))
    {
        return Some(dialog);
    }
    if let Some(panel) = s.locker.panel.as_ref() {
        return Some(recolor_dialog(u, panel));
    }
    match s.locker.dialog.as_ref()? {
        Dialog::Name(dialog) => Some(name_dialog(u, dialog)),
        Dialog::Picker(id) => Some(picker_dialog(s, u, id)),
        Dialog::Manage => Some(manage_dialog(u, &s.locker.categories)),
        Dialog::Shuffle => Some(cards::shuffle_dialog(s, u)),
        Dialog::Applied => Some(cards::applied_dialog(s, u)),
        Dialog::Card(dialog) => Some(cards::card_dialog(s, u, dialog)),
    }
}

/// A mod row's "..." menu: Move earlier, Move later and Preview in 3D for a hero's skins,
/// and Change the picture for every mod.
fn more_menu(s: &State, u: &Ui, slot: u16) -> El {
    use crate::screens::settings::i18n::tr;
    let close = |st: &mut State| st.locker.more = None;
    let label = s.locker.mods.iter().find(|m| m.slot == slot).map(|m| m.label.clone()).unwrap_or_default();
    let mut menu = u.menu("locker-more-menu", &format!("locker-more-{slot}"), 220.0);
    if let Page::Hero(name) = &s.locker.page
        && s.locker.hero_section == HeroSection::Mods
        && let Some(hero) = s.locker.heroes.iter().find(|h| &h.display_name == name)
    {
        let mut skins: Vec<u16> = s
            .locker
            .mods
            .iter()
            .filter(|m| m.heroes.iter().any(|h| h == &hero.display_name))
            .map(|m| m.slot)
            .collect();
        skins.sort_unstable();
        if let Some(index) = skins.iter().position(|x| *x == slot) {
            let last = skins.len().saturating_sub(1);
            let (up, down, view) = (hero.display_name.clone(), hero.display_name.clone(), hero.display_name.clone());
            let off = |e: El, off: bool| e.disabled(off).opacity(if off { 0.45 } else { 1.0 });
            menu = menu
                .child(off(
                    u.menu_item("locker-more-up", &icons::ARROW_UP, tr("Move earlier"), false).on_press(move |st: &mut State| {
                        st.locker.more = None;
                        crate::data::move_locker_skin(st, &up, slot, index.saturating_sub(1))
                    }),
                    index == 0,
                ))
                .child(off(
                    u.menu_item("locker-more-down", &icons::CHEVRON_DOWN, tr("Move later"), false).on_press(move |st: &mut State| {
                        st.locker.more = None;
                        crate::data::move_locker_skin(st, &down, slot, index + 1)
                    }),
                    index >= last,
                ))
                .child({
                    let label = label.clone();
                    u.menu_item("locker-more-3d", &icons::BOX, tr("Preview in 3D"), false).on_press(
                        move |st: &mut State| {
                            st.locker.more = None;
                            crate::viewer::dialog::open_on(st, slot, &label, &view)
                        },
                    )
                });
        }
    }
    menu = menu.child(
        u.menu_item("locker-more-picture", &icons::SQUARE_PEN, tr("Change the picture"), false)
            .on_press(move |st: &mut State| {
                st.locker.more = None;
                crate::screens::installed::open_editor(st, slot)
            }),
    );
    u.popover_layer("locker-more-layer", el().on_press(close), menu.on_key(move |st: &mut State, ev: &rune::KeyEvent, _| {
        if ev.key == rune::Key::Escape {
            close(st);
            true
        } else {
            false
        }
    }))
}

pub(super) fn close_dialog(st: &mut State) {
    st.locker.dialog = None;
}

pub(super) fn shell(u: &Ui, key: &str, label: String, body: El) -> El {
    u.dialog_shell()
        .key(key.to_string())
        .role("dialog")
        .label(label)
        .w(480.0)
        .max_w(480.0)
        .padding(24.0)
        .modal(true, close_dialog)
        .on_key(move |st: &mut State, ev: &rune::KeyEvent, _| {
            if ev.key == rune::Key::Escape {
                close_dialog(st);
                true
            } else {
                false
            }
        })
        .child(body)
}

pub(super) fn dialog_title(u: &Ui, title: String, close_key: &'static str) -> El {
    row()
        .w_full()
        .gap(8.0)
        .center_y()
        .child(text(title).font(H1).color(u.ink).grow(1.0).min_w(0.0).truncate())
        .child(
            u.icon_button(&icons::X, "Close")
                .key(close_key)
                .cursor(rune::Cursor::Pointer)
                .on_press(close_dialog),
        )
}

fn name_dialog(u: &Ui, dialog: &NameDialog) -> El {
    let renaming = dialog.editing.is_some();
    let title = if renaming { "Rename category" } else { "New category" };
    let mut body = col()
        .w_full()
        .gap(16.0)
        .child(dialog_title(u, title.to_string(), "locker-name-close"))
        .child(
            row()
                .w_full()
                .h(32.0)
                .px(10.0)
                .center_y()
                .radius(crate::ui::r(8.0))
                .fill(u.field)
                .border(1.0, u.hair)
                .child(
                    rune::text_input(&dialog.name, BODY)
                        .key("locker-category-name")
                        .label("Category name")
                        .single_line()
                        .autofocus()
                        .color(u.ink)
                        .placeholder("e.g. My favorites", u.faint)
                        .caret(u.accent, u.accent.alpha(0.20))
                        .on_edit(|st: &mut State| match st.locker.dialog.as_mut() {
                            Some(Dialog::Name(dialog)) => &mut dialog.name,
                            _ => &mut st.locker.notes_placeholder,
                        })
                        .on_submit(|st: &mut State, _| submit_name(st))
                        .grow(1.0)
                        .min_w(0.0),
                ),
        );
    if let Some(error) = &dialog.error {
        body = body.child(text(error.clone()).font(SMALL).color(u.rose));
    }
    let ready = !dialog.name.text.trim().is_empty();
    body = body.child(
        row()
            .w_full()
            .gap(8.0)
            .child(spacer())
            .child(
                u.action("Cancel", ActionKind::Ghost)
                    .key("locker-name-cancel")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(close_dialog),
            )
            .child(
                u.action(if renaming { "Rename" } else { "Create" }, ActionKind::Primary)
                    .key("locker-name-save")
                    .disabled(!ready)
                    .opacity(if ready { 1.0 } else { 0.45 })
                    .cursor(rune::Cursor::Pointer)
                    .on_press(submit_name),
            ),
    );
    shell(u, "locker-name-dialog", title.to_string(), body)
}

fn picker_dialog(s: &State, u: &Ui, id: &str) -> El {
    let v = &s.locker;
    let Some(category) = v.categories.iter().find(|c| c.id == id) else {
        return shell(
            u,
            "locker-picker",
            "Add mods".to_string(),
            dialog_title(u, "Add mods".to_string(), "locker-picker-close"),
        );
    };
    let title = format!("Add mods to {}", category.name);
    let mut list = col().w_full();
    let mut mods: Vec<&ModRow> = v.mods.iter().collect();
    mods.sort_by_key(|a| a.label.to_lowercase());
    for m in &mods {
        let filed = category.keys.iter().any(|key| key == &m.key);
        let (cid, key) = (category.id.clone(), m.key.clone());
        let what = if !m.heroes.is_empty() {
            m.heroes.join(", ")
        } else {
            m.general
                .map(|kind| kind.label().to_string())
                .unwrap_or_else(|| format!("Slot {}", deadlock_core::addons::slot_label(m.slot)))
        };
        list = list.child(
            row()
                .key(format!("locker-pick-{}", m.key))
                .w_full()
                .h(44.0)
                .px(8.0)
                .gap(12.0)
                .center_y()
                .radius(crate::ui::r(8.0))
                .hover(u.hover)
                .focusable(true)
                .role("checkbox")
                .checked(filed)
                .label(m.label.clone())
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    let next = deadlock_core::locker::toggle_category_membership(
                        &st.locker.categories,
                        &cid,
                        &key,
                    );
                    crate::data::save_locker_categories(st, next);
                })
                .child(
                    el().size(16.0, 16.0)
                        .center()
                        .radius(crate::ui::r(4.0))
                        .fill(if filed { u.accent } else { Color::TRANSPARENT })
                        .border(1.0, if filed { u.accent } else { u.hair_strong })
                        .child(if filed {
                            rune::icon(&icons::CHECK, 12.0).color(u.accent_ink)
                        } else {
                            el()
                        })
                        .a11y_hidden(),
                )
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .child(text(m.label.clone()).font(BODY_MED).color(u.ink).truncate())
                        .child(text(what).font(SMALL).color(u.faint).truncate())
                        .a11y_hidden(),
                ),
        );
    }
    let body = col()
        .w_full()
        .gap(16.0)
        .child(dialog_title(u, title.clone(), "locker-picker-close"))
        .child(
            text("Filing a mod here never turns it on or off.")
                .font(BODY)
                .color(u.muted),
        )
        .child(if mods.is_empty() {
            text("No installed mods yet.").font(BODY).color(u.muted)
        } else {
            col()
                .key("locker-picker-scroll")
                .w_full()
                .max_h(360.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(list)
        })
        .child(
            row().w_full().child(spacer()).child(
                u.action("Done", ActionKind::Primary)
                    .key("locker-picker-done")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(close_dialog),
            ),
        );
    shell(u, "locker-picker", title, body)
}

fn manage_dialog(u: &Ui, categories: &[LockerCategory]) -> El {
    let mut list = col().w_full();
    for category in categories {
        let (rename_id, delete_id) = (category.id.clone(), category.id.clone());
        list = list.child(
            row()
                .key(format!("locker-manage-{}", category.id))
                .w_full()
                .h(44.0)
                .gap(4.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(
                    text(category.name.clone())
                        .font(BODY_MED)
                        .color(u.ink)
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate(),
                )
                .child(
                    text(plural(category.keys.len(), "mod", "mods"))
                        .font(SMALL)
                        .color(u.faint)
                        .pr(8.0),
                )
                .child(
                    u.icon_button(&icons::SQUARE_PEN, &format!("Rename {}", category.name))
                        .key(format!("locker-rename-{}", category.id))
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| {
                            open_name_dialog(st, Some(rename_id.clone()), None)
                        }),
                )
                .child(
                    u.icon_button(&icons::TRASH_2, &format!("Delete {}", category.name))
                        .key(format!("locker-delete-{}", category.id))
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| {
                            let next = deadlock_core::locker::delete_category(
                                &st.locker.categories,
                                &delete_id,
                            );
                            if st.locker.general_tab == Some(GeneralTab::Custom(delete_id.clone())) {
                                st.locker.general_tab = None;
                                open_general(st);
                            }
                            crate::data::save_locker_categories(st, next);
                        }),
                ),
        );
    }
    let body = col()
        .w_full()
        .gap(16.0)
        .child(dialog_title(u, "Manage categories".to_string(), "locker-manage-close"))
        .child(
            text("Deleting a category never deletes its mods.")
                .font(BODY)
                .color(u.muted),
        )
        .child(if categories.is_empty() {
            text("No categories yet.").font(BODY).color(u.muted)
        } else {
            list
        })
        .child(
            row().w_full().child(spacer()).child(
                u.action("Done", ActionKind::Primary)
                    .key("locker-manage-done")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(close_dialog),
            ),
        );
    shell(u, "locker-manage", "Manage categories".to_string(), body)
}

/// A representative colour for a palette preset, for its swatch.
fn swatch(name: &str, hue_shift: i32, saturation: f64) -> Color {
    let hue = match name.to_ascii_lowercase().as_str() {
        "ember" => 50.0,
        "frost" => 215.0,
        "toxic" => 140.0,
        "void" => 295.0,
        _ => (30 + hue_shift).rem_euclid(360) as f32,
    };
    oklch(0.72, (0.11 * saturation as f32).clamp(0.04, 0.17), hue)
}

fn title_case(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn recolor_dialog(u: &Ui, panel: &Panel) -> El {
    let palettes = deadlock_core::recolor::presets();

    let mut options = el().w_full().equal_columns(palettes.len().max(1)).gap(8.0);
    for (index, palette) in palettes.iter().enumerate() {
        let on = index == panel.palette;
        options = options.child(
            row()
                .key(format!("palette-{}", palette.name))
                .h(36.0)
                .px(12.0)
                .gap(8.0)
                .center_y()
                .radius(crate::ui::r(8.0))
                .fill(if on { u.selected } else { Color::TRANSPARENT })
                .border(1.0, if on { u.hair_strong } else { u.hair })
                .hover(u.hover)
                .focusable(true)
                .role("radio")
                .checked(on)
                .label(format!("{} palette", palette.name))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    if let Some(panel) = st.locker.panel.as_mut() {
                        panel.palette = index;
                    }
                })
                .child(
                    el().size(10.0, 10.0)
                        .radius(5.0)
                        .fill(swatch(&palette.name, palette.hue_shift, palette.saturation))
                        .a11y_hidden(),
                )
                .child(
                    text(title_case(&palette.name))
                        .font(BODY_MED)
                        .color(if on { u.ink } else { u.muted })
                        .a11y_hidden(),
                ),
        );
    }
    let chosen_note = palettes
        .get(panel.palette)
        .map(|palette| title_case(&palette.note))
        .unwrap_or_default();

    let mut files = col().w_full().gap(4.0);
    if panel.targets.is_empty() {
        files = files.child(
            text(if panel.in_catalog {
                "This hero has no materials or textures to recolor yet."
            } else {
                "Index the game files to find this hero's materials."
            })
            .font(SMALL)
            .color(u.muted)
            .wrap(),
        );
    } else {
        for target in panel.targets.iter().take(6) {
            files = files.child(
                text(target.clone())
                    .font(MONO)
                    .color(u.muted)
                    .truncate()
                    .selectable(true),
            );
        }
        if panel.targets.len() > 6 {
            files = files.child(
                text(format!("and {} more", panel.targets.len() - 6))
                    .font(SMALL)
                    .color(u.faint),
            );
        }
    }

    let section = |title: &str, meta: String| {
        row()
            .w_full()
            .gap(6.0)
            .center_y()
            .child(text(title.to_string()).font(BODY_MED).color(u.ink))
            .child(text(meta).font(SMALL).color(u.faint))
    };

    let ready = !panel.targets.is_empty();
    let mut body = col()
        .w_full()
        .gap(16.0)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .items_start()
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(4.0)
                        .child(
                            text(format!("Recolor {}", panel.display_name))
                                .font(H1)
                                .color(u.ink)
                                .truncate(),
                        )
                        .child(
                            text("Pick a palette and save a recolor brief to your workspace.")
                                .font(BODY)
                                .color(u.muted)
                                .wrap(),
                        ),
                )
                .child(
                    u.icon_button(&icons::X, "Close")
                        .key("recolor-close")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| st.locker.panel = None),
                ),
        )
        .child(
            col()
                .w_full()
                .gap(8.0)
                .child(section("Palette", String::new()))
                .child(options)
                .child(text(chosen_note).font(SMALL).color(u.faint)),
        )
        .child(
            col()
                .w_full()
                .gap(8.0)
                .child(section(
                    "Files",
                    if panel.targets.is_empty() {
                        String::new()
                    } else {
                        panel.targets.len().to_string()
                    },
                ))
                .child(files),
        )
        .child(
            col()
                .w_full()
                .gap(8.0)
                .child(section("Note", String::new()))
                .child(
                    row()
                        .w_full()
                        .h(32.0)
                        .px(10.0)
                        .center_y()
                        .radius(crate::ui::r(8.0))
                        .fill(u.field)
                        .border(1.0, u.hair)
                        .child(
                            rune::text_input(&panel.note, BODY)
                                .key("recolor-note")
                                .label("Recolor note")
                                .single_line()
                                .color(u.ink)
                                .placeholder("Optional", u.faint)
                                .caret(u.accent, u.accent.alpha(0.20))
                                .on_edit(|st: &mut State| {
                                    st.locker
                                        .panel
                                        .as_mut()
                                        .map(|panel| &mut panel.note)
                                        .unwrap_or(&mut st.locker.notes_placeholder)
                                })
                                .grow(1.0)
                                .min_w(0.0),
                        ),
                ),
        );

    if let Some(log) = &panel.log {
        let saved = log
            .strip_prefix("Brief written to ")
            .and_then(|rest| rest.split(". It lists").next());
        body = body.child(match saved {
            Some(path) => row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(
                    rune::icon(&icons::CIRCLE_CHECK, 14.0)
                        .color(u.muted)
                        .a11y_hidden(),
                )
                .child(
                    text(format!("Saved to {path}"))
                        .font(SMALL)
                        .color(u.prose)
                        .truncate()
                        .selectable(true)
                        .grow(1.0)
                        .min_w(0.0),
                ),
            None => text(log.clone()).font(SMALL).color(u.rose).wrap(),
        });
    }

    body = body.child(
        row()
            .w_full()
            .gap(8.0)
            .child(spacer())
            .child(
                u.action("Cancel", ActionKind::Ghost)
                    .key("recolor-cancel")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| st.locker.panel = None),
            )
            .child(
                u.action("Save brief", ActionKind::Primary)
                    .key("recolor-save")
                    .disabled(!ready)
                    .opacity(if ready { 1.0 } else { 0.45 })
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| crate::data::write_recolor_brief(st)),
            ),
    );

    let close = |st: &mut State| st.locker.panel = None;
    u.dialog_shell()
        .key("recolor-dialog")
        .role("dialog")
        .label(format!("Recolor {}", panel.display_name))
        .w(520.0)
        .max_w(520.0)
        .padding(24.0)
        .modal(true, close)
        .on_key(move |st: &mut State, ev: &rune::KeyEvent, _| {
            if ev.key == rune::Key::Escape {
                close(st);
                true
            } else {
                false
            }
        })
        .child(body)
}

fn same_hero(hero: &HeroCard, key: &str) -> bool {
    let wanted = deadlock_core::locker::hero_asset_key(key);
    if deadlock_core::locker::hero_asset_key(&hero.codename) == wanted
        || deadlock_core::locker::hero_asset_key(&hero.display_name) == wanted
    {
        return true;
    }
    // A project target names a folder such as `inferno_v4`.
    deadlock_core::locker::hero_for_codename(key).is_some_and(|roster| roster.name == hero.display_name)
}

// --- scenes ------------------------------------------------------------------------

/// A hero page from the live roster.
pub fn scene_hero(s: &mut State) {
    s.screen = Screen::Locker;
    if rune::is_headless() {
        crate::data::locker_now(s);
    }
    open_hero(s, "Haze");
}

/// The hero page's Browse skins pressed: Browse on Skins with the hero picked.
pub fn scene_browse_hero(s: &mut State) {
    scene_hero(s);
    crate::screens::browse::open_for_hero(s, "Haze");
    if rune::is_headless() {
        crate::data::browse_now(s, 1);
    }
}

/// Installed mods the screens draw in a scene: a real install may be absent, and a page of
/// empty rows shows nothing about the layout.
fn fixture_mods(s: &mut State) {
    if !s.locker.mods.is_empty() {
        return;
    }
    let sound = |hero: &str, ability: u8| LockerSound {
        hero: hero.to_string(),
        ability,
    };
    let mut rows = vec![
        ModRow {
            slot: 11,
            file: "pak11_dir.vpk".to_string(),
            enabled: true,
            key: "vpk:haze-neon".to_string(),
            label: "Haze Neon".to_string(),
            heroes: vec!["Haze".to_string()],
            general: None,
            sounds: Vec::new(),
            parts: Default::default(),
        },
        ModRow {
            slot: 12,
            file: "pak12_dir.vpk.disabled".to_string(),
            enabled: false,
            key: "vpk:haze-frost".to_string(),
            label: "Haze Frost".to_string(),
            heroes: vec!["Haze".to_string()],
            general: None,
            sounds: Vec::new(),
            parts: Default::default(),
        },
        ModRow {
            slot: 13,
            file: "pak13_dir.vpk".to_string(),
            enabled: true,
            key: "vpk:haze-ember".to_string(),
            label: "Haze Ember".to_string(),
            heroes: vec!["Haze".to_string()],
            general: None,
            sounds: Vec::new(),
            parts: Default::default(),
        },
        ModRow {
            slot: 14,
            file: "pak14_dir.vpk".to_string(),
            enabled: true,
            key: "vpk:wraith-royal".to_string(),
            label: "Wraith Royal".to_string(),
            heroes: vec!["Wraith".to_string()],
            general: None,
            sounds: Vec::new(),
            parts: Default::default(),
        },
        ModRow {
            slot: 15,
            file: "pak15_dir.vpk".to_string(),
            enabled: true,
            key: "vpk:haze-smoke".to_string(),
            label: "Smoke Bomb, quieter".to_string(),
            heroes: Vec::new(),
            general: Some(GeneralType::Announcer),
            sounds: vec![sound("Haze", 2)],
            parts: Default::default(),
        },
        ModRow {
            slot: 16,
            file: "pak16_dir.vpk.disabled".to_string(),
            enabled: false,
            key: "vpk:haze-smoke-2".to_string(),
            label: "Smoke Bomb, echo".to_string(),
            heroes: Vec::new(),
            general: Some(GeneralType::Announcer),
            sounds: vec![sound("Haze", 2)],
            parts: Default::default(),
        },
        ModRow {
            slot: 17,
            file: "pak17_dir.vpk".to_string(),
            enabled: true,
            key: "vpk:haze-ult".to_string(),
            label: "Bullet Dance, brass".to_string(),
            heroes: Vec::new(),
            general: Some(GeneralType::Announcer),
            sounds: vec![sound("Haze", 4)],
            parts: Default::default(),
        },
    ];
    rows.sort_by_key(|row| row.slot);
    s.locker.mods = rows;
    for hero in &mut s.locker.heroes {
        hero.mod_count = s
            .locker
            .mods
            .iter()
            .filter(|m| m.heroes.iter().any(|name| name == &hero.display_name))
            .count();
    }
}

fn fixture_hero(s: &State, name: &str) -> Option<HeroCard> {
    s.locker
        .heroes
        .iter()
        .find(|hero| hero.display_name == name)
        .cloned()
}

/// Haze's page with the 3D view on and the fixture skins, for the viewer's scenes. The caller
/// opens the view (stand-in, loading, failed, no game).
pub fn scene_view3d(s: &mut State) {
    s.screen = Screen::Locker;
    if rune::is_headless() {
        crate::data::locker_now(s);
    }
    fixture_mods(s);
    s.locker.view3d = true;
    s.locker.page = Page::Hero("Haze".to_string());
    if let Some(render) = s.locker.heroes.iter().find(|h| h.display_name == "Haze").and_then(|h| h.render.clone()) {
        crate::data::request_locker_art(s, &render);
    }
    s.locker.hero_section = HeroSection::Mods;
    let (page_w, page_h) = s.locker.page_size;
    let (pane, room) = pane_with_3d(page_w);
    let w = if room { page_w - RAIL_W - pane - 24.0 } else { page_w - RAIL_W - 32.0 };
    crate::viewer::headless_points(s, crate::viewer::prepare::LOCKER_KEY, [w, page_h - 76.0]);
}

/// The grid in list view, two heroes starred and Haze open to its skins.
pub fn scene_list(s: &mut State) {
    s.screen = Screen::Locker;
    if rune::is_headless() {
        crate::data::locker_now(s);
    }
    fixture_mods(s);
    s.locker.list_view = true;
    s.locker.prefs.toggle_favorite("Haze");
    s.locker.prefs.toggle_favorite("Abrams");
    s.locker.expanded.insert("haze".to_string());
}

/// The grid with the starred filter on.
pub fn scene_starred(s: &mut State) {
    scene_list(s);
    s.locker.list_view = false;
    s.locker.starred_only = true;
}

/// The grid filtered by the hero search, as type-ahead leaves it.
pub fn scene_search(s: &mut State) {
    s.screen = Screen::Locker;
    if rune::is_headless() {
        crate::data::locker_now(s);
    }
    fixture_mods(s);
    s.locker.query = TextEdit::new("talon");
    s.locker.query_text = "talon".to_string();
}

/// A hero page on its ability sounds.
pub fn scene_sounds(s: &mut State) {
    s.screen = Screen::Locker;
    if rune::is_headless() {
        crate::data::locker_now(s);
    }
    fixture_mods(s);
    open_hero(s, "Haze");
    s.locker.hero_section = HeroSection::Sounds;
}

/// The custom card dialog with an image chosen.
pub fn scene_card(s: &mut State) {
    s.screen = Screen::Locker;
    if rune::is_headless() {
        crate::data::locker_now(s);
    }
    fixture_mods(s);
    let Some(hero) = fixture_hero(s, "Haze") else {
        return;
    };
    open_hero(s, "Haze");
    cards::open_card_dialog(s, &hero);
    let source = hero.render.clone();
    let size = hero.render_size;
    if let Some(path) = source {
        crate::data::request_locker_art(s, &path);
        if let Some(Dialog::Card(dialog)) = s.locker.dialog.as_mut() {
            dialog.path = Some(path);
            dialog.size = size;
            dialog.focus_x = 46.0;
        }
    }
}

/// The shuffle switches, with two heroes on.
pub fn scene_shuffle(s: &mut State) {
    s.screen = Screen::Locker;
    if rune::is_headless() {
        crate::data::locker_now(s);
    }
    fixture_mods(s);
    s.locker.prefs.toggle_shuffle("Haze");
    s.locker.prefs.toggle_shuffle("Wraith");
    s.locker.dialog = Some(Dialog::Shuffle);
}

/// The applied changes review with a card, shuffle, a sound and a brief.
pub fn scene_applied(s: &mut State) {
    s.screen = Screen::Locker;
    if rune::is_headless() {
        crate::data::locker_now(s);
    }
    fixture_mods(s);
    s.locker.prefs.toggle_shuffle("Haze");
    if let Some(hero) = fixture_hero(s, "Haze")
        && let Some(path) = hero.render.clone()
    {
        s.locker.prefs.set_card(
            "Haze",
            deadlock_core::locker::CustomCard {
                file: "haze-card.png".to_string(),
                focus_x: 55.0,
                width: 100,
                height: 100,
            },
        );
        if let Some(found) = s
            .locker
            .heroes
            .iter_mut()
            .find(|found| found.display_name == "Haze")
        {
            found.custom = Some(path);
            found.custom_size = hero.render_size;
            found.custom_focus = 55.0;
        }
    }
    s.locker.briefs = vec![BriefRow {
        hero: "inferno".to_string(),
        display_name: "Infernus".to_string(),
        palette: "ember".to_string(),
        note: "winter event".to_string(),
    }];
    s.locker.dialog = Some(Dialog::Applied);
}

/// The General page with the Global priority folder turned on.
pub fn scene_global(s: &mut State) {
    s.screen = Screen::Locker;
    if rune::is_headless() {
        crate::data::locker_now(s);
    }
    fixture_mods(s);
    open_general(s);
    s.locker.global.configured = true;
    s.locker.global.folder = true;
}

#[cfg(test)]
mod tests {
    use super::{hero_name_matches, image_size};

    #[test]
    fn reads_png_and_webp_sizes() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&1100u32.to_be_bytes());
        png.extend_from_slice(&967u32.to_be_bytes());
        assert_eq!(image_size(&png), Some((1100, 967)));

        let mut webp = b"RIFF\0\0\0\0WEBPVP8X\0\0\0\0\0\0\0\0".to_vec();
        webp.extend_from_slice(&[0x4b, 0x04, 0x00, 0xc6, 0x03, 0x00]);
        assert_eq!(image_size(&webp), Some((1100, 967)));
        assert_eq!(image_size(b"nope"), None);
    }

    #[test]
    fn hero_search_reaches_names_codenames_and_aliases() {
        assert!(hero_name_matches("Grey Talon", "orion", ""));
        assert!(hero_name_matches("Grey Talon", "orion", "talon"));
        assert!(hero_name_matches("Grey Talon", "orion", "orion"));
        assert!(hero_name_matches("Grey Talon", "orion", "archer"));
        assert!(hero_name_matches("Abrams", "atlas", "bull"));
        assert!(!hero_name_matches("Haze", "haze", "talon"));
        assert!(!hero_name_matches("Haze", "haze", "wraith"));
    }
}

/// `w4a-locker-parts`: Haze's Weapons and voice section with a weapon skin, voice lines and a
/// gun sound mod.
pub fn scene_parts(s: &mut State) {
    use deadlock_core::locker_parts::HeroPart;
    scene_hero(s);
    fixture_mods(s);
    let mut add = |slot: u16, label: &str, on: bool, parts: &[HeroPart]| {
        let mut map = Parts::new();
        map.insert("Haze".to_string(), parts.iter().copied().collect());
        s.locker.mods.push(ModRow {
            slot,
            file: format!("pak{slot:02}_dir.vpk"),
            enabled: on,
            key: format!("vpk:part-{slot}"),
            label: label.to_string(),
            heroes: Vec::new(),
            general: None,
            sounds: Vec::new(),
            parts: map,
        });
    };
    add(18, "Haze Gold Pistols", true, &[HeroPart::Weapon]);
    add(19, "Haze Chrome Pistols", false, &[HeroPart::Weapon]);
    add(20, "Haze, dubbed", true, &[HeroPart::Voice]);
    add(21, "Silenced pistols", true, &[HeroPart::WeaponSound]);
    s.locker.hero_section = HeroSection::Parts;
}
