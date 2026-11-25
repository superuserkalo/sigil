//! Custom mod sources inside Browse (Deadlock Mod Manager `ef8af43b`, `d9ba608c`, `becf7cda`,
//! `cfa9554d`, `b79a7485`; behaviour only, never its code or text).
//!
//! - DMM: a source picker in the store header; a source's mods as cards with a detail page
//!   (Markdown description, release history when the source declares it, the multi-file
//!   picker for a release with several files).
//! - Grimoire: no third-party sources.
//! - Sigil: Settings > Mods > Sources adds a `dmm-source` 1.0 source after showing every
//!   origin it contacts (`settings/sources.rs`). Each enabled source is a chip beside
//!   GameBanana in Browse's header. Its page has a search and cards; a card opens the details
//!   with every image, tags, the description (Markdown read as safe text with Sigil's styles,
//!   https links only), the source's release history on demand, and every file of the current
//!   release with its own Install.
//!   Images load only from the source's declared media or api origins. Installs go through the
//!   source install in `settings/sources.rs`: the declared sha256 is checked, the game folder
//!   is written only by that press, and it waits while the game runs.

use std::collections::HashMap;

use deadlock_core::mod_markdown::{self, Block, BlockKind};
use deadlock_core::mod_sources::{self, Descriptor, Listing, Release};
use rune::{Link, Span, rich};

use super::*;
use crate::screens::settings::i18n::{fill, tr};
use crate::screens::settings::options;
use crate::ui::MONO;

/// What Browse keeps for sources between frames.
#[derive(Default)]
pub struct SourceView {
    /// The source Browse shows, by its descriptor address; `None` is GameBanana.
    pub url: Option<String>,
    pub search: TextEdit,
    /// The listing whose details are open.
    pub open: Option<String>,
    /// The image shown large in the details.
    pub image: usize,
    /// Release history by `source url \0 listing id`, read when the details open.
    pub history: HashMap<String, Result<Vec<Release>, String>>,
    pub history_job: Job<(String, Vec<Release>)>,
    history_for: Option<String>,
}

/// The enabled sources, in the order Settings lists them: `(url, name)`.
pub fn enabled(s: &State) -> Vec<(String, String)> {
    let _ = s;
    options::get()
        .sources
        .iter()
        .filter(|src| src.enabled)
        .map(|src| {
            let name = if src.name.is_empty() {
                mod_sources::origin_of(&src.url).unwrap_or_else(|| src.url.clone())
            } else {
                src.name.clone()
            };
            (src.url.clone(), name)
        })
        .collect()
}

fn synced<'a>(s: &'a State, url: &str) -> Option<&'a crate::screens::settings::sources::Synced> {
    s.settings.extra.sources.synced.get(url)
}

/// Show a source's page, syncing it the first time.
pub fn pick(st: &mut State, url: Option<String>) {
    st.browse_view.sources.url = url.clone();
    st.browse_view.sources.open = None;
    st.browse_view.sources.search = TextEdit::new("");
    if let Some(url) = url
        && !st.settings.extra.sources.synced.contains_key(&url)
        && !st.demo
        && !rune::is_headless()
    {
        crate::screens::settings::sources::start_fetch(st, url, false);
    }
}

