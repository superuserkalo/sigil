//! Saved Browse state, dates, the local catalog copy and hero helpers.

use super::*;

// --- saved state ---------------------------------------------------------------

pub(crate) fn snapshot(s: &State) -> persist::Persisted {
    let f = &s.browse_view.filters;
    persist::Persisted {
        section: f.section.model().to_string(),
        tab: s.browse_view.tab.id().to_string(),
        category: f.category.as_ref().map(|pick| (pick.id, pick.name.clone())),
        sub: f.sub.as_ref().map(|pick| (pick.id, pick.name.clone())),
        hero: f.hero.clone(),
        order: f.order.map(|order| order.id().to_string()),
        period: f.period.id().to_string(),
        date_field: f.date_field.id().to_string(),
        date_from: f.date_from,
        date_to: f.date_to,
        featured: f.featured,
        creator: f.creator.as_ref().map(|pick| (pick.id, pick.name.clone())),
        owned: f.owned.id().to_string(),
        mature: f.mature.id().to_string(),
        hide_outdated: f.hide_outdated,
        exclude: f.exclude,
        layout: s.browse_view.layout.id().to_string(),
        card_size: s.browse_view.card_size.id().to_string(),
        paged: s.browse_view.paged,
        audio_volume: s.browse_view.audio.volume,
        audio_unmuted: s.bar.unmuted,
        hidden_mods: s
            .browse_view
            .hidden_mods
            .iter()
            .map(|id| persist::HiddenMod {
                id: *id,
                model: "Mod".to_string(),
                name: s
                    .browse_view
                    .hidden_names
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| format!("Mod {id}")),
            })
            .collect(),
        hidden_authors: s.browse_view.hidden_authors.iter().cloned().collect(),
        hidden_meta: s
            .browse_view
            .hidden_meta
            .iter()
            .map(|(key, meta)| (key.clone(), meta.clone()))
            .collect(),
        ignored_updates: s.browse_view.ignored_updates.clone(),
    }
}

/// Store the browse state. Called from the press that changed it, never from a frame.
pub(crate) fn save_browse_state(st: &mut State) {
    if st.demo || rune::is_headless() {
        return;
    }
    let _ = persist::write(&snapshot(st));
}

pub(crate) fn start_persisted_load(st: &mut State) {
    if st.demo || rune::is_headless() || st.browse_view.loaded_persisted {
        return;
    }
    if st.browse_view.persisted_job.is_running() {
        return;
    }
    let wake = st.wake.clone();
    st.browse_view
        .persisted_job
        .start(wake, || Ok(persist::read()));
}

