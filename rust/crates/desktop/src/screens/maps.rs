//! Custom maps (Deadlock Mod Manager's experimental "custom-maps" feature): a Maps page with
//! GameBanana's Maps category, and "How to play" on a map's details, on the Maps page and on an
//! installed map mod.
//!
//! - The list is Browse's catalog read (`gamebanana::fetch_browse`) pinned to the Maps
//!   category; installs go through Browse's install button (`browse::quick`), so a map lands in
//!   an addon slot like every other mod. Network: `gamebanana.com` only (allowlisted).
//! - The map name comes from the installed VPK's `maps/<name>.vpk` entries
//!   (`deadlock_core::maps`), else from the author's description, as DMM reads it.
//! - Launch map writes `map <name>` into Sigil's managed `autoexec.cfg` block (backed up,
//!   DMM's way: `servers::write_map_launch`), then starts the game through the launch gate and
//!   the launch progress steps. The press is the approval. A `steam://run/1422450//+map`
//!   URL would also carry the command, but Steam stops to ask about launch arguments from a link.
//! - Invite friends follows `console.log` for `ServerSteamID=[A:...]` (DMM's watcher) and shows
//!   `connect <code>`, the line friends paste in their console (F7).
//! - Host this map hands the map to Servers > Host (`host::open_with_map`).
//! - "Play with friends, no setup" sits on top (`host::listen::card`): pick a map, Launch (this
//!   file's `launch`, with `-condebug` added first), then a Sigil invite and the connect line.
//!
//! Off by default: Settings > Experimental > Custom maps (`FLAG`) adds the page and the menus.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use deadlock_core::gamebanana::{self, BrowseMod, BrowseOrder, BrowseQuery, BrowseSection};
use deadlock_core::maps::{self as core_maps, ConnectWatch, InstalledMap};
use rune::{Color, El, ScrollSpec, SurfaceFit, Tx, col, el, icon, row, text};

use crate::app::State;
use crate::icons;
use crate::jobs::Job;
use crate::model::Screen;
use crate::screens::browse::quick;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{ActionKind, BODY, BODY_MED, H1, MONO, PAGE_TOP, PAGE_X, SMALL, Ui};

/// Settings > Experimental > Custom maps.
pub const FLAG: &str = "custom_maps";
/// GameBanana's Maps root under Mods (game 20948).
pub const MAPS_CATEGORY: u64 = deadlock_core::servers::MAP_CATEGORY_ID;
/// How long Invite friends waits for the map server.
pub const WATCH_FOR: Duration = Duration::from_secs(180);
const PANEL_W: f32 = 380.0;
const PER_PAGE: usize = 50;

/// The section title DMM gives the instructions, shared by every place that shows them.
pub const HOW_TO_PLAY: &str = "How to play";

/// What the panel shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Open {
    /// A map from the GameBanana list.
    Mod { id: u64, name: String, author: String, page_url: String },
    /// An installed map mod, by slot.
    Slot(u16),
}

/// Invite friends.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Invite {
    #[default]
    Idle,
    /// Following console.log for the map server's connect code.
    Watching(String),
    Ready { map: String, code: String },
    /// The wait ran out or was stopped.
    Missed(String),
}

/// A Maps category page and the thumbnails it shows, as (url, bytes).
pub type CatalogRead = (Vec<BrowseMod>, Vec<(String, Vec<u8>)>);

#[derive(Default)]
pub struct Page {
    pub catalog: Option<Result<Vec<BrowseMod>, String>>,
    pub job: Job<CatalogRead>,
    pub installed: Vec<InstalledMap>,
    pub scanned: bool,
    pub scan_job: Job<Vec<InstalledMap>>,
    pub open: Option<Open>,
    /// Map names read from a GameBanana description, by mod id (None: the page names none).
    pub described: std::collections::HashMap<u64, Option<String>>,
    pub describe_job: Job<(u64, Option<String>)>,
    /// The command last copied, so its button reads "Copied".
    pub copied: Option<String>,
    pub invite: Invite,
    pub watch_job: Job<Option<String>>,
    pub watch_stop: Option<Arc<AtomicBool>>,
    pub write_job: Job<String>,
    /// The map a Launch map press just set, and when: a launch within two minutes is that map
    /// launch (through the gameinfo gate's Fix and launch); any other launch clears the block.
    pub armed: Option<(String, Instant)>,
}

