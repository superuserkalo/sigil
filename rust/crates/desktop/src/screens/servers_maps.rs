//! Custom maps for custom and Deadworks servers: GameBanana's Maps category, the console name a
//! map answers to, and the one-press launch that writes it into `autoexec.cfg`.
//!
//! Network: `gamebanana.com` (already on the architecture allowlist) for the list and a
//! description. Writes: [`launch`] sets the managed autoexec block through
//! `deadlock_core::servers::write_map_launch`, which backs the old file up, then asks Steam to
//! start Deadlock. Nothing here writes into `citadel/maps`.

use deadlock_core::servers::{self as dw, MapEntry};
use rune::{El, Tx, col, el, icon, row, text};

use crate::app::State;
use crate::icons;
use crate::jobs::Job;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, MICRO, MONO, SMALL, Ui};

pub const PANEL_W: f32 = 380.0;

#[derive(Default)]
pub struct MapsState {
    pub entries: Option<Result<Vec<MapEntry>, String>>,
    pub job: Job<Vec<MapEntry>>,
    pub detail_job: Job<(u64, Option<String>)>,
    pub installed: Vec<String>,
    pub open: Option<MapEntry>,
    pub error: Option<String>,
    pub copied: bool,
    pub launch_job: Job<String>,
}

pub fn start_load(st: &mut State) {
    if st.demo || rune::is_headless() || st.servers.maps.job.is_running() {
        return;
    }
    st.servers.maps.error = None;
    let wake = st.wake.clone();
    st.servers.maps.job.start(wake, || {
        dw::fetch_maps(1).map_err(|error| error.to_string())
    });
}

fn start_detail(st: &mut State, entry: MapEntry) {
    st.servers.maps.open = Some(entry.clone());
    st.servers.maps.copied = false;
    if entry.console_name.is_some() || st.demo || rune::is_headless() {
        return;
    }
    if st.servers.maps.detail_job.is_running() {
        return;
    }
    let id = entry.id;
    let wake = st.wake.clone();
    st.servers.maps.detail_job.start(wake, move || {
        let description = dw::fetch_map_description(id).map_err(|error| error.to_string())?;
        Ok((id, description))
    });
}

pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.servers.maps.job.try_take() {
        match result {
            Ok(mut entries) => {
                let installed = st
                    .game_root
                    .as_deref()
                    .map(dw::installed_maps)
                    .unwrap_or_default();
                for entry in &mut entries {
                    entry.installed = installed.iter().any(|name| {
                        entry
                            .name
                            .to_lowercase()
                            .contains(&name.to_lowercase())
                    });
                }
                st.servers.maps.installed = installed;
                if let Some(open) = &st.servers.maps.open
                    && let Some(fresh) = entries.iter().find(|entry| entry.id == open.id)
                {
                    st.servers.maps.open = Some(fresh.clone());
                }
                st.servers.maps.entries = Some(Ok(entries));
            }
            Err(error) => st.servers.maps.entries = Some(Err(error)),
        }
        changed = true;
    }
    if let Some(result) = st.servers.maps.detail_job.try_take() {
        match result {
            Ok((id, description)) => {
                if let Some(open) = st.servers.maps.open.as_mut()
                    && open.id == id
                {
                    let description = description.unwrap_or_default();
                    if description.trim().is_empty() {
                        st.servers.maps.error =
                            Some("This map's page has no description to read.".to_string());
                    } else {
                        open.console_name = dw::console_map_name(&description);
                        if open.console_name.is_none() {
                            st.servers.maps.error = Some(
                                "This map's page does not name its console map. Open the page to read how the author says to load it."
                                    .to_string(),
                            );
                        }
                    }
                }
            }
            Err(error) => {
                st.servers.maps.error = Some(crate::screens::profiles::friendly(&error))
            }
        }
        changed = true;
    }
    if let Some(result) = st.servers.maps.launch_job.try_take() {
        match result {
            Ok(line) => st.status = line,
            Err(error) => crate::notify::push(
                st,
                crate::notify::Notice::error(crate::screens::profiles::friendly(&error)),
            ),
        }
        changed = true;
    }
    changed
}

