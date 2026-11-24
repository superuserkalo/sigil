//! The Browse landing top (`design/browse-top.md`): a billboard with its segment strip, then
//! image-led rows (Top 10 this week, New this week, up to three "Trending in" category rows,
//! Popular this month, All-time favourites).
//!
//! One pure `plan` decides what is shown; the view and the cover queue both read it, so a tile
//! that is drawn always has its cover asked for and a hidden mod is never drawn or fetched.
//! Every box is sized from the page width alone, so nothing moves when images land. Hover is
//! paint only. The billboard advances by itself every 8 seconds through one timer wake
//! (`tick`), never a running animation, and pauses for everything in spec section 2.3.

use super::*;
use crate::screens::browse::grid::{image_fill, nsfw_tag, pixels_for, veiled};
use crate::ui::{BILLBOARD, RANK, SECTION};

/// Ink on a white control drawn over art: the billboard's near black, `#0c0c0e`.
pub const INK_ON_WHITE: Color = Color { r: 0.0036, g: 0.0036, b: 0.0044, a: 1.0 };
/// The medium billboard title, 22/28.
const BILLBOARD_S: TextStyle = TextStyle::new(22.0, 600.0).lh(28.0).tracking(-0.2);

/// Space between tiles, and the slice of the next tile left showing at a row's right edge.
pub const GAP: f32 = 8.0;
pub const PEEK: f32 = 40.0;
/// One pager bar on the billboard.
const SEG: f32 = 32.0;
/// Tiles a row holds at most.
pub const MAX_ROW: usize = 20;
/// Billboard picks at most.
pub const MAX_PICKS: usize = 6;
/// Seconds between automatic billboard steps.
pub const ADVANCE: Duration = Duration::from_secs(7);
/// No input for this long and the billboard stops until the next one.
pub const IDLE: Duration = Duration::from_secs(120);
/// The active pager bar fills in this many steps over `ADVANCE`: one small commit each, and the
/// bar glides between them on `motion::countdown`.
pub const STEPS: u8 = 14;

/// The original upload behind a GameBanana thumbnail: `.../530-90_a.jpg` is `.../a.jpg`. The
/// billboard draws it once it is decoded; the thumbnail stands in until then.
pub fn large_of(url: &str) -> String {
    for size in ["/530-90_", "/220-90_", "/100-90_"] {
        if let Some(at) = url.rfind(size) {
            return format!("{}/{}", &url[..at], &url[at + size.len()..]);
        }
    }
    url.to_string()
}

/// A pointer move or key on Browse: the billboard counts from here. When it had stopped for
/// idleness, the poll is woken once to arm a full interval again. Never commits.
pub fn note_input(st: &mut State, tx: &mut Tx) {
    tx.live_only();
    note_key(st, tx);
}

/// `note_input` for a key passing through: the key may still act, so this never marks the
/// event live only.
pub fn note_key(st: &mut State, tx: &mut Tx) {
    let was_idle = st.browse_view.shelves.last_input.is_none_or(|t| t.elapsed() >= IDLE);
    st.browse_view.shelves.last_input = Some(Instant::now());
    if was_idle {
        tx.wake_after(Duration::ZERO);
    }
}
const DAY: i64 = 24 * 60 * 60;

// --- the plan ------------------------------------------------------------------------------

/// Which row, which is also its page slot in `Shelves::pages`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Top10,
    New,
}

impl RowKind {
    pub fn slot(self) -> usize {
        match self {
            RowKind::Top10 => 0,
            RowKind::New => 1,
        }
    }
}

/// What a row's title link applies to the grid.
#[derive(Clone, Debug, PartialEq)]
pub struct See {
    pub order: BrowseOrder,
    pub period: BrowsePeriod,
    pub category: Option<String>,
}

pub struct Row<'a> {
    pub kind: RowKind,
    /// The title, already in the player's language.
    pub title: String,
    pub cards: Vec<&'a BrowseCard>,
    /// What the title link applies to the grid; a row picked for the player has none.
    pub see: Option<See>,
    /// Why each mod is here, shown on its tile ("Because you play Haze").
    pub reasons: Option<&'a std::collections::HashMap<u64, super::foryou::Reason>>,
}

/// One billboard slide.
pub struct Slide {
    pub card: BrowseCard,
    /// "Trending today" or "This week in Skins".
    pub eyebrow: String,
    pub description: String,
    /// The large image: the top list's 800 px art, else the original upload.
    pub image: Option<String>,
    /// The thumbnail, shown until the large image is decoded or when it cannot be read.
    pub small: Option<String>,
}

#[derive(Default)]
pub struct Plan<'a> {
    pub billboard: Vec<Slide>,
    pub rows: Vec<Row<'a>>,
    /// False until any read has landed: the view reserves placeholders.
    pub known: bool,
}

/// The root category of a card's path: "Skins" of "Skins / Haze".
pub fn root_category(card: &BrowseCard) -> &str {
    card.category.split(" / ").next().unwrap_or("").trim()
}

/// A top-list row as a card, so details open with its mature flag, author and link.
pub fn card_of(t: &TopSub) -> BrowseCard {
    BrowseCard {
        id: t.id,
        name: t.name.clone(),
        author: t.author.clone(),
        category: t.category.clone(),
        updated_at: String::new(),
        likes: t.likes,
        views: 0,
        profile_url: t.profile_url.clone(),
        thumbnail_url: t.image_url.clone(),
        version: None,
        added_ts: 0,
        updated_ts: 0,
        nsfw: t.nsfw,
        model: "Mod".to_string(),
        audio_url: None,
        featured: false,
        comments: 0,
        author_id: 0,
        obsolete: false,
        sub_category_id: 0,
    }
}

/// Tiles per page for a page `width` points wide.
pub fn per_page(width: f32) -> usize {
    if width >= 960.0 {
        4
    } else if width >= 720.0 {
        3
    } else if width >= 528.0 {
        2
    } else {
        1
    }
}

/// One tile's width and height: `n` tiles, their gaps and the peek fill the page.
pub fn tile_size(width: f32, n: usize) -> (f32, f32) {
    let w = ((width - PEEK - GAP * n as f32) / n as f32).floor().max(1.0);
    (w, (w * 9.0 / 16.0).round())
}

/// The pages a row of `len` tiles has at `n` a page: every page shows `n` whole tiles, the last
/// one aligned to the row's end.
pub fn page_count(len: usize, n: usize) -> usize {
    len.div_ceil(n.max(1)).max(1)
}

/// The tiles page `page` draws (`n + 1`, the last cut by the clip) and the strip's x offset:
/// 0, or for the last page the offset that ends the row flush right with the previous tile
/// peeking at the left.
pub fn page_window(len: usize, n: usize, page: usize, width: f32) -> (usize, usize, f32) {
    let (tw, _) = tile_size(width, n);
    let pages = page_count(len, n);
    let page = page.min(pages - 1);
    if page + 1 < pages || len <= n {
        let start = page * n;
        return (start, (start + n + 1).min(len), 0.0);
    }
    let start = len.saturating_sub(n + 1);
    let count = (len - start) as f32;
    (start, len, width - (count * tw + (count - 1.0) * GAP))
}

/// The tiles page `page` shows whole (the peek excluded).
pub fn whole_tiles(len: usize, n: usize, page: usize) -> std::ops::Range<usize> {
    let pages = page_count(len, n);
    let page = page.min(pages - 1);
    if page + 1 < pages || len <= n { page * n..(page * n + n).min(len) } else { len - n..len }
}

