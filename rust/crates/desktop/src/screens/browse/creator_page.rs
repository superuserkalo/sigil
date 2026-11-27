//! A creator's page: when Browse is scoped to one creator, a header over their mods with the
//! profile, totals across everything they posted and their links.
//!
//! Behaviour after Deadlock Mod Manager's mod author profiles (GPL-3.0, behaviour only);
//! Grimoire's artist mode is the grid scoped to one creator, which Browse already has. The data
//! is `deadlock_core::creators::fetch_creator`: GameBanana's member profile and their Mod and
//! Sound lists, read once per creator per run on a worker.
//!
//! `open` is the way in from anywhere (a library row, a mod page): it switches to Browse and
//! scopes the grid to the creator. `open_for_mod` first asks GameBanana who posted a mod, for
//! library rows saved before Sigil kept creator ids.

use std::collections::HashMap;

use deadlock_core::creators::{self as core, CreatorPage};
use rune::{Color, El, SurfaceFit, Tx, col, el, row, text};

use crate::app::State;
use crate::jobs::Job;
use crate::screens::settings::i18n::fill;
use crate::ui::{ActionKind, BODY_MED, H1, SMALL, Ui};

use super::Pick;

const DE: &[(&str, &str)] = &[
    ("Creator", "Ersteller"),
    ("Member since {}", "Mitglied seit {}"),
    ("{} subscribers", "{} Abonnenten"),
    ("Submissions", "Beiträge"),
    ("Likes", "Likes"),
    ("Views", "Aufrufe"),
    ("Downloads", "Downloads"),
    ("Comments", "Kommentare"),
    ("Last update", "Letztes Update"),
    ("Most liked: {}", "Meiste Likes: {}"),
    ("Open on GameBanana", "Auf GameBanana öffnen"),
    ("Reading the creator's page", "Seite des Erstellers wird gelesen"),
    ("The creator's page did not load: {}", "Die Seite des Erstellers hat nicht geladen: {}"),
    ("Try again", "Erneut versuchen"),
    ("Counts the newest {} submissions; GameBanana lists more.", "Zählt die neuesten {} Beiträge; GameBanana listet mehr."),
    ("Downloads from mods whose page you opened.", "Downloads aus Mods, deren Seite du geöffnet hast."),
    ("Looking up the creator", "Ersteller wird gesucht"),
    ("GameBanana does not name a creator for this mod.", "GameBanana nennt für diesen Mod keinen Ersteller."),
    ("Sigil is offline.", "Sigil ist offline."),
    ("More from this creator", "Mehr von diesem Ersteller"),
    ("More from {}", "Mehr von {}"),
    ("{} mod", "{} Mod"),
    ("{} mods", "{} Mods"),
    ("{} likes", "{} Likes"),
    ("{} views", "{} Aufrufe"),
    ("Updated {}", "Aktualisiert {}"),
    ("View all their mods", "Alle Mods ansehen"),
    ("Back to Browse", "Zurück zu Durchsuchen"),
    ("Browse", "Durchsuchen"),
    ("Show again", "Wieder zeigen"),
    ("Hide creator", "Ersteller ausblenden"),
    ("Hide every mod by this creator in Browse. Undo under Hidden items.", "Blendet jeden Mod dieses Erstellers in Durchsuchen aus. Rückgängig unter Ausgeblendet."),
];

fn tr(text: &str) -> &str {
    use crate::screens::settings::i18n::{self, Lang};
    if i18n::current() == Lang::German
        && let Some((_, de)) = DE.iter().find(|(en, _)| *en == text)
    {
        return de;
    }
    i18n::tr(text)
}

/// Creator pages read this run, by member id. A failure is kept so it is not retried each frame.
#[derive(Default)]
pub struct CreatorState {
    pub pages: HashMap<u64, Result<CreatorPage, String>>,
    pub job: Job<(u64, Result<CreatorPage, String>)>,
    /// A library row asked GameBanana for its creator.
    pub lookup: Job<Result<(u64, String), String>>,
}