/// Apply the saved state once. True when the filters changed under the loaded catalog.
pub(crate) fn apply_persisted(st: &mut State, saved: persist::Persisted) -> bool {
    st.browse_view.loaded_persisted = true;
    let before = st.browse_view.filters.clone();
    let tab = if saved.tab.is_empty() {
        st.browse_view.tab
    } else {
        Tab::from_id(&saved.tab)
    };
    st.browse_view.tab = tab;
    st.browse_view.layout = Layout::from_id(&saved.layout);
    st.browse_view.card_size = CardSize::from_id(&saved.card_size);
    st.browse_view.paged = saved.paged;
    // A saved 0 is a mute and comes back as one (wave 4); the level before it stays for Unmute.
    if saved.audio_volume >= 0.0 {
        st.browse_view.audio.set_volume(saved.audio_volume.min(1.0));
    }
    st.bar.unmuted = saved.audio_unmuted;
    st.browse_view.hidden_mods = saved.hidden_mods.iter().map(|hidden| hidden.id).collect();
    st.browse_view.hidden_names = saved
        .hidden_mods
        .iter()
        .map(|hidden| (hidden.id, hidden.name.clone()))
        .collect();
    st.browse_view.hidden_authors = saved.hidden_authors.into_iter().collect();
    st.browse_view.hidden_meta = saved.hidden_meta.into_iter().collect();
    st.browse_view.ignored_updates = saved.ignored_updates;
    st.browse_view.filters = Filters {
        section: tab.section(),
        category: saved.category.map(|(id, name)| Pick { id, name }),
        sub: saved.sub.map(|(id, name)| Pick { id, name }),
        hero: saved.hero,
        order: saved.order.as_deref().and_then(BrowseOrder::from_id),
        period: BrowsePeriod::from_id(&saved.period),
        date_field: DateField::from_id(&saved.date_field),
        date_from: saved.date_from,
        date_to: saved.date_to,
        featured: saved.featured,
        creator: saved.creator.map(|(id, name)| Pick { id, name }),
        owned: Owned::from_id(&saved.owned),
        mature: MatureView::from_id(&saved.mature),
        hide_outdated: saved.hide_outdated,
        exclude: saved.exclude,
        favorites: false,
        for_you: false,
    };
    st.browse_view.date_from_text = TextEdit::new(if saved.date_from > 0 {
        date_input(saved.date_from)
    } else {
        String::new()
    });
    st.browse_view.date_to_text = TextEdit::new(if saved.date_to > 0 {
        date_input(saved.date_to)
    } else {
        String::new()
    });
    if tab == Tab::Maps {
        let name = st
            .browse_view
            .filters
            .category
            .as_ref()
            .map(|pick| pick.name.clone())
            .unwrap_or_else(|| "Maps".to_string());
        let id = st
            .browse_view
            .filters
            .category
            .as_ref()
            .map(|pick| pick.id)
            .unwrap_or(MAPS_CATEGORY);
        st.browse_view.filters.category = Some(Pick { id, name });
    }
    start_categories(st, st.browse_view.filters.section);
    if st.browse_view.filters != before && !st.demo {
        crate::data::start_browse_fetch(st, 1);
    }
    st.browse_view.filters != before
}

// --- dates -----------------------------------------------------------------------

/// `2026-01-22` as unix seconds at midnight UTC. A wrong shape is None.
pub fn parse_date(text: &str) -> Option<i64> {
    let text = text.trim();
    let mut parts = text.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let days = days_from_civil(year, month, day);
    Some(days * 86_400)
}

/// Unix seconds as `2026-01-22`, in UTC.
pub fn date_input(ts: i64) -> String {
    let (year, month, day) = civil_from_days(ts.div_euclid(86_400));
    format!("{year:04}-{month:02}-{day:02}")
}

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
pub(crate) fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month = i64::from(month);
    let doy = (153 * ((month + 9) % 12) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date for days since 1970-01-01.
pub(crate) fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = (mp + 2) % 12 + 1;
    (year + i64::from(month <= 2), month as u32, day as u32)
}

// --- the local catalog copy --------------------------------------------------------

pub(crate) fn start_mirror_load(st: &mut State) {
    if st.demo || rune::is_headless() || st.browse_view.mirror.is_some() {
        return;
    }
    if st.browse_view.mirror_job.is_running() {
        return;
    }
    let wake = st.wake.clone();
    st.browse_view.mirror_job.start(wake, || {
        let mirror = Mirror::load(&crate::data::data_dir());
        Ok(mirror)
    });
}

/// Read a few more pages in the background, then replace the in-memory copy.
pub(crate) fn start_mirror_sync(st: &mut State) {
    if st.prefs.offline
        || st.demo
        || rune::is_headless()
        || st.browse_view.mirror_sync_job.is_running()
        || st.browse_view.mirror_save_job.is_running()
    {
        return;
    }
    let Some(mut mirror) = st.browse_view.mirror.clone() else {
        return;
    };
    st.browse_view.last_sync = Some(Instant::now());
    let wake = st.wake.clone();
    st.browse_view.mirror_sync_job.start(wake, move || {
        mirror.sync_step().map_err(|error| error.to_string())?;
        Ok(mirror)
    });
}

