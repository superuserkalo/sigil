//! Shelves for the landing page, as Steam and Netflix lay out a store front
//! (`design/browse.md` 2.2): a 24 point title row with "See all" and two arrows, over one page of
//! fixed-size cover cards. The arrows step a whole page; the offset snaps, nothing slides.
//!
//! A shelf draws only its current page, so nothing scrolls and nothing is measured per frame.
//! Card sizes come from the page width alone. Every cover reserves its box before its image
//! arrives (a flat well until then), so nothing moves when pictures load. Hover lights a ring on
//! the cover through the card's hover group; no state changes, no rebuild.

use super::*;

/// How many shelves a page holds. Each keeps its own page index.
pub const SLOTS: usize = 6;
/// Space between cards in a shelf, the grid's gap.
pub const GAP: f32 = 12.0;
/// Items read per shelf.
pub const PER_SHELF: usize = 15;
/// The arrow buttons and "See all": 24 points.
const SMALL_CONTROL: f32 = 24.0;

pub const TRENDING: usize = 0;
pub const MONTH: usize = 1;
pub const FEATURED: usize = 2;

/// The two reads behind the shelves: most liked this week and this month. (All time is the
/// Most liked sort itself, one press away, so it has no shelf.)
#[derive(Clone, Default)]
pub struct Feeds {
    pub week: Vec<BrowseCard>,
    pub month: Vec<BrowseCard>,
    /// Most liked over three months: what refills a shelf the gate and the one-mod-once rule
    /// left short, so a shelf is never half empty.
    pub reserve: Vec<BrowseCard>,
}

#[derive(Default)]
pub struct Shelves {
    /// The page each shelf shows.
    pub pages: [usize; SLOTS],
    /// The way each shelf last paged (1 forward, -1 back), so the new page slides in from
    /// the side it comes from.
    pub dirs: [i8; SLOTS],
    /// The same for the spotlight.
    pub spot_dir: i8,
    /// The reads, once they land. Decided once per run: a late read never inserts a shelf
    /// above what is on screen, because every shelf is drawn at full size from the start.
    pub feeds: Option<Feeds>,
    pub job: Job<Feeds>,
    /// The spotlight's current pick.
    pub spot: usize,
    /// Whether this run shows the Featured shelf, decided once when Browse first polls, so a
    /// saved catalog that loads late never inserts a shelf above the grid (next run shows it).
    pub featured: Option<bool>,
    /// What the landing's covers were last queued for, so a poll asks again only when what
    /// the landing shows (or the image store, or the image queue) changed.
    pub queued_for: u64,
}

/// Cards per page for a page `width` points wide.
pub fn per_page(width: f32) -> usize {
    if width >= 960.0 {
        5
    } else if width >= 720.0 {
        4
    } else if width >= 528.0 {
        3
    } else {
        2
    }
}

/// One card's width: `n` cards and their gaps fill the page edge to edge.
pub fn card_width(width: f32, n: usize) -> f32 {
    ((width - GAP * (n as f32 - 1.0)) / n as f32).floor().max(0.0)
}

/// Hero tiles per page: 72 wide with a gap of 8.
pub fn heroes_per_page(width: f32) -> usize {
    (((width + 8.0) / 80.0).floor() as usize).max(1)
}

/// The pages a shelf of `len` items has at `n` a page.
pub fn page_count(len: usize, n: usize) -> usize {
    len.div_ceil(n.max(1)).max(1)
}

