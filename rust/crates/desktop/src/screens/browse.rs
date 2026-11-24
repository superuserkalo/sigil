//! Browse: the public GameBanana Deadlock feed as one long image grid.
//!
//! Pages are read on a worker and appended as the player scrolls near the end, so the grid
//! reads as one catalog. Only the mounted rows are built (`virtual_rows`). Thumbnails arrive
//! one by one through the shared image crew and its disk cache.
//!
//! A card opens the mod details view, a port of Grimoire's `ModDetailsModal` (MIT): gallery on
//! the left; category, files, artist, about, changelog and comments on the right. Install on a
//! card, a list row, the details or a file installs at once (`quick`): the press is the
//! approval, and each mod's button shows its own download, install and Installed state. The
//! install dialog stays for archives that hold several mods and for the Downloads page.

use std::borrow::Cow;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use deadlock_core::catalog_mirror::{self, CreatorHit, Mirror, MirrorFilter};
use deadlock_core::gamebanana::{CollectionSummary, DateField};
use deadlock_core::gamebanana::{
    self, BrowseCategory, BrowseMod, BrowseOrder, BrowsePeriod, BrowseQuery, BrowseSection,
    ModComments, ModFile, ModImage, ModMember, ModProfile, ModUpdate, ModUpdates, TopSub,
};
use deadlock_core::install::VpkChoice;
use deadlock_core::mod_updates::RemoteFile;
use deadlock_core::requirements::Requirement;
use rune::{
    Align, Color, Drag, El, GrowthSpec, Icon, Key, KeyEvent, Phase, ScrollSpec, Side, SurfaceFit,
    TextEdit, TextStyle, Tx, col, el, icon, row, spacer, text,
};

use crate::app::State;
use crate::icons;
use crate::jobs::Job;
use crate::model::{BrowseCard, BrowsePageView, Screen, format_count};
use crate::ui::{ActionKind, BODY, BODY_MED, H1, H2, LABEL, PAGE_TOP, PAGE_X, SMALL, Tone, Ui};

mod audio;
pub mod collections;
pub mod creator_page;
mod detail_state;
pub mod foryou;
pub mod details;
pub mod grid;
pub mod hidden;
pub mod sources;
mod hero;
mod install;
pub mod page;
mod panels;
mod persist;
pub mod multi;
pub mod quick;
pub mod w4;
pub mod paste;
pub mod replace;
pub mod scenes;
pub mod store;
pub mod shelf;
pub mod top;
pub mod toolbar;
pub mod tree;

use self::collections::*;
pub(crate) use self::detail_state::*;
pub(crate) use self::details::*;
pub(crate) use self::grid::*;
pub(crate) use self::install::*;
pub(crate) use self::store::*;
pub use self::details::dialog;
pub use self::store::{card_from_browse, load_more_if_near_end, mirror_page};
pub use self::page::view;
pub use self::hero::open_for_hero;
pub use self::install::{InstallOutcome, InstallView, PlanSummary, after_install};
pub use self::scenes::*;

const GRID_GAP: f32 = 12.0;
const CARD_RADIUS: f32 = 8.0;
/// Card width bounds, as Grimoire's `clamp(220, 118 + 7vw + 3vh, 300)` with a little more room
/// for the Install button.
const CARD_MIN: f32 = 220.0;
const CARD_MAX: f32 = 320.0;
/// Card text block under the image, fixed so every card and the virtual row estimate agree:
/// 10 padding, 22 title, 2 + 18 byline, 8 + 20 counts, 6 + 32 action row, 10 padding.
const CARD_TEXT_H: f32 = 112.0;
/// Grid width before the first layout reports one: the default window minus the sidebar.
const DEFAULT_GRID_W: f32 = 1064.0;
/// Rows one browsed page holds, matching `data::BROWSE_PER_PAGE` and GameBanana's cap.
pub const PAGE_SIZE: usize = 50;
/// Rows of look-ahead: the next page is read once the viewport is this close to the end.
const LOOKAHEAD_ROWS: usize = 3;
/// Decoded thumbnails kept before far-away cards give theirs back (the disk cache reloads them).
/// A decoded 530 px card thumbnail with its mips is about 0.85 MB, so this bounds Browse near
/// 40 MB however far the catalog scrolls.
const THUMB_CAP: usize = 48;
/// Rows either side of the viewport whose thumbnails are always kept.
const THUMB_MARGIN_ROWS: usize = 2;

const DETAILS_MAX_W: f32 = 1152.0;
const DETAILS_MAX_H: f32 = 920.0;
const SIDEBAR_W: f32 = 440.0;
const COMMENTS_PER_PAGE: usize = 20;
const UPDATES_PER_PAGE: usize = 10;

const MICRO_CAPS: TextStyle = TextStyle::new(10.0, 650.0).lh(12.0).tracking(0.6);

