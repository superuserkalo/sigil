//! The open mod: its data, the reader that fills it, and the poll that applies it.

use super::*;

pub struct Details {
    pub id: u64,
    /// Mods, sounds and WiPs live under different API models.
    pub section: BrowseSection,
    pub name: String,
    pub author: String,
    pub profile_url: String,
    pub thumbnail_url: Option<String>,
    pub nsfw: bool,
    pub added_ts: i64,
    pub updated_ts: i64,
    pub profile: Option<ModProfile>,
    pub comments: Option<Result<ModComments, String>>,
    pub updates: Option<Result<ModUpdates, String>>,
    /// Required and recommended mods, from `_aRequirements`.
    pub requirements: Option<Result<Vec<Requirement>, String>>,
    pub error: Option<String>,
    pub loading: bool,
    /// The carousel image in the docked view.
    pub image: usize,
    pub lightbox: Option<usize>,
    pub changelog_open: bool,
    pub open_updates: HashSet<u64>,
    pub archived_open: bool,
    /// The hero the page mentions, and the player's pick.
    pub hero: Option<String>,
    pub hero_override: Option<String>,
    /// Pages read so far, for Load more.
    pub comments_page: usize,
    pub updates_page: usize,
}

impl Details {
    pub(crate) fn from_card(card: &BrowseCard) -> Self {
        Details {
            id: card.id,
            section: BrowseSection::from_model(&card.model),
            name: card.name.clone(),
            author: card.author.clone(),
            profile_url: card.profile_url.clone(),
            thumbnail_url: card.thumbnail_url.clone(),
            nsfw: card.nsfw,
            added_ts: card.added_ts,
            updated_ts: card.updated_ts,
            profile: None,
            comments: None,
            updates: None,
            requirements: None,
            error: None,
            loading: true,
            image: 0,
            lightbox: None,
            changelog_open: false,
            open_updates: HashSet::new(),
            archived_open: false,
            hero: None,
            hero_override: None,
            comments_page: 1,
            updates_page: 1,
        }
    }

    /// A submission opened by id (a pasted link, a top pick) before its page is read.
    pub(crate) fn from_id(section: BrowseSection, id: u64, name: &str, image: Option<String>) -> Self {
        let card = BrowseCard {
            id,
            name: name.to_string(),
            author: String::new(),
            category: String::new(),
            updated_at: String::new(),
            likes: 0,
            views: 0,
            profile_url: format!(
                "https://gamebanana.com/{}s/{id}",
                section.model().to_ascii_lowercase()
            ),
            thumbnail_url: image,
            version: None,
            added_ts: 0,
            updated_ts: 0,
            nsfw: false,
            model: section.model().to_string(),
            audio_url: None,
            featured: false,
            comments: 0,
            author_id: 0,
            obsolete: false,
            sub_category_id: 0,
        };
        Details::from_card(&card)
    }

    /// The card this view saves to favorites.
    pub(crate) fn favorite(&self) -> Favorite {
        Favorite {
            id: self.id,
            model: self.section.model().to_string(),
            name: self.name.clone(),
            author: self.author.clone(),
            author_id: self
                .profile
                .as_ref()
                .and_then(|p| p.submitter.as_ref())
                .map(|m| m.id)
                .unwrap_or(0),
            category: self
                .profile
                .as_ref()
                .map(|p| {
                    [p.super_category.as_str(), p.category.as_str()]
                        .into_iter()
                        .filter(|part| !part.is_empty())
                        .collect::<Vec<_>>()
                        .join(" / ")
                })
                .unwrap_or_default(),
            profile_url: self.profile_url.clone(),
            thumbnail_url: self
                .thumbnail_url
                .clone()
                .or_else(|| self.images().first().map(|image| image.url.clone())),
            audio_url: self.profile.as_ref().and_then(|p| p.audio_url.clone()),
            likes: self.profile.as_ref().map(|p| p.likes).unwrap_or(0),
            views: self.profile.as_ref().map(|p| p.views).unwrap_or(0),
            added_ts: self.added_ts,
            updated_ts: self.updated_ts,
            nsfw: self.nsfw,
            saved_at: gamebanana::now_unix(),
        }
    }