/// A normal launch must not start the map from an earlier Launch map press (the block stays in
/// autoexec.cfg). Removes that block, with a backup, unless this is the map launch itself.
pub fn clear_stale_map_launch(st: &mut State) {
    let fresh = st
        .maps
        .armed
        .take()
        .is_some_and(|(_, at)| at.elapsed() < Duration::from_secs(120));
    if fresh || st.demo || rune::is_headless() {
        return;
    }
    let Some(root) = st.game_root.clone() else { return };
    let Some(map) = deadlock_core::servers::read_map_launch(&root) else { return };
    if deadlock_core::servers::write_map_launch(&root, None, true).is_ok() {
        crate::notify::push(
            st,
            crate::notify::Notice::info(fill(tr("{} is no longer set as the start map."), &[&map]))
                .detail(tr("Use Launch map on the Maps page to start a map again.")),
        );
    }
}

pub fn on(s: &State) -> bool {
    s.prefs.flag(FLAG)
}

// --- loading ------------------------------------------------------------------------------

/// The Maps page opened: read the list once and scan the addon folder for maps.
pub fn on_open(st: &mut State) {
    if st.maps.catalog.is_none() {
        start_catalog(st);
    }
    start_scan(st);
}

fn query(st: &State) -> BrowseQuery {
    BrowseQuery {
        section: BrowseSection::Mods,
        category: Some(MAPS_CATEGORY),
        order: BrowseOrder::from_sort(st.prefs.browse_sort),
        ..BrowseQuery::default()
    }
}

/// Read the Maps category, as Browse reads any category. Thumbnails come with it.
pub fn start_catalog(st: &mut State) {
    if st.demo || rune::is_headless() || st.maps.job.is_running() {
        return;
    }
    let query = query(st);
    let wake = st.wake.clone();
    st.maps.job.start(wake, move || {
        let page = gamebanana::fetch_browse(&query, 1, PER_PAGE).map_err(|error| error.to_string())?;
        let thumbs = page
            .mods
            .iter()
            .filter_map(|m| m.thumbnail_url.clone())
            .filter_map(|url| crate::data::fetch_thumb_bytes(&url).map(|bytes| (url, bytes)))
            .collect();
        Ok((page.mods, thumbs))
    });
}

/// Which installed mods carry a map. Reads the addon folder and the install journal.
pub fn start_scan(st: &mut State) {
    if st.demo || rune::is_headless() || st.maps.scan_job.is_running() {
        return;
    }
    let Some(addons) = st.addons_dir.clone().map(PathBuf::from) else {
        return;
    };
    let wake = st.wake.clone();
    st.maps.scan_job.start(wake, move || {
        let mut maps = core_maps::installed_maps(&addons);
        let journal = deadlock_core::install::read_journal(&crate::data::workspace_dir()).unwrap_or_default();
        core_maps::attach_titles(&mut maps, &journal);
        Ok(maps)
    });
}

fn start_describe(st: &mut State, id: u64) {
    if st.maps.described.contains_key(&id) || st.demo || rune::is_headless() || st.maps.describe_job.is_running() {
        return;
    }
    let wake = st.wake.clone();
    st.maps.describe_job.start(wake, move || {
        let text = deadlock_core::servers::fetch_map_description(id).map_err(|error| error.to_string())?;
        Ok((id, core_maps::pick_map_name(&[], text.as_deref())))
    });
}

pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.maps.job.try_take() {
        st.maps.catalog = Some(match result {
            Ok((mods, thumbs)) => {
                st.pending_thumbs.extend(thumbs);
                Ok(mods)
            }
            Err(error) => Err(crate::screens::profiles::friendly(&error)),
        });
        changed = true;
    }
    if let Some(result) = st.maps.scan_job.try_take() {
        if let Ok(maps) = result {
            st.maps.installed = maps;
        }
        st.maps.scanned = true;
        changed = true;
    }
    if let Some(result) = st.maps.describe_job.try_take() {
        if let Ok((id, name)) = result {
            st.maps.described.insert(id, name);
        }
        changed = true;
    }
    if let Some(result) = st.maps.write_job.try_take() {
        match result {
            Ok(line) => st.status = line,
            Err(error) => crate::notify::push(
                st,
                crate::notify::Notice::error(crate::screens::profiles::friendly(&error)),
            ),
        }
        changed = true;
    }
    if let Some(result) = st.maps.watch_job.try_take() {
        st.maps.watch_stop = None;
        if let Invite::Watching(map) = st.maps.invite.clone() {
            st.maps.invite = match result {
                Ok(Some(code)) => Invite::Ready { map, code },
                _ => Invite::Missed(map),
            };
        }
        changed = true;
    }
    // "Play with friends, no setup": the Sigil invite from the code (servers_host/listen.rs).
    changed |= crate::screens::host::listen::refresh(st);
    changed
}