/// Save download counts a mod page added.
pub(crate) fn start_mirror_save(st: &mut State) {
    if st.demo || rune::is_headless() || !st.browse_view.mirror_dirty {
        return;
    }
    if st.browse_view.mirror_save_job.is_running()
        || st.browse_view.mirror_sync_job.is_running()
    {
        return;
    }
    let Some(mut mirror) = st.browse_view.mirror.clone() else {
        return;
    };
    st.browse_view.mirror_dirty = false;
    let wake = st.wake.clone();
    st.browse_view.mirror_save_job.start(wake, move || {
        mirror.save().map_err(|error| error.to_string())?;
        Ok(())
    });
}

pub(crate) fn mirror_filter(s: &State) -> MirrorFilter {
    let f = &s.browse_view.filters;
    let now = gamebanana::now_unix();
    let mut filter = MirrorFilter {
        section: f.section,
        all_models: s.browse_view.tab == Tab::All,
        query: s.browse_view.applied_search.trim().to_string(),
        category: f.category.as_ref().map(|pick| pick.name.clone()),
        subcategory: f.sub.as_ref().map(|pick| pick.name.clone()),
        subtree: f
            .sub
            .as_ref()
            .map(|pick| super::tree::subtree(s, f.section, pick.id))
            .unwrap_or_default(),
        // The hero's branches anywhere in the section's tree, or a title that names them.
        hero: f.hero.as_ref().map(|hero| catalog_mirror::HeroMatch {
            name: hero.clone(),
            ids: super::tree::hero_ids(s, f.section, hero),
        }),
        exclude_category: f.exclude,
        author_id: f.creator.as_ref().map(|pick| pick.id),
        hide_mature: s.prefs.mature == crate::prefs::Mature::Hide || f.mature == MatureView::Hide,
        only_mature: f.mature == MatureView::Only,
        hide_outdated: f.hide_outdated,
        ..MirrorFilter::default()
    };
    let start = if f.date_from > 0 {
        f.date_from
    } else {
        f.period.cutoff(now).unwrap_or(0)
    };
    match f.date_field {
        DateField::Added => {
            filter.added_from = start;
            filter.added_to = f.date_to;
        }
        DateField::Updated => {
            filter.updated_from = start;
            filter.updated_to = f.date_to;
        }
    }
    filter
}

/// One catalog row as the card the grid draws.
pub fn card_from_browse(row: &BrowseMod) -> BrowseCard {
    BrowseCard {
        id: row.id,
        name: row.name.clone(),
        author: row.author.clone(),
        category: row.category_line(),
        updated_at: row.updated_at.clone(),
        likes: row.likes,
        views: row.views,
        profile_url: row.profile_url.clone(),
        thumbnail_url: row.thumbnail_url.clone(),
        version: row.version.clone(),
        added_ts: row.added_ts,
        updated_ts: row.updated_ts,
        nsfw: row.nsfw,
        model: row.model.clone(),
        audio_url: row.audio_url.clone(),
        featured: row.featured,
        comments: row.comments,
        author_id: row.author_id,
        obsolete: row.obsolete,
        sub_category_id: row.sub_category_id,
    }
}

/// A card copy, for a view that queues or lists cards it already has.
pub fn clone_card(card: &BrowseCard) -> BrowseCard {
    BrowseCard {
        id: card.id,
        name: card.name.clone(),
        author: card.author.clone(),
        category: card.category.clone(),
        updated_at: card.updated_at.clone(),
        likes: card.likes,
        views: card.views,
        profile_url: card.profile_url.clone(),
        thumbnail_url: card.thumbnail_url.clone(),
        version: card.version.clone(),
        added_ts: card.added_ts,
        updated_ts: card.updated_ts,
        nsfw: card.nsfw,
        model: card.model.clone(),
        audio_url: card.audio_url.clone(),
        featured: card.featured,
        comments: card.comments,
        author_id: card.author_id,
        obsolete: card.obsolete,
        sub_category_id: card.sub_category_id,
    }
}