    pub(crate) fn images(&self) -> &[ModImage] {
        self.profile
            .as_ref()
            .map(|profile| profile.images.as_slice())
            .unwrap_or_default()
    }
}

pub struct DetailsLoad {
    pub id: u64,
    pub profile: Result<ModProfile, String>,
    pub comments: Result<ModComments, String>,
    pub updates: Result<ModUpdates, String>,
    pub requirements: Result<Vec<Requirement>, String>,
    pub update_cache: Option<deadlock_core::mod_updates::UpdateCache>,
}

pub(crate) fn fetch_details(
    section: BrowseSection,
    id: u64,
    known: Option<ModProfile>,
    data_dir: std::path::PathBuf,
) -> DetailsLoad {
    let (profile, (comments, (updates, requirements))) = std::thread::scope(|scope| {
        let comments = scope.spawn(move || {
            gamebanana::fetch_comments(section, id, COMMENTS_PER_PAGE).map_err(|e| e.to_string())
        });
        let updates = scope.spawn(move || {
            gamebanana::fetch_updates(section, id, UPDATES_PER_PAGE).map_err(|e| e.to_string())
        });
        let requirements = scope.spawn(move || {
            if section == BrowseSection::Mods {
                deadlock_core::requirements::fetch_requirements(id).map_err(|e| e.to_string())
            } else {
                Ok(Vec::new())
            }
        });
        let profile = match known {
            Some(profile) => Ok(profile),
            None => gamebanana::fetch_profile(section, id).map_err(|e| e.to_string()),
        };
        let stopped = || "The reader stopped before reporting.".to_string();
        let comments = comments.join().unwrap_or_else(|_| Err(stopped()));
        let updates = updates.join().unwrap_or_else(|_| Err(stopped()));
        let requirements = requirements.join().unwrap_or_else(|_| Err(stopped()));
        (profile, (comments, (updates, requirements)))
    });
    let update_cache = deadlock_core::mod_updates::read_cache(
        &deadlock_core::mod_updates::cache_path(&data_dir),
    );
    DetailsLoad {
        id,
        profile,
        comments,
        updates,
        requirements,
        update_cache: (update_cache.checked_at > 0).then_some(update_cache),
    }
}

pub(crate) fn open_details(st: &mut State, id: u64) {
    let Some(card) = card_by_id(st, id) else {
        return;
    };
    let mut details = Details::from_card(card);
    // A hover prefetch already read the profile; the details open filled in.
    if let Some(profile) = st
        .browse_view
        .prefetched
        .iter()
        .find(|(known, _)| *known == id)
        .map(|(_, profile)| profile.clone())
    {
        details.name = profile.name.clone();
        details.author = profile
            .submitter
            .as_ref()
            .map(|member| member.name.clone())
            .unwrap_or_else(|| details.author.clone());
        details.profile = Some(profile);
    }
    show_details(st, details);
}

/// Opens a GameBanana submission by id, as a pasted link does: Browse, with its details over
/// the grid. "Install with Sigil" links come through here (`crate::deeplink`).
pub fn open_submission(st: &mut State, section: BrowseSection, id: u64) {
    st.go(Screen::Browse);
    show_details(st, Details::from_id(section, id, "", None));
}

pub(crate) fn show_details(st: &mut State, details: Details) {
    st.browse_view.menu = None;
    st.browse_view.context = None;
    st.browse_view.hero_picker = None;
    close_details(st);
    if let Some(url) = details.thumbnail_url.clone() {
        crate::data::queue_images(st, vec![url], true);
    }
    crate::screens::settings::product::action(st, Some(details.id), deadlock_core::product_data::ModAction::Opened);
    crate::screens::settings::product::rec_outcome(st, details.id, deadlock_core::product_data::RecOutcome::Opened);
    st.browse_view.details = Some(details);
    start_details_fetch(st);
}

