//! Settings > Mods > Sources: community mod catalogs beside GameBanana. A source is a
//! `dmm-source` 1.0 descriptor at an https address (the format Deadlock Mod Manager
//! documents), so sources made for that app work in Sigil. Adding one shows its name and
//! every origin it may contact before anything is saved; installs go through the same
//! download, extract and install steps as Browse, with the source's hash checked.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use deadlock_core::mod_sources::{self, Descriptor, Listing, SourceAction, SourceLink};
use rune::{El, Key, KeyEvent, TextEdit, Tx, col, row, spacer, text};

use super::i18n::{fill, tr};
use super::options::{self, Source};
use super::{button, detail, disabled, field, say, section, setting_row, stacked_row, trailing};
use crate::app::State;
use crate::icons;
use crate::jobs::Job;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, MONO, SMALL, Ui};

pub const GUIDE_URL: &str = "https://docs.deadlockmods.app/developer-docs/mod-sources";

/// A fetched catalog, kept for this run.
#[derive(Clone)]
pub struct Synced {
    pub descriptor: Descriptor,
    pub listings: Vec<Listing>,
    pub at: Instant,
}

pub enum Outcome {
    /// A new source, waiting for the user to confirm.
    Preview(Box<Synced>),
    Refreshed(String, Box<Synced>),
}

#[derive(Default)]
pub struct Sources {
    pub edit: TextEdit,
    pub job: Job<Outcome>,
    /// Which source the running job is for.
    pub busy: Option<String>,
    pub synced: BTreeMap<String, Synced>,
    pub errors: BTreeMap<String, String>,
    pub search: TextEdit,
    pub install_job: Job<String>,
    pub installing: Option<String>,
}

/// Valid addresses for a saved source.
pub fn validate_url(url: &str) -> Result<(), String> {
    mod_sources::check_url(url).map_err(|e| e.to_string())
}

pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.settings.extra.sources.job.try_take() {
        let url = st.settings.extra.sources.busy.take().unwrap_or_default();
        match result {
            Ok(Outcome::Preview(synced)) => {
                st.settings.extra.dialog = Some(super::extra::Dialog::SourceAdd(synced));
            }
            Ok(Outcome::Refreshed(url, synced)) => {
                let name = synced.descriptor.name.clone();
                st.settings.extra.sources.errors.remove(&url);
                st.settings.extra.sources.synced.insert(url.clone(), *synced);
                options::edit_quiet(|o| {
                    if let Some(s) = o.sources.iter_mut().find(|s| s.url == url) {
                        s.name = name;
                    }
                });
            }
            Err(error) => {
                if url.is_empty() {
                    say(st, error, true);
                } else {
                    st.settings.extra.sources.errors.insert(url, error);
                }
            }
        }
        changed = true;
    }
    if let Some(result) = st.settings.extra.sources.install_job.try_take() {
        st.settings.extra.sources.installing = None;
        match result {
            Ok(line) => {
                st.status = line;
                crate::data::scan_addons_now(st);
                crate::data::start_addon_scan(st);
            }
            Err(error) => st.status = error,
        }
        changed = true;
    }
    changed
}

pub(crate) fn start_fetch(st: &mut State, url: String, preview: bool) {
    if st.settings.extra.sources.job.is_running() {
        return;
    }
    if st.prefs.offline {
        return say(st, tr("Offline mode is on."), true);
    }
    if let Err(error) = validate_url(&url) {
        return say(st, error, true);
    }
    st.settings.extra.sources.busy = Some(if preview { String::new() } else { url.clone() });
    let wake = st.wake.clone();
    st.settings.extra.sources.job.start(wake, move || {
        let descriptor = mod_sources::fetch_descriptor(&url).map_err(|e| e.to_string())?;
        let listings = mod_sources::fetch_catalog(&descriptor).map_err(|e| e.to_string())?;
        let synced = Box::new(Synced {
            descriptor,
            listings,
            at: Instant::now(),
        });
        Ok(if preview { Outcome::Preview(synced) } else { Outcome::Refreshed(url, synced) })
    });
}

