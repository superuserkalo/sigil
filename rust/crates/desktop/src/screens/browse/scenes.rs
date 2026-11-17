//! Fixture scenes for the Browse and details states.

use super::*;

// --- scenes ------------------------------------------------------------------

const FIXTURE_SUBFEED: &str = include_str!("../../../fixtures/browse/subfeed.json");
const FIXTURE_PROFILE: &str = include_str!("../../../fixtures/browse/profile.json");
const FIXTURE_POSTS: &str = include_str!("../../../fixtures/browse/posts.json");
const FIXTURE_UPDATES: &str = include_str!("../../../fixtures/browse/updates.json");
const FIXTURE_CATEGORIES: &str = include_str!("../../../fixtures/browse/categories.json");
const FIXTURE_TOP: &str = include_str!("../../../fixtures/browse/top.json");
const FIXTURE_COLLECTIONS: &str = include_str!("../../../fixtures/browse/collections.json");
/// Haze under Skins on GameBanana.
const HAZE: u64 = 33311;

/// The fixture grid with stored categories and the given filters, read for those filters.
pub(crate) fn scene_with(s: &mut State, edit: impl FnOnce(&mut Filters)) {
    s.screen = crate::model::Screen::Browse;
    if let Ok(list) = serde_json::from_str::<Vec<BrowseCategory>>(FIXTURE_CATEGORIES) {
        if rune::is_headless() {
            for url in list
                .iter()
                .flat_map(|root| root.children.iter())
                .filter_map(|child| child.icon_url.clone())
            {
                if let Some(bytes) = crate::data::fetch_thumb_bytes(&url) {
                    s.pending_thumbs.push((url, bytes));
                }
            }
        }
        s.browse_view.categories = vec![(BrowseSection::Mods, list)];
    }
    s.browse_view.favorites = Some(Vec::new());
    s.browse_view.tab = Tab::Mods;
    edit(&mut s.browse_view.filters);
    scene_fixture(s);
}

/// Filters on: the chip row, active dropdowns and the dimmed count.
pub fn scene_filters(s: &mut State) {
    scene_with(s, |f| {
        f.category = Some(Pick { id: 33295, name: "Skins".into() });
        f.sub = Some(Pick { id: HAZE, name: "Haze".into() });
        f.period = BrowsePeriod::Week;
        f.featured = true;
        f.hide_outdated = true;
        f.order = Some(BrowseOrder::MostLiked);
    });
    s.browse_view.applied_search = "hair".into();
    s.browse_query = TextEdit::new("hair");
    // The fixture page stands for these filters.
    let key = query_key(s, &current_query(s));
    if let Some(page) = s.browse.page.as_mut() {
        page.key = key;
    }
}

/// Filtering everything out: the same picks with the match mode inverted.
pub fn scene_filters_exclude(s: &mut State) {
    scene_with(s, |f| {
        f.category = Some(Pick { id: 33295, name: "Skins".into() });
        f.hero = Some("Haze".into());
        f.exclude = true;
        f.date_field = DateField::Updated;
        f.period = BrowsePeriod::Month;
    });
}

/// The hero picker open under Skins.
pub fn scene_hero_menu(s: &mut State) {
    scene_with(s, |f| {
        f.category = Some(Pick { id: 33295, name: "Skins".into() });
    });
    s.browse_view.menu = Some(FilterMenu::Sub);
}

pub fn scene_sort_menu(s: &mut State) {
    scene_with(s, |_| {});
    s.browse_view.menu = Some(FilterMenu::Sort);
}

pub fn scene_more_menu(s: &mut State) {
    scene_with(s, |f| f.owned = Owned::NotInstalled);
    s.browse_view.menu = Some(FilterMenu::More);
}

/// The date menu: field, window and a custom range.
pub fn scene_date_menu(s: &mut State) {
    scene_with(s, |f| {
        f.date_field = DateField::Updated;
        f.period = BrowsePeriod::Month;
    });
    s.browse_view.menu = Some(FilterMenu::Date);
}