// --- what a map is ------------------------------------------------------------------------

/// Everything "How to play" needs about one map.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Play {
    /// The console name, when Sigil knows it.
    pub map: Option<String>,
    /// The addon slot that holds it, when installed.
    pub slot: Option<u16>,
    /// The slot is on, so the game loads the map.
    pub enabled: bool,
    /// The slot's directory VPK, when the map is installed and on.
    pub vpk: Option<PathBuf>,
    pub page_url: Option<String>,
    /// The GameBanana mod, for its install button.
    pub mod_id: Option<u64>,
    pub title: String,
}

fn slot_enabled(s: &State, slot: u16) -> bool {
    s.addons.addons.iter().any(|a| a.slot == slot && a.enabled)
}

/// A map from GameBanana: the installed copy's name first, else the description's.
pub fn play_for_mod(s: &State, id: u64, title: &str, page_url: &str, description: Option<&str>) -> Play {
    let slot = quick::owned_slot(s, id);
    let installed: Vec<&InstalledMap> = slot
        .map(|slot| s.maps.installed.iter().filter(|m| m.slot == slot).collect())
        .unwrap_or_default();
    let vpk_names: Vec<String> = installed.iter().map(|m| m.name.clone()).collect();
    let described = s.maps.described.get(&id).cloned().flatten();
    let map = core_maps::pick_map_name(&vpk_names, description).or(described);
    Play {
        map,
        slot,
        enabled: slot.is_some_and(|slot| slot_enabled(s, slot)),
        vpk: installed.first().map(|m| m.vpk.clone()),
        page_url: Some(page_url.to_string()).filter(|u| !u.is_empty()),
        mod_id: Some(id),
        title: title.to_string(),
    }
}

/// An installed map mod, by slot.
pub fn play_for_slot(s: &State, slot: u16) -> Play {
    let installed = s.maps.installed.iter().find(|m| m.slot == slot);
    let meta = s.installed.library.mods.iter().find(|m| m.slot == slot);
    let id = meta.and_then(|m| m.gamebanana_id);
    Play {
        map: installed.map(|m| m.name.clone()),
        slot: Some(slot),
        enabled: slot_enabled(s, slot),
        vpk: installed.map(|m| m.vpk.clone()),
        page_url: id.map(|id| format!("https://gamebanana.com/mods/{id}")),
        mod_id: id,
        title: installed
            .and_then(|m| m.title.clone())
            .or_else(|| meta.map(|m| m.name.clone()).filter(|n| !n.is_empty()))
            .unwrap_or_else(|| format!("pak{slot:02}")),
    }
}

// --- actions ------------------------------------------------------------------------------

/// Open the Maps page on an installed map mod (Installed's card menu).
pub fn open_slot(st: &mut State, slot: u16) {
    st.go(Screen::Maps);
    st.maps.open = Some(Open::Slot(slot));
    st.maps.copied = None;
}

fn open_mod(st: &mut State, m: &BrowseMod) {
    st.maps.open = Some(Open::Mod {
        id: m.id,
        name: m.name.clone(),
        author: m.author.clone(),
        page_url: m.profile_url.clone(),
    });
    st.maps.copied = None;
    let installed_name = quick::owned_slot(st, m.id)
        .is_some_and(|slot| st.maps.installed.iter().any(|map| map.slot == slot));
    if !installed_name {
        start_describe(st, m.id);
    }
}