/// Save a previewed source.
pub fn confirm_add(st: &mut State, synced: Synced) {
    let url = synced.descriptor.url.clone();
    let name = synced.descriptor.name.clone();
    options::edit(st, |o| {
        if !o.sources.iter().any(|s| s.url == url) {
            o.sources.push(Source {
                url: url.clone(),
                name: name.clone(),
                enabled: true,
            });
        }
    });
    st.settings.extra.sources.synced.insert(url, synced);
    st.settings.extra.sources.edit = TextEdit::new("");
    st.settings.extra.dialog = None;
    say(st, fill(tr("Added {}."), &[&name]), false);
}

pub fn section_el(s: &State, u: &Ui) -> El {
    let o = options::get();
    let x = &s.settings.extra.sources;
    let mut rows = vec![setting_row(
        u,
        "settings-source-gamebanana",
        "GameBanana",
        detail(u, "Built in. Browse reads it live."),
        rune::el(),
    )];
    for (index, source) in o.sources.iter().enumerate() {
        rows.push(source_row(s, u, index, source));
    }
    let typed = x.edit.text.trim().to_string();
    let busy_adding = x.busy.as_deref() == Some("");
    rows.push(stacked_row(
        u,
        "settings-source-add",
        "Add a source",
        "Paste the address of a source file (dmm-source.json). Sigil shows what it contacts before adding it.",
        row()
            .w_full()
            .gap(8.0)
            .center_y()
            .child(
                field(
                    rune::text_input(&x.edit, MONO)
                        .key("settings-source-input")
                        .label(tr("Source address"))
                        .single_line()
                        .grow(1.0)
                        .color(u.ink)
                        .placeholder("https://example.org/dmm-source.json", u.faint)
                        .caret(u.accent, u.accent.alpha(0.22))
                        .on_edit(|st: &mut State| &mut st.settings.extra.sources.edit),
                    u,
                )
                .grow(1.0)
                .min_w(0.0),
            )
            .child(disabled(
                button(u, "settings-source-add-run", if busy_adding { "Checking" } else { "Add" }, ActionKind::Secondary)
                    .on_press(|st: &mut State| {
                        let url = st.settings.extra.sources.edit.text.trim().to_string();
                        start_fetch(st, url, true)
                    }),
                typed.is_empty() || x.job.is_running() || s.prefs.offline,
            ))
            .child(
                u.icon_button(&icons::EXTERNAL_LINK, tr("How to publish your own source"))
                    .key("settings-source-guide")
                    .cursor(rune::Cursor::Pointer)
                    .on_press_tx(|st: &mut State, tx: &mut Tx| super::tabs::open_url(st, tx, GUIDE_URL)),
            ),
    ));
    section(u, "Sources", rows)
}