/// The view menu: layout, card size, pages, volume and the saved catalog.
pub fn scene_view_menu(s: &mut State) {
    scene_with(s, |_| {});
    s.browse_view.mirror_summary = "3,204 mods saved, 1 of 3 sections read in full".into();
    s.browse_view.menu = Some(FilterMenu::View);
}

/// The card context menu over the fixture grid.
pub fn scene_context_menu(s: &mut State) {
    scene_with(s, |_| {});
    if let Some(card) = s
        .browse
        .page
        .as_ref()
        .and_then(|page| page.mods.first())
    {
        s.browse_view.context = Some(card.id);
    }
}

/// The list layout over the fixture grid.
pub fn scene_list(s: &mut State) {
    scene_with(s, |_| {});
    s.browse_view.layout = Layout::List;
}

/// Numbered pages with the fixture as page 2 of 40.
pub fn scene_paged(s: &mut State) {
    scene_with(s, |_| {});
    s.browse_view.paged = true;
    if let Some(page) = s.browse.page.as_mut() {
        page.page = 2;
        page.page_count = 40;
        page.total = 2000;
        page.complete = false;
    }
}

/// Live: Skins for one hero, past month, most viewed. Reads the network like `browse`.
pub fn scene_live_filtered(s: &mut State) {
    s.screen = crate::model::Screen::Browse;
    s.browse_view.tab = Tab::Mods;
    s.browse_view.filters.category = Some(Pick { id: 33295, name: "Skins".into() });
    s.browse_view.filters.sub = Some(Pick { id: HAZE, name: "Haze".into() });
    s.browse_view.filters.period = BrowsePeriod::Month;
    s.browse_view.filters.order = Some(BrowseOrder::MostViewed);
    if rune::is_headless() {
        crate::data::browse_now(s, 1);
    }
}

/// Live: sounds, most liked, searched.
pub fn scene_live_sounds(s: &mut State) {
    s.screen = crate::model::Screen::Browse;
    s.browse_view.tab = Tab::Sounds;
    s.browse_view.filters.section = BrowseSection::Sounds;
    s.browse_view.filters.order = Some(BrowseOrder::MostLiked);
    s.browse_view.applied_search = "haze".into();
    s.browse_query = TextEdit::new("haze");
    if rune::is_headless() {
        crate::data::browse_now(s, 1);
    }
}

pub fn scene_category_menu(s: &mut State) {
    scene_with(s, |_| {});
    s.browse_view.menu = Some(FilterMenu::Category);
}

/// Favorites on with nothing saved.
pub fn scene_favorites_empty(s: &mut State) {
    scene_with(s, |f| f.favorites = true);
    let key = query_key(s, &current_query(s));
    s.browse.page = local_page(s, key);
}

/// The grid from a stored GameBanana page, so it renders without the network. Thumbnails
/// still come from the disk cache or the network when either answers.
pub fn scene_fixture(s: &mut State) {
    s.screen = crate::model::Screen::Browse;
    if rune::is_headless() {
        crate::data::browse_from_body(s, FIXTURE_SUBFEED);
        // Stored data: there is no later page to read in a snapshot.
        if let Some(page) = s.browse.page.as_mut() {
            page.complete = true;
        }
    }
}

/// The landing dashboard: trending rails and browse by hero over the fixture grid.
pub fn scene_dashboard(s: &mut State) {
    scene_with(s, |_| {});
    if let Ok(top) = gamebanana::parse_top_subs(FIXTURE_TOP) {
        let urls: Vec<String> = top.iter().filter_map(|top| top.image_url.clone()).collect();
        for url in urls {
            if rune::is_headless()
                && let Some(bytes) = crate::data::fetch_thumb_bytes(&url)
            {
                s.pending_thumbs.push((url, bytes));
            }
        }
        s.browse_view.top = Some(top);
    }
}