/// Forget a source Settings removed or turned off; ask for the shown source's images and the
/// open listing's history.
pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(url) = st.browse_view.sources.url.clone()
        && !enabled(st).iter().any(|(u, _)| *u == url)
    {
        st.browse_view.sources = SourceView::default();
        changed = true;
    }
    if let Some(result) = st.browse_view.sources.history_job.try_take() {
        let key = st.browse_view.sources.history_for.take().unwrap_or_default();
        st.browse_view.sources.history.insert(key, result.map(|(_, list)| list));
        changed = true;
    }
    let Some(url) = st.browse_view.sources.url.clone() else { return changed };
    if st.screen != Screen::Browse || st.demo || rune::is_headless() {
        return changed;
    }
    let Some(sync) = synced(st, &url).cloned() else { return changed };
    let open = st.browse_view.sources.open.clone();
    let mut urls: Vec<String> = Vec::new();
    for listing in shown(st, &sync.listings).into_iter().take(60) {
        let wanted: Vec<&String> = if open.as_deref() == Some(listing.id.as_str()) {
            listing.images.iter().collect()
        } else {
            listing.image.iter().collect()
        };
        urls.extend(wanted.into_iter().filter(|u| mod_sources::image_allowed(&sync.descriptor, u)).cloned());
    }
    urls.retain(|u| !st.thumbnails.contains_key(u));
    if !urls.is_empty() {
        crate::data::queue_images(st, urls, false);
    }
    // History is read when the details open, not during a sync (DMM's rule).
    if let Some(id) = open
        && let Some(listing) = sync.listings.iter().find(|l| l.id == id)
        && mod_sources::history_url(&sync.descriptor, listing).is_some()
    {
        let key = format!("{url}\0{id}");
        if !st.browse_view.sources.history.contains_key(&key)
            && !st.browse_view.sources.history_job.is_running()
            && !st.prefs.offline
        {
            st.browse_view.sources.history_for = Some(key);
            let descriptor = sync.descriptor.clone();
            let listing = listing.clone();
            let wake = st.wake.clone();
            st.browse_view.sources.history_job.start(wake, move || {
                mod_sources::fetch_history(&descriptor, &listing)
                    .map(|list| (listing.id.clone(), list))
                    .map_err(|e| e.to_string())
            });
        }
    }
    changed
}

/// The listings the search and the mature setting leave, unknown ratings hidden with mature
/// content as DMM does.
fn shown<'a>(s: &State, listings: &'a [Listing]) -> Vec<&'a Listing> {
    let hide_mature = s.prefs.mature == crate::prefs::Mature::Hide;
    let hide_unknown = crate::screens::settings::nsfw::hide_unclassified(s.prefs.mature);
    mod_sources::search(listings, &s.browse_view.sources.search.text)
        .into_iter()
        .filter(|l| !(hide_mature && l.nsfw == Some(true)) && !(hide_unknown && l.nsfw.is_none()))
        .collect()
}

// --- header ---------------------------------------------------------------------------------

/// GameBanana and each enabled source as one switch in Browse's header. `None` with no source.
pub fn picker(s: &State, u: &Ui) -> Option<El> {
    let sources = enabled(s);
    if sources.is_empty() {
        return None;
    }
    let current = s.browse_view.sources.url.clone();
    let mut r = row()
        .key("browse-store")
        .h(32.0)
        .padding(2.0)
        .gap(2.0)
        .radius(crate::ui::r(8.0))
        .fill(u.well)
        .border(1.0, u.hair)
        .role("radiogroup")
        .label(tr("Mod source"));
    let choices = std::iter::once((None, "GameBanana".to_string()))
        .chain(sources.into_iter().map(|(url, name)| (Some(url), name)));
    for (index, (url, name)) in choices.enumerate() {
        let on = url == current;
        r = r.child(
            row()
                .key(format!("browse-store-{index}"))
                .h_full()
                .px(12.0)
                .max_w(180.0)
                .center()
                .radius(crate::ui::r(6.0))
                .child(crate::ui::seg_thumb(on, "browse-store-thumb", u.raised, u.hair_strong, crate::ui::r(6.0)))
                .hover(if on { u.raised } else { u.hover })
                .focusable(true)
                .role("radio")
                .label(name.clone())
                .checked(on)
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| pick(st, url.clone()))
                .child(text(name).font(LABEL).color(if on { u.ink } else { u.muted }).truncate().a11y_hidden()),
        );
    }
    Some(r)
}

// --- the page -------------------------------------------------------------------------------