fn source_row(s: &State, u: &Ui, index: usize, source: &Source) -> El {
    let x = &s.settings.extra.sources;
    let origin = mod_sources::origin_of(&source.url).unwrap_or_default();
    let refreshing = x.busy.as_deref() == Some(source.url.as_str());
    let line = if !source.enabled {
        fill(tr("{} · Off"), &[&origin])
    } else if refreshing {
        fill(tr("{} · Syncing"), &[&origin])
    } else if let Some(error) = x.errors.get(&source.url) {
        let cached = if x.synced.contains_key(&source.url) { tr(", showing the last list") } else { "" };
        format!("{origin} · {error}{cached}")
    } else if let Some(sync) = x.synced.get(&source.url) {
        fill(
            tr("{} · {} mods, synced {} min ago"),
            &[&origin, &sync.listings.len().to_string(), &(sync.at.elapsed().as_secs() / 60).to_string()],
        )
    } else {
        fill(tr("{} · Not synced"), &[&origin])
    };
    let url = source.url.clone();
    let browse_url = url.clone();
    let remove_url = url.clone();
    let toggle_url = url.clone();
    let error = x.errors.contains_key(&source.url);
    setting_row(
        u,
        &format!("settings-source-{index}"),
        if source.name.is_empty() { &origin } else { &source.name },
        text(line).font(SMALL).color(if error { u.amber } else { u.muted }).truncate(),
        trailing(vec![
            disabled(
                button(u, &format!("settings-source-browse-{index}"), "Browse", ActionKind::Secondary).on_press(
                    move |st: &mut State| {
                        let url = browse_url.clone();
                        if !st.settings.extra.sources.synced.contains_key(&url) {
                            start_fetch(st, url.clone(), false);
                        }
                        st.settings.extra.sources.search = TextEdit::new("");
                        st.settings.extra.dialog = Some(super::extra::Dialog::SourceBrowse(url));
                    },
                ),
                !source.enabled,
            ),
            disabled(
                u.icon_button(&icons::REFRESH_CW, tr("Sync again"))
                    .key(format!("settings-source-refresh-{index}"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| start_fetch(st, url.clone(), false)),
                !source.enabled || x.job.is_running() || s.prefs.offline,
            ),
            u.toggle(source.enabled)
                .key(format!("settings-source-switch-{index}"))
                .label(tr("Use this source"))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    let url = toggle_url.clone();
                    options::edit(st, |o| {
                        if let Some(s) = o.sources.iter_mut().find(|s| s.url == url) {
                            s.enabled = !s.enabled;
                        }
                    })
                }),
            u.icon_button(&icons::TRASH_2, tr("Remove this source"))
                .key(format!("settings-source-remove-{index}"))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    st.settings.extra.dialog = Some(super::extra::Dialog::SourceRemove(remove_url.clone()))
                }),
        ]),
    )
}

pub fn remove(st: &mut State, url: &str) {
    options::edit(st, |o| o.sources.retain(|s| s.url != url));
    st.settings.extra.sources.synced.remove(url);
    st.settings.extra.sources.errors.remove(url);
    st.settings.extra.dialog = None;
    say(st, tr("Removed. Mods you installed from it stay."), false);
}

// --- dialogs --------------------------------------------------------------------------------

pub fn add_dialog(u: &Ui, synced: &Synced) -> El {
    let d = &synced.descriptor;
    let mut origins: Vec<String> = d.api.iter().chain(&d.downloads).chain(&d.media).cloned().collect();
    origins.sort();
    origins.dedup();
    let keep = synced.clone();
    let mut list = col().w_full().gap(4.0);
    for o in origins {
        list = list.child(text(o).font(MONO).color(u.prose).truncate());
    }
    super::extra::shell(u, "settings-source-add-dialog", &fill(tr("Add {}"), &[&d.name]))
        .child(
            text(fill(
                tr("{} mods. Sigil will contact these addresses for this source, and nothing else:"),
                &[&synced.listings.len().to_string()],
            ))
            .font(BODY)
            .color(u.muted)
            .wrap()
            .w_full(),
        )
        .child(list)
        .child(super::extra::buttons(
            u,
            "Add source",
            false,
            move |st: &mut State| confirm_add(st, keep.clone()),
        ))
}