pub fn mirror_card(entry: &catalog_mirror::MirrorEntry) -> BrowseCard {    BrowseCard {
        id: entry.id,
        name: entry.name.clone(),
        author: entry.author.clone(),
        category: entry.category_line(),
        updated_at: String::new(),
        likes: entry.likes,
        views: entry.views,
        profile_url: entry.profile_url.clone(),
        thumbnail_url: entry.thumbnail_url.clone(),
        version: None,
        added_ts: entry.added_ts,
        updated_ts: entry.updated_ts,
        nsfw: entry.nsfw,
        model: entry.model.clone(),
        audio_url: entry.audio_url.clone(),
        featured: entry.featured,
        comments: entry.comments,
        author_id: entry.author_id,
        obsolete: entry.obsolete,
        sub_category_id: entry.sub_category_id,
    }
}

/// A page from the local copy, when it has one. Page 1 replaces the grid; later pages append.
pub fn mirror_page(st: &mut State, page: usize) -> Option<BrowsePageView> {
    let mirror = st.browse_view.mirror.as_ref()?;
    let filter = mirror_filter(st);
    if !mirror.covers(&filter) {
        return None;
    }
    let query = current_query(st);
    let read = mirror.page(&filter, query.shown_order(), page, PAGE_SIZE);
    let mut cards: Vec<BrowseCard> = read.entries.iter().map(mirror_card).collect();
    let owned = st.browse_view.filters.owned;
    if owned != Owned::Any {
        let installed = installed_ids(st);
        cards.retain(|card| {
            let known = card.model == "Mod" && installed.contains(&card.id);
            (owned == Owned::Installed) == known
        });
    }
    // Thumbnails load as cards scroll into view (`track_visible`).
    let key = query_key(st, &query);
    Some(BrowsePageView {
        key,
        page,
        per_page: PAGE_SIZE,
        total: read.total,
        complete: read.complete,
        page_count: read.total.div_ceil(PAGE_SIZE).max(1),
        mods: cards,
    })
}

/// The hero names the Skins tree lists, with their categories.
pub fn hero_list(s: &State) -> Vec<Pick> {
    let Some(list) = categories_of(s, BrowseSection::Mods) else {
        return Vec::new();
    };
    let Some(skins) = list
        .iter()
        .find(|root| root.name.eq_ignore_ascii_case("Skins"))
    else {
        return Vec::new();
    };
    let mut heroes: Vec<Pick> = skins
        .children
        .iter()
        .map(|child| Pick {
            id: child.id,
            name: child.name.clone(),
        })
        .collect();
    heroes.sort_by_key(|hero| hero.name.to_lowercase());
    heroes
}

/// GameBanana's Skins root, from the loaded category tree.
pub fn skins_pick(s: &State) -> Option<Pick> {
    categories_of(s, BrowseSection::Mods)?
        .iter()
        .find(|root| root.name.eq_ignore_ascii_case("Skins"))
        .map(|root| Pick {
            id: root.id,
            name: root.name.clone(),
        })
}

/// The hero a mod page mentions, if any: its category, sub category or name.
pub fn detected_hero(s: &State, name: &str, category: &str, sub: &str) -> Option<String> {
    let text = format!("{name} {category} {sub}");
    let folded = catalog_mirror::fold(&text);
    hero_list(s)
        .into_iter()
        .find(|hero| {
            let hero_key = catalog_mirror::fold(&hero.name);
            !hero_key.is_empty() && folded.contains(&hero_key)
        })
        .map(|hero| hero.name)
}

/// Set the grid to Sounds with `hero` as the name match, as the details hero row asks.
pub(crate) fn browse_hero_sounds(st: &mut State, tx: &mut Tx, hero: String) {
    close_details(st);
    st.browse_view.tab = Tab::Sounds;
    set_filters(st, Some(tx), move |f| {
        *f = Filters {
            section: BrowseSection::Sounds,
            hero: Some(hero),
            ..Filters::default()
        };
    });
    st.go(Screen::Browse);
}