// Lucide (ISC) icons this screen needs beyond the shared set.
static EYE_OFF: Icon = Icon {
    name: "sigil-eye-off",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M10.733 5.076a10.744 10.744 0 0 1 11.205 6.575 1 1 0 0 1 0 .696 10.747 10.747 0 0 1-1.444 2.49"/><path d="M14.084 14.158a3 3 0 0 1-4.242-4.242"/><path d="M17.479 17.499a10.75 10.75 0 0 1-15.417-5.151 1 1 0 0 1 0-.696 10.75 10.75 0 0 1 4.446-5.143"/><path d="m2 2 20 20"/></g>"#,
};
static PANEL_RIGHT: Icon = Icon {
    name: "sigil-panel-right",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><rect width="18" height="18" x="3" y="3" rx="2"/><path d="M15 3v18"/></g>"#,
};
static MAXIMIZE_2: Icon = Icon {
    name: "sigil-maximize-2",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M15 3h6v6"/><path d="m21 3-7 7"/><path d="m3 21 7-7"/><path d="M9 21H3v-6"/></g>"#,
};
static CLOCK: Icon = Icon {
    name: "sigil-clock",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><path d="M12 6v6l4 2"/></g>"#,
};
static STAR: Icon = Icon {
    name: "sigil-star",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M11.525 2.295a.53.53 0 0 1 .95 0l2.31 4.679a2.123 2.123 0 0 0 1.595 1.16l5.166.756a.53.53 0 0 1 .294.904l-3.736 3.638a2.123 2.123 0 0 0-.611 1.878l.882 5.14a.53.53 0 0 1-.771.56l-4.618-2.428a2.122 2.122 0 0 0-1.973 0L6.396 21.01a.53.53 0 0 1-.77-.56l.881-5.139a2.122 2.122 0 0 0-.611-1.879L2.16 9.795a.53.53 0 0 1 .294-.906l5.165-.755a2.122 2.122 0 0 0 1.597-1.16z"/></g>"#,
};
static STAR_FILLED: Icon = Icon {
    name: "sigil-star-filled",
    view: 24.0,
    body: r#"<g fill="currentColor" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M11.525 2.295a.53.53 0 0 1 .95 0l2.31 4.679a2.123 2.123 0 0 0 1.595 1.16l5.166.756a.53.53 0 0 1 .294.904l-3.736 3.638a2.123 2.123 0 0 0-.611 1.878l.882 5.14a.53.53 0 0 1-.771.56l-4.618-2.428a2.122 2.122 0 0 0-1.973 0L6.396 21.01a.53.53 0 0 1-.77-.56l.881-5.139a2.122 2.122 0 0 0-.611-1.879L2.16 9.795a.53.53 0 0 1 .294-.906l5.165-.755a2.122 2.122 0 0 0 1.597-1.16z"/></g>"#,
};
static SORT: Icon = Icon {
    name: "sigil-arrow-down-wide-narrow",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="m3 16 4 4 4-4"/><path d="M7 20V4"/><path d="M11 4h10"/><path d="M11 8h7"/><path d="M11 12h4"/></g>"#,
};
static FILE_ARCHIVE: Icon = Icon {
    name: "sigil-file-archive",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M10 12v-1"/><path d="M10 18v-2"/><path d="M10 7V6"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="M15.5 22H18a2 2 0 0 0 2-2V7l-5-5H6a2 2 0 0 0-2 2v16a2 2 0 0 0 .274 1.01"/><circle cx="10" cy="20" r="2"/></g>"#,
};

// --- state -------------------------------------------------------------------

/// The tab over the grid. Mods, Sounds, WiPs and the mixed feed read submissions; Collections
/// reads GameBanana collections; Maps is the Maps category under Mods.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Tab {
    /// Mods, sounds and WiPs in one feed.
    All,
    #[default]
    Mods,
    Sounds,
    Wips,
    Collections,
    Maps,
}

impl Tab {
    pub const ALL: [Tab; 6] = [
        Tab::All,
        Tab::Mods,
        Tab::Sounds,
        Tab::Wips,
        Tab::Collections,
        Tab::Maps,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tab::All => "All",
            Tab::Mods => "Mods",
            Tab::Sounds => "Sounds",
            Tab::Wips => "WiPs",
            Tab::Collections => "Collections",
            Tab::Maps => "Maps",
        }
    }

    pub fn section(self) -> BrowseSection {
        match self {
            Tab::Sounds => BrowseSection::Sounds,
            Tab::Wips => BrowseSection::Wips,
            _ => BrowseSection::Mods,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Tab::All => "all",
            Tab::Mods => "mods",
            Tab::Sounds => "sounds",
            Tab::Wips => "wips",
            Tab::Collections => "collections",
            Tab::Maps => "maps",
        }
    }

    fn from_id(id: &str) -> Tab {
        Tab::ALL
            .into_iter()
            .find(|tab| tab.id().eq_ignore_ascii_case(id))
            .unwrap_or_default()
    }
}

/// GameBanana's Maps root under Mods, verified 2026-11-17.
const MAPS_CATEGORY: u64 = 37225;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Layout {
    #[default]
    Grid,
    List,
}

impl Layout {
    pub const ALL: [Layout; 2] = [Layout::Grid, Layout::List];

    pub fn label(self) -> &'static str {
        match self {
            Layout::Grid => "Grid",
            Layout::List => "List",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Layout::Grid => "grid",
            Layout::List => "list",
        }
    }

    fn from_id(id: &str) -> Layout {
        if id.eq_ignore_ascii_case("list") { Layout::List } else { Layout::Grid }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CardSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl CardSize {
    pub const ALL: [CardSize; 3] = [CardSize::Small, CardSize::Medium, CardSize::Large];

    pub fn label(self) -> &'static str {
        match self {
            CardSize::Small => "Small",
            CardSize::Medium => "Medium",
            CardSize::Large => "Large",
        }
    }

    /// The narrowest and widest a card of this size may be.
    fn bounds(self) -> (f32, f32) {
        match self {
            // 192 is the narrowest card whose three count columns hold "888m".
            CardSize::Small => (192.0, 244.0),
            CardSize::Medium => (CARD_MIN, CARD_MAX),
            CardSize::Large => (264.0, 396.0),
        }
    }

    fn id(self) -> &'static str {
        match self {
            CardSize::Small => "small",
            CardSize::Medium => "medium",
            CardSize::Large => "large",
        }
    }

    fn from_id(id: &str) -> CardSize {
        match id.to_ascii_lowercase().as_str() {
            "small" => CardSize::Small,
            "large" => CardSize::Large,
            _ => CardSize::Medium,
        }
    }
}

/// A management list over the grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Hidden,
    Ignored,
}

/// Where the hero picker writes its pick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeroTarget {
    /// The grid's hero filter, for sounds and WiPs.
    Filter,
    /// The open mod's detected hero.
    Details,
}