/// Start the two reads once per run, off the UI thread.
pub fn start(st: &mut State) {
    if st.browse_view.shelves.featured.is_none() {
        st.browse_view.shelves.featured = Some(!st.browse_view.w4.featured.is_empty());
    }
    let v = &st.browse_view.shelves;
    if st.demo || rune::is_headless() || v.feeds.is_some() || v.job.is_running() {
        return;
    }
    let wake = st.wake.clone();
    st.browse_view.shelves.job.start(wake, || {
        // GameBanana has no date filter: a query with a period is served newest first
        // (`BrowseQuery::server_order`), so asking for "most liked this week" got the 15
        // newest mods for both shelves. Read the last 30 days newest first instead, and the
        // all-time most liked, and rank the windows here (`feeds_from`).
        let read = |order: BrowseOrder, page: usize| -> Result<Vec<BrowseMod>, String> {
            let query = BrowseQuery { section: BrowseSection::Mods, order, ..BrowseQuery::default() };
            gamebanana::fetch_browse(&query, page, gamebanana::GAMEBANANA_MAX_PERPAGE)
                .map(|page| page.mods)
                .map_err(|e| e.to_string())
        };
        let start = gamebanana::now_unix() - 30 * DAY;
        let mut recent: Vec<BrowseMod> = Vec::new();
        for page in 1..=RECENT_PAGES {
            let Ok(rows) = read(BrowseOrder::Newest, page) else { break };
            let done = rows.is_empty() || rows.iter().any(|m| m.added_ts > 0 && m.added_ts < start);
            recent.extend(rows);
            if done {
                break;
            }
        }
        let liked = read(BrowseOrder::MostLiked, 1).unwrap_or_default();
        Ok(feeds_from(&recent, &liked, gamebanana::now_unix()))
    });
}

const DAY: i64 = 24 * 60 * 60;
/// Newest-first pages read for the last 30 days, 50 a page.
const RECENT_PAGES: usize = 4;

/// The shelves from the reads: most liked of the mods added in the last 7 and 30 days, and
/// the all-time most liked as the reserve that refills a short shelf.
pub fn feeds_from(recent: &[BrowseMod], liked: &[BrowseMod], now: i64) -> Feeds {
    let window = |days: i64| -> Vec<BrowseCard> {
        let start = now - days * DAY;
        let mut rows: Vec<&BrowseMod> = recent.iter().filter(|m| m.added_ts >= start).collect();
        rows.sort_by(|a, b| b.likes.cmp(&a.likes).then(b.added_ts.cmp(&a.added_ts)));
        rows.into_iter().map(card_from_browse).collect()
    };
    Feeds { week: window(7), month: window(30), reserve: liked.iter().map(card_from_browse).collect() }
}

/// Take the reads when they land and ask for the first two pages of covers.
pub fn poll(st: &mut State) -> bool {
    let Some(result) = st.browse_view.shelves.job.try_take() else {
        return false;
    };
    st.browse_view.shelves.feeds = Some(result.unwrap_or_default());
    // The covers are asked for by `queue_covers`, from what the landing then shows.
    true
}

/// What the landing shows, in order, decided in one place for the view and the cover queue:
/// the spotlight's picks, then the week, month and featured shelves. The gate applies to all
/// of them (hidden mods and creators, and mature mods while Settings hide them, are never
/// shown), and a mod shows once: not in a shelf when the spotlight has it, in one shelf only.
/// A week or month shelf the rule leaves short refills from the month and three-month reads,
/// and shows whole pages only, so no page ends in empty slots. `None` for a shelf whose read
/// has not landed (it draws placeholders).
pub struct Landing<'a> {
    pub week: Option<Vec<&'a BrowseCard>>,
    pub month: Option<Vec<&'a BrowseCard>>,
    pub featured: Vec<&'a BrowseCard>,
}

pub fn landing<'a>(s: &'a State, n: usize) -> Landing<'a> {
    let n = n.max(1);
    let mut seen: HashSet<u64> = super::spotlight::picks(s).iter().map(|t| t.id).collect();
    let feeds = s.browse_view.shelves.feeds.as_ref();
    let mut fill = |sources: &[&'a [BrowseCard]]| -> Vec<&'a BrowseCard> {
        let mut out: Vec<&'a BrowseCard> = Vec::new();
        for list in sources {
            for card in allowed(s, list) {
                if out.len() == PER_SHELF {
                    break;
                }
                if seen.insert(card.id) {
                    out.push(card);
                }
            }
        }
        // Whole pages only: a short last page is dropped (its mods go back to the pool), unless
        // that would leave the shelf with nothing.
        let keep = if out.len() >= n { out.len() - out.len() % n } else { out.len() };
        for card in out.drain(keep..) {
            seen.remove(&card.id);
        }
        out
    };
    let week = feeds.map(|f| fill(&[&f.week, &f.month, &f.reserve]));
    let month = feeds.map(|f| fill(&[&f.month, &f.reserve]));
    let featured: Vec<&'a BrowseCard> = s
        .browse_view
        .w4
        .featured
        .iter()
        .map(|(_, card)| card)
        .filter(|card| content_allowed(s, card.id, card.nsfw, &card.author) && seen.insert(card.id))
        .collect();
    Landing { week, month, featured }
}