/// A source's page in place of the GameBanana grid.
pub fn page(s: &State, u: &Ui, url: &str) -> El {
    let x = &s.settings.extra.sources;
    let name = enabled(s).into_iter().find(|(u, _)| u == url).map(|(_, n)| n).unwrap_or_default();
    let syncing = x.busy.as_deref() == Some(url);
    let body = match synced(s, url) {
        None if syncing => u.empty_state("", tr("Syncing the catalog"), "", None),
        None => u.empty_state(
            "",
            tr("This source did not answer"),
            x.errors.get(url).map(String::as_str).unwrap_or(tr("Not synced yet.")),
            Some(sync_button(s, u, url)),
        ),
        Some(sync) => {
            let found = shown(s, &sync.listings);
            if found.is_empty() {
                u.empty_state("", tr("No mods match."), tr("Try another word."), None)
            } else {
                let width = super::toolbar::page_width(s);
                let (cols, _) = grid_columns(width, s.browse_view.card_size);
                let mut grid = el().key("browse-source-grid").w_full().equal_columns(cols.max(1)).gap(GRID_GAP);
                for (index, listing) in found.iter().take(120).enumerate() {
                    grid = grid.child(card(s, u, &sync.descriptor, url, index, listing));
                }
                col().w_full().gap(12.0).pb(24.0).child(grid).children(
                    (found.len() > 120).then(|| {
                        text(fill(tr("{} more. Search to narrow the list."), &[&(found.len() - 120).to_string()]))
                            .font(SMALL)
                            .color(u.faint)
                    }),
                )
            }
        }
    };
    let count = synced(s, url)
        .map(|sync| fill(tr("{} mods"), &[&shown(s, &sync.listings).len().to_string()]))
        .unwrap_or_default();
    let mut bar = row().w_full().gap(8.0).center_y().child(
        row()
            .grow(1.0)
            .min_w(0.0)
            .h(32.0)
            .px(10.0)
            .gap(8.0)
            .center_y()
            .radius(crate::ui::r(8.0))
            .fill(u.well)
            .border(1.0, u.hair)
            .child(icon(&icons::SEARCH, 14.0).color(u.faint).a11y_hidden())
            .child(
                rune::text_input(&s.browse_view.sources.search, BODY)
                    .key("browse-source-search")
                    .label(tr("Search this source"))
                    .single_line()
                    .grow(1.0)
                    .color(u.ink)
                    .placeholder(fill(tr("Search {}"), &[&name]), u.faint)
                    .caret(u.accent, u.accent.alpha(0.22))
                    .on_edit(|st: &mut State| &mut st.browse_view.sources.search),
            ),
    );
    bar = bar.child(sync_button(s, u, url));
    u.scroll_area(col()
        .key("browse-scroll")
        .w_full()
        .h_full()
        .min_h(0.0)
        .scroll(ScrollSpec { stick_bottom: false })
        .on_layout_tx(|st: &mut State, r: [f32; 4], tx: &mut Tx| {
            let width = (r[2] - 2.0 * PAGE_X).max(0.0);
            if (st.browse_view.width - width).abs() > 0.5 {
                st.browse_view.width = width;
            } else {
                tx.live_only();
            }
        })
        .child(
            col()
                .key("browse-page")
                .w_full()
                .px(PAGE_X)
                .pt(PAGE_TOP)
                .gap(16.0)
                .child(u.page_head("", "Browse", "", Some(head_tools(s, u, &count))))
                .child(bar)
                .child(body),
        ))
}

fn head_tools(s: &State, u: &Ui, count: &str) -> El {
    let mut r = row().gap(12.0).center_y().child(text(count.to_string()).font(SMALL).color(u.faint));
    if let Some(picker) = picker(s, u) {
        r = r.child(picker);
    }
    r
}