/// The landing's plan for the page width's `n` and the clock `now`: what the billboard and
/// each row show, after the gate (hidden mods and creators, mature under Hide), with no mod
/// twice and every row a full page plus its peek or not there at all.
pub fn plan(s: &State, n: usize, now: i64) -> Plan<'_> {
    let v = &s.browse_view;
    let feeds = v.shelves.feeds.as_ref();
    let known = feeds.is_some() || v.top.is_some() || !v.w4.featured.is_empty();
    let ok = |c: &&BrowseCard| {
        content_allowed(s, c.id, c.nsfw, &c.author) && !c.obsolete && c.thumbnail_url.is_some()
    };
    let none: &[BrowseCard] = &[];
    let recent: Vec<&BrowseCard> = feeds.map_or(none, |f| &f.recent[..]).iter().filter(ok).collect();
    let liked: Vec<&BrowseCard> = feeds.map_or(none, |f| &f.liked[..]).iter().filter(ok).collect();
    let within = |c: &BrowseCard, days: i64| c.added_ts >= now - days * DAY;
    let by_likes = |list: &mut Vec<&BrowseCard>| list.sort_by(|a, b| b.likes.cmp(&a.likes).then(b.added_ts.cmp(&a.added_ts)));
    let min_fill = n + 1;
    // Blur and Hide never lead with a mature mod: a large blurred lead is a dead banner. Show
    // treats mature mods as any other.
    let mature_lead = s.prefs.mature == crate::prefs::Mature::Show;
    let failed = |url: &Option<String>| url.as_deref().is_none_or(|u| u.is_empty() || s.thumb_crew.failed(u));

    // The billboard: today's first top mod, then the week's pick in each category.
    let mut billboard: Vec<Slide> = Vec::new();
    let mut lead: HashSet<u64> = HashSet::new();
    let top: &[TopSub] = v.top.as_deref().unwrap_or(&[]);
    let top_ok = |t: &TopSub| {
        t.model.eq_ignore_ascii_case("mod")
            && content_allowed(s, t.id, t.nsfw, &t.author)
            && (mature_lead || !t.nsfw)
            && !failed(&t.image_url)
    };
    let push_top = |billboard: &mut Vec<Slide>, lead: &mut HashSet<u64>, t: &TopSub| {
        if billboard.len() < MAX_PICKS && lead.insert(t.id) {
            billboard.push(Slide {
                card: card_of(t),
                eyebrow: super::tree::tr(match t.period.as_str() {
                    "today" => "Trending today",
                    "week" => "Trending this week",
                    _ => "Popular this month",
                })
                .to_string(),
                description: t.description.clone(),
                image: t.image_url.clone(),
                small: t.image_url.clone(),
            });
        }
    };
    if let Some(t) = top.iter().find(|t| t.period == "today" && top_ok(t)) {
        push_top(&mut billboard, &mut lead, t);
    }
    for (category, card) in &v.w4.featured {
        if billboard.len() >= MAX_PICKS {
            break;
        }
        let allowed = content_allowed(s, card.id, card.nsfw, &card.author)
            && (mature_lead || !card.nsfw)
            && !card.obsolete
            && !failed(&card.thumbnail_url);
        if allowed && lead.insert(card.id) {
            billboard.push(Slide {
                card: super::store::clone_card(card),
                eyebrow: crate::screens::settings::i18n::fill(super::tree::tr("This week in {}"), &[category]),
                description: String::new(),
                image: card.thumbnail_url.as_deref().map(large_of),
                small: card.thumbnail_url.clone(),
            });
        }
    }
    if billboard.len() < 2 && v.w4.featured.is_empty() {
        for period in ["today", "week", "month"] {
            for t in top.iter().filter(|t| t.period == period && top_ok(t)) {
                push_top(&mut billboard, &mut lead, t);
            }
        }
    }

    let mut rows: Vec<Row> = Vec::new();
    let mut taken: HashSet<u64> = HashSet::new();
    let see = |order, period, category: Option<&str>| Some(See { order, period, category: category.map(str::to_string) });

    // Top 10: the true ranking, so it may repeat a billboard pick. The title names the window.
    let windows: [(Option<i64>, &str, BrowsePeriod); 4] = [
        (Some(7), "Top 10 this week", BrowsePeriod::Week),
        (Some(14), "Top 10 of the last 14 days", BrowsePeriod::Month),
        (Some(30), "Top 10 this month", BrowsePeriod::Month),
        (None, "Top 10 of all time", BrowsePeriod::Any),
    ];
    for (days, title, period) in windows {
        let mut list: Vec<&BrowseCard> = match days {
            Some(days) => recent.iter().copied().filter(|c| within(c, days)).collect(),
            None => liked.clone(),
        };
        by_likes(&mut list);
        if list.len() >= 10 {
            list.truncate(10);
            taken.extend(list.iter().map(|c| c.id));
            rows.push(Row { kind: RowKind::Top10, title: super::tree::tr(title).to_string(), cards: list, see: see(BrowseOrder::MostLiked, period, None), reasons: None });
            break;
        }
    }
    taken.extend(lead.iter().copied());
    // The second row is the player's own: picked from their heroes, mods and favorites, or
    // "Rising" while there is no history. New this week stands in until those picks exist.
    let fy = &v.shelves.for_you;
    let mine = fresh_cards(fy.picks.iter().filter(ok).collect(), &taken);
    if mine.len() >= min_fill {
        let title = super::tree::tr(if fy.personal { "For you" } else { "Rising" }).to_string();
        let see = if fy.personal { None } else { see(BrowseOrder::MostLiked, BrowsePeriod::Week, None) };
        push_row(&mut rows, &mut taken, min_fill, Row { kind: RowKind::New, title, cards: mine, see, reasons: Some(&fy.reasons) });
    }
    // New this week: newest first, liked at least once; then without the floor; then 14 days.
    let newest = |days: i64, floor: u64| -> Vec<&BrowseCard> {
        let mut list: Vec<&BrowseCard> = recent.iter().copied().filter(|c| within(c, days) && c.likes >= floor).collect();
        list.sort_by(|a, b| b.added_ts.cmp(&a.added_ts));
        list
    };
    for (days, floor, title, period) in [
        (7, 1, "New this week", BrowsePeriod::Week),
        (7, 0, "New this week", BrowsePeriod::Week),
        (14, 0, "New mods", BrowsePeriod::Month),
        (30, 0, "New mods", BrowsePeriod::Month),
    ] {
        if rows.iter().any(|r| r.kind == RowKind::New) {
            break;
        }
        let cards = fresh_cards(newest(days, floor), &taken);
        if cards.len() >= min_fill {
            push_row(&mut rows, &mut taken, min_fill, Row { kind: RowKind::New, title: super::tree::tr(title).to_string(), cards, see: see(BrowseOrder::Newest, period, None), reasons: None });
            break;
        }
    }

    // Two rows only: the catalog starts right under them.
    // No saved catalog and nothing on the top list: the Top 10's head leads.
    if billboard.is_empty()
        && let Some(card) = rows.first().and_then(|r| r.cards.iter().find(|c| mature_lead || !c.nsfw))
    {
        billboard.push(Slide {
            card: super::store::clone_card(card),
            eyebrow: rows[0].title.clone(),
            description: String::new(),
            image: card.thumbnail_url.as_deref().map(large_of),
            small: card.thumbnail_url.clone(),
        });
    }
    Plan { billboard, rows, known }
}

/// The mods of `list` no earlier row took, once each, at most `MAX_ROW`.
fn fresh_cards<'a>(list: Vec<&'a BrowseCard>, taken: &HashSet<u64>) -> Vec<&'a BrowseCard> {
    let mut seen = HashSet::new();
    list.into_iter().filter(|c| !taken.contains(&c.id) && seen.insert(c.id)).take(MAX_ROW).collect()
}

/// Add a row only when it holds a page and its peek; its mods are then taken.
fn push_row<'a>(rows: &mut Vec<Row<'a>>, taken: &mut HashSet<u64>, min_fill: usize, row: Row<'a>) {
    if row.cards.len() >= min_fill {
        taken.extend(row.cards.iter().map(|c| c.id));
        rows.push(row);
    }
}