/// GAME WRITE + PROCESS. Put `map <name>` in autoexec.cfg's managed block (backed up), then
/// start Deadlock through the gameinfo gate and the launch steps. The press is the approval.
pub fn launch(st: &mut State, map: String) {
    if core_maps::map_command(&map).is_none() {
        return;
    }
    if st.demo || rune::is_headless() {
        st.status = fill(tr("The next launch starts {}."), &[&map]);
        st.maps.invite = Invite::Watching(map);
        return;
    }
    if deadlock_core::profiles::game_running() {
        crate::notify::push(st, crate::notify::Notice::warning(tr("Deadlock is running. Close it, then launch the map.")));
        return;
    }
    let Some(root) = st.game_root.clone() else {
        crate::notify::push(st, crate::notify::Notice::error(tr("Deadlock was not found. Nothing was changed.")));
        return;
    };
    if let Err(error) = deadlock_core::servers::write_map_launch(&root, Some(&map), true) {
        crate::notify::push(st, crate::notify::Notice::error(tr("The map could not be set")).detail(error.to_string()));
        return;
    }
    st.status = fill(tr("The next launch starts {}."), &[&map]);
    st.maps.armed = Some((map.clone(), Instant::now()));
    start_watch(st, map);
    // The gameinfo gate first, as every launch. The map mod question of a normal launch is
    // skipped: this launch is for the map.
    if crate::screens::launch_gate::blocked(st, false, true) {
        return;
    }
    st.maps.armed = None;
    crate::screens::launch_progress::begin_checked(st, false, true);
}

/// GAME WRITE. Remove the launch block, so the next launch starts the game as usual.
fn clear_launch(st: &mut State) {
    if st.demo || rune::is_headless() {
        st.status = tr("The map launch is removed.").to_string();
        return;
    }
    let Some(root) = st.game_root.clone() else {
        return;
    };
    let wake = st.wake.clone();
    st.maps.write_job.start(wake, move || {
        deadlock_core::servers::write_map_launch(&root, None, true).map_err(|error| error.to_string())?;
        Ok(tr("The map launch is removed.").to_string())
    });
}

/// Follow console.log for the map server's connect code, up to [`WATCH_FOR`].
pub fn start_watch(st: &mut State, map: String) {
    stop_watch(st);
    st.maps.invite = Invite::Watching(map);
    if st.demo || rune::is_headless() {
        return;
    }
    let Some(root) = st.game_root.clone() else {
        return;
    };
    let stop = Arc::new(AtomicBool::new(false));
    st.maps.watch_stop = Some(stop.clone());
    let wake = st.wake.clone();
    st.maps.watch_job.start(wake, move || {
        let path = deadlock_core::game_session::console_log_path(&root);
        let mut watch = ConnectWatch::from_end(&path);
        let started = Instant::now();
        while !stop.load(Ordering::Relaxed) && started.elapsed() < WATCH_FOR {
            if let Some(code) = watch.poll(&path) {
                return Ok(Some(code));
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        Ok(None)
    });
}

fn stop_watch(st: &mut State) {
    if let Some(stop) = st.maps.watch_stop.take() {
        stop.store(true, Ordering::Relaxed);
    }
}

// --- views --------------------------------------------------------------------------------

fn command_box(s: &State, u: &Ui, key: &str, command: String) -> El {
    let copied = s.maps.copied.as_deref() == Some(command.as_str());
    row()
        .key(key.to_string())
        .w_full()
        .h(36.0)
        .px(10.0)
        .gap(8.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .fill(u.well)
        .border(1.0, u.hair)
        .child(
            text(command.clone())
                .font(MONO)
                .color(u.prose)
                .grow(1.0)
                .min_w(0.0)
                .truncate()
                .selectable(true),
        )
        .child(
            u.icon_button(
                if copied { &icons::CHECK } else { &icons::COPY },
                if copied { tr("Copied") } else { tr("Copy command") },
            )
            .key(format!("{key}-copy"))
            .cursor(rune::Cursor::Pointer)
            .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                tx.copy(command.clone());
                st.maps.copied = Some(command.clone());
            }),
        )
}

fn line(words: &str, color: Color) -> El {
    text(tr(words).to_string()).font(SMALL).color(color).wrap()
}

/// "How to play": the map name and its console command, Launch map, Host this map, and
/// Invite friends. `key` keeps two copies on one screen apart.
pub fn how_to_play(s: &State, u: &Ui, key: &str, play: &Play) -> El {
    let mut c = col()
        .key(format!("{key}-how-to-play"))
        .w_full()
        .gap(8.0)
        .child(text(tr(HOW_TO_PLAY)).font(BODY_MED).color(u.ink));
    match &play.map {
        Some(map) => {
            c = c
                .child(line("Open the console in Deadlock (F7) and run this, or let Sigil start the map:", u.muted))
                .child(command_box(s, u, &format!("{key}-map"), format!("map {map}")));
        }
        None => {
            c = c.child(line(
                "Sigil could not find this map's name. Check the author's description for the command to use.",
                u.amber,
            ));
        }
    }
    let ready = play.map.is_some() && play.vpk.is_some() && play.enabled;
    if play.slot.is_none() {
        c = c.child(line("Install the map first. Friends who join need it installed too.", u.muted));
        if let Some(id) = play.mod_id {
            let b = quick::button(s, id);
            c = c.child(row().child(quick::button_el(u, &b, format!("{key}-install"), id, &play.title, quick::Size::Row)));
        }
    } else if !play.enabled {
        c = c.child(line("This map is switched off. Turn it on in Installed, then launch it.", u.amber));
    } else if play.vpk.is_none() && play.map.is_some() {
        c = c.child(line("The installed files carry no map under maps/. The map may load under another name.", u.amber));
    }
    let mut actions = row().w_full().gap(8.0).center_y().flex_wrap();
    if let Some(map) = play.map.clone().filter(|_| ready) {
        actions = actions.child(
            u.icon_action(&icons::PLAY, tr("Launch map"), ActionKind::Primary)
                .key(format!("{key}-launch"))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| launch(st, map.clone())),
        );
    }
    if let (Some(map), Some(vpk)) = (play.map.clone(), play.vpk.clone()) {
        actions = actions.child(
            u.icon_action(&crate::screens::servers::SERVER_ICON, tr("Host this map"), ActionKind::Secondary)
                .key(format!("{key}-host"))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| crate::screens::host::open_with_map(st, &map, &vpk)),
        );
    }
    if let Some(url) = play.page_url.clone() {
        actions = actions.child(
            u.icon_action(&icons::EXTERNAL_LINK, tr("Open page"), ActionKind::Ghost)
                .key(format!("{key}-page"))
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                    if let Err(error) = tx.open_url(&url) {
                        st.status = error.to_string();
                    }
                }),
        );
    }
    c = c.child(actions);
    if ready {
        c = c.child(line(
            "Launch map writes the map command into autoexec.cfg (the old file is backed up) and starts Deadlock.",
            u.faint,
        ));
    }
    c.child(invite(s, u, key, play))
}

