//! A GameBanana config's page: the author's files, the license, the download,
//! and the review of the gameinfo variants inside the archive.
//!
//! Nothing is cached until the player picks a file and confirms the review.
//! Downloading, extracting and reading the archive run on a worker; the page
//! only shows what came back. Applying the config afterwards is the ordinary
//! Performance Apply with its confirm.

use deadlock_core::performance::{
    self, Catalog, ConfigDef,
    hosted::{self as core, Page, Prepared, Variant},
};
use rune::{El, col, el, icon, row, text};

use crate::app::State;
use crate::icons;
use crate::screens::settings::i18n::fill;
use crate::jobs::Job;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, H2, MONO, SMALL, Ui};

use super::{catalog, config, data_dir, open_url, page, simulated, tr};

pub struct Hosted {
    pub id: String,
    pub page: Option<Page>,
    pub fetch: Job<Page>,
    pub file: Option<u64>,
    pub prepare: Job<Prepared>,
    pub install: Job<Catalog>,
    pub prepared: Option<Prepared>,
    pub variant: usize,
    pub message: Option<(String, bool)>,
}

impl Hosted {
    fn new(id: &str) -> Hosted {
        Hosted {
            id: id.to_string(),
            page: None,
            fetch: Job::default(),
            file: None,
            prepare: Job::default(),
            install: Job::default(),
            prepared: None,
            variant: 0,
            message: None,
        }
    }
}

/// Open a hosted config's page and read what GameBanana offers.
pub fn open(st: &mut State, id: &str) {
    st.performance.hosted = Some(Hosted::new(id));
    start_fetch(st);
}

fn start_fetch(st: &mut State) {
    let simulated = simulated(st);
    let Some(hosted) = st.performance.hosted.as_mut() else {
        return;
    };
    if hosted.page.is_some() || hosted.fetch.is_running() || simulated {
        return;
    }
    let id = hosted.id.clone();
    let wake = st.wake.clone();
    hosted.fetch.start(wake, move || {
        core::fetch_page(&id).map_err(|error| error.to_string())
    });
}

/// The file the review will download: the chosen one, else the newest.
fn chosen_file<'a>(hosted: &'a Hosted, def: &ConfigDef) -> Option<&'a deadlock_core::gamebanana::ModFile> {
    let page = hosted.page.as_ref()?;
    let pinned = def.gamebanana.and_then(|hosted| hosted.file_id);
    match hosted.file {
        Some(id) => page.files.iter().find(|file| file.id == id),
        None => page.chosen_file(pinned),
    }
}

fn start_prepare(st: &mut State) {
    let Some(hosted) = st.performance.hosted.as_ref() else {
        return;
    };
    if hosted.prepare.is_running() {
        return;
    }
    let Some(def) = catalog::config(&hosted.id) else {
        return;
    };
    let Some(file) = chosen_file(hosted, def).cloned() else {
        return;
    };
    let Some(game_root) = st.game_root.clone() else {
        if let Some(hosted) = st.performance.hosted.as_mut() {
            hosted.message = Some((
                tr("Set the game folder in Settings first.").to_string(),
                true,
            ));
        }
        return;
    };
    if simulated(st) {
        if let Some(hosted) = st.performance.hosted.as_mut() {
            hosted.prepared = Some(core::sample());
            hosted.variant = hosted.prepared.as_ref().map_or(0, |p| p.chosen);
        }
        return;
    }
    let id = hosted.id.clone();
    let data = data_dir();
    let wake = st.wake.clone();
    st.performance.hosted.as_mut().unwrap().prepare.start(wake, move || {
        let text = std::fs::read_to_string(performance::gameinfo_path(&game_root))
            .map_err(|error| fill(tr("The game's gameinfo.gi could not be read: {}"), &[&error.to_string()]))?;
        core::prepare(&id, &file, &data, &text).map_err(|error| error.to_string())
    });
}