/// The covers the landing shows now and soon: the billboard's pick and its two neighbours,
/// and for each row its current page and the next. Read from the same plan the view draws.
pub fn cover_urls(s: &State) -> (Vec<String>, Vec<String>) {
    let width = super::toolbar::page_width(s);
    let n = per_page(width);
    let p = plan(s, n, gamebanana::now_unix());
    let mut now = Vec::new();
    let mut next = Vec::new();
    let count = p.billboard.len();
    if count > 0 {
        let at = s.browse_view.shelves.spot % count;
        for (i, pick) in p.billboard.iter().enumerate() {
            let near = i == (at + 1) % count || i == (at + count - 1) % count;
            for url in [pick.small.clone(), pick.image.clone()].into_iter().flatten() {
                if i == at {
                    now.push(url);
                } else if near {
                    next.push(url);
                }
            }
        }
    }
    for (index, row) in p.rows.iter().enumerate() {
        let page = s.browse_view.shelves.pages[row.kind.slot()];
        let pages = page_count(row.cards.len(), n);
        let (a, b, _) = page_window(row.cards.len(), n, page, width);
        let (c, d, _) = page_window(row.cards.len(), n, (page + 1).min(pages - 1), width);
        for (i, card) in row.cards.iter().enumerate() {
            let Some(url) = card.thumbnail_url.clone() else { continue };
            if (a..b).contains(&i) && index < 2 {
                now.push(url);
            } else if (a..b).contains(&i) || (c..d).contains(&i) {
                next.push(url);
            }
        }
    }
    (now, next)
}

// --- the billboard's own timer -------------------------------------------------------------

/// Why the billboard does not advance by itself right now, if it does not.
pub fn paused(s: &State, focused: Option<&str>) -> Option<&'static str> {
    let v = &s.browse_view;
    if crate::motion::reduced() || s.prefs.reduce_motion {
        return Some("reduce motion");
    }
    if !s.prefs.browse_auto_advance {
        return Some("setting");
    }
    if s.screen != Screen::Browse || v.details.is_some() || v.menu.is_some() || v.context.is_some() {
        return Some("not in view");
    }
    if !super::grid::landing_shown(s) {
        return Some("not on the landing");
    }
    if v.shelves.hovered || v.shelves.window_inactive {
        return Some("hover or background");
    }
    if v.shelves.offscreen {
        return Some("scrolled past");
    }
    if v.shelves.last_input.is_some_and(|t| t.elapsed() >= IDLE) {
        return Some("idle");
    }
    // Focus inside the billboard does not pause it: a press on a pager button leaves focus
    // there, which used to stop the billboard for good after the first manual step.
    let _ = focused;
    if v.audio.playing.is_some() {
        return Some("sample");
    }
    let at = v.shelves.spot;
    let width = super::toolbar::page_width(s);
    let p = plan(s, per_page(width), gamebanana::now_unix());
    if p.billboard.len() < 2 {
        return Some("one pick");
    }
    if p.billboard.get(at % p.billboard.len()).is_some_and(|pick| v.installs.items.get(&pick.card.id).is_some_and(|e| e.busy())) {
        return Some("installing");
    }
    None
}

/// Count the billboard's interval in `STEPS` small steps and move to the next pick when it is
/// full. One wake per step while it runs; while it is paused nothing is armed and the bar
/// keeps its place, so it goes on from there. True when the view must run.
pub fn tick(st: &mut State, tx: &mut Tx) -> bool {
    // The first look counts as input: a fresh window is not idle.
    if st.browse_view.shelves.last_input.is_none() {
        st.browse_view.shelves.last_input = Some(Instant::now());
    }
    let focused = tx.focused().map(str::to_string);
    if paused(st, focused.as_deref()).is_some() {
        st.browse_view.shelves.due = None;
        return false;
    }
    let step = ADVANCE / STEPS as u32;
    let now = Instant::now();
    let Some(due) = st.browse_view.shelves.due else {
        st.browse_view.shelves.due = Some(now + step);
        tx.wake_after(step);
        return false;
    };
    if now < due {
        tx.wake_after(due - now);
        return false;
    }
    st.browse_view.shelves.due = Some(now + step);
    tx.wake_after(step);
    // The bar glides to its new length (`motion::countdown`); nothing else moves.
    tx.animated();
    if st.browse_view.shelves.step + 1 < STEPS {
        st.browse_view.shelves.step += 1;
        return true;
    }
    st.browse_view.shelves.step = 0;
    // Step only to a pick whose image is decoded: the next ready one, or count again.
    let width = super::toolbar::page_width(st);
    let p = plan(st, per_page(width), gamebanana::now_unix());
    let count = p.billboard.len();
    let at = st.browse_view.shelves.spot % count.max(1);
    let ready = (1..count).map(|k| (at + k) % count).find(|&i| {
        let pick = &p.billboard[i];
        [&pick.image, &pick.small].into_iter().flatten().any(|url| st.thumbnails.contains_key(url))
    });
    drop(p);
    if let Some(next) = ready {
        st.browse_view.shelves.spot = next;
        st.browse_view.shelves.spot_dir = 1;
    }
    true
}

/// A manual step: at once, and the interval starts over.
fn step_to(st: &mut State, index: usize, dir: i8) {
    st.browse_view.shelves.spot = index;
    st.browse_view.shelves.spot_dir = dir;
    st.browse_view.shelves.step = 0;
    st.browse_view.shelves.due = None;
}

// --- the view ------------------------------------------------------------------------------

pub fn billboard_height(width: f32) -> f32 {
    if width >= 960.0 {
        288.0
    } else if width >= 720.0 {
        248.0
    } else {
        200.0
    }
}

/// The landing top: billboard and strip, then the rows. Placeholders at their final size
/// until a read lands.
pub fn view(s: &State, u: &Ui) -> El {
    let width = super::toolbar::page_width(s);
    let n = per_page(width);
    let p = plan(s, n, gamebanana::now_unix());
    let mut c = col().key("browse-dashboard").w_full();
    if !p.known {
        c = c.child(billboard_placeholder(u, width));
        for i in 0..2 {
            c = c.child(row_placeholder(u, width, n, i).mt(if i == 0 { 32.0 } else { 28.0 }));
        }
        return c.child(el().h(32.0));
    }
    if !p.billboard.is_empty() {
        c = c.child(billboard(s, u, &p.billboard, width));
    }
    for (i, r) in p.rows.iter().enumerate() {
        let top = if i == 0 && p.billboard.is_empty() { 0.0 } else { 24.0 };
        c = c.child(row_view(s, u, r, width, n).mt(top));
    }
    c.child(el().h(32.0))
}

fn bar(u: &Ui, w: f32, h: f32) -> El {
    el().w(w).h(h).radius(crate::ui::r(4.0)).fill(u.hover)
}

fn billboard_placeholder(u: &Ui, width: f32) -> El {
    let h = billboard_height(width);
    let r = crate::ui::r(8.0);
    col().key("bb-placeholder")
        .child(
            col().w_full()
                .h(h)
                .radius(r)
                .fill(u.well)
                .padding(32.0)
                .gap(12.0)
                .justify_end()
                .child(bar(u, 220.0, 20.0))
                .child(bar(u, 160.0, 12.0))
                .child(bar(u, 120.0, 32.0).mt(12.0)),
        )
}

fn row_placeholder(u: &Ui, width: f32, n: usize, i: usize) -> El {
    let (tw, th) = tile_size(width, n);
    let r = crate::ui::r(8.0);
    col().key(format!("top-row-placeholder-{i}"))
        .w_full()
        .gap(8.0)
        .child(row().h(24.0).center_y().child(bar(u, 120.0, 10.0)))
        .child(
            row().w_full().h(th).gap(GAP).clip_rounded(0.0).children((0..=n).map(|_| {
                el().size(tw, th)
                    .min_w(tw)
                    .radius(r)
                    .fill(u.well)
                    .border(1.0, u.hair)
                    .child(bar(u, (tw * 0.6).floor(), 10.0).pin(Some(10.0), None, None, Some(12.0)))
            })),
        )
}