fn invite(s: &State, u: &Ui, key: &str, play: &Play) -> El {
    let mut c = col()
        .key(format!("{key}-invite"))
        .w_full()
        .gap(8.0)
        .pt(8.0)
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(icon(&icons::USER, 14.0).color(u.muted).a11y_hidden())
                .child(text(tr("Invite friends")).font(BODY_MED).color(u.ink)),
        );
    let this_map = |name: &str| play.map.as_deref() == Some(name);
    match &s.maps.invite {
        Invite::Ready { map, code } if this_map(map) => {
            c = c
                .child(line("Your map server is up. Friends open the console in Deadlock (F7), paste this and press Enter:", u.muted))
                .child(command_box(s, u, &format!("{key}-connect"), core_maps::connect_command(code)))
                .child(line("They need the same map installed.", u.faint));
        }
        Invite::Watching(map) if this_map(map) => {
            c = c
                .child(line("Waiting for the map server. The connect command shows here when it is up.", u.muted))
                .child(
                    row().child(
                        u.action(tr("Stop waiting"), ActionKind::Ghost)
                            .key(format!("{key}-invite-stop"))
                            .cursor(rune::Cursor::Pointer)
                            .on_press(|st: &mut State| {
                                stop_watch(st);
                                if let Invite::Watching(map) = st.maps.invite.clone() {
                                    st.maps.invite = Invite::Missed(map);
                                }
                            }),
                    ),
                );
        }
        Invite::Missed(map) if this_map(map) => {
            let map = map.clone();
            c = c
                .child(line("No connect code showed up. Sigil reads it from console.log, which the game writes with -condebug.", u.amber))
                .child(
                    row().child(
                        u.action(tr("Watch again"), ActionKind::Secondary)
                            .key(format!("{key}-invite-again"))
                            .cursor(rune::Cursor::Pointer)
                            .on_press(move |st: &mut State| start_watch(st, map.clone())),
                    ),
                );
        }
        _ => {
            c = c.child(line("Launch the map and Sigil shows the command friends paste in their console (F7) to join you.", u.muted));
            if !crate::screens::settings::options::get().condebug {
                c = c.child(line(
                    "Sigil reads the command from the game's console log. Turn on the console log (-condebug) in Settings first.",
                    u.faint,
                ));
            }
        }
    }
    c
}

