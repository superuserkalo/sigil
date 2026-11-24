//! The Regions tab's page and its two confirm dialogs. Reads `RegionsState` and the options;
//! every press calls an action in `super`.

use rune::{Color, El, Key, KeyEvent, col, el, icon, row, spacer, text};

use deadlock_core::relays::probe::{self, Health};
use deadlock_core::relays::{CatalogSource, Location, Region};

use super::{Change, Confirm, RegionsState, Sort};
use crate::app::State;
use crate::icons;
use crate::screens::settings::i18n::{fill, tr};
use crate::screens::settings::options;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, H2, MONO, SMALL, SMALL_NUM, Ui};

/// The page width, so lines stay readable on a wide window.
const MAX_W: f32 = 720.0;
const PING_W: f32 = 72.0;
const STATE_W: f32 = 110.0;

pub fn view(s: &State, u: &Ui) -> El {
    let regions = &s.servers.regions;
    let mut page = col().key("regions-page").w_full().max_w(MAX_W).gap(20.0);
    page = page.child(intro(u));
    if !regions.windows {
        page = page.child(not_windows(u));
    } else {
        page = page.child(status(s, u));
    }
    if let Some(error) = &regions.error {
        page = page.child(text(error.clone()).font(SMALL).color(u.rose).wrap().w_full());
    }
    let Some(catalog) = &regions.catalog else {
        let body = match &regions.catalog_error {
            Some(error) => u.empty_state(
                "",
                tr("Valve's relay list could not be loaded"),
                error,
                Some(
                    u.action(tr("Try again"), ActionKind::Secondary)
                        .key("regions-retry")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(super::refresh),
                ),
            ),
            None => u.empty_state("", tr("Loading Valve's relay locations"), "", None),
        };
        return page.child(body);
    };
    if regions.windows {
        page = page.child(action_bar(s, u));
    }
    page = page.child(toolbar(s, u));
    if regions.windows {
        page = page.child(presets(s, u));
    }
    page = page.child(list_bar(s, u));
    let shown = regions.visible();
    if shown.is_empty() && !catalog.locations.is_empty() {
        return page.child(u.empty_state(
            "",
            &fill(tr("No location matches {}"), &[regions.search.text.trim()]),
            tr("Search by city, country or the location's short id."),
            Some(
                u.action(tr("Clear search"), ActionKind::Secondary)
                    .key("regions-search-clear")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| st.servers.regions.search = rune::TextEdit::new("")),
            ),
        ));
    }
    for (region, locations) in shown {
        page = page.child(group(s, u, region, &locations));
    }
    page
}

/// Search, sort, the shift-press hint and the status bar switch above the list.
fn list_bar(s: &State, u: &Ui) -> El {
    let regions = &s.servers.regions;
    let search = rune::text_input(&regions.search, SMALL)
        .key("regions-search")
        .label(tr("Search locations"))
        .single_line()
        .color(u.ink)
        .placeholder(tr("Search locations").to_string(), u.faint)
        .caret(u.accent, u.accent.alpha(0.20))
        .on_edit(|st: &mut State| &mut st.servers.regions.search)
        .on_input_key(|st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                st.servers.regions.search = rune::TextEdit::new("");
                true
            } else {
                false
            }
        });
    let mut sorts = vec![(Sort::Name, tr("Name")), (Sort::Ping, tr("Ping"))];
    if regions.history.as_ref().is_some_and(|h| !h.is_empty()) {
        sorts.push((Sort::Matches, tr("My matches")));
    }
    let items = sorts
        .into_iter()
        .map(|(sort, label)| {
            crate::screens::subpage::segment(u, label, regions.sort == sort)
                .key(format!("regions-sort-{sort:?}"))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| super::set_sort(st, sort))
        })
        .collect();
    let on = options::get().relay_regions.health_bar;
    let mut bar = row()
        .key("regions-list-bar")
        .w_full()
        .gap(10.0)
        .center_y()
        .flex_wrap()
        .child(u.search_shell(search))
        .child(text(tr("Sort")).font(SMALL).color(u.faint))
        .child(crate::screens::subpage::segmented(u, items))
        .child(spacer());
    if regions.windows {
        bar = bar.child(text(tr("Shift-click ticks a range")).font(SMALL_NUM).color(u.faint));
    }
    bar.child(
        row()
            .key("regions-health-bar")
            .gap(8.0)
            .center_y()
            .focusable(true)
            .role("switch")
            .checked(on)
            .label(tr("Relay health in the status bar"))
            .cursor(rune::Cursor::Pointer)
            .on_press(super::toggle_health_bar)
            .child(u.toggle_mark(on))
            .child(text(tr("Relay health in the status bar")).font(SMALL).color(u.muted).a11y_hidden()),
    )
}