/// GAME WRITE + PROCESS. Set the launch map in autoexec.cfg (with its backup) and start Deadlock.
fn launch(st: &mut State, map: String) {
    if st.demo || rune::is_headless() {
        st.status = format!("The next launch starts {map}.");
        return;
    }
    if deadlock_core::profiles::game_running() {
        crate::notify::push(
            st,
            crate::notify::Notice::warning(
                "Deadlock is running. Close it, then launch the map.",
            ),
        );
        return;
    }
    let Some(root) = st.game_root.clone() else {
        crate::notify::push(
            st,
            crate::notify::Notice::error("Deadlock was not found. Nothing was changed."),
        );
        return;
    };
    let wake = st.wake.clone();
    let name = map.clone();
    st.servers.maps.launch_job.start(wake, move || {
        dw::write_map_launch(&root, Some(&map), true).map_err(|error| error.to_string())?;
        deadlock_core::launch::launch_modded(true).map_err(|error| error.to_string())?;
        Ok(format!("The next launch starts {name}."))
    });
}

fn clear_launch_map(st: &mut State) {
    if st.demo || rune::is_headless() {
        st.status = "The map launch is removed.".to_string();
        return;
    }
    let Some(root) = st.game_root.clone() else {
        return;
    };
    let wake = st.wake.clone();
    st.servers.maps.launch_job.start(wake, move || {
        dw::write_map_launch(&root, None, true).map_err(|error| error.to_string())?;
        Ok("The map launch is removed.".to_string())
    });
}

pub fn view(s: &State, u: &Ui) -> El {
    let v = &s.servers.maps;
    let mut page = col().key("maps-body").w_full().gap(16.0);

    let body = match &v.entries {
        None => u.empty_state("", crate::screens::servers::tr("Getting maps"), "The list comes from GameBanana.", None),
        Some(Err(error)) => u.empty_state(
            "",
            crate::screens::servers::tr("Maps could not load"),
            error,
            Some(
                u.action(crate::screens::servers::tr("Try again"), ActionKind::Secondary)
                    .key("maps-retry")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(start_load),
            ),
        ),
        Some(Ok(entries)) if entries.is_empty() => u.empty_state(
            "",
            crate::screens::servers::tr("No maps yet"),
            "Nobody has posted a map for Deadlock.",
            None,
        ),
        Some(Ok(entries)) => {
            let open = v.open.as_ref();
            let list = map_list(s, u, entries, open);
            let mut body = row()
                .w_full()
                .gap(20.0)
                .items_start()
                .child(list);
            if open.is_some() {
                body = body.child(panel(s, u));
            }
            body
        }
    };

    let launch = s.game_root.as_deref().and_then(dw::read_map_launch);
    if let Some(current) = launch {
        let mut line = row()
            .key("maps-current")
            .w_full()
            .gap(8.0)
            .center_y()
            .child(icon(&icons::PIN, 14.0).color(u.muted).a11y_hidden())
            .child(
                text(format!("The next launch starts {current}"))
                    .font(BODY)
                    .color(u.prose)
                    .grow(1.0),
            );
        line = line.child(
            u.action(crate::screens::servers::tr("Remove"), ActionKind::Ghost)
                .key("maps-clear-launch")
                .cursor(rune::Cursor::Pointer)
                .on_press(clear_launch_map),
        );
        page = page.child(line);
    }
    page.child(body)
}