fn sync_button(s: &State, u: &Ui, url: &str) -> El {
    let url = url.to_string();
    let busy = s.settings.extra.sources.job.is_running();
    u.icon_button(&icons::REFRESH_CW, tr("Sync again"))
        .key("browse-source-sync")
        .disabled(busy || s.prefs.offline)
        .opacity(if busy || s.prefs.offline { 0.45 } else { 1.0 })
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| crate::screens::settings::sources::start_fetch(st, url.clone(), false))
}

fn card(s: &State, u: &Ui, d: &Descriptor, url: &str, index: usize, l: &Listing) -> El {
    let image = l.image.clone().filter(|i| mod_sources::image_allowed(d, i));
    let mature = l.nsfw == Some(true);
    let blur = mature && s.prefs.mature != crate::prefs::Mature::Show;
    let mut meta = Vec::new();
    if !l.author.is_empty() {
        meta.push(l.author.clone());
    }
    if !l.version.is_empty() {
        meta.push(version_label(&l.version));
    }
    let id = l.id.clone();
    let _ = url;
    col()
        .key(format!("browse-source-card-{index}"))
        .min_w(0.0)
        .radius(crate::ui::r(12.0))
        .clip_rounded(crate::ui::r(12.0))
        .fill(u.well)
        .border(1.0, u.hair)
        .hover(u.hover)
        .focusable(true)
        .role("button")
        .label(l.title.clone())
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            st.browse_view.sources.open = Some(id.clone());
            st.browse_view.sources.image = 0;
        })
        .child(
            el().w_full()
                .h(132.0)
                .fill(u.over.alpha(0.03))
                .child(image_fill(s, format!("browse-source-thumb-{index}"), image.as_ref(), [12.0, 12.0, 0.0, 0.0], blur)),
        )
        .child(
            col()
                .w_full()
                .padding(10.0)
                .gap(2.0)
                .child(text(l.title.clone()).font(BODY_MED).color(u.ink).truncate())
                .child(text(meta.join(" · ")).font(SMALL).color(u.muted).truncate())
                .children(mature.then(|| text(tr("NSFW")).font(SMALL).color(u.amber))),
        )
}

// --- details --------------------------------------------------------------------------------

