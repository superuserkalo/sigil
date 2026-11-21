//! Servers: Deadworks community servers, a filterable table, a detail panel, favorites, recent
//! servers and direct connect. The Maps tab lives in `screens::servers_maps`.
//!
//! The list and pings run on workers and the list refreshes every 15 seconds while the tab is
//! open. Joining is two steps: a read-only check (the server's A2S rules, its content plan,
//! gameinfo.gi and the Deadworks UI pack), then, after the user confirms what will be written,
//! `deadlock_core::servers::prepare_join` on a worker and Steam's `steam://connect`. Layout
//! after Grimoire's Servers page (MIT); filters respond to the columns' own sort order.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use deadlock_core::servers::{
    self as dw, BootstrapState, ContentKind, Continent, ItemAction, JoinCheck, JoinProgress,
    JoinReport, PasswordFilter, SavedServers, Server, ServerFilter, ServerList, ServerSort,
    ServerSortKey, SortDirection,
};
use rune::{
    Align, Color, El, Icon, Key, KeyEvent, ScrollSpec, Side, TextEdit, Tx, col, el, icon, row,
    spacer, text,
};

use crate::app::State;
use crate::icons;
use crate::jobs::Job;
use crate::model::Screen;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, MICRO, MONO, PAGE_TOP, PAGE_X, SMALL, Ui};


#[path = "servers_de.rs"]
mod de;

/// Join with invite (ADR 0015, second amendment): a Sigil invite installs the server's
/// GameBanana content, then joins.
#[path = "servers_invite.rs"]
pub mod invite;

/// Host a server on this PC (ADR 0015), the Host tab.
#[path = "servers_host/mod.rs"]
pub mod host;

/// The relay region picker, the Regions tab.
#[path = "servers_regions/mod.rs"]
pub mod regions;

/// The Host tab's words, from Sigil's tables (`settings/locales_host.rs`).
fn host_tr(text: &str) -> &str {
    crate::screens::settings::i18n::tr(text)
}

/// The UI text in the player's language: this page's German table first, then Sigil's.
pub(crate) fn tr(text: &str) -> &str {
    use crate::screens::settings::i18n::{self, Lang};
    if i18n::current() == Lang::German
        && let Some(de) = de::get(text)
    {
        return de;
    }
    i18n::tr(text)
}
/// Lucide `server` (ISC), for the sidebar.
pub static SERVER_ICON: Icon = Icon {
    name: "sigil-server",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><rect width="20" height="8" x="2" y="2" rx="2" ry="2"/><rect width="20" height="8" x="2" y="14" rx="2" ry="2"/><line x1="6" x2="6.01" y1="6" y2="6"/><line x1="6" x2="6.01" y1="18" y2="18"/></g>"#,
};

const PING_TIMEOUT: Duration = Duration::from_millis(1500);
/// How often the live list refreshes while the Servers tab is open.
const REFRESH_SECONDS: f64 = 15.0;

const COL_NAME: f32 = 0.0;
const COL_MAP: f32 = 156.0;
const COL_PLAYERS: f32 = 92.0;
const COL_PING: f32 = 64.0;
const COL_REGION: f32 = 72.0;
const COL_JOIN: f32 = 76.0;
const PANEL_W: f32 = 360.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Servers,
    Maps,
    Host,
    Regions,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    All,
    Favorites,
    Recent,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FilterMenu {
    Mode,
    Region,
    Map,
}

impl FilterMenu {
    fn anchor(self) -> &'static str {
        match self {
            FilterMenu::Mode => "servers-mode",
            FilterMenu::Region => "servers-region",
            FilterMenu::Map => "servers-map",
        }
    }
}

pub struct View {
    pub list: Option<Result<ServerList, String>>,
    pub pings: HashMap<String, Option<u32>>,
    pub query: TextEdit,
    pub filters: ServerFilter,
    pub sort: Option<ServerSort>,
    pub scope: Scope,
    pub tab: Tab,
    pub menu: Option<FilterMenu>,
    pub selected: Option<Server>,
    pub saved: SavedServers,
    pub list_job: Job<ServerList>,
    pub ping_job: Job<HashMap<String, Option<u32>>>,
    pub dialog: Option<Dialog>,
    pub password: TextEdit,
    pub direct: TextEdit,
    pub check_job: Job<JoinCheck>,
    pub join_job: Job<JoinDone>,
    pub progress: Arc<Mutex<JoinProgress>>,
    /// The join progress the window last drew, so a poll commits only when it moved.
    shown_progress: JoinProgress,
    /// The check the user confirmed, kept for an "Install anyway" retry.
    pub last_check: Option<Box<JoinCheck>>,
    pub maps: crate::screens::servers_maps::MapsState,
    pub host: host::HostState,
    /// The relay region picker.
    pub regions: regions::RegionsState,
    /// Join with invite: the paste field and where the join is.
    pub invite: invite::InviteState,
    /// The clock time the next automatic refresh is due.
    pub next_refresh: Option<f64>,
    pub started: bool,
}

impl Default for View {
    fn default() -> Self {
        View {
            list: None,
            pings: HashMap::new(),
            query: TextEdit::new(""),
            filters: ServerFilter::default(),
            sort: None,
            scope: Scope::All,
            tab: Tab::Servers,
            menu: None,
            selected: None,
            saved: SavedServers::default(),
            list_job: Job::default(),
            ping_job: Job::default(),
            dialog: None,
            password: TextEdit::new(""),
            direct: TextEdit::new(""),
            check_job: Job::default(),
            join_job: Job::default(),
            progress: Arc::new(Mutex::new(JoinProgress::default())),
            shown_progress: JoinProgress::default(),
            last_check: None,
            maps: crate::screens::servers_maps::MapsState::default(),
            host: host::HostState::default(),
            regions: regions::RegionsState::default(),
            invite: invite::InviteState::default(),
            next_refresh: None,
            started: false,
        }
    }
}

#[derive(Clone)]
pub enum Dialog {
    Join { server: Box<Server>, stage: Stage },
    Direct,
    Invite,
}

#[derive(Clone)]
pub enum Stage {
    Checking,
    Ready(Box<JoinCheck>),
    Working,
    /// The host serves a different build of these files than the server lists.
    Mismatch {
        check: Box<JoinCheck>,
        files: Vec<String>,
    },
    Done(String),
    Failed(String),
}

pub enum JoinDone {
    Connected,
    Mismatch(JoinReport),
}

pub fn loaded(s: &State) -> bool {
    s.servers.list.is_some() || s.servers.list_job.is_running()
}

pub fn start_list(st: &mut State) {
    if st.demo || rune::is_headless() {
        return;
    }
    let wake = st.wake.clone();
    st.servers
        .list_job
        .start(wake, || dw::fetch_servers(dw::DEADWORKS_API_BASE).map_err(|e| e.to_string()));
}

fn start_pings(st: &mut State) {
    let Some(Ok(list)) = &st.servers.list else { return };
    let targets: Vec<(String, std::net::SocketAddr)> = list
        .servers
        .iter()
        .filter_map(|s| s.socket_addr().map(|a| (s.id.clone(), a)))
        .collect();
    let wake = st.wake.clone();
    st.servers
        .ping_job
        .start(wake, move || Ok(dw::ping_many(targets, PING_TIMEOUT)));
}

fn refresh(st: &mut State) {
    if st.servers.list_job.is_running() {
        return;
    }
    st.servers.pings.clear();
    start_list(st);
}

/// The 15 second heartbeat. Only the visible Servers tab asks for the next one.
fn schedule_refresh(st: &mut State, now: f64, tx: &mut Tx) {
    if st.screen != Screen::Servers || st.servers.tab != Tab::Servers {
        st.servers.next_refresh = None;
        return;
    }
    let due = st.servers.next_refresh.unwrap_or(now);
    if now >= due {
        if !st.servers.list_job.is_running() && !st.demo && !rune::is_headless() {
            refresh(st);
        }
        st.servers.next_refresh = Some(now + REFRESH_SECONDS);
    }
    tx.wake_at(
        st.servers
            .next_refresh
            .unwrap_or(now + REFRESH_SECONDS),
    );
}

fn open_join(st: &mut State, server: Server) {
    if server.is_full() || !st.game_detected {
        return;
    }
    st.servers.password = TextEdit::new("");
    st.servers.dialog = Some(Dialog::Join {
        server: Box::new(server.clone()),
        stage: Stage::Checking,
    });
    if st.demo || rune::is_headless() {
        return;
    }
    let Some(root) = st.game_root.clone() else { return };
    let wake = st.wake.clone();
    st.servers.check_job.start(wake, move || {
        dw::check_join(&root, &server.address, dw::DEADWORKS_API_BASE).map_err(|e| e.to_string())
    });
}

fn set_stage(st: &mut State, next: Stage) {
    if let Some(Dialog::Join { stage, .. }) = &mut st.servers.dialog {
        *stage = next;
    }
}