fn map_list(s: &State, u: &Ui, entries: &[MapEntry], open: Option<&MapEntry>) -> El {
    let mut list = col().w_full().grow(1.0).min_w(0.0);
    for entry in entries {
        let chosen = open.is_some_and(|open| open.id == entry.id);
        let target = entry.clone();
        let console = entry
            .console_name
            .clone()
            .map(|name| format!("map {name}"))
            .unwrap_or_else(|| {
                if entry.installed {
                    crate::screens::servers::tr("Installed").to_string()
                } else {
                    crate::screens::servers::tr("Not installed").to_string()
                }
            });
        list = list.child(
            row()
                .key(format!("map-{}", entry.id))
                .w_full()
                .h(56.0)
                .px(8.0)
                .gap(12.0)
                .center_y()
                .radius(crate::ui::r(8.0))
                .fill(if chosen { u.selected } else { rune::Color::TRANSPARENT })
                .hover(u.hover)
                .focusable(true)
                .role("button")
                .label(format!("{}, by {}", entry.name, entry.author))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| start_detail(st, target.clone()))
                .child(
                    el()
                        .size(40.0, 40.0)
                        .radius(crate::ui::r(8.0))
                        .fill(u.well)
                        .border(1.0, u.hair)
                        .center()
                        .child(
                            icon(&icons::LAYOUT_GRID, 16.0)
                                .color(u.faint)
                                .a11y_hidden(),
                        ),
                )
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(
                            text(entry.name.clone())
                                .font(BODY_MED)
                                .color(u.ink)
                                .truncate()
                                .a11y_hidden(),
                        )
                        .child(
                            text(format!("{}  ·  {console}", entry.author))
                                .font(SMALL)
                                .color(u.faint)
                                .truncate()
                                .a11y_hidden(),
                        ),
                )
                .child(
                    text(plural(entry.likes.max(0) as usize, "like"))
                        .font(MICRO)
                        .color(u.faint)
                        .a11y_hidden(),
                ),
        );
    }
    let _ = s;
    list
}

fn panel(s: &State, u: &Ui) -> El {
    let v = &s.servers.maps;
    let Some(entry) = v.open.as_ref() else {
        return el();
    };
    let mut panel = col()
        .key("map-panel")
        .w(PANEL_W)
        .min_w(PANEL_W)
        .gap(12.0)
        .pt(4.0);
    panel = panel
        .child(text(entry.name.clone()).font(H1).color(u.ink).wrap())
        .child(
            text(format!("by {}", entry.author))
                .font(BODY)
                .color(u.muted),
        );

    let installed = entry
        .console_name
        .as_ref()
        .is_some_and(|name| v.installed.iter().any(|have| have == name))
        || entry.installed;
    let console = entry.console_name.clone();
    if let Some(name) = &console {
        panel = panel.child(
            col()
                .w_full()
                .gap(6.0)
                .child(text(crate::screens::servers::tr("How to play")).font(BODY_MED).color(u.ink))
                .child(
                    text("Open the console with ` and run:")
                        .font(SMALL)
                        .color(u.muted),
                )
                .child(
                    row()
                        .w_full()
                        .h(36.0)
                        .px(10.0)
                        .gap(8.0)
                        .center_y()
                        .radius(crate::ui::r(8.0))
                        .fill(u.well)
                        .border(1.0, u.hair)
                        .child(
                            text(format!("map {name}"))
                                .font(MONO)
                                .color(u.prose)
                                .grow(1.0)
                                .min_w(0.0)
                                .a11y_hidden(),
                        )
                        .child(
                            u.icon_button(
                                if v.copied { &icons::CHECK } else { &icons::COPY },
                                if v.copied { crate::screens::servers::tr("Copied") } else { crate::screens::servers::tr("Copy command") },
                            )
                            .key("map-copy")
                            .cursor(rune::Cursor::Pointer)
                            .on_press_tx({
                                let command = format!("map {name}");
                                move |st: &mut State, tx: &mut Tx| {
                                    tx.copy(command.clone());
                                    st.servers.maps.copied = true;
                                }
                            }),
                        ),
                ),
        );
    } else {
        panel = panel.child(
            col()
                .w_full()
                .gap(6.0)
                .child(text(crate::screens::servers::tr("How to play")).font(BODY_MED).color(u.ink))
                .child(
                    text("This map's page does not name its console map. Open the page to read how the author says to load it.")
                        .font(BODY)
                        .color(u.muted)
                        .wrap(),
                )
                .child(
                    u.icon_action(&icons::EXTERNAL_LINK, crate::screens::servers::tr("Open page"), ActionKind::Ghost)
                        .key("map-page")
                        .cursor(rune::Cursor::Pointer)
                        .on_press_tx({
                            let url = entry.page_url.clone();
                            move |st: &mut State, tx: &mut Tx| {
                                if let Err(error) = tx.open_url(&url) {
                                    st.status = format!("The page could not be opened: {error}");
                                }
                            }
                        }),
                ),
        );
    }
    if let Some(error) = &v.error {
        panel = panel.child(text(error.clone()).font(SMALL).color(u.amber).wrap());
    }
    if !installed {
        panel = panel.child(
            text("The map is not in citadel/maps yet. Install it from its page first, then launch.")
                .font(SMALL)
                .color(u.amber)
                .wrap(),
        );
        panel = panel.child(
            u.icon_action(&icons::EXTERNAL_LINK, crate::screens::servers::tr("Open page"), ActionKind::Secondary)
                .key("map-page-install")
                .cursor(rune::Cursor::Pointer)
                .on_press_tx({
                    let url = entry.page_url.clone();
                    move |st: &mut State, tx: &mut Tx| {
                        if let Err(error) = tx.open_url(&url) {
                            st.status = format!("The page could not be opened: {error}");
                        }
                    }
                }),
        );
    } else if let Some(name) = console {
        panel = panel.child(
            u.action(crate::screens::servers::tr("Launch map"), ActionKind::Primary)
                .key("map-launch")
                .cursor(rune::Cursor::Pointer)
                .on_press({
                    let name = name.clone();
                    move |st: &mut State| launch(st, name.clone())
                }),
        );
        panel = panel.child(
            text("Sigil writes `map <name>` into autoexec.cfg (the old file is backed up) and starts Deadlock.")
                .font(SMALL)
                .color(u.faint)
                .wrap(),
        );
    }
    panel = panel.child(
        u.icon_action(&icons::EXTERNAL_LINK, crate::screens::servers::tr("Open page"), ActionKind::Ghost)
            .key("map-page-open")
            .cursor(rune::Cursor::Pointer)
            .on_press_tx({
                let url = entry.page_url.clone();
                move |st: &mut State, tx: &mut Tx| {
                    if let Err(error) = tx.open_url(&url) {
                        st.status = format!("The page could not be opened: {error}");
                    }
                }
            }),
    );
    panel
}