/// Read the next page when the viewport is within a few rows of the end. Also called after a
/// page lands, since a page of mostly non-mod records may not add a row to scroll into.
pub fn load_more_if_near_end(st: &mut State) {
    if st.demo
        || st.browse.loading
        || st.browse.error.is_some()
        || st.browse_job.is_running()
        || !more_to_load(st)
        || st.browse_view.auto_streak >= AUTO_EMPTY_PAGES
    {
        return;
    }
    let cols = st.browse_view.cols.max(1);
    let shown = shown_indices(st).len();
    let rows = shown.div_ceil(cols);
    if st.browse_view.visible.end + LOOKAHEAD_ROWS > rows {
        // Client-side filters can empty page after page; stop reading after a few of those
        // and let the end row ask instead.
        if shown == st.browse_view.auto_shown {
            st.browse_view.auto_streak += 1;
        } else {
            st.browse_view.auto_streak = 0;
        }
        st.browse_view.auto_shown = shown;
        load_next_page(st);
    }
}

pub(crate) fn load_next_page(st: &mut State) {
    let Some(next) = st.browse.page.as_ref().map(|page| page.page + 1) else {
        return;
    };
    if st.browse_job.is_running() {
        return;
    }
    st.browse.error = None;
    st.browse_view.refreshing = false;
    crate::data::start_browse_fetch(st, next);
}

/// The grid's visible rows moved: fetch the thumbnails around them, give back the far ones,
/// and read the next page near the end. True when the view must run again.
pub(crate) fn track_visible(st: &mut State, range: Range<usize>, shown: &[usize], cols: usize) -> bool {
    st.browse_view.visible = range.clone();
    st.browse_view.cols = cols;

    let cols = cols.max(1);
    let near = |margin: usize| {
        let first = range.start.saturating_sub(margin) * cols;
        let last = ((range.end + margin) * cols).min(shown.len());
        first.min(last)..last
    };
    let urls_in = |st: &State, cards: Range<usize>| -> Vec<String> {
        let Some(page) = st.browse.page.as_ref() else {
            return Vec::new();
        };
        shown[cards]
            .iter()
            .filter_map(|&index| page.mods.get(index))
            .filter_map(|card| card.thumbnail_url.clone())
            .collect()
    };

    let wanted = urls_in(st, near(1));
    crate::data::queue_images(st, wanted, true);

    if st.thumbnails.len() > THUMB_CAP {
        let keep: HashSet<String> = urls_in(st, near(THUMB_MARGIN_ROWS)).into_iter().collect();
        let mut keep = keep;
        // The landing's covers are the same mods as the top of the grid: never evict them
        // while the landing shows them.
        let (now, next) = super::top::cover_urls(st);
        keep.extend(now);
        keep.extend(next);
        if let Some(details) = &st.browse_view.details {
            keep.extend(details.thumbnail_url.clone());
        }
        let far: Vec<String> = st
            .browse
            .page
            .as_ref()
            .map(|page| {
                page.mods
                    .iter()
                    .filter_map(|card| card.thumbnail_url.clone())
                    .filter(|url| !keep.contains(url))
                    .collect()
            })
            .unwrap_or_default();
        for url in far {
            st.thumbnails.remove(&url);
        }
    }

    let was_loading = st.browse.loading;
    load_more_if_near_end(st);
    st.browse.loading != was_loading
}

pub(crate) fn refresh(st: &mut State, tx: &mut Tx) {
    if st.demo {
        st.status = "Sample data: there is nothing to refresh.".to_string();
        return;
    }
    tx.scroll_to_instant("browse-scroll", 0.0, 0.0);
    st.browse_view.refreshing = true;
    st.browse_view.visible = 0..0;
    st.browse_view.auto_streak = 0;
    st.browse_view.top = None;
    start_top(st);
    let section = st.browse_view.filters.section;
    st.browse_view.categories_failed.retain(|s| *s != section);
    start_categories(st, section);
    crate::data::start_browse_fetch(st, 1);
}

/// The press on the end row after paging paused: read on.
pub(crate) fn load_more_pressed(st: &mut State) {
    st.browse_view.auto_streak = 0;
    st.browse_view.auto_shown = shown_indices(st).len();
    load_next_page(st);
}