fn start_join(st: &mut State, check: JoinCheck, accept: Vec<String>) {
    let Some(Dialog::Join { server, .. }) = st.servers.dialog.clone() else {
        return;
    };
    let address = server.address.clone();
    let password = server
        .password
        .then(|| st.servers.password.text.trim().to_string())
        .filter(|p| !p.is_empty());
    set_stage(st, Stage::Working);
    if st.demo || rune::is_headless() {
        return;
    }
    let Some(root) = st.game_root.clone() else { return };
    let progress = st.servers.progress.clone();
    *progress.lock().unwrap_or_else(|e| e.into_inner()) = JoinProgress::default();
    let workspace = crate::data::workspace_dir();
    let recent = Server {
        address: address.clone(),
        ..(*server).clone()
    };
    let wake = st.wake.clone();
    let tick = wake.clone();
    st.servers.join_job.start(wake, move || {
        let report = dw::prepare_join(&root, &check, &accept, &|p| {
            *progress.lock().unwrap_or_else(|e| e.into_inner()) = p;
            if let Some(tick) = &tick {
                tick();
            }
        })
        .map_err(|e| e.to_string())?;
        if !report.mismatched.is_empty() {
            return Ok(JoinDone::Mismatch(report));
        }
        dw::open_connect(&address, password.as_deref())
            .map(|_| JoinDone::Connected)
            .map_err(|e| e.to_string())?;
        let _ = dw::mark_recent(&workspace, &recent);
        Ok(JoinDone::Connected)
    });
}

/// Take finished work. Returns true when the screen changed.
pub fn poll(st: &mut State, now: f64, tx: &mut Tx) -> bool {
    let mut changed = false;
    if !st.servers.started {
        st.servers.started = true;
        st.servers.saved = dw::read_saved_servers(&crate::data::workspace_dir());
        changed = true;
    }
    if st.screen == Screen::Servers
        && st.servers.tab == Tab::Maps
        && st.servers.maps.entries.is_none()
        && !st.servers.maps.job.is_running()
    {
        crate::screens::servers_maps::start_load(st);
    }
    if let Some(result) = st.servers.list_job.try_take() {
        let ok = result.is_ok();
        if let Ok(list) = &result
            && let Some(map) = &st.servers.filters.map
            && !list.servers.iter().any(|s| &s.map == map)
        {
            st.servers.filters.map = None;
        }
        st.servers.list = Some(result);
        if ok {
            start_pings(st);
        }
        changed = true;
    }
    if let Some(Ok(pings)) = st.servers.ping_job.try_take() {
        st.servers.pings = pings;
        changed = true;
    }
    if let Some(result) = st.servers.check_job.try_take() {
        set_stage(
            st,
            match result {
                Ok(check) => Stage::Ready(Box::new(check)),
                Err(error) => Stage::Failed(error),
            },
        );
        changed = true;
    }
    if let Some(result) = st.servers.join_job.try_take() {
        let stage = match result {
            Ok(JoinDone::Connected) => {
                let name = match &st.servers.dialog {
                    Some(Dialog::Join { server, .. }) => server.name.clone(),
                    _ => "the server".to_string(),
                };
                Stage::Done(format!("Steam is opening {name}."))
            }
            Ok(JoinDone::Mismatch(report)) => match &st.servers.dialog {
                Some(Dialog::Join {
                    stage: Stage::Working,
                    ..
                }) => {
                    let check = st.servers.last_check.take();
                    match check {
                        // A Sigil host names its files by hash: a mismatch there means the file
                        // was changed, so there is no "take it anyway".
                        Some(check) if check.sigil_host => Stage::Failed(crate::screens::settings::i18n::fill(
                            crate::screens::settings::i18n::tr(
                                "The host's copy of {} does not match what the server advertises, so Sigil refused it. Ask the host to restart the server in Sigil, then join again.",
                            ),
                            &[&report
                                .mismatched
                                .iter()
                                .map(|(key, _)| key.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")],
                        )),
                        Some(check) => Stage::Mismatch {
                            check,
                            files: report.mismatched.into_iter().map(|(key, _)| key).collect(),
                        },
                        None => Stage::Failed("The join was interrupted.".to_string()),
                    }
                }
                _ => Stage::Failed("The join was interrupted.".to_string()),
            },
            Err(error) => Stage::Failed(error),
        };
        set_stage(st, stage);
        changed = true;
    }
    // The worker wakes the window on each step of a join; returning true on every poll
    // instead redrew the whole window at display rate until the join ended.
    if st.servers.join_job.is_running() {
        let now = st.servers.progress.lock().map(|p| p.clone()).unwrap_or_default();
        if now != st.servers.shown_progress {
            st.servers.shown_progress = now;
            changed = true;
        }
    }
    changed |= invite::advance(st);
    changed |= crate::screens::servers_maps::poll(st);
    changed |= host::poll(st, now, tx);
    changed |= regions::poll(st, now, tx);
    schedule_refresh(st, now, tx);
    changed
}

// ---------------------------------------------------------------------------------------------
// Filtering and sorting
// ---------------------------------------------------------------------------------------------

fn active_labels(filter: &ServerFilter) -> Vec<(String, FilterChipKind)> {
    let mut out = Vec::new();
    if !filter.query.trim().is_empty() {
        out.push((filter.query.trim().to_string(), FilterChipKind::Search));
    }
    if filter.has_players {
        out.push((tr("With players").to_string(), FilterChipKind::Players));
    }
    if filter.has_space {
        out.push((tr("Has space").to_string(), FilterChipKind::Space));
    }
    match filter.password {
        PasswordFilter::Open => out.push((tr("Open").to_string(), FilterChipKind::Password)),
        PasswordFilter::Password => out.push((tr("Password").to_string(), FilterChipKind::Password)),
        PasswordFilter::All => {}
    }
    if let Some(mode) = &filter.mode {
        out.push((mode.clone(), FilterChipKind::Mode));
    }
    if let Some(country) = &filter.country {
        out.push((dw::region_label(country), FilterChipKind::Region));
    } else if let Some(continent) = filter.continent {
        out.push((continent.label().to_string(), FilterChipKind::Region));
    }
    if let Some(map) = &filter.map {
        out.push((map.clone(), FilterChipKind::Map));
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FilterChipKind {
    Search,
    Players,
    Space,
    Password,
    Mode,
    Region,
    Map,
}

fn clear_chip(filters: &mut ServerFilter, kind: FilterChipKind) {
    match kind {
        FilterChipKind::Search => filters.query.clear(),
        FilterChipKind::Players => filters.has_players = false,
        FilterChipKind::Space => filters.has_space = false,
        FilterChipKind::Password => filters.password = PasswordFilter::All,
        FilterChipKind::Mode => filters.mode = None,
        FilterChipKind::Region => {
            filters.continent = None;
            filters.country = None;
        }
        FilterChipKind::Map => filters.map = None,
    }
}

/// The servers the page shows: the live list, or the favorites or recent addresses.
fn scoped(s: &State) -> Vec<Server> {
    let live: Vec<Server> = match &s.servers.list {
        Some(Ok(list)) => list.servers.clone(),
        _ => Vec::new(),
    };
    match s.servers.scope {
        Scope::All => live,
        Scope::Favorites => saved_servers(&s.servers.saved.favorites, &live),
        Scope::Recent => saved_servers(&s.servers.saved.recent, &live),
    }
}

fn saved_servers(saved: &[dw::SavedServer], live: &[Server]) -> Vec<Server> {
    saved
        .iter()
        .map(|entry| {
            live.iter()
                .find(|server| server.address == entry.address)
                .cloned()
                .unwrap_or_else(|| direct_server(&entry.address, &entry.name, &entry.map))
        })
        .collect()
}

fn direct_server(address: &str, name: &str, map: &str) -> Server {
    Server {
        id: address.to_string(),
        name: if name.trim().is_empty() {
            address.to_string()
        } else {
            name.to_string()
        },
        address: address.to_string(),
        map: map.to_string(),
        players: 0,
        max_players: 0,
        bots: 0,
        country: String::new(),
        version: String::new(),
        game_mode: String::new(),
        registered: false,
        password: false,
        has_addons: false,
        roster: Vec::new(),
        content_hint: Vec::new(),
    }
}

fn visible(s: &State) -> Vec<Server> {
    let mut filters = s.servers.filters.clone();
    filters.query = s.servers.query.text.clone();
    let shown: Vec<Server> = scoped(s)
        .into_iter()
        .filter(|server| filters.matches(server))
        .collect();
    let refs: Vec<&Server> = shown.iter().collect();
    let sorted: Vec<&Server> = dw::sorted_servers(refs, s.servers.sort, &s.servers.pings);
    sorted.into_iter().cloned().collect()
}

// ---------------------------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------------------------

pub fn view(s: &State, u: &Ui) -> El {
    let loading = s.servers.list_job.is_running();
    let mut actions = row().gap(8.0).center_y();
    if s.servers.tab == Tab::Servers {
        actions = actions
            .child(
                u.icon_action(&icons::LOG_IN, tr("Connect"), ActionKind::Ghost)
                    .key("servers-connect")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| open_direct(st)),
            )
            .child(
                u.icon_action(&icons::USER, tr("Join with invite"), ActionKind::Ghost)
                    .key("servers-invite")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| invite::open(st, "")),
            )
            .child(
                u.icon_action(&SERVER_ICON, tr("Host a server"), ActionKind::Ghost)
                    .key("servers-host")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| {
                        st.servers.tab = Tab::Host;
                        st.servers.menu = None;
                    }),
            );
    }
    if s.servers.tab == Tab::Host {
        if s.servers.host.windows {
            actions = actions.child(
                u.icon_action(&icons::REFRESH_CW, host_tr("Check again"), ActionKind::Secondary)
                    .key("servers-refresh")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(host::recheck),
            );
        }
    } else if s.servers.tab != Tab::Regions {
        // Regions has its own Refresh beside the list.
        actions = actions.child(
            u.icon_action(&icons::REFRESH_CW, tr("Refresh"), ActionKind::Secondary)
                .key("servers-refresh")
                .opacity(if loading { 0.5 } else { 1.0 })
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| {
                    if st.servers.tab == Tab::Maps {
                        st.servers.maps.entries = None;
                        crate::screens::servers_maps::start_load(st);
                        st.servers.maps.error = None;
                    } else {
                        refresh(st);
                    }
                }),
        );
    }

    let _detail = match &s.servers.list {
        _ if s.servers.tab == Tab::Host || s.servers.tab == Tab::Regions => String::new(),
        Some(Ok(list)) => {
            let playing: u32 = list.servers.iter().map(|sv| sv.players).sum();
            format!(
                "{} server{}, {playing} playing",
                list.servers.len(),
                if list.servers.len() == 1 { "" } else { "s" }
            )
        }
        _ => String::new(),
    };

    let mut page = col()
        .key("servers-page")
        .w_full()
        .px(PAGE_X)
        .pt(PAGE_TOP)
        .pb(40.0)
        .gap(16.0)
        .child(u.page_toolbar(
            u.page_title(tr("Servers")),
            Vec::new(),
            Some(crate::app::page_search(s, u)),
            vec![actions],
        ))
        .child(tabs(s, u));

    if s.servers.tab == Tab::Maps || s.servers.tab == Tab::Host || s.servers.tab == Tab::Regions {
        page = page.child(if s.servers.tab == Tab::Host {
            host::view(s, u)
        } else if s.servers.tab == Tab::Regions {
            regions::view(s, u)
        } else {
            crate::screens::servers_maps::view(s, u)
        });
        return col()
            .key("servers-scroll")
            .w_full()
            .h_full()
            .min_h(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(page);
    }

    let mut body = col().w_full().gap(16.0);
    if !s.game_detected {
        body = body.child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(
                    text("Set the game folder in Settings to join a server.")
                        .font(BODY)
                        .color(u.muted),
                )
                .child(
                    u.action("Open Settings", ActionKind::Ghost)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| st.go(Screen::Settings)),
                ),
        );
    }
    if let Some(Ok(list)) = &s.servers.list
        && list.steam_degraded
    {
        body = body.child(
            text("Steam's server list is down. Showing servers registered with Deadworks.")
                .font(SMALL)
                .color(u.amber),
        );
    }
    body = body.child(filters(s, u));
    if s.servers.filters.active_count() > 0 || !s.servers.query.text.trim().is_empty() {
        body = body.child(chips(s, u));
    }

    let shown = visible(s);
    let table = match &s.servers.list {
        None => u.empty_state("", tr("Loading servers"), "", None),
        Some(Err(error)) if s.servers.scope == Scope::All => u.empty_state(
            "",
            "Deadworks could not be reached",
            error,
            Some(
                u.action(tr("Try again"), ActionKind::Secondary)
                    .key("servers-retry")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(refresh),
            ),
        ),
        _ if s.servers.scope != Scope::All && scoped(s).is_empty() => match s.servers.scope {
            Scope::Favorites => u.empty_state(
                "",
                tr("No favorites yet"),
                "Pick a server and press the heart to keep it here.",
                None,
            ),
            _ => u.empty_state(
                "",
                tr("No recent servers"),
                "Servers you join show up here.",
                None,
            ),
        },
        _ if shown.is_empty() => u.empty_state(
            "",
            tr("Nothing matches"),
            "Clear a filter or search for another name.",
            Some(
                u.action(tr("Clear filters"), ActionKind::Secondary)
                    .key("servers-clear")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(clear_filters),
            ),
        ),
        _ => table(s, u, &shown),
    };

    let mut content = row().w_full().gap(20.0).items_start().child(table);
    if let Some(server) = &s.servers.selected {
        content = content.child(detail_panel(s, u, server));
    }
    body = body.child(content);
    page = page.child(body);

    col()
        .key("servers-scroll")
        .w_full()
        .h_full()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(page)
}