/// Browse state beyond the catalog itself (`State::browse`).
#[derive(Default)]
pub struct View {
    /// Grid width in points, from the last layout. Zero before the first one.
    pub width: f32,
    /// Columns the grid was last built with.
    pub cols: usize,
    /// Grid rows on screen, from `on_visible_rows`.
    pub visible: Range<usize>,
    /// The page's box in the window, for a context menu that stays on screen.
    pub page_rect: [f32; 4],
    /// NSFW mods the player chose to show.
    pub revealed: HashSet<u64>,
    /// Mods and creators hidden for this session and later runs, and the names the
    /// management list shows (a set loses them on load).
    pub hidden_mods: HashSet<u64>,
    pub hidden_authors: HashSet<String>,
    pub hidden_names: std::collections::HashMap<u64, String>,
    /// When each hidden entry was hidden, the mod a creator was hidden from, a note and a
    /// picture (Grimoire #580). Keyed by `hidden::mod_key` and `hidden::author_key`.
    pub hidden_meta: std::collections::HashMap<String, hidden::HiddenMeta>,
    /// The hidden entry whose note is being written, and the text.
    pub hidden_note: Option<(String, TextEdit)>,
    /// The hidden row under the pointer: its pictures show unblurred.
    pub hidden_hover: Option<String>,
    /// The custom mod source Browse shows instead of GameBanana (`sources.rs`).
    pub sources: sources::SourceView,
    /// The third level of the category tree and the landing rail's drag (`tree.rs`).
    pub tree: tree::Tree,
    /// The landing shelves' scroll positions (`shelf.rs`).
    pub shelves: shelf::Shelves,
    /// The open menu is a sub panel of Filters: a pick goes back to Filters.
    pub from_filters: bool,
    /// The search field is open over the narrowest toolbar line.
    pub search_open: bool,
    pub details: Option<Details>,
    /// The details view sits beside the grid instead of over it.
    pub docked: bool,
    /// The last catalog read was a refresh of page 1, not the next page.
    pub refreshing: bool,
    pub details_job: Job<DetailsLoad>,
    /// A profile read ahead of a press, kept so the details open filled in.
    pub prefetch_job: Job<(u64, ModProfile)>,
    pub prefetched: Vec<(u64, ModProfile)>,
    /// When the pointer rested on a card, for the 250 ms prefetch.
    pub hover_since: Option<(u64, Instant)>,
    /// Comments or changelog pages beyond the first.
    pub more_job: Job<MoreLoad>,

    /// The filters the grid shows: the browse "address", kept while a mod is open and when
    /// the player leaves Browse and comes back.
    pub filters: Filters,
    /// The open tab. Collections and Maps are views of their own.
    pub tab: Tab,
    pub layout: Layout,
    pub card_size: CardSize,
    /// Numbered pages instead of one long feed.
    pub paged: bool,
    /// The search text the catalog was read for. The titlebar field applies it after a pause.
    pub applied_search: String,
    /// When the titlebar search last changed and has not been applied yet.
    pub search_edited: Option<Instant>,
    /// The open filter dropdown.
    pub menu: Option<FilterMenu>,
    /// Type-ahead inside a long dropdown (heroes).
    pub menu_query: TextEdit,
    /// The category tree per section, read once per session.
    pub categories: Vec<(BrowseSection, Vec<BrowseCategory>)>,
    pub categories_job: Job<(BrowseSection, Vec<BrowseCategory>)>,
    pub categories_failed: Vec<BrowseSection>,
    /// GameBanana's top submissions per period, for the dashboard rails.
    pub top: Option<Vec<TopSub>>,
    pub top_job: Job<Vec<TopSub>>,
    /// Saved mods, read from disk on first use.
    pub favorites: Option<Vec<Favorite>>,
    /// Filters, view options and hidden lists, read once on the first Browse frame.
    pub persisted_job: Job<persist::Persisted>,
    pub loaded_persisted: bool,
    /// The custom date range's two fields, parsed on Apply.
    pub date_from_text: TextEdit,
    pub date_to_text: TextEdit,
    /// The query key of the page read in flight, so a read for old filters is dropped.
    pub in_flight: Option<u64>,
    /// Shown cards when the last automatic page read was asked, and how many reads in a row
    /// added none (client-side filters can empty whole pages). Paging pauses after a few.
    pub auto_shown: usize,
    pub auto_streak: usize,
    /// The card under the pointer, which shows its favorite star.
    pub hovered: Option<u64>,
    /// The card whose context menu is open.
    pub context: Option<u64>,
    /// A management dialog over the grid.
    pub panel: Option<Panel>,
    /// The hero picker, and the type-ahead inside it.
    pub hero_picker: Option<HeroTarget>,
    pub hero_query: TextEdit,

    /// The local catalog copy, loaded on a worker and synced a few pages at a time.
    pub mirror: Option<Mirror>,
    pub mirror_job: Job<Mirror>,
    pub mirror_sync_job: Job<Mirror>,
    pub mirror_save_job: Job<()>,
    pub mirror_dirty: bool,
    pub last_sync: Option<Instant>,
    pub mirror_summary: String,
    /// Creators matching the applied search, from the mirror.
    pub creator_hits: Vec<CreatorHit>,
    /// Download counts the saved catalog knows, by submission id, so a card reads one map.
    pub download_counts: std::collections::HashMap<u64, u64>,
    /// The catalog currently shown came from the mirror, not the network.
    pub from_mirror: bool,

    /// Collections: the first page, the open one and its items.
    pub collections: Option<CollectionLoad>,
    pub collections_job: Job<CollectionLoad>,
    pub collection: Option<CollectionOpen>,
    /// Which size of collection the Collections tab keeps.
    pub collection_bucket: collections::Bucket,

    /// Sound previews.
    pub audio: audio::Player,
    /// Mods waiting for the install queue, with the names the dialog shows.
    pub queue: Vec<(u64, String)>,
    pub queue_total: usize,
    pub queue_done: usize,
    /// Archives with several VPKs, read after a plan refused to choose.
    pub vpk_job: Job<Vec<VpkChoice>>,
    /// Updates the player chose to skip.
    pub ignored_updates: Vec<u64>,
    /// The last update check's cache, read when a mod page opens so its tags can show.
    pub update_cache: Option<deadlock_core::mod_updates::UpdateCache>,
    /// One-press installs, per mod, and which mods are installed.
    pub installs: quick::Installs,
    /// A Replace from a file row, waiting for its confirm.
    pub replace: Option<replace::Pending>,
    /// Wave 4: dock width, random hero, featured picks, creator links, image menu.
    pub w4: w4::W4,
    /// Wave 4: the bulk paste dialog (Grimoire).
    pub paste: paste::Paste,
}

/// One page of collections.
pub struct CollectionLoad {
    pub page: usize,
    pub total: usize,
    pub complete: bool,
    pub items: Vec<CollectionSummary>,
    pub error: Option<String>,
}

/// An open collection and the submissions it lists.
pub struct CollectionOpen {
    pub summary: CollectionSummary,
    pub items: Vec<BrowseCard>,
    pub page: usize,
    pub complete: bool,
    /// Items ticked for the queue. Every item is queued when none is ticked.
    pub selected: HashSet<u64>,
    pub job: Job<CollectionItemsLoad>,
    pub error: Option<String>,
}

pub struct CollectionItemsLoad {
    pub collection_id: u64,
    pub page: usize,
    pub result: Result<gamebanana::BrowsePage, String>,
}