/// The billboard: blurred backdrop, sharp art at the right, scrim, text and actions, arrows.
fn billboard(s: &State, u: &Ui, picks: &[Slide], width: f32) -> El {
    let h = billboard_height(width);
    let (title_font, title_h, desc_lines, inset) = if width >= 960.0 {
        (BILLBOARD, 64.0, 2usize, 32.0)
    } else if width >= 720.0 {
        (BILLBOARD_S, 56.0, 1, 24.0)
    } else {
        (crate::ui::PAGE_TITLE, 24.0, 0, 24.0)
    };
    let count = picks.len();
    let at = s.browse_view.shelves.spot % count;
    let p = &picks[at];
    let id = p.card.id;
    let r = crate::ui::r(8.0);
    // The sharp art is the original upload once it is decoded, else the thumbnail.
    let decoded = |url: Option<&String>| pixels_for(s, url).filter(|px| px.size().0 > 0);
    let pixels = decoded(p.image.as_ref()).or_else(|| decoded(p.small.as_ref()));
    // The art keeps its own shape, so nothing is cropped or stretched: full height, as wide as
    // its aspect asks, within limits. The text column does not depend on it.
    let aspect = pixels.map_or(16.0 / 9.0, |px| {
        let (w, h) = px.size();
        w as f32 / h.max(1) as f32
    });
    let aw = (h * aspect).clamp(h * 0.75, width * 0.60).floor();
    // The art melts into the dark side over a wide band that starts fully opaque, so where the
    // picture begins cannot be seen.
    let feather = (aw * 0.55).clamp(220.0, 420.0).floor();
    let dir = s.browse_view.shelves.spot_dir;
    let motion = crate::motion::paged(dir);
    let mut b = el()
        .key("bb-billboard")
        .w_full()
        .h(h)
        .radius(r)
        .clip_rounded(r)
        .fill(u.well)
        .hover_group()
        .on_hover_change(|st: &mut State, over: bool, tx: &mut Tx| {
            // The view does not read this: pausing never commits. The bar keeps its place
            // and the poll goes on from there once the pointer has left.
            tx.live_only();
            st.browse_view.shelves.hovered = over;
            if over {
                st.browse_view.shelves.due = None;
            } else {
                tx.wake_after(Duration::ZERO);
            }
        })
        .on_window_focus(|st: &mut State, focused: bool, tx: &mut Tx| {
            tx.live_only();
            st.browse_view.shelves.window_inactive = !focused;
            if focused {
                tx.wake_after(Duration::ZERO);
            }
        })
        // Scrolled out of view: it waits, and goes on when it is back.
        .on_visible(|st: &mut State, on: bool, tx: &mut Tx| {
            tx.live_only();
            st.browse_view.shelves.offscreen = !on;
            if on {
                tx.wake_after(Duration::ZERO);
            }
        })
        .on_key(move |st: &mut State, ev: &KeyEvent, _tx: &mut Tx| {
            let step = match ev.key {
                Key::Left => count - 1,
                Key::Right => 1,
                _ => return false,
            };
            let next = (st.browse_view.shelves.spot % count + step) % count;
            step_to(st, next, if step == 1 { 1 } else { -1 });
            true
        });
    if let Some(px) = pixels {
        b = b
            .child(
                el().key(format!("bb-back-{id}"))
                    .absolute(0.0, 0.0)
                    .w_full()
                    .h_full()
                    .pixels_fit(px, SurfaceFit::Fill)
                    .blur(32.0)
                    .transition(motion)
                    .a11y_hidden(),
            )
            .child(
                el().key(format!("bb-art-{id}"))
                    .pin(None, Some(0.0), Some(0.0), Some(0.0))
                    .w(aw)
                    .transition(motion)
                    .a11y_hidden()
                    .child(el().absolute(0.0, 0.0).w_full().h_full().pixels_fit(px, SurfaceFit::Fill))
                    // The feather into the scrim, drawn with the art so it moves with it.
                    .child(
                        el().absolute(0.0, 0.0)
                            .w(feather)
                            .h_full()
                            .linear_gradient(
                                std::f32::consts::FRAC_PI_2,
                                [
                                    (0.0, INK_ON_WHITE.alpha(1.0)),
                                    (0.18, INK_ON_WHITE.alpha(0.94)),
                                    (0.45, INK_ON_WHITE.alpha(0.66)),
                                    (0.75, INK_ON_WHITE.alpha(0.26)),
                                    (1.0, INK_ON_WHITE.alpha(0.0)),
                                ],
                            ),
                    ),
            );
    }
    // The scrim is the same in every theme: text on art is always white.
    let edge = ((width - aw + feather * 0.15) / width).clamp(0.0, 1.0);
    let fade = ((width - aw + feather) / width).clamp(edge, 1.0);
    b = b
        .child(
            el().absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .linear_gradient(
                    std::f32::consts::FRAC_PI_2,
                    [(0.0, INK_ON_WHITE.alpha(0.94)), (edge, INK_ON_WHITE.alpha(0.94)), (fade, INK_ON_WHITE.alpha(0.0)), (1.0, INK_ON_WHITE.alpha(0.0))],
                )
                // Over the art, which slides in with its own paint order.
                .paint_order(2)
                .a11y_hidden(),
        )
        .child(
            el().pin(Some(0.0), None, Some(0.0), Some(0.0))
                .h(96.0)
                .gradient(Color::BLACK.alpha(0.0), Color::BLACK.alpha(0.55))
                .paint_order(2)
                .a11y_hidden(),
        );

    // Text and actions in fixed slots, so stepping picks moves nothing.
    let white = |a: f32| Color::WHITE.alpha(a);
    let mut meta = row().h(20.0).gap(8.0).center_y().min_w(0.0);
    let category = if p.card.category.is_empty() { "-".to_string() } else { p.card.category.clone() };
    meta = meta
        .child(text(category).font(BODY).color(white(0.7)).truncate().min_w(0.0).shrink())
        .child(text("·").font(BODY).color(white(0.5)))
        .child(text(if p.card.author.is_empty() { "-".to_string() } else { p.card.author.clone() }).font(BODY).color(white(0.7)).truncate().min_w(0.0).shrink());
    if p.card.likes > 0 {
        meta = meta
            .child(text("·").font(BODY).color(white(0.5)))
            .child(icon(&icons::THUMBS_UP, 12.0).color(white(0.7)).a11y_hidden())
            .child(text(card_count(p.card.likes)).font(crate::ui::BODY_NUM).color(white(0.7)));
    }
    let state = super::quick::button(s, id);
    let saved = is_favorite(s, "Mod", id);
    let favorite = Favorite::from_card(&p.card);
    let dark_button = |key: String| {
        row().key(key)
            .h(32.0)
            .gap(6.0)
            .center()
            .radius(crate::ui::r(8.0))
            .fill(Color::BLACK.alpha(0.45))
            .border(1.0, white(0.18))
            .hover(white(0.10))
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
    };
    let opened = super::store::clone_card(&p.card);
    let actions = row()
        .key("bb-actions")
        .h(32.0)
        .gap(8.0)
        .center_y()
        .child(super::quick::button_on_art(u, &state, format!("bb-install-{id}"), id, &p.card.name, 32.0, 120.0))
        .child(
            dark_button("bb-details".into())
                .px(12.0)
                .role("button")
                .label(format!("{} {}", super::tree::tr("Details"), p.card.name))
                .on_press(move |st: &mut State| show_details(st, Details::from_card(&opened)))
                .child(icon(&icons::INFO, 14.0).color(Color::WHITE).a11y_hidden())
                .child(text(super::tree::tr("Details")).font(LABEL).color(Color::WHITE).a11y_hidden()),
        )
        .child(
            dark_button("bb-star".into())
                .w(32.0)
                .role("button")
                .label(if saved { "Remove from favorites" } else { "Save to favorites" })
                .on_press(move |st: &mut State| toggle_favorite(st, favorite.clone()))
                .child(icon(if saved { &STAR_FILLED } else { &STAR }, 16.0).color(Color::WHITE).a11y_hidden()),
        );
    let text_w = (width * 0.42).clamp(200.0, 440.0);
    let mut column = col()
        .key("bb-text")
        .paint_order(3)
        .pin(Some(inset), None, None, Some(inset))
        .w(text_w)
        // The eyebrow sits on the title, whatever its line count; the block keeps one height.
        .child(
            col().w_full().h(16.0 + 8.0 + title_h).gap(8.0).justify_end()
                .child(row().h(16.0).child(text(p.eyebrow.clone()).font(LABEL).color(white(0.7)).truncate().min_w(0.0)))
                .child(text(p.card.name.clone()).font(title_font).color(Color::WHITE).wrap().max_lines(if title_h > 30.0 { 2 } else { 1 }).min_w(0.0)),
        )
        .child(meta.mt(8.0).w_full());
    if desc_lines > 0 {
        column = column.child(
            el().w_full().h(20.0 * desc_lines as f32).mt(4.0).child(
                text(p.description.clone()).font(SMALL).color(white(0.6)).wrap().max_lines(desc_lines).min_w(0.0),
            ),
        );
    }
    column = column.child(actions.mt(16.0));
    let opened = super::store::clone_card(&p.card);
    b = b
        .focusable(true)
        .role("group")
        .label(p.card.name.clone())
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| show_details(st, Details::from_card(&opened)))
        .child(column);
    if count > 1 {
        // The pager sits in its own corner, clear of the text: one bar per pick, the current
        // one filling as its turn runs, then the two step buttons. Always shown.
        let step = s.browse_view.shelves.step.min(STEPS) as f32 / STEPS as f32;
        let bars = picks.iter().enumerate().map(|(i, pick)| {
            let on = i == at;
            let filled = if on { (SEG * step).round() } else { 0.0 };
            el().key(format!("bb-seg-{i}"))
                .size(SEG, 24.0)
                .center()
                .focusable(true)
                .role("button")
                .checked(on)
                .label(pick.card.name.clone())
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    let from = st.browse_view.shelves.spot % count;
                    step_to(st, i, if i >= from { 1 } else { -1 });
                })
                .child(
                    el().w(SEG)
                        .h(3.0)
                        .radius(1.5)
                        .clip_rounded(1.5)
                        .fill(white(if on { 0.38 } else { 0.28 }))
                        .child(el().key(format!("bb-fill-{i}")).h(3.0).w(filled).fill(Color::WHITE).motion(crate::motion::countdown())),
                )
        });
        let arrow = |forward: bool| {
            dark_button(if forward { "bb-next".into() } else { "bb-prev".into() })
                .w(32.0)
                .role("button")
                .label(if forward { "Next pick" } else { "Previous pick" })
                .on_press(move |st: &mut State| {
                    let at = st.browse_view.shelves.spot % count;
                    let next = if forward { (at + 1) % count } else { (at + count - 1) % count };
                    step_to(st, next, if forward { 1 } else { -1 });
                })
                .child(icon(if forward { &icons::CHEVRON_RIGHT } else { &icons::CHEVRON_LEFT }, 16.0).color(Color::WHITE).a11y_hidden())
        };
        b = b.child(
            row().key("bb-pager")
                .pin(None, None, Some(inset), Some(inset))
                .h(40.0)
                .px(8.0)
                .gap(6.0)
                .center_y()
                // Its own dark plate, so the bars read on any art.
                .radius(crate::ui::r(8.0))
                .fill(Color::BLACK.alpha(0.84))
                .border(1.0, white(0.14))
                .paint_order(3)
                .children(bars)
                .child(el().w(6.0))
                .child(arrow(false))
                .child(arrow(true)),
        );
    }
    b
}