fn clear_filters(st: &mut State) {
    st.servers.filters = ServerFilter::default();
    st.servers.query = TextEdit::new("");
}

fn tabs(s: &State, u: &Ui) -> El {
    let servers = tab_button(u, "servers-tab-servers", tr("Servers"), s.servers.tab == Tab::Servers);
    let maps = tab_button(u, "servers-tab-maps", tr("Maps"), s.servers.tab == Tab::Maps);
    let hosting = tab_button(u, "servers-tab-host", host_tr("Host"), s.servers.tab == Tab::Host);
    let relay = tab_button(u, "servers-tab-regions", host_tr("Regions"), s.servers.tab == Tab::Regions);
    let bar = row()
        .key("servers-tabs")
        .w_full()
        .gap(4.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(servers.on_press(|st: &mut State| st.servers.tab = Tab::Servers))
        .child(maps.on_press(|st: &mut State| st.servers.tab = Tab::Maps))
        .child(hosting.on_press(|st: &mut State| st.servers.tab = Tab::Host))
        .child(relay.on_press(|st: &mut State| st.servers.tab = Tab::Regions))
        .child(spacer());
    if s.servers.tab == Tab::Host {
        return bar.child(host::tab_status(s, u));
    }
    if s.servers.tab == Tab::Regions {
        return bar;
    }
    bar
        .child(scope_button(
            u,
            "servers-scope-all",
            tr("All servers"),
            s.servers.scope == Scope::All,
            Scope::All,
        ))
        .child(scope_button(
            u,
            "servers-scope-favorites",
            &format!("Favorites ({})", s.servers.saved.favorites.len()),
            s.servers.scope == Scope::Favorites,
            Scope::Favorites,
        ))
        .child(scope_button(
            u,
            "servers-scope-recent",
            &format!("Recent ({})", s.servers.saved.recent.len()),
            s.servers.scope == Scope::Recent,
            Scope::Recent,
        ))
}

fn tab_button(u: &Ui, key: &str, label: &str, on: bool) -> El {
    row()
        .key(key.to_string())
        .h(36.0)
        .px(2.0)
        .pr(16.0)
        .gap(0.0)
        .center_y()
        .focusable(true)
        .role("tab")
        .checked(on)
        .label(label.to_string())
        .cursor(rune::Cursor::Pointer)
        .child(
            col()
                .gap(4.0)
                .child(
                    text(label.to_string())
                        .font(BODY_MED)
                        .color(if on { u.ink } else { u.muted })
                        .a11y_hidden(),
                )
                .child(
                    el()
                        .h(2.0)
                        .w_full()
                        .radius(crate::ui::r(1.0))
                        .fill(if on { u.ink } else { Color::TRANSPARENT }),
                ),
        )
}

fn scope_button(u: &Ui, key: &str, label: &str, on: bool, scope: Scope) -> El {
    row()
        .key(key.to_string())
        .h(28.0)
        .px(10.0)
        .gap(6.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .fill(if on { u.selected } else { Color::TRANSPARENT })
        .hover(u.hover)
        .focusable(true)
        .role("button")
        .checked(on)
        .label(label.to_string())
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            st.servers.scope = scope;
            st.servers.selected = None;
        })
        .child(
            text(label.to_string())
                .font(SMALL)
                .color(if on { u.ink } else { u.muted })
                .a11y_hidden(),
        )
}