/// A page of comments or changelog entries that landed after the first.
pub struct MoreLoad {
    pub id: u64,
    pub kind: MoreKind,
    pub page: usize,
    pub result: Result<MorePage, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoreKind {
    Comments,
    Updates,
}

pub enum MorePage {
    Comments(ModComments),
    Updates(ModUpdates),
}

/// Which installed state the grid keeps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Owned {
    #[default]
    Any,
    Installed,
    NotInstalled,
}

impl Owned {
    pub const ALL: [Owned; 3] = [Owned::Any, Owned::Installed, Owned::NotInstalled];
    fn label(self) -> &'static str {
        match self {
            Owned::Any => "All",
            Owned::Installed => "Installed",
            Owned::NotInstalled => "Not installed",
        }
    }

    /// The Filters menu's segment words, under the label "Installed".
    fn short(self) -> &'static str {
        match self {
            Owned::Any => "Any",
            Owned::Installed => "Yes",
            Owned::NotInstalled => "No",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Owned::Any => "any",
            Owned::Installed => "installed",
            Owned::NotInstalled => "notInstalled",
        }
    }

    fn from_id(id: &str) -> Owned {
        match id {
            "installed" => Owned::Installed,
            "notInstalled" => Owned::NotInstalled,
            _ => Owned::Any,
        }
    }
}

/// NSFW content in this view, on top of the app-wide setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MatureView {
    /// Follow Settings (show, blur or hide).
    #[default]
    Setting,
    Hide,
    Only,
}

impl MatureView {
    pub const ALL: [MatureView; 3] = [MatureView::Setting, MatureView::Hide, MatureView::Only];
    fn label(self) -> &'static str {
        match self {
            MatureView::Setting => "As in Settings",
            MatureView::Hide => "Hide NSFW",
            MatureView::Only => "NSFW only",
        }
    }

    /// The Filters menu's segment words.
    fn short(self) -> &'static str {
        match self {
            MatureView::Setting => "Setting",
            MatureView::Hide => "Hide",
            MatureView::Only => "Only",
        }
    }

    fn id(self) -> &'static str {
        match self {
            MatureView::Setting => "setting",
            MatureView::Hide => "hide",
            MatureView::Only => "only",
        }
    }

    fn from_id(id: &str) -> MatureView {
        match id {
            "hide" => MatureView::Hide,
            "only" => MatureView::Only,
            _ => MatureView::Setting,
        }
    }
}

/// A picked category or creator, by id with the name the chip shows.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Pick {
    pub id: u64,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Filters {
    pub section: BrowseSection,
    /// A root category, for example Skins.
    pub category: Option<Pick>,
    /// A child of `category`: a hero under Skins, a kind of HUD and so on.
    pub sub: Option<Pick>,
    /// A hero name match on sections without hero categories (sounds, WiPs).
    pub hero: Option<String>,
    /// `None` follows the Browse sort in Settings.
    pub order: Option<BrowseOrder>,
    pub period: BrowsePeriod,
    /// Which stamp the date window reads.
    pub date_field: DateField,
    /// A custom range, unix seconds, zero for open. Overrides `period` when set.
    pub date_from: i64,
    pub date_to: i64,
    pub featured: bool,
    pub creator: Option<Pick>,
    pub owned: Owned,
    pub mature: MatureView,
    pub hide_outdated: bool,
    /// Category, hero and creator picks exclude instead of include.
    pub exclude: bool,
    /// Show the saved mods instead of the catalog.
    pub favorites: bool,
}

impl Filters {
    /// Filters beyond the section and the order, as the chip row counts them.
    fn active(&self) -> usize {
        [
            self.category.is_some(),
            self.sub.is_some(),
            self.hero.is_some(),
            self.period != BrowsePeriod::Any || self.date_from > 0 || self.date_to > 0,
            self.featured,
            self.creator.is_some(),
            self.owned != Owned::Any,
            self.mature != MatureView::Setting,
            self.hide_outdated,
            self.exclude,
            self.favorites,
        ]
        .iter()
        .filter(|on| **on)
        .count()
    }

    fn window(&self) -> bool {
        self.period != BrowsePeriod::Any || self.date_from > 0 || self.date_to > 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterMenu {
    Category,
    Sub,
    Date,
    Sort,
    /// The Filters menu.
    More,
    View,
    /// The section tabs as one dropdown, on a narrow line.
    Section,
    /// More: refresh, paste, sources, hidden items, volume, the saved catalog.
    Tools,
}

impl FilterMenu {
    fn anchor(self) -> &'static str {
        match self {
            FilterMenu::Category => "browse-filter-category",
            FilterMenu::Sub => "browse-filter-sub",
            FilterMenu::Date => "browse-filter-date",
            FilterMenu::Sort => "browse-filter-sort",
            FilterMenu::More => "browse-filter-more",
            FilterMenu::View => "browse-filter-view",
            FilterMenu::Section => "browse-filter-section",
            FilterMenu::Tools => "browse-filter-tools",
        }
    }
}

/// Mods last changed before this are flagged outdated: the January 22, 2026 update moved
/// enough paths that older uploads often load as the game's own files (Grimoire's cutoff).
const OUTDATED_BEFORE: i64 = 1_769_040_000;
/// The titlebar search applies after this pause in typing.
const SEARCH_PAUSE: Duration = Duration::from_millis(350);
/// Automatic page reads in a row that may add no visible card before paging waits for a press.
const AUTO_EMPTY_PAGES: usize = 6;

/// A saved mod. Enough of the card to show it offline in the Favorites view.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Favorite {
    pub id: u64,
    pub model: String,
    pub name: String,
    pub author: String,
    pub author_id: u64,
    pub category: String,
    pub profile_url: String,
    pub thumbnail_url: Option<String>,
    pub audio_url: Option<String>,
    pub likes: u64,
    pub views: u64,
    pub added_ts: i64,
    pub updated_ts: i64,
    pub nsfw: bool,
    pub saved_at: i64,
}