/// The mirror with a few rows, for the creator rail and the saved catalog row.
pub fn scene_mirror(s: &mut State) {
    scene_dashboard(s);
    let mut mirror = Mirror::default();
    for (id, name, author, author_id) in [
        (741478, "Long Hair Haze", "747shove", 5544023),
        (741608, "Random Respawn Dances", "747shove", 5544023),
        (741500, "Haze jacket", "hazel", 22),
    ] {
        mirror.upsert(catalog_mirror::MirrorEntry {
            id,
            model: "Mod".into(),
            name: name.into(),
            author: author.into(),
            author_id,
            category: "Skins".into(),
            subcategory: "Haze".into(),
            profile_url: format!("https://gamebanana.com/mods/{id}"),
            added_ts: 1_780_000_000,
            updated_ts: 1_790_000_000,
            likes: id % 100,
            views: 10_000,
            comments: 12,
            downloads: Some(4_000 + id % 1000),
            ..catalog_mirror::MirrorEntry::default()
        });
    }
    mirror.meta.steps = 4;
    s.browse_view.mirror_summary = mirror.summary();
    s.browse_view.download_counts = mirror
        .entries
        .iter()
        .filter_map(|entry| entry.downloads.map(|downloads| (entry.id, downloads)))
        .collect();
    s.browse_view.mirror = Some(mirror);
    s.browse_view.applied_search = "haze".into();
    s.browse_query = TextEdit::new("haze");
    s.browse_view.creator_hits = vec![
        CreatorHit {
            id: 5544023,
            name: "747shove".into(),
            count: 2,
            newest: 1_790_000_000,
        },
        CreatorHit {
            id: 22,
            name: "hazel".into(),
            count: 1,
            newest: 1_790_000_000,
        },
    ];
    // The grid reads the saved copy, so its cards carry download counts.
    let cards: Vec<BrowseCard> = s
        .browse_view
        .mirror
        .as_ref()
        .map(|mirror| mirror.entries.iter().map(mirror_card).collect())
        .unwrap_or_default();
    let key = query_key(s, &current_query(s));
    s.browse.page = Some(BrowsePageView {
        key,
        page: 1,
        per_page: cards.len(),
        total: cards.len(),
        complete: true,
        page_count: 1,
        mods: cards,
    });
    s.browse_view.from_mirror = true;
}

/// The Collections tab from the stored page.
pub fn scene_collections(s: &mut State) {
    s.screen = crate::model::Screen::Browse;
    s.browse_view.tab = Tab::Collections;
    if let Ok(read) = gamebanana::parse_collections(FIXTURE_COLLECTIONS, 1, 6) {
        if rune::is_headless() {
            for url in read.items.iter().filter_map(|item| item.image_url.clone()) {
                if let Some(bytes) = crate::data::fetch_thumb_bytes(&url) {
                    s.pending_thumbs.push((url, bytes));
                }
            }
        }
        s.browse_view.collections = Some(CollectionLoad {
            page: 1,
            total: read.total,
            complete: read.complete,
            items: read.items,
            error: None,
        });
    }
}

/// One collection open, its items taken from the fixture grid.
pub fn scene_collection_open(s: &mut State) {
    scene_collections(s);
    scene_fixture(s);
    let Some(load) = s.browse_view.collections.as_ref() else {
        return;
    };
    let Some(summary) = load.items.first().cloned() else {
        return;
    };
    let items: Vec<BrowseCard> = s
        .browse
        .page
        .as_ref()
        .map(|page| page.mods.iter().map(clone_card).collect())
        .unwrap_or_default();
    s.browse_view.collection = Some(CollectionOpen {
        summary,
        items,
        page: 1,
        complete: true,
        selected: HashSet::new(),
        job: Job::default(),
        error: None,
    });
}

/// The Maps tab, with the Maps category picked.
pub fn scene_maps(s: &mut State) {
    scene_with(s, |f| {
        f.category = Some(Pick {
            id: MAPS_CATEGORY,
            name: "Maps".into(),
        });
    });
    s.browse_view.tab = Tab::Maps;
    let key = query_key(s, &current_query(s));
    if let Some(page) = s.browse.page.as_mut() {
        page.key = key;
    }
}