/// One filter row: presets, the password segment and the three dropdowns.
fn filters(s: &State, u: &Ui) -> El {
    let f = &s.servers.filters;
    let joinable = f.has_players && f.has_space && f.password == PasswordFilter::Open;
    let mut top = row()
        .w_full()
        .gap(4.0)
        .center_y()
        .child(toggle_chip(
            u,
            "servers-joinable",
            tr("Joinable"),
            joinable,
            |st| {
                let preset = st.servers.filters.has_players
                    && st.servers.filters.has_space
                    && st.servers.filters.password == PasswordFilter::Open;
                if preset {
                    st.servers.filters.has_players = false;
                    st.servers.filters.has_space = false;
                    st.servers.filters.password = PasswordFilter::All;
                } else {
                    st.servers.filters.has_players = true;
                    st.servers.filters.has_space = true;
                    st.servers.filters.password = PasswordFilter::Open;
                }
            },
        ))
        .child(el().w(8.0))
        .child(toggle_chip(
            u,
            "servers-has-players",
            tr("With players"),
            f.has_players,
            |st| st.servers.filters.has_players = !st.servers.filters.has_players,
        ))
        .child(toggle_chip(
            u,
            "servers-has-space",
            tr("Has space"),
            f.has_space,
            |st| st.servers.filters.has_space = !st.servers.filters.has_space,
        ))
        .child(el().w(8.0))
        .child(password_segment(u, f.password))
        .child(spacer())
        .child(dropdown(
            u,
            FilterMenu::Mode,
            tr("Mode"),
            f.mode.as_deref().unwrap_or(tr("All modes")),
            f.mode.is_some(),
            s.servers.menu == Some(FilterMenu::Mode),
        ))
        .child(dropdown(
            u,
            FilterMenu::Region,
            tr("Region"),
            &f.country
                .as_ref()
                .map(|country| dw::region_label(country))
                .unwrap_or_else(|| {
                    f.continent
                        .map(|continent| continent.label().to_string())
                        .unwrap_or_else(|| tr("Every region").to_string())
                }),
            f.continent.is_some() || f.country.is_some(),
            s.servers.menu == Some(FilterMenu::Region),
        ))
        .child(dropdown(
            u,
            FilterMenu::Map,
            tr("Map"),
            f.map.as_deref().unwrap_or(tr("All maps")),
            f.map.is_some(),
            s.servers.menu == Some(FilterMenu::Map),
        ));
    if f.active_count() > 0 {
        top = top.child(
            u.action(tr("Clear"), ActionKind::Ghost)
                .key("servers-clear-filters")
                .cursor(rune::Cursor::Pointer)
                .on_press(clear_filters),
        );
    }
    top
}

fn toggle_chip(u: &Ui, key: &str, label: &str, on: bool, press: impl Fn(&mut State) + 'static) -> El {
    row()
        .key(key.to_string())
        .h(32.0)
        .px(10.0)
        .gap(6.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .fill(if on { u.selected } else { Color::TRANSPARENT })
        .hover(u.hover)
        .focusable(true)
        .role("switch")
        .checked(on)
        .label(label.to_string())
        .cursor(rune::Cursor::Pointer)
        .on_press(press)
        .child(
            text(label.to_string())
                .font(SMALL)
                .color(if on { u.ink } else { u.muted })
                .a11y_hidden(),
        )
}

fn password_segment(u: &Ui, current: PasswordFilter) -> El {
    let mut segment = row()
        .key("servers-password-filter")
        .h(32.0)
        .px(2.0)
        .py(2.0)
        .gap(2.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .fill(u.well)
        .border(1.0, u.hair);
    for option in PasswordFilter::ALL {
        let on = current == option;
        segment = segment.child(
            row()
                .key(format!("servers-password-{}", option.id()))
                .h(28.0)
                .px(10.0)
                .center_y()
                .radius(crate::ui::r(6.0))
                .fill(if on { u.selected } else { Color::TRANSPARENT })
                .focusable(true)
                .role("button")
                .checked(on)
                .label(option.label().to_string())
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| st.servers.filters.password = option)
                .child(
                    text(option.label().to_string())
                        .font(SMALL)
                        .color(if on { u.ink } else { u.muted })
                        .a11y_hidden(),
                ),
        );
    }
    segment
}

fn dropdown(
    u: &Ui,
    menu: FilterMenu,
    label: &str,
    value: &str,
    active: bool,
    open: bool,
) -> El {
    row()
        .key(menu.anchor().to_string())
        .h(32.0)
        .px(10.0)
        .gap(6.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .fill(if open { u.selected } else { Color::TRANSPARENT })
        .hover(u.hover)
        .focusable(true)
        .role("button")
        .label(format!("{label}: {value}"))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            st.servers.menu = if st.servers.menu == Some(menu) {
                None
            } else {
                Some(menu)
            }
        })
        .child(
            text(value.to_string())
                .font(SMALL)
                .color(if active { u.ink } else { u.muted })
                .a11y_hidden(),
        )
        .child(icon(&icons::CHEVRON_DOWN, 12.0).color(u.faint).a11y_hidden())
}

fn chips(s: &State, u: &Ui) -> El {
    let mut filters = s.servers.filters.clone();
    filters.query = s.servers.query.text.clone();
    let labels = active_labels(&filters);
    let count = labels.len();
    let mut bar = row()
        .key("servers-chips")
        .w_full()
        .gap(6.0)
        .center_y()
        .child(text("Active").font(SMALL).color(u.faint));
    for (label, kind) in labels {
        bar = bar.child(
            row()
                .key(format!("servers-chip-{:?}", kind))
                .h(26.0)
                .px(8.0)
                .gap(6.0)
                .center_y()
                .radius(crate::ui::r(6.0))
                .fill(u.well)
                .border(1.0, u.hair)
                .child(text(truncate(&label, 28)).font(SMALL).color(u.prose))
                .child(
                    row()
                        .key(format!("servers-chip-x-{:?}", kind))
                        .size(16.0, 16.0)
                        .center()
                        .radius(crate::ui::r(4.0))
                        .hover(u.hover)
                        .focusable(true)
                        .role("button")
                        .label(format!("Remove {label}"))
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| {
                            clear_chip(&mut st.servers.filters, kind);
                            if kind == FilterChipKind::Search {
                                st.servers.query = TextEdit::new("");
                            }
                        })
                        .child(icon(&icons::X, 10.0).color(u.muted).a11y_hidden()),
                ),
        );
    }
    bar.child(spacer()).child(
        u.action(&format!("Clear ({count})"), ActionKind::Ghost)
            .key("servers-clear-all")
            .cursor(rune::Cursor::Pointer)
            .on_press(clear_filters),
    )
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let mut out: String = value.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn head(u: &Ui, key: ServerSortKey, width: f32, _end: bool, sort: Option<ServerSort>) -> El {
    let label = key.label();
    let direction = sort.filter(|sort| sort.key == key).map(|sort| sort.direction);
    let mut cell = row()
        .key(format!("servers-head-{}", key.id()))
        .h(32.0)
        .gap(4.0)
        .center_y()
        .hover(u.hover)
        .focusable(true)
        .role("button")
        .label(format!("Sort by {label}"))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            st.servers.sort = dw::next_sort(st.servers.sort, key);
            st.servers.selected = None;
        })
        .child(
            text(label)
                .font(SMALL)
                .color(if direction.is_some() { u.ink } else { u.faint })
                .a11y_hidden(),
        );
    if let Some(direction) = direction {
        cell = cell.child(
            text(if direction == SortDirection::Asc { "↑" } else { "↓" })
                .font(SMALL)
                .color(u.muted)
                .a11y_hidden(),
        );
    }
    if width > 0.0 {
        cell.w(width)
    } else {
        cell.grow(1.0).min_w(0.0)
    }
}