impl Favorite {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "model": self.model,
            "name": self.name,
            "author": self.author,
            "authorId": self.author_id,
            "category": self.category,
            "profileUrl": self.profile_url,
            "thumbnailUrl": self.thumbnail_url,
            "audioUrl": self.audio_url,
            "likes": self.likes,
            "views": self.views,
            "addedTs": self.added_ts,
            "updatedTs": self.updated_ts,
            "nsfw": self.nsfw,
            "savedAt": self.saved_at,
        })
    }

    fn from_json(value: &serde_json::Value) -> Option<Self> {
        let text = |key: &str| value.get(key).and_then(|v| v.as_str()).unwrap_or_default().to_string();
        let opt = |key: &str| value.get(key).and_then(|v| v.as_str()).map(str::to_string);
        let num = |key: &str| value.get(key).and_then(|v| v.as_u64()).unwrap_or(0);
        let ts = |key: &str| value.get(key).and_then(|v| v.as_i64()).unwrap_or(0);
        let id = num("id");
        (id > 0).then(|| Favorite {
            id,
            model: Some(text("model")).filter(|m| !m.is_empty()).unwrap_or_else(|| "Mod".into()),
            name: text("name"),
            author: text("author"),
            author_id: num("authorId"),
            category: text("category"),
            profile_url: text("profileUrl"),
            thumbnail_url: opt("thumbnailUrl"),
            audio_url: opt("audioUrl"),
            likes: num("likes"),
            views: num("views"),
            added_ts: ts("addedTs"),
            updated_ts: ts("updatedTs"),
            nsfw: value.get("nsfw").and_then(|v| v.as_bool()).unwrap_or(false),
            saved_at: ts("savedAt"),
        })
    }

    fn from_card(card: &BrowseCard) -> Self {
        Favorite {
            id: card.id,
            model: card.model.clone(),
            name: card.name.clone(),
            author: card.author.clone(),
            author_id: card.author_id,
            category: card.category.clone(),
            profile_url: card.profile_url.clone(),
            thumbnail_url: card.thumbnail_url.clone(),
            audio_url: card.audio_url.clone(),
            likes: card.likes,
            views: card.views,
            added_ts: card.added_ts,
            updated_ts: card.updated_ts,
            nsfw: card.nsfw,
            saved_at: gamebanana::now_unix(),
        }
    }

    fn card(&self) -> BrowseCard {
        BrowseCard {
            id: self.id,
            name: self.name.clone(),
            author: self.author.clone(),
            category: self.category.clone(),
            updated_at: String::new(),
            likes: self.likes,
            views: self.views,
            profile_url: self.profile_url.clone(),
            thumbnail_url: self.thumbnail_url.clone(),
            version: None,
            added_ts: self.added_ts,
            updated_ts: self.updated_ts,
            nsfw: self.nsfw,
            model: self.model.clone(),
            audio_url: self.audio_url.clone(),
            featured: false,
            comments: 0,
            author_id: self.author_id,
            obsolete: false,
            sub_category_id: 0,
        }
    }
}

fn favorites_path() -> PathBuf {
    crate::data::data_dir().join("browse").join("favorites.json")
}

fn read_favorites() -> Vec<Favorite> {
    std::fs::read(favorites_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| value.as_array().cloned())
        .map(|list| list.iter().filter_map(Favorite::from_json).collect())
        .unwrap_or_default()
}

