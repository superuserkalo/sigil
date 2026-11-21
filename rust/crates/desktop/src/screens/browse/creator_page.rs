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
use rune::{Color, El, SurfaceFit, Tx, col, el, row, spacer, text};

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

/// Scope Browse to one creator.
pub fn open(st: &mut State, id: u64, name: String) {
    if id == 0 {
        return;
    }
    st.go(crate::model::Screen::Browse);
    super::set_filters(st, None, move |f| {
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
            urls.retain(|url| !url.to_ascii_lowercase().ends_with(".gif"));
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
    super::store::date_input(ts)
}

fn stat(u: &Ui, label: &str, value: String) -> El {
    col()
        .gap(0.0)
        .child(text(value).font(BODY_MED).color(u.ink))
        .child(text(tr(label).to_string()).font(SMALL).color(u.faint))
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

/// The header over a creator scoped grid. `None` when Browse is not scoped to one creator.
pub fn header(s: &State, u: &Ui) -> Option<El> {
    let pick = s.browse_view.filters.creator.as_ref()?;
    let mut card = row()
        .key("browse-creator-page")
        .w_full()
        .padding(16.0)
        .gap(16.0)
        .items_start()
        .radius(crate::ui::r(12.0))
        .fill(Color::WHITE.alpha(0.02))
        .border(1.0, u.hair_strong);
    match s.browse_view.w4.creator.pages.get(&pick.id) {
        None => {
            Some(
                card.child(picture(s, u, None, &pick.name, 64.0)).child(
                    col()
                        .grow(1.0)
                        .gap(4.0)
                        .child(text(tr("Creator")).font(SMALL).color(u.faint))
                        .child(text(pick.name.clone()).font(H1).color(u.ink))
                        .child(text(tr("Reading the creator's page")).font(SMALL).color(u.muted)),
                ),
            )
        }
        Some(Err(error)) => {
            let id = pick.id;
            Some(
                card.child(picture(s, u, None, &pick.name, 64.0)).child(
                    col()
                        .grow(1.0)
                        .gap(6.0)
                        .child(text(pick.name.clone()).font(H1).color(u.ink))
                        .child(text(fill(tr("The creator's page did not load: {}"), &[error])).font(SMALL).color(u.rose).wrap())
                        .child(
                            u.action(tr("Try again"), ActionKind::Secondary)
                                .key("browse-creator-retry")
                                .cursor(rune::Cursor::Pointer)
                                .on_press(move |st: &mut State| {
                                    st.browse_view.w4.creator.pages.remove(&id);
                                }),
                        ),
                ),
            )
        }
        Some(Ok(page)) => {
            let totals = core::stats(&page.mods);
            let name = if page.profile.name.is_empty() { pick.name.clone() } else { page.profile.name.clone() };
            let mut meta: Vec<String> = Vec::new();
            if let Some(title) = &page.extras.title {
                meta.push(title.clone());
            }
            if let Some(joined) = page.extras.joined {
                meta.push(fill(tr("Member since {}"), &[&month_year(joined)]));
            }
            if page.extras.subscribers > 0 {
                meta.push(fill(tr("{} subscribers"), &[&grouped(page.extras.subscribers)]));
            }
            let downloads: u64 = page
                .mods
                .iter()
                .filter_map(|item| s.browse_view.download_counts.get(&item.id))
                .sum();
            let mut stats = row()
                .gap(24.0)
                .flex_wrap()
                .child(stat(u, "Submissions", grouped(totals.submissions as u64)))
                .child(stat(u, "Likes", grouped(totals.likes)))
                .child(stat(u, "Views", grouped(totals.views)))
                .child(stat(u, "Comments", grouped(totals.comments)));
            if downloads > 0 {
                stats = stats.child(stat(u, "Downloads", grouped(downloads)).help(tr("Downloads from mods whose page you opened.")));
            }
            if totals.last_update > 0 {
                stats = stats.child(stat(u, "Last update", day(totals.last_update)));
            }
            let mut body = col()
                .grow(1.0)
                .min_w(0.0)
                .gap(6.0)
                .child(text(tr("Creator")).font(SMALL).color(u.faint))
                .child(text(name.clone()).font(H1).color(u.ink).truncate())
                .children((!meta.is_empty()).then(|| text(meta.join("  ·  ")).font(SMALL).color(u.muted)))
                .children(super::w4::author_links(s, u, pick.id))
                .child(el().h(4.0))
                .child(stats);
            let categories: Vec<String> = totals
                .categories
                .iter()
                .take(5)
                .map(|(category, count)| format!("{category} {count}"))
                .collect();
            if !categories.is_empty() {
                body = body.child(text(categories.join("  ·  ")).font(SMALL).color(u.faint));
            }
            if let Some((_, top, likes)) = &totals.top
                && *likes > 0
            {
                body = body.child(text(fill(tr("Most liked: {}"), &[top])).font(SMALL).color(u.muted).truncate());
            }
            if page.more {
                body = body.child(
                    text(fill(tr("Counts the newest {} submissions; GameBanana lists more."), &[&totals.submissions.to_string()]))
                        .font(SMALL)
                        .color(u.faint),
                );
            }
            let url = page.profile.profile_url.clone();
            card = card
                .child(picture(s, u, page.extras.hd_avatar.as_ref().or(page.profile.avatar_url.as_ref()), &name, 64.0))
                .child(body)
                .child(spacer().w(0.0))
                .children((!url.is_empty()).then(|| {
                    u.action(tr("Open on GameBanana"), ActionKind::Secondary)
                        .key("browse-creator-gamebanana")
                        .cursor(rune::Cursor::Pointer)
                        .on_press_tx(move |st: &mut State, tx: &mut Tx| super::detail_state::open_url(st, tx, &url))
                }));
            Some(card)
        }
    }
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
        assert_eq!(day(1_763_000_000), "2025-11-13");
    }

    #[test]
    fn numbers_group_by_thousands() {
        assert_eq!(grouped(1_284), "1,284");
        assert_eq!(grouped(12), "12");
    }
}