fn table(s: &State, u: &Ui, shown: &[Server]) -> El {
    let sort = s.servers.sort;
    let header = row()
        .key("servers-head")
        .w_full()
        .h(32.0)
        .px(8.0)
        .gap(12.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(el().w(16.0))
        .child(head(u, ServerSortKey::Name, COL_NAME, false, sort))
        .child(head(u, ServerSortKey::Map, COL_MAP, false, sort))
        .child(head(u, ServerSortKey::Players, COL_PLAYERS, true, sort))
        .child(head(u, ServerSortKey::Ping, COL_PING, true, sort))
        .child(head(u, ServerSortKey::Region, COL_REGION, true, sort))
        .child(el().w(COL_JOIN));
    let mut rows = col().w_full().grow(1.0).min_w(0.0).child(header);
    for server in shown {
        rows = rows.child(server_row(s, u, server));
    }
    rows
}

fn ping_label(s: &State, server: &Server) -> (String, Option<u32>) {
    match s.servers.pings.get(&server.id) {
        Some(Some(ms)) => (format!("{ms} ms"), Some(*ms)),
        Some(None) => ("-".to_string(), None),
        None if s.servers.ping_job.is_running() => ("…".to_string(), None),
        None => ("-".to_string(), None),
    }
}

fn server_meta(server: &Server) -> String {
    let mut parts = Vec::new();
    if !server.version.is_empty() {
        parts.push(format!("Deadworks {}", server.version));
    }
    if server.has_addons || !server.content_hint.is_empty() {
        parts.push("Custom content".to_string());
    }
    if server.bots > 0 {
        parts.push(format!("{} bots", server.bots));
    }
    let mode = server.mode_label();
    if mode != "Standard" && mode != "Unknown" {
        parts.insert(0, mode);
    }
    parts.join("  ·  ")
}

fn server_row(s: &State, u: &Ui, server: &Server) -> El {
    let full = server.is_full();
    let can_join = s.game_detected && !full;
    let (ping, ms) = ping_label(s, server);
    let dot = if full {
        u.amber
    } else if server.players > 0 {
        u.teal
    } else {
        Color::WHITE.alpha(0.22)
    };
    let mut name_line = row().gap(6.0).center_y().min_w(0.0).child(
        text(server.name.clone())
            .font(BODY_MED)
            .color(u.ink)
            .truncate(),
    );
    if server.password {
        name_line = name_line.child(icon(&icons::LOCK, 12.0).color(u.faint).a11y_hidden());
    }
    let favorite = dw::is_favorite(&s.servers.saved, &server.address);
    if favorite {
        name_line = name_line.child(icon(&icons::HEART, 12.0).color(u.muted).a11y_hidden());
    }
    let meta = server_meta(server);
    let mut name_col = col().grow(1.0).min_w(0.0).gap(2.0).child(name_line);
    if !meta.is_empty() {
        name_col = name_col.child(text(meta).font(SMALL).color(u.faint).truncate());
    }
    let select = server.clone();
    let join_target = server.clone();
    let chosen = s
        .servers
        .selected
        .as_ref()
        .is_some_and(|selected| selected.address == server.address);
    row()
        .key(format!("server-{}", server.id))
        .w_full()
        .h(56.0)
        .px(8.0)
        .gap(12.0)
        .center_y()
        .radius(crate::ui::r(6.0))
        .fill(if chosen { u.selected } else { Color::TRANSPARENT })
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .hover(u.hover)
        .focusable(true)
        .role("button")
        .label(format!("{}, {} of {} players", server.name, server.players, server.max_players))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            st.servers.selected = if st
                .servers
                .selected
                .as_ref()
                .is_some_and(|selected| selected.address == select.address)
            {
                None
            } else {
                Some(select.clone())
            }
        })
        .child(el().size(16.0, 16.0).center().child(el().size(8.0, 8.0).radius(4.0).fill(dot).a11y_hidden()))
        .child(name_col)
        .child(
            text(server.map_label().to_string())
                .font(BODY)
                .color(u.prose)
                .w(COL_MAP)
                .truncate(),
        )
        .child(capacity(u, server))
        .child(
            text(ping)
                .font(BODY)
                .color(match ms {
                    Some(ms) if ms >= 150 => u.amber,
                    Some(_) => u.prose,
                    None => u.faint,
                })
                .w(COL_PING)
                .align(Align::End),
        )
        .child(
            text(dw::region_label(&server.country))
                .font(SMALL)
                .color(u.muted)
                .w(COL_REGION),
        )
        .child(
            u.icon_action(&icons::PLAY, tr("Join"), ActionKind::Secondary)
                .key(format!("server-join-{}", server.id))
                .w(COL_JOIN)
                .opacity(if can_join { 1.0 } else { 0.4 })
                .disabled(!can_join)
                .help(if full {
                    "This server is full"
                } else if !s.game_detected {
                    "Set the game folder in Settings first"
                } else {
                    tr("Join")
                })
                .cursor(if can_join {
                    rune::Cursor::Pointer
                } else {
                    rune::Cursor::Default
                })
                .on_press(move |st: &mut State| open_join(st, join_target.clone())),
        )
}

fn capacity(u: &Ui, server: &Server) -> El {
    let max = server.max_players.max(1);
    let fraction = (server.players as f32 / max as f32).clamp(0.0, 1.0);
    let full = server.is_full();
    col()
        .key(format!("server-capacity-{}", server.id))
        .w(COL_PLAYERS)
        .gap(3.0)
        .child(
            text(if server.max_players == 0 {
                format!("{}", server.players)
            } else {
                format!("{}/{}", server.players, server.max_players)
            })
            .font(BODY)
            .color(if full { u.amber } else { u.prose }),
        )
        .child(
            row()
                .w(56.0)
                .h(3.0)
                .radius(crate::ui::r(2.0))
                .fill(Color::WHITE.alpha(0.08))
                .child(
                    el()
                        .grow(fraction.max(0.02))
                        .h(3.0)
                        .radius(crate::ui::r(2.0))
                        .fill(if full { u.amber } else { u.muted }),
                )
                .child(el().grow((1.0 - fraction).max(0.02)).h(3.0)),
        )
}

fn detail_panel(s: &State, u: &Ui, server: &Server) -> El {
    let (ping, _) = ping_label(s, server);
    let mut body = col()
        .key("server-panel")
        .w(PANEL_W)
        .min_w(PANEL_W)
        .gap(10.0)
        .pt(4.0);
    let favorite = dw::is_favorite(&s.servers.saved, &server.address);
    let favorite_target = server.clone();
    body = body.child(
        row()
            .w_full()
            .gap(8.0)
            .items_start()
            .child(
                text(server.name.clone())
                    .font(H1)
                    .color(u.ink)
                    .grow(1.0)
                    .min_w(0.0)
                    .wrap(),
            )
            .child(
                u.icon_button(&icons::HEART, if favorite { tr("Remove favorite") } else { tr("Keep") })
                    .key("server-favorite")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        let workspace = crate::data::workspace_dir();
                        match dw::toggle_favorite(&workspace, &favorite_target) {
                            Ok(on) => {
                                st.servers.saved = dw::read_saved_servers(&workspace);
                                crate::notify::push(
                                    st,
                                    crate::notify::Notice::success(if on {
                                        "Kept this server in Favorites"
                                    } else {
                                        "Removed this server from Favorites"
                                    }),
                                );
                            }
                            Err(error) => st.status = error.to_string(),
                        }
                    }),
            )
            .child(
                u.icon_button(&icons::X, tr("Close details"))
                    .key("server-panel-close")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| st.servers.selected = None),
            ),
    );
    body = body.child(
        text(
            [server.map_label().to_string(), server.mode_label(), dw::region_label(&server.country)]
                .into_iter()
                .filter(|part| !part.is_empty() && part != "-")
                .collect::<Vec<_>>()
                .join("  ·  "),
        )
        .font(BODY)
        .color(u.muted),
    );
    if let Some(port) = server.address.rsplit(':').next() {
        body = body.child(u.kv(tr("Port"), port, u.ink));
    }
    body = body
        .child(u.kv(
            tr("Players"),
            &format!("{} of {}", server.players, server.max_players.max(server.players)),
            if server.is_full() { u.amber } else { u.ink },
        ))
        .child(u.kv(tr("Ping"), &ping, u.ink));
    if !server.version.is_empty() {
        body = body.child(u.kv(tr("Version"), &format!("Deadworks {}", server.version), u.ink));
    }
    body = body.child(u.kv(tr("Region"), &dw::region_label(&server.country), u.ink));
    let address = server.address.clone();
    body = body.child(
        row()
            .w_full()
            .gap(8.0)
            .center_y()
            .child(text("Address").font(SMALL).color(u.muted).w(72.0))
            .child(
                text(server.address.clone())
                    .font(MONO)
                    .color(u.ink)
                    .grow(1.0)
                    .min_w(0.0)
                    .truncate(),
            )
            .child(
                u.icon_button(&icons::COPY, tr("Copy address"))
                    .key("server-copy-address")
                    .cursor(rune::Cursor::Pointer)
                    .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                        tx.copy(address.clone());
                        st.status = "Address copied.".to_string();
                    }),
            ),
    );
    let mods: Vec<String> = server
        .content_hint
        .iter()
        .filter(|name| !name.trim().is_empty())
        .cloned()
        .collect();
    if mods.is_empty() {
        body = body.child(u.kv(tr("Required mods"), tr("None"), u.muted));
    } else {
        body = body.child(u.kv(tr("Required mods"), &mods.join(", "), u.ink));
    }

    if !server.roster.is_empty() {
        let mut roster = server.roster.clone();
        roster.sort_by(|a, b| a.team.cmp(&b.team).then(b.kills.cmp(&a.kills)));
        let mut list = col().w_full().pt(6.0).child(
            row()
                .w_full()
                .pb(4.0)
                .center_y()
                .child(text(tr("Playing now")).font(BODY_MED).color(u.ink))
                .child(spacer())
                .child(text("K / D / A").font(MICRO).color(u.faint)),
        );
        let mut last_team = None;
        for player in roster.iter().take(12) {
            if last_team.is_some_and(|team| team != player.team) {
                list = list.child(el().h(10.0));
            }
            last_team = Some(player.team);
            list = list.child(
                row()
                    .w_full()
                    .h(30.0)
                    .gap(8.0)
                    .center_y()
                    .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                    .child(
                        text(team_label(player.team))
                            .font(MICRO)
                            .color(u.faint)
                            .w(28.0),
                    )
                    .child(
                        text(player.name.clone())
                            .font(BODY)
                            .color(u.ink)
                            .grow(1.0)
                            .min_w(0.0)
                            .truncate(),
                    )
                    .child(
                        text(player.hero.clone())
                            .font(SMALL)
                            .color(u.muted)
                            .w(74.0)
                            .truncate(),
                    )
                    .child(
                        text(format!("{} / {} / {}", player.kills, player.deaths, player.assists))
                            .font(MICRO)
                            .color(u.faint)
                            .w(64.0)
                            .align(Align::End),
                    ),
            );
        }
        if roster.len() > 12 {
            list = list.child(
                text(format!("and {} more", roster.len() - 12))
                    .font(SMALL)
                    .color(u.faint)
                    .pt(6.0),
            );
        }
        body = body.child(list);
    }

    let can_join = s.game_detected && !server.is_full();
    let target = server.clone();
    body = body.child(
        u.icon_action(&icons::PLAY, tr("Join"), ActionKind::Primary)
            .key("server-panel-join")
            .w_full()
            .opacity(if can_join { 1.0 } else { 0.4 })
            .disabled(!can_join)
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| open_join(st, target.clone())),
    );
    body
}