/// The open listing: images, tags, the description, release history and every file.
pub fn details_dialog(s: &State, u: &Ui) -> Option<El> {
    let view = &s.browse_view.sources;
    let url = view.url.as_deref()?;
    let id = view.open.as_deref()?;
    let sync = synced(s, url)?;
    let l = sync.listings.iter().find(|l| l.id == id)?;
    let d = &sync.descriptor;
    let close = |st: &mut State| st.browse_view.sources.open = None;
    let images: Vec<&String> = l.images.iter().filter(|i| mod_sources::image_allowed(d, i)).collect();
    let pick = view.image.min(images.len().saturating_sub(1));
    let blur = l.nsfw == Some(true) && s.prefs.mature != crate::prefs::Mature::Show;

    let mut c = col().w_full().gap(14.0);
    if let Some(big) = images.get(pick) {
        c = c.child(
            el().w_full()
                .h(280.0)
                .clip_rounded(crate::ui::r(10.0))
                .fill(u.over.alpha(0.03))
                .child(image_fill(s, "browse-source-image".to_string(), Some(big), [10.0; 4], blur)),
        );
        if images.len() > 1 {
            let mut strip = row().w_full().gap(6.0);
            for (index, image) in images.iter().enumerate().take(8) {
                strip = strip.child(
                    el().key(format!("browse-source-image-{index}"))
                        .w(64.0)
                        .h(40.0)
                        .clip_rounded(crate::ui::r(6.0))
                        .border(1.0, if index == pick { u.accent } else { u.hair })
                        .focusable(true)
                        .role("button")
                        .label(fill(tr("Image {}"), &[&(index + 1).to_string()]))
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| st.browse_view.sources.image = index)
                        .child(image_fill(s, format!("browse-source-image-thumb-{index}"), Some(image), [crate::ui::r(6.0); 4], blur)),
                );
            }
            c = c.child(strip);
        }
    }
    let mut facts = Vec::new();
    if !l.author.is_empty() {
        facts.push(fill(tr("by {}"), &[&l.author]));
    }
    if !l.version.is_empty() {
        facts.push(version_label(&l.version));
    }
    if let Some(date) = crate::prefs::show_stamp_date(&l.updated_at) {
        facts.push(fill(tr("Updated {}"), &[&date]));
    }
    c = c.child(text(facts.join(" · ")).font(SMALL).color(u.muted).wrap().w_full());
    if !l.tags.is_empty() {
        let mut chips = row().w_full().gap(6.0).flex_wrap();
        for t in l.tags.iter().take(12) {
            chips = chips.child(tag(u, t));
        }
        c = c.child(chips);
    }
    if let Some(description) = description_blocks(u, l) {
        c = c.child(description);
    }

    // Files: every one of the current release, each with its own Install (DMM's file picker).
    c = c.child(text(tr("Files")).font(BODY_MED).color(u.ink));
    if l.artifacts.is_empty() {
        c = c.child(text(tr("This release has no file to install.")).font(SMALL).color(u.muted));
    }
    let installing = s.settings.extra.sources.installing.as_deref() == Some(l.id.as_str());
    let busy = s.settings.extra.sources.install_job.is_running();
    for (index, a) in l.artifacts.iter().enumerate() {
        let size = a.size.map(crate::model::format_size).unwrap_or_default();
        let name = if a.filename.is_empty() { a.id.clone() } else { a.filename.clone() };
        let (source, listing, artifact) = (url.to_string(), l.id.clone(), a.id.clone());
        let checked = if a.sha256.is_some() { tr("sha256 checked") } else { tr("no checksum") };
        c = c.child(
            row()
                .key(format!("browse-source-file-{index}"))
                .w_full()
                .min_h(48.0)
                .py(6.0)
                .gap(12.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(text(name).font(BODY).color(u.ink).truncate())
                        .child(text([size, checked.to_string()].iter().filter(|p| !p.is_empty()).cloned().collect::<Vec<_>>().join(" · ")).font(SMALL).color(u.faint)),
                )
                .child(
                    u.action(tr(if installing { "Installing" } else { "Install" }), ActionKind::Secondary)
                        .key(format!("browse-source-install-{index}"))
                        .disabled(busy || s.game_root.is_none())
                        .opacity(if busy || s.game_root.is_none() { 0.45 } else { 1.0 })
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| {
                            crate::screens::settings::sources::install_artifact(st, &source, &listing, Some(&artifact))
                        }),
                ),
        );
    }

    // Release history, when the source declares it.
    if mod_sources::history_url(d, l).is_some() {
        c = c.child(text(tr("Release history")).font(BODY_MED).color(u.ink));
        match view.history.get(&format!("{url}\0{}", l.id)) {
            None => c = c.child(text(tr("Reading the release history.")).font(SMALL).color(u.muted)),
            Some(Err(error)) => c = c.child(text(error.clone()).font(SMALL).color(u.amber).wrap().w_full()),
            Some(Ok(list)) if list.is_empty() => c = c.child(text(tr("No releases listed.")).font(SMALL).color(u.muted)),
            Some(Ok(list)) => {
                for (index, release) in list.iter().take(30).enumerate() {
                    let mut head = vec![if release.version.is_empty() { release.id.clone() } else { version_label(&release.version) }];
                    if let Some(date) = crate::prefs::show_stamp_date(&release.published_at) {
                        head.push(date);
                    }
                    if release.id == l.release_id {
                        head.push(tr("current").to_string());
                    }
                    let mut entry = col()
                        .key(format!("browse-source-release-{index}"))
                        .w_full()
                        .gap(2.0)
                        .child(text(head.join(" · ")).font(BODY).color(u.ink));
                    if !release.changelog.is_empty() {
                        entry = entry.child(text(release.changelog.clone()).font(SMALL).color(u.muted).wrap().w_full());
                    }
                    c = c.child(entry);
                }
            }
        }
    }

    let homepage = l.homepage.clone().or_else(|| d.homepage.clone()).filter(|h| h.starts_with("https://"));
    let mut buttons = row().w_full().gap(8.0).center_y();
    if let Some(page) = homepage {
        buttons = buttons.child(
            u.icon_action(&icons::EXTERNAL_LINK, tr("Open the mod's page"), ActionKind::Ghost)
                .key("browse-source-homepage")
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |st: &mut State, tx: &mut Tx| crate::screens::settings::tabs::open_url(st, tx, &page)),
        );
    }
    buttons = buttons.child(spacer()).child(
        u.action(tr("Done"), ActionKind::Secondary)
            .key("browse-source-done")
            .cursor(rune::Cursor::Pointer)
            .on_press(close),
    );
    Some(
        u.dialog_shell()
            .key("browse-source-details")
            .role("dialog")
            .label(l.title.clone())
            .w(640.0)
            .max_h(720.0)
            .padding(24.0)
            .gap(14.0)
            .modal(true, close)
            .on_key(move |st: &mut State, ev: &KeyEvent, _| {
                if ev.key == Key::Escape {
                    close(st);
                    true
                } else {
                    false
                }
            })
            .child(
                col()
                    .w_full()
                    .gap(2.0)
                    .child(text(l.title.clone()).font(H1).color(u.ink).wrap().w_full())
                    .child(text(d.name.clone()).font(SMALL).color(u.faint)),
            )
            .child(u.scroll_area(col().w_full().min_h(0.0).grow(1.0).scroll(ScrollSpec { stick_bottom: false }).child(c)))
            .child(buttons),
    )
}