// ---------------------------------------------------------------------------------------------
// Header
// ---------------------------------------------------------------------------------------------

fn intro(u: &Ui) -> El {
    let mut downsides = col().w_full().gap(4.0);
    for line in [
        tr("Queues can take longer: fewer regions means fewer players to match with."),
        tr("A block you forget about looks like broken matchmaking. The status bar shows it on every page while one is on."),
        tr("It steers matchmaking; it does not pick a server. Only Deadlock is affected, not other games."),
    ] {
        downsides = downsides.child(
            row()
                .w_full()
                .gap(8.0)
                .items_start()
                .child(el().size(4.0, 4.0).mt(8.0).fill(u.muted).a11y_hidden())
                .child(text(line).font(SMALL).color(u.prose).grow(1.0).min_w(0.0).wrap()),
        );
    }
    col()
        .w_full()
        .gap(10.0)
        .child(
            text(tr("Deadlock reaches its matches through Valve's relay network. Tick the locations you do not want, press Apply, and Sigil blocks their relays in Windows Firewall so matchmaking places you in the open regions. Use it to avoid far regions, language barriers, or to agree on a region with friends."))
                .font(BODY)
                .color(u.prose)
                .wrap(),
        )
        .child(
            // A quiet note, not an alarm: hairline box, the amber only in the icon and a 2 point
            // bar on its left edge.
            row()
                .key("regions-downsides")
                .w_full()
                .radius(crate::ui::r(4.0))
                .clip_rounded(crate::ui::r(4.0))
                .border(1.0, u.hair)
                .child(el().w(2.0).self_stretch().fill(u.amber).a11y_hidden())
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(6.0)
                        .px(12.0)
                        .py(10.0)
                        .child(
                            row()
                                .gap(8.0)
                                .center_y()
                                .child(icon(&icons::TRIANGLE_ALERT, 14.0).color(u.amber).a11y_hidden())
                                .child(text(tr("Before you block a region")).font(BODY_MED).color(u.ink)),
                        )
                        .child(downsides.pl(22.0)),
                ),
        )
}

fn not_windows(u: &Ui) -> El {
    row()
        .key("regions-not-windows")
        .w_full()
        .gap(10.0)
        .items_start()
        .padding(14.0)
        .radius(crate::ui::r(4.0))
        .fill(u.well)
        .border(1.0, u.hair)
        .child(icon(&icons::INFO, 16.0).color(u.muted).mt(2.0).a11y_hidden())
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(4.0)
                .child(text(tr("Blocking needs Windows")).font(BODY_MED).color(u.ink))
                .child(
                    text(tr("Sigil blocks relays with Windows Firewall rules for deadlock.exe, so it can only block on Windows. The list and the pings work here, to see which regions are close."))
                        .font(SMALL)
                        .color(u.muted)
                        .wrap(),
                ),
        )
}