fn team_label(team: i64) -> &'static str {
    match team {
        1 => "T",
        2 => "H",
        3 => "M",
        _ => "-",
    }
}

// ---------------------------------------------------------------------------------------------
// Dialogs
// ---------------------------------------------------------------------------------------------

fn close(st: &mut State) {
    if st.servers.join_job.is_running() {
        return;
    }
    st.servers.dialog = None;
}

fn shell(u: &Ui, key: &str, title: String, body: El) -> El {
    u.dialog_shell()
        .key(key.to_string())
        .role("dialog")
        .label(title)
        .w(500.0)
        .padding(24.0)
        .gap(16.0)
        .modal(true, close)
        .on_key(|st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                close(st);
                true
            } else {
                false
            }
        })
        .child(body)
}

fn buttons(children: Vec<El>) -> El {
    let mut r = row().w_full().gap(8.0).pt(8.0).child(spacer());
    for child in children {
        r = r.child(child);
    }
    r
}

fn cancel(u: &Ui, label: &str) -> El {
    u.action(label, ActionKind::Ghost)
        .key("servers-dialog-cancel")
        .cursor(rune::Cursor::Pointer)
        .on_press(close)
}

fn open_direct(st: &mut State) {
    st.servers.direct = TextEdit::new("");
    st.servers.dialog = Some(Dialog::Direct);
}

fn start_direct(st: &mut State) {
    let input = st.servers.direct.text.clone();
    // A Sigil invite pasted here goes to Join with invite.
    if input.trim_start().starts_with("sigil-invite:") {
        invite::open(st, &input);
        return;
    }
    let server = match dw::parse_direct_address(&input) {
        Ok(server) => server,
        Err(error) => {
            st.status = error.to_string();
            return;
        }
    };
    open_join(st, server);
}

pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    if s.servers.dialog.is_none() && s.servers.tab == Tab::Host && s.screen == Screen::Servers {
        return host::dialog(s, u);
    }
    if s.servers.dialog.is_none() && s.servers.tab == Tab::Regions && s.screen == Screen::Servers {
        return regions::dialog(s, u);
    }
    match s.servers.dialog.as_ref()? {
        Dialog::Direct => Some(direct_dialog(s, u)),
        Dialog::Invite => Some(invite::dialog(s, u)),
        Dialog::Join { server, stage } => Some(join_dialog(s, u, server, stage)),
    }
}

fn direct_dialog(s: &State, u: &Ui) -> El {
    let body = col()
        .w_full()
        .gap(12.0)
        .child(text(tr("Connect to a server")).font(H1).color(u.ink))
        .child(
            text("Type the address the host gave you, for example 203.0.113.7:27015.")
                .font(BODY)
                .color(u.muted)
                .wrap(),
        )
        .child(
            row()
                .w_full()
                .h(32.0)
                .px(10.0)
                .gap(8.0)
                .center_y()
                .radius(crate::ui::r(8.0))
                .fill(u.well)
                .border(1.0, u.hair)
                .child(
                    rune::text_input(&s.servers.direct, MONO)
                        .key("servers-direct-input")
                        .label(tr("Server address"))
                        .single_line()
                        .grow(1.0)
                        .min_w(0.0)
                        .color(u.ink)
                        .placeholder("203.0.113.7:27015".to_string(), u.faint)
                        .caret(u.accent, u.accent.alpha(0.20))
                        .on_edit(|st: &mut State| &mut st.servers.direct),
                ),
        )
        .child(buttons(vec![
            cancel(u, tr("Cancel")),
            u.icon_action(&icons::LOG_IN, tr("Connect"), ActionKind::Primary)
                .key("servers-direct-go")
                .cursor(rune::Cursor::Pointer)
                .on_press(start_direct),
        ]));
    shell(u, "servers-direct", tr("Direct connect").to_string(), body)
}

fn bullet_line(u: &Ui, line: String) -> El {
    row()
        .w_full()
        .gap(10.0)
        .items_start()
        .child(
            el().size(4.0, 4.0)
                .radius(2.0)
                .fill(u.faint)
                .mt(8.0)
                .a11y_hidden(),
        )
        .child(text(line).font(BODY).color(u.prose).grow(1.0).min_w(0.0).wrap())
}

fn progress_bar(u: &Ui, fraction: Option<f32>) -> El {
    let f = fraction.unwrap_or(0.0).clamp(0.0, 1.0);
    row()
        .w_full()
        .h(4.0)
        .radius(crate::ui::r(2.0))
        .fill(Color::WHITE.alpha(0.08))
        .child(el().grow(f.max(0.001)).h(4.0).radius(crate::ui::r(2.0)).fill(u.accent))
        .child(el().grow((1.0 - f).max(0.001)).h(4.0))
}

fn join_dialog(s: &State, u: &Ui, server: &Server, stage: &Stage) -> El {
    let title = format!("Join {}", server.name);
    let mut body = col()
        .w_full()
        .gap(8.0)
        .child(text(title.clone()).font(H1).color(u.ink).wrap());
    match stage {
        Stage::Checking => {
            body = body
                .child(
                    row()
                        .gap(8.0)
                        .center_y()
                        .child(icon(&icons::LOADER, 14.0).color(u.faint).a11y_hidden())
                        .child(text("Checking what this server needs").font(BODY).color(u.muted)),
                )
                .child(buttons(vec![cancel(u, tr("Cancel"))]));
        }
        Stage::Ready(check) => {
            let blockers = check.blockers();
            if !blockers.is_empty() {
                for line in blockers {
                    body = body.child(text(line).font(BODY).color(u.rose).wrap());
                }
                body = body.child(buttons(vec![cancel(u, tr("Close"))]));
                return shell(u, "servers-join", title, body);
            }
            let writes = check.writes();
            if writes.is_empty() {
                body = body.child(
                    text("Everything this server needs is in place.")
                        .font(BODY)
                        .color(u.muted)
                        .wrap(),
                );
            } else {
                body = body.child(
                    text("Before Steam connects, Sigil will:")
                        .font(BODY)
                        .color(u.muted),
                );
                let mut list = col().w_full().gap(6.0).py(4.0);
                for line in writes {
                    list = list.child(bullet_line(u, line));
                }
                body = body.child(list);
                let mut notes = Vec::new();
                if let Some(host) = check.plan.fastdl_host()
                    && check.plan.downloads().next().is_some()
                {
                    let (bytes, unknown) = check.plan.download_size();
                    notes.push(if unknown > 0 || bytes == 0 {
                        format!("Downloads come from {host}, the host this server names.")
                    } else {
                        format!(
                            "{} from {host}, the host this server names.",
                            dw::format_bytes(bytes)
                        )
                    });
                }
                notes.push("Quit Deadlock first if it is running.".to_string());
                for note in notes {
                    body = body.child(text(note).font(SMALL).color(u.faint).wrap());
                }
            }
            let kept: Vec<String> = check
                .plan
                .items
                .iter()
                .filter(|i| i.action == ItemAction::KeepLocal)
                .map(|i| i.entry.name.clone())
                .collect();
            if !kept.is_empty() {
                body = body.child(
                    text(format!(
                        "Your copy of {} differs from the server's. It is kept; update the game if the join fails.",
                        kept.join(", ")
                    ))
                    .font(SMALL)
                    .color(u.amber)
                    .wrap(),
                );
            }
            if check.plan.truncated {
                body = body.child(
                    text("The server lists more files than fit in its reply. Some may be missing.")
                        .font(SMALL)
                        .color(u.amber)
                        .wrap(),
                );
            }
            if server.password {
                body = body.child(password_field(s, u));
            }
            let label = if check.needs_writes() {
                "Install and join"
            } else {
                tr("Join")
            };
            let confirmed = (**check).clone();
            body = body.child(buttons(vec![
                cancel(u, tr("Cancel")),
                u.icon_action(&icons::PLAY, label, ActionKind::Primary)
                    .key("servers-join-confirm")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        st.servers.last_check = Some(Box::new(confirmed.clone()));
                        start_join(st, confirmed.clone(), Vec::new());
                    }),
            ]));
        }
        Stage::Working => {
            let progress = s
                .servers
                .progress
                .lock()
                .map(|p| p.clone())
                .unwrap_or_default();
            let line = if progress.line.is_empty() {
                "Getting ready".to_string()
            } else {
                progress.line.clone()
            };
            let percent = progress
                .fraction
                .map(|f| format!("{}%", (f * 100.0).round() as u32))
                .unwrap_or_default();
            body = body
                .child(
                    row()
                        .w_full()
                        .gap(12.0)
                        .center_y()
                        .child(text(line).font(BODY).color(u.muted).grow(1.0).min_w(0.0).truncate())
                        .child(text(percent).font(MICRO).color(u.faint)),
                )
                .child(el().h(4.0))
                .child(progress_bar(u, progress.fraction))
                .child(
                    text("Keep Sigil open until Steam takes over.")
                        .font(SMALL)
                        .color(u.faint),
                );
        }
        Stage::Mismatch { check, files } => {
            let names: Vec<String> = files
                .iter()
                .map(|key| key.rsplit('/').next().unwrap_or(key).to_string())
                .collect();
            body = body
                .child(
                    text(format!(
                        "The server's host has a different build of {} than the server lists. The host is usually just out of date.",
                        names.join(", ")
                    ))
                    .font(BODY)
                    .color(u.muted)
                    .wrap(),
                )
                .child(
                    text("Install it anyway to try the join. It is checked again next time.")
                        .font(SMALL)
                        .color(u.faint)
                        .wrap(),
                );
            let check = (**check).clone();
            let accept = files.clone();
            body = body.child(buttons(vec![
                cancel(u, tr("Cancel")),
                u.action("Install anyway", ActionKind::Primary)
                    .key("servers-accept-mismatch")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        st.servers.last_check = Some(Box::new(check.clone()));
                        start_join(st, check.clone(), accept.clone());
                    }),
            ]));
        }
        Stage::Done(line) => {
            body = body
                .child(text(line.clone()).font(BODY).color(u.muted).wrap())
                .child(buttons(vec![
                    u.action(tr("Done"), ActionKind::Secondary)
                        .key("servers-join-done")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(close),
                ]));
        }
        Stage::Failed(error) => {
            let target = server.clone();
            body = body
                .child(text(error.clone()).font(BODY).color(u.rose).wrap())
                .child(buttons(vec![
                    cancel(u, tr("Close")),
                    u.action(tr("Try again"), ActionKind::Secondary)
                        .key("servers-join-retry")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| open_join(st, target.clone())),
                ]));
        }
    }
    shell(u, "servers-join", title, body)
}