pub(crate) fn start_details_fetch(st: &mut State) {
    if st.demo {
        if let Some(details) = st.browse_view.details.as_mut() {
            details.loading = false;
        }
        return;
    }
    let Some((section, id, known)) = st.browse_view.details.as_ref().map(|details| {
        (
            details.section,
            details.id,
            details.profile.clone().filter(|_| details.loading),
        )
    }) else {
        return;
    };
    if st.browse_view.details_job.is_running() {
        // The running read reports for another mod; `poll` starts this one when it lands.
        return;
    }
    let wake = st.wake.clone();
    let data_dir = crate::data::data_dir();
    st.browse_view
        .details_job
        .start(wake, move || Ok(fetch_details(section, id, known, data_dir)));
}

/// Apply finished reads (details, categories, the top list, the mirror, previews) and the
/// titlebar search once typing pauses. Called from `app::poll`.
pub fn poll(st: &mut State, tx: &mut Tx) -> bool {
    let copied = super::w4::poll_copy(st, tx);
    let mut changed = poll_search(st, tx) | copied;
    // One-press installs: queue rows, the install worker, Installed and Update states.
    changed |= super::quick::poll(st);
    if st.screen == Screen::Browse {
        // All guard themselves: one read per session, none while one runs.
        start_top(st);
        super::shelf::start(st);
        start_persisted_load(st);
        start_mirror_load(st);
        let section = st.browse_view.filters.section;
        start_categories(st, section);
        // The hero picker and "browse by hero" read the Skins tree, whichever tab is open.
        start_categories(st, BrowseSection::Mods);
        if st.browse_view.tab == Tab::Collections
            && st.browse_view.collections.is_none()
            && !st.browse_view.collections_job.is_running()
        {
            collections::start_collections(st, 1);
        }
        if st.browse_view.mirror.is_some()
            && st
                .browse_view
                .last_sync
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(300))
        {
            start_mirror_sync(st);
        }
        start_mirror_save(st);
    }
    super::hidden::poll(st);
    changed |= super::sources::poll(st);
    if st.browse_view.audio.poll() {
        if let Some(error) = st.browse_view.audio.error.clone() {
            st.status = error;
        }
        changed = true;
    }
    if let Some(result) = st.browse_view.persisted_job.try_take() {
        if let Ok(saved) = result {
            changed |= apply_persisted(st, saved);
        } else {
            st.browse_view.loaded_persisted = true;
        }
    }
    if let Some(result) = st.browse_view.mirror_job.try_take() {
        if let Ok(mirror) = result {
            st.browse_view.mirror_summary = mirror.summary();
            st.browse_view.download_counts = download_counts(&mirror);
            st.browse_view.mirror = Some(mirror);
            refresh_creator_hits(st);
            // Offline from the start: the first read had no copy to show.
            if st.prefs.offline && st.browse.page.is_none() && st.screen == Screen::Browse {
                crate::data::start_browse_fetch(st, 1);
            }
        }
        changed = true;
    }
    if let Some(result) = st.browse_view.mirror_sync_job.try_take() {
        if let Ok(mirror) = result {
            st.browse_view.mirror_summary = mirror.summary();
            st.browse_view.download_counts = download_counts(&mirror);
            st.browse_view.mirror = Some(mirror);
            refresh_creator_hits(st);
        }
        changed = true;
    }
    if st.browse_view.mirror_save_job.try_take().is_some() {
        changed = true;
    }
    if let Some(result) = st.browse_view.vpk_job.try_take() {
        match result {
            Ok(choices) => {
                st.install.busy = false;
                st.install.vpks = choices;
                st.install.unchecked.clear();
                st.install.log = Some(if st.install.vpks.len() > 1 {
                    crate::screens::settings::i18n::fill(
                        crate::screens::settings::i18n::tr("This archive holds {} files. Each is checked; uncheck the ones you do not want."),
                        &[&st.install.vpks.len().to_string()],
                    )
                } else {
                    "The archive has one mod.".to_string()
                });
                st.install.tone = Tone::Info;
            }
            Err(error) => {
                st.install.busy = false;
                let cancelled = error == super::CANCELLED;
                st.install.tone = if cancelled { Tone::Quiet } else { Tone::Bad };
                st.install.log = Some(if cancelled {
                    crate::screens::settings::i18n::tr("Cancelled. Nothing was installed.").to_string()
                } else {
                    error
                });
            }
        }
        st.install.cancel = None;
        changed = true;
    }
    if let Some(result) = st.browse_view.collections_job.try_take() {
        match result {
            Ok(load) => {
                if load.page > 1
                    && let Some(existing) = st.browse_view.collections.as_mut()
                {
                    let known: HashSet<u64> =
                        existing.items.iter().map(|item| item.id).collect();
                    existing.items.extend(
                        load.items
                            .into_iter()
                            .filter(|item| !known.contains(&item.id)),
                    );
                    existing.page = load.page;
                    existing.total = load.total;
                    existing.complete = load.complete;
                } else {
                    st.browse_view.collections = Some(load);
                }
            }
            Err(error) => {
                st.browse_view.collections = Some(CollectionLoad {
                    page: 1,
                    total: 0,
                    complete: true,
                    items: Vec::new(),
                    error: Some(error),
                });
            }
        }
        changed = true;
    }
    changed |= poll_collection(st);
    if let Some(result) = st.browse_view.more_job.try_take() {
        apply_more(st, result);
        changed = true;
    }
    if let Some(result) = st.browse_view.prefetch_job.try_take() {
        if let Ok((id, profile)) = result {
            st.browse_view.prefetched.retain(|(known, _)| *known != id);
            st.browse_view.prefetched.push((id, profile));
            let len = st.browse_view.prefetched.len();
            if len > 24 {
                st.browse_view.prefetched.drain(0..len - 24);
            }
        }
        changed = true;
    }
    changed |= poll_hover_prefetch(st, tx);
    changed |= super::tree::poll(st);
    if let Some(result) = st.browse_view.categories_job.try_take() {
        match result {
            Ok((section, list)) => {
                st.browse_view.categories.retain(|(owner, _)| *owner != section);
                // Hero portraits show in the hero picker.
                let icons: Vec<String> = list
                    .iter()
                    .flat_map(|root| root.children.iter())
                    .filter_map(|child| child.icon_url.clone())
                    .collect();
                crate::data::queue_images(st, icons, false);
                st.browse_view.categories.push((section, list));
                super::tree::start_deepen(st, section);
            }
            Err(_) => {
                let section = st.browse_view.filters.section;
                st.browse_view.categories_failed.push(section);
            }
        }
        let section = st.browse_view.filters.section;
        if !st.browse_view.categories_failed.contains(&section) {
            start_categories(st, section);
        }
        changed = true;
    }
    changed |= super::shelf::poll(st);
    if let Some(result) = st.browse_view.top_job.try_take() {
        let top = result.unwrap_or_default();
        st.browse_view.top = Some(top);
        changed = true;
    }
    // Every cover the landing shows (spotlight and shelves, current and next page), through
    // the shared gate: hidden or mature art is never fetched. Asked again whenever what the
    // landing shows changes, an image is evicted or the queue is cleared.
    changed |= super::foryou::refresh(st);
    super::shelf::queue_covers(st);
    changed |= super::top::tick(st, tx);
    if let Some(result) = st.browse_view.details_job.try_take() {
        match result {
            Ok(load) => apply_details(st, load),
            Err(error) => {
                if let Some(details) = st.browse_view.details.as_mut() {
                    details.loading = false;
                    details.error = Some(error);
                }
            }
        }
        changed = true;
    }
    changed
}