/// The Installed card's menu entry for a mod's creator.
pub fn more_label(author: &str) -> String {
    if author.trim().is_empty() {
        tr("More from this creator").to_string()
    } else {
        fill(tr("More from {}"), &[author.trim()])
    }
}

/// Scope Browse to one creator.
pub fn open(st: &mut State, id: u64, name: String) {
    open_with(st, None, id, name);
}

/// Scope Browse to one creator from anywhere, closing an open mod's details first. Back (the
/// page's Back link, mouse Back, Alt+Left) returns to the grid as it was.
pub fn open_with(st: &mut State, tx: Option<&mut Tx>, id: u64, name: String) {
    if id == 0 {
        return;
    }
    super::close_details(st);
    st.go(crate::model::Screen::Browse);
    super::set_filters(st, tx, move |f| {
        f.creator = Some(Pick { id, name });
        f.favorites = false;
        if f.owned == super::Owned::Installed {
            f.owned = super::Owned::Any;
        }
    });
}

/// Find who posted GameBanana mod `mod_id`, then [`open`] their page.
pub fn open_for_mod(st: &mut State, mod_id: u64) {
    if st.browse_view.w4.creator.lookup.is_running() {
        return;
    }
    if st.prefs.offline {
        crate::notify::push(st, crate::notify::Notice::info(tr("Sigil is offline.")));
        return;
    }
    crate::notify::push(st, crate::notify::Notice::info(tr("Looking up the creator")));
    let wake = st.wake.clone();
    st.browse_view.w4.creator.lookup.start(wake, move || {
        let profile = deadlock_core::gamebanana::fetch_mod_profile(mod_id).map_err(|error| error.to_string())?;
        Ok(profile
            .submitter
            .filter(|member| member.id > 0)
            .map(|member| (member.id, member.name))
            .ok_or_else(|| tr("GameBanana does not name a creator for this mod.").to_string()))
    });
}

/// Start the read for the scoped creator when it is not known yet. Called from the poll.
fn want(st: &mut State) {
    let Some(id) = st.browse_view.filters.creator.as_ref().map(|pick| pick.id).filter(|id| *id > 0) else {
        return;
    };
    let c = &mut st.browse_view.w4.creator;
    if c.pages.contains_key(&id) || c.job.is_running() || st.prefs.offline || rune::is_headless() {
        return;
    }
    let wake = st.wake.clone();
    c.job.start(wake, move || Ok((id, core::fetch_creator(id).map_err(|error| error.to_string()))));
}

pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(Ok((id, result))) = st.browse_view.w4.creator.job.try_take() {
        if let Ok(page) = &result {
            st.browse_view.w4.authors.entry(id).or_insert_with(|| Ok(page.profile.clone()));
            let mut urls: Vec<String> = Vec::new();
            urls.extend(page.extras.hd_avatar.clone().or_else(|| page.profile.avatar_url.clone()));
            urls.extend(page.extras.banner.clone());
            // The avatar comes from the API response: only GameBanana's own hosts are fetched.
            urls.retain(|url| deadlock_core::gamebanana::is_gamebanana_url(url) && !url.to_ascii_lowercase().ends_with(".gif"));
            crate::data::queue_images(st, urls, false);
        }
        st.browse_view.w4.creator.pages.insert(id, result);
        changed = true;
    }
    if let Some(result) = st.browse_view.w4.creator.lookup.try_take() {
        match result.and_then(|inner| inner) {
            Ok((id, name)) => open(st, id, name),
            Err(error) => crate::notify::push(st, crate::notify::Notice::error(error)),
        }
        changed = true;
    }
    want(st);
    changed
}

