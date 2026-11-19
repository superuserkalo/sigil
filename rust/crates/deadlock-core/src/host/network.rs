//! How friends reach the hosted server (ADR 0015, second amendment).
//!
//! - [`Network::Relay`] (the default): Steam networking and Valve's relays, no forwarded port.
//!   The SERVER's own `game/citadel/gameinfo.gi` gets the two edits the Deadworks launcher makes
//!   for its "SDR" mode (`launcher/src-tauri/src/hosting/netcfg.rs`, MIT):
//!   `CreateListenSocketP2P 2` as the first entry of `NetworkSystem` and
//!   `"net_p2p_listen_dedicated" "1"` as the first entry of `ConVars`. The convar is
//!   development-only, so it cannot go on the command line. The server starts with `-nomaster`
//!   and `serverbrowser.unlisted` (it cannot be reached at the address a list would show), and
//!   friends join with `connect [A:1:...]`, the `ServerSteamID` the console prints. That code
//!   changes on every restart.
//! - [`Network::Direct`]: the address and port, as before (firewall rule, router forwarding or
//!   UPnP). The server's gameinfo.gi is Steam's own again.
//!
//! The player's own Deadlock install is never touched: everything here writes inside the server
//! folder, after [`super::check_server_dir`], with the previous file kept in
//! `sigil-content-backups/<time>/`. SteamCMD's `validate` resets gameinfo.gi, so the edit is
//! applied again on every Start.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::require_approval;
use super::setup::check_server_dir;
use crate::error::{Error, Result};
pub use crate::servers::invite::valid_steam_code;
use crate::util::write_atomic;

/// How friends reach the server.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Network {
    /// Steam relay: no port forwarding, a join code that changes every restart.
    #[default]
    Relay,
    /// The server's address and port: forwarding (or UPnP) for friends outside the LAN.
    Direct,
}

const NETWORK_BLOCK: &str = "NetworkSystem";
const NETWORK_KEY: &str = "CreateListenSocketP2P";
const NETWORK_LINE: &str = "CreateListenSocketP2P\t2";
const CONVARS_BLOCK: &str = "ConVars";
const CONVARS_KEY: &str = "net_p2p_listen_dedicated";
const CONVARS_LINE: &str = "\"net_p2p_listen_dedicated\"\t\"1\"";
/// Sigil's record of what it set in `deadworks.jsonc` for the network mode.
pub const NETWORK_MARKER: &str = "sigil-network.json";

/// `<server>/game/citadel/gameinfo.gi`.
pub fn server_gameinfo_path(server_dir: &Path) -> PathBuf {
    server_dir.join("game").join("citadel").join("gameinfo.gi")
}

/// The server's gameinfo.gi for `network`, from its current text. Relay makes the two entries
/// the first of their blocks (replacing any earlier value); Direct takes them out again, which
/// gives back Steam's file. Idempotent; line endings and indentation follow the file.
pub fn render_server_gameinfo(text: &str, network: Network) -> Result<String> {
    match network {
        Network::Relay => {
            let text = set_first_entry(text, NETWORK_BLOCK, NETWORK_KEY, Some(NETWORK_LINE))?;
            set_first_entry(&text, CONVARS_BLOCK, CONVARS_KEY, Some(CONVARS_LINE))
        }
        Network::Direct => {
            let text = set_first_entry(text, NETWORK_BLOCK, NETWORK_KEY, None)?;
            set_first_entry(&text, CONVARS_BLOCK, CONVARS_KEY, None)
        }
    }
}

/// True when the text carries both relay entries.
pub fn gameinfo_has_relay(text: &str) -> bool {
    render_server_gameinfo(text, Network::Relay).is_ok_and(|rendered| rendered == text)
        && text.contains(NETWORK_KEY)
}

