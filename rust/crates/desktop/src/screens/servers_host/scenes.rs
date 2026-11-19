//! Headless scenes for the Host tab, one per state. Registered in `main.rs`. Fixtures only:
//! nothing is read, downloaded or started.

use std::path::{Path, PathBuf};

use rune::TextEdit;

use super::{Busy, Confirm, Content, Firewall, HostState, Phase, Plugin, Setup, bridge};
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