/// The titlebar search reads GameBanana once typing pauses. A pasted id or link opens that
/// submission instead.
pub(crate) fn poll_search(st: &mut State, tx: &mut Tx) -> bool {
    let typed = st.browse_query.text.trim().to_string();
    if typed == st.browse_view.applied_search {
        st.browse_view.search_edited = None;
        return false;
    }
    let Some(edited) = st.browse_view.search_edited else {
        st.browse_view.search_edited = Some(Instant::now());
        tx.wake_after(SEARCH_PAUSE);
        return false;
    };
    let waited = edited.elapsed();
    if waited < SEARCH_PAUSE {
        tx.wake_after(SEARCH_PAUSE - waited);
        return false;
    }
    st.browse_view.search_edited = None;
    if let Some((section, id)) = pasted_submission(&typed) {
        st.browse_view.applied_search = typed;
        if st.screen == Screen::Browse {
            show_details(st, Details::from_id(section, id, "", None));
        }
        return true;
    }
    st.browse_view.applied_search = typed;
    refresh_creator_hits(st);
    if st.demo {
        return true;
    }
    tx.scroll_to_instant("browse-scroll", 0.0, 0.0);
    st.browse_view.visible = 0..0;
    st.browse_view.refreshing = true;
    st.browse_view.auto_streak = 0;
    crate::data::start_browse_fetch(st, 1);
    true
}