/// Bring the server's gameinfo.gi in line with `network`, inside the player's Start press.
/// Returns true when the file changed (its previous copy went to the backups first).
pub fn apply_server_gameinfo(server_dir: &Path, network: Network, approve: bool) -> Result<bool> {
    require_approval(approve, "Changing the server's gameinfo.gi")?;
    check_server_dir(server_dir, None)?;
    let path = server_gameinfo_path(server_dir);
    let meta = fs::symlink_metadata(&path).map_err(|_| {
        Error::msg("The server folder has no game/citadel/gameinfo.gi. Download the server files first.")
    })?;
    if !meta.is_file() {
        return Err(Error::msg("The server's gameinfo.gi is not a plain file. Nothing was changed."));
    }
    let text = fs::read_to_string(&path)?;
    let rendered = render_server_gameinfo(&text, network)?;
    if rendered == text {
        return Ok(false);
    }
    let backups = super::content::backup_dir(server_dir);
    let target = backups.join("game").join("citadel").join("gameinfo.gi");
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(&path, &target)?;
    write_atomic(&path, rendered.as_bytes())?;
    Ok(true)
}

/// Everything the network mode changes in the server folder before a start: gameinfo.gi and the
/// Deadworks listing. Part of the player's Start press. Returns true when gameinfo.gi changed.
pub fn prepare_network(server_dir: &Path, network: Network, approve: bool) -> Result<bool> {
    let changed = apply_server_gameinfo(server_dir, network, approve)?;
    apply_listing(server_dir, network)?;
    Ok(changed)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct NetworkMarker {
    /// Sigil set `serverbrowser.unlisted` for relay mode, so it may take it out again.
    unlisted_by_sigil: bool,
}

/// Keep a relay server off Deadworks' server list (`serverbrowser.unlisted`), as the Deadworks
/// launcher does: the list would show an address nobody can reach. Direct mode takes out only
/// a value Sigil set; one the player set by hand stays.
pub(super) fn apply_listing(server_dir: &Path, network: Network) -> Result<()> {
    let marker_path = server_dir.join(NETWORK_MARKER);
    let mut marker: NetworkMarker = fs::read_to_string(&marker_path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let unlisted = super::content::serverbrowser_value(server_dir, "unlisted").and_then(|v| v.as_bool());
    let before = marker.clone();
    match network {
        Network::Relay if unlisted != Some(true) => {
            super::content::set_serverbrowser_value(server_dir, "unlisted", Some(serde_json::Value::Bool(true)))?;
            marker.unlisted_by_sigil = true;
        }
        Network::Relay => {}
        Network::Direct if marker.unlisted_by_sigil => {
            if unlisted == Some(true) {
                super::content::set_serverbrowser_value(server_dir, "unlisted", None)?;
            }
            marker.unlisted_by_sigil = false;
        }
        Network::Direct => {}
    }
    if marker != before {
        let mut bytes = serde_json::to_vec_pretty(&marker)?;
        bytes.push(b'\n');
        write_atomic(&marker_path, &bytes)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// The server console: join code and relay readiness
// ---------------------------------------------------------------------------------------------

/// What one console line says about Steam networking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsoleEvent {
    /// `SV:  ServerSteamID=[A:1:570253314:51555] (90293420156608514).`: the join code.
    ServerSteamId(String),
    /// `SDR RelayNetworkStatus:  avail=OK ...`: Valve's relays are reachable.
    RelayReady,
    /// `Gameserver logged on to Steam` or `Connection to Steam servers successful`.
    LoggedOn,
}

/// Read one console line. Each signal is matched from the start of the line, never as a
/// substring, because player names appear inside other lines (as the Deadworks launcher's
/// `console.rs` does).
pub fn parse_console_line(line: &str) -> Option<ConsoleEvent> {
    let t = line.trim_end();
    if let Some(code) = server_steam_id(t) {
        return Some(ConsoleEvent::ServerSteamId(code));
    }
    if t.starts_with("SDR RelayNetworkStatus:") && t.contains("avail=OK") {
        return Some(ConsoleEvent::RelayReady);
    }
    if t.starts_with("Gameserver logged on to Steam") || t.starts_with("Connection to Steam servers successful") {
        return Some(ConsoleEvent::LoggedOn);
    }
    None
}

fn server_steam_id(t: &str) -> Option<String> {
    let rest = t.strip_prefix("SV:").unwrap_or(t).trim_start().strip_prefix("ServerSteamID=")?;
    let id = &rest[..=rest.find(']')?];
    valid_steam_code(id).then(|| id.to_string())
}

/// What the console has said so far about Steam networking.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelayState {
    /// The first `ServerSteamID` of this run. The server prints it once, before anyone can be
    /// connected; later lines of that shape are not believed.
    pub join_code: Option<String>,
    pub relay_ready: bool,
    pub logged_on: bool,
}

impl RelayState {
    /// Take in one console line.
    pub fn feed(&mut self, line: &str) {
        match parse_console_line(line) {
            Some(ConsoleEvent::ServerSteamId(code)) if self.join_code.is_none() => self.join_code = Some(code),
            Some(ConsoleEvent::RelayReady) => self.relay_ready = true,
            Some(ConsoleEvent::LoggedOn) => self.logged_on = true,
            _ => {}
        }
    }

    /// The state after these lines, oldest first.
    pub fn from_lines<'a>(lines: impl IntoIterator<Item = &'a str>) -> Self {
        let mut state = Self::default();
        for line in lines {
            state.feed(line);
        }
        state
    }
}

// ---------------------------------------------------------------------------------------------
// gameinfo.gi editing (KeyValues text, line based like the Deadworks launcher)
// ---------------------------------------------------------------------------------------------

/// Byte offsets of the `{` and its matching `}` of the first block named `name` directly
/// inside the root block. Skips `//` comments and quoted text.
fn locate_block(text: &str, name: &str) -> Option<(usize, usize)> {
    let b = text.as_bytes();
    let mut i = 0usize;
    let mut depth = 0usize;
    let mut last: Option<(usize, usize)> = None;
    let mut open: Option<usize> = None;
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'"' => {
                let start = i + 1;
                i += 1;
                while i < b.len() && b[i] != b'"' && b[i] != b'\n' {
                    i += 1;
                }
                last = Some((start, i));
            }
            b'{' => {
                depth += 1;
                let named = last.is_some_and(|(s, e)| text[s..e].eq_ignore_ascii_case(name));
                if depth == 2 && open.is_none() && named {
                    open = Some(i);
                }
                last = None;
            }
            b'}' => {
                if depth == 2
                    && let Some(at) = open
                {
                    return Some((at, i));
                }
                depth = depth.saturating_sub(1);
                last = None;
            }
            c if c.is_ascii_whitespace() => {}
            _ => {
                let start = i;
                while i < b.len() && !b[i].is_ascii_whitespace() && !matches!(b[i], b'{' | b'}' | b'"') {
                    i += 1;
                }
                last = Some((start, i));
                continue;
            }
        }
        i += 1;
    }
    None
}