/// The mixed feed tab: Mods, Sounds and WiPs in one grid.
pub fn scene_combined(s: &mut State) {
    scene_with(s, |_| {});
    s.browse_view.tab = Tab::All;
    let key = query_key(s, &current_query(s));
    if let Some(page) = s.browse.page.as_mut() {
        page.key = key;
    }
}

/// The mod details over the fixture grid, for the mature mod in the fixture.
pub fn scene_details(s: &mut State) {
    scene_with(s, |_| {});
    if !rune::is_headless() {
        return;
    }
    let profile = gamebanana::parse_mod_profile_page(FIXTURE_PROFILE);
    let Ok(profile) = profile else {
        return;
    };
    let Some(card) = card_by_id(s, profile.id) else {
        return;
    };
    let mut details = Details::from_card(card);
    details.loading = false;
    details.profile = Some(profile);
    details.comments = Some(gamebanana::parse_mod_comments(FIXTURE_POSTS).map_err(|e| e.to_string()));
    details.updates = Some(gamebanana::parse_mod_updates(FIXTURE_UPDATES).map_err(|e| e.to_string()));
    details.changelog_open = true;
    if let Some(Ok(updates)) = &details.updates
        && let Some(first) = updates.items.first()
    {
        details.open_updates.insert(first.id);
    }
    s.browse_view.revealed.insert(details.id);
    for url in details_image_urls(&details) {
        if let Some(bytes) = crate::data::fetch_thumb_bytes(&url) {
            s.pending_thumbs.push((url, bytes));
        }
    }
    s.browse_view.details = Some(details);
}

/// The details with every added panel: notices, requirements, hero, videos, donations,
/// tags and more comments to load.
pub fn scene_details_extras(s: &mut State) {
    scene_details(s);
    let Some(mut details) = s.browse_view.details.take() else {
        return;
    };
    let Some(profile) = details.profile.as_mut() else {
        s.browse_view.details = Some(details);
        return;
    };
    profile.videos = vec![
        "https://www.youtube.com/watch?v=PbThLMeH-tg".into(),
        "https://vimeo.com/1247326746".into(),
    ];
    profile.obsolete = true;
    if let Some(member) = profile.submitter.as_mut() {
        member.donations = vec![gamebanana::ProfileLink {
            title: "Ko-fi".into(),
            url: "https://ko-fi.com/example".into(),
        }];
    }
    details.requirements = Some(Ok(vec![Requirement {
        name: "Base skin pack".into(),
        url: "https://gamebanana.com/mods/741478".into(),
        mod_id: Some(741478),
    }]));
    details.hero = Some("Haze".into());
    details.hero_override = None;
    // An installed slot and an offered update, so the tag row shows every state.
    s.installed.library.mods.push(deadlock_core::import::ModMeta {
        slot: 7,
        size_bytes: 1,
        name: details.name.clone(),
        author: Some(details.author.clone()),
        thumbnail_url: None,
        gamebanana_id: Some(details.id),
        source: "browse".into(),
        recorded_at: "2026-11-01T10:00:00Z".into(),
    });
    let mut remote = deadlock_core::mod_updates::RemoteMod::default();
    remote.files.push(deadlock_core::mod_updates::RemoteFile {
        file_id: 99,
        file_name: "newer.zip".into(),
        added: 1_800_000_000,
        archived: false,
    });
    s.installed.updates.cache.mods.insert(details.id, remote);
    s.browse_view.details = Some(details);
}

/// The details docked beside the grid.
pub fn scene_docked(s: &mut State) {
    scene_details(s);
    s.browse_view.docked = true;
    s.browse_view.revealed.clear();
}

/// The hero picker over the details.
pub fn scene_hero_picker(s: &mut State) {
    scene_details(s);
    s.browse_view.hero_picker = Some(HeroTarget::Details);
}

