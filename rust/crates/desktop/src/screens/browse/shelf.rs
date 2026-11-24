//! The reads and state behind the Browse landing top (`design/browse-top.md`): the feeds job,
//! each row's page, the billboard's pick and its timer, and the cover queue. What is drawn is
//! decided by `top::plan`; this module never decides it a second time.

use super::*;

/// Rows a landing holds at most. Each keeps its own page index.
pub const SLOTS: usize = 8;

/// The two list reads behind the rows: mods added in the last 30 days (newest first, with
/// their likes and stamps) and the all-time most liked. Ranking happens in `top::plan`.
#[derive(Clone, Default)]
pub struct Feeds {
    pub recent: Vec<BrowseCard>,
    pub liked: Vec<BrowseCard>,
}

#[derive(Default)]
pub struct Shelves {
    /// The page each row shows.
    pub pages: [usize; SLOTS],
    /// The way each row last paged (1 forward, -1 back), so the new page slides in from the
    /// side it comes from.
    pub dirs: [i8; SLOTS],
    /// The billboard's current pick and the way it last moved.
    pub spot: usize,
    pub spot_dir: i8,
    /// The reads, once they land.
    pub feeds: Option<Feeds>,
    pub job: Job<Feeds>,
    /// Whether this run shows the Featured row, decided once when Browse first polls.
    pub featured: Option<bool>,
    /// What the landing's covers were last queued for, so a poll asks again only when what
    /// the landing shows (or the image store, or the image queue) changed.
    pub queued_for: u64,
    /// The billboard advances by itself (`top::tick`): when the next pick is due, how far the
    /// progress bar on the active segment is (0 to `top::STEPS`), whether the pointer is over
    /// the billboard and whether the window is in the background.
    pub due: Option<Instant>,
    pub step: u8,
    pub hovered: bool,
    pub window_inactive: bool,
    /// The billboard is scrolled out of view: it waits until it is back.
    pub offscreen: bool,
    /// The last pointer or key input on Browse. Two minutes without one stops the billboard:
    /// a window left alone draws nothing, focused or not.
    pub last_input: Option<Instant>,
}

/// Hero tiles per page: 72 wide with a gap of 8.
pub fn heroes_per_page(width: f32) -> usize {
    (((width + 8.0) / 80.0).floor() as usize).max(1)
}

const DAY: i64 = 24 * 60 * 60;
/// Newest-first pages read for the last 30 days, 50 a page.
const RECENT_PAGES: usize = 4;

/// Start the reads once per run, off the UI thread. GameBanana has no date filter: a query
/// with a period is served newest first (`BrowseQuery::server_order`), so asking for "most
/// liked this week" got the newest mods. The last 30 days are read newest first instead and
/// ranked in `top::plan`, with the all-time most liked as the last fallback.
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
            recent.extend(rows.into_iter().filter(|m| m.added_ts <= 0 || m.added_ts >= start));
            if done {
                break;
            }
        }
        let liked = read(BrowseOrder::MostLiked, 1).unwrap_or_default();
        Ok(Feeds {
            recent: recent.iter().map(card_from_browse).collect(),
            liked: liked.iter().map(card_from_browse).collect(),
        })
    });
}

/// Take the reads when they land.
pub fn poll(st: &mut State) -> bool {
    let Some(result) = st.browse_view.shelves.job.try_take() else {
        return false;
    };
    st.browse_view.shelves.feeds = Some(result.unwrap_or_default());
    true
}

/// The cards a row may show: hidden mods, hidden creators and (while Settings hide mature
/// content) mature mods never reach the landing.
pub fn allowed<'a>(s: &State, list: &'a [BrowseCard]) -> Vec<&'a BrowseCard> {
    list.iter().filter(|c| content_allowed(s, c.id, c.nsfw, &c.author)).collect()
}

/// Ask for the landing's covers when what it shows changed, or the image store or queue did
/// (an image evicted, the queue cleared for a new grid page), and once a minute for covers
/// that failed. Cheap when nothing changed: one hash of the url lists.
pub fn queue_covers(st: &mut State) -> bool {
    use std::hash::{Hash, Hasher};
    if st.browse_view.shelves.feeds.is_none() && st.browse_view.top.is_none() && st.browse_view.w4.featured.is_empty() {
        return false;
    }
    let (now, next) = super::top::cover_urls(st);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (&now, &next, st.thumbnails.len(), st.thumb_crew.generation(), crate::data::minute_bucket()).hash(&mut h);
    let key = h.finish();
    if st.browse_view.shelves.queued_for == key {
        return false;
    }
    st.browse_view.shelves.queued_for = key;
    // On screen first, ahead of the grid's own; the next pages and picks behind.
    crate::data::queue_images(st, now, true);
    crate::data::queue_images(st, next, false);
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Root cause of the empty second shelf: GameBanana serves a query with a period newest
    /// first, so "most liked this week" and "most liked this month" were the same newest mods.
    #[test]
    fn a_bounded_read_is_newest_first() {
        let bounded = BrowseQuery { order: BrowseOrder::MostLiked, period: BrowsePeriod::Week, ..BrowseQuery::default() };
        assert_eq!(bounded.server_order(), BrowseOrder::Newest);
        let open = BrowseQuery { order: BrowseOrder::MostLiked, ..BrowseQuery::default() };
        assert_eq!(open.server_order(), BrowseOrder::MostLiked);
    }

    #[test]
    fn hero_tiles_per_page() {
        assert_eq!(heroes_per_page(984.0), 12);
        assert_eq!(heroes_per_page(744.0), 9);
    }
}