/// The covers the landing shows now and next: the spotlight's pick and the one after it, and
/// for each shelf its current page and the next. Through the same lists the view draws, so a
/// tile that is shown always has its cover asked for, and hidden art never is.
pub fn landing_cover_urls(s: &State) -> (Vec<String>, Vec<String>) {
    let n = per_page(super::toolbar::page_width(s));
    let lists = landing(s, n);
    let mut now = Vec::new();
    let mut next = Vec::new();
    let picks = super::spotlight::picks(s);
    let spot = s.browse_view.shelves.spot.min(picks.len().saturating_sub(1));
    for (i, pick) in picks.iter().enumerate() {
        if let Some(url) = pick.image_url.clone() {
            if i == spot {
                now.push(url);
            } else if i == spot + 1 {
                next.push(url);
            }
        }
    }
    let shelves: [(usize, Option<&Vec<&BrowseCard>>); 3] =
        [(TRENDING, lists.week.as_ref()), (MONTH, lists.month.as_ref()), (FEATURED, Some(&lists.featured))];
    for (slot, list) in shelves {
        let Some(list) = list else { continue };
        let page = s.browse_view.shelves.pages[slot].min(page_count(list.len(), n) - 1);
        for (i, card) in list.iter().enumerate() {
            let Some(url) = card.thumbnail_url.clone() else { continue };
            if i / n == page {
                now.push(url);
            } else if i / n == page + 1 {
                next.push(url);
            }
        }
    }
    (now, next)
}

/// Ask for the landing's covers when what it shows changed, or the image store or queue did
/// (an image evicted, the queue cleared for a new grid page), and once a minute for covers
/// that failed. Cheap when nothing changed: one hash of the lists.
pub fn queue_covers(st: &mut State) -> bool {
    use std::hash::{Hash, Hasher};
    if st.browse_view.shelves.feeds.is_none() && st.browse_view.top.is_none() && st.browse_view.w4.featured.is_empty() {
        return false;
    }
    let (now, next) = landing_cover_urls(st);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (&now, &next, st.thumbnails.len(), st.thumb_crew.generation(), crate::data::minute_bucket()).hash(&mut h);
    let key = h.finish();
    if st.browse_view.shelves.queued_for == key {
        return false;
    }
    st.browse_view.shelves.queued_for = key;
    // On screen first, ahead of the grid's own; the next pages behind.
    crate::data::queue_images(st, now, true);
    crate::data::queue_images(st, next, false);
    false
}

/// The cards a shelf may show, in order: hidden mods, hidden creators and (while Settings hide
/// mature content) mature mods never reach a shelf.
pub fn allowed<'a>(s: &State, list: &'a [BrowseCard]) -> Vec<&'a BrowseCard> {
    list.iter().filter(|c| content_allowed(s, c.id, c.nsfw, &c.author)).collect()
}

/// A flat placeholder bar for text that has not arrived.
fn bar(u: &Ui, w: f32, h: f32) -> El {
    el().w(w).h(h).radius(crate::ui::r(4.0)).fill(u.hover)
}

/// One cover: a fixed `w` by `h` box, the image filling it once loaded, a flat well before. The
/// ring lights when the card around it (a hover group) is hovered.
pub fn cover(s: &State, u: &Ui, key: String, url: Option<&String>, w: f32, h: f32, veil: bool) -> El {
    let radius = crate::ui::r(8.0);
    let mut b = el()
        .key(format!("{key}-box"))
        .size(w, h)
        .radius(radius)
        .clip_rounded(radius)
        .fill(u.hover)
        .child(image_fill(s, key.clone(), url, [radius; 4], veil));
    // A mod with no image at all says so, as a grid card does, so it never reads as loading.
    if url.is_none() {
        b = b.child(
            el().absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .center()
                .child(icon(&icons::PACKAGE, 28.0).color(u.faint.alpha(0.6)).a11y_hidden()),
        );
    }
    b
        .child(
            el().key(format!("{key}-ring"))
                .absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .radius(radius)
                .border(1.0, u.hair_strong)
                .group_opacity(0.0, 1.0)
                .a11y_hidden(),
        )
        .child(
            el().absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .radius(radius)
                .border(1.0, u.hair)
                .group_opacity(1.0, 0.0)
                .a11y_hidden(),
        )
}