/// What Windows Firewall holds now, with Reset.
fn status(s: &State, u: &Ui) -> El {
    let regions = &s.servers.regions;
    let saved = options::get().relay_regions.clone();
    let cover = regions.coverage();
    let names = |ids: &[String]| -> String {
        ids.iter()
            .map(|id| {
                regions
                    .catalog
                    .as_ref()
                    .and_then(|c| c.location(id))
                    .map_or_else(|| id.clone(), |l| l.city.clone())
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let (tone, headline, detail) = match (&regions.installed, &cover, regions.change) {
        (_, _, Some(Change::Apply | Change::Reset)) => (
            u.accent,
            tr("Waiting for Windows").to_string(),
            tr("Press Yes in Windows' admin prompt to change the firewall. No leaves it as it is.").to_string(),
        ),
        (None, _, _) if regions.read_job.is_running() => (
            u.faint,
            tr("Reading Windows Firewall").to_string(),
            String::new(),
        ),
        (None, _, _) => (u.faint, tr("Firewall not checked yet").to_string(), String::new()),
        (Some(Err(error)), _, _) => (
            u.amber,
            tr("Windows Firewall could not be read").to_string(),
            error.clone(),
        ),
        (Some(Ok(_)), Some(cover), _) if !cover.any() => (
            u.teal,
            tr("Default routing: every region is open").to_string(),
            tr("Sigil has no relay rules in Windows Firewall.").to_string(),
        ),
        (Some(Ok(_)), Some(cover), _) => {
            let count = cover.blocked.len() + cover.partial.len() + cover.unknown.len();
            (
                u.amber,
                fill(tr("{} relay locations blocked"), &[&count.to_string()]),
                names(&cover.blocked.iter().chain(&cover.partial).chain(&cover.unknown).cloned().collect::<Vec<_>>()),
            )
        }
        (Some(Ok(_)), None, _) => (u.faint, String::new(), String::new()),
    };
    let mut warnings = Vec::new();
    if regions.firewall_off() {
        warnings.push(tr("Windows Firewall is off for this network, so the rules do nothing.").to_string());
    }
    if let Some(cover) = &cover {
        if !cover.partial.is_empty() {
            warnings.push(fill(
                tr("Only some relays are blocked for {}: Valve changed its list or a rule was removed. Press Apply again or Reset."),
                &[&names(&cover.partial)],
            ));
        }
        if !cover.unknown.is_empty() {
            warnings.push(fill(
                tr("Rules for locations Valve no longer lists: {}. Apply or Reset removes them."),
                &[&cover.unknown.join(", ")],
            ));
        }
        if !cover.disabled.is_empty() {
            warnings.push(fill(
                tr("Some Sigil rules are turned off in Windows Firewall: {}. Apply turns them on again."),
                &[&names(&cover.disabled)],
            ));
        }
    }
    if saved.pending && regions.change.is_none() {
        warnings.push(tr("The last firewall change did not report back. What is shown is what Windows holds now.").to_string());
    }
    let mut text_col = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(2.0)
        .child(text(headline).font(BODY_MED).color(u.ink).wrap());
    if !detail.is_empty() {
        text_col = text_col.child(text(detail).font(SMALL).color(u.muted).wrap().w_full());
    }
    for line in warnings {
        text_col = text_col.child(
            row()
                .w_full()
                .gap(6.0)
                .items_start()
                .pt(4.0)
                .child(icon(&icons::TRIANGLE_ALERT, 13.0).color(u.amber).mt(2.0).a11y_hidden())
                .child(text(line).font(SMALL).color(u.prose).grow(1.0).min_w(0.0).wrap()),
        );
    }
    let mut card = row()
        .key("regions-status")
        .role("status")
        .w_full()
        .gap(12.0)
        .items_start()
        .padding(14.0)
        .radius(crate::ui::r(4.0))
        .fill(u.well)
        .border(1.0, u.hair)
        .child(el().size(8.0, 8.0).mt(6.0).radius(4.0).fill(tone).a11y_hidden())
        .child(text_col);
    if cover.as_ref().is_some_and(|c| c.any()) {
        card = card.child(
            u.icon_action(&icons::ROTATE_CCW, tr("Reset"), ActionKind::Secondary)
                .key("regions-reset")
                .opacity(if regions.busy() { 0.5 } else { 1.0 })
                .cursor(rune::Cursor::Pointer)
                .on_press(super::ask_reset),
        );
    }
    card
}

// ---------------------------------------------------------------------------------------------
// Toolbar and presets
// ---------------------------------------------------------------------------------------------

fn toolbar(s: &State, u: &Ui) -> El {
    let regions = &s.servers.regions;
    let measuring = regions.probe_job.is_running();
    let total = regions.catalog.as_ref().map_or(0, |c| c.locations.len());
    let mut line = match (measuring, regions.probed_at) {
        (true, _) => fill(tr("Measuring {} of {}"), &[&regions.pings.len().to_string(), &total.to_string()]),
        (false, Some(at)) => fill(tr("Pings measured {}"), &[&crate::statusbar::ago(super::now() - at)]),
        (false, None) => tr("Pings not measured yet").to_string(),
    };
    if let Some((location, ms)) = fastest_open(regions) {
        let gap = if line.ends_with('.') { " " } else { ". " };
        line = format!("{line}{gap}{}", fill(tr("Fastest open: {} ({} ms)"), &[&location, &ms.to_string()]));
    }
    let source = match &regions.source {
        Some(CatalogSource::Stale(_)) => tr("Valve could not be reached: showing the saved list.").to_string(),
        _ => regions
            .catalog
            .as_ref()
            .map(|c| fill(tr("Valve's list from {}"), &[&crate::statusbar::ago(super::now() - c.fetched_at)]))
            .unwrap_or_default(),
    };
    row()
        .key("regions-toolbar")
        .w_full()
        .gap(10.0)
        .center_y()
        .child(
            u.icon_action(
                if measuring { &icons::SQUARE_STOP } else { &icons::GAUGE },
                if measuring { tr("Stop") } else { tr("Measure ping") },
                ActionKind::Secondary,
            )
            .key("regions-probe")
            .cursor(rune::Cursor::Pointer)
            .on_press(super::toggle_probe),
        )
        .child(text(line).font(SMALL).color(u.muted).grow(1.0).min_w(0.0).truncate())
        .child(text(source).font(SMALL_NUM).color(u.faint))
        .child(
            u.icon_button(&icons::REFRESH_CW, tr("Refresh the list"))
                .key("regions-refresh")
                .cursor(rune::Cursor::Pointer)
                .on_press(super::refresh),
        )
}

/// The lowest ping among the locations the selection leaves open.
fn fastest_open(regions: &RegionsState) -> Option<(String, u32)> {
    let catalog = regions.catalog.as_ref()?;
    catalog
        .locations
        .iter()
        .filter(|l| !regions.draft.contains(&l.id))
        .filter_map(|l| Some((l.city.clone(), (*regions.pings.get(&l.id)?)?)))
        .min_by_key(|(_, ms)| *ms)
}

fn presets(s: &State, u: &Ui) -> El {
    let regions = &s.servers.regions;
    let saved = options::get().relay_regions.presets.clone();
    let mut chips = row().gap(6.0).center_y().flex_wrap();
    for preset in &saved {
        let name = preset.name.clone();
        let remove = preset.name.clone();
        let current = preset.blocked.iter().cloned().collect::<std::collections::BTreeSet<_>>() == regions.draft;
        chips = chips.child(
            row()
                .key(format!("regions-preset-{}", preset.name))
                .h(28.0)
                .pl(10.0)
                .pr(2.0)
                .gap(4.0)
                .center_y()
                .radius(crate::ui::r(14.0))
                .fill(if current { u.accent.alpha(0.12) } else { u.over.alpha(0.035) })
                .border(1.0, if current { u.accent.alpha(0.45) } else { u.hair_strong })
                .child(
                    row()
                        .gap(6.0)
                        .center_y()
                        .focusable(true)
                        .role("button")
                        .label(fill(tr("Load preset {}"), &[&preset.name]))
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| super::load_preset(st, &name))
                        .child(text(preset.name.clone()).font(SMALL).color(u.ink).a11y_hidden())
                        .child(
                            text(fill(tr("{} blocked"), &[&preset.blocked.len().to_string()]))
                                .font(SMALL_NUM)
                                .color(u.faint)
                                .a11y_hidden(),
                        ),
                )
                .child(
                    el().size(22.0, 22.0)
                        .center()
                        .radius(11.0)
                        .hover(u.hover)
                        .focusable(true)
                        .role("button")
                        .label(fill(tr("Delete preset {}"), &[&preset.name]))
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| super::delete_preset(st, &remove))
                        .child(icon(&icons::X, 12.0).color(u.faint).a11y_hidden()),
                ),
        );
    }
    let trailing = if regions.naming {
        row()
            .gap(6.0)
            .center_y()
            .child(
                row()
                    .w(200.0)
                    .h(28.0)
                    .px(10.0)
                    .center_y()
                    .radius(crate::ui::r(8.0))
                    .fill(u.well)
                    .border(1.0, u.hair)
                    .child(
                        rune::text_input(&regions.preset_name, SMALL)
                            .key("regions-preset-name")
                            .label(tr("Preset name"))
                            .single_line()
                            .color(u.ink)
                            .placeholder(tr("Preset name").to_string(), u.faint)
                            .caret(u.accent, u.accent.alpha(0.20))
                            .on_edit(|st: &mut State| &mut st.servers.regions.preset_name)
                            .on_submit(|st: &mut State, _: &mut rune::Tx| super::save_preset(st))
                            .grow(1.0)
                            .min_w(0.0),
                    ),
            )
            .child(
                u.action(tr("Save"), ActionKind::Secondary)
                    .key("regions-preset-save")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(super::save_preset),
            )
            .child(
                u.action(tr("Cancel"), ActionKind::Ghost)
                    .key("regions-preset-cancel")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| st.servers.regions.naming = false),
            )
    } else {
        row().child(
            u.icon_action(&icons::PLUS, tr("Save as preset"), ActionKind::Ghost)
                .key("regions-preset-new")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| st.servers.regions.naming = true),
        )
    };
    if let Some(played) = regions.where_i_play().filter(|_| s.prefs.allow_stats) {
        let current = played.iter().cloned().collect::<std::collections::BTreeSet<_>>() == regions.draft;
        let open = regions.catalog.as_ref().map_or(0, |c| c.locations.len()).saturating_sub(played.len());
        chips = row().gap(6.0).center_y().flex_wrap().child(
            row()
                .key("regions-preset-played")
                .h(28.0)
                .px(10.0)
                .gap(6.0)
                .center_y()
                .radius(crate::ui::r(14.0))
                .fill(if current { u.accent.alpha(0.12) } else { u.over.alpha(0.035) })
                .border(1.0, if current { u.accent.alpha(0.45) } else { u.hair_strong })
                .focusable(true)
                .role("button")
                .label(fill(
                    tr("Where I play: block every location you had no match on in the last {} months"),
                    &[&deadlock_core::relays::history::RECENT_MONTHS.to_string()],
                ))
                .cursor(rune::Cursor::Pointer)
                .on_press(super::load_where_i_play)
                .child(icon(&icons::PIN, 12.0).color(u.accent).a11y_hidden())
                .child(text(tr("Where I play")).font(SMALL).color(u.ink).a11y_hidden())
                .child(text(fill(tr("{} open"), &[&open.to_string()])).font(SMALL_NUM).color(u.faint).a11y_hidden()),
        )
        .child(chips);
    }
    let mut body = row().key("regions-presets").w_full().gap(10.0).center_y().child(text(tr("Presets")).font(BODY_MED).color(u.ink));
    if saved.is_empty() && !regions.naming && (regions.where_i_play().is_none() || !s.prefs.allow_stats) {
        body = body.child(
            text(tr("Save a selection to switch to it in one press."))
                .font(SMALL)
                .color(u.faint),
        );
    }
    body.child(chips).child(spacer()).child(trailing)
}