pub(crate) fn apply_details(st: &mut State, load: DetailsLoad) {
    let Some(mut details) = st.browse_view.details.take() else {
        return;
    };
    if details.id != load.id {
        st.browse_view.details = Some(details);
        start_details_fetch(st);
        return;
    }
    details.loading = false;
    match load.profile {
        Ok(profile) => {
            details.error = None;
            // A pasted link or a top pick opens before its name and dates are known.
            if !profile.name.is_empty() {
                details.name = profile.name.clone();
            }
            if details.added_ts == 0 {
                details.added_ts = profile.added_ts;
                details.updated_ts = profile.updated_ts;
            }
            details.nsfw |= profile.nsfw;
            if details.author.is_empty() {
                details.author = profile
                    .submitter
                    .as_ref()
                    .map(|m| m.name.clone())
                    .unwrap_or_default();
            }
            details.profile = Some(profile);
        }
        Err(error) => details.error = Some(error),
    }
    details.comments = Some(load.comments);
    details.updates = Some(load.updates);
    details.requirements = Some(load.requirements);
    if load.update_cache.is_some() {
        st.browse_view.update_cache = load.update_cache;
    }
    details.comments_page = 1;
    details.updates_page = 1;
    // The hero the page mentions, unless the player already picked one.
    if details.hero_override.is_none() {
        let (name, category, sub) = details
            .profile
            .as_ref()
            .map(|profile| {
                (
                    profile.name.clone(),
                    profile.category.clone(),
                    profile.super_category.clone(),
                )
            })
            .unwrap_or_else(|| (details.name.clone(), String::new(), String::new()));
        details.hero = detected_hero(st, &name, &category, &sub);
    }
    // A mod page carries the download count list rows lack; keep it in the local copy.
    if let Some(profile) = details.profile.clone() {
        let model = details.section.model().to_string();
        // DMM `208df742`, `6bdb2970`: a mod GameBanana no longer shows publicly leaves the
        // saved catalog and Featured; a lifted withhold lets it back in.
        if let Some(mirror) = st.browse_view.mirror.as_mut() {
            let moved = if profile.is_hidden() {
                mirror.remove(&model, load.id)
            } else {
                mirror.restore(&model, load.id)
            };
            if moved {
                st.browse_view.mirror_dirty = true;
                super::w4::forget_featured(st);
            }
        }
        if let Some(mirror) = st.browse_view.mirror.as_mut()
            && mirror.upsert_downloads(&model, load.id, profile.downloads)
        {
            st.browse_view.mirror_dirty = true;
            st.browse_view
                .download_counts
                .insert(load.id, profile.downloads);
        }
    }
    let urls = details_image_urls(&details);
    crate::data::queue_images(st, urls, true);
    st.browse_view.details = Some(details);
    start_mirror_save(st);
}

pub(crate) fn download_counts(mirror: &Mirror) -> std::collections::HashMap<u64, u64> {
    mirror
        .entries
        .iter()
        .filter_map(|entry| entry.downloads.map(|downloads| (entry.id, downloads)))
        .collect()
}