pub fn browse_dialog(s: &State, u: &Ui, url: &str) -> El {
    let x = &s.settings.extra.sources;
    let title = options::get()
        .sources
        .iter()
        .find(|src| src.url == url)
        .map(|src| src.name.clone())
        .or_else(|| x.synced.get(url).map(|sync| sync.descriptor.name.clone()))
        .unwrap_or_else(|| url.to_string());
    let mut body = col().w_full().gap(0.0);
    match x.synced.get(url) {
        None if x.busy.as_deref() == Some(url) => {
            body = body.child(text(tr("Syncing the catalog")).font(BODY).color(u.muted));
        }
        None => {
            let line = x.errors.get(url).cloned().unwrap_or_else(|| tr("Not synced yet.").to_string());
            body = body.child(text(line).font(BODY).color(u.muted).wrap().w_full());
        }
        Some(sync) => {
            let hide_mature = s.prefs.mature == crate::prefs::Mature::Hide;
            let hide_unknown = super::nsfw::hide_unclassified(s.prefs.mature);
            let found: Vec<&Listing> = mod_sources::search(&sync.listings, &x.search.text)
                .into_iter()
                .filter(|l| !(hide_mature && l.nsfw == Some(true)) && !(hide_unknown && l.nsfw.is_none()))
                .take(60)
                .collect();
            if found.is_empty() {
                body = body.child(text(tr("No mods match.")).font(BODY).color(u.muted));
            }
            for (index, listing) in found.into_iter().enumerate() {
                body = body.child(listing_row(s, u, index, url, listing));
            }
        }
    }
    let search = row()
        .w_full()
        .h(32.0)
        .px(10.0)
        .gap(8.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .fill(u.well)
        .border(1.0, u.hair)
        .child(rune::icon(&icons::SEARCH, 14.0).color(u.faint).a11y_hidden())
        .child(
            rune::text_input(&x.search, BODY)
                .key("settings-source-search")
                .label(tr("Search this source"))
                .single_line()
                .grow(1.0)
                .color(u.ink)
                .placeholder(tr("Search this source"), u.faint)
                .caret(u.accent, u.accent.alpha(0.22))
                .on_edit(|st: &mut State| &mut st.settings.extra.sources.search),
        );
    super::extra::shell(u, "settings-source-browse", &title)
        .w(600.0)
        .child(search)
        .child(u.scroll_area(col().w_full().max_h(360.0).scroll(rune::ScrollSpec { stick_bottom: false }).child(body)))
        .child(
            row()
                .w_full()
                .child(spacer())
                .child(button(u, "settings-source-browse-close", "Done", ActionKind::Ghost).on_press(super::extra::close)),
        )
        .on_key(|st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                super::extra::close(st);
                true
            } else {
                false
            }
        })
}

fn listing_row(s: &State, u: &Ui, index: usize, url: &str, l: &Listing) -> El {
    let mut meta = Vec::new();
    if !l.author.is_empty() {
        meta.push(l.author.clone());
    }
    if !l.version.is_empty() {
        meta.push(l.version.clone());
    }
    if l.nsfw == Some(true) {
        meta.push(tr("NSFW").to_string());
    }
    let installing = s.settings.extra.sources.installing.as_deref() == Some(l.id.as_str());
    let busy = s.settings.extra.sources.install_job.is_running();
    let can = !l.artifacts.is_empty();
    let id = l.id.clone();
    let url = url.to_string();
    row()
        .key(format!("settings-source-item-{index}"))
        .w_full()
        .min_h(52.0)
        .py(8.0)
        .gap(12.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(text(l.title.clone()).font(BODY_MED).color(u.ink).truncate())
                .child(text(meta.join(" · ")).font(SMALL).color(u.muted).truncate()),
        )
        .child(disabled(
            button(
                u,
                &format!("settings-source-install-{index}"),
                if installing { "Installing" } else if can { "Install" } else { "No file" },
                ActionKind::Secondary,
            )
            .on_press(move |st: &mut State| install(st, &url, &id)),
            busy || !can || s.game_root.is_none(),
        ))
}

// --- the link confirm ------------------------------------------------------------------------

/// Download, check and install a listing's first file, landing off like Browse installs.
fn install(st: &mut State, url: &str, listing_id: &str) {
    install_artifact(st, url, listing_id, None);
}