/// Hidden mods and creators, with the management list open.
pub fn scene_hidden(s: &mut State) {
    scene_with(s, |_| {});
    s.browse_view.hidden_mods = [741478, 83198].into_iter().collect();
    s.browse_view
        .hidden_names
        .insert(741478, "Long Hair Haze".into());
    s.browse_view
        .hidden_names
        .insert(83198, "Drifter Flow".into());
    s.browse_view.hidden_authors = ["someone".to_string()].into_iter().collect();
    s.browse_view.panel = Some(Panel::Hidden);
}

/// Ignored updates, with their list open.
pub fn scene_ignored(s: &mut State) {
    scene_with(s, |_| {});
    s.browse_view.ignored_updates = vec![741478, 741608];
    s.browse_view.panel = Some(Panel::Ignored);
}

/// The install dialog on an archive that holds several mods.
pub fn scene_vpk_picker(s: &mut State) {
    scene_with(s, |_| {});
    if let Some(card) = s.browse.page.as_ref().and_then(|page| page.mods.first()) {
        s.install = InstallView::for_mod(card.id, &card.name);
        s.install.vpks = vec![
            VpkChoice {
                path: "pak01_dir.vpk".into(),
                bytes: 42_000_000,
                chunks: 3,
            },
            VpkChoice {
                path: "pak02_dir.vpk".into(),
                bytes: 12_000_000,
                chunks: 1,
            },
        ];
        s.install.log = Some("This archive holds 2 mods. Choose the one you want.".into());
        s.install.tone = Tone::Info;
    }
}

/// A few sound rows without the network, so the section, its hero filter and the inline
/// player all have something to show.
pub fn scene_sounds(s: &mut State) {
    scene_with(s, |f| {
        f.section = BrowseSection::Sounds;
        f.hero = Some("Haze".into());
        f.order = Some(BrowseOrder::MostLiked);
    });
    s.browse_view.tab = Tab::Sounds;
    s.browse_view.applied_search.clear();
    s.browse_query = TextEdit::new("");
    let names = [
        "Haze kill sound",
        "Haze ability sounds",
        "Haze footsteps",
        "Haze ult chime",
        "Haze reload",
        "Haze dash",
    ];
    let cards: Vec<BrowseCard> = names
        .iter()
        .enumerate()
        .map(|(index, name)| BrowseCard {
            id: 83000 + index as u64,
            name: name.to_string(),
            author: "Someone".into(),
            category: "Killsounds".into(),
            updated_at: String::new(),
            likes: 40 - index as u64,
            views: 20_000,
            profile_url: format!("https://gamebanana.com/sounds/{}", 83000 + index),
            thumbnail_url: None,
            version: None,
            added_ts: 1_780_000_000,
            updated_ts: 1_790_000_000,
            nsfw: false,
            model: "Sound".into(),
            audio_url: Some(format!("https://files.gamebanana.com/preview/sounds/{index}.mp3")),
            featured: false,
            comments: 3,
            author_id: 77,
            obsolete: false,
        })
        .collect();
    let key = query_key(s, &current_query(s));
    s.browse.page = Some(BrowsePageView {
        key,
        page: 1,
        per_page: cards.len(),
        total: cards.len(),
        complete: true,
        page_count: 1,
        mods: cards,
    });
}

/// A sound playing in the grid.
pub fn scene_audio(s: &mut State) {
    scene_sounds(s);
    if let Some(sound) = s
        .browse
        .page
        .as_ref()
        .and_then(|page| page.mods.first())
    {
        s.browse_view.audio.playing = Some(sound.id);
        s.browse_view.audio.volume = 0.65;
    }
}

// --- one-press installs ------------------------------------------------------------------