fn start_install(st: &mut State) {
    let Some(hosted) = st.performance.hosted.as_ref() else {
        return;
    };
    if hosted.install.is_running() {
        return;
    }
    let Some(prepared) = hosted.prepared.clone() else {
        return;
    };
    let variant = hosted.variant;
    let data = data_dir();
    let wake = st.wake.clone();
    st.performance.hosted.as_mut().unwrap().install.start(wake, move || {
        core::install(&prepared, variant, &data).map_err(|error| error.to_string())?;
        Ok(performance::load(&data))
    });
}

pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    let fetched = st
        .performance
        .hosted
        .as_mut()
        .and_then(|hosted| hosted.fetch.try_take());
    if let Some(result) = fetched {
        if let Some(hosted) = st.performance.hosted.as_mut() {
            match result {
                Ok(page) => {
                    hosted.file = page
                        .chosen_file(
                            catalog::config(&hosted.id)
                                .and_then(|def| def.gamebanana)
                                .and_then(|hosted| hosted.file_id),
                        )
                        .map(|file| file.id);
                    hosted.page = Some(page);
                }
                Err(error) => hosted.message = Some((error, true)),
            }
        }
        changed = true;
    }
    let prepared = st
        .performance
        .hosted
        .as_mut()
        .and_then(|hosted| hosted.prepare.try_take());
    if let Some(result) = prepared {
        if let Some(hosted) = st.performance.hosted.as_mut() {
            match result {
                Ok(prepared) => {
                    hosted.variant = prepared.chosen;
                    hosted.prepared = Some(prepared);
                }
                Err(error) => hosted.message = Some((error, true)),
            }
        }
        changed = true;
    }
    let installed = st
        .performance
        .hosted
        .as_mut()
        .and_then(|hosted| hosted.install.try_take());
    if let Some(result) = installed {
        match result {
            Ok(catalog) => {
                let id = st.performance.hosted.as_ref().map(|hosted| hosted.id.clone());
                st.performance.catalog = Some(catalog);
                st.performance.fetch = super::Fetch::Idle;
                st.performance.hosted = None;
                st.performance.open = id.clone();
                if let Some(id) = id
                    && let Some(def) = catalog::config(&id)
                {
                    crate::notify::push(
                        st,
                        crate::notify::Notice::success(format!("{} is ready", def.name)),
                    );
                }
            }
            Err(error) => {
                if let Some(hosted) = st.performance.hosted.as_mut() {
                    hosted.message = Some((error, true));
                }
            }
        }
        changed = true;
    }
    changed
}

pub fn view(s: &State, u: &Ui) -> El {
    let Some(hosted) = s.performance.hosted.as_ref() else {
        return el();
    };
    let def = catalog::config(&hosted.id);
    let name = hosted
        .page
        .as_ref()
        .map(|page| page.name.clone())
        .or_else(|| def.map(|def| def.name.to_string()))
        .unwrap_or_else(|| hosted.id.clone());
    let author = hosted
        .page
        .as_ref()
        .map(|page| page.author.clone())
        .or_else(|| def.map(|def| def.author.to_string()))
        .unwrap_or_default();
    let tier = def.map(|def| def.tier.label()).unwrap_or("");

    let back = row().w_full().h(32.0).center_y().child(
        row()
            .key("perf-hosted-back")
            .h(28.0)
            .pl(6.0)
            .pr(8.0)
            .ml(-6.0)
            .gap(6.0)
            .center_y()
            .radius(crate::ui::r(6.0))
            .hover(u.hover)
            .focusable(true)
            .role("button")
            .label(tr("Back to Performance"))
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| st.performance.hosted = None)
            .child(icon(&icons::ARROW_LEFT, 14.0).color(u.muted).a11y_hidden())
            .child(text(tr("Performance")).font(BODY).color(u.muted).a11y_hidden()),
    );

    let cached = config(s, &hosted.id).is_some();
    let mut actions = row().gap(8.0).center_y();
    if cached {
        let id = hosted.id.clone();
        actions = actions.child(
            u.action(tr("Open details"), ActionKind::Primary)
                .key("perf-hosted-open")
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    st.performance.open = Some(id.clone());
                    st.performance.hosted = None;
                }),
        );
    }
    if let Some(page) = &hosted.page {
        let url = page.url.clone();
        actions = actions.child(
            u.action(tr("View on GameBanana"), ActionKind::Ghost)
                .key("perf-hosted-link")
                .cursor(rune::Cursor::Pointer)
                .on_press(move |_: &mut State| open_url(&url)),
        );
    }

    let meta = format!("{author}{}{}", if tier.is_empty() { "" } else { " · " }, tier);
    let mut out = page("perf-hosted-page").child(
        col()
            .w_full()
            .gap(4.0)
            .child(back)
            .child(
                row()
                    .w_full()
                    .gap(16.0)
                    .items_end()
                    .child(
                        col()
                            .grow(1.0)
                            .min_w(0.0)
                            .gap(4.0)
                            .child(text(name).font(H1).color(u.ink))
                            .child(text(meta).font(BODY).color(u.muted).truncate()),
                    )
                    .child(actions),
            ),
    );

    if let Some((line, error)) = &hosted.message {
        out = out.child(
            text(line.clone())
                .font(SMALL)
                .color(if *error { u.rose } else { u.muted })
                .wrap(),
        );
    }

    if let Some(page) = &hosted.page {
        if let Some(license) = &page.license {
            out = out.child(license_row(u, license, &page.url));
        }
        if !page.text.is_empty() {
            out = out.child(
                text(clip(&page.text, 700))
                    .font(BODY)
                    .color(u.prose)
                    .wrap(),
            );
        }
    }

    if hosted.prepared.is_some() {
        out = out.child(review(s, u, hosted));
    } else {
        out = out.child(files(s, u, hosted, cached));
    }
    out
}

fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    let cut = cut.rsplit_once(' ').map(|(head, _)| head).unwrap_or(&cut);
    format!("{cut}...")
}

fn license_row(u: &Ui, license: &str, url: &str) -> El {
    row()
        .key("perf-hosted-license")
        .w_full()
        .gap(8.0)
        .items_start()
        .border_sides([0.0, 1.0, 0.0, 0.0], u.hair)
        .pb(12.0)
        .child(
            el()
                .pt(2.0)
                .child(icon(&icons::INFO, 14.0).color(u.faint).a11y_hidden()),
        )
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(text(tr("License")).font(BODY_MED).color(u.ink))
                .child(text(clip(license, 320)).font(SMALL).color(u.muted).wrap()),
        )
        .child(
            text(tr("Read it on GameBanana"))
                .font(SMALL)
                .color(u.muted)
                .role("link")
                .focusable(true)
                .cursor(rune::Cursor::Pointer)
                .on_press({
                    let url = url.to_string();
                    move |_: &mut State| open_url(&url)
                }),
        )
}

fn files(_s: &State, u: &Ui, hosted: &Hosted, cached: bool) -> El {
    let Some(page) = &hosted.page else {
        if hosted.fetch.is_running() {
            return row()
                .key("perf-hosted-loading")
                .w_full()
                .h(52.0)
                .gap(8.0)
                .center_y()
                .child(icon(&icons::LOADER, 14.0).color(u.faint).a11y_hidden())
                .child(
                    text(tr("Reading the mod page from GameBanana"))
                        .font(BODY)
                        .color(u.muted),
                );
        }
        return col()
            .key("perf-hosted-empty")
            .w_full()
            .gap(8.0)
            .child(
                text(tr("The GameBanana page did not load."))
                    .font(BODY)
                    .color(u.muted),
            )
            .child(
                u.action(tr("Try again"), ActionKind::Secondary)
                    .key("perf-hosted-retry")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| start_fetch(st)),
            );
    };
    let def = catalog::config(&hosted.id);
    let chosen = chosen_file(hosted, def.expect("hosted def"));
    let preparing = hosted.prepare.is_running();
    let mut rows = col().w_full().border_sides([1.0, 0.0, 0.0, 0.0], u.hair);
    if page.files.is_empty() {
        rows = rows.child(
            row()
                .w_full()
                .h(52.0)
                .center_y()
                .child(
                    text(tr("The mod page lists no files to download."))
                        .font(BODY)
                        .color(u.muted),
                ),
        );
    }
    for file in &page.files {
        let on = chosen.is_some_and(|chosen| chosen.id == file.id);
        let id = file.id;
        let mut sub = format!("{} · {}", file_size(file.size_bytes), calendar(file.added_ts));
        if !file.description.is_empty() {
            sub = format!("{sub} · {}", clip(&file.description, 90));
        }
        rows = rows.child(
            row()
                .key(format!("perf-hosted-file-{}", file.id))
                .w_full()
                .h(52.0)
                .gap(12.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .hover(u.hover)
                .focusable(true)
                .role("radio")
                .checked(on)
                .label(format!("{} {}", if on { tr("Selected") } else { tr("Choose") }, file.name))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    if let Some(hosted) = st.performance.hosted.as_mut() {
                        hosted.file = Some(id);
                        hosted.prepared = None;
                    }
                })
                .child(
                    el().size(16.0, 16.0).center().child(
                        icon(if on { &icons::CIRCLE_CHECK } else { &icons::CIRCLE_X }, 16.0)
                            .color(if on { u.accent } else { u.faint })
                            .a11y_hidden(),
                    ),
                )
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(text(file.name.clone()).font(BODY_MED).color(u.ink).truncate())
                        .child(text(sub).font(SMALL).color(u.faint).truncate()),
                ),
        );
    }
    let mut primary = u.action(
        if preparing {
            tr("Downloading")
        } else {
            tr("Download and review")
        },
        ActionKind::Primary,
    )
    .key("perf-hosted-download")
    .disabled(preparing || chosen.is_none())
    .opacity(if preparing || chosen.is_none() { 0.45 } else { 1.0 })
    .cursor(rune::Cursor::Pointer)
    .on_press(start_prepare);
    if cached {
        primary = u.action(tr("Download again"), ActionKind::Secondary)
            .key("perf-hosted-download")
            .disabled(preparing)
            .cursor(rune::Cursor::Pointer)
            .on_press(start_prepare);
    }
    col()
        .w_full()
        .gap(8.0)
        .child(
            col()
                .w_full()
                .gap(2.0)
                .child(
                    text(if cached {
                        tr("Newer file")
                    } else {
                        tr("Choose a file")
                    })
                    .font(H2)
                    .color(u.ink),
                )
                .child(
                    text(if cached {
                        tr("Sigil already has this config. Download again to replace it with the newest file.")
                    } else {
                        tr("Some mod pages list several archives. Sigil reads the one you pick and shows the gameinfo files inside before anything is saved.")
                    })
                    .font(SMALL)
                    .color(u.muted)
                    .wrap(),
                ),
        )
        .child(rows)
        .child(row().w_full().pt(8.0).child(primary))
}