/// A shelf card: the cover, then the title, the byline and a meta line (category left, likes
/// right), each one line at a fixed height. A missing value shows "-".
pub fn card(s: &State, u: &Ui, scope: &str, card: &BrowseCard, w: f32) -> El {
    let id = card.id;
    let h = (w * 9.0 / 16.0).round();
    let veil = veiled(s, id, card.nsfw);
    // Details open with the card's own state, so its mature gate, author and dates hold.
    let opened = card.clone();
    let byline = if card.author.is_empty() { "-".to_string() } else { card.author.clone() };
    let category = if card.category.is_empty() { "-".to_string() } else { card.category.clone() };
    let mut frame = cover(s, u, format!("shelf-{scope}-{id}"), card.thumbnail_url.as_ref(), w, h, veil);
    // A veiled mature cover says so (the tag and Show, as on a grid card), so it never reads
    // as a broken image.
    if veil {
        frame = frame.child(nsfw_veil(u, &format!("shelf-{scope}"), id, true));
    }
    if let Some(url) = card.audio_url.clone() {
        let playing = s.browse_view.audio.playing == Some(id);
        frame = frame.child(
            el().key(format!("shelf-{scope}-{id}-play"))
                .pin(None, None, Some(8.0), Some(8.0))
                .size(24.0, 24.0)
                .center()
                .radius(crate::ui::r(4.0))
                .fill(Color::BLACK.alpha(0.55))
                .hover(Color::BLACK.alpha(0.75))
                .focusable(true)
                .role("button")
                .label(if playing { "Stop the sample" } else { "Play the sample" })
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| st.browse_view.audio.toggle(id, url.clone()))
                // On a darkened image in every theme, so the ink is always white.
                .child(icon(if playing { &icons::SQUARE_STOP } else { &icons::PLAY }, 12.0).color(Color::WHITE).a11y_hidden()),
        );
    }
    col()
        .key(format!("shelf-{scope}-{id}"))
        .w(w)
        .min_w(w)
        .hover_group()
        .focusable(true)
        .role("button")
        .label(format!("{} by {byline}", card.name))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| show_details(st, Details::from_card(&opened)))
        .child(frame)
        .child(
            row()
                .w_full()
                .h(20.0)
                .mt(8.0)
                .center_y()
                .child(text(card.name.clone()).font(BODY_MED).color(u.ink).truncate().min_w(0.0).shrink()),
        )
        .child(row().w_full().h(16.0).center_y().child(text(byline).font(SMALL).color(u.muted).truncate().min_w(0.0).shrink()))
        .child(
            row()
                .w_full()
                .h(14.0)
                .gap(8.0)
                .center_y()
                .child(text(category).font(crate::ui::MICRO).color(u.muted).truncate().min_w(0.0).shrink())
                .child(spacer())
                .child(icon(&icons::THUMBS_UP, 12.0).color(u.faint).a11y_hidden())
                .child(text(card_count(card.likes)).font(crate::ui::MICRO).color(u.muted)),
        )
}

/// A card's place before its data: the cover box and two flat bars, at the final size.
pub fn placeholder(u: &Ui, w: f32) -> El {
    let h = (w * 9.0 / 16.0).round();
    col()
        .w(w)
        .min_w(w)
        .child(el().size(w, h).radius(crate::ui::r(8.0)).fill(u.hover))
        .child(row().h(20.0).mt(8.0).center_y().child(bar(u, (w * 0.6).floor(), 10.0)))
        .child(row().h(16.0).center_y().child(bar(u, (w * 0.4).floor(), 8.0)))
        .child(el().h(14.0))
}