/// The fixture grid's first `n` mod cards, as (id, name).
fn first_mods(s: &State, n: usize) -> Vec<(u64, String)> {
    s.browse
        .page
        .as_ref()
        .map(|page| {
            page.mods
                .iter()
                .filter(|card| card.model.eq_ignore_ascii_case("mod"))
                .take(n)
                .map(|card| (card.id, card.name.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// Mod `id` installed in `slot` with a newer file on GameBanana since.
fn stage_update(s: &mut State, id: u64, name: &str, slot: u8) {
    use deadlock_core::mod_updates::{RemoteFile, RemoteMod};
    s.installed.library.mods.push(deadlock_core::import::ModMeta {
        slot,
        size_bytes: 1,
        name: name.to_string(),
        author: None,
        thumbnail_url: None,
        gamebanana_id: Some(id),
        source: "browse".into(),
        recorded_at: "2025-01-01T00:00:00Z".into(),
    });
    s.installed.updates.cache.mods.insert(
        id,
        RemoteMod {
            files: vec![RemoteFile {
                file_id: 1,
                file_name: "newer.zip".into(),
                added: 1_790_000_000,
                archived: false,
            }],
        },
    );
}

/// The grid with the first card downloading at 40% and the second untouched: one press
/// changes one card.
pub fn scene_install_downloading(s: &mut State) {
    scene_with(s, |_| {});
    if let Some((id, name)) = first_mods(s, 1).into_iter().next() {
        super::quick::stage(s, id, &name, super::quick::Step::Downloading(Some(0.4)));
    }
}

/// The grid with the first card installing and the rest as they were.
pub fn scene_install_installing(s: &mut State) {
    scene_with(s, |_| {});
    if let Some((id, name)) = first_mods(s, 1).into_iter().next() {
        super::quick::stage(s, id, &name, super::quick::Step::Installing);
    }
}

/// The grid with the first card installed.
pub fn scene_install_installed(s: &mut State) {
    scene_with(s, |_| {});
    if let Some((id, name)) = first_mods(s, 1).into_iter().next() {
        super::quick::stage(s, id, &name, super::quick::Step::Done);
    }
}

/// The grid with the first card installed and a newer file out.
pub fn scene_install_update(s: &mut State) {
    scene_with(s, |_| {});
    if let Some((id, name)) = first_mods(s, 1).into_iter().next() {
        stage_update(s, id, &name, 4);
    }
}

/// Every state at once: downloading, installing, installed, update, queued, failed, and an
/// untouched card.
pub fn scene_install_mixed(s: &mut State) {
    scene_with(s, |_| {});
    let mods = first_mods(s, 7);
    use super::quick::Step;
    let steps = [
        Some(Step::Downloading(Some(0.4))),
        Some(Step::Installing),
        Some(Step::Done),
        None,
        Some(Step::Queued),
        Some(Step::Failed("GameBanana returned HTTP 503 for this file.".into())),
    ];
    for (index, (id, name)) in mods.iter().enumerate() {
        match steps.get(index) {
            Some(Some(step)) => super::quick::stage(s, *id, name, step.clone()),
            Some(None) => stage_update(s, *id, name, 6),
            None => {}
        }
    }
}

/// The list layout with the same states.
pub fn scene_install_list(s: &mut State) {
    scene_install_mixed(s);
    s.browse_view.layout = Layout::List;
}

/// The details with its Install button downloading.
pub fn scene_install_details(s: &mut State) {
    scene_details(s);
    if let Some(d) = s.browse_view.details.as_ref() {
        let (id, name) = (d.id, d.name.clone());
        super::quick::stage(s, id, &name, super::quick::Step::Downloading(Some(0.4)));
    }
}

/// The details installed.
pub fn scene_install_details_done(s: &mut State) {
    scene_details(s);
    if let Some(d) = s.browse_view.details.as_ref() {
        let (id, name) = (d.id, d.name.clone());
        super::quick::stage(s, id, &name, super::quick::Step::Done);
    }
}

/// The details' file menu open beside its Install button.
pub fn scene_install_files_menu(s: &mut State) {
    scene_details(s);
    if let Some(d) = s.browse_view.details.as_ref() {
        s.browse_view.installs.files_menu = Some(d.id);
    }
}