fn review(_s: &State, u: &Ui, hosted: &Hosted) -> El {
    let prepared = hosted.prepared.as_ref().expect("prepared");
    let installing = hosted.install.is_running();
    let mut rows = col().w_full().border_sides([1.0, 0.0, 0.0, 0.0], u.hair);
    for (index, variant) in prepared.variants.iter().enumerate() {
        let on = index == hosted.variant;
        rows = rows.child(variant_row(u, variant, on, index));
    }
    let chosen = prepared.variants.get(hosted.variant);
    let id = prepared.id.clone();
    col()
        .w_full()
        .gap(12.0)
        .child(
            col()
                .w_full()
                .gap(2.0)
                .child(text(tr("Review")).font(H2).color(u.ink))
                .child(
                    text(format!(
                        "{} from {}. Pick the settings file you want; Sigil writes it with the usual backup and lets you turn it off.",
                        prepared.version, prepared.file_name
                    ))
                    .font(SMALL)
                    .color(u.muted)
                    .wrap(),
                ),
        )
        .child(rows)
        .child(
            row()
                .w_full()
                .gap(16.0)
                .center_y()
                .pt(4.0)
                .child(
                    text(chosen.map(variant_line).unwrap_or_default())
                        .font(SMALL)
                        .color(u.muted)
                        .grow(1.0)
                        .min_w(0.0)
                        .wrap(),
                )
                .child(
                    u.action(tr("Cancel"), ActionKind::Ghost)
                        .key("perf-hosted-cancel")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| {
                            if let Some(hosted) = st.performance.hosted.as_mut() {
                                hosted.prepared = None;
                            }
                        }),
                )
                .child(
                    u.action(
                        if installing {
                            tr("Adding")
                        } else {
                            tr("Add to Performance")
                        },
                        ActionKind::Primary,
                    )
                    .key("perf-hosted-add")
                    .disabled(installing)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        let _ = &id;
                        start_install(st);
                    }),
                ),
        )
}