/// A `deadlock-mod-manager://sources/...` link: what it asks for, over the source's own
/// address, before anything is added, enabled or installed. The player's press is the
/// approval; a link alone never changes a source or the game folder.
pub fn link_dialog(s: &State, u: &Ui, link: &SourceLink) -> El {
    let origin = mod_sources::origin_of(&link.url).unwrap_or_else(|| link.url.clone());
    let saved = options::get();
    let added = saved.sources.iter().any(|src| src.url == link.url);
    let enabled = saved.sources.iter().any(|src| src.url == link.url && src.enabled);
    let title = match link.action {
        SourceAction::Add => tr("Add this mod source?"),
        SourceAction::Install => tr("Install from this mod source?"),
    };
    let mut body = col().w_full().gap(10.0);
    let mut actions = row()
        .w_full()
        .gap(8.0)
        .pt(8.0)
        .child(spacer())
        .child(
            button(u, "settings-source-link-cancel", "Cancel", ActionKind::Ghost)
                .on_press(super::extra::close),
        );
    if link.action == SourceAction::Add {
        body = body.child(
            text(fill(
                tr("{} asked Sigil to add this source. Nothing is added until you confirm."),
                &[&origin],
            ))
            .font(BODY)
            .color(u.muted)
            .wrap()
            .w_full(),
        );
        body = body.child(address(u, &link.url));
        let add_url = link.url.clone();
        actions = actions.child(
            button(u, "settings-source-link-add", "Add source", ActionKind::Primary)
                .on_press(move |st: &mut State| start_fetch(st, add_url.clone(), true)),
        );
        return super::extra::shell(u, "settings-source-link-dialog", title)
            .w(560.0)
            .child(body)
            .child(actions);
    }
    if !added {
        body = body.child(
            text(tr("This link's source is not added yet. Add it first; nothing is downloaded now."))
                .font(BODY)
                .color(u.muted)
                .wrap()
                .w_full(),
        );
        body = body.child(address(u, &link.url));
        let add_url = link.url.clone();
        actions = actions.child(
            button(u, "settings-source-link-add", "Add source", ActionKind::Primary)
                .on_press(move |st: &mut State| start_fetch(st, add_url.clone(), true)),
        );
    } else if !enabled {
        body = body.child(
            text(tr("This link's source is turned off. Turn it on in Settings, then open the link again."))
                .font(BODY)
                .color(u.muted)
                .wrap()
                .w_full(),
        );
        body = body.child(address(u, &link.url));
        actions = actions.child(
            button(u, "settings-source-link-settings", "Open Settings", ActionKind::Secondary)
                .on_press(|st: &mut State| {
                    super::extra::close(st);
                    super::extra::open_on(st, super::Tab::Mods);
                }),
        );
    } else {
        let x = &s.settings.extra.sources;
        let sync = x.synced.get(&link.url);
        if sync.is_none() && x.busy.as_deref() == Some(link.url.as_str()) {
            body = body.child(
                text(tr("Reading the source.")).font(BODY).color(u.muted).wrap().w_full(),
            );
        } else if sync.is_none() {
            body = body.child(
                text(
                    x.errors
                        .get(&link.url)
                        .map(String::as_str)
                        .unwrap_or(tr("The source has not been read yet.")),
                )
                .font(BODY)
                .color(if x.errors.contains_key(&link.url) { u.amber } else { u.muted })
                .wrap()
                .w_full(),
            );
            body = body.child(address(u, &link.url));
            let read_url = link.url.clone();
            actions = actions.child(
                button(u, "settings-source-link-read", "Read the source", ActionKind::Secondary)
                    .on_press(move |st: &mut State| start_fetch(st, read_url.clone(), false)),
            );
        } else if let Some(sync) = sync {
            body = body.child(address(u, &link.url));
            let Some(listing_id) = link.listing.as_deref() else {
                body = body.child(
                    text(tr("The link names no mod.")).font(BODY).color(u.amber).wrap().w_full(),
                );
                return super::extra::shell(u, "settings-source-link-dialog", title)
                    .w(560.0)
                    .child(body)
                    .child(actions);
            };
            if sync.descriptor.source_id != link.source {
                body = body.child(
                    text(tr("The source at that address is not the one the link names."))
                        .font(BODY)
                        .color(u.amber)
                        .wrap()
                        .w_full(),
                );
            } else if !sync.listings.iter().any(|l| l.id == listing_id) {
                body = body.child(
                    text(tr("That mod is not in this source any more."))
                        .font(BODY)
                        .color(u.amber)
                        .wrap()
                        .w_full(),
                );
            } else {
                let listing = sync.listings.iter().find(|l| l.id == listing_id).unwrap();
                let release_ok = link
                    .release
                    .as_ref()
                    .is_none_or(|release| *release == listing.release_id);
                let artifact = link
                    .artifact
                    .as_ref()
                    .and_then(|id| listing.artifacts.iter().find(|a| a.id == *id));
                let file_ok = match (&link.artifact, listing.artifacts.len()) {
                    (Some(_), _) => artifact.is_some(),
                    (None, 0) => false,
                    (None, 1) => true,
                    (None, _) => false,
                };
                let can_install = release_ok && file_ok;
                body = body.child(
                    text(fill(tr("Install {} from {}?"), &[&listing.title, &origin]))
                        .font(BODY_MED)
                        .color(u.ink)
                        .wrap()
                        .w_full(),
                );
                let mut facts = Vec::new();
                if !listing.author.is_empty() {
                    facts.push(fill(tr("by {}"), &[&listing.author]));
                }
                if !listing.version.is_empty() {
                    facts.push(listing.version.clone());
                }
                if !facts.is_empty() {
                    body = body.child(text(facts.join(" · ")).font(SMALL).color(u.muted).wrap().w_full());
                }
                if !release_ok {
                    body = body.child(
                        text(tr("This link names an older release. Open the mod to choose the current one."))
                            .font(BODY)
                            .color(u.amber)
                            .wrap()
                            .w_full(),
                    );
                } else if let Some(id) = link.artifact.as_ref()
                    && artifact.is_none()
                {
                    body = body.child(
                        text(fill(tr("That file is not in the current release: {}."), &[id]))
                            .font(BODY)
                            .color(u.amber)
                            .wrap()
                            .w_full(),
                    );
                } else if listing.artifacts.is_empty() {
                    body = body.child(
                        text(tr("This release has no file to install.")).font(BODY).color(u.muted).wrap().w_full(),
                    );
                } else if link.artifact.is_none() && listing.artifacts.len() > 1 {
                    body = body.child(
                        text(tr("This release has several files. Choose one on the mod's page."))
                            .font(BODY)
                            .color(u.muted)
                            .wrap()
                            .w_full(),
                    );
                } else if let Some(file) = artifact.or_else(|| listing.artifacts.first()) {
                    let name = if file.filename.is_empty() { file.id.clone() } else { file.filename.clone() };
                    body = body.child(text(name).font(MONO).color(u.prose).wrap().w_full());
                }
                let (open_url, open_listing) = (link.url.clone(), listing.id.clone());
                actions = actions.child(
                    button(u, "settings-source-link-open", "Open the mod", ActionKind::Secondary)
                        .on_press(move |st: &mut State| {
                            crate::screens::browse::sources::pick(st, Some(open_url.clone()));
                            st.browse_view.sources.open = Some(open_listing.clone());
                            st.go(crate::model::Screen::Browse);
                            super::extra::close(st);
                        }),
                );
                let (install_url, install_listing, install_file) =
                    (link.url.clone(), listing.id.clone(), link.artifact.clone());
                actions = actions.child(disabled(
                    button(u, "settings-source-link-install", "Install", ActionKind::Primary)
                        .on_press(move |st: &mut State| {
                            install_artifact(
                                st,
                                &install_url,
                                &install_listing,
                                install_file.as_deref(),
                            );
                            super::extra::close(st);
                        }),
                    !can_install || s.game_root.is_none(),
                ));
            }
        }
    }
    super::extra::shell(u, "settings-source-link-dialog", title)
        .w(560.0)
        .child(body)
        .child(actions)
}