fn grouped(value: u64) -> String {
    let text = value.to_string();
    let mut out = String::new();
    for (index, digit) in text.chars().enumerate() {
        if index > 0 && (text.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

fn month_year(ts: i64) -> String {
    let (year, month, _) = super::store::civil_from_days(ts.div_euclid(86_400));
    format!("{} {year}", MONTHS[(month as usize).clamp(1, 12) - 1])
}

fn day(ts: i64) -> String {
    crate::prefs::show_unix_date(ts)
}

fn picture(s: &State, u: &Ui, url: Option<&String>, name: &str, size: f32) -> El {
    match url.and_then(|url| s.thumbnails.get(url)) {
        Some(pixels) => el()
            .size(size, size)
            .radius(size / 2.0)
            .pixels_fit(pixels, SurfaceFit::Fill)
            .a11y_hidden(),
        None => el()
            .size(size, size)
            .radius(size / 2.0)
            .center()
            .fill(u.accent.alpha(0.15))
            .border(1.0, u.accent.alpha(0.30))
            .a11y_hidden()
            .child(
                text(name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_else(|| "?".into()))
                    .font(H1)
                    .color(u.accent),
            ),
    }
}

/// One line about a creator for the details' creator box: their mod count and likes once
/// their page was read this run, else what the press does.
pub fn facts_line(s: &State, id: u64) -> String {
    match s.browse_view.w4.creator.pages.get(&id) {
        Some(Ok(page)) => {
            let totals = core::stats(&page.mods);
            let mut facts = vec![count_label(totals.submissions as u64, "{} mod", "{} mods", page.more)];
            if totals.likes > 0 {
                facts.push(fill(tr("{} likes"), &[&grouped(totals.likes)]));
            }
            facts.join("  ·  ")
        }
        _ => tr("View all their mods").to_string(),
    }
}

fn count_label(count: u64, one: &str, many: &str, more: bool) -> String {
    let text = fill(tr(if count == 1 { one } else { many }), &[&grouped(count)]);
    if more { format!("{text}+") } else { text }
}

/// The page's Back link, to the grid as it was before the creator was opened.
fn back_link(u: &Ui) -> El {
    row()
        .key("browse-creator-back")
        .h(28.0)
        .pl(6.0)
        .pr(10.0)
        .ml(-6.0)
        .gap(6.0)
        .center_y()
        .radius(crate::ui::r(6.0))
        .hover(u.hover)
        .press(u.over.alpha(0.08))
        .focusable(true)
        .role("button")
        .label(tr("Back to Browse").to_string())
        .cursor(rune::Cursor::Pointer)
        .on_press_tx(|st: &mut State, tx: &mut Tx| {
            super::set_filters(st, Some(tx), |f| f.creator = None);
        })
        .child(rune::icon(&crate::icons::ARROW_LEFT, 14.0).color(u.muted).a11y_hidden())
        .child(text(tr("Browse").to_string()).font(BODY_MED).color(u.muted).a11y_hidden())
}

/// The creator's name: two lines before anything is cut, and the whole name as its tooltip.
fn name_text(u: &Ui, name: &str) -> El {
    text(name.to_string()).font(H1).color(u.ink).max_lines(2).wrap_anywhere().w_full().help(name.to_string())
}

/// Hide creator, or Show again when they are hidden: the same list as a card's menu.
fn hide_action(s: &State, u: &Ui, name: &str) -> Option<El> {
    if name.trim().is_empty() {
        return None;
    }
    let author = name.to_string();
    Some(if super::author_hidden(s, name) {
        u.icon_action(&crate::icons::EYE, tr("Show again"), ActionKind::Secondary)
            .key("browse-creator-unhide")
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| super::unhide_author(st, &author))
    } else {
        u.icon_action(&super::EYE_OFF, tr("Hide creator"), ActionKind::Ghost)
            .key("browse-creator-hide")
            .help(tr("Hide every mod by this creator in Browse. Undo under Hidden items."))
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| super::hide_author(st, author.clone()))
    })
}

/// The header over a creator scoped grid: Back, then one row with the avatar, the full name,
/// one line of facts and the creator's links, and the actions at the right edge. `None` when
/// Browse is not scoped to one creator.
pub fn header(s: &State, u: &Ui) -> Option<El> {
    let pick = s.browse_view.filters.creator.as_ref()?;
    let card = row()
        .key("browse-creator-page")
        .w_full()
        .padding(16.0)
        .gap(16.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .fill(Color::WHITE.alpha(0.02))
        .border(1.0, u.hair_strong);
    let frame = |card: El| col().w_full().gap(8.0).child(back_link(u)).child(card);
    let mut actions = row().gap(8.0).center_y();
    let body = match s.browse_view.w4.creator.pages.get(&pick.id) {
        None => {
            actions = actions.children(hide_action(s, u, &pick.name));
            return Some(frame(
                card.child(picture(s, u, None, &pick.name, 56.0))
                    .child(
                        col()
                            .grow(1.0)
                            .min_w(0.0)
                            .gap(4.0)
                            .child(name_text(u, &pick.name))
                            .child(text(tr("Reading the creator's page")).font(SMALL).color(u.muted)),
                    )
                    .child(actions),
            ));
        }
        Some(Err(error)) => {
            let id = pick.id;
            actions = actions
                .child(
                    u.icon_action(&crate::icons::REFRESH_CW, tr("Try again"), ActionKind::Secondary)
                        .key("browse-creator-retry")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| {
                            st.browse_view.w4.creator.pages.remove(&id);
                        }),
                )
                .children(hide_action(s, u, &pick.name));
            return Some(frame(
                card.child(picture(s, u, None, &pick.name, 56.0))
                    .child(
                        col()
                            .grow(1.0)
                            .min_w(0.0)
                            .gap(4.0)
                            .child(name_text(u, &pick.name))
                            .child(text(fill(tr("The creator's page did not load: {}"), &[error])).font(SMALL).color(u.rose).wrap()),
                    )
                    .child(actions),
            ));
        }
        Some(Ok(page)) => page,
    };
    let page = body;
    let totals = core::stats(&page.mods);
    let name = if page.profile.name.is_empty() { pick.name.clone() } else { page.profile.name.clone() };
    // Facts GameBanana gives: what they posted for Deadlock, its likes and views, then the
    // member's own title, join date and subscribers.
    let mut facts: Vec<String> = vec![count_label(totals.submissions as u64, "{} mod", "{} mods", page.more)];
    if totals.likes > 0 {
        facts.push(fill(tr("{} likes"), &[&grouped(totals.likes)]));
    }
    if totals.views > 0 {
        facts.push(fill(tr("{} views"), &[&grouped(totals.views)]));
    }
    if page.extras.subscribers > 0 {
        facts.push(fill(tr("{} subscribers"), &[&grouped(page.extras.subscribers)]));
    }
    if let Some(joined) = page.extras.joined {
        facts.push(fill(tr("Member since {}"), &[&month_year(joined)]));
    }
    if totals.last_update > 0 {
        facts.push(fill(tr("Updated {}"), &[&day(totals.last_update)]));
    }
    let mut who = col().grow(1.0).min_w(0.0).gap(4.0);
    if let Some(title) = page.extras.title.as_ref().filter(|title| !title.trim().is_empty()) {
        who = who.child(text(title.clone()).font(SMALL).color(u.faint).truncate().w_full());
    }
    who = who
        .child(name_text(u, &name))
        .child(text(facts.join("  ·  ")).font(SMALL).color(u.muted).wrap().w_full())
        .children(super::w4::author_links(s, u, pick.id));
    if page.more {
        who = who.child(
            text(fill(tr("Counts the newest {} submissions; GameBanana lists more."), &[&totals.submissions.to_string()]))
                .font(SMALL)
                .color(u.faint),
        );
    }
    // From the API response: the button only ever opens a GameBanana page.
    let url = Some(page.profile.profile_url.clone())
        .filter(|url| deadlock_core::gamebanana::is_gamebanana_url(url))
        .unwrap_or_default();
    actions = actions
        .children((!url.is_empty()).then(|| {
            u.icon_action(&crate::icons::EXTERNAL_LINK, tr("Open on GameBanana"), ActionKind::Secondary)
                .key("browse-creator-gamebanana")
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |st: &mut State, tx: &mut Tx| super::detail_state::open_url(st, tx, &url))
        }))
        .children(hide_action(s, u, &name));
    Some(frame(
        card.child(picture(s, u, page.extras.hd_avatar.as_ref().or(page.profile.avatar_url.as_ref()), &name, 56.0))
            .child(who)
            .child(actions),
    ))
}

