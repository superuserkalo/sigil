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
use crate::ui::{BILLBOARD, SECTION};

/// Ink on a white control drawn over art: the billboard's near black, `#0c0c0e`.
pub const INK_ON_WHITE: Color = Color { r: 0.0036, g: 0.0036, b: 0.0044, a: 1.0 };
/// The medium billboard title, 22/28.
const BILLBOARD_S: TextStyle = TextStyle::new(22.0, 600.0).lh(28.0).tracking(-0.2);

/// Space between tiles, and the slice of the next tile left showing at a row's right edge.
pub const GAP: f32 = 8.0;
pub const PEEK: f32 = 40.0;
/// One pager bar on the billboard.
const SEG: f32 = 32.0;
/// The words under a row tile's picture: a title line and a line of who and likes.
pub const TILE_TEXT: f32 = 52.0;
/// Tiles a row holds at most.
pub const MAX_ROW: usize = 20;
/// Billboard picks at most.
pub const MAX_PICKS: usize = 6;
/// Seconds between automatic billboard steps.
pub const ADVANCE: Duration = Duration::from_secs(7);
/// No input for this long and the billboard stops until the next one.
pub const IDLE: Duration = Duration::from_secs(120);
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
    ForYou,
    Rising,
    /// "New this week": stands in for Rising while nothing can be measured.
    New,
}

impl RowKind {
    pub fn slot(self) -> usize {
        match self {
            RowKind::Top10 => 0,
            RowKind::ForYou => 1,
            RowKind::Rising | RowKind::New => 2,
        }
    }
}

/// What a row's title link applies to the grid.
#[derive(Clone, Debug, PartialEq)]
pub struct See {
    pub order: BrowseOrder,
    pub period: BrowsePeriod,
    pub category: Option<String>,
    /// Open the page of every mod picked for the player instead of a sorted catalog.
    pub for_you: bool,
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

    let mut rows: Vec<Row> = Vec::new();
    let mut taken: HashSet<u64> = HashSet::new();
    let see = |order, period, category: Option<&str>| Some(See { order, period, category: category.map(str::to_string), for_you: false });

    // Top 10: the true ranking by likes. The title names the window.
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

    // The billboard: what GameBanana's staff featured lately. It is an editor's choice, so it
    // is not ranked by likes, holds nothing the Top 10 shows, and is spread out: one mod per
    // hero or sub-category and two per category at most, newest first.
    let mut billboard: Vec<Slide> = Vec::new();
    let mut lead: HashSet<u64> = HashSet::new();
    let mut per_leaf: HashSet<String> = HashSet::new();
    let mut per_root: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let mut featured: Vec<&BrowseCard> = feeds.map_or(none, |f| &f.featured[..]).iter().filter(ok).collect();
    featured.sort_by(|a, b| b.added_ts.cmp(&a.added_ts));
    for card in featured {
        if billboard.len() >= MAX_PICKS {
            break;
        }
        let fits = within(card, 30) && card.likes >= 3 && (mature_lead || !card.nsfw) && !failed(&card.thumbnail_url) && !taken.contains(&card.id);
        if !fits || lead.contains(&card.id) {
            continue;
        }
        let root = root_category(card).to_lowercase();
        let leaf = card.category.to_lowercase();
        if per_leaf.contains(&leaf) || per_root.get(&root).copied().unwrap_or(0) >= 2 {
            continue;
        }
        per_leaf.insert(leaf);
        *per_root.entry(root).or_default() += 1;
        lead.insert(card.id);
        billboard.push(Slide {
            card: super::store::clone_card(card),
            eyebrow: super::tree::tr("Featured on GameBanana").to_string(),
            description: String::new(),
            image: card.thumbnail_url.as_deref().map(large_of),
            small: card.thumbnail_url.clone(),
        });
    }
    // Too few featured mods to turn over: today's first top mod and the week's pick in each
    // category fill in, as before.
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
    if billboard.len() < 2 {
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
    }
    taken.extend(lead.iter().copied());