fn variant_row(u: &Ui, variant: &Variant, on: bool, index: usize) -> El {
    let label = format!(
        "{} settings{}{}",
        variant.convars,
        if variant.engine > 0 {
            format!(" · {} engine edits", variant.engine)
        } else {
            String::new()
        },
        if variant.ignored > 0 {
            format!(" · {} left out", variant.ignored)
        } else {
            String::new()
        }
    );
    let videos = if variant.videos.is_empty() {
        String::new()
    } else {
        format!(" · video settings: {}", variant.videos.join(", "))
    };
    row()
        .key(format!("perf-hosted-variant-{index}"))
        .w_full()
        .h(56.0)
        .gap(12.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .hover(u.hover)
        .focusable(true)
        .role("radio")
        .checked(on)
        .label(format!("{} {}", variant.label, label))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            if let Some(hosted) = st.performance.hosted.as_mut() {
                hosted.variant = index;
            }
        })
        .child(
            el().size(16.0, 16.0).center().child(
                icon(if on { &icons::CIRCLE_CHECK } else { &icons::CIRCLE_X }, 16.0)
                    .color(if on { u.accent } else { u.faint })
                    .a11y_hidden(),
            ),
        )
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(
                    text(variant.label.clone())
                        .font(BODY_MED)
                        .color(u.ink)
                        .truncate(),
                )
                .child(
                    text(format!("{label}{videos}"))
                        .font(SMALL)
                        .color(u.faint)
                        .truncate(),
                ),
        )
        .child(
            text(variant.path.clone())
                .font(MONO)
                .color(u.faint)
                .max_w(360.0)
                .truncate(),
        )
}

fn variant_line(variant: &Variant) -> String {
    format!(
        "{}: {} settings{}{}",
        variant.path,
        variant.convars,
        if variant.engine > 0 {
            format!(", {} engine edits", variant.engine)
        } else {
            String::new()
        },
        if variant.videos.is_empty() {
            String::new()
        } else {
            format!(", {}", variant.videos.join(", "))
        }
    )
}

fn file_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{bytes} bytes")
    }
}

fn calendar(ts: i64) -> String {
    deadlock_core::gamebanana::calendar_date(ts)
}

/// A stand-in page and review for the headless scenes.
pub(super) fn demo_page() -> Page {
    let file = |id: u64, name: &str, added: i64, size: u64, description: &str| {
        deadlock_core::gamebanana::ModFile {
            id,
            name: name.to_string(),
            size_bytes: size,
            downloads: 1200,
            added_ts: added,
            description: description.to_string(),
            download_url: format!("https://gamebanana.com/dl/{id}"),
            archived: false,
        }
    };
    Page {
        name: "Hanturaya's competitive".into(),
        author: "Hanturaya".into(),
        license: Some("Creative Commons Attribution-NonCommercial-NoDerivs 4.0 Unported License".into()),
        url: "https://gamebanana.com/mods/609804".into(),
        version: Some("2.4".into()),
        text: "A competitive config that changes ConVars only. Pick the file for the current patch.".into(),
        files: vec![
            file(1712001, "hanturaya-competitive.zip", 1_760_000_000, 42_000, "ConVars only, current patch"),
            file(1712000, "hanturaya-competitive-old.zip", 1_744_000_000, 39_000, "Before the March patch"),
        ],
    }
}

/// A page with the demo files loaded, for the headless scenes.
pub(super) fn demo(id: &str) -> Hosted {
    let mut hosted = Hosted::new(id);
    let page = demo_page();
    hosted.file = page.files.first().map(|file| file.id);
    hosted.page = Some(page);
    hosted
}

/// The same page after a review, with two variants to choose from.
pub(super) fn demo_review(id: &str) -> Hosted {
    let mut hosted = demo(id);
    hosted.prepared = Some(core::sample());
    hosted.variant = 0;
    hosted
}