fn password_field(s: &State, u: &Ui) -> El {
    col()
        .w_full()
        .gap(6.0)
        .pt(4.0)
        .child(text(tr("Password")).font(BODY_MED).color(u.ink))
        .child(
            row()
                .w_full()
                .h(32.0)
                .px(10.0)
                .center_y()
                .radius(crate::ui::r(8.0))
                .fill(u.well)
                .border(1.0, u.hair)
                .child(
                    rune::text_input(&s.servers.password, BODY)
                        .key("servers-password")
                        .label("Server password")
                        .single_line()
                        .secure()
                        .color(u.ink)
                        .placeholder("Ask the host", u.faint)
                        .caret(u.accent, u.accent.alpha(0.20))
                        .on_edit(|st: &mut State| &mut st.servers.password)
                        .grow(1.0)
                        .min_w(0.0),
                ),
        )
}

// ---------------------------------------------------------------------------------------------
// Filter menus
// ---------------------------------------------------------------------------------------------

/// The open filter menu, mounted at the root so it draws over the page and escapes its scroll.
pub fn overlay(s: &State, u: &Ui) -> Option<El> {
    if s.servers.tab == Tab::Host {
        return host::overlay(s, u);
    }
    if s.servers.tab == Tab::Regions {
        return None;
    }
    let menu = s.servers.menu?;
    if s.screen != Screen::Servers {
        return None;
    }
    let servers = scoped(s);
    let mut items = col().w_full().gap(0.0);
    items = items.child(
        menu_row(u, tr("All"), "", "", true)
            .on_press(|st: &mut State| {
                st.servers.filters.mode = None;
                st.servers.filters.continent = None;
                st.servers.filters.country = None;
                st.servers.filters.map = None;
                st.servers.menu = None;
            }),
    );
    match menu {
        FilterMenu::Mode => {
            let mut modes: BTreeMap<String, usize> = BTreeMap::new();
            for server in &servers {
                *modes.entry(server.mode_label()).or_default() += 1;
            }
            for (mode, count) in modes {
                let pick = mode.clone();
                let on = s.servers.filters.mode.as_deref() == Some(mode.as_str());
                items = items.child(
                    menu_row(u, &mode, &count.to_string(), &format!("servers-mode-item-{mode}"), on)
                        .on_press(move |st: &mut State| {
                            st.servers.filters.mode = Some(pick.clone());
                            st.servers.menu = None;
                        }),
                );
            }
        }
        FilterMenu::Region => {
            let mut by_continent: BTreeMap<usize, (Continent, BTreeMap<String, usize>)> =
                BTreeMap::new();
            for server in &servers {
                let country = server.country.trim().to_ascii_uppercase();
                if country.is_empty() {
                    continue;
                }
                let continent = Continent::of(&country);
                by_continent
                    .entry(continent as usize)
                    .or_insert_with(|| (continent, BTreeMap::new()))
                    .1
                    .entry(country)
                    .and_modify(|count| *count += 1)
                    .or_insert(1);
            }
            for (_, (continent, countries)) in by_continent {
                items = items.child(
                    row()
                        .w_full()
                        .h(28.0)
                        .px(10.0)
                        .center_y()
                        .child(
                            text(continent.label().to_string())
                                .font(SMALL)
                                .color(u.faint),
                        ),
                );
                let on = s.servers.filters.continent == Some(continent)
                    && s.servers.filters.country.is_none();
                items = items.child(
                    menu_row(
                        u,
                        &format!("All of {}", continent.label()),
                        "",
                        &format!("servers-region-{}", continent.id()),
                        on,
                    )
                    .on_press(move |st: &mut State| {
                        st.servers.filters.continent = Some(continent);
                        st.servers.filters.country = None;
                        st.servers.menu = None;
                    }),
                );
                for (country, count) in countries {
                    let pick = country.clone();
                    let continent_here = continent;
                    let on = s.servers.filters.country.as_deref() == Some(country.as_str());
                    items = items.child(
                        menu_row(
                            u,
                            &dw::region_label(&country),
                            &count.to_string(),
                            &format!("servers-region-item-{country}"),
                            on,
                        )
                        .on_press(move |st: &mut State| {
                            st.servers.filters.continent = Some(continent_here);
                            st.servers.filters.country = Some(pick.clone());
                            st.servers.menu = None;
                        }),
                    );
                }
            }
        }
        FilterMenu::Map => {
            let mut maps: BTreeMap<String, usize> = BTreeMap::new();
            for server in &servers {
                if server.map.trim().is_empty() {
                    continue;
                }
                *maps.entry(server.map.clone()).or_default() += 1;
            }
            for (map, count) in maps {
                let pick = map.clone();
                let on = s.servers.filters.map.as_deref() == Some(map.as_str());
                items = items.child(
                    menu_row(u, &map, &count.to_string(), &format!("servers-map-item-{map}"), on)
                        .on_press(move |st: &mut State| {
                            st.servers.filters.map = Some(pick.clone());
                            st.servers.menu = None;
                        }),
                );
            }
        }
    }
    Some(
        col()
            .key("servers-menu")
            .role("menu")
            .w(248.0)
            .padding(4.0)
            .fill(u.raised)
            .radius(crate::ui::r(10.0))
            .border(1.0, u.hair_strong)
            .shadow(12.0, 32.0, crate::ui::soft(Color::BLACK.alpha(0.5)))
            .anchored(menu.anchor(), Side::Bottom, Align::Start, 6.0)
            .modal(true, |st: &mut State| st.servers.menu = None)
            .on_key(|st: &mut State, ev: &KeyEvent, _| {
                if ev.key == Key::Escape {
                    st.servers.menu = None;
                    true
                } else {
                    false
                }
            })
            .child(col().w_full().max_h(360.0).scroll(ScrollSpec { stick_bottom: false }).child(items)),
    )
}