    // For you: the player's own row, once their heroes, mods and favorites say enough. It
    // sits between the Top 10 and Rising and never replaces either.
    let fy = &v.shelves.for_you;
    if fy.personal {
        let mine = fresh_cards(fy.picks.iter().filter(ok).collect(), &taken);
        // See all opens the whole ranking as a page of its own.
        let see = Some(See { order: BrowseOrder::MostLiked, period: BrowsePeriod::Any, category: None, for_you: true });
        push_row(&mut rows, &mut taken, min_fill, Row { kind: RowKind::ForYou, title: super::tree::tr("For you").to_string(), cards: mine, see, reasons: Some(&fy.reasons) });
    }
    // Rising: what is gaining fastest, for everyone. Always there when it can be measured.
    let rising = fresh_cards(fy.rising.iter().filter(ok).collect(), &taken);
    push_row(&mut rows, &mut taken, min_fill, Row { kind: RowKind::Rising, title: super::tree::tr("Rising").to_string(), cards: rising, see: None, reasons: None });
    // New this week stands in while Rising cannot fill: newest first, liked at least once;
    // then without the floor; then 14 and 30 days.
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
        if rows.iter().any(|r| matches!(r.kind, RowKind::Rising | RowKind::New)) {
            break;
        }
        let cards = fresh_cards(newest(days, floor), &taken);
        if cards.len() >= min_fill {
            push_row(&mut rows, &mut taken, min_fill, Row { kind: RowKind::New, title: super::tree::tr(title).to_string(), cards, see: see(BrowseOrder::Newest, period, None), reasons: None });
            break;
        }
    }

    // Three rows at most: the catalog starts right under them.
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

/// Run the billboard's interval: start it (the bar begins to fill), and when it is over move
/// to the next pick. While paused nothing is armed and the bar is empty; the count starts
/// over afterwards. Two commits per interval, none in between. True when the view must run.
pub fn tick(st: &mut State, tx: &mut Tx) -> bool {
    // The first look counts as input: a fresh window is not idle.
    if st.browse_view.shelves.last_input.is_none() {
        st.browse_view.shelves.last_input = Some(Instant::now());
    }
    let focused = tx.focused().map(str::to_string);
    if paused(st, focused.as_deref()).is_some() {
        st.browse_view.shelves.due = None;
        // The bar empties at once: this commit carries no motion.
        return std::mem::take(&mut st.browse_view.shelves.running);
    }
    let now = Instant::now();
    let Some(due) = st.browse_view.shelves.due else {
        // Start: the bar is mounted empty, and this commit fills it over the whole interval on
        // one even curve. Nothing else changes in it.
        st.browse_view.shelves.due = Some(now + ADVANCE);
        st.browse_view.shelves.running = true;
        tx.motion_curve(crate::motion::countdown(ADVANCE.as_secs_f32()));
        tx.wake_after(ADVANCE);
        return true;
    };
    if now < due {
        tx.wake_after(due - now);
        return false;
    }
    // Over: the bar empties at once and the next pick slides in. The next poll starts its bar.
    st.browse_view.shelves.due = None;
    st.browse_view.shelves.running = false;
    let width = super::toolbar::page_width(st);
    let p = plan(st, per_page(width), gamebanana::now_unix());
    let count = p.billboard.len();
    let at = st.browse_view.shelves.spot % count.max(1);
    // Only to a pick whose image is decoded: the next ready one, or stay and count again.
    let ready = (1..count).map(|k| (at + k) % count).find(|&i| {
        let pick = &p.billboard[i];
        [&pick.image, &pick.small].into_iter().flatten().any(|url| st.thumbnails.contains_key(url))
    });
    drop(p);
    if let Some(next) = ready {
        st.browse_view.shelves.spot = next;
        st.browse_view.shelves.spot_dir = 1;
        tx.animated();
    }
    tx.wake_after(Duration::ZERO);
    true
}

/// A manual step: at once, and the interval starts over with an empty bar.
fn step_to(st: &mut State, index: usize, dir: i8) {
    st.browse_view.shelves.spot = index;
    st.browse_view.shelves.spot_dir = dir;
    st.browse_view.shelves.running = false;
    st.browse_view.shelves.due = None;
}

// --- the view ------------------------------------------------------------------------------