/// One row: its title link, then a clipped viewport that shows a page of tiles and the peek.
fn row_view(s: &State, u: &Ui, r: &Row, width: f32, n: usize) -> El {
    let slot = r.kind.slot();
    let len = r.cards.len();
    let (tw, th) = tile_size(width, n);
    let pages = page_count(len, n);
    let page = s.browse_view.shelves.pages[slot].min(pages - 1);
    let (start, end, x) = page_window(len, n, page, width);
    let ranked = r.kind == RowKind::Top10;
    let mut strip = row()
        .key(format!("top-row-{slot}-page-{page}"))
        .absolute(x, 0.0)
        .gap(GAP)
        .paint_order(1)
        .transition(crate::motion::paged(s.browse_view.shelves.dirs[slot]));
    // The numeral only on whole tiles: a peeking tile's rank would be cut mid glyph.
    let last_page = page + 1 == pages && len > n;
    // Left and Right move focus tile to tile, and page the row when focus crosses its edge.
    let ids: std::rc::Rc<Vec<u64>> = std::rc::Rc::new(r.cards.iter().map(|c| c.id).collect());
    for (i, card) in r.cards.iter().enumerate().take(end).skip(start) {
        let whole = if last_page { i > start } else { i < start + n };
        let ids = ids.clone();
        let why = r.reasons.and_then(|m| m.get(&card.id)).map(reason_text);
        let keyed = tile(s, u, slot, card, tw, th, (ranked && whole).then_some(i + 1), why).on_key(move |st: &mut State, ev: &KeyEvent, tx: &mut Tx| {
            let to = match ev.key {
                Key::Left if i > 0 => i - 1,
                Key::Right if i + 1 < ids.len() => i + 1,
                _ => return false,
            };
            let page = st.browse_view.shelves.pages[slot].min(pages - 1);
            let shown = whole_tiles(ids.len(), n, page);
            if !shown.contains(&to) {
                let next = if to > i { (page + 1).min(pages - 1) } else { page.saturating_sub(1) };
                st.browse_view.shelves.pages[slot] = next;
                st.browse_view.shelves.dirs[slot] = if to > i { 1 } else { -1 };
            }
            tx.focus_when_shown(format!("top-{slot}-{}", ids[to]), true);
            true
        });
        strip = strip.child(keyed);
    }
    // The step buttons sit in the row's header, clear of the tiles: always shown, dimmed at
    // either end.
    let arrow = |forward: bool| {
        let live = if forward { page + 1 < pages } else { page > 0 };
        let mut b = el().key(format!("top-row-{slot}-{}", if forward { "next" } else { "prev" }))
            .size(28.0, 28.0)
            .center()
            .radius(crate::ui::r(4.0))
            .border(1.0, u.hair_strong)
            .role("button")
            .label(if forward { "Next page" } else { "Previous page" })
            .disabled(!live)
            .child(icon(if forward { &icons::CHEVRON_RIGHT } else { &icons::CHEVRON_LEFT }, 16.0).color(if live { u.ink } else { u.faint }).a11y_hidden());
        if live {
            b = b.hover(u.hover).focusable(true).cursor(rune::Cursor::Pointer).on_press(move |st: &mut State| {
                let p = &mut st.browse_view.shelves.pages[slot];
                *p = if forward { (*p + 1).min(pages - 1) } else { p.saturating_sub(1) };
                st.browse_view.shelves.dirs[slot] = if forward { 1 } else { -1 };
            });
        }
        b
    };
    // The peek fades out at the row's edge, so a cut tile reads as more to come.
    let last = page + 1 >= pages;
    let viewport = el()
        .key(format!("top-row-{slot}"))
        .w_full()
        .h(th)
        .child(
            el().absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .clip_fade_linear(if last && len > n { PEEK } else { 0.0 }, 0.0, if last { 0.0 } else { PEEK }, 0.0)
                .child(strip),
        );
    let mut title = row().key(format!("top-row-{slot}-head")).h(28.0).gap(4.0).center_y().child(text(r.title.clone()).font(SECTION).color(u.ink));
    if let Some(see) = r.see.clone() {
        title = title
            .focusable(true)
            .role("link")
            .label(r.title.clone())
            .cursor(rune::Cursor::Pointer)
            .on_press_tx(move |st: &mut State, tx: &mut Tx| apply_see(st, tx, &see))
            .child(text(super::tree::tr("See all")).font(SMALL).color(u.muted).ml(8.0))
            .child(icon(&icons::CHEVRON_RIGHT, 12.0).color(u.muted).a11y_hidden());
    }
    let mut head = row().w_full().h(28.0).gap(6.0).center_y().child(title).child(spacer());
    if pages > 1 {
        head = head.child(arrow(false)).child(arrow(true));
    }
    col().key(format!("top-row-{slot}-box")).w_full().gap(8.0).child(head).child(viewport)
}

fn apply_see(st: &mut State, tx: &mut Tx, see: &See) {
    let category = see.category.as_ref().and_then(|name| {
        st.browse_view
            .categories
            .iter()
            .filter(|(section, _)| *section == BrowseSection::Mods)
            .flat_map(|(_, list)| list.iter())
            .find(|c| c.name.eq_ignore_ascii_case(name))
            .map(|c| super::Pick { id: c.id, name: c.name.clone() })
    });
    let (order, period) = (see.order, see.period);
    set_filters(st, Some(tx), move |f| {
        f.order = Some(order);
        f.period = period;
        if category.is_some() {
            f.category = category;
        }
    });
}