fn arrow(u: &Ui, slot: usize, key: &'static str, forward: bool, enabled: bool, pages: usize) -> El {
    let (glyph, label) = if forward {
        (&icons::CHEVRON_RIGHT, super::tree::tr("Next page"))
    } else {
        (&icons::CHEVRON_LEFT, super::tree::tr("Previous page"))
    };
    let b = el()
        .key(format!("{key}-{}", if forward { "next" } else { "prev" }))
        .size(SMALL_CONTROL, SMALL_CONTROL)
        .center()
        .radius(crate::ui::r(4.0))
        .border(1.0, u.hair)
        .role("button")
        .label(label)
        .child(icon(glyph, 14.0).color(u.muted).a11y_hidden());
    if !enabled {
        return b.disabled(true).opacity(0.4);
    }
    b.hover(u.hover)
        .press(u.over.alpha(0.08))
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            let page = &mut st.browse_view.shelves.pages[slot];
            *page = if forward { (*page + 1).min(pages.saturating_sub(1)) } else { page.saturating_sub(1) };
            st.browse_view.shelves.dirs[slot] = if forward { 1 } else { -1 };
        })
}

/// The quiet "See all" in a shelf's title row.
pub fn see_all(u: &Ui, key: &'static str) -> El {
    row()
        .key(key)
        .h(SMALL_CONTROL)
        .px(8.0)
        .center_y()
        .radius(crate::ui::r(4.0))
        .hover(u.hover)
        .press(u.over.alpha(0.08))
        .focusable(true)
        .role("button")
        .label(super::tree::tr("See all"))
        .cursor(rune::Cursor::Pointer)
        .child(text(super::tree::tr("See all")).font(LABEL).color(u.muted).a11y_hidden())
}