// ---------------------------------------------------------------------------------------------
// The list
// ---------------------------------------------------------------------------------------------

fn group(s: &State, u: &Ui, region: Region, locations: &[&Location]) -> El {
    let regions = &s.servers.regions;
    // Counted over the whole region, as Block all and Allow all act on it, not the search.
    let all: Vec<&Location> = regions
        .catalog
        .as_ref()
        .map(|c| c.locations.iter().filter(|l| l.region == region).collect())
        .unwrap_or_default();
    let total = all.len();
    let blocked = all.iter().filter(|l| regions.draft.contains(&l.id)).count();
    let count = match blocked {
        0 => fill(tr("All {} open"), &[&total.to_string()]),
        n if n == total => fill(tr("All {} blocked"), &[&total.to_string()]),
        n => fill(tr("{} of {} blocked"), &[&n.to_string(), &total.to_string()]),
    };
    let mut head = row()
        .key(format!("regions-group-{region:?}"))
        .w_full()
        .h(32.0)
        .gap(10.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(text(tr(region.label())).font(H2).color(u.ink))
        .child(text(count).font(SMALL).color(if blocked > 0 { u.amber } else { u.faint }))
        .child(spacer());
    if regions.windows {
        head = head
            .child(
                u.action(tr("Block all"), ActionKind::Ghost)
                    .key(format!("regions-block-{region:?}"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| super::set_region(st, region, true)),
            )
            .child(
                u.action(tr("Allow all"), ActionKind::Ghost)
                    .key(format!("regions-allow-{region:?}"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| super::set_region(st, region, false)),
            );
    }
    let mut list = col().w_full();
    for location in locations {
        list = list.child(location_row(s, u, location));
    }
    col().w_full().gap(2.0).child(head).child(list)
}

fn checkbox(u: &Ui, on: bool, disabled: bool) -> El {
    let base = el().size(16.0, 16.0).radius(crate::ui::r(4.0)).center().a11y_hidden();
    if disabled {
        base.border(1.0, u.hair)
    } else if on {
        base.fill(u.rose).child(icon(&icons::X, 12.0).color(Color::WHITE).a11y_hidden())
    } else {
        base.border(1.0, u.hair_strong)
    }
}

fn ping_color(u: &Ui, health: Health) -> Color {
    match health {
        Health::Good => u.teal,
        Health::Fair => u.amber,
        Health::Poor => u.rose,
        Health::NoAnswer => u.faint,
    }
}

fn location_row(s: &State, u: &Ui, location: &Location) -> El {
    let regions = &s.servers.regions;
    let ticked = regions.draft.contains(&location.id);
    let now = regions.blocked_now();
    let partial = regions.coverage().is_some_and(|c| c.partial.contains(&location.id));
    let was = now.contains(&location.id);
    let (state, state_color) = match (was, ticked) {
        (true, true) if partial => (tr("Partly blocked"), u.amber),
        (true, true) => (tr("Blocked"), u.rose),
        (false, true) => (tr("Will block"), u.amber),
        (true, false) => (tr("Will open"), u.teal),
        (false, false) => ("", u.faint),
    };
    let (ping, ping_tone) = match regions.pings.get(&location.id) {
        Some(Some(ms)) => (fill(tr("{} ms"), &[&ms.to_string()]), ping_color(u, probe::health(Some(*ms)))),
        Some(None) => (tr("No answer").to_string(), u.faint),
        None if regions.probe_job.is_running() => ("...".to_string(), u.faint),
        None => (String::new(), u.faint),
    };
    let id = location.id.clone();
    let mut r = row()
        .key(format!("regions-row-{}", location.id))
        .w_full()
        .h(32.0)
        .px(8.0)
        .gap(10.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair.alpha(0.5))
        .child(checkbox(u, ticked, !regions.windows))
        .child(
            text(location.city.clone())
                .font(BODY)
                .color(if ticked { u.muted } else { u.ink })
                .a11y_hidden(),
        )
        .child(
            text(location.country.clone().unwrap_or_default())
                .font(SMALL)
                .color(u.faint)
                .grow(1.0)
                .min_w(0.0)
                .truncate()
                .a11y_hidden(),
        )
        .child(match regions.history.as_ref().and_then(|h| h.get(&location.id)) {
            Some(n) if *n > 0 => text(if *n == 1 {
                tr("1 match").to_string()
            } else {
                fill(tr("{} matches"), &[&n.to_string()])
            })
            .key(format!("regions-played-{}", location.id))
            .font(SMALL_NUM)
            .color(u.accent)
            .a11y_hidden(),
            _ => el(),
        })
        .child(text(location.id.clone()).font(MONO).color(u.faint).a11y_hidden())
        .child(
            row()
                .w(STATE_W)
                .child(text(state).font(SMALL).color(state_color).a11y_hidden()),
        )
        .child(
            row()
                .w(PING_W)
                .gap(6.0)
                .center_y()
                .child(spacer())
                .child(text(ping.clone()).font(SMALL_NUM).color(ping_tone).a11y_hidden())
                .child(if ping.is_empty() || ping == "..." {
                    el().size(6.0, 6.0)
                } else {
                    el().size(6.0, 6.0).radius(3.0).fill(ping_tone).a11y_hidden()
                }),
        );
    let mut label = match regions.pings.get(&location.id) {
        Some(Some(ms)) => format!("{}, {} ({} ms)", location.city, location.country.clone().unwrap_or_default(), ms),
        _ => format!("{}, {}", location.city, location.country.clone().unwrap_or_default()),
    };
    if let Some(n) = regions.history.as_ref().and_then(|h| h.get(&location.id)).filter(|n| **n > 0) {
        label = format!("{label}, {}", fill(tr("{} matches"), &[&n.to_string()]));
    }
    if regions.windows {
        r = r
            .focusable(true)
            .role("checkbox")
            .checked(ticked)
            .label(fill(tr("Block {}"), &[&label]))
            .hover(u.hover)
            .cursor(rune::Cursor::Pointer)
            .on_press_tx(move |st: &mut State, tx: &mut rune::Tx| super::pick(st, &id, tx.mods().shift));
    } else {
        r = r.label(label);
    }
    r
}

fn action_bar(s: &State, u: &Ui) -> El {
    let regions = &s.servers.regions;
    let changes = regions.changes();
    let all = regions.all_blocked();
    let repair = regions.needs_repair();
    let can_apply = (changes > 0 || repair) && !all && !regions.busy();
    let line = if all {
        tr("Leave at least one location open, or Deadlock cannot find a match.").to_string()
    } else if regions.change_job.is_running() {
        tr("Waiting for Windows' admin prompt...").to_string()
    } else if changes == 0 && repair {
        tr("Sigil's rules do not match your ticks. Press Apply to set them right.").to_string()
    } else if changes == 0 {
        tr("No changes. Tick a location to block it.").to_string()
    } else {
        fill(tr("{} changes not applied yet"), &[&changes.to_string()])
    };
    // One fixed 48 point bar. Discard keeps its place while there is nothing to discard, so
    // the line and Apply never move when the first location is ticked.
    let line_color = if all {
        u.rose
    } else if changes > 0 {
        u.ink
    } else {
        u.muted
    };
    row()
        .key("regions-actions")
        .w_full()
        .h(48.0)
        .gap(8.0)
        .center_y()
        .px(12.0)
        .radius(crate::ui::r(4.0))
        .fill(u.well)
        .border(1.0, if all { u.rose.alpha(0.4) } else { u.hair })
        .child(text(line.clone()).font(BODY).color(line_color).grow(1.0).min_w(0.0).truncate().help_when_truncated(line.clone()))
        .child(
            crate::motion::dims(
                u.action_m(tr("Discard"), ActionKind::Secondary)
                    .key("regions-discard")
                    .disabled(changes == 0)
                    .opacity(if changes > 0 { 1.0 } else { 0.0 })
                    .cursor(rune::Cursor::Pointer)
                    .on_press(super::discard),
            ),
        )
        .child(
            u.icon_action_m(&icons::SHIELD, tr("Apply"), ActionKind::Primary)
                .key("regions-apply")
                .disabled(!can_apply)
                .opacity(if can_apply { 1.0 } else { 0.45 })
                .cursor(if can_apply { rune::Cursor::Pointer } else { rune::Cursor::Default })
                .on_press(super::ask_apply),
        )
}

// ---------------------------------------------------------------------------------------------
// Dialogs
// ---------------------------------------------------------------------------------------------

fn close(st: &mut State) {
    st.servers.regions.confirm = None;
}

fn shell(u: &Ui, key: &str, title: &str, body: El) -> El {
    u.dialog_shell()
        .key(key.to_string())
        .role("dialog")
        .label(title.to_string())
        .w(500.0)
        .padding(24.0)
        .gap(12.0)
        .modal(true, close)
        .on_key(|st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                close(st);
                true
            } else {
                false
            }
        })
        .child(text(title.to_string()).font(H1).color(u.ink).wrap())
        .child(body)
}

fn buttons(u: &Ui, confirm: El) -> El {
    row()
        .w_full()
        .gap(8.0)
        .pt(8.0)
        .child(spacer())
        .child(
            u.action(tr("Cancel"), ActionKind::Ghost)
                .key("regions-dialog-cancel")
                .cursor(rune::Cursor::Pointer)
                .on_press(close),
        )
        .child(confirm)
}

fn prompt_note(u: &Ui, line: &str) -> El {
    row()
        .w_full()
        .gap(10.0)
        .items_start()
        .padding(12.0)
        .radius(crate::ui::r(8.0))
        .fill(u.well)
        .child(icon(&icons::SHIELD, 16.0).color(u.amber).mt(2.0).a11y_hidden())
        .child(text(line.to_string()).font(BODY).color(u.ink).grow(1.0).min_w(0.0).wrap())
}

/// The open confirm dialog on the Regions tab, if any.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    match s.servers.regions.confirm? {
        Confirm::Apply => Some(apply_dialog(s, u)),
        Confirm::Reset => Some(reset_dialog(s, u)),
    }
}