fn menu_row(u: &Ui, label: &str, detail: &str, key: &str, on: bool) -> El {
    row()
        .key(format!("servers-menu-{key}"))
        .w_full()
        .h(32.0)
        .px(10.0)
        .gap(8.0)
        .center_y()
        .radius(crate::ui::r(6.0))
        .hover(u.hover)
        .focusable(true)
        .role("menuitem")
        .checked(on)
        .label(label.to_string())
        .cursor(rune::Cursor::Pointer)
        .child(
            text(label.to_string())
                .font(BODY)
                .color(if on { u.ink } else { u.prose })
                .grow(1.0)
                .min_w(0.0)
                .truncate()
                .a11y_hidden(),
        )
        .child(
            text(detail.to_string())
                .font(SMALL)
                .color(u.faint)
                .a11y_hidden(),
        )
        .child(if on {
            icon(&icons::CHECK, 14.0).color(u.muted).a11y_hidden()
        } else {
            el().size(14.0, 14.0)
        })
}

// ---------------------------------------------------------------------------------------------
// Scenes
// ---------------------------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn fixture_server(
    id: &str,
    name: &str,
    address: &str,
    map: &str,
    players: u32,
    max: u32,
    country: &str,
    content: &[&str],
) -> Server {
    let roster = (0..players.min(6))
        .map(|i| dw::Player {
            name: ["ember", "quill", "mossy", "vexa", "rook", "juno"][i as usize].to_string(),
            hero: ["Haze", "Infernus", "Lash", "Mo & Krill", "Seven", "Vindicta"][i as usize]
                .to_string(),
            team: 2 + (i as i64 % 2),
            kills: 12 - i as i64,
            deaths: 3 + i as i64,
            assists: 7,
        })
        .collect();
    Server {
        id: id.to_string(),
        name: name.to_string(),
        address: address.to_string(),
        map: map.to_string(),
        players,
        max_players: max,
        bots: 0,
        country: country.to_string(),
        version: if country.is_empty() { String::new() } else { "0.6.4".to_string() },
        game_mode: String::new(),
        registered: !country.is_empty(),
        password: false,
        has_addons: !content.is_empty(),
        roster,
        content_hint: content.iter().map(|c| c.to_string()).collect(),
    }
}

/// Invented servers in the shape of the live list, for headless scenes.
pub fn fixture_list() -> ServerList {
    let mut servers = vec![
        fixture_server("1", "EU Movement Arena", "203.0.113.10:27015", "dl_midtown", 7, 31, "LT", &[]),
        fixture_server("2", "Retake | EU", "203.0.113.11:27018", "dl_midtown", 6, 30, "AT", &[]),
        fixture_server("3", "Street Football", "203.0.113.12:27015", "start", 4, 31, "FR", &[]),
        fixture_server("4", "Overthrow 5v5", "203.0.113.13:27015", "overthrow10", 10, 10, "FR", &["overthrow10"]),
        fixture_server("5", "Harbor Pickup", "203.0.113.14:27015", "dl_harbor", 2, 8, "US", &["harbor"]),
        fixture_server("6", "Squid Tag", "203.0.113.15:27015", "st_dome_a2", 0, 31, "US", &["st_dome_a2"]),
        fixture_server("7", "Deadlock Roulette: Random Party Modes", "203.0.113.16:27015", "dl_midtown", 0, 24, "US", &[]),
        fixture_server("8", "Lab | Pickup 1v1 | Warmup", "203.0.113.17:27015", "dl_midtown", 0, 31, "DE", &[]),
        fixture_server("9", "Deadworks Server", "203.0.113.18:27015", "dl_midtown", 0, 31, "", &[]),
        fixture_server("10", "NA Movement Arena", "203.0.113.19:27015", "dl_midtown", 0, 31, "US", &[]),
        fixture_server("11", "Tokyo Late Night", "203.0.113.20:27015", "dl_harbor", 5, 24, "JP", &[]),
        fixture_server("12", "Sydney Brawl", "203.0.113.21:27015", "st_dome_a2", 3, 16, "AU", &[]),
        fixture_server("13", "Dubai Runbacks", "203.0.113.22:27015", "custom_arena", 0, 12, "AE", &["custom_arena"]),
    ];
    servers[1].password = true;
    ServerList {
        servers,
        registry_ok: true,
        steam_degraded: false,
    }
}

fn fixture_pings() -> HashMap<String, Option<u32>> {
    [
        ("1", Some(48)),
        ("2", Some(31)),
        ("3", Some(36)),
        ("4", Some(44)),
        ("5", Some(164)),
        ("6", Some(112)),
        ("7", Some(118)),
        ("8", Some(29)),
        ("9", None),
        ("10", Some(96)),
        ("11", Some(210)),
        ("12", Some(240)),
        ("13", Some(140)),
    ]
    .into_iter()
    .map(|(id, ms)| (id.to_string(), ms))
    .collect()
}

fn fixture_saved() -> SavedServers {
    let list = fixture_list();
    let favorite = list.servers[0].clone();
    let recent = list.servers[4].clone();
    SavedServers {
        schema: dw::SAVED_SERVERS_SCHEMA,
        favorites: vec![dw::SavedServer {
            address: favorite.address,
            name: favorite.name,
            map: favorite.map,
            at: "2026-11-16T20:00:00.000Z".to_string(),
        }],
        recent: vec![dw::SavedServer {
            address: recent.address,
            name: recent.name,
            map: recent.map,
            at: "2026-11-17T09:00:00.000Z".to_string(),
        }],
    }
}

pub fn scene(s: &mut State) {
    s.servers.list = Some(Ok(fixture_list()));
    s.servers.pings = fixture_pings();
    s.servers.saved = fixture_saved();
    s.servers.started = true;
}

pub fn scene_detail(s: &mut State) {
    scene(s);
    if let Some(Ok(list)) = &s.servers.list {
        s.servers.selected = Some(list.servers[0].clone());
    }
}

/// Filters on: joinable plus a region, to show the chip row and a sorted column.
pub fn scene_filters(s: &mut State) {
    scene(s);
    s.servers.filters = ServerFilter::joinable();
    s.servers.filters.continent = Some(Continent::Europe);
    s.servers.sort = Some(ServerSort {
        key: ServerSortKey::Players,
        direction: SortDirection::Desc,
    });
    s.servers.query = TextEdit::new("arena");
}

pub fn scene_favorites(s: &mut State) {
    scene(s);
    s.servers.scope = Scope::Favorites;
    if let Some(Ok(list)) = &s.servers.list {
        s.servers.selected = Some(list.servers[0].clone());
    }
}

pub fn scene_menu(s: &mut State) {
    scene(s);
    s.servers.menu = Some(FilterMenu::Region);
}

pub fn scene_connect(s: &mut State) {
    scene(s);
    s.servers.direct = TextEdit::new("203.0.113.7:27015");
    s.servers.dialog = Some(Dialog::Direct);
}

/// The confirm step for a server that needs a map and the UI pack on a fresh install.
pub fn scene_join(s: &mut State) {
    scene(s);
    let Some(Ok(list)) = &s.servers.list else { return };
    let server = list.servers[5].clone();
    let map = dw::ContentEntry {
        kind: ContentKind::Map,
        name: "st_dome_a2".to_string(),
        hash: "4ac96b19d03e1f22".to_string(),
        size: Some(48_310_272),
    };
    let check = JoinCheck {
        address: server.address.clone(),
        advertised: true,
        plan: dw::ContentPlan {
            items: vec![dw::PlanItem {
                dest: PathBuf::from("citadel/maps/st_dome_a2.vpk"),
                entry: map,
                action: ItemAction::Download,
            }],
            truncated: false,
            fastdl: Some("https://api.deadworks.net/fastdl/u/squidtag".to_string()),
            hashed: Default::default(),
        },
        gameinfo: dw::DeadworksGameinfo {
            present: true,
            search_paths: true,
            ..Default::default()
        },
        bootstrap: BootstrapState::Needed {
            manifest: dw::BootstrapManifest {
                version: 8,
                compressed_size: 299_548,
                sha256: "0".repeat(64),
                download_url: "https://api.deadworks.net/api/bootstrap/v/8.vpk.bz2".to_string(),
                min_version: 0,
            },
            installed: None,
        },
        allow_private: false,
        sigil_host: false,
    };
    s.servers.dialog = Some(Dialog::Join {
        server: Box::new(server),
        stage: Stage::Ready(Box::new(check)),
    });
}

pub fn scene_join_progress(s: &mut State) {
    scene_join(s);
    *s.servers.progress.lock().unwrap() = JoinProgress {
        line: "Downloading st_dome_a2".to_string(),
        fraction: Some(0.42),
    };
    set_stage(s, Stage::Working);
}

/// The real list and pings, fetched synchronously for a headless snapshot.
pub fn scene_live(s: &mut State) {
    let list = dw::fetch_servers(dw::DEADWORKS_API_BASE).map_err(|e| e.to_string());
    if let Ok(list) = &list {
        let targets = list
            .servers
            .iter()
            .filter_map(|sv| sv.socket_addr().map(|a| (sv.id.clone(), a)))
            .collect();
        s.servers.pings = dw::ping_many(targets, PING_TIMEOUT);
    }
    s.servers.list = Some(list);
}