/// Recompute the creator rail from the mirror for the applied search.
pub(crate) fn refresh_creator_hits(st: &mut State) {
    let search = st.browse_view.applied_search.trim().to_string();
    let hits = st
        .browse_view
        .mirror
        .as_ref()
        .map(|mirror| mirror.creators(&search, 8))
        .unwrap_or_default();
    st.browse_view.creator_hits = hits;
}

/// Start a details read after the pointer rests on a card for a moment. The profile lands in
/// the cache, so the press opens filled in instead of on a skeleton.
pub(crate) fn poll_hover_prefetch(st: &mut State, tx: &mut Tx) -> bool {
    let Some((id, since)) = st.browse_view.hover_since else {
        return false;
    };
    if st.browse_view.hovered != Some(id) {
        st.browse_view.hover_since = None;
        return false;
    }
    if st
        .browse_view
        .prefetched
        .iter()
        .any(|(known, _)| *known == id)
        || st.browse_view.details.as_ref().is_some_and(|d| d.id == id)
        || st.browse_view.prefetch_job.is_running()
        || st.demo
        || rune::is_headless()
    {
        return false;
    }
    const PREFETCH_PAUSE: Duration = Duration::from_millis(250);
    let waited = since.elapsed();
    if waited < PREFETCH_PAUSE {
        tx.wake_after(PREFETCH_PAUSE - waited);
        return false;
    }
    let Some(card) = card_by_id(st, id) else {
        return false;
    };
    let section = BrowseSection::from_model(&card.model);
    let wake = st.wake.clone();
    st.browse_view.prefetch_job.start(wake, move || {
        gamebanana::fetch_profile(section, id)
            .map(|profile| (id, profile))
            .map_err(|error| error.to_string())
    });
    false
}

/// Append a comments or changelog page.
pub(crate) fn apply_more(st: &mut State, result: Result<MoreLoad, String>) {
    let Ok(load) = result else {
        return;
    };
    let Some(details) = st.browse_view.details.as_mut() else {
        return;
    };
    if details.id != load.id {
        return;
    }
    match (load.kind, load.result) {
        (MoreKind::Comments, Ok(MorePage::Comments(page))) => {
            if let Some(Ok(existing)) = details.comments.as_mut() {
                let known: HashSet<u64> = existing.items.iter().map(|item| item.id).collect();
                existing
                    .items
                    .extend(page.items.into_iter().filter(|item| !known.contains(&item.id)));
                existing.total = page.total;
            }
            details.comments_page = load.page;
        }
        (MoreKind::Updates, Ok(MorePage::Updates(page))) => {
            if let Some(Ok(existing)) = details.updates.as_mut() {
                let known: HashSet<u64> = existing.items.iter().map(|item| item.id).collect();
                existing
                    .items
                    .extend(page.items.into_iter().filter(|item| !known.contains(&item.id)));
                existing.total = page.total;
            }
            details.updates_page = load.page;
        }
        _ => {}
    }
}

pub(crate) fn load_more_comments(st: &mut State) {
    let Some((section, id, page)) = st.browse_view.details.as_ref().map(|details| {
        (details.section, details.id, details.comments_page + 1)
    }) else {
        return;
    };
    if st.browse_view.more_job.is_running() {
        return;
    }
    let wake = st.wake.clone();
    st.browse_view.more_job.start(wake, move || {
        let page_read = gamebanana::fetch_comments_page(section, id, page, COMMENTS_PER_PAGE)
            .map_err(|error| error.to_string())?;
        Ok(MoreLoad {
            id,
            kind: MoreKind::Comments,
            page,
            result: Ok(MorePage::Comments(page_read)),
        })
    });
}