fn city_list(s: &State, ids: &[&String]) -> String {
    let catalog = s.servers.regions.catalog.as_ref();
    ids.iter()
        .map(|id| catalog.and_then(|c| c.location(id)).map_or_else(|| id.to_string(), |l| l.city.clone()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn apply_dialog(s: &State, u: &Ui) -> El {
    let regions = &s.servers.regions;
    let now = regions.blocked_now();
    let block: Vec<&String> = regions.draft.iter().filter(|id| !now.contains(*id)).collect();
    let open: Vec<&String> = now.iter().filter(|id| !regions.draft.contains(*id)).collect();
    let mut body = col().w_full().gap(10.0);
    if regions.draft.is_empty() {
        body = body.child(
            text(tr("Sigil removes its relay rules, so every region is open again."))
                .font(BODY)
                .color(u.prose)
                .wrap(),
        );
    } else {
        body = body.child(
            text(fill(
                tr("Sigil adds Windows Firewall rules that stop deadlock.exe from reaching the relays of {} locations. Nothing else changes, and other programs are not affected."),
                &[&regions.draft.len().to_string()],
            ))
            .font(BODY)
            .color(u.prose)
            .wrap(),
        );
    }
    if !block.is_empty() {
        body = body.child(text(fill(tr("Blocks: {}"), &[&city_list(s, &block)])).font(SMALL).color(u.ink).wrap().w_full());
    }
    if !open.is_empty() {
        body = body.child(text(fill(tr("Opens again: {}"), &[&city_list(s, &open)])).font(SMALL).color(u.ink).wrap().w_full());
    }
    if !regions.draft.is_empty() {
        body = body.child(
            text(tr("Queues can take longer, and the block stays after Sigil closes until you press Reset. Restart Deadlock if it is running."))
                .font(SMALL)
                .color(u.muted)
                .wrap(),
        );
    }
    body = body
        .child(prompt_note(u, tr("Windows shows its admin prompt next. Press Yes there to change the rules; No leaves the firewall as it is.")))
        .child(buttons(
            u,
            u.icon_action(&icons::SHIELD, tr("Apply"), ActionKind::Primary)
                .key("regions-apply-confirm")
                .cursor(rune::Cursor::Pointer)
                .on_press(super::apply),
        ));
    shell(u, "regions-apply-dialog", tr("Block relay regions"), body)
}

fn reset_dialog(s: &State, u: &Ui) -> El {
    let rules = s.servers.regions.coverage().map_or(0, |c| c.rules);
    let body = col()
        .w_full()
        .gap(10.0)
        .child(
            text(fill(
                tr("Sigil removes all {} of its relay rules from Windows Firewall. Every region is open again and matchmaking goes back to normal."),
                &[&rules.to_string()],
            ))
            .font(BODY)
            .color(u.prose)
            .wrap(),
        )
        .child(prompt_note(u, tr("Windows shows its admin prompt next. Press Yes there to change the rules; No leaves the firewall as it is.")))
        .child(buttons(
            u,
            u.icon_action(&icons::ROTATE_CCW, tr("Reset"), ActionKind::Primary)
                .key("regions-reset-confirm")
                .cursor(rune::Cursor::Pointer)
                .on_press(super::reset),
        ));
    shell(u, "regions-reset-dialog", tr("Unblock every region"), body)
}

/// Each region's relays as the last sweep found them, under the status bar's relay chip.
pub fn health_popover(s: &State, u: &Ui, sum: &probe::Summary) -> El {
    let regions = &s.servers.regions;
    let mut lines = col().w_full().gap(4.0);
    for line in &sum.regions {
        let tone = match (line.answering, line.measured) {
            (0, _) => u.rose,
            (a, m) if a == m => ping_color(u, probe::health(line.best)),
            _ => u.amber,
        };
        let best = line.best.map(|ms| fill(tr("{} ms"), &[&ms.to_string()])).unwrap_or_else(|| tr("No answer").to_string());
        lines = lines.child(
            row()
                .key(format!("statusbar-relays-{:?}", line.region))
                .w_full()
                .gap(8.0)
                .center_y()
                .child(el().size(6.0, 6.0).radius(3.0).fill(tone).a11y_hidden())
                .child(text(tr(line.region.label())).font(SMALL).color(u.ink).grow(1.0).min_w(0.0).truncate())
                .child(
                    text(fill(tr("{} of {}"), &[&line.answering.to_string(), &line.measured.to_string()]))
                        .font(SMALL_NUM)
                        .color(u.faint),
                )
                .child(row().w(56.0).child(spacer()).child(text(best).font(SMALL_NUM).color(tone))),
        );
    }
    let when = match regions.probed_at {
        Some(at) => fill(tr("Pings measured {}"), &[&crate::statusbar::ago(super::now() - at)]),
        None => tr("Measuring...").to_string(),
    };
    let fastest = sum
        .best
        .as_ref()
        .map(|(id, ms)| {
            let city = regions.catalog.as_ref().and_then(|c| c.location(id)).map_or(id.as_str(), |l| l.city.as_str());
            fill(tr("Fastest: {} ({} ms)"), &[city, &ms.to_string()])
        })
        .unwrap_or_default();
    col()
        .key("statusbar-relays-popover")
        .role("dialog")
        .label(tr("Relay health"))
        .w(280.0)
        .padding(12.0)
        .gap(8.0)
        .fill(u.raised)
        .radius(crate::ui::r(4.0))
        .border(1.0, u.hair_strong)
        .shadow(12.0, 32.0, crate::ui::soft(Color::BLACK.alpha(0.5)))
        .anchored("statusbar-relays", rune::Side::Top, rune::Align::Start, 8.0)
        .transition(crate::motion::pop(true))
        .modal(true, |st: &mut State| st.servers.regions.health_open = false)
        .on_key(|st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                st.servers.regions.health_open = false;
                true
            } else {
                false
            }
        })
        .child(
            row()
                .w_full()
                .center_y()
                .child(text(tr("Relay health")).font(SMALL).color(u.muted))
                .child(spacer())
                .child(text(when).font(SMALL_NUM).color(u.faint)),
        )
        .child(lines)
        .child(text(fastest).font(SMALL_NUM).color(u.faint).wrap().w_full())
        .child(
            text(tr("Ping from this computer to Valve's relays. It says how far each region is, not whether Deadlock's servers are up."))
                .font(SMALL_NUM)
                .color(u.faint)
                .wrap()
                .w_full(),
        )
        .child(
            row()
                .w_full()
                .gap(6.0)
                .child(
                    u.action(tr("Measure again"), ActionKind::Ghost)
                        .key("statusbar-relays-measure")
                        .opacity(if regions.probe_job.is_running() { 0.5 } else { 1.0 })
                        .cursor(rune::Cursor::Pointer)
                        .on_press(super::measure_again),
                )
                .child(spacer())
                .child(
                    u.action(tr("Open Regions"), ActionKind::Secondary)
                        .key("statusbar-relays-open")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| {
                            st.servers.regions.health_open = false;
                            super::open(st);
                        }),
                ),
        )
}