/// The empty grid under a creator's page: their mods are hidden, or this section has none.
pub fn empty_copy(s: &State) -> Option<(&'static str, &'static str)> {
    let pick = s.browse_view.filters.creator.as_ref()?;
    Some(if super::author_hidden(s, &pick.name) {
        ("You hid this creator", "Their mods stay out of Browse. Show them again from the page above.")
    } else {
        ("No mods from this creator here", "Try another section or fewer filters, or open their GameBanana page.")
    })
}

/// `browse-creator`: the fixture grid scoped to a creator, with their page over it.
pub fn scene_creator(s: &mut State) {
    super::scenes::scene_with(s, |_| {});
    stage_scene(s);
    // The fixture page stands for this creator's grid.
    let key = super::query_key(s, &super::current_query(s));
    if let Some(page) = s.browse.page.as_mut() {
        page.key = key;
    }
}

/// `browse-creator-loading`: the page while GameBanana is read.
pub fn scene_creator_loading(s: &mut State) {
    scene_creator(s);
    s.browse_view.w4.creator.pages.clear();
    s.browse.page = None;
    s.browse.loading = true;
}

/// `browse-creator-error`: the profile did not load; Try again in the header.
pub fn scene_creator_error(s: &mut State) {
    scene_creator(s);
    s.browse_view.w4.creator.pages.insert(1_874_253, Err("GameBanana answered 503".to_string()));
}