/// Write the list next to its final name, then rename it over, so a crash never leaves half
/// a file.
fn write_favorites(list: &[Favorite]) -> Result<(), String> {
    let path = favorites_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let list: Vec<serde_json::Value> = list.iter().map(Favorite::to_json).collect();
    let body = serde_json::to_vec_pretty(&list).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

fn favorites(s: &State) -> &[Favorite] {
    s.browse_view.favorites.as_deref().unwrap_or_default()
}

fn ensure_favorites(st: &mut State) {
    if st.browse_view.favorites.is_none() {
        st.browse_view.favorites = Some(if st.demo || rune::is_headless() {
            Vec::new()
        } else {
            read_favorites()
        });
    }
}

fn is_favorite(s: &State, model: &str, id: u64) -> bool {
    favorites(s)
        .iter()
        .any(|f| f.id == id && f.model.eq_ignore_ascii_case(model))
}

fn toggle_favorite(st: &mut State, card: Favorite) {
    ensure_favorites(st);
    let list = st.browse_view.favorites.get_or_insert_with(Vec::new);
    let before = list.len();
    list.retain(|f| !(f.id == card.id && f.model.eq_ignore_ascii_case(&card.model)));
    let saved = list.len() == before;
    if saved {
        list.insert(0, card.clone());
    }
    let list = list.clone();
    if !st.demo
        && !rune::is_headless()
        && let Err(error) = write_favorites(&list)
    {
        st.status = format!("Favorites could not be saved: {error}");
        return;
    }
    st.status = if saved {
        format!("Saved {} to favorites.", card.name)
    } else {
        format!("Removed {} from favorites.", card.name)
    };
    if st.browse_view.filters.favorites {
        crate::data::start_browse_fetch(st, 1);
    }
}

// --- query ---------------------------------------------------------------------

/// The GameBanana query the filters stand for.
pub fn current_query(s: &State) -> BrowseQuery {
    let f = &s.browse_view.filters;
    let mut order = f
        .order
        .unwrap_or_else(|| BrowseOrder::from_sort(s.prefs.browse_sort));
    if f.window() && order == BrowseOrder::MostDownloaded {
        // A dated window is ordered here, and list rows carry no download count.
        order = BrowseOrder::MostLiked;
    }
    // An excluded pick cannot be expressed on the server, so it is dropped here and applied
    // to the loaded rows in `shown_indices`.
    let category = if f.exclude {
        None
    } else {
        f.sub.as_ref().or(f.category.as_ref()).map(|pick| pick.id)
    };
    let typed = s.browse_view.applied_search.trim();
    let search = if typed.is_empty() {
        f.hero.clone().unwrap_or_default()
    } else {
        typed.to_string()
    };
    BrowseQuery {
        section: f.section,
        category,
        order,
        search,
        featured: f.featured,
        submitter: f.creator.as_ref().map(|pick| pick.id),
        period: f.period,
        date_field: f.date_field,
        date_from: f.date_from,
        date_to: f.date_to,
        all_models: s.browse_view.tab == Tab::All,
    }
}

/// A key for the catalog a query and the local views produce. Pages carry it, so one read
/// for other filters is never mixed into the grid.
pub fn query_key(s: &State, query: &BrowseQuery) -> u64 {
    let f = &s.browse_view.filters;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    query.hash(&mut hasher);
    f.favorites.hash(&mut hasher);
    (f.owned == Owned::Installed).hash(&mut hasher);
    s.browse_view.tab.hash(&mut hasher);
    hasher.finish()
}

/// The page a local view shows without the network: saved mods, or the installed mods the
/// library knows from GameBanana.
pub fn local_page(st: &mut State, key: u64) -> Option<BrowsePageView> {
    let f = st.browse_view.filters.clone();
    let mut cards: Vec<BrowseCard> = if f.favorites {
        ensure_favorites(st);
        let model = f.section.model();
        favorites(st)
            .iter()
            .filter(|fav| fav.model.eq_ignore_ascii_case(model))
            .map(Favorite::card)
            .collect()
    } else if f.owned == Owned::Installed {
        st.installed
            .library
            .mods
            .iter()
            .filter_map(|meta| {
                let id = meta.gamebanana_id?;
                Some(BrowseCard {
                    id,
                    name: meta.name.clone(),
                    author: meta.author.clone().unwrap_or_default(),
                    category: String::new(),
                    updated_at: String::new(),
                    likes: 0,
                    views: 0,
                    profile_url: format!("https://gamebanana.com/mods/{id}"),
                    thumbnail_url: meta.thumbnail_url.clone(),
                    version: None,
                    added_ts: 0,
                    updated_ts: 0,
                    nsfw: false,
                    model: "Mod".to_string(),
                    audio_url: None,
                    featured: false,
                    comments: 0,
                    author_id: 0,
                    obsolete: false,
                    sub_category_id: 0,
                })
            })
            .collect()
    } else {
        return None;
    };
    let mut seen = HashSet::new();
    cards.retain(|card| seen.insert((card.model.clone(), card.id)));
    // Thumbnails load as cards scroll into view (`store::track_visible`).
    Some(BrowsePageView {
        key,
        page: 1,
        per_page: cards.len().max(1),
        total: cards.len(),
        complete: true,
        page_count: 1,
        mods: cards,
    })
}

/// Order a dated window the way the grid shows it. List rows carry no download count, so
/// that order is never asked for a window (see `current_query`).
pub fn order_window(mods: &mut [BrowseMod], order: BrowseOrder) {
    match order {
        BrowseOrder::Newest => mods.sort_by_key(|m| std::cmp::Reverse(m.added_ts)),
        BrowseOrder::Updated | BrowseOrder::LatestComment => {
            mods.sort_by_key(|m| std::cmp::Reverse(m.updated_ts))
        }
        BrowseOrder::MostViewed => mods.sort_by_key(|m| std::cmp::Reverse(m.views)),
        BrowseOrder::MostCommented => mods.sort_by_key(|m| std::cmp::Reverse(m.comments)),
        BrowseOrder::AToZ => mods.sort_by_key(|m| m.name.to_lowercase()),
        BrowseOrder::Popular | BrowseOrder::MostLiked | BrowseOrder::MostDownloaded => {
            mods.sort_by_key(|m| std::cmp::Reverse(m.likes))
        }
    }
}

/// Change the filters and read page 1 for them. The grid keeps the old cards, dimmed, until
/// the new page lands.
fn set_filters(st: &mut State, tx: Option<&mut Tx>, edit: impl FnOnce(&mut Filters)) {
    let before = st.browse_view.filters.clone();
    edit(&mut st.browse_view.filters);
    // A change made in the Filters menu or one of its panels keeps Filters open.
    let in_filters = st.browse_view.menu == Some(FilterMenu::More) || st.browse_view.from_filters;
    st.browse_view.from_filters = false;
    st.browse_view.menu = in_filters.then_some(FilterMenu::More);
    if st.browse_view.filters == before {
        return;
    }
    if let Some(tx) = tx {
        tx.scroll_to_instant("browse-scroll", 0.0, 0.0);
    }
    st.browse_view.visible = 0..0;
    st.browse_view.refreshing = true;
    st.browse_view.auto_streak = 0;
    let section = st.browse_view.filters.section;
    start_categories(st, section);
    save_browse_state(st);
    if st.demo {
        return;
    }
    crate::data::start_browse_fetch(st, 1);
}

/// Switch tabs. Collections have their own read; Maps pins the Maps root; the rest reuse the
/// filter path so the chip row, sort and reload stay one behaviour.
fn set_tab(st: &mut State, tx: Option<&mut Tx>, tab: Tab) {
    if st.browse_view.tab == tab {
        return;
    }
    st.browse_view.tab = tab;
    st.browse_view.applied_search.clear();
    st.browse_query = TextEdit::new("");
    st.browse_view.search_edited = None;
    if let Some(tx) = tx {
        tx.scroll_to_instant("browse-scroll", 0.0, 0.0);
    }
    st.browse_view.visible = 0..0;
    st.browse_view.refreshing = true;
    st.browse_view.auto_streak = 0;
    st.browse_view.context = None;
    let mut map_name: Option<Pick> = None;
    if tab == Tab::Maps {
        map_name = categories_of(st, BrowseSection::Mods)
            .and_then(|list| list.iter().find(|root| root.name.eq_ignore_ascii_case("Maps")).cloned())
            .map(|root| Pick { id: root.id, name: root.name });
        if map_name.is_none() {
            map_name = Some(Pick { id: MAPS_CATEGORY, name: "Maps".into() });
        }
    }
    let hero = tree::picked_hero(st, &st.browse_view.filters);
    let mut filters = Filters {
        section: tab.section(),
        category: map_name,
        order: st.browse_view.filters.order,
        mature: st.browse_view.filters.mature,
        hide_outdated: st.browse_view.filters.hide_outdated,
        ..Filters::default()
    };
    // DMM 2.4.0: a hero pick follows the player from Mods to Sounds and WiPs and back.
    tree::follow_hero(st, hero, tab, &mut filters);
    st.browse_view.filters = filters;
    save_browse_state(st);
    if tab == Tab::Collections {
        st.browse_view.collections = None;
        start_collections(st, 1);
        return;
    }
    if tab != Tab::Collections {
        start_categories(st, tab.section());
    }
    if st.demo {
        return;
    }
    crate::data::start_browse_fetch(st, 1);
}

fn clear_filters(st: &mut State, tx: &mut Tx) {
    st.browse_query = TextEdit::new("");
    st.browse_view.applied_search.clear();
    st.browse_view.search_edited = None;
    st.browse_view.date_from_text = TextEdit::new("");
    st.browse_view.date_to_text = TextEdit::new("");
    set_filters(st, Some(tx), |f| {
        *f = Filters {
            section: f.section,
            ..Filters::default()
        }
    });
    if !st.demo {
        crate::data::start_browse_fetch(st, 1);
    }
}

fn categories_of(s: &State, section: BrowseSection) -> Option<&[BrowseCategory]> {
    s.browse_view
        .categories
        .iter()
        .find(|(owner, _)| *owner == section)
        .map(|(_, list)| list.as_slice())
}

/// Read the category tree of a section once per session.
fn start_categories(st: &mut State, section: BrowseSection) {
    if st.demo
        || rune::is_headless()
        || categories_of(st, section).is_some()
        || st.browse_view.categories_failed.contains(&section)
        || st.browse_view.categories_job.is_running()
    {
        return;
    }
    let wake = st.wake.clone();
    st.browse_view.categories_job.start(wake, move || {
        gamebanana::fetch_categories(section)
            .map(|list| (section, list))
            .map_err(|e| e.to_string())
    });
}

fn start_top(st: &mut State) {
    if st.demo || rune::is_headless() || st.browse_view.top.is_some() || st.browse_view.top_job.is_running() {
        return;
    }
    let wake = st.wake.clone();
    st.browse_view
        .top_job
        .start(wake, || gamebanana::fetch_top_subs().map_err(|e| e.to_string()));
}

fn is_installed(s: &State, id: u64) -> bool {
    quick::owned_slot(s, id).is_some()
}

/// The installed GameBanana mod ids, from the library store and the install journal.
fn installed_ids(s: &State) -> HashSet<u64> {
    let mut ids: HashSet<u64> = s.browse_view.installs.owned.keys().copied().collect();
    ids.extend(s.installed.library.mods.iter().filter_map(|meta| meta.gamebanana_id));
    ids
}

/// A pasted GameBanana id or link: `741478`, `gamebanana.com/mods/741478`,
/// `.../sounds/83198`, `.../wips/105160`.
fn pasted_submission(text: &str) -> Option<(BrowseSection, u64)> {
    let text = text.trim().trim_end_matches('/');
    if let Ok(id) = text.parse::<u64>() {
        return (id > 0).then_some((BrowseSection::Mods, id));
    }
    let rest = text.split("gamebanana.com/").nth(1)?;
    let mut parts = rest.split(['/', '?', '#']);
    let section = match parts.next()? {
        "mods" => BrowseSection::Mods,
        "sounds" => BrowseSection::Sounds,
        "wips" => BrowseSection::Wips,
        _ => return None,
    };
    let id = parts.next()?.parse::<u64>().ok()?;
    Some((section, id))
}

// --- catalog -------------------------------------------------------------------

fn card_by_id(s: &State, id: u64) -> Option<&BrowseCard> {
    s.browse
        .page
        .as_ref()
        .and_then(|page| page.mods.iter().find(|card| card.id == id))
}

fn matches_query(card: &BrowseCard, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    card.name.to_lowercase().contains(query)
        || card.author.to_lowercase().contains(query)
        || card.category.to_lowercase().contains(query)
}

/// The card is older than the last path-moving game update, or its creator retired it.
fn outdated(card: &BrowseCard) -> bool {
    card.obsolete || (card.updated_ts > 0 && card.updated_ts < OUTDATED_BEFORE)
}

/// True when a card's name or category mentions `hero`, folded so "Mo & Krill" matches.
fn mentions_hero(card: &BrowseCard, hero: &str) -> bool {
    let hero = catalog_mirror::fold(hero);
    if hero.is_empty() {
        return false;
    }
    catalog_mirror::fold(&card.name).contains(&hero)
        || catalog_mirror::fold(&card.category).contains(&hero)
}

/// True when a card is inside the picked category, by folded name.
fn in_category(card: &BrowseCard, category: &str) -> bool {
    let category = catalog_mirror::fold(category);
    card.category
        .split('/')
        .next()
        .is_some_and(|root| catalog_mirror::fold(root) == category)
}

/// True when a card matches the picked sub category, by folded name or by a name mention.
fn in_sub(card: &BrowseCard, sub: &str) -> bool {
    let sub = catalog_mirror::fold(sub);
    !sub.is_empty()
        && (catalog_mirror::fold(&card.category).contains(&sub)
            || catalog_mirror::fold(&card.name).contains(&sub))
}

/// Indices into the catalog of the cards the grid shows: hidden mods and creators left out,
/// and the filters GameBanana cannot apply (installed, mature, outdated, exclude, dates)
/// applied here. The typed search filters the loaded cards at once while the server search
/// waits for a pause, and always in the local views.
fn shown_indices(s: &State) -> Vec<usize> {
    let v = &s.browse_view;
    let f = &v.filters;
    let typed = s.browse_query.text.trim().to_lowercase();
    let local = f.favorites || f.owned == Owned::Installed;
    let query = if local || typed != v.applied_search.to_lowercase() {
        typed
    } else {
        String::new()
    };
    let installed = if f.owned == Owned::NotInstalled {
        installed_ids(s)
    } else {
        HashSet::new()
    };
    let hide_mature = s.prefs.mature == crate::prefs::Mature::Hide || f.mature == MatureView::Hide;
    // A pick holds the categories under it: Haze holds its styles (`tree.rs`).
    let sub_ids = f.sub.as_ref().map(|pick| tree::subtree(s, f.section, pick.id)).unwrap_or_default();
    let sub_complete = f.sub.as_ref().is_none_or(|pick| tree::subtree_complete(s, f.section, pick.id));
    let hero_ids = f.hero.as_deref().map(|hero| tree::hero_ids(s, f.section, hero)).unwrap_or_default();
    let in_sub = |card: &BrowseCard, name: &str| {
        if card.sub_category_id > 0 && !sub_ids.is_empty() {
            // Until the third level is read, the server's filter stands for an included pick.
            sub_ids.contains(&card.sub_category_id) || (!sub_complete && !f.exclude)
        } else {
            in_sub(card, name)
        }
    };
    let mentions_hero = |card: &BrowseCard, hero: &str| {
        (card.sub_category_id > 0 && hero_ids.contains(&card.sub_category_id)) || mentions_hero(card, hero)
    };
    let window_start = if f.date_from > 0 {
        f.date_from
    } else {
        f.period.cutoff(gamebanana::now_unix()).unwrap_or(0)
    };
    let window_end = f.date_to;
    let stamp = |card: &BrowseCard| match f.date_field {
        DateField::Added => card.added_ts,
        DateField::Updated => card.updated_ts,
    };
    s.browse
        .page
        .as_ref()
        .map(|page| {
            page.mods
                .iter()
                .enumerate()
                .filter(|(_, card)| {
                    if !content_allowed(s, card.id, false, &card.author) {
                        return false;
                    }
                    if card.nsfw && hide_mature && f.mature != MatureView::Only {
                        return false;
                    }
                    if f.mature == MatureView::Only && !card.nsfw {
                        return false;
                    }
                    if f.hide_outdated && outdated(card) {
                        return false;
                    }
                    if f.owned == Owned::NotInstalled
                        && card.model == "Mod"
                        && installed.contains(&card.id)
                    {
                        return false;
                    }
                    if window_start > 0 && stamp(card) > 0 && stamp(card) < window_start {
                        return false;
                    }
                    if window_end > 0 && stamp(card) > window_end {
                        return false;
                    }
                    if f.exclude {
                        if f
                            .category
                            .as_ref()
                            .is_some_and(|pick| in_category(card, &pick.name))
                        {
                            return false;
                        }
                        if f.sub
                            .as_ref()
                            .is_some_and(|pick| in_sub(card, &pick.name))
                        {
                            return false;
                        }
                        if f.hero.as_ref().is_some_and(|hero| mentions_hero(card, hero)) {
                            return false;
                        }
                    } else {
                        if f
                            .sub
                            .as_ref()
                            .is_some_and(|pick| !in_sub(card, &pick.name))
                        {
                            return false;
                        }
                        if f.hero.as_ref().is_some_and(|hero| !mentions_hero(card, hero)) {
                            return false;
                        }
                    }
                    matches_query(card, &query)
                })
                .map(|(index, _)| index)
                .collect()
        })
        .unwrap_or_default()
}

/// Columns and card width for a grid `width` points wide.
/// The one gate every Browse surface (grid, shelves, spotlight, image queue) shares: a hidden
/// mod or creator never shows, and a mature one never shows while Settings hide NSFW content.
/// `nsfw` is false where the caller applies its own mature rule (the grid's Mature filter).
pub(crate) fn content_allowed(s: &State, id: u64, nsfw: bool, author: &str) -> bool {
    !(nsfw && s.prefs.mature == crate::prefs::Mature::Hide) && !mod_hidden(s, id) && !author_hidden(s, author)
}

fn grid_columns(width: f32, size: CardSize) -> (usize, f32) {
    let (min, max) = size.bounds();
    let width = if width > 0.0 { width } else { DEFAULT_GRID_W };
    let mut cols = ((width + GRID_GAP) / (max + GRID_GAP)).ceil().max(1.0) as usize;
    let card = |cols: usize| (width - GRID_GAP * (cols as f32 - 1.0)) / cols as f32;
    while cols > 1 && card(cols) < min {
        cols -= 1;
    }
    (cols, card(cols).max(0.0))
}

fn more_to_load(s: &State) -> bool {
    !s.browse_view.paged
        && s.browse
            .page
            .as_ref()
            .is_some_and(|page| !page.complete && page.page < page.page_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pasted_ids_and_links_open_a_submission() {
        assert_eq!(pasted_submission("741478"), Some((BrowseSection::Mods, 741478)));
        assert_eq!(
            pasted_submission("https://gamebanana.com/sounds/83198/"),
            Some((BrowseSection::Sounds, 83198))
        );
        assert_eq!(
            pasted_submission("gamebanana.com/wips/105160?x=1"),
            Some((BrowseSection::Wips, 105160))
        );
        assert_eq!(pasted_submission("long hair"), None);
        assert_eq!(pasted_submission("0"), None);
        assert_eq!(pasted_submission("https://gamebanana.com/members/7"), None);
    }

    #[test]
    fn a_window_orders_itself() {
        let row = |id: u64, likes: u64, added: i64| BrowseMod {
            id,
            name: format!("m{id}"),
            profile_url: String::new(),
            model: "Mod".into(),
            author: String::new(),
            category: String::new(),
            subcategory: String::new(),
            sub_category_id: 0,
            version: None,
            updated_at: String::new(),
            likes,
            views: 0,
            has_files: true,
            thumbnail_url: None,
            added_ts: added,
            updated_ts: added,
            nsfw: false,
            audio_url: None,
            featured: false,
            comments: 0,
            author_id: 0,
            obsolete: false,
        };
        let mut mods = vec![row(1, 5, 30), row(2, 9, 10), row(3, 1, 20)];
        order_window(&mut mods, BrowseOrder::MostLiked);
        assert_eq!(mods.iter().map(|m| m.id).collect::<Vec<_>>(), vec![2, 1, 3]);
        order_window(&mut mods, BrowseOrder::Newest);
        assert_eq!(mods.iter().map(|m| m.id).collect::<Vec<_>>(), vec![1, 3, 2]);
    }

    #[test]
    fn dates_round_trip_as_the_input_shape() {
        assert_eq!(parse_date("2026-01-22"), Some(1_769_040_000));
        assert_eq!(date_input(1_769_040_000), "2026-01-22");
        assert_eq!(parse_date("2026-1-2"), Some(1_767_312_000));
        assert_eq!(parse_date("2026-13-01"), None);
        assert_eq!(parse_date("2026-01-32"), None);
        assert_eq!(parse_date("22-01-2026"), None);
        assert_eq!(parse_date(""), None);
    }

    #[test]
    fn favorites_round_trip_through_json() {
        let favorite = Favorite {
            id: 83198,
            model: "Sound".into(),
            name: "Drifter Flow".into(),
            author: "Hopelessone".into(),
            author_id: 4913678,
            category: "Killsounds".into(),
            profile_url: "https://gamebanana.com/sounds/83198".into(),
            thumbnail_url: None,
            audio_url: Some("https://files.gamebanana.com/preview/sounds/a.mp3".into()),
            likes: 60,
            views: 50015,
            added_ts: 1,
            updated_ts: 2,
            nsfw: true,
            saved_at: 3,
        };
        assert_eq!(Favorite::from_json(&favorite.to_json()), Some(favorite));
        assert_eq!(Favorite::from_json(&serde_json::json!({"name": "no id"})), None);
    }
}

/// Close the mod's details: the app calls this when the player leaves Browse.
pub fn close_details(st: &mut crate::app::State) {
    detail_state::close_details(st);
}
