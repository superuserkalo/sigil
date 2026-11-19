//! Headless scenes for the Host tab, one per state. Registered in `main.rs`. Fixtures only:
//! nothing is read, downloaded or started.

use std::path::{Path, PathBuf};

use rune::TextEdit;

use deadlock_core::host::{ContentOrigin, ServerContentKind};
use deadlock_core::servers::invite::{Invite, InviteItem, InviteTarget};

use super::listen::ConDebug;
use super::{
    Busy, Confirm, Content, Firewall, HostState, Network, Phase, Plugin, Setup, Upnp, bridge,
};
use crate::screens::servers::invite::{Item, Plan, Stage};
use crate::app::State;
use crate::model::Screen;

pub type Scene = (&'static str, fn(&mut State));

pub const SCENES: &[Scene] = &[
    ("host-not-windows", not_windows),
    ("host-checking", checking),
    ("host-setup", fresh),
    ("host-low-disk", low_disk),
    ("host-steamcmd-installing", steamcmd_installing),
    ("host-files-dialog", files_dialog),
    ("host-steamcmd-open", steamcmd_open),
    ("host-deadworks-installing", deadworks_installing),
    ("host-ready", ready),
    ("host-running", running),
    ("host-running-de", running_german),
    ("host-firewall-dialog", firewall_dialog),
    ("host-outdated", outdated),
    ("host-failed", failed),
    ("host-exited", exited),
    ("host-custom-map", custom_map),
    ("host-map-menu", map_menu),
    ("host-running-custom", running_custom),
    ("host-running-downloads", running_downloads),
    ("host-running-downloads-de", running_downloads_german),
    ("host-downloads-rule-missing", downloads_rule_missing),
    ("host-listen", listen),
    ("host-listen-waiting", listen_waiting),
    ("host-listen-no-condebug", listen_no_condebug),
    ("host-relay-waiting", relay_waiting),
    ("host-relay-running", relay_running),
    ("host-relay-running-de", relay_running_german),
    ("host-direct-upnp", direct_upnp),
    ("host-upnp-failed", upnp_failed),
    ("maps-listen", maps_listen),
    ("servers-invite-enter", invite_enter),
    ("servers-invite-review", invite_review),
    ("servers-invite-installing", invite_installing),
    ("servers-invite-join", invite_join),
];

const GAME_BUILD: &str = "6123456";

fn base(s: &mut State) {
    s.load_demo();
    s.screen = Screen::Servers;
    s.game_detected = true;
    crate::screens::servers::scene(s);
    s.servers.tab = crate::screens::servers::Tab::Host;
    let host = &mut s.servers.host;
    *host = HostState::default();
    host.windows = true;
    host.loaded = true;
    host.server_dir = Some(PathBuf::from(r"D:\SigilServer"));
    host.lan = vec!["192.168.1.20".to_string()];
    host.name = TextEdit::new("Friday scrims");
}

fn fresh_setup() -> Setup {
    Setup {
        dotnet: None,
        free_bytes: Some(412 * 1024 * 1024 * 1024),
        enough_disk: true,
        steamcmd: false,
        server_build: None,
        game_build: Some(GAME_BUILD.to_string()),
        needs_update: false,
        deadworks: None,
        deadworks_behind: false,
        deadworks_pinned: "v0.6.4".to_string(),
        custom_maps: vec![
            Content {
                name: "harbor_night".to_string(),
                vpk: PathBuf::from(r"D:\Deadlock\game\citadel\addons\pak04_dir.vpk"),
            },
            Content {
                name: "st_dome_a2".to_string(),
                vpk: PathBuf::from(r"D:\Deadlock\game\citadel\addons\pak07_dir.vpk"),
            },
        ],
        server_maps: vec!["harbor_night".to_string()],
    }
}

fn done_setup() -> Setup {
    Setup {
        dotnet: Some("10.0.1".to_string()),
        steamcmd: true,
        server_build: Some(GAME_BUILD.to_string()),
        deadworks: Some("v0.6.4".to_string()),
        ..fresh_setup()
    }
}

fn plugins() -> Vec<Plugin> {
    [
        ("DeathmatchPlugin", true),
        ("ItemRotationPlugin", true),
        ("TagPlugin", false),
        ("RollTheDicePlugin", false),
    ]
    .into_iter()
    .map(|(name, on)| Plugin {
        name: name.to_string(),
        on,
    })
    .collect()
}

fn not_windows(s: &mut State) {
    base(s);
    s.servers.host.windows = false;
}

fn checking(s: &mut State) {
    base(s);
}

fn fresh(s: &mut State) {
    base(s);
    s.servers.host.setup = Some(Ok(fresh_setup()));
    s.servers.host.firewall = Firewall::Unknown;
}

fn low_disk(s: &mut State) {
    base(s);
    s.servers.host.setup = Some(Ok(Setup {
        dotnet: Some("10.0.1".to_string()),
        free_bytes: Some(21 * 1024 * 1024 * 1024 + 300 * 1024 * 1024),
        enough_disk: false,
        steamcmd: true,
        ..fresh_setup()
    }));
    s.servers.host.server_dir = Some(PathBuf::from(
        r"C:\Users\ember\AppData\Roaming\DeadlockAgentHarness\server",
    ));
}

fn steamcmd_installing(s: &mut State) {
    base(s);
    s.servers.host.setup = Some(Ok(Setup {
        dotnet: Some("10.0.1".to_string()),
        ..fresh_setup()
    }));
    s.servers.host.busy = Some(Busy::SteamCmd);
}

fn files_dialog(s: &mut State) {
    base(s);
    s.servers.host.setup = Some(Ok(Setup {
        dotnet: Some("10.0.1".to_string()),
        steamcmd: true,
        ..fresh_setup()
    }));
    s.servers.host.steam_user = TextEdit::new("ember_plays");
    s.servers.host.confirm = Some(Confirm::ServerFiles { update: false });
}

fn steamcmd_open(s: &mut State) {
    base(s);
    s.servers.host.setup = Some(Ok(Setup {
        dotnet: Some("10.0.1".to_string()),
        steamcmd: true,
        ..fresh_setup()
    }));
    s.servers.host.steamcmd = Some(bridge::SteamCmdRun::fixture());
    s.servers.host.steamcmd_status = Some(
        " Update state (0x61) downloading, progress: 42.17 (15032385536 / 35651584000)".to_string(),
    );
}

fn deadworks_installing(s: &mut State) {
    base(s);
    s.servers.host.setup = Some(Ok(Setup {
        deadworks: None,
        ..done_setup()
    }));
    s.servers.host.busy = Some(Busy::Deadworks);
}

fn ready(s: &mut State) {
    base(s);
    let host = &mut s.servers.host;
    host.setup = Some(Ok(done_setup()));
    host.plugins = Some(Ok(plugins()));
    host.firewall = Firewall::Missing;
}

fn console_lines() -> Vec<String> {
    [
        "Deadworks 0.6.4 starting (managed host .NET 10.0.1)",
        "Loading plugins from game\\bin\\win64\\managed\\plugins",
        "[DeathmatchPlugin] loaded",
        "[ItemRotationPlugin] loaded",
        "Network: IP 0.0.0.0, mode MP, dedicated Yes, ports 27067 SV / 27005 CL",
        "Host_NewGame on map dl_midtown",
        "Warning: no Steam game server account token; the server is not listed publicly",
        "SV:  Server is hibernating",
        "Client \"ember\" connected (192.168.1.20:27005)",
        "Client \"quill\" connected (192.168.1.34:27005)",
        "[DeathmatchPlugin] round 1 started, 2 players",
        "Client \"mossy\" connected (192.168.1.41:27005)",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn running(s: &mut State) {
    ready(s);
    let host = &mut s.servers.host;
    host.phase = Phase::Running;
    host.firewall = Firewall::Present;
    host.password = TextEdit::new("lantern");
    host.lan = vec!["192.168.1.20".to_string(), "10.0.0.7".to_string()];
    host.push_lines(console_lines());
}

fn running_german(s: &mut State) {
    s.prefs.language = crate::screens::settings::i18n::Lang::German;
    crate::screens::settings::i18n::set(s.prefs.language);
    running(s);
}

fn firewall_dialog(s: &mut State) {
    ready(s);
    s.servers.host.confirm = Some(Confirm::Firewall);
}

fn outdated(s: &mut State) {
    ready(s);
    s.servers.host.setup = Some(Ok(Setup {
        server_build: Some("6120246".to_string()),
        needs_update: true,
        deadworks: Some("v0.6.3".to_string()),
        deadworks_behind: true,
        ..done_setup()
    }));
    s.servers.host.firewall = Firewall::Partial;
}

fn failed(s: &mut State) {
    ready(s);
    s.servers.host.phase = Phase::Failed(
        "deadworks.exe could not start: game\\bin\\win64\\deadworks.exe is missing. Install Deadworks again.".to_string(),
    );
}

fn exited(s: &mut State) {
    ready(s);
    s.servers.host.phase = Phase::Exited("exit code 1".to_string());
    s.servers
        .host
        .push_lines(console_lines().into_iter().take(5));
    s.servers.host.push_lines([
        "Unhandled exception: System.IO.FileNotFoundException: DeadworksManaged.Api.dll"
            .to_string(),
    ]);
    s.servers.host.port = TextEdit::new("27067");
}

/// Arrived from the Maps tab with a custom map: the map is chosen and its VPK queued.
fn custom_map(s: &mut State) {
    ready(s);
    super::open_with_map(
        s,
        "lantern_ruins",
        Path::new(r"C:\Users\ember\Downloads\lantern_ruins.vpk"),
    );
}

fn map_menu(s: &mut State) {
    custom_map(s);
    s.servers.host.map_menu = true;
}

/// Running on a custom map: the invite says Sigil players get the map offered.
fn running_custom(s: &mut State) {
    running(s);
    s.servers.host.map = TextEdit::new("harbor_night");
    s.servers.host.live_map = Some("harbor_night".to_string());
}

/// Running with custom content: the map download server answers beside it on port 27068.
fn running_downloads(s: &mut State) {
    running_custom(s);
    s.servers.host.serving = Some(27068);
}

fn running_downloads_german(s: &mut State) {
    s.prefs.language = crate::screens::settings::i18n::Lang::German;
    crate::screens::settings::i18n::set(s.prefs.language);
    running_downloads(s);
}

/// The game port's rules are there from before; the download port's is not yet.
fn downloads_rule_missing(s: &mut State) {
    running_downloads(s);
    s.servers.host.firewall = Firewall::Partial;
}

// --- no port forwarding (ADR 0015, second amendment) -----------------------------------------

const CODE: &str = "[A:1:570253314:51555]";

/// "Play with friends, no setup" after Launch: the code is in and the invite carries the map.
fn listen(s: &mut State) {
    ready(s);
    let host = &mut s.servers.host;
    host.listen_map = "harbor_night".to_string();
    host.listen_condebug = Some(ConDebug::Added);
    host.listen_invite = Some((CODE.to_string(), format!("sigil-invite:v1:{CODE}:o:715100/1414905")));
    s.maps.invite = crate::screens::maps::Invite::Ready { map: "harbor_night".to_string(), code: CODE.to_string() };
}

fn listen_waiting(s: &mut State) {
    ready(s);
    s.servers.host.listen_condebug = Some(ConDebug::Already);
    s.maps.invite = crate::screens::maps::Invite::Watching("dl_midtown".to_string());
}

fn listen_no_condebug(s: &mut State) {
    ready(s);
    s.servers.host.listen_condebug = Some(ConDebug::Failed(
        "Steam is running and would overwrite the change when it exits. Quit Steam, then save again.".to_string(),
    ));
    s.maps.invite = crate::screens::maps::Invite::Missed("dl_midtown".to_string());
}

fn origins() -> Vec<ContentOrigin> {
    vec![
        ContentOrigin {
            key: "maps/harbor_night".to_string(),
            kind: ServerContentKind::Map,
            name: "harbor_night".to_string(),
            item: Some(InviteItem { mod_id: 715100, file_id: 1414905 }),
        },
        ContentOrigin {
            key: "addons/lantern_hud".to_string(),
            kind: ServerContentKind::Addon,
            name: "lantern_hud".to_string(),
            item: None,
        },
    ]
}

/// Relay mode, just started: Steam has not printed the code yet.
fn relay_waiting(s: &mut State) {
    running_custom(s);
    let host = &mut s.servers.host;
    host.network = Network::Relay;
    host.origins = Some(origins());
}

/// Relay mode with the code: the invite carries the GameBanana map, the HUD addon is not on
/// GameBanana.
fn relay_running(s: &mut State) {
    relay_waiting(s);
    let host = &mut s.servers.host;
    host.join_code = Some(CODE.to_string());
    host.relay_ready = true;
    host.serving = Some(27068);
    host.push_lines([
        "SV:  ServerSteamID=[A:1:570253314:51555] (90293420156608514).".to_string(),
        "SDR RelayNetworkStatus:  avail=OK  config=OK  anyrelay=OK   (OK.  Relays: 25 valid)".to_string(),
    ]);
}

fn relay_running_german(s: &mut State) {
    s.prefs.language = crate::screens::settings::i18n::Lang::German;
    crate::screens::settings::i18n::set(s.prefs.language);
    relay_running(s);
}

/// Direct mode with UPnP on: the router opened the game port, refused the download port, and
/// said its public address, which the invite uses.
fn direct_upnp(s: &mut State) {
    running_downloads(s);
    let host = &mut s.servers.host;
    host.network = Network::Direct;
    host.upnp = true;
    host.origins = Some(origins());
    host.upnp_result = Some(Upnp::Mapped {
        ports: vec![
            ("TCP 27067".to_string(), None),
            ("UDP 27067".to_string(), None),
            ("TCP 27068".to_string(), Some("Port 27068 is already forwarded to another device on your router.".to_string())),
        ],
        external_ip: Some("203.0.113.7".to_string()),
    });
}

fn upnp_failed(s: &mut State) {
    running(s);
    let host = &mut s.servers.host;
    host.network = Network::Direct;
    host.upnp = true;
    host.origins = Some(Vec::new());
    host.upnp_result = Some(Upnp::Failed(
        "No router answered the UPnP search. UPnP may be off on your router. Turn it on there, or forward the ports by hand.".to_string(),
    ));
}

/// The Maps page with the card on top.
fn maps_listen(s: &mut State) {
    listen(s);
    s.screen = Screen::Maps;
    s.maps.scanned = true;
    s.maps.catalog = Some(Ok(Vec::new()));
}

fn invite_base(s: &mut State) {
    s.load_demo();
    s.screen = Screen::Servers;
    s.game_detected = true;
    crate::screens::servers::scene(s);
    s.servers.dialog = Some(crate::screens::servers::Dialog::Invite);
}

fn invite_enter(s: &mut State) {
    invite_base(s);
    s.servers.invite.text = TextEdit::new("connect [A:1:570253314:51555]");
    s.servers.invite.stage = Stage::Enter(Some(
        "This is not a Sigil invite. Ask your friend to copy it again from Sigil's Host page.".to_string(),
    ));
}

fn invite() -> Invite {
    Invite::new(
        InviteTarget::Steam(CODE.to_string()),
        true,
        [
            InviteItem { mod_id: 715100, file_id: 1414905 },
            InviteItem { mod_id: 600009, file_id: 1388001 },
            InviteItem { mod_id: 598120, file_id: 1370442 },
        ],
    )
    .expect("fixture invite")
}

fn items(plans: [Plan; 3]) -> Vec<Item> {
    let name = |id: u64| match id {
        715100 => "Harbor Night",
        600009 => "Lantern HUD",
        _ => "Quiet Footsteps",
    };
    invite()
        .items
        .into_iter()
        .zip(plans)
        .map(|(item, plan)| Item { item, name: name(item.mod_id).to_string(), plan, file_name: None })
        .collect()
}

fn invite_review(s: &mut State) {
    invite_base(s);
    s.servers.invite.stage = Stage::Review {
        invite: invite(),
        items: items([Plan::Get, Plan::TurnOn(5), Plan::Replace(vec![9])]),
    };
}

fn invite_installing(s: &mut State) {
    invite_base(s);
    s.servers.invite.stage = Stage::Installing {
        invite: invite(),
        items: items([Plan::Working, Plan::TurnOn(5), Plan::Working]),
    };
}

fn invite_join(s: &mut State) {
    invite_base(s);
    s.servers.password = TextEdit::new("lantern");
    s.servers.invite.stage = Stage::Join(invite());
}