/// Browse's details: "How to play" for a mod GameBanana files under Maps. None when Custom
/// maps is off or the mod is not a map.
pub fn details_section(s: &State, u: &Ui, id: u64, root: &str, leaf: &str, page_url: &str, description: &str) -> Option<El> {
    if !on(s) || !crate::screens::installed::maps_category(Some(root), Some(leaf)) {
        return None;
    }
    let play = play_for_mod(s, id, "", page_url, Some(description));
    Some(how_to_play(s, u, "details-map", &Play { page_url: None, ..play }))
}

fn thumb(s: &State, u: &Ui, url: Option<&String>) -> El {
    let frame = el().size(64.0, 36.0).radius(crate::ui::r(6.0)).clip_rounded(crate::ui::r(6.0)).fill(u.well).border(1.0, u.hair);
    match url.and_then(|url| s.thumbnails.get(url)) {
        Some(px) => frame.child(el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).pixels_fit(px, SurfaceFit::Fill)).a11y_hidden(),
        None => frame.center().child(icon(&icons::LAYOUT_GRID, 14.0).color(u.faint).a11y_hidden()).a11y_hidden(),
    }
}

#[allow(clippy::too_many_arguments)]
fn list_row(u: &Ui, key: String, chosen: bool, label: String, lead: El, title: String, sub: String, trailing: El) -> El {
    row()
        .key(key)
        .w_full()
        .min_h(52.0)
        .px(8.0)
        .py(6.0)
        .gap(12.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .fill(if chosen { u.selected } else { Color::TRANSPARENT })
        .hover(u.hover)
        .focusable(true)
        .role("button")
        .label(label)
        .cursor(rune::Cursor::Pointer)
        .child(lead)
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(text(title).font(BODY_MED).color(u.ink).truncate().a11y_hidden())
                .child(text(sub).font(SMALL).color(u.faint).truncate().a11y_hidden()),
        )
        .child(trailing)
}

fn installed_list(s: &State, u: &Ui) -> Option<El> {
    if s.maps.installed.is_empty() {
        return None;
    }
    let mut list = col().key("maps-installed").w_full().gap(2.0).child(u.section(tr("Installed maps")));
    for m in &s.maps.installed {
        let slot = m.slot;
        let chosen = s.maps.open == Some(Open::Slot(slot));
        let title = m.title.clone().unwrap_or_else(|| m.name.clone());
        list = list.child(
            list_row(
                u,
                format!("maps-slot-{slot}-{}", m.name),
                chosen,
                title.clone(),
                el().size(64.0, 36.0)
                    .radius(crate::ui::r(6.0))
                    .fill(u.well)
                    .border(1.0, u.hair)
                    .center()
                    .child(icon(&icons::PLAY, 14.0).color(u.teal).a11y_hidden())
                    .a11y_hidden(),
                title,
                format!("map {}  ·  pak{:02}", m.name, slot),
                el(),
            )
            .on_press(move |st: &mut State| {
                st.maps.open = Some(Open::Slot(slot));
                st.maps.copied = None;
            }),
        );
    }
    Some(list)
}

fn catalog_list(s: &State, u: &Ui) -> El {
    let mut list = col().key("maps-catalog").w_full().gap(2.0).child(u.section(tr("On GameBanana")));
    match &s.maps.catalog {
        None => list.child(u.empty_state("", tr("Getting maps"), tr("The list comes from GameBanana."), None)),
        Some(Err(error)) => list.child(u.empty_state(
            "",
            tr("Maps could not load"),
            error,
            Some(
                u.action(tr("Try again"), ActionKind::Secondary)
                    .key("maps-retry")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(start_catalog),
            ),
        )),
        Some(Ok(mods)) if mods.is_empty() => {
            list.child(u.empty_state("", tr("No maps yet"), tr("Nobody has posted a map for Deadlock."), None))
        }
        Some(Ok(mods)) => {
            for m in mods {
                let chosen = matches!(&s.maps.open, Some(Open::Mod { id, .. }) if *id == m.id);
                let b = quick::button(s, m.id);
                let target = m.clone();
                list = list.child(
                    list_row(
                        u,
                        format!("maps-mod-{}", m.id),
                        chosen,
                        fill(tr("{}, by {}"), &[&m.name, &m.author]),
                        thumb(s, u, m.thumbnail_url.as_ref()),
                        m.name.clone(),
                        fill(tr("by {}  ·  {} likes"), &[&m.author, &m.likes.to_string()]),
                        quick::button_el(u, &b, format!("maps-install-{}", m.id), m.id, &m.name, quick::Size::Icon),
                    )
                    .on_press(move |st: &mut State| open_mod(st, &target)),
                );
            }
            list
        }
    }
}