pub(crate) fn load_more_updates(st: &mut State) {
    let Some((section, id, page)) = st.browse_view.details.as_ref().map(|details| {
        (details.section, details.id, details.updates_page + 1)
    }) else {
        return;
    };
    if st.browse_view.more_job.is_running() {
        return;
    }
    let wake = st.wake.clone();
    st.browse_view.more_job.start(wake, move || {
        let page_read = gamebanana::fetch_updates_page(section, id, page, UPDATES_PER_PAGE)
            .map_err(|error| error.to_string())?;
        Ok(MoreLoad {
            id,
            kind: MoreKind::Updates,
            page,
            result: Ok(MorePage::Updates(page_read)),
        })
    });
}

/// Walk the shown grid with the arrow keys while a mod is open. Cards hidden or filtered out
/// are skipped, and a pasted submission that is not in the grid stays put.
pub(crate) fn step_details(st: &mut State, next: bool) {
    let Some(id) = st.browse_view.details.as_ref().map(|details| details.id) else {
        return;
    };
    let shown = shown_indices(st);
    let order: Vec<u64> = st
        .browse
        .page
        .as_ref()
        .map(|page| {
            shown
                .iter()
                .filter_map(|index| page.mods.get(*index))
                .map(|card| card.id)
                .collect()
        })
        .unwrap_or_default();
    let Some(at) = order.iter().position(|card| *card == id) else {
        return;
    };
    let target = if next {
        order.get(at + 1).or_else(|| order.first())
    } else {
        at.checked_sub(1).and_then(|last| order.get(last)).or_else(|| order.last())
    };
    if let Some(target) = target.copied()
        && target != id
    {
        open_details(st, target);
    }
}

/// The slot the mod sits in, from the library store and the install journal.
pub(crate) fn installed_slot(s: &State, id: u64) -> Option<u16> {
    super::quick::owned_slot(s, id)
}

/// A newer GameBanana file than the installed one, from the update check's last read. None
/// when the check has not seen this mod yet or knows it is current.
pub(crate) fn available_update(s: &State, id: u64) -> Option<RemoteFile> {
    let meta = s
        .installed
        .library
        .mods
        .iter()
        .find(|meta| meta.gamebanana_id == Some(id))?;
    let remote = s
        .installed
        .updates
        .cache
        .mods
        .get(&id)
        .or_else(|| s.browse_view.update_cache.as_ref().and_then(|cache| cache.mods.get(&id)))?;
    // A repaired mod keeps matching its update (C3): the alias maps it back to the original.
    let baseline = deadlock_core::mod_updates::baseline_for_repaired(
        &s.installed.updates.journal,
        &s.installed.updates.aliases,
        id,
        meta.size_bytes,
        None,
        &meta.recorded_at,
    );
    match deadlock_core::mod_updates::compare(baseline, remote) {
        deadlock_core::mod_updates::UpdateState::Available(file) => Some(file),
        _ => None,
    }
}

/// Every image the details view shows, in the order it shows them.
pub(crate) fn details_image_urls(details: &Details) -> Vec<String> {
    let mut urls: Vec<String> = details.images().iter().map(|image| image.url.clone()).collect();
    if let Some(profile) = &details.profile {
        urls.extend(profile.category_icon.clone());
        urls.extend(profile.submitter.as_ref().and_then(avatar_url));
    }
    if let Some(Ok(comments)) = &details.comments {
        urls.extend(comments.items.iter().filter_map(|c| avatar_url(&c.poster)));
    }
    urls
}

/// GameBanana's default avatar is a GIF the image decoder does not read; an initial stands
/// in for it.
pub(crate) fn avatar_url(member: &ModMember) -> Option<String> {
    member
        .avatar_url
        .clone()
        .filter(|url| !url.to_ascii_lowercase().ends_with(".gif"))
}

/// Close the details and give back its gallery images; the card thumbnail stays for the grid.
pub(crate) fn close_details(st: &mut State) {
    let Some(details) = st.browse_view.details.take() else {
        return;
    };
    let mut urls = details_image_urls(&details);
    urls.extend(details.images().iter().map(|image| image.full_url.clone()));
    for url in urls {
        if Some(&url) != details.thumbnail_url.as_ref() {
            st.thumbnails.remove(&url);
        }
    }
}