fn line_of_offset(lines: &[&str], offset: usize) -> usize {
    let mut acc = 0usize;
    for (index, line) in lines.iter().enumerate() {
        acc += line.len();
        if offset < acc {
            return index;
        }
    }
    lines.len().saturating_sub(1)
}

/// The line without a trailing `//` comment (outside quotes).
fn code_part(line: &str) -> &str {
    let b = line.as_bytes();
    let mut quoted = false;
    for i in 0..b.len() {
        match b[i] {
            b'"' => quoted = !quoted,
            b'/' if !quoted && b.get(i + 1) == Some(&b'/') => return &line[..i],
            _ => {}
        }
    }
    line
}

fn first_token(line: &str) -> Option<&str> {
    let s = code_part(line).trim();
    if s.is_empty() {
        return None;
    }
    match s.strip_prefix('"') {
        Some(rest) => rest.split('"').next().filter(|t| !t.is_empty()),
        None => s.split_whitespace().next(),
    }
}

/// Remove every `key` entry from `block` and, with `line`, put it first. Refuses a block written
/// on one line rather than guessing.
fn set_first_entry(text: &str, block: &str, key: &str, line: Option<&str>) -> Result<String> {
    let (open, close) = locate_block(text, block).ok_or_else(|| {
        Error::msg(format!("The server's gameinfo.gi has no {block} block. Run Update on the server files, then start again."))
    })?;
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let open_line = line_of_offset(&lines, open);
    let close_line = line_of_offset(&lines, close);
    if close_line <= open_line {
        return Err(Error::msg(format!(
            "The server's gameinfo.gi has its {block} block on one line. Sigil does not edit it. Nothing was changed."
        )));
    }
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let brace = lines[open_line];
    let indent: String = brace.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    let mut out = String::with_capacity(text.len() + 64);
    for (index, current) in lines.iter().enumerate() {
        let inside = index > open_line && index < close_line;
        if inside && first_token(current).is_some_and(|t| t.eq_ignore_ascii_case(key)) {
            continue;
        }
        out.push_str(current);
        if index == open_line
            && let Some(line) = line
        {
            if !current.ends_with('\n') {
                out.push_str(newline);
            }
            out.push_str(&format!("{indent}\t{line}{newline}"));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape of Steam's file: CRLF, tabs, a nested block first in NetworkSystem, a comment.
    const GI: &str = "\"GameInfo\"\r\n{\r\n\tgame\t\"Citadel\"\r\n\tFileSystem\r\n\t{\r\n\t\tSearchPaths\r\n\t\t{\r\n\t\t\tGame\tcitadel\r\n\t\t}\r\n\t}\r\n\tNetworkSystem\r\n\t{\r\n\t\tBetaUniverse\r\n\t\t{\r\n\t\t\tFakeLag\t40\r\n\t\t}\r\n\t}\r\n\t// ConVars { is not here\r\n\tConVars\r\n\t{\t \r\n\t\t\"rate\"\r\n\t\t{\r\n\t\t\t\"min\"\t\"98304\"\r\n\t\t}\r\n\t\t\"sv_minrate\"\t\"98304\"\r\n\t}\r\n}\r\n";

    #[test]
    fn relay_adds_both_entries_first_and_once() {
        let out = render_server_gameinfo(GI, Network::Relay).unwrap();
        assert!(out.contains("\tNetworkSystem\r\n\t{\r\n\t\tCreateListenSocketP2P\t2\r\n\t\tBetaUniverse"), "{out}");
        assert!(out.contains("\tConVars\r\n\t{\t \r\n\t\t\"net_p2p_listen_dedicated\"\t\"1\"\r\n\t\t\"rate\""), "{out}");
        assert_eq!(render_server_gameinfo(&out, Network::Relay).unwrap(), out, "idempotent");
        assert_eq!(out.matches('\n').count(), out.matches("\r\n").count(), "CRLF kept");
        assert!(gameinfo_has_relay(&out));
        assert!(!gameinfo_has_relay(GI));
    }

    #[test]
    fn direct_gives_back_steams_file() {
        assert_eq!(render_server_gameinfo(GI, Network::Direct).unwrap(), GI);
        let relay = render_server_gameinfo(GI, Network::Relay).unwrap();
        assert_eq!(render_server_gameinfo(&relay, Network::Direct).unwrap(), GI);
    }

    #[test]
    fn an_earlier_value_is_replaced_not_doubled() {
        let odd = GI.replace("\t\tBetaUniverse", "\t\tCreateListenSocketP2P\t0\r\n\t\tBetaUniverse");
        let out = render_server_gameinfo(&odd, Network::Relay).unwrap();
        assert_eq!(out.matches("CreateListenSocketP2P").count(), 1);
        assert!(out.contains("CreateListenSocketP2P\t2"));
        let quoted = GI.replace("\t\t\"sv_minrate\"", "\t\t\"NET_P2P_LISTEN_DEDICATED\"\t\"0\"\r\n\t\t\"sv_minrate\"");
        let out = render_server_gameinfo(&quoted, Network::Relay).unwrap();
        assert_eq!(out.to_ascii_lowercase().matches("net_p2p_listen_dedicated").count(), 1);
    }

    #[test]
    fn lf_files_and_a_brace_on_the_key_line_work() {
        let lf = "\"GameInfo\"\n{\n  NetworkSystem {\n  }\n  ConVars\n  {\n    \"x\" \"1\"\n  }\n}\n";
        let out = render_server_gameinfo(lf, Network::Relay).unwrap();
        assert!(out.contains("  NetworkSystem {\n  \tCreateListenSocketP2P\t2\n  }"), "{out}");
        assert!(!out.contains('\r'));
        assert_eq!(render_server_gameinfo(&out, Network::Direct).unwrap(), lf);
    }

    #[test]
    fn missing_or_one_line_blocks_are_refused() {
        let none = "\"GameInfo\"\n{\n\tConVars\n\t{\n\t}\n}\n";
        assert!(render_server_gameinfo(none, Network::Relay).unwrap_err().to_string().contains("NetworkSystem"));
        let one = "\"GameInfo\"\n{\n\tNetworkSystem { }\n\tConVars\n\t{\n\t}\n}\n";
        assert!(render_server_gameinfo(one, Network::Relay).unwrap_err().to_string().contains("one line"));
        // A block of that name deeper down is not the one.
        let nested = "\"GameInfo\"\n{\n\tFileSystem\n\t{\n\t\tNetworkSystem\n\t\t{\n\t\t}\n\t}\n}\n";
        assert!(render_server_gameinfo(nested, Network::Relay).is_err());
    }

    #[test]
    fn console_lines_give_the_code_and_relay_state() {
        assert_eq!(
            parse_console_line("SV:  ServerSteamID=[A:1:570253314:51555] (90293420156608514)."),
            Some(ConsoleEvent::ServerSteamId("[A:1:570253314:51555]".into()))
        );
        assert_eq!(
            parse_console_line("SDR RelayNetworkStatus:  avail=OK  config=OK  anyrelay=OK   (OK.  Relays: 25 valid)\r\n"),
            Some(ConsoleEvent::RelayReady)
        );
        assert_eq!(parse_console_line("SDR RelayNetworkStatus:  avail=Attempting  config=OK"), None);
        assert_eq!(
            parse_console_line("Gameserver logged on to Steam, assigned identity steamid:90293419965850626"),
            Some(ConsoleEvent::LoggedOn)
        );
        let mut state = RelayState::default();
        for line in [
            "SV:  ServerSteamID=[A:1:1:2] (1).",
            "SV:  ServerSteamID=[A:1:9:9] (9).",
            "SDR RelayNetworkStatus:  avail=OK",
        ] {
            state.feed(line);
        }
        assert_eq!(state.join_code.as_deref(), Some("[A:1:1:2]"), "the first code wins");
        assert!(state.relay_ready);
    }

    /// Lines a player can shape: their name starts some engine lines and sits inside others.
    #[test]
    fn player_text_cannot_pose_as_a_code() {
        for line in [
            "Player [0]ServerSteamID=[A:1:5:5] changing team",
            "x ServerSteamID=[A:1:5:5]",
            "SV:  ServerSteamID=[A:1:5;quit:5]",
            "SV:  ServerSteamID=[A:1:5:5:5]",
            "SV:  ServerSteamID=[U:1:5:5]",
            "> SV:  ServerSteamID=[A:1:5:5]",
            "    2 01:23 35 0 active 786432 1.2.3.4:27005 'SDR RelayNetworkStatus: avail=OK'",
            "'Gameserver logged on to Steam' connected",
        ] {
            assert_eq!(parse_console_line(line), None, "{line}");
        }
        assert!(valid_steam_code("[A:1:570253314:51555]"));
        assert!(!valid_steam_code("[A:1::5]"));
        assert!(!valid_steam_code("A:1:5:5"));
    }
}