fn panel(s: &State, u: &Ui) -> Option<El> {
    let open = s.maps.open.as_ref()?;
    let (play, author) = match open {
        Open::Mod { id, name, author, page_url } => {
            (play_for_mod(s, *id, name, page_url, None), Some(author.clone()))
        }
        Open::Slot(slot) => (play_for_slot(s, *slot), None),
    };
    let mut c = col()
        .key("maps-panel")
        .w(PANEL_W)
        .min_w(PANEL_W)
        .gap(12.0)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .items_start()
                .child(text(play.title.clone()).font(H1).color(u.ink).wrap().grow(1.0).min_w(0.0))
                .child(
                    u.icon_button(&icons::X, tr("Close"))
                        .key("maps-panel-close")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| st.maps.open = None),
                ),
        );
    if let Some(author) = author.filter(|a| !a.is_empty()) {
        c = c.child(text(fill(tr("by {}"), &[&author])).font(BODY).color(u.muted));
    }
    if matches!(open, Open::Mod { id, .. } if s.maps.describe_job.is_running() && !s.maps.described.contains_key(id))
        && play.map.is_none()
    {
        c = c.child(line("Reading the map's page for its name.", u.faint));
    }
    Some(c.child(how_to_play(s, u, "maps-panel", &play)))
}

fn launch_line(s: &State, u: &Ui) -> Option<El> {
    let current = s.game_root.as_deref().and_then(deadlock_core::servers::read_map_launch)?;
    Some(
        row()
            .key("maps-current")
            .w_full()
            .gap(8.0)
            .center_y()
            .child(icon(&icons::PIN, 14.0).color(u.muted).a11y_hidden())
            .child(text(fill(tr("Every launch starts {} until you remove it."), &[&current])).font(BODY).color(u.prose).grow(1.0).min_w(0.0).wrap())
            .child(
                u.action(tr("Remove"), ActionKind::Ghost)
                    .key("maps-clear-launch")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(clear_launch),
            ),
    )
}

pub fn view(s: &State, u: &Ui) -> El {
    let refresh = u
        .icon_button(&icons::REFRESH_CW, tr("Refresh"))
        .key("maps-refresh")
        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| {
            st.maps.catalog = None;
            on_open(st);
        });
    let head = u.page_head("", tr(Screen::Maps.title()), tr("Custom maps from GameBanana. Install one, then launch it and invite friends."), Some(refresh));
    let mut lists = col().grow(1.0).min_w(0.0).gap(20.0);
    if let Some(line) = launch_line(s, u) {
        lists = lists.child(line);
    }
    if let Some(installed) = installed_list(s, u) {
        lists = lists.child(installed);
    }
    lists = lists.child(catalog_list(s, u));
    let mut body = row().w_full().gap(24.0).items_start().child(lists);
    if let Some(panel) = panel(s, u) {
        body = body.child(panel);
    }
    col()
        .key("maps-scroll")
        .w_full()
        .h_full()
        .min_h(0.0)
        .scroll(ScrollSpec { stick_bottom: false })
        .child(
            col()
                .key("maps-page")
                .w_full()
                .px(PAGE_X)
                .pt(PAGE_TOP)
                .pb(32.0)
                .gap(16.0)
                .child(head)
                .child(crate::screens::host::listen::card(s, u, "maps"))
                .child(body),
        )
}

// --- scenes -------------------------------------------------------------------------------