/// A source description in Sigil's own text styles. A Markdown one is read as blocks with
/// spans and https links; plain text stays one paragraph. No HTML, no image and no other link
/// scheme reaches the UI: the reader drops them before this runs.
fn description_blocks(u: &Ui, l: &Listing) -> Option<El> {
    if !l.markdown {
        let plain = l.description.trim();
        if plain.is_empty() {
            return None;
        }
        return Some(
            text(plain.to_string())
                .font(BODY)
                .color(u.prose)
                .wrap()
                .w_full(),
        );
    }
    let blocks = mod_markdown::parse(&l.description);
    let mut body = col().w_full().gap(8.0);
    let mut any = false;
    for block in blocks.into_iter().take(mod_markdown::MAX_BLOCKS) {
        if block.text.trim().is_empty() {
            continue;
        }
        any = true;
        let font = match block.kind {
            BlockKind::Heading(1 | 2) => H2,
            BlockKind::Heading(_) => BODY_MED,
            BlockKind::Code => MONO,
            _ => BODY,
        };
        let color = match block.kind {
            BlockKind::Quote => u.muted,
            _ => u.prose,
        };
        let mut spans = inline_spans(&block, u);
        for link in &block.links {
            spans.push(
                Span::new(link.start as usize, link.end as usize)
                    .color(u.accent)
                    .underline(u.accent),
            );
        }
        let mut line = rich(block.text.clone(), spans)
            .font(font)
            .color(color)
            .wrap()
            .w_full();
        for span in &block.spans {
            if span.strike {
                line = line.strike(span.start as usize, span.end as usize, None);
            }
        }
        for link in &block.links {
            line = line.link(Link::new(link.start as usize..link.end as usize, link.url.clone()));
        }
        if block.kind == BlockKind::Quote {
            line = line.pl(10.0).border_sides([1.0, 0.0, 0.0, 0.0], u.hair);
        }
        if block.kind == BlockKind::Code {
            line = line
                .padding(10.0)
                .radius(crate::ui::r(8.0))
                .fill(u.well)
                .border(1.0, u.hair);
        }
        if !block.links.is_empty() {
            line = line.on_link(|st: &mut State, url: &str, tx: &mut Tx| {
                if deadlock_core::mod_sources::check_url(url).is_ok() {
                    crate::screens::settings::tabs::open_url(st, tx, url);
                }
            });
        }
        body = body.child(line);
    }
    any.then_some(body)
}