/// A shelf: the title row over one page of `tiles` laid out `n` to a page, each `tile_w` wide.
/// A short last page keeps its empty slots, so the row never changes height or spacing.
#[allow(clippy::too_many_arguments)]
pub fn shelf(
    s: &State,
    u: &Ui,
    slot: usize,
    key: &'static str,
    title: &str,
    action: Option<El>,
    tiles: Vec<El>,
    n: usize,
    tile_w: f32,
    gap: f32,
) -> El {
    let pages = page_count(tiles.len(), n);
    let page = s.browse_view.shelves.pages[slot].min(pages - 1);
    let mut controls = row().gap(4.0).center_y();
    if let Some(action) = action {
        controls = controls.child(action.mr(4.0));
    }
    controls = controls
        .child(arrow(u, slot, key, false, page > 0, pages))
        .child(arrow(u, slot, key, true, page + 1 < pages, pages));
    let head = u.section_head(title, None, Some(controls));
    // One page row keyed by its number: a new page comes in 32 points from the side it pages
    // from while fading in, the old one fades out in place. Its slots keep their size, so
    // nothing around the shelf moves; the page's scroll viewport clips the travel at its edges
    // (a clip here would cut the cards' focus rings).
    let mut strip = row()
        .key(format!("{key}-page-{page}"))
        .w_full()
        .gap(gap)
        // A leaving page stays in the tree after the arriving one, so it would paint over it.
        .paint_order(1)
        .transition(crate::motion::paged(s.browse_view.shelves.dirs[slot]));
    if tiles.is_empty() {
        // Everything this shelf had is already on the landing: one quiet line at the shelf's
        // full height, so nothing below moves.
        let h = (tile_w * 9.0 / 16.0).round() + 8.0 + 20.0 + 16.0 + 14.0;
        return col()
            .key(format!("{key}-shelf"))
            .w_full()
            .gap(8.0)
            .child(u.section(title))
            .child(
                row()
                    .key(key)
                    .w_full()
                    .h(h)
                    .center_y()
                    .child(text(super::tree::tr("Nothing new here yet")).font(SMALL).color(u.muted)),
            );
    }
    let mut shown = 0;
    for tile in tiles.into_iter().skip(page * n).take(n) {
        strip = strip.child(tile);
        shown += 1;
    }
    for _ in shown..n {
        strip = strip.child(el().w(tile_w).min_w(tile_w));
    }
    col()
        .key(format!("{key}-shelf"))
        .w_full()
        .gap(8.0)
        .child(head)
        .child(col().key(key).w_full().child(strip))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: u64, author: &str, nsfw: bool) -> BrowseCard {
        let mut card = super::super::spotlight::card_of(&TopSub {
            id,
            name: format!("Mod {id}"),
            author: author.to_string(),
            image_url: Some(format!("https://images.example/{id}.jpg")),
            nsfw,
            ..TopSub::default()
        });
        card.model = "Mod".into();
        card
    }

    /// A mature mod (Settings hide mature), a hidden mod and a hidden creator never reach a
    /// shelf, the spotlight or the image queue.
    #[test]
    fn hidden_and_mature_mods_stay_off_every_landing_surface() {
        let mut st = State::default();
        st.prefs.mature = crate::prefs::Mature::Hide;
        st.browse_view.hidden_mods.insert(2);
        st.browse_view.hidden_authors.insert("Hidden Maker".to_string());
        let list = vec![row(1, "Fine", false), row(2, "Fine", false), row(3, "hidden maker", false), row(4, "Fine", true)];
        let ids: Vec<u64> = allowed(&st, &list).iter().map(|c| c.id).collect();
        assert_eq!(ids, vec![1], "shelves");
        st.browse_view.shelves.feeds = Some(Feeds { week: list.clone(), month: list.clone(), reserve: Vec::new() });
        let (now, next) = landing_cover_urls(&st);
        assert_eq!(now, vec!["https://images.example/1.jpg".to_string()], "shelf image queue");
        assert!(next.is_empty());
        st.browse_view.top = Some(
            list.iter()
                .map(|c| TopSub {
                    id: c.id,
                    name: c.name.clone(),
                    model: "Mod".into(),
                    period: "today".into(),
                    author: c.author.clone(),
                    image_url: c.thumbnail_url.clone(),
                    nsfw: c.nsfw,
                    ..TopSub::default()
                })
                .collect(),
        );
        let picks: Vec<u64> = super::super::spotlight::picks(&st).iter().map(|t| t.id).collect();
        assert_eq!(picks, vec![1], "spotlight");
        // Mod 1 is the spotlight's now, so no shelf repeats it; its cover is the spotlight's.
        assert_eq!(landing_cover_urls(&st).0, vec!["https://images.example/1.jpg".to_string()], "spotlight image queue");
        // The grid's gate is the same function.
        assert!(!content_allowed(&st, 2, false, "Fine"));
        assert!(!content_allowed(&st, 3, false, "HIDDEN MAKER"));
        assert!(!content_allowed(&st, 4, true, "Fine"));
    }

    fn ids(list: &[&BrowseCard]) -> Vec<u64> {
        list.iter().map(|c| c.id).collect()
    }

    /// The bug the owner saw: a shelf tile that stayed a grey box until it was opened. The
    /// cover queue asked for the first two pages of each read before the one-mod-once rule,
    /// so the month shelf (whose first mods the week shelf already had) showed tiles nobody
    /// asked a cover for, and the featured shelf asked for none. Now the queue reads the lists
    /// the view draws: every tile on a shown page has its cover asked for, in every mature mode.
    #[test]
    fn every_shown_tile_has_its_cover_asked_for() {
        for mode in crate::prefs::Mature::ALL {
            let mut st = State::default();
            st.prefs.mature = mode;
            let week: Vec<BrowseCard> = (1..=15).map(|i| row(i, "a", i % 4 == 0)).collect();
            let month: Vec<BrowseCard> = (1..=30).map(|i| row(i, "a", i % 4 == 0)).collect();
            st.browse_view.shelves.feeds = Some(Feeds { week, month, reserve: Vec::new() });
            st.browse_view.w4.featured = vec![("Skins".into(), row(99, "b", false)), ("HUD".into(), row(98, "b", true))];
            let n = per_page(super::super::toolbar::page_width(&st));
            for page in 0..3 {
                st.browse_view.shelves.pages = [page; SLOTS];
                let lists = landing(&st, n);
                let (now, _) = landing_cover_urls(&st);
                let shelves = [lists.week.clone().unwrap(), lists.month.clone().unwrap(), lists.featured.clone()];
                for list in &shelves {
                    let page = page.min(page_count(list.len(), n) - 1);
                    for card in list.iter().skip(page * n).take(n) {
                        let url = card.thumbnail_url.clone().unwrap();
                        assert!(now.contains(&url), "{mode:?} page {page}: tile {} shown without its cover asked for", card.id);
                    }
                }
                // Hide keeps mature mods off every shelf and out of the queue.
                if mode == crate::prefs::Mature::Hide {
                    for list in &shelves {
                        assert!(list.iter().all(|c| !c.nsfw), "a mature mod reached a shelf");
                    }
                    assert!(!now.iter().any(|u| u.ends_with("/4.jpg") || u.ends_with("/98.jpg")));
                }
            }
        }
    }

    /// A shelf shows whole pages: the one-mod-once rule and the gate never leave a page that
    /// ends in empty slots. A short shelf refills from the month and three-month reads first.
    #[test]
    fn shelves_refill_and_show_whole_pages() {
        let mut st = State::default();
        let n = 5;
        // Week has 7, all of them also in month: week takes 5 (a whole page) and gives two back.
        let week: Vec<BrowseCard> = (1..=7).map(|i| row(i, "a", false)).collect();
        let month: Vec<BrowseCard> = (1..=9).map(|i| row(i, "a", false)).collect();
        let reserve: Vec<BrowseCard> = (20..=40).map(|i| row(i, "a", false)).collect();
        st.browse_view.shelves.feeds = Some(Feeds { week: week.clone(), month: month.clone(), reserve: Vec::new() });
        let lists = landing(&st, n);
        assert_eq!(ids(lists.week.as_ref().unwrap()), vec![1, 2, 3, 4, 5, 6, 7, 8, 9].into_iter().take(5).collect::<Vec<_>>());
        // Month had 6..9 left: 4, fewer than a page, so it shows them rather than nothing.
        assert_eq!(ids(lists.month.as_ref().unwrap()), vec![6, 7, 8, 9]);
        // With the three-month read, both shelves fill whole pages and repeat nothing.
        st.browse_view.shelves.feeds = Some(Feeds { week, month, reserve });
        let lists = landing(&st, n);
        let (w, m) = (ids(lists.week.as_ref().unwrap()), ids(lists.month.as_ref().unwrap()));
        assert_eq!(w.len(), 15);
        assert_eq!(m.len(), 15);
        assert!(w.iter().all(|id| !m.contains(id)), "a mod shows on two shelves");
    }

    /// Root cause of the empty second shelf: GameBanana serves a query with a period newest
    /// first, so "most liked this week" and "most liked this month" were the same newest mods.
    /// The shelves now rank the windows here.
    #[test]
    fn shelves_rank_their_windows_locally() {
        let bounded = BrowseQuery { order: BrowseOrder::MostLiked, period: BrowsePeriod::Week, ..BrowseQuery::default() };
        assert_eq!(bounded.server_order(), BrowseOrder::Newest, "a bounded read is newest first");
        let now = 1_800_000_000;
        let day = 24 * 60 * 60;
        let m = |id: u64, days_ago: i64, likes: u64| -> BrowseMod {
            serde_json::from_value(serde_json::json!({
                "id": id, "name": format!("Mod {id}"), "profileUrl": "", "model": "Mod", "author": "a",
                "category": "Skins", "subcategory": "", "version": null, "updatedAt": "", "likes": likes,
                "views": 0, "hasFiles": true, "thumbnailUrl": null, "addedTs": now - days_ago * day,
            }))
            .expect("row")
        };
        // Newest first, as the read returns them.
        let recent = vec![m(1, 1, 5), m(2, 3, 50), m(3, 6, 20), m(4, 10, 90), m(5, 25, 10), m(6, 40, 999)];
        let feeds = feeds_from(&recent, &[m(7, 400, 5000)], now);
        let ids = |l: &[BrowseCard]| l.iter().map(|c| c.id).collect::<Vec<_>>();
        assert_eq!(ids(&feeds.week), vec![2, 3, 1], "week: added in 7 days, most liked first");
        assert_eq!(ids(&feeds.month), vec![4, 2, 3, 5, 1], "month: added in 30 days, most liked first");
        assert_eq!(ids(&feeds.reserve), vec![7]);
    }

    #[test]
    fn cards_per_page_follow_the_spec() {
        assert_eq!(per_page(984.0), 5);
        assert_eq!(per_page(744.0), 4);
        assert_eq!(per_page(544.0), 3);
        assert_eq!(per_page(304.0), 2);
        assert_eq!(card_width(984.0, 5), 187.0);
        assert_eq!(card_width(744.0, 4), 177.0);
        assert_eq!(heroes_per_page(984.0), 12);
        assert_eq!(heroes_per_page(744.0), 9);
    }

    #[test]
    fn pages_round_up_and_never_reach_zero() {
        assert_eq!(page_count(15, 5), 3);
        assert_eq!(page_count(16, 5), 4);
        assert_eq!(page_count(0, 5), 1);
    }
}