fn fixture_mod(id: u64, name: &str, author: &str, likes: u64) -> BrowseMod {
    BrowseMod {
        id,
        name: name.to_string(),
        profile_url: format!("https://gamebanana.com/mods/{id}"),
        model: "Mod".to_string(),
        author: author.to_string(),
        category: "Maps".to_string(),
        subcategory: String::new(),
        version: None,
        updated_at: String::new(),
        likes,
        views: 0,
        has_files: true,
        thumbnail_url: None,
        added_ts: 0,
        updated_ts: 0,
        nsfw: false,
        audio_url: None,
        featured: false,
        comments: 0,
        author_id: 0,
        obsolete: false,
    }
}

fn stage(s: &mut State) {
    s.prefs.flags.insert(FLAG.to_string(), true);
    s.screen = Screen::Maps;
    s.maps.catalog = Some(Ok(vec![
        fixture_mod(660_392, "Street Ball", "quill", 96),
        fixture_mod(634_638, "Mindscape", "mossy", 41),
        fixture_mod(658_096, "Coal Town", "vexa", 17),
        fixture_mod(636_058, "Movement Practice", "rook", 63),
    ]));
    let addons = PathBuf::from("/games/deadlock/game/citadel/addons");
    s.maps.installed = vec![InstalledMap {
        slot: 7,
        name: "streetball".to_string(),
        title: Some("Street Ball".to_string()),
        vpk: addons.join("pak07_dir.vpk"),
    }];
    s.maps.scanned = true;
    s.addons.addons.push(crate::model::AddonRow {
        slot: 7,
        file: "pak07_dir.vpk".to_string(),
        size_bytes: 48_000_000,
        chunk_files: Vec::new(),
        entry_count: Some(212),
        enabled: true,
    });
    s.addons.scanned = true;
}

fn scene_base(s: &mut State) {
    s.load_demo();
    s.game_detected = true;
    s.addons.addons.retain(|a| a.slot != 7);
}

pub fn scene_list(s: &mut State) {
    scene_base(s);
    stage(s);
}

pub fn scene_installed(s: &mut State) {
    scene_list(s);
    s.maps.open = Some(Open::Slot(7));
}

pub fn scene_watching(s: &mut State) {
    scene_installed(s);
    s.maps.invite = Invite::Watching("streetball".to_string());
}

pub fn scene_invite(s: &mut State) {
    scene_installed(s);
    s.maps.invite = Invite::Ready { map: "streetball".to_string(), code: "[A:1:3695290372:49365]".to_string() };
}

pub fn scene_not_installed(s: &mut State) {
    scene_list(s);
    s.maps.described.insert(634_638, Some("ns_mindscape".to_string()));
    s.maps.open = Some(Open::Mod {
        id: 634_638,
        name: "Mindscape".to_string(),
        author: "mossy".to_string(),
        page_url: "https://gamebanana.com/mods/634638".to_string(),
    });
}

pub fn scene_unnamed(s: &mut State) {
    scene_list(s);
    s.maps.described.insert(658_096, None);
    s.maps.open = Some(Open::Mod {
        id: 658_096,
        name: "Coal Town".to_string(),
        author: "vexa".to_string(),
        page_url: "https://gamebanana.com/mods/658096".to_string(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_installed_map_reads_its_name_from_the_vpk() {
        let mut s = State::default();
        stage(&mut s);
        let play = play_for_slot(&s, 7);
        assert_eq!(play.map.as_deref(), Some("streetball"));
        assert!(play.enabled && play.vpk.is_some());
        assert_eq!(play.title, "Street Ball");
    }

    #[test]
    fn a_switched_off_map_cannot_launch() {
        let mut s = State::default();
        stage(&mut s);
        s.addons.addons[0].enabled = false;
        let play = play_for_slot(&s, 7);
        assert!(!play.enabled);
    }

    #[test]
    fn a_catalog_map_falls_back_to_its_description() {
        let mut s = State::default();
        stage(&mut s);
        let play = play_for_mod(&s, 634_638, "Mindscape", "", Some("f7 and type map ns_mindscape"));
        assert_eq!(play.map.as_deref(), Some("ns_mindscape"));
        assert_eq!(play.slot, None);
        let play = play_for_mod(&s, 658_096, "Coal Town", "", Some("No command here."));
        assert_eq!(play.map, None);
    }

    #[test]
    fn the_flag_is_off_by_default() {
        let s = State::default();
        assert!(!on(&s));
    }
}