fn plural(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

// ---------------------------------------------------------------------------------------------
// Scenes
// ---------------------------------------------------------------------------------------------

fn fixture_maps() -> Vec<MapEntry> {
    let entry = |id: u64, name: &str, author: &str, likes: i64, installed: bool, console: Option<&str>| {
        MapEntry {
            id,
            name: name.to_string(),
            author: author.to_string(),
            likes,
            updated_unix: 1_791_800_000,
            page_url: format!("https://gamebanana.com/mods/{id}"),
            thumbnail: None,
            console_name: console.map(str::to_string),
            installed,
        }
    };
    vec![
        entry(741_438, "Custom Maps Menu", "Eldric_", 24, false, None),
        entry(742_901, "Harbor at Night", "quill", 96, true, Some("harbor_night")),
        entry(743_112, "Small Arena", "mossy", 41, false, Some("small_arena")),
        entry(743_207, "Street Duel", "vexa", 17, false, None),
        entry(743_400, "Midtown Fog", "rook", 63, false, Some("midtown_fog")),
    ]
}

pub fn scene(s: &mut State) {
    s.servers.maps.installed = vec!["harbor_night".to_string(), "dl_midtown".to_string()];
    s.servers.maps.entries = Some(Ok(fixture_maps()));
}

pub fn scene_detail(s: &mut State) {
    scene(s);
    let maps = fixture_maps();
    s.servers.maps.open = Some(maps[1].clone());
}

pub fn scene_unresolved(s: &mut State) {
    scene(s);
    let maps = fixture_maps();
    s.servers.maps.open = Some(maps[3].clone());
    s.servers.maps.error = Some(
        "This map's page does not name its console map. Open the page to read how the author says to load it."
            .to_string(),
    );
}