/// A wide banner, 21:9 of the page, within sensible limits.
pub fn billboard_height(width: f32) -> f32 {
    (width * 9.0 / 21.0).round().clamp(220.0, 420.0)
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
            row().w_full().h(th + TILE_TEXT).gap(GAP).clip_rounded(0.0).children((0..=n).map(|_| {
                el().size(tw, th + TILE_TEXT)
                    .min_w(tw)
                    .radius(r)
                    .fill(u.well)
                    .border(1.0, u.hair)
                    .child(bar(u, (tw * 0.6).floor(), 10.0).pin(Some(10.0), None, None, Some(12.0)))
            })),
        )
}

/// The billboard: the pick's picture fills the whole banner (cropped from the top, as a wide
/// banner wants), fading into the page at the bottom and the left, with the title and the
/// actions on that fade. The picture is the mod's full-size upload, so it stays sharp.
fn billboard(s: &State, u: &Ui, picks: &[Slide], width: f32) -> El {
    let h = billboard_height(width);
    let (title_font, title_lines, desc_lines, pad) = if width >= 960.0 {
        (BILLBOARD, 2usize, 2usize, 32.0)
    } else if width >= 720.0 {
        (BILLBOARD_S, 2, 1, 24.0)
    } else {
        (crate::ui::PAGE_TITLE, 1, 0, 20.0)
    };
    let count = picks.len();
    let at = s.browse_view.shelves.spot % count;
    let p = &picks[at];
    let id = p.card.id;
    let r = crate::ui::r(8.0);
    // The full-size upload once it is decoded, else the thumbnail.
    let decoded = |url: Option<&String>| pixels_for(s, url).filter(|px| px.size().0 > 0);
    let pixels = decoded(p.image.as_ref()).or_else(|| decoded(p.small.as_ref()));
    let dir = s.browse_view.shelves.spot_dir;
    let base = u.bg;
    let opened = super::store::clone_card(&p.card);
    let mut b = el()
        .key("bb-billboard")
        .w_full()
        .h(h)
        .radius(r)
        .clip_rounded(r)
        .fill(u.well)
        .border(1.0, u.hair)
        .cursor(rune::Cursor::Pointer)
        .focusable(true)
        .role("group")
        .label(p.card.name.clone())
        .on_press(move |st: &mut State| show_details(st, Details::from_card(&opened)))
        .on_hover_change(|st: &mut State, over: bool, tx: &mut Tx| {
            // The view does not read this: the poll empties the bar when the pointer arrives
            // and starts it over when it left.
            tx.live_only();
            st.browse_view.shelves.hovered = over;
            if over {
                st.browse_view.shelves.due = None;
            }
            tx.wake_after(Duration::ZERO);
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
        // Wider than it is tall by more than the banner: keep the top of the picture, where
        // a mod's subject usually is.
        let (pw, ph) = px.size();
        let scale = (width / pw.max(1) as f32).max(h / ph.max(1) as f32);
        let (dw, dh) = ((pw as f32 * scale).ceil(), (ph as f32 * scale).ceil());
        b = b.child(
            el().key(format!("bb-art-{id}"))
                .absolute(((width - dw) / 2.0).floor(), 0.0)
                .size(dw, dh)
                .pixels_fit(px, SurfaceFit::Fill)
                .transition(crate::motion::paged(dir))
                .a11y_hidden(),
        );
    }
    // Two fades into the page colour: up from the bottom, where the text sits, and in from
    // the left. They follow the theme, so the text is the theme's own ink.
    b = b
        .child(
            el().absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .linear_gradient(
                    std::f32::consts::PI,
                    [(0.0, base.alpha(0.0)), (0.30, base.alpha(0.0)), (0.62, base.alpha(0.72)), (0.86, base.alpha(0.96)), (1.0, base.alpha(1.0))],
                )
                .paint_order(2)
                .a11y_hidden(),
        )
        .child(
            el().absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .linear_gradient(std::f32::consts::FRAC_PI_2, [(0.0, base.alpha(0.80)), (0.35, base.alpha(0.28)), (0.65, base.alpha(0.0)), (1.0, base.alpha(0.0))])
                .paint_order(2)
                .a11y_hidden(),
        );

    let author = if p.card.author.is_empty() { "-".to_string() } else { p.card.author.clone() };
    let category = if p.card.category.is_empty() { "-".to_string() } else { p.card.category.clone() };
    let state = super::quick::button(s, id);
    let saved = is_favorite(s, "Mod", id);
    let favorite = Favorite::from_card(&p.card);
    let glass = |key: String| {
        row().key(key)
            .h(32.0)
            .gap(6.0)
            .center()
            .radius(crate::ui::r(4.0))
            .fill(base.alpha(0.72))
            .border(1.0, u.hair_strong)
            .hover(u.hover)
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
    };
    let opened = super::store::clone_card(&p.card);
    let mut facts = row().h(24.0).gap(10.0).center_y().min_w(0.0).child(
        row().h(22.0).px(8.0).center().radius(crate::ui::r(4.0)).fill(base.alpha(0.72)).border(1.0, u.hair_strong).child(
            text(category).font(SMALL).color(u.prose).truncate().min_w(0.0),
        ),
    );
    if p.card.likes > 0 {
        facts = facts
            .child(icon(&icons::THUMBS_UP, 12.0).color(u.muted).a11y_hidden())
            .child(text(card_count(p.card.likes)).font(crate::ui::BODY_NUM).color(u.prose));
    }
    let actions = row()
        .key("bb-actions")
        .h(32.0)
        .gap(8.0)
        .center_y()
        .child(super::quick::button_on_art(u, &state, format!("bb-install-{id}"), id, &p.card.name, 32.0, 132.0))
        .child(
            glass("bb-details".into())
                .px(12.0)
                .role("button")
                .label(format!("{} {}", super::tree::tr("Details"), p.card.name))
                .on_press(move |st: &mut State| show_details(st, Details::from_card(&opened)))
                .child(icon(&icons::INFO, 14.0).color(u.ink).a11y_hidden())
                .child(text(super::tree::tr("Details")).font(LABEL).color(u.ink).a11y_hidden()),
        )
        .child(
            glass("bb-star".into())
                .w(32.0)
                .role("button")
                .label(if saved { "Remove from favorites" } else { "Save to favorites" })
                .on_press(move |st: &mut State| toggle_favorite(st, favorite.clone()))
                .child(icon(if saved { &STAR_FILLED } else { &STAR }, 16.0).color(if saved { u.amber } else { u.ink }).a11y_hidden()),
        );
    let text_w = (width * 0.6).clamp(220.0, 640.0);
    let mut column = col()
        .key("bb-text")
        .paint_order(3)
        .pin(Some(pad), None, None, Some(pad))
        .w(text_w)
        .child(row().h(16.0).child(text(p.eyebrow.to_uppercase()).font(super::MICRO_CAPS).color(u.accent).truncate().min_w(0.0)))
        .child(text(p.card.name.clone()).font(title_font).color(u.ink).wrap().max_lines(title_lines).min_w(0.0).mt(8.0))
        .child(row().h(20.0).mt(6.0).child(text(author).font(BODY).color(u.prose).truncate().min_w(0.0)));
    if desc_lines > 0 && !p.description.trim().is_empty() {
        column = column.child(text(p.description.clone()).font(BODY).color(u.muted).wrap().max_lines(desc_lines).min_w(0.0).mt(6.0));
    }
    column = column.child(facts.mt(12.0)).child(actions.mt(16.0));
    b = b.child(column);
    if count > 1 {
        // The pager sits on the fade in the bottom right corner, clear of the text and of the
        // picture's subject: one bar per pick, the current one filling as its turn runs.
        let running = s.browse_view.shelves.running;
        let bars = picks.iter().enumerate().map(|(i, pick)| {
            let on = i == at;
            // Full while its turn runs (reached on the start commit's curve), else empty at once.
            let filled = if on && running { SEG } else { 0.0 };
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
                        .fill(u.hair_strong)
                        .child(el().key(format!("bb-fill-{i}")).h(3.0).w(filled).fill(u.ink)),
                )
        });
        let arrow = |forward: bool| {
            glass(if forward { "bb-next".into() } else { "bb-prev".into() })
                .w(32.0)
                .role("button")
                .label(if forward { "Next pick" } else { "Previous pick" })
                .on_press(move |st: &mut State| {
                    let at = st.browse_view.shelves.spot % count;
                    let next = if forward { (at + 1) % count } else { (at + count - 1) % count };
                    step_to(st, next, if forward { 1 } else { -1 });
                })
                .child(icon(if forward { &icons::CHEVRON_RIGHT } else { &icons::CHEVRON_LEFT }, 16.0).color(u.ink).a11y_hidden())
        };
        b = b.child(
            row().key("bb-pager")
                .pin(None, None, Some(pad), Some(pad))
                .h(32.0)
                .gap(6.0)
                .center_y()
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
        .h(th + TILE_TEXT)
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
    if see.for_you {
        set_filters(st, Some(tx), |f| f.for_you = true);
        return;
    }
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
        Reason::AlsoUsed => super::tree::tr("Players with your mods also use").to_string(),
    }
}

pub(crate) fn tile(s: &State, u: &Ui, slot: usize, card: &BrowseCard, tw: f32, th: f32, rank: Option<usize>, why: Option<String>) -> El {
    let id = card.id;
    let r = crate::ui::r(8.0);
    let veil = veiled(s, id, card.nsfw);
    let url = card.thumbnail_url.as_ref();
    let loaded = pixels_for(s, url).is_some();
    let failed = url.is_some_and(|u| s.thumb_crew.failed(u));
    // The tile opens with its own card: a mod in these rows is usually not on the catalog page
    // loaded below, so it cannot be looked up there by id.
    let opened = std::rc::Rc::new(super::store::clone_card(card));
    let open_tile = opened.clone();
    let white = |a: f32| Color::WHITE.alpha(a);

    // The picture: nothing written on it at rest but a rank or an NSFW tag.
    let mut art = el().key(format!("top-art-{slot}-{id}")).size(tw, th).fill(u.well).clip_rounded(0.0);
    if loaded {
        art = art.child(image_fill(s, format!("top-thumb-{slot}-{id}"), url, [0.0; 4], veil));
        if veil {
            art = art.child(el().absolute(0.0, 0.0).w_full().h_full().fill(Color::BLACK.alpha(0.35)).a11y_hidden());
        }
    } else {
        // Pending or failed: a finished graphite box with a glyph, never a hole.
        art = art.child(el().absolute(0.0, 0.0).w_full().h_full().center().child(icon(&icons::PACKAGE, 24.0).color(if failed { u.muted } else { u.faint }).a11y_hidden()));
    }
    if let Some(rank) = rank {
        art = art.child(
            row().pin(Some(8.0), Some(8.0), None, None)
                .h(24.0)
                .min_w(24.0)
                .px(7.0)
                .center()
                .radius(crate::ui::r(4.0))
                .fill(Color::BLACK.alpha(0.78))
                .border(1.0, white(0.16))
                .child(text(rank.to_string()).font(crate::ui::BODY_NUM).color(Color::WHITE).a11y_hidden()),
        );
    }
    // On hover: Install and the star along the picture's lower edge. The text below stays.
    let mut hover = row()
        .key(format!("top-hover-{slot}-{id}"))
        .pin(Some(0.0), None, Some(0.0), Some(0.0))
        .h(44.0)
        .px(8.0)
        .gap(6.0)
        .center_y()
        .linear_gradient(std::f32::consts::PI, [(0.0, Color::BLACK.alpha(0.0)), (1.0, Color::BLACK.alpha(0.78))])
        .group_opacity(0.0, 1.0);
    if veil {
        // A deliberate veil: the tag over the eye, as on the catalog's cards.
        art = art.child(
            col().absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .center()
                .gap(8.0)
                .group_opacity(1.0, 0.0)
                .child(nsfw_tag(u))
                .child(icon(&icons::EYE, 20.0).color(white(0.8)).a11y_hidden()),
        );
        hover = hover.child(spacer()).child(
            row().key(format!("top-show-{slot}-{id}"))
                .h(28.0)
                .px(12.0)
                .gap(6.0)
                .center()
                .radius(crate::ui::r(4.0))
                .fill(Color::BLACK.alpha(0.55))
                .border(1.0, white(0.18))
                .hover(white(0.10))
                .focusable(true)
                .role("button")
                .label("Show NSFW preview")
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| crate::screens::settings::nsfw::revealed(st, id))
                .child(icon(&icons::EYE, 14.0).color(Color::WHITE).a11y_hidden())
                .child(text(super::tree::tr("Show")).font(LABEL).color(Color::WHITE).a11y_hidden()),
        ).child(spacer());
    } else {
        if card.nsfw && crate::screens::settings::nsfw::always_tag() {
            art = art.child(row().pin(None, Some(8.0), Some(8.0), None).child(nsfw_tag(u)));
        }
        let state = super::quick::button(s, id);
        let saved = is_favorite(s, &card.model, id);
        let favorite = Favorite::from_card(card);
        let small = |key: String, label: &'static str| {
            el().key(key)
                .size(28.0, 28.0)
                .center()
                .radius(crate::ui::r(4.0))
                .fill(Color::BLACK.alpha(0.55))
                .border(1.0, white(0.18))
                .hover(white(0.10))
                .focusable(true)
                .role("button")
                .label(label)
                .cursor(rune::Cursor::Pointer)
        };
        if card.model.eq_ignore_ascii_case("mod") {
            hover = hover.child(super::quick::button_on_art(u, &state, format!("top-install-{slot}-{id}"), id, &card.name, 28.0, 0.0).grow(1.0).min_w(0.0));
        } else {
            hover = hover.child(spacer());
        }
        if why.is_some() {
            hover = hover.child(
                small(format!("top-less-{slot}-{id}"), "Not interested")
                    .help(super::tree::tr("Not interested: show fewer like this"))
                    .on_press(move |st: &mut State| super::foryou::dismiss(st, id))
                    .child(icon(&icons::X, 14.0).color(Color::WHITE).a11y_hidden()),
            );
        }
        hover = hover.child(
            small(format!("top-star-{slot}-{id}"), if saved { "Remove from favorites" } else { "Save to favorites" })
                .on_press(move |st: &mut State| toggle_favorite(st, favorite.clone()))
                .child(icon(if saved { &STAR_FILLED } else { &STAR }, 14.0).color(Color::WHITE).a11y_hidden()),
        );
    }
    // Help build Sigil: a health chip on the picture, top right (the rank badge holds the top
    // left), under the NSFW tag when that shows. At rest only: the hover layer has the actions.
    if !veil && let Some(chip) = crate::insights::chip(s, id, card.updated_ts) {
        let top = if card.nsfw && crate::screens::settings::nsfw::always_tag() { 36.0 } else { 8.0 };
        art = art.child(
            row()
                .pin(None, Some(top), Some(8.0), None)
                .group_opacity(1.0, 0.0)
                .child(crate::insights::chip_el(s, u, format!("top-health-{slot}-{id}"), &chip, true, Some((tw - 56.0).max(40.0)))),
        );
    }
    art = art.child(hover);

    // Under the picture, on the card: the title, then who made it (or why it is here) and likes.
    let sub = why.unwrap_or_else(|| if card.author.is_empty() { "-".to_string() } else { card.author.clone() });
    let words = col()
        .w(tw)
        .h(TILE_TEXT)
        .px(10.0)
        .justify_center()
        .gap(2.0)
        .child(text(card.name.clone()).font(BODY_MED).color(u.ink).truncate().min_w(0.0))
        .child(
            row().h(16.0).gap(6.0).center_y().min_w(0.0)
                .child(text(sub).font(SMALL).color(u.muted).truncate().min_w(0.0).grow(1.0))
                .child(icon(&icons::THUMBS_UP, 12.0).color(u.muted).a11y_hidden())
                .child(text(card_count(card.likes)).font(crate::ui::SMALL_NUM).color(u.muted)),
        );
    col()
        .key(format!("top-{slot}-{id}"))
        .size(tw, th + TILE_TEXT)
        .min_w(tw)
        .radius(r)
        .clip_rounded(r)
        .fill(u.raised)
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
            // Only an NSFW tile that reveals on hover draws differently; otherwise the hover
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
        })
        .child(art)
        .child(words)
        // The two rings: quiet at rest, stronger on hover.
        .child(el().absolute(0.0, 0.0).w_full().h_full().radius(r).border(1.0, u.hair).group_opacity(1.0, 0.0).a11y_hidden())
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
        st.browse_view.shelves.feeds = Some(Feeds { recent, liked, featured: Vec::new() });
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

    /// The landing is short: at most three rows, so the catalog starts right under them.
    #[test]
    fn the_landing_has_at_most_three_rows() {
        let recent: Vec<BrowseCard> =
            (0..200u64).map(|i| card(i + 1, (i % 30) as i64, i * 7 % 50, ["Skins", "HUD", "Sounds", "Maps"][(i % 4) as usize], false)).collect();
        let liked: Vec<BrowseCard> = (0..60).map(|i| card(1000 + i, 400, 5000 - i, "Skins", false)).collect();
        let st = state_with(recent, liked);
        let p = plan(&st, 4, NOW);
        assert!(p.rows.len() <= 3, "{} rows", p.rows.len());
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
        let rising: Vec<BrowseCard> = (0..12u64).map(|i| card(if i == 0 { 501 } else { 700 + i }, 2, 30 + i, "HUD", false)).collect();
        st.browse_view.shelves.for_you = crate::screens::browse::foryou::ForYou { picks, rising, reasons, personal: true, key: 1, ..Default::default() };
        let p = plan(&st, 4, NOW);
        // For you sits between the Top 10 and Rising, and Rising stays.
        assert_eq!(p.rows.len(), 3);
        assert_eq!(p.rows[1].title, "For you");
        assert_eq!((p.rows[2].kind, p.rows[2].title.as_str()), (RowKind::Rising, "Rising"));
        assert!(!ids(&p.rows[2]).contains(&501), "mod 501 is in For you already");
        // Without enough history there is no row of the player's own; Rising is still there.
        st.browse_view.shelves.for_you.personal = false;
        let q = plan(&st, 4, NOW);
        assert_eq!(q.rows.iter().map(|r| r.kind).collect::<Vec<_>>(), vec![RowKind::Top10, RowKind::Rising]);
        st.browse_view.shelves.for_you.personal = true;
        let p = plan(&st, 4, NOW);
        assert!(p.rows[1].see.as_ref().is_some_and(|see| see.for_you) && p.rows[1].reasons.is_some());
        assert!(!ids(&p.rows[1]).contains(&1), "mod 1 is in the Top 10 already");
        assert_rows_ok(&p, 4);
    }

    /// The billboard is the editor's choice: featured mods, newest first, none of them in the
    /// Top 10, one per hero and two per category at most, never a mature one unless shown.
    #[test]
    fn the_billboard_is_what_gamebanana_featured() {
        let recent: Vec<BrowseCard> = (0..60u64).map(|i| card(i + 1, (i % 7) as i64, 100 - i, "Skins", false)).collect();
        let mut st = state_with(recent, Vec::new());
        let mut featured = vec![
            card(1, 1, 100, "Skins / Haze", false),    // in the Top 10
            card(900, 1, 40, "Skins / Haze", false),
            card(901, 2, 400, "Skins / Haze", false),  // a second Haze skin
            card(902, 3, 20, "Skins / Wraith", false),
            card(903, 4, 20, "Skins / Abrams", false), // a third skin
            card(904, 5, 9, "HUD", true),              // mature
            card(905, 6, 5, "Sounds", false),
            card(906, 7, 1, "Maps", false),            // hardly liked
            card(907, 90, 500, "Maps", false),         // old
        ];
        for c in &mut featured {
            c.featured = true;
        }
        st.browse_view.shelves.feeds.as_mut().unwrap().featured = featured;
        let p = plan(&st, 4, NOW);
        assert_eq!(p.billboard.iter().map(|b| b.card.id).collect::<Vec<_>>(), vec![900, 902, 905]);
        assert!(p.billboard.iter().all(|b| b.eyebrow == "Featured on GameBanana"));
        let top: Vec<u64> = ids(&p.rows[0]);
        assert!(p.billboard.iter().all(|b| !top.contains(&b.card.id)));
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