/// A tile: the art is the tile. Title on a gradient at rest; on hover a scrim with the title,
/// the byline and likes, Install and the star. Pending and failed images keep the title.
/// A pick's reason in the player's language.
fn reason_text(reason: &super::foryou::Reason) -> String {
    use super::foryou::Reason;
    let fill = crate::screens::settings::i18n::fill;
    match reason {
        Reason::Hero(hero) => fill(super::tree::tr("Because you play {}"), &[hero]),
        Reason::Author(author) => fill(super::tree::tr("More from {}"), &[author]),
        Reason::Category(category) => fill(super::tree::tr("More {}"), &[category]),
        Reason::Rising => super::tree::tr("Rising fast").to_string(),
        Reason::Explore => super::tree::tr("Something different").to_string(),
    }
}

fn tile(s: &State, u: &Ui, slot: usize, card: &BrowseCard, tw: f32, th: f32, rank: Option<usize>, why: Option<String>) -> El {
    let id = card.id;
    let r = crate::ui::r(8.0);
    let veil = veiled(s, id, card.nsfw);
    let url = card.thumbnail_url.as_ref();
    let loaded = pixels_for(s, url).is_some();
    let failed = url.is_some_and(|u| s.thumb_crew.failed(u));
    // The title starts after the numeral: one digit, or the wider "10".
    let left = match rank {
        Some(10..) => 84.0,
        Some(_) => 60.0,
        None => 10.0,
    };
    // The tile opens with its own card: a mod in these rows is usually not on the catalog page
    // loaded below, so it cannot be looked up there by id.
    let opened = std::rc::Rc::new(super::store::clone_card(card));
    let open_tile = opened.clone();
    let mut b = el()
        .key(format!("top-{slot}-{id}"))
        .size(tw, th)
        .min_w(tw)
        .radius(r)
        .clip_rounded(r)
        .fill(u.well)
        .hover_group()
        .focusable(true)
        .role("button")
        .label(format!("{} by {}", card.name, card.author))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| show_details(st, Details::from_card(&open_tile)))
        .on_secondary_press(move |st: &mut State, _press, _tx: &mut Tx| {
            st.browse_view.context = Some(id);
            st.browse_view.menu = None;
        })
        .on_hover_change(move |st: &mut State, over: bool, tx: &mut Tx| {
            // Only a mature tile that reveals on hover draws differently; otherwise the hover
            // is for the details prefetch and never commits.
            if !(card_nsfw_reveals(st, id)) {
                tx.live_only();
            }
            if over {
                st.browse_view.hovered = Some(id);
                st.browse_view.hover_since = Some((id, Instant::now()));
            } else if st.browse_view.hovered == Some(id) {
                st.browse_view.hovered = None;
                st.browse_view.hover_since = None;
            }
        });
    let title = |color: Color, lines: usize| {
        let t = text(card.name.clone()).font(BODY_MED).color(color).min_w(0.0);
        if lines == 1 { t.truncate() } else { t.wrap().max_lines(lines) }
    };
    // At rest: the title and, under it, the creator and likes, on the bottom gradient.
    let rest_text = |ink: Color, quiet: Color| {
        col()
            .pin(Some(left), None, Some(10.0), Some(8.0))
            .group_opacity(1.0, 0.0)
            .child(title(ink, 1))
            .child(
                row().h(16.0).gap(4.0).center_y().min_w(0.0)
                    .child(text(why.clone().unwrap_or_else(|| if card.author.is_empty() { "-".to_string() } else { card.author.clone() })).font(SMALL).color(quiet).truncate().min_w(0.0).shrink())
                    .child(text("·").font(SMALL).color(quiet))
                    .child(icon(&icons::THUMBS_UP, 12.0).color(quiet).a11y_hidden())
                    .child(text(card_count(card.likes)).font(crate::ui::SMALL_NUM).color(quiet)),
            )
    };
    if loaded {
        b = b.child(image_fill(s, format!("top-thumb-{slot}-{id}"), url, [r; 4], veil));
        if veil {
            b = b.child(el().absolute(0.0, 0.0).w_full().h_full().fill(Color::BLACK.alpha(0.35)).a11y_hidden());
        }
        b = b
            .child(
                el().pin(Some(0.0), None, Some(0.0), Some(0.0))
                    .h(96.0)
                    .linear_gradient(std::f32::consts::PI, [(0.0, Color::BLACK.alpha(0.0)), (0.35, Color::BLACK.alpha(0.66)), (1.0, Color::BLACK.alpha(0.94))])
                    .group_opacity(1.0, 0.0)
                    .a11y_hidden(),
            )
            // A flat band under the two text lines, so busy art never shows through them.
            .child(el().pin(Some(0.0), None, Some(0.0), Some(0.0)).h(48.0).fill(Color::BLACK.alpha(0.40)).group_opacity(1.0, 0.0).a11y_hidden())
            .child(rest_text(Color::WHITE, Color::WHITE.alpha(0.72)));
    } else {
        // Pending or failed: a finished graphite tile with the title, never a hole.
        b = b
            .child(
                el().absolute(0.0, 0.0)
                    .w_full()
                    .h(th - 24.0)
                    .center()
                    .child(icon(&icons::PACKAGE, 24.0).color(if failed { u.muted } else { u.faint }).a11y_hidden()),
            )
            .child(rest_text(u.prose, u.muted));
    }
    if let Some(rank) = rank.filter(|_| loaded || !failed) {
        b = b
            .child(
                el().pin(Some(0.0), None, None, Some(0.0))
                    .size(160.0, 96.0)
                    .radial_gradient([0.0, 1.0], [160.0, 96.0], [(0.0, Color::BLACK.alpha(0.8)), (1.0, Color::BLACK.alpha(0.0))])
                    .group_opacity(1.0, 0.0)
                    .a11y_hidden(),
            )
            .child(
                text(rank.to_string())
                    .font(RANK)
                    .color(Color::WHITE)
                    .pin(Some(8.0), None, None, Some(0.0))
                    // Out of the way of Install and the star on hover.
                    .group_opacity(1.0, 0.0)
                    .a11y_hidden(),
            );
    }
    // Hover layer: scrim, title, byline and likes, then Install and the star. A veiled tile
    // offers Show instead of Install.
    let white = |a: f32| Color::WHITE.alpha(a);
    let author = if card.author.is_empty() { "-".to_string() } else { card.author.clone() };
    let mut hover = col()
        .key(format!("top-hover-{slot}-{id}"))
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .padding(10.0)
        .gap(2.0)
        .fill(Color::BLACK.alpha(0.62))
        .group_opacity(0.0, 1.0)
        // The layer covers the tile, so it opens the details itself: a press anywhere on the
        // tile that is not Install or the star goes to the mod.
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| show_details(st, Details::from_card(&opened)))
        .child(title(Color::WHITE, 2))
        .child(
            row().gap(4.0).center_y().min_w(0.0)
                .child(text(author).font(SMALL).color(white(0.7)).truncate().min_w(0.0).shrink())
                .child(text("·").font(SMALL).color(white(0.5)))
                .child(icon(&icons::THUMBS_UP, 12.0).color(white(0.7)).a11y_hidden())
                .child(text(card_count(card.likes)).font(crate::ui::SMALL_NUM).color(white(0.7))),
        )
        .child(spacer());
    if veil {
        hover = hover.child(row().w_full().center().child(nsfw_tag(u))).child(el().h(6.0)).child(
            row().w_full().center().child(
                row().key(format!("top-show-{slot}-{id}"))
                    .h(28.0)
                    .px(12.0)
                    .gap(6.0)
                    .center()
                    .radius(crate::ui::r(4.0))
                    .fill(Color::BLACK.alpha(0.45))
                    .border(1.0, white(0.18))
                    .hover(white(0.10))
                    .focusable(true)
                    .role("button")
                    .label("Show NSFW preview")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| crate::screens::settings::nsfw::revealed(st, id))
                    .child(icon(&icons::EYE, 14.0).color(Color::WHITE).a11y_hidden())
                    .child(text(super::tree::tr("Show")).font(LABEL).color(Color::WHITE).a11y_hidden()),
            ),
        );
    } else {
        let state = super::quick::button(s, id);
        let saved = is_favorite(s, &card.model, id);
        let favorite = Favorite::from_card(card);
        let mut actions = row().w_full().h(28.0).gap(6.0).center_y();
        if card.model.eq_ignore_ascii_case("mod") {
            actions = actions.child(super::quick::button_on_art(u, &state, format!("top-install-{slot}-{id}"), id, &card.name, 28.0, 0.0).grow(1.0).min_w(0.0));
        } else {
            actions = actions.child(spacer());
        }
        if why.is_some() {
            actions = actions.child(
                el().key(format!("top-less-{slot}-{id}"))
                    .size(28.0, 28.0)
                    .center()
                    .radius(crate::ui::r(4.0))
                    .fill(Color::BLACK.alpha(0.45))
                    .border(1.0, white(0.18))
                    .hover(white(0.10))
                    .focusable(true)
                    .role("button")
                    .label("Not interested")
                    .help(super::tree::tr("Not interested: show fewer like this"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| super::foryou::dismiss(st, id))
                    .child(icon(&icons::X, 14.0).color(Color::WHITE).a11y_hidden()),
            );
        }
        actions = actions.child(
            el().key(format!("top-star-{slot}-{id}"))
                .size(28.0, 28.0)
                .center()
                .radius(crate::ui::r(4.0))
                .fill(Color::BLACK.alpha(0.45))
                .border(1.0, white(0.18))
                .hover(white(0.10))
                .focusable(true)
                .role("button")
                .label(if saved { "Remove from favorites" } else { "Save to favorites" })
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| toggle_favorite(st, favorite.clone()))
                .child(icon(if saved { &STAR_FILLED } else { &STAR }, 14.0).color(Color::WHITE).a11y_hidden()),
        );
        hover = hover.child(actions);
    }
    b = b.child(hover);
    if veil {
        // A deliberate veil: the tag and an eye at rest, never an empty box.
        // The tag sits over the eye in the middle, as on the catalog's cards, clear of the
        // title; on hover the layer above shows the same tag over its Show button.
        b = b.child(
            col().absolute(0.0, 0.0)
                .w_full()
                .h(th - 40.0)
                .center()
                .gap(8.0)
                .group_opacity(1.0, 0.0)
                .child(nsfw_tag(u))
                .child(icon(&icons::EYE, 20.0).color(white(0.8)).a11y_hidden()),
        );
    } else if card.nsfw && crate::screens::settings::nsfw::always_tag() {
        b = b.child(row().pin(Some(8.0), Some(8.0), None, None).child(nsfw_tag(u)));
    }
    // The two rings: quiet at rest, stronger on hover.
    b.child(el().absolute(0.0, 0.0).w_full().h_full().radius(r).border(1.0, u.hair).group_opacity(1.0, 0.0).a11y_hidden())
        .child(el().absolute(0.0, 0.0).w_full().h_full().radius(r).border(1.0, u.hair_strong).group_opacity(0.0, 1.0).a11y_hidden())
}

