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
pub const HEROES: usize = 3;
pub const ALL_TIME: usize = 4;

/// The three reads behind the shelves: most liked this week, this month and of all time.
#[derive(Clone, Default)]
pub struct Feeds {
    pub week: Vec<BrowseCard>,
    pub month: Vec<BrowseCard>,
    pub all: Vec<BrowseCard>,
}

#[derive(Default)]
pub struct Shelves {
    /// The page each shelf shows.
    pub pages: [usize; SLOTS],
    /// The reads, once they land. Decided once per run: a late read never inserts a shelf
    /// above what is on screen, because every shelf is drawn at full size from the start.
    pub feeds: Option<Feeds>,
    pub job: Job<Feeds>,
    /// The spotlight's current pick.
    pub spot: usize,
    /// Whether this run shows the Featured shelf, decided once when Browse first polls, so a
    /// saved catalog that loads late never inserts a shelf above the grid (next run shows it).
    pub featured: Option<bool>,
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

/// Start the three reads once per run, off the UI thread.
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
        let read = |order: BrowseOrder, period: BrowsePeriod| -> Result<Vec<BrowseCard>, String> {
            let query = BrowseQuery {
                section: BrowseSection::Mods,
                order,
                period,
                ..BrowseQuery::default()
            };
            gamebanana::fetch_browse(&query, 1, PER_SHELF)
                .map(|page| page.mods.iter().map(card_from_browse).collect())
                .map_err(|e| e.to_string())
        };
        Ok(Feeds {
            week: read(BrowseOrder::MostLiked, BrowsePeriod::Week).unwrap_or_default(),
            month: read(BrowseOrder::MostLiked, BrowsePeriod::Month).unwrap_or_default(),
            all: read(BrowseOrder::MostLiked, BrowsePeriod::Any).unwrap_or_default(),
        })
    });
}

/// Take the reads when they land and ask for the first two pages of covers.
pub fn poll(st: &mut State) -> bool {
    let Some(result) = st.browse_view.shelves.job.try_take() else {
        return false;
    };
    let feeds = result.unwrap_or_default();
    let n = per_page(super::toolbar::page_width(st));
    let urls = cover_urls(st, &feeds, n * 2);
    crate::data::queue_images(st, urls, false);
    st.browse_view.shelves.feeds = Some(feeds);
    true
}

/// The cards a shelf may show, in order: hidden mods, hidden creators and (while Settings hide
/// mature content) mature mods never reach a shelf.
pub fn allowed<'a>(s: &State, list: &'a [BrowseCard]) -> Vec<&'a BrowseCard> {
    list.iter().filter(|c| content_allowed(s, c.id, c.nsfw, &c.author)).collect()
}

/// The covers worth fetching: the first `count` allowed cards of each shelf. Hidden art is
/// never downloaded.
pub fn cover_urls(s: &State, feeds: &Feeds, count: usize) -> Vec<String> {
    [&feeds.week, &feeds.month, &feeds.all]
        .into_iter()
        .flat_map(|list| allowed(s, list).into_iter().take(count).filter_map(|c| c.thumbnail_url.clone()))
        .collect()
}

/// A flat placeholder bar for text that has not arrived.
fn bar(u: &Ui, w: f32, h: f32) -> El {
    el().w(w).h(h).radius(crate::ui::r(4.0)).fill(u.hover)
}

/// One cover: a fixed `w` by `h` box, the image filling it once loaded, a flat well before. The
/// ring lights when the card around it (a hover group) is hovered.
pub fn cover(s: &State, u: &Ui, key: String, url: Option<&String>, w: f32, h: f32, veil: bool) -> El {
    let radius = crate::ui::r(8.0);
    el().key(format!("{key}-box"))
        .size(w, h)
        .radius(radius)
        .clip_rounded(radius)
        .fill(u.hover)
        .child(image_fill(s, key.clone(), url, [radius; 4], veil))
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
    let byline = if card.author.is_empty() { "-".to_string() } else { format!("by {}", card.author) };
    let category = if card.category.is_empty() { "-".to_string() } else { card.category.clone() };
    let mut frame = cover(s, u, format!("shelf-{scope}-{id}"), card.thumbnail_url.as_ref(), w, h, veil);
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
        .label(format!("{}, {byline}", card.name))
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
    let mut head = row()
        .w_full()
        .h(24.0)
        .gap(4.0)
        .center_y()
        .child(text(title.to_string()).font(crate::ui::H2).color(u.ink).truncate().min_w(0.0).shrink())
        .child(spacer());
    if let Some(action) = action {
        head = head.child(action.mr(4.0));
    }
    head = head
        .child(arrow(u, slot, key, false, page > 0, pages))
        .child(arrow(u, slot, key, true, page + 1 < pages, pages));
    let mut strip = row().key(key).w_full().gap(gap);
    let mut shown = 0;
    for tile in tiles.into_iter().skip(page * n).take(n) {
        strip = strip.child(tile);
        shown += 1;
    }
    for _ in shown..n {
        strip = strip.child(el().w(tile_w).min_w(tile_w));
    }
    col().key(format!("{key}-shelf")).w_full().gap(8.0).child(head).child(strip)
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
        let feeds = Feeds { week: list.clone(), month: list.clone(), all: list.clone() };
        let urls = cover_urls(&st, &feeds, 15);
        assert_eq!(urls, vec!["https://images.example/1.jpg".to_string(); 3], "shelf image queue");
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
        assert_eq!(super::super::spotlight::cover_urls(&st), vec!["https://images.example/1.jpg".to_string()], "spotlight image queue");
        // The grid's gate is the same function.
        assert!(!content_allowed(&st, 2, false, "Fine"));
        assert!(!content_allowed(&st, 3, false, "HIDDEN MAKER"));
        assert!(!content_allowed(&st, 4, true, "Fine"));
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