/// The card is hidden by id, whatever its section.
pub(crate) fn mod_hidden(s: &State, id: u64) -> bool {
    s.browse_view.hidden_mods.contains(&id)
}

pub(crate) fn author_hidden(s: &State, author: &str) -> bool {
    !author.is_empty()
        && s
            .browse_view
            .hidden_authors
            .iter()
            .any(|hidden| hidden.eq_ignore_ascii_case(author))
}

pub(crate) fn hide_mod(st: &mut State, id: u64) {
    crate::screens::settings::product::action(st, Some(id), deadlock_core::product_data::ModAction::NotInterested);
    let (name, model, thumb, author) = st
        .browse_view
        .details
        .as_ref()
        .filter(|details| details.id == id)
        .map(|details| {
            (details.name.clone(), details.section.model().to_string(), details.thumbnail_url.clone(), details.author.clone())
        })
        .or_else(|| {
            card_by_id(st, id).map(|card| (card.name.clone(), card.model.clone(), card.thumbnail_url.clone(), card.author.clone()))
        })
        .unwrap_or((String::new(), "Mod".to_string(), None, String::new()));
    if !mod_hidden(st, id) {
        st.browse_view.hidden_mods.insert(id);
        st.browse_view
            .hidden_names
            .insert(id, if name.is_empty() { format!("{model} {id}") } else { name.clone() });
        st.browse_view.hidden_meta.insert(
            hidden::mod_key(id),
            hidden::HiddenMeta {
                hidden_at: gamebanana::now_unix(),
                thumb,
                author: (!author.is_empty()).then_some(author),
                ..hidden::HiddenMeta::default()
            },
        );
    }
    close_details(st);
    st.browse_view.context = None;
    save_browse_state(st);
    st.status = format!("{name} is hidden. Restore it under Hidden items.");
}

pub(crate) fn unhide_mod(st: &mut State, id: u64) {
    st.browse_view.hidden_mods.remove(&id);
    st.browse_view.hidden_names.remove(&id);
    st.browse_view.hidden_meta.remove(&hidden::mod_key(id));
    save_browse_state(st);
}

pub(crate) fn hide_author(st: &mut State, author: String) {
    if author.is_empty() {
        return;
    }
    if !author_hidden(st, &author) {
        st.browse_view.hidden_authors.insert(author.clone());
        // Grimoire: remember the mod of theirs whose details it was hidden from.
        let from = st
            .browse_view
            .details
            .as_ref()
            .filter(|details| details.author.eq_ignore_ascii_case(&author))
            .map(|details| (details.id, details.name.clone()));
        st.browse_view.hidden_meta.insert(
            hidden::author_key(&author),
            hidden::HiddenMeta {
                hidden_at: gamebanana::now_unix(),
                from,
                ..hidden::HiddenMeta::default()
            },
        );
    }
    close_details(st);
    st.browse_view.context = None;
    save_browse_state(st);
    st.status = format!("Mods by {author} are hidden. Restore them under Hidden items.");
}

pub(crate) fn unhide_author(st: &mut State, author: &str) {
    st.browse_view
        .hidden_authors
        .retain(|hidden| !hidden.eq_ignore_ascii_case(author));
    st.browse_view.hidden_meta.remove(&hidden::author_key(author));
    save_browse_state(st);
}

pub fn update_ignored(s: &State, id: u64) -> bool {
    s.browse_view.ignored_updates.contains(&id)
}

/// Skip update notices for one mod, or check it again.
pub(crate) fn set_update_ignored(st: &mut State, id: u64, ignored: bool) {
    st.browse_view.ignored_updates.retain(|other| *other != id);
    if ignored {
        st.browse_view.ignored_updates.push(id);
    }
    save_browse_state(st);
}

pub(crate) fn open_url(st: &mut State, tx: &mut Tx, url: &str) {
    if let Err(error) = tx.open_url(url) {
        st.status = format!("The page could not be opened: {error}");
    }
}