fn card_nsfw_reveals(st: &State, _id: u64) -> bool {
    crate::screens::settings::nsfw::reveal_on_hover() && st.prefs.mature == crate::prefs::Mature::Blur
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::browse::shelf::Feeds;

    const NOW: i64 = 1_800_000_000;

    fn card(id: u64, days_ago: i64, likes: u64, category: &str, nsfw: bool) -> BrowseCard {
        let mut c = card_of(&TopSub {
            id,
            name: format!("Mod {id}"),
            author: "maker".into(),
            image_url: Some(format!("https://images.example/{id}.jpg")),
            nsfw,
            ..TopSub::default()
        });
        c.category = format!("{category} / Sub");
        c.likes = likes;
        c.added_ts = NOW - days_ago * DAY;
        c
    }

    fn state_with(recent: Vec<BrowseCard>, liked: Vec<BrowseCard>) -> State {
        let mut st = State::default();
        st.browse_view.shelves.feeds = Some(Feeds { recent, liked });
        st
    }

    fn ids(row: &Row) -> Vec<u64> {
        row.cards.iter().map(|c| c.id).collect()
    }

    fn assert_rows_ok(p: &Plan, n: usize) {
        let mut seen = HashSet::new();
        for r in &p.rows {
            if r.kind == RowKind::Top10 {
                assert_eq!(r.cards.len(), 10, "{}", r.title);
            } else {
                assert!(r.cards.len() > n, "{} has {} tiles", r.title, r.cards.len());
                assert!(r.cards.len() <= MAX_ROW);
            }
            for c in &r.cards {
                assert!(seen.insert(c.id), "mod {} is in two rows", c.id);
            }
        }
    }

    /// Every row in the plan is a full page plus its peek, or not there at all; the Top 10 has
    /// exactly ten; no mod is in two rows.
    #[test]
    fn plan_fills_or_drops() {
        for count in [0usize, 3, 9, 40, 200] {
            let recent: Vec<BrowseCard> =
                (0..count as u64).map(|i| card(i + 1, (i % 30) as i64, i * 7 % 50, ["Skins", "HUD", "Sounds", "Maps"][(i % 4) as usize], false)).collect();
            let liked: Vec<BrowseCard> = (0..40).map(|i| card(1000 + i, 400, 5000 - i, "Skins", false)).collect();
            let st = state_with(recent, liked);
            for n in [1, 2, 3, 4] {
                let p = plan(&st, n, NOW);
                assert_rows_ok(&p, n);
                assert!(p.rows.iter().any(|r| r.kind == RowKind::Top10), "{count} recent: no Top 10");
            }
        }
    }

    /// A thin week ranks the month and says so.
    #[test]
    fn plan_title_names_the_window() {
        // Three mods this week, twelve more this month.
        let mut recent: Vec<BrowseCard> = (1..=3).map(|i| card(i, 2, 10 * i, "Skins", false)).collect();
        recent.extend((4..=15).map(|i| card(i, 20, i, "Skins", false)));
        let st = state_with(recent, Vec::new());
        let p = plan(&st, 4, NOW);
        assert_eq!(p.rows[0].title, "Top 10 this month");
        assert_eq!(ids(&p.rows[0])[0], 3, "most liked first");
    }

    /// Hide: mature mods are nowhere, not even in the image queue. Blur: in rows, never on the
    /// billboard. Show: everywhere, the billboard included.
    #[test]
    fn plan_respects_mature() {
        use crate::prefs::Mature;
        for mode in Mature::ALL {
            let recent: Vec<BrowseCard> = (1..=40).map(|i| card(i, (i % 7) as i64, 100 - i, "Skins", i % 2 == 0)).collect();
            let mut st = state_with(recent, Vec::new());
            st.prefs.mature = mode;
            st.browse_view.top = Some(vec![
                TopSub { id: 2, model: "Mod".into(), period: "today".into(), image_url: Some("https://images.example/2.jpg".into()), nsfw: true, ..TopSub::default() },
                TopSub { id: 3, model: "Mod".into(), period: "today".into(), image_url: Some("https://images.example/3.jpg".into()), ..TopSub::default() },
            ]);
            let p = plan(&st, 4, NOW);
            let shown: Vec<bool> = p.rows.iter().flat_map(|r| r.cards.iter().map(|c| c.nsfw)).collect();
            let lead_mature = p.billboard.iter().any(|pick| pick.card.nsfw);
            match mode {
                Mature::Hide => {
                    assert!(!shown.contains(&true), "Hide: a mature mod in a row");
                    assert!(!lead_mature);
                }
                Mature::Blur => {
                    assert!(shown.contains(&true), "Blur: mature mods stay in rows");
                    assert!(!lead_mature, "Blur: never a billboard pick");
                }
                Mature::Show => {
                    assert!(shown.contains(&true));
                    assert_eq!(p.billboard[0].card.id, 2, "Show: a mature mod may lead");
                }
            }
        }
    }

    /// Every page a row can show starts with `n` whole tiles and never leaves an empty slot.
    #[test]
    fn pages_end_flush_with_no_empty_slots() {
        let w = 1016.0;
        let n = 4;
        let (tw, _) = tile_size(w, n);
        assert_eq!(page_count(10, 4), 3);
        assert_eq!(page_window(10, 4, 0, w), (0, 5, 0.0));
        assert_eq!(page_window(10, 4, 1, w), (4, 9, 0.0));
        let (start, end, x) = page_window(10, 4, 2, w);
        assert_eq!((start, end), (5, 10));
        // The last tile ends at the right edge; the first of the five peeks at the left.
        assert!((x + 5.0 * tw + 4.0 * GAP - w).abs() < 0.5);
        assert!(x < 0.0 && x > -tw);
    }

    /// The tile sizes the spec gives at the two window widths.
    #[test]
    fn tiles_follow_the_spec() {
        assert_eq!(per_page(1016.0), 4);
        assert_eq!(tile_size(1016.0, 4), (236.0, 133.0));
        assert_eq!(per_page(776.0), 3);
        assert_eq!(tile_size(776.0, 3), (237.0, 133.0));
    }

    /// The cover queue reads the plan: every tile on a shown page and the billboard's pick are
    /// asked for, under every mature mode, and Hide never asks for mature art.
    #[test]
    fn every_shown_tile_has_its_cover_asked_for() {
        for mode in crate::prefs::Mature::ALL {
            let recent: Vec<BrowseCard> = (1..=60).map(|i| card(i, (i % 30) as i64, 200 - i, ["Skins", "HUD"][(i % 2) as usize], i % 5 == 0)).collect();
            let liked: Vec<BrowseCard> = (100..=140).map(|i| card(i, 400, 900 - i, "Skins", false)).collect();
            let mut st = state_with(recent, liked);
            st.prefs.mature = mode;
            let width = super::super::toolbar::page_width(&st);
            let n = per_page(width);
            for page in 0..4 {
                st.browse_view.shelves.pages = [page; super::super::shelf::SLOTS];
                let (now, next) = cover_urls(&st);
                let p = plan(&st, n, gamebanana::now_unix());
                for r in &p.rows {
                    let (a, b, _) = page_window(r.cards.len(), n, page, width);
                    for c in &r.cards[a..b] {
                        let url = c.thumbnail_url.clone().unwrap();
                        assert!(now.contains(&url) || next.contains(&url), "{mode:?}: tile {} shown without its cover asked for", c.id);
                    }
                }
                if mode == crate::prefs::Mature::Hide {
                    let mature: Vec<String> = (1..=60u64).filter(|i| i % 5 == 0).map(|i| format!("https://images.example/{i}.jpg")).collect();
                    assert!(now.iter().chain(&next).all(|u| !mature.contains(u)), "Hide asked for mature art");
                }
            }
        }
    }

    /// The landing is short: at most two rows, so the catalog starts right under them.
    #[test]
    fn the_landing_has_at_most_two_rows() {
        let recent: Vec<BrowseCard> =
            (0..200u64).map(|i| card(i + 1, (i % 30) as i64, i * 7 % 50, ["Skins", "HUD", "Sounds", "Maps"][(i % 4) as usize], false)).collect();
        let liked: Vec<BrowseCard> = (0..60).map(|i| card(1000 + i, 400, 5000 - i, "Skins", false)).collect();
        let st = state_with(recent, liked);
        let p = plan(&st, 4, NOW);
        assert!(p.rows.len() <= 2, "{} rows", p.rows.len());
        assert_eq!(p.rows[0].kind, RowKind::Top10);
    }

    /// With picks for the player, the second row is theirs and says why each mod is there; a
    /// mod already in the Top 10 is not repeated in it.
    #[test]
    fn the_second_row_is_for_the_player() {
        let recent: Vec<BrowseCard> = (0..60u64).map(|i| card(i + 1, (i % 7) as i64, 100 - i, "Skins", false)).collect();
        let mut st = state_with(recent, Vec::new());
        let picks: Vec<BrowseCard> = (0..12u64).map(|i| card(if i == 0 { 1 } else { 500 + i }, 40, 30 + i, "Skins", false)).collect();
        let reasons = picks.iter().map(|c| (c.id, crate::screens::browse::foryou::Reason::Hero("Haze".into()))).collect();
        st.browse_view.shelves.for_you = crate::screens::browse::foryou::ForYou { picks, reasons, personal: true, key: 1, ..Default::default() };
        let p = plan(&st, 4, NOW);
        assert_eq!(p.rows.len(), 2);
        assert_eq!(p.rows[1].title, "For you");
        assert!(p.rows[1].see.is_none() && p.rows[1].reasons.is_some());
        assert!(!ids(&p.rows[1]).contains(&1), "mod 1 is in the Top 10 already");
        assert_rows_ok(&p, 4);
    }

    /// The billboard asks for the original upload behind a thumbnail.
    #[test]
    fn the_large_image_is_the_thumbnail_without_its_size() {
        assert_eq!(large_of("https://images.gamebanana.com/img/ss/mods/530-90_a.jpg"), "https://images.gamebanana.com/img/ss/mods/a.jpg");
        assert_eq!(large_of("https://images.gamebanana.com/img/ss/mods/220-90_b.png"), "https://images.gamebanana.com/img/ss/mods/b.png");
        assert_eq!(large_of("https://images.example/t1.jpg"), "https://images.example/t1.jpg");
    }

    /// The timer: never armed under Reduce motion, with the setting off, while hovered, in the
    /// background, off Browse, or with one pick.
    #[test]
    fn billboard_pauses() {
        let recent: Vec<BrowseCard> = (1..=30).map(|i| card(i, 1, 100 - i, "Skins", false)).collect();
        let mut st = state_with(recent, Vec::new());
        st.screen = Screen::Browse;
        st.prefs.browse_auto_advance = true;
        st.browse_view.top = Some(
            (1..=3)
                .map(|i| TopSub { id: 900 + i, model: "Mod".into(), period: "today".into(), image_url: Some(format!("https://images.example/t{i}.jpg")), ..TopSub::default() })
                .collect(),
        );
        st.browse_view.w4.featured = vec![("HUD".into(), card(950, 1, 1, "HUD", false))];
        assert_eq!(paused(&st, None), None, "it runs on the landing");
        fn check(st: &mut State, what: &str, change: &dyn Fn(&mut State), focus: Option<&str>) {
            change(st);
            assert!(paused(st, focus).is_some(), "{what} should pause it");
        }
        check(&mut st, "hover", &|s| s.browse_view.shelves.hovered = true, None);
        st.browse_view.shelves.hovered = false;
        assert_eq!(paused(&st, Some("bb-next")), None, "a pressed pager button must not stop it");
        check(&mut st, "background", &|s| s.browse_view.shelves.window_inactive = true, None);
        check(&mut st, "scrolled past", &|s| {
            s.browse_view.shelves.window_inactive = false;
            s.browse_view.shelves.offscreen = true;
        }, None);
        st.browse_view.shelves.offscreen = false;
        check(&mut st, "details open", &|s| {
            s.browse_view.shelves.window_inactive = false;
            s.browse_view.context = Some(1);
        }, None);
        st.browse_view.context = None;
        assert_eq!(paused(&st, None), None, "running again once every cause ended");
        check(&mut st, "two minutes without input", &|s| s.browse_view.shelves.last_input = Some(Instant::now() - IDLE - Duration::from_secs(1)), None);
        st.browse_view.shelves.last_input = Some(Instant::now());
        assert_eq!(paused(&st, None), None, "input wakes it again");
        let mut s = std::mem::take(&mut st);
        s.screen = Screen::Installed;
        assert!(paused(&s, None).is_some(), "another page");
        s.screen = Screen::Browse;
        s.prefs.reduce_motion = true;
        assert_eq!(paused(&s, None), Some("reduce motion"));
        s.prefs.reduce_motion = false;
        s.prefs.browse_auto_advance = false;
        assert_eq!(paused(&s, None), Some("setting"));
        s.prefs.browse_auto_advance = true;
        s.browse_view.top = None;
        s.browse_view.w4.featured.clear();
        assert!(paused(&s, None).is_some(), "one pick");
    }
}