/// `browse-creator-empty`: the creator has nothing in this section.
pub fn scene_creator_empty(s: &mut State) {
    scene_creator(s);
    if let Some(page) = s.browse.page.as_mut() {
        page.mods.clear();
        page.total = 0;
    }
}

/// Browse scoped to a creator with their page read.
pub fn stage_scene(s: &mut State) {
    use deadlock_core::library_facts::{AuthorContact, AuthorProfile, ContactKind};
    let id = 1_874_253;
    s.browse_view.filters.creator = Some(Pick { id, name: "Wren".to_string() });
    let cards: Vec<_> = s
        .browse
        .page
        .as_ref()
        .map(|page| page.mods.iter().map(super::store::clone_card).collect())
        .unwrap_or_default();
    let mods: Vec<deadlock_core::gamebanana::BrowseMod> = cards
        .iter()
        .take(9)
        .enumerate()
        .map(|(index, card)| deadlock_core::gamebanana::BrowseMod {
            id: card.id,
            name: card.name.clone(),
            profile_url: card.profile_url.clone(),
            model: "Mod".into(),
            author: "Wren".into(),
            category: if index % 3 == 0 { "HUD".into() } else { card.category.clone() },
            subcategory: String::new(),
            sub_category_id: 0,
            version: None,
            updated_at: card.updated_at.clone(),
            likes: card.likes,
            views: card.views,
            has_files: true,
            thumbnail_url: None,
            added_ts: 1_740_000_000 + index as i64 * 86_400,
            updated_ts: 1_763_000_000 - index as i64 * 86_400,
            nsfw: false,
            audio_url: None,
            featured: false,
            comments: 3 + index as u64,
            author_id: id,
            obsolete: false,
        })
        .collect();
    s.browse_view.w4.authors.insert(
        id,
        Ok(AuthorProfile {
            member_id: id,
            name: "Wren".to_string(),
            profile_url: format!("https://gamebanana.com/members/{id}"),
            avatar_url: None,
            contacts: vec![AuthorContact {
                kind: ContactKind::Kofi,
                value: "wren".to_string(),
                url: Some("https://ko-fi.com/wren".to_string()),
            }],
        }),
    );
    s.browse_view.w4.creator.pages.insert(
        id,
        Ok(CreatorPage {
            profile: AuthorProfile {
                member_id: id,
                name: "Wren".to_string(),
                profile_url: format!("https://gamebanana.com/members/{id}"),
                avatar_url: None,
                contacts: Vec::new(),
            },
            extras: core::ProfileExtras {
                title: Some("Skin maker".to_string()),
                joined: Some(1_726_000_000),
                subscribers: 1_284,
                banner: None,
                hd_avatar: None,
            },
            mods,
            more: false,
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg_attr(not(feature = "languages"), ignore = "translations are parked: --features languages")]
    fn every_string_has_german() {
        let mut seen = std::collections::HashSet::new();
        for (en, de) in DE {
            assert!(seen.insert(*en), "duplicate {en:?}");
            assert_eq!(en.matches("{}").count(), de.matches("{}").count(), "holes in {en:?}");
        }
        let source = include_str!("creator_page.rs");
        let body = source.split("#[cfg(test)]").next().unwrap();
        for (at, _) in body.match_indices("tr(\"") {
            let rest = &body[at + 4..];
            let literal = &rest[..rest.find('"').unwrap()];
            assert!(DE.iter().any(|(en, _)| *en == literal), "untranslated {literal:?}");
        }
        for label in ["Submissions", "Likes", "Views", "Comments", "Downloads", "Last update"] {
            assert!(DE.iter().any(|(en, _)| *en == label));
        }
    }

    #[test]
    fn dates_read_in_utc() {
        assert_eq!(month_year(1_726_000_000), "Sep 2024");
        assert_eq!(day(1_763_000_000), "13.11.2025");
    }

    #[test]
    fn numbers_group_by_thousands() {
        assert_eq!(grouped(1_284), "1,284");
        assert_eq!(grouped(12), "12");
    }
}

#[cfg(test)]
mod plan_tests {
    use deadlock_core::gamebanana::BrowseOrder;

    use super::*;

    /// A creator's page is Browse's own read with the member filter: paged, in the grid's
    /// order, never the unfiltered subfeed.
    #[test]
    fn the_creator_grid_reads_their_pages_in_the_chosen_order() {
        let mut st = State::default();
        st.browse_view.filters.creator = Some(Pick { id: 4_183_371, name: "Kirill".into() });
        for (order, token) in [(BrowseOrder::Newest, "Generic_Newest"), (BrowseOrder::MostLiked, "Generic_MostLiked")] {
            st.browse_view.filters.order = Some(order);
            let query = super::super::current_query(&st);
            assert_eq!(query.submitter, Some(4_183_371));
            assert!(!query.uses_subfeed());
            let url = query.url(2, 50);
            assert!(url.contains("/Mod/Index?_nPage=2&_nPerpage=50"), "{url}");
            assert!(url.contains(&format!("_sSort={token}")), "{url}");
            assert!(url.contains("_aFilters%5BGeneric_Submitter%5D=4183371"), "{url}");
        }
        // The default (popular) order still reads the member's own index.
        st.browse_view.filters.order = None;
        assert!(!super::super::current_query(&st).uses_subfeed());
    }

    #[test]
    fn facts_say_what_the_press_does_until_the_page_is_read() {
        let mut st = State::default();
        assert_eq!(facts_line(&st, 7), "View all their mods");
        st.browse_view.w4.creator.pages.insert(7, Err("down".into()));
        assert_eq!(facts_line(&st, 7), "View all their mods");
    }

    #[test]
    fn hidden_creators_get_their_own_empty_page() {
        let mut st = State::default();
        assert!(empty_copy(&st).is_none());
        st.browse_view.filters.creator = Some(Pick { id: 7, name: "Wren".into() });
        assert_eq!(empty_copy(&st).unwrap().0, "No mods from this creator here");
        st.browse_view.hidden_authors.insert("wren".into());
        assert_eq!(empty_copy(&st).unwrap().0, "You hid this creator");
    }
}