fn inline_spans(block: &Block, u: &Ui) -> Vec<Span> {
    block
        .spans
        .iter()
        .map(|span| {
            let mut out = Span::new(span.start as usize, span.end as usize);
            if span.bold {
                out = out.bold();
            }
            if span.italic {
                out = out.italic();
            }
            if span.code {
                out = out.mono().bg(u.well, 3.0);
            }
            out
        })
        .collect()
}

pub fn stage_scene(st: &mut State, open: bool) {
    use deadlock_core::mod_sources::{Artifact, Catalog};
    use crate::screens::settings::options::Source;
    let url = "https://mods.example.org/dmm-source.json".to_string();
    let descriptor = Descriptor {
        url: url.clone(),
        source_id: "community".into(),
        name: "Community Mods".into(),
        catalog: Catalog::Static { index_url: "https://mods.example.org/catalog.json".into() },
        api: vec!["https://mods.example.org".into()],
        downloads: vec!["https://mods.example.org".into()],
        media: vec![],
        homepage: Some("https://mods.example.org/".into()),
        origin: "https://mods.example.org".into(),
        release_history: true,
        history_template: Some("https://mods.example.org/history/{listingId}".into()),
    };
    let listing = |id: &str, title: &str, author: &str, version: &str| Listing {
        id: id.into(),
        title: title.into(),
        author: author.into(),
        nsfw: Some(false),
        version: version.into(),
        updated_at: "2026-11-18T10:00:00Z".into(),
        tags: vec!["hud".into(), "minimal".into()],
        release_id: format!("{id}-r2"),
        ..Listing::default()
    };
    let mut hud = listing("clean-hud", "Clean HUD", "Kalo", "2.1.0");
    hud.markdown = true;
    hud.description = "## A quieter HUD\n\nRemoves the **glow** around ability icons.\n\n- Smaller minimap\n- No kill feed flash\n\nSee [the guide](https://mods.example.org/guide).".into();
    hud.artifacts = vec![
        Artifact { id: "blue".into(), filename: "clean-hud-blue.zip".into(), size: Some(2_400_000), sha256: Some("ab".into()), ..Artifact::default() },
        Artifact { id: "red".into(), filename: "clean-hud-red.zip".into(), size: Some(2_380_000), ..Artifact::default() },
    ];
    let listings = vec![
        listing("haze-coat", "Haze Night Coat", "someone", "1.0"),
        hud,
        listing("abrams-sounds", "Abrams Voice Pack", "voiceguy", "0.9"),
    ];
    // In memory only: a scene never writes the player's options.
    let mut o = options::get_owned();
    o.sources = vec![Source { url: url.clone(), name: "Community Mods".into(), enabled: true }];
    options::set(o);
    st.settings.extra.sources.synced.insert(
        url.clone(),
        crate::screens::settings::sources::Synced { descriptor, listings, at: std::time::Instant::now() },
    );
    st.browse_view.sources.url = Some(url.clone());
    if open {
        st.browse_view.sources.open = Some("clean-hud".into());
        st.browse_view.sources.history.insert(
            format!("{url}\0clean-hud"),
            Ok(vec![
                Release { id: "clean-hud-r2".into(), version: "2.1.0".into(), published_at: "2026-11-18T10:00:00Z".into(), changelog: "Red variant.".into() },
                Release { id: "clean-hud-r1".into(), version: "2.0.0".into(), published_at: "2026-10-02T09:00:00Z".into(), changelog: "First release.".into() },
            ]),
        );
    }
}