fn address(u: &Ui, url: &str) -> El {
    text(url.to_string()).font(MONO).color(u.prose).wrap().w_full()
}


/// Download, check and install one file of a listing (its first when `artifact_id` is
/// `None`): Browse's file choice for a source mod.
pub(crate) fn install_artifact(st: &mut State, url: &str, listing_id: &str, artifact_id: Option<&str>) {
    if super::watch::refuse_while_running(st) {
        return;
    }
    let Some(sync) = st.settings.extra.sources.synced.get(url).cloned() else { return };
    let Some(listing) = sync.listings.iter().find(|l| l.id == listing_id).cloned() else { return };
    let artifact = match artifact_id {
        Some(id) => listing.artifacts.iter().find(|a| a.id == id).cloned(),
        None => listing.artifacts.first().cloned(),
    };
    let Some(artifact) = artifact else { return };
    if st.demo || rune::is_headless() {
        return say(st, tr("Demo data: nothing was changed."), false);
    }
    let workspace = crate::data::workspace_dir();
    let game_root = st.game_root.clone();
    let land_disabled = st.prefs.land_disabled;
    let library = deadlock_core::import::library_path(&crate::data::data_dir());
    let dir: PathBuf = deadlock_core::install::downloads_dir(&workspace)
        .join(format!("source-{}", mod_sources::source_key(&sync.descriptor)));
    st.settings.extra.sources.installing = Some(listing.id.clone());
    let wake = st.wake.clone();
    st.settings.extra.sources.install_job.start(wake, move || {
        let archive = mod_sources::download_artifact(&sync.descriptor, &artifact, &dir).map_err(|e| e.to_string())?;
        let request = deadlock_core::install::InstallRequest::new(archive.display().to_string(), workspace)
            .map_err(|e| e.to_string())?;
        let plan = deadlock_core::install::plan_install(
            &request,
            &deadlock_core::install::PlanOptions {
                game_root,
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())?;
        let record = deadlock_core::install::apply_install_with(
            &plan,
            true,
            // Written switched off; one that should land on goes on once the safety scan passes.
            deadlock_core::install::ApplyOptions { land_disabled: true },
        )
        .map_err(|e| e.to_string())?;
        let bytes = std::fs::metadata(&record.target).map(|m| m.len()).unwrap_or(0);
        let gated = (!land_disabled)
            .then(|| crate::screens::installed::safety_gate::land_checked(&plan.game_root, plan.slot, &record.target, None))
            .flatten();
        let _ = deadlock_core::import::record_meta(
            &library,
            deadlock_core::import::ModMeta {
                slot: plan.slot,
                size_bytes: bytes,
                name: listing.title.clone(),
                author: (!listing.author.is_empty()).then(|| listing.author.clone()),
                thumbnail_url: listing.image.clone(),
                gamebanana_id: None,
                source: "source".into(),
                recorded_at: String::new(),
            },
        );
        Ok(match (land_disabled, gated) {
            (false, Some(found)) => format!(
                "Installed {} in slot {:02}, switched off: the mod safety check asks about {} files. Turn it on in Installed to review them.",
                listing.title,
                plan.slot,
                found.len()
            ),
            _ => format!("Installed {} in slot {:02}.", listing.title, plan.slot),
        })
    });
}

/// The confirm before removing a source.
pub fn remove_dialog(u: &Ui, url: &str) -> El {
    let url = url.to_string();
    super::extra::shell(u, "settings-source-remove-dialog", tr("Remove this source"))
        .child(
            text(tr("Sigil stops reading it. Mods you installed from it stay installed."))
                .font(BODY)
                .color(u.muted)
                .wrap()
                .w_full(),
        )
        .child(super::extra::buttons(u, "Remove", true, move |st: &mut State| remove(st, &url)))
}

#[allow(dead_code)]
fn heading(u: &Ui, s: &str) -> El {
    text(s.to_string()).font(H1).color(u.ink)
}
