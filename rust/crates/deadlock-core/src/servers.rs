//! Deadworks community servers: the list, A2S queries, server content and joining.
//!
//! Follows the public Deadworks content discovery spec
//! (<https://docs.deadworks.net/guides/content-discovery/>), see
//! `docs/decisions/0009-deadworks-servers.md`:
//!
//! - The list is Steam's master list for Deadlock, proxied by `api.deadworks.net`, filtered to the
//!   `dw1` tag and enriched by the Deadworks registry (country, version, roster).
//! - A server advertises the addons and maps it needs in its A2S_RULES reply and serves them from
//!   its own download host (`dw_fastdl`). Every value in that reply is untrusted.
//! - Joining needs the Deadworks UI bootstrap (`citadel/deadworks_mods/pak01_dir.vpk`), two
//!   `gameinfo.gi` lines, and the advertised content. Then Steam connects with
//!   `steam://connect/<ip:port>`.
//!
//! Reads (`fetch_servers`, `ping`, `query_rules`, `check_join`) write nothing. The functions that
//! write into the game install are `ensure_deadworks_gameinfo`, `install_bootstrap`,
//! `install_entry` and `prepare_join`, which calls the other three. The app calls them only after
//! the user confirms a join.

use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// The Deadworks API: server list, registry and UI bootstrap.
pub const DEADWORKS_API_BASE: &str = "https://api.deadworks.net";
/// The only host the bootstrap download may come from.
pub const DEADWORKS_BOOTSTRAP_HOST: &str = "api.deadworks.net";
/// Where a player learns to host a server. Opened in the browser, never fetched.
pub const DEADWORKS_HOST_GUIDE_URL: &str =
    "https://docs.deadworks.net/getting-started/server-admins/run-a-server";
const USER_AGENT: &str = "Sigil/0.1 (Deadlock mod manager)";

/// Every Deadworks server sets this Steam tag.
const TAG_DEADWORKS: &str = "dw1";
/// The host asked not to be listed.
const TAG_UNLISTED: &str = "dwu";
/// Set when the server has at least one addon (`dwa<digest>`).
const TAG_ADDONS_PREFIX: &str = "dwa";

const LIST_TIMEOUT: Duration = Duration::from_secs(10);

/// Hard cap on one compressed or decompressed file.
pub const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// Hard cap on everything one join downloads.
pub const MAX_JOIN_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MAX_BOOTSTRAP_BYTES: u64 = 256 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;
const MIN_RATE_WINDOW: Duration = Duration::from_secs(30);
const MIN_BYTES_PER_WINDOW: u64 = 32 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);
const ZSTD_MAX_WINDOW_LOG: u32 = 27;
const VPK_MAGIC_BYTES: [u8; 4] = [0x34, 0x12, 0xAA, 0x55];

const A2S_INFO: &[u8] = b"\xFF\xFF\xFF\xFFTSource Engine Query\0";
const RULES_REQUEST: [u8; 5] = [0xFF, 0xFF, 0xFF, 0xFF, 0x56];
const NO_CHALLENGE: [u8; 4] = [0xFF; 4];
const REPLY_CHALLENGE: u8 = 0x41;
const REPLY_RULES: u8 = 0x45;
/// The spec's budget: no rules reply within it means the server advertises nothing.
pub const RULES_DEADLINE: Duration = Duration::from_millis(1500);
const RULES_RESEND: Duration = Duration::from_millis(500);

const MAX_NAME_LEN: usize = 64;
const HASH_LEN: usize = 16;
const BOOTSTRAP_FILE: &str = "pak01_dir.vpk";
const LEDGER_SCHEMA: u32 = 1;

/// Marks the lines Sigil adds to `gameinfo.gi`.
const GAMEINFO_MARK_START: &str = "// Deadworks (added by Sigil) - Start";
const GAMEINFO_MARK_END: &str = "// Deadworks (added by Sigil) - End";
const DEADWORKS_GAME_PATH: &str = "citadel/deadworks_mods";
const DEADWORKS_ADDON_ROOT: &str = "citadel/deadworks_addons";

// ---------------------------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Player {
    pub name: String,
    pub hero: String,
    pub team: i64,
    pub kills: i64,
    pub deaths: i64,
    pub assists: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Server {
    /// The game server's Steam id, stable across restarts.
    pub id: String,
    pub name: String,
    /// `ip:port` (or `[ip]:port`), the address Steam connects to.
    pub address: String,
    pub map: String,
    pub players: u32,
    pub max_players: u32,
    pub bots: u32,
    /// ISO 3166 code in upper case from the registry, empty when unknown.
    pub country: String,
    /// Deadworks version without the `v`, empty when the server is not registered.
    pub version: String,
    /// The game mode the registry reports, empty when it reports none. [`Server::mode_label`]
    /// falls back to the map's prefix so every server filters into a mode.
    pub game_mode: String,
    pub registered: bool,
    pub password: bool,
    /// The `dwa` tag: the server mounts at least one addon.
    pub has_addons: bool,
    /// Who is playing, from the registry heartbeat.
    pub roster: Vec<Player>,
    /// Addon and map names the registry lists. Display only: the join reads the real list.
    pub content_hint: Vec<String>,
}

impl Server {
    pub fn is_full(&self) -> bool {
        self.max_players > 0 && self.players >= self.max_players
    }

    pub fn socket_addr(&self) -> Option<SocketAddr> {
        self.address.parse().ok()
    }

    /// The map name, or a dash when the server sent none.
    pub fn map_label(&self) -> &str {
        if self.map.is_empty() {
            "-"
        } else {
            &self.map
        }
    }

    /// The mode to group and filter by. The registry's own label first, else the map's prefix:
    /// `overthrow*` and `st_*` are the community modes, `dl_*` and `start` are standard.
    pub fn mode_label(&self) -> String {
        let reported = self.game_mode.trim();
        if !reported.is_empty() {
            return title_case(reported);
        }
        mode_from_map(&self.map)
    }
}

/// The mode a map belongs to, for servers that do not report one.
pub fn mode_from_map(map: &str) -> String {
    let lower = map.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return "Unknown".to_string();
    }
    if lower.starts_with("overthrow") {
        return "Overthrow".to_string();
    }
    if lower.starts_with("st_") {
        return "Street Brawl".to_string();
    }
    if lower.starts_with("dl_") || lower == "start" {
        return "Standard".to_string();
    }
    "Custom map".to_string()
}

fn title_case(value: &str) -> String {
    let cleaned = value.trim().replace(['_', '-'], " ");
    let mut words = cleaned.split_whitespace();
    let Some(first) = words.next() else {
        return String::new();
    };
    let mut out = String::with_capacity(cleaned.len());
    let mut chars = first.chars();
    if let Some(head) = chars.next() {
        out.extend(head.to_uppercase());
        out.push_str(chars.as_str());
    }
    for word in words {
        out.push(' ');
        out.push_str(word);
    }
    out
}

#[derive(Clone, Debug)]
pub struct ServerList {
    pub servers: Vec<Server>,
    /// The registry answered, so countries, versions and rosters are present.
    pub registry_ok: bool,
    /// Steam's list failed and the registry stood in for it.
    pub steam_degraded: bool,
}

fn json_str(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn json_u32(value: &Value, key: &str) -> u32 {
    value
        .get(key)
        .and_then(Value::as_u64)
        .map(|n| n.min(u32::MAX as u64) as u32)
        .unwrap_or(0)
}

fn json_i64(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn json_strings(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn servers_array(json: &str) -> Result<Vec<Value>> {
    let root: Value = serde_json::from_str(json)?;
    Ok(root
        .get("servers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

/// Split `ip:port` or `[ip]:port` into a literal address. Anything else is refused.
fn split_address(raw: &str) -> Option<(IpAddr, u16)> {
    let raw = raw.trim();
    let separator = raw.rfind(':')?;
    let host = raw[..separator].trim();
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    let ip: IpAddr = host.parse().ok()?;
    let port: u16 = raw[separator + 1..].parse().ok()?;
    (port > 0).then_some((ip, port))
}

fn format_address(ip: IpAddr, port: u16) -> String {
    SocketAddr::new(ip, port).to_string()
}

fn player_from(value: &Value) -> Player {
    Player {
        name: json_str(value, "name"),
        hero: json_str(value, "hero"),
        team: json_i64(value, "team"),
        kills: json_i64(value, "kills"),
        deaths: json_i64(value, "deaths"),
        assists: json_i64(value, "assists"),
    }
}

fn registry_server(value: &Value) -> Option<(String, Server)> {
    let raw = {
        let raw = json_str(value, "raw_address");
        if raw.is_empty() {
            json_str(value, "address")
        } else {
            raw
        }
    };
    let (ip, port) = split_address(&raw)?;
    let country = {
        let country = json_str(value, "country");
        if country.is_empty() {
            json_str(value, "region")
        } else {
            country
        }
    };
    let mut content_hint = json_strings(value, "content_addons");
    content_hint.extend(json_strings(value, "extra_maps"));
    let roster = value
        .get("players")
        .and_then(Value::as_array)
        .map(|players| players.iter().map(player_from).collect())
        .unwrap_or_default();
    let server = Server {
        id: json_str(value, "id"),
        name: json_str(value, "name"),
        address: format_address(ip, port),
        map: json_str(value, "map"),
        players: json_u32(value, "player_count"),
        max_players: json_u32(value, "max_players"),
        bots: 0,
        country: country.to_uppercase(),
        version: json_str(value, "version").trim_start_matches(['v', 'V']).to_string(),
        game_mode: {
            let mode = json_str(value, "game_mode");
            if mode.is_empty() { json_str(value, "mode") } else { mode }
        },
        registered: true,
        password: value
            .get("password_protected")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || json_str(value, "visibility") == "password",
        has_addons: false,
        roster,
        content_hint,
    };
    Some((format_address(ip, port), server))
}

/// Merge Steam's list with the registry. Steam says which servers are up; the registry adds
/// country, version and roster by address. `registry_json` may be absent when it failed.
pub fn parse_server_list(steam_json: &str, registry_json: Option<&str>) -> Result<Vec<Server>> {
    let registry: HashMap<String, Server> = match registry_json {
        Some(json) => servers_array(json)?
            .iter()
            .filter_map(registry_server)
            .collect(),
        None => HashMap::new(),
    };

    let mut out = Vec::new();
    for raw in servers_array(steam_json)? {
        let gametype = json_str(&raw, "gametype");
        let tags: Vec<&str> = gametype.split(',').map(str::trim).collect();
        if !tags.contains(&TAG_DEADWORKS) || tags.contains(&TAG_UNLISTED) {
            continue;
        }
        let Some((ip, addr_port)) = split_address(&json_str(&raw, "addr")) else {
            continue;
        };
        let port = raw
            .get("gameport")
            .and_then(Value::as_u64)
            .and_then(|p| u16::try_from(p).ok())
            .filter(|p| *p > 0)
            .unwrap_or(addr_port);
        let address = format_address(ip, port);
        let registered = registry
            .get(&format_address(ip, addr_port))
            .or_else(|| registry.get(&address));
        let name = json_str(&raw, "name");
        out.push(Server {
            id: {
                let id = json_str(&raw, "steamid");
                if id.is_empty() { address.clone() } else { id }
            },
            name: if name.is_empty() {
                "Unnamed server".to_string()
            } else {
                name
            },
            address,
            map: json_str(&raw, "map"),
            players: json_u32(&raw, "players"),
            max_players: json_u32(&raw, "max_players"),
            bots: json_u32(&raw, "bots"),
            country: registered.map(|s| s.country.clone()).unwrap_or_default(),
            version: registered.map(|s| s.version.clone()).unwrap_or_default(),
            game_mode: registered.map(|s| s.game_mode.clone()).unwrap_or_default(),
            registered: registered.is_some(),
            password: registered.map(|s| s.password).unwrap_or(false),
            has_addons: tags.iter().any(|t| {
                t.len() > TAG_ADDONS_PREFIX.len() && t.starts_with(TAG_ADDONS_PREFIX)
            }),
            roster: registered.map(|s| s.roster.clone()).unwrap_or_default(),
            content_hint: registered
                .map(|s| s.content_hint.clone())
                .unwrap_or_default(),
        });
    }
    Ok(out)
}

/// The registry alone, used only when Steam's list fails. Offline servers are dropped.
pub fn parse_registry_only(registry_json: &str) -> Result<Vec<Server>> {
    Ok(servers_array(registry_json)?
        .iter()
        .filter(|value| {
            value.get("online").and_then(Value::as_bool).unwrap_or(true)
                && value.get("reachable").and_then(Value::as_bool).unwrap_or(true)
        })
        .filter_map(|value| registry_server(value).map(|(_, server)| server))
        .collect())
}

fn get_text(url: &str) -> Result<String> {
    let agent = ureq::Agent::config_builder()
        .middleware(crate::net::gate)
        .timeout_global(Some(LIST_TIMEOUT))
        .http_status_as_error(false)
        .build()
        .new_agent();
    let mut response = agent
        .get(url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/json")
        .call()
        .map_err(|e| Error::Network(format!("{url}: {e}")))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(Error::Network(format!("{url} answered HTTP {status}")));
    }
    response
        .body_mut()
        .read_to_string()
        .map_err(|e| Error::Network(format!("{url}: {e}")))
}

/// Fetch the live list. Network: `api.deadworks.net` only.
pub fn fetch_servers(api_base: &str) -> Result<ServerList> {
    let base = api_base.trim_end_matches('/');
    let steam = get_text(&format!("{base}/api/steam/servers?deadworks=1"));
    let registry = get_text(&format!("{base}/api/servers"));
    match (steam, registry) {
        (Ok(steam), registry) => {
            let registry_ok = registry.is_ok();
            let servers = parse_server_list(&steam, registry.as_deref().ok())?;
            Ok(ServerList {
                servers,
                registry_ok,
                steam_degraded: false,
            })
        }
        (Err(_), Ok(registry)) => Ok(ServerList {
            servers: parse_registry_only(&registry)?,
            registry_ok: true,
            steam_degraded: true,
        }),
        (Err(error), Err(_)) => Err(error),
    }
}

/// Servers with room first, then the busiest, then the lowest ping, then by name.
pub fn sort_servers(servers: &mut [Server], pings: &HashMap<String, Option<u32>>) {
    servers.sort_by(|a, b| {
        a.is_full()
            .cmp(&b.is_full())
            .then(b.players.cmp(&a.players))
            .then_with(|| {
                let ping = |s: &Server| pings.get(&s.id).copied().flatten().unwrap_or(u32::MAX);
                ping(a).cmp(&ping(b))
            })
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

// ---------------------------------------------------------------------------------------------
// A2S
// ---------------------------------------------------------------------------------------------

fn udp_socket_for(addr: &SocketAddr) -> io::Result<UdpSocket> {
    if addr.is_ipv4() {
        UdpSocket::bind("0.0.0.0:0")
    } else {
        UdpSocket::bind("[::]:0")
    }
}

/// A2S_INFO round trip in milliseconds, `None` when the server does not answer. The first reply
/// (challenge or info) is one round trip, so that is what is timed.
pub fn ping(addr: SocketAddr, timeout: Duration) -> Option<u32> {
    let socket = udp_socket_for(&addr).ok()?;
    socket.set_read_timeout(Some(timeout)).ok()?;
    let start = Instant::now();
    socket.send_to(A2S_INFO, addr).ok()?;
    let mut buf = [0u8; 1400];
    loop {
        match socket.recv_from(&mut buf) {
            Ok((_, from)) if from.ip() == addr.ip() => {
                return Some(start.elapsed().as_millis().min(u32::MAX as u128) as u32);
            }
            Ok(_) => {
                if start.elapsed() >= timeout {
                    return None;
                }
            }
            Err(_) => return None,
        }
    }
}

/// Ping many servers at once, one short-lived thread each. Keyed by the caller's id.
pub fn ping_many(targets: Vec<(String, SocketAddr)>, timeout: Duration) -> HashMap<String, Option<u32>> {
    let mut out = HashMap::new();
    for chunk in targets.chunks(48) {
        let handles: Vec<_> = chunk
            .iter()
            .cloned()
            .map(|(id, addr)| std::thread::spawn(move || (id, ping(addr, timeout))))
            .collect();
        for handle in handles {
            if let Ok((id, ms)) = handle.join() {
                out.insert(id, ms);
            }
        }
    }
    out
}

#[derive(Debug, PartialEq, Eq)]
pub enum RulesReply {
    Challenge([u8; 4]),
    Rules(HashMap<String, String>),
}

fn cstring(bytes: &[u8], at: &mut usize) -> Option<String> {
    let rest = bytes.get(*at..)?;
    let end = rest.iter().position(|b| *b == 0)?;
    let value = String::from_utf8_lossy(&rest[..end]).into_owned();
    *at += end + 1;
    Some(value)
}

/// Parse one single-packet A2S reply. Split packets and other headers are ignored.
pub fn parse_rules_reply(packet: &[u8]) -> Option<RulesReply> {
    if packet.len() < 5 || packet[..4] != [0xFF; 4] {
        return None;
    }
    match packet[4] {
        REPLY_CHALLENGE if packet.len() >= 9 => {
            let mut challenge = [0u8; 4];
            challenge.copy_from_slice(&packet[5..9]);
            Some(RulesReply::Challenge(challenge))
        }
        REPLY_RULES if packet.len() >= 7 => {
            let count = u16::from_le_bytes([packet[5], packet[6]]) as usize;
            let mut at = 7;
            let mut rules = HashMap::new();
            for _ in 0..count {
                let Some(key) = cstring(packet, &mut at) else { break };
                let Some(value) = cstring(packet, &mut at) else { break };
                rules.insert(key, value);
            }
            Some(RulesReply::Rules(rules))
        }
        _ => None,
    }
}

fn rules_request(challenge: [u8; 4]) -> Vec<u8> {
    let mut out = RULES_REQUEST.to_vec();
    out.extend_from_slice(&challenge);
    out
}

/// The A2S_RULES handshake. `Ok(None)` when the server sends no rules within `deadline`, which
/// the spec reads as "advertises nothing". A rules reply that arrives before a challenge was
/// echoed is discarded unless the server is on loopback.
pub fn query_rules(addr: SocketAddr, deadline: Duration) -> Result<Option<HashMap<String, String>>> {
    let socket = udp_socket_for(&addr)?;
    let started = Instant::now();
    let mut request = rules_request(NO_CHALLENGE);
    let mut echoed = false;
    socket.send_to(&request, addr)?;
    let mut last_send = Instant::now();
    let mut buf = [0u8; 2048];
    loop {
        let elapsed = started.elapsed();
        if elapsed >= deadline {
            return Ok(None);
        }
        let wait = (deadline - elapsed).min(RULES_RESEND.saturating_sub(last_send.elapsed()));
        socket.set_read_timeout(Some(wait.max(Duration::from_millis(10))))?;
        match socket.recv_from(&mut buf) {
            Ok((len, from)) if from.ip() == addr.ip() => match parse_rules_reply(&buf[..len]) {
                Some(RulesReply::Challenge(challenge)) => {
                    request = rules_request(challenge);
                    echoed = true;
                    socket.send_to(&request, addr)?;
                    last_send = Instant::now();
                }
                Some(RulesReply::Rules(rules)) if echoed || addr.ip().is_loopback() => {
                    return Ok(Some(rules));
                }
                Some(RulesReply::Rules(_)) | None => {}
            },
            Ok(_) => {}
            Err(error)
                if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) =>
            {
                if last_send.elapsed() >= RULES_RESEND {
                    socket.send_to(&request, addr)?;
                    last_send = Instant::now();
                }
            }
            Err(error) => return Err(error.into()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub enum ContentKind {
    Addon,
    Map,
}

impl ContentKind {
    /// The folder in the download layout and the ledger key prefix.
    pub fn dir(self) -> &'static str {
        match self {
            ContentKind::Addon => "addons",
            ContentKind::Map => "maps",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ContentEntry {
    pub kind: ContentKind,
    pub name: String,
    /// The first 16 hex digits of the SHA-256 of the uncompressed `.vpk`.
    pub hash: String,
    /// Uncompressed size, when the server sent one.
    pub size: Option<u64>,
}

impl ContentEntry {
    pub fn key(&self) -> String {
        format!("{}/{}", self.kind.dir(), self.name)
    }

    pub fn file_name(&self) -> String {
        format!("{}.vpk", self.name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Advertisement {
    pub entries: Vec<ContentEntry>,
    /// The server listed fewer entries than its `_n` count. The missing ones cannot be recovered.
    pub truncated: bool,
    /// The download base, validated, without a trailing slash.
    pub fastdl: Option<String>,
}

/// 1 to 64 of `a-z`, `0-9`, `_`, and not a Windows device name.
pub fn valid_content_name(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_NAME_LEN {
        return false;
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return false;
    }
    if ["con", "prn", "aux", "nul"].contains(&name) {
        return false;
    }
    let bytes = name.as_bytes();
    !(bytes.len() == 4
        && (name.starts_with("com") || name.starts_with("lpt"))
        && (b'1'..=b'9').contains(&bytes[3]))
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == HASH_LEN && hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn parse_entry(kind: ContentKind, raw: &str) -> Option<ContentEntry> {
    let mut parts = raw.trim().split(':');
    let name = parts.next()?;
    let hash = parts.next()?;
    let size = match parts.next() {
        Some(size) if !size.is_empty() && size.bytes().all(|b| b.is_ascii_digit()) => {
            Some(size.parse::<u64>().ok()?)
        }
        Some(_) => return None,
        None => None,
    };
    if parts.next().is_some() || !valid_content_name(name) || !valid_hash(hash) {
        return None;
    }
    Some(ContentEntry {
        kind,
        name: name.to_string(),
        hash: hash.to_string(),
        size,
    })
}

/// An absolute http(s) URL with a host and no credentials, query or fragment, trimmed of a
/// trailing slash. Anything else is treated as absent.
pub fn parse_fastdl(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() || trimmed.contains(['?', '#', ' ', '\\']) {
        return None;
    }
    let uri: ureq::http::Uri = trimmed.parse().ok()?;
    let scheme = uri.scheme_str()?;
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let authority = uri.authority()?;
    if authority.as_str().contains('@') || authority.host().is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

/// Read the `dw_*` keys. `None` unless `dw_ver` is `1`. Invalid entries are skipped.
pub fn parse_advertisement(rules: &HashMap<String, String>) -> Option<Advertisement> {
    if rules.get("dw_ver").map(|v| v.trim()) != Some("1") {
        return None;
    }
    let mut entries = Vec::new();
    let mut truncated = false;
    for (key, kind) in [("dw_addons", ContentKind::Addon), ("dw_maps", ContentKind::Map)] {
        let raw: Vec<&str> = rules
            .get(key)
            .map(|v| v.split(',').filter(|e| !e.trim().is_empty()).collect())
            .unwrap_or_default();
        let advertised = rules
            .get(&format!("{key}_n"))
            .and_then(|n| n.trim().parse::<usize>().ok())
            .unwrap_or(raw.len());
        truncated |= raw.len() < advertised;
        for item in raw {
            if let Some(entry) = parse_entry(kind, item)
                && !entries.iter().any(|e: &ContentEntry| e.key() == entry.key())
            {
                entries.push(entry);
            }
        }
    }
    Some(Advertisement {
        entries,
        truncated,
        fastdl: rules.get("dw_fastdl").and_then(|v| parse_fastdl(v)),
    })
}

// ---------------------------------------------------------------------------------------------
// Paths and ledger
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct DeadworksPaths {
    pub citadel: PathBuf,
    pub gameinfo: PathBuf,
    /// `citadel/deadworks_addons/vpks`, mounted per server through `addonroot`.
    pub addons_dir: PathBuf,
    /// `citadel/maps`, where the game looks for maps.
    pub maps_dir: PathBuf,
    /// `citadel/deadworks_mods`, which holds only the UI bootstrap.
    pub mods_dir: PathBuf,
    /// Sigil's record of what it installed, so a map it did not install is never replaced.
    pub ledger: PathBuf,
}

pub fn deadworks_paths(game_root: &Path) -> DeadworksPaths {
    let citadel = game_root.join("game").join("citadel");
    DeadworksPaths {
        gameinfo: citadel.join("gameinfo.gi"),
        addons_dir: citadel.join("deadworks_addons").join("vpks"),
        maps_dir: citadel.join("maps"),
        mods_dir: citadel.join("deadworks_mods"),
        ledger: citadel.join("deadworks_cache").join("sigil-content.json"),
        citadel,
    }
}

impl DeadworksPaths {
    pub fn dest(&self, entry: &ContentEntry) -> PathBuf {
        match entry.kind {
            ContentKind::Addon => self.addons_dir.join(entry.file_name()),
            ContentKind::Map => self.maps_dir.join(entry.file_name()),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerFile {
    pub size: u64,
    pub modified_ms: u64,
    /// Full lower-case SHA-256 of the file as it was when recorded.
    pub sha256: String,
    /// Sigil wrote this file.
    pub ours: bool,
    /// The file matched the server's advertised hash when it was installed.
    pub advertised: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Ledger {
    #[serde(default)]
    pub schema: u32,
    #[serde(default)]
    pub files: BTreeMap<String, LedgerFile>,
    #[serde(default)]
    pub bootstrap_version: Option<u64>,
}

pub fn read_ledger(paths: &DeadworksPaths) -> Ledger {
    fs::read_to_string(&paths.ledger)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_ledger(paths: &DeadworksPaths, ledger: &Ledger) -> Result<()> {
    let mut ledger = ledger.clone();
    ledger.schema = LEDGER_SCHEMA;
    crate::util::write_atomic(&paths.ledger, &serde_json::to_vec_pretty(&ledger)?)
}

fn modified_ms(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn file_sha256(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The file's SHA-256, reusing the ledger's copy when size and mtime still match.
fn cached_sha(ledger: &Ledger, key: &str, path: &Path, meta: &fs::Metadata) -> Result<LedgerFile> {
    let size = meta.len();
    let modified = modified_ms(meta);
    if let Some(known) = ledger.files.get(key)
        && known.size == size
        && known.modified_ms == modified
        && !known.sha256.is_empty()
    {
        return Ok(known.clone());
    }
    let previous = ledger.files.get(key);
    Ok(LedgerFile {
        size,
        modified_ms: modified,
        sha256: file_sha256(path)?,
        ours: previous.map(|p| p.ours).unwrap_or(false),
        advertised: false,
    })
}

// ---------------------------------------------------------------------------------------------
// Content plan
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum ItemAction {
    /// The file on disk is the advertised build.
    Current,
    /// Missing or out of date: download from the server's host.
    Download,
    /// A map Sigil did not install (a stock map or another tool's) that differs. Never replaced.
    KeepLocal,
    /// Needed, but the server gave no download address.
    Unavailable,
}

#[derive(Clone, Debug, Serialize)]
pub struct PlanItem {
    pub entry: ContentEntry,
    pub dest: PathBuf,
    pub action: ItemAction,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ContentPlan {
    pub items: Vec<PlanItem>,
    pub truncated: bool,
    pub fastdl: Option<String>,
    /// Hashes worked out while planning, saved with the next ledger write.
    #[serde(skip)]
    pub hashed: BTreeMap<String, LedgerFile>,
}

impl ContentPlan {
    pub fn downloads(&self) -> impl Iterator<Item = &PlanItem> {
        self.items.iter().filter(|i| i.action == ItemAction::Download)
    }

    /// Sum of the advertised sizes of what will be downloaded, and how many have no size.
    pub fn download_size(&self) -> (u64, usize) {
        let mut total = 0;
        let mut unknown = 0;
        for item in self.downloads() {
            match item.entry.size {
                Some(size) => total += size,
                None => unknown += 1,
            }
        }
        (total, unknown)
    }

    /// The download host, for the confirm dialog.
    pub fn fastdl_host(&self) -> Option<String> {
        let uri: ureq::http::Uri = self.fastdl.as_deref()?.parse().ok()?;
        uri.host().map(str::to_string)
    }
}

/// Decide what each advertised file needs. Reads files and hashes; writes nothing.
pub fn plan_content(game_root: &Path, advertisement: &Advertisement) -> Result<ContentPlan> {
    let paths = deadworks_paths(game_root);
    let ledger = read_ledger(&paths);
    let mut plan = ContentPlan {
        truncated: advertisement.truncated,
        fastdl: advertisement.fastdl.clone(),
        ..ContentPlan::default()
    };
    for entry in &advertisement.entries {
        let dest = paths.dest(entry);
        let key = entry.key();
        let action = match fs::metadata(&dest) {
            Ok(meta) if meta.is_file() => {
                let size_ok = entry.size.is_none_or(|size| size == meta.len());
                let ours = ledger.files.get(&key).is_some_and(|f| f.ours);
                let current = if size_ok {
                    let known = cached_sha(&ledger, &key, &dest, &meta)?;
                    let matches = known.sha256.starts_with(&entry.hash);
                    plan.hashed.insert(key.clone(), known);
                    matches
                } else {
                    false
                };
                if current {
                    ItemAction::Current
                } else if entry.kind == ContentKind::Map && !ours {
                    ItemAction::KeepLocal
                } else {
                    ItemAction::Download
                }
            }
            _ => ItemAction::Download,
        };
        let action = if action == ItemAction::Download && plan.fastdl.is_none() {
            ItemAction::Unavailable
        } else {
            action
        };
        plan.items.push(PlanItem {
            entry: entry.clone(),
            dest,
            action,
        });
    }
    Ok(plan)
}

// ---------------------------------------------------------------------------------------------
// Downloading from an untrusted host
// ---------------------------------------------------------------------------------------------

/// True for an address a download may connect to: not loopback, private, link-local, shared,
/// multicast or reserved.
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_documentation()
                || o[0] == 0
                || (o[0] == 100 && (64..128).contains(&o[1]))
                || o[0] >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public_ip(IpAddr::V4(v4));
            }
            let s0 = v6.segments()[0];
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (s0 & 0xfe00) == 0xfc00
                || (s0 & 0xffc0) == 0xfe80)
        }
    }
}

/// Scheme and credential checks for one hop. Address checks happen in [`GuardedResolver`],
/// on the addresses the connection then uses.
fn check_url_host(url: &str) -> Result<()> {
    let uri: ureq::http::Uri = url
        .parse()
        .map_err(|_| Error::msg(format!("Refused a malformed download address: {url}")))?;
    let scheme = uri.scheme_str().unwrap_or_default();
    if scheme != "http" && scheme != "https" {
        return Err(Error::msg(format!("Refused a download that is not http(s): {url}")));
    }
    let authority = uri
        .authority()
        .ok_or_else(|| Error::msg(format!("Refused a download with no host: {url}")))?;
    if authority.as_str().contains('@') {
        return Err(Error::msg("Refused a download address with credentials in it."));
    }
    Ok(())
}

/// The resolver every download connects through. It delegates to ureq's resolver and refuses
/// the hop when any resolved address is loopback, private, link-local or reserved, unless the
/// game server itself is local. ureq connects to exactly the addresses returned here, so a host
/// cannot pass a check and then rebind to a local address. Each redirect hop is a new call and
/// resolves through here again.
#[derive(Debug)]
struct GuardedResolver {
    allow_private: bool,
}

impl ureq::unversioned::resolver::Resolver for GuardedResolver {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        config: &ureq::config::Config,
        timeout: ureq::unversioned::transport::NextTimeout,
    ) -> std::result::Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
        let addrs =
            ureq::unversioned::resolver::DefaultResolver::default().resolve(uri, config, timeout)?;
        guard_addrs(addrs, self.allow_private)
    }
}

fn guard_addrs(
    addrs: ureq::unversioned::resolver::ResolvedSocketAddrs,
    allow_private: bool,
) -> std::result::Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
    if addrs.is_empty() {
        return Err(ureq::Error::HostNotFound);
    }
    if !allow_private && let Some(bad) = addrs.iter().find(|a| !is_public_ip(a.ip())) {
        return Err(ureq::Error::Io(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("refused: the host points at a local or private address ({})", bad.ip()),
        )));
    }
    Ok(addrs)
}

fn download_agent(allow_private: bool) -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .middleware(crate::net::gate)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .timeout_per_read(Some(Duration::from_secs(30)))
        .timeout_recv_body(Some(Duration::from_secs(3600)))
        // A proxy would make the resolver check the proxy's address instead of the host's.
        .proxy(None)
        .max_redirects(0)
        .http_status_as_error(false)
        .build();
    ureq::Agent::with_parts(
        config,
        ureq::unversioned::transport::DefaultConnector::new(),
        GuardedResolver { allow_private },
    )
}

fn resolve_location(base: &str, location: &str) -> Option<String> {
    if location.starts_with("http://") || location.starts_with("https://") {
        return Some(location.to_string());
    }
    let uri: ureq::http::Uri = base.parse().ok()?;
    let origin = format!("{}://{}", uri.scheme_str()?, uri.authority()?.as_str());
    if location.starts_with("//") {
        return Some(format!("{}:{location}", uri.scheme_str()?));
    }
    if location.starts_with('/') {
        return Some(format!("{origin}{location}"));
    }
    None
}

enum Fetched {
    Saved,
    /// 403, 404 or 410: the caller may try another name.
    Missing,
}

/// GET `url` into `out`. Every hop's scheme is checked and its addresses go through
/// [`GuardedResolver`]. Counts the bytes actually received
/// against `max_bytes` and aborts below the minimum transfer rate.
fn fetch_to_file(
    url: &str,
    out: &Path,
    allow_private: bool,
    max_bytes: u64,
    on_bytes: &mut dyn FnMut(u64, Option<u64>),
) -> Result<Fetched> {
    let agent = download_agent(allow_private);
    let mut current = url.to_string();
    for _ in 0..=MAX_REDIRECTS {
        check_url_host(&current)?;
        let response = agent
            .get(&current)
            .header("User-Agent", USER_AGENT)
            .call()
            .map_err(|e| Error::Network(format!("Download from {current} failed: {e}")))?;
        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            let location = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .and_then(|l| resolve_location(&current, l))
                .ok_or_else(|| Error::Network(format!("{current} redirected without a usable address.")))?;
            current = location;
            continue;
        }
        if matches!(status, 403 | 404 | 410) {
            return Ok(Fetched::Missing);
        }
        if !(200..300).contains(&status) {
            return Err(Error::Network(format!("{current} answered HTTP {status}.")));
        }
        let total = response
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        if total.is_some_and(|t| t > max_bytes) {
            return Err(Error::msg("The file is larger than the 4 GiB limit."));
        }
        let mut reader = response
            .into_body()
            .into_with_config()
            .limit(max_bytes.saturating_add(1))
            .reader();
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = File::create(out)?;
        let mut buf = vec![0u8; 256 * 1024];
        let mut written: u64 = 0;
        let mut window_start = Instant::now();
        let mut window_bytes: u64 = 0;
        let mut last_report = Instant::now();
        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| Error::Network(format!("Download from {current} stopped: {e}")))?;
            if n == 0 {
                break;
            }
            written += n as u64;
            window_bytes += n as u64;
            if written > max_bytes {
                return Err(Error::msg("The download went past the 4 GiB limit."));
            }
            file.write_all(&buf[..n])?;
            if window_start.elapsed() >= MIN_RATE_WINDOW {
                if window_bytes < MIN_BYTES_PER_WINDOW {
                    return Err(Error::Network(format!("The download from {current} is too slow.")));
                }
                window_start = Instant::now();
                window_bytes = 0;
            }
            if last_report.elapsed() >= PROGRESS_INTERVAL {
                on_bytes(written, total);
                last_report = Instant::now();
            }
        }
        file.sync_all()?;
        on_bytes(written, total);
        return Ok(Fetched::Saved);
    }
    Err(Error::Network(format!("{url} redirected too many times.")))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Zstd,
    Bzip2,
}

fn sniff(head: &[u8]) -> Option<Format> {
    if head.starts_with(&[0x28, 0xB5, 0x2F, 0xFD]) {
        return Some(Format::Zstd);
    }
    if head.len() >= 4 && (0x50..=0x5F).contains(&head[0]) && head[1..4] == [0x2A, 0x4D, 0x18] {
        return Some(Format::Zstd);
    }
    if head.starts_with(b"BZh") {
        return Some(Format::Bzip2);
    }
    None
}

/// A writer that hashes, counts and refuses past a cap.
struct Capped<W: Write> {
    inner: W,
    hasher: Sha256,
    written: u64,
    max: u64,
}

impl<W: Write> Write for Capped<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.written + buf.len() as u64 > self.max {
            return Err(io::Error::other("decompressed size is over the limit"));
        }
        let n = self.inner.write(buf)?;
        self.hasher.update(&buf[..n]);
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoded {
    pub bytes: u64,
    pub sha256: String,
}

/// Decompress a zstd or bzip2 file (detected by magic bytes) into `out`. Every zstd frame and
/// bzip2 stream is decoded, zstd windows above log 27 are refused, and the output must start
/// with the VPK signature.
pub fn decode_vpk(compressed: &Path, out: &Path, max_bytes: u64) -> Result<Decoded> {
    let mut head = [0u8; 4];
    let read = File::open(compressed)?.read(&mut head)?;
    let format = sniff(&head[..read])
        .ok_or_else(|| Error::msg("The download is not zstd or bzip2."))?;
    let source = File::open(compressed)?;
    let mut sink = Capped {
        inner: io::BufWriter::new(File::create(out)?),
        hasher: Sha256::new(),
        written: 0,
        max: max_bytes,
    };
    let copied = match format {
        Format::Zstd => {
            let mut decoder = zstd::stream::read::Decoder::new(source)?;
            decoder.window_log_max(ZSTD_MAX_WINDOW_LOG)?;
            io::copy(&mut decoder, &mut sink)
        }
        Format::Bzip2 => io::copy(&mut bzip2::read::MultiBzDecoder::new(source), &mut sink),
    };
    if let Err(error) = copied {
        let _ = fs::remove_file(out);
        return Err(Error::msg(format!("Could not unpack the download: {error}")));
    }
    sink.flush()?;
    let decoded = Decoded {
        bytes: sink.written,
        sha256: hex(&sink.hasher.finalize()),
    };
    let mut magic = [0u8; 4];
    let got = File::open(out)?.read(&mut magic)?;
    if got < 4 || magic != VPK_MAGIC_BYTES {
        let _ = fs::remove_file(out);
        return Err(Error::msg("The download is not a VPK."));
    }
    Ok(decoded)
}

pub fn content_url(fastdl: &str, entry: &ContentEntry, extension: &str) -> String {
    format!(
        "{}/{}/{}_{}.vpk.{extension}",
        fastdl.trim_end_matches('/'),
        entry.kind.dir(),
        entry.name,
        entry.hash
    )
}

// ---------------------------------------------------------------------------------------------
// Safety gate
// ---------------------------------------------------------------------------------------------

const BLOCKED_EXTENSIONS: [&str; 18] = [
    ".exe", ".dll", ".bat", ".cmd", ".ps1", ".scr", ".sh", ".app", ".dylib", ".so", ".msi",
    ".vbs", ".com", ".zip", ".7z", ".rar", ".tar", ".gz",
];

fn vpk_paths(path: &Path) -> Result<Vec<String>> {
    let directory = crate::vpk::read_vpk_directory_with(
        path,
        &crate::vpk::ReadVpkOptions {
            keep_preload: false,
        },
    )?;
    if directory.skipped > 0 {
        return Err(Error::msg(format!(
            "{} has {} entries with unsafe paths.",
            file_label(path),
            directory.skipped
        )));
    }
    Ok(directory.entries.into_iter().map(|e| e.path.to_lowercase()).collect())
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Server addons and maps: a readable VPK with safe paths and no executables or archives.
/// They may carry game data: the server mounts them only while you play on it.
pub fn check_server_vpk(path: &Path) -> Result<usize> {
    let entries = vpk_paths(path)?;
    if let Some(bad) = entries
        .iter()
        .find(|p| BLOCKED_EXTENSIONS.iter().any(|ext| p.ends_with(ext)))
    {
        return Err(Error::msg(format!(
            "{} contains {bad}, which a server never needs. It was not installed.",
            file_label(path)
        )));
    }
    Ok(entries.len())
}

/// The UI bootstrap is mounted on every launch, matchmaking included, so it may only hold
/// compiled Panorama UI files.
pub fn check_bootstrap_vpk(path: &Path) -> Result<usize> {
    let entries = check_server_vpk(path)?;
    for entry in vpk_paths(path)? {
        if !entry.starts_with("panorama/") || !entry.ends_with("_c") {
            return Err(Error::msg(format!(
                "The Deadworks UI pack contains {entry}, which is not a compiled Panorama file. It was not installed."
            )));
        }
    }
    Ok(entries)
}

// ---------------------------------------------------------------------------------------------
// Installing content
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryOutcome {
    Installed,
    /// The host's copy differs from what the server advertises. Nothing was installed.
    Mismatch { got_bytes: u64, got_hash: String },
}

fn remove_quiet(path: &Path) {
    let _ = fs::remove_file(path);
}

fn rename_into_place(from: &Path, to: &Path) -> Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(from, to).map_err(|error| {
        remove_quiet(from);
        if matches!(error.kind(), io::ErrorKind::PermissionDenied) || error.raw_os_error() == Some(32)
        {
            Error::msg(format!(
                "{} is in use. Quit Deadlock and try again.",
                file_label(to)
            ))
        } else {
            error.into()
        }
    })
}

/// Download, verify and install one advertised file. GAME WRITE: creates or replaces
/// `citadel/deadworks_addons/vpks/<name>.vpk` or `citadel/maps/<name>.vpk` through a `.part`
/// file and a rename, and updates the ledger. A map Sigil did not install is never replaced.
/// With `accept_mismatch`, the host's copy is installed even when its size or hash differs from
/// the advertised build, but the caps and VPK checks still apply and the ledger does not record
/// it as the advertised build.
pub fn install_entry(
    game_root: &Path,
    fastdl: &str,
    entry: &ContentEntry,
    allow_private: bool,
    accept_mismatch: bool,
    on_bytes: &mut dyn FnMut(u64, Option<u64>),
) -> Result<EntryOutcome> {
    install_entry_capped(game_root, fastdl, entry, allow_private, accept_mismatch, MAX_FILE_BYTES, on_bytes)
}

/// [`install_entry`] with at most `max_bytes` downloaded and at most `max_bytes` decoded, so a
/// join can hold the whole server to [`MAX_JOIN_BYTES`] by what it actually writes rather than
/// by the sizes the server advertises.
fn install_entry_capped(
    game_root: &Path,
    fastdl: &str,
    entry: &ContentEntry,
    allow_private: bool,
    accept_mismatch: bool,
    max_bytes: u64,
    on_bytes: &mut dyn FnMut(u64, Option<u64>),
) -> Result<EntryOutcome> {
    if !valid_content_name(&entry.name) || !valid_hash(&entry.hash) {
        return Err(Error::msg(format!("Refused an invalid content name: {}", entry.name)));
    }
    let paths = deadworks_paths(game_root);
    let dest = paths.dest(entry);
    let mut ledger = read_ledger(&paths);
    let key = entry.key();
    if entry.kind == ContentKind::Map
        && dest.exists()
        && !ledger.files.get(&key).is_some_and(|f| f.ours)
    {
        return Err(Error::msg(format!(
            "{} was not installed by Sigil, so it is left as it is.",
            entry.file_name()
        )));
    }
    let dir = dest.parent().map(Path::to_path_buf).unwrap_or_default();
    fs::create_dir_all(&dir)?;
    let compressed = dir.join(format!("{}.download", entry.file_name()));
    let part = dir.join(format!("{}.part", entry.file_name()));

    let result = (|| -> Result<EntryOutcome> {
        let mut fetched = fetch_to_file(
            &content_url(fastdl, entry, "zst"),
            &compressed,
            allow_private,
            max_bytes,
            on_bytes,
        )?;
        if matches!(fetched, Fetched::Missing) {
            fetched = fetch_to_file(
                &content_url(fastdl, entry, "bz2"),
                &compressed,
                allow_private,
                max_bytes,
                on_bytes,
            )?;
        }
        if matches!(fetched, Fetched::Missing) {
            return Err(Error::Network(format!(
                "The server's host does not have {}.",
                entry.file_name()
            )));
        }
        let decoded = decode_vpk(&compressed, &part, max_bytes)?;
        remove_quiet(&compressed);
        let size_ok = entry.size.is_none_or(|s| s == decoded.bytes);
        let hash_ok = decoded.sha256.starts_with(&entry.hash);
        if !(size_ok && hash_ok) && !accept_mismatch {
            remove_quiet(&part);
            return Ok(EntryOutcome::Mismatch {
                got_bytes: decoded.bytes,
                got_hash: decoded.sha256[..HASH_LEN].to_string(),
            });
        }
        check_server_vpk(&part)?;
        rename_into_place(&part, &dest)?;
        let meta = fs::metadata(&dest)?;
        ledger.files.insert(
            key.clone(),
            LedgerFile {
                size: meta.len(),
                modified_ms: modified_ms(&meta),
                sha256: decoded.sha256,
                ours: true,
                advertised: size_ok && hash_ok,
            },
        );
        write_ledger(&paths, &ledger)?;
        Ok(EntryOutcome::Installed)
    })();
    if result.is_err() {
        remove_quiet(&compressed);
        remove_quiet(&part);
    }
    result
}

// ---------------------------------------------------------------------------------------------
// UI bootstrap
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct BootstrapManifest {
    pub version: u64,
    #[serde(default)]
    pub compressed_size: u64,
    /// SHA-256 of the decompressed VPK.
    pub sha256: String,
    pub download_url: String,
    /// The oldest installed version allowed to join; 0 allows any.
    #[serde(default)]
    pub min_version: u64,
}

pub fn parse_bootstrap_manifest(json: &str) -> Result<BootstrapManifest> {
    let manifest: BootstrapManifest = serde_json::from_str(json)?;
    if manifest.sha256.len() != 64 || !manifest.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::msg("The Deadworks UI pack manifest has no valid hash."));
    }
    Ok(manifest)
}

/// `Ok(None)` when nothing is published (404).
pub fn fetch_bootstrap_manifest(api_base: &str) -> Result<Option<BootstrapManifest>> {
    let url = format!("{}/api/bootstrap", api_base.trim_end_matches('/'));
    let agent = ureq::Agent::config_builder()
        .middleware(crate::net::gate)
        .timeout_global(Some(LIST_TIMEOUT))
        .http_status_as_error(false)
        .build()
        .new_agent();
    let mut response = agent
        .get(&url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| Error::Network(format!("{url}: {e}")))?;
    match response.status().as_u16() {
        404 => Ok(None),
        200..=299 => {
            let text = response
                .body_mut()
                .read_to_string()
                .map_err(|e| Error::Network(format!("{url}: {e}")))?;
            parse_bootstrap_manifest(&text).map(Some)
        }
        status => Err(Error::Network(format!("{url} answered HTTP {status}"))),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BootstrapState {
    /// The installed file matches the published one.
    Current { version: u64 },
    /// Missing or older than the published one.
    Needed {
        manifest: BootstrapManifest,
        installed: Option<u64>,
    },
    /// The manifest could not be read. The installed version, if any, is used as it is.
    Unknown { installed: Option<u64>, reason: String },
    /// Deadworks publishes no bootstrap right now.
    NotPublished,
}

impl BootstrapState {
    /// The version on disk, when Sigil installed it.
    pub fn installed_version(&self) -> Option<u64> {
        match self {
            BootstrapState::Current { version } => Some(*version),
            BootstrapState::Needed { installed, .. } | BootstrapState::Unknown { installed, .. } => {
                *installed
            }
            BootstrapState::NotPublished => None,
        }
    }
}

/// Compare the installed bootstrap with the manifest. Reads only.
pub fn bootstrap_state(
    game_root: &Path,
    manifest: std::result::Result<Option<BootstrapManifest>, String>,
) -> Result<BootstrapState> {
    let paths = deadworks_paths(game_root);
    let ledger = read_ledger(&paths);
    let dest = paths.mods_dir.join(BOOTSTRAP_FILE);
    let installed = dest.is_file().then_some(ledger.bootstrap_version).flatten();
    let manifest = match manifest {
        Ok(Some(manifest)) => manifest,
        Ok(None) => return Ok(BootstrapState::NotPublished),
        Err(reason) => return Ok(BootstrapState::Unknown { installed, reason }),
    };
    if dest.is_file() && file_sha256(&dest)?.eq_ignore_ascii_case(&manifest.sha256) {
        return Ok(BootstrapState::Current {
            version: manifest.version,
        });
    }
    Ok(BootstrapState::Needed { manifest, installed })
}

/// GAME WRITE: install the Deadworks UI bootstrap as `citadel/deadworks_mods/pak01_dir.vpk`.
/// The download must come from `api.deadworks.net` over https, match the manifest's full
/// SHA-256 and hold only compiled Panorama files. Any other `.vpk` in `deadworks_mods` is
/// removed, as the spec requires. The game must be closed.
pub fn install_bootstrap(
    game_root: &Path,
    manifest: &BootstrapManifest,
    on_bytes: &mut dyn FnMut(u64, Option<u64>),
) -> Result<()> {
    let uri: ureq::http::Uri = manifest
        .download_url
        .parse()
        .map_err(|_| Error::msg("The Deadworks UI pack address is malformed."))?;
    if uri.scheme_str() != Some("https") || uri.host() != Some(DEADWORKS_BOOTSTRAP_HOST) {
        return Err(Error::msg(format!(
            "Refused the Deadworks UI pack from {}: it must come from {DEADWORKS_BOOTSTRAP_HOST}.",
            uri.host().unwrap_or("an unknown host")
        )));
    }
    let paths = deadworks_paths(game_root);
    fs::create_dir_all(&paths.mods_dir)?;
    let compressed = paths.mods_dir.join("bootstrap.download");
    let part = paths.mods_dir.join(format!("{BOOTSTRAP_FILE}.part"));
    let result = (|| -> Result<()> {
        if matches!(
            fetch_to_file(&manifest.download_url, &compressed, false, MAX_BOOTSTRAP_BYTES, on_bytes)?,
            Fetched::Missing
        ) {
            return Err(Error::Network("The Deadworks UI pack is not available.".to_string()));
        }
        let decoded = decode_vpk(&compressed, &part, MAX_BOOTSTRAP_BYTES)?;
        remove_quiet(&compressed);
        if !decoded.sha256.eq_ignore_ascii_case(&manifest.sha256) {
            return Err(Error::msg(
                "The Deadworks UI pack does not match its published hash. It was not installed.",
            ));
        }
        check_bootstrap_vpk(&part)?;
        rename_into_place(&part, &paths.mods_dir.join(BOOTSTRAP_FILE))?;
        remove_stray_vpks(&paths.mods_dir)?;
        let mut ledger = read_ledger(&paths);
        ledger.bootstrap_version = Some(manifest.version);
        write_ledger(&paths, &ledger)
    })();
    if result.is_err() {
        remove_quiet(&compressed);
        remove_quiet(&part);
    }
    result
}

/// `deadworks_mods` holds the bootstrap and nothing else the game would mount.
fn remove_stray_vpks(mods_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut removed = Vec::new();
    for entry in fs::read_dir(mods_dir)? {
        let path = entry?.path();
        let is_vpk = path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("vpk"));
        if is_vpk && path.file_name().is_some_and(|n| n != BOOTSTRAP_FILE) {
            fs::remove_file(&path)?;
            removed.push(path);
        }
    }
    Ok(removed)
}

// ---------------------------------------------------------------------------------------------
// gameinfo.gi
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct DeadworksGameinfo {
    pub present: bool,
    /// `Game citadel/deadworks_mods` is in SearchPaths.
    pub game_line: bool,
    /// `addonroot citadel/deadworks_addons` is in SearchPaths.
    pub addonroot_line: bool,
    /// SearchPaths has a `Mod` or `Write` entry.
    pub mod_write: bool,
    /// A SearchPaths block was found.
    pub search_paths: bool,
}

impl DeadworksGameinfo {
    pub fn ready(&self) -> bool {
        self.present && self.game_line && self.addonroot_line && self.mod_write
    }
}

struct SearchPathsBlock {
    /// Line index of the first line inside the braces.
    first: usize,
    /// Line index of the closing brace.
    end: usize,
}

fn strip_comment(line: &str) -> &str {
    match line.find("//") {
        Some(at) => &line[..at],
        None => line,
    }
}

fn key_value(line: &str) -> Option<(String, String)> {
    let mut parts = strip_comment(line).split_whitespace();
    let key = parts.next()?.trim_matches('"').to_string();
    let value = parts
        .next()
        .map(|v| v.trim_matches('"').replace('\\', "/").trim_end_matches('/').to_lowercase())
        .unwrap_or_default();
    Some((key, value))
}

fn find_search_paths(lines: &[&str]) -> Option<SearchPathsBlock> {
    let start = lines.iter().position(|line| {
        strip_comment(line)
            .trim()
            .trim_matches('"')
            .eq_ignore_ascii_case("searchpaths")
    })?;
    let mut depth = 0i32;
    let mut first = None;
    for (index, line) in lines.iter().enumerate().skip(start) {
        for ch in strip_comment(line).chars() {
            match ch {
                '{' => {
                    depth += 1;
                    if depth == 1 {
                        first = Some(index + 1);
                    }
                }
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(SearchPathsBlock {
                            first: first?,
                            end: index,
                        });
                    }
                }
                _ => {}
            }
        }
    }
    None
}

fn scan_gameinfo(text: &str) -> DeadworksGameinfo {
    let lines: Vec<&str> = text.split('\n').collect();
    let Some(block) = find_search_paths(&lines) else {
        return DeadworksGameinfo {
            present: true,
            ..DeadworksGameinfo::default()
        };
    };
    let mut status = DeadworksGameinfo {
        present: true,
        search_paths: true,
        ..DeadworksGameinfo::default()
    };
    for line in &lines[block.first..block.end] {
        let Some((key, value)) = key_value(line) else { continue };
        if key.eq_ignore_ascii_case("game") && value == DEADWORKS_GAME_PATH {
            status.game_line = true;
        } else if key.eq_ignore_ascii_case("addonroot") && value == DEADWORKS_ADDON_ROOT {
            status.addonroot_line = true;
        } else if key.eq_ignore_ascii_case("mod") || key.eq_ignore_ascii_case("write") {
            status.mod_write = true;
        }
    }
    status
}

/// What `gameinfo.gi` has for Deadworks. Reads only.
pub fn read_deadworks_gameinfo(gameinfo_path: &Path) -> Result<DeadworksGameinfo> {
    match fs::read_to_string(gameinfo_path) {
        Ok(text) => Ok(scan_gameinfo(&text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(DeadworksGameinfo::default()),
        Err(error) => Err(error.into()),
    }
}

/// The text with the missing Deadworks lines added above the first `Game` entry, or `None`
/// when nothing is missing. Pure: no file access.
pub fn add_deadworks_lines(text: &str) -> Result<Option<String>> {
    let status = scan_gameinfo(text);
    if status.ready() {
        return Ok(None);
    }
    let ending = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let lines: Vec<&str> = text.split('\n').collect();
    let block = find_search_paths(&lines)
        .ok_or_else(|| Error::msg("gameinfo.gi has no SearchPaths block. Verify the game files in Steam."))?;
    let insert_at = (block.first..block.end)
        .find(|&i| key_value(lines[i]).is_some_and(|(k, _)| k.eq_ignore_ascii_case("game")))
        .ok_or_else(|| Error::msg("gameinfo.gi has no Game entry in SearchPaths. Verify the game files in Steam."))?;
    let anchor = lines[insert_at].trim_end_matches('\r');
    let indent = &anchor[..anchor.len() - anchor.trim_start().len()];

    let mut added = vec![GAMEINFO_MARK_START.to_string()];
    if !status.game_line {
        added.push(format!("Game\t\t{DEADWORKS_GAME_PATH}"));
    }
    if !status.mod_write {
        added.push("Mod\t\tcitadel".to_string());
        added.push("Write\t\tcitadel".to_string());
    }
    if !status.addonroot_line {
        added.push(format!("addonroot\t{DEADWORKS_ADDON_ROOT}"));
    }
    added.push(GAMEINFO_MARK_END.to_string());

    let mut out = String::with_capacity(text.len() + 256);
    for (index, line) in lines.iter().enumerate() {
        if index == insert_at {
            for new_line in &added {
                out.push_str(indent);
                out.push_str(new_line);
                out.push_str(ending);
            }
        }
        out.push_str(line);
        if index + 1 < lines.len() {
            out.push('\n');
        }
    }
    Ok(Some(out))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameinfoEdit {
    pub changed: bool,
    pub backup: Option<PathBuf>,
}

/// GAME WRITE: add `Game citadel/deadworks_mods`, `addonroot citadel/deadworks_addons` and,
/// when SearchPaths has neither, `Mod citadel` and `Write citadel`, above the first `Game`
/// entry. Lines already present (from the Deadworks launcher or another manager) are left
/// alone. The original is copied to `gameinfo.gi.dah-backup-<stamp>` and the new text is
/// written atomically. A no-op when nothing is missing.
pub fn ensure_deadworks_gameinfo(gameinfo_path: &Path) -> Result<GameinfoEdit> {
    let text = fs::read_to_string(gameinfo_path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            Error::msg("gameinfo.gi is missing. Verify the game files in Steam.")
        } else {
            error.into()
        }
    })?;
    let Some(updated) = add_deadworks_lines(&text)? else {
        return Ok(GameinfoEdit {
            changed: false,
            backup: None,
        });
    };
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%3fZ");
    let name = gameinfo_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "gameinfo.gi".to_string());
    let backup = gameinfo_path.with_file_name(format!("{name}.dah-backup-{stamp}"));
    fs::copy(gameinfo_path, &backup)?;
    crate::util::write_atomic(gameinfo_path, updated.as_bytes()).map_err(|error| {
        Error::msg(format!(
            "gameinfo.gi could not be written ({error}). Quit Deadlock and try again."
        ))
    })?;
    Ok(GameinfoEdit {
        changed: true,
        backup: Some(backup),
    })
}

// ---------------------------------------------------------------------------------------------
// Join
// ---------------------------------------------------------------------------------------------

/// `steam://connect/<ip:port>[/<password>]`. Only a literal address is accepted, and a password
/// only from an allowlist (ASCII letters, digits, `-_.~`, 1 to 64), so nothing a shell or URL
/// handler would interpret can ride along.
pub fn connect_url(address: &str, password: Option<&str>) -> Result<String> {
    let (ip, port) = split_address(address)
        .ok_or_else(|| Error::msg(format!("{address} is not an ip:port address.")))?;
    let mut url = format!("steam://connect/{}", format_address(ip, port));
    if let Some(password) = password.map(str::trim).filter(|p| !p.is_empty()) {
        let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~');
        if password.len() > 64 || !password.chars().all(allowed) {
            return Err(Error::msg(
                "Sigil can pass a password of up to 64 letters, digits, and - _ . ~ only. Join from the game console for this one.",
            ));
        }
        url.push('/');
        url.push_str(password);
    }
    Ok(url)
}

/// Hand `steam://connect/...` to the platform opener. Steam starts the game if it is closed.
pub fn open_connect(address: &str, password: Option<&str>) -> Result<String> {
    let url = connect_url(address, password)?;
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut c = std::process::Command::new("open");
        c.arg(&url);
        c
    };
    // No shell: rundll32 hands the URL to its protocol handler as one argument.
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut c = std::process::Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", &url]);
        c
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut command = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(&url);
        c
    };
    let status = command
        .status()
        .map_err(|e| Error::msg(format!("Could not open Steam: {e}.")))?;
    if !status.success() {
        return Err(Error::msg("Steam did not take the connect link. Is Steam installed?"));
    }
    connect_url(address, None)
}

/// Everything a join would change, worked out before the user confirms. Reads only, plus the
/// network reads: A2S_RULES to the game server and the bootstrap manifest from Deadworks.
#[derive(Clone, Debug)]
pub struct JoinCheck {
    pub address: String,
    /// The server answered A2S_RULES with `dw_ver` 1.
    pub advertised: bool,
    pub plan: ContentPlan,
    pub gameinfo: DeadworksGameinfo,
    pub bootstrap: BootstrapState,
    /// The game server is on this machine or the local network, so its download host may be too.
    pub allow_private: bool,
}

impl JoinCheck {
    /// Problems that stop the join before anything is written.
    pub fn blockers(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.gameinfo.present {
            out.push("gameinfo.gi is missing. Verify the game files in Steam.".to_string());
        } else if !self.gameinfo.search_paths {
            out.push("gameinfo.gi has no SearchPaths block. Verify the game files in Steam.".to_string());
        }
        for item in &self.plan.items {
            if item.action == ItemAction::Unavailable {
                out.push(format!(
                    "The server needs {} but gives no download address.",
                    item.entry.file_name()
                ));
            }
        }
        if let BootstrapState::Unknown {
            installed: None,
            reason,
        } = &self.bootstrap
        {
            out.push(format!("The Deadworks UI pack could not be checked: {reason}"));
        }
        out
    }

    /// Writes the join would make, in plain words, for the confirm dialog.
    pub fn writes(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.gameinfo.present && !self.gameinfo.ready() {
            out.push("Add the Deadworks lines to gameinfo.gi (a backup is kept)".to_string());
        }
        if let BootstrapState::Needed { manifest, installed } = &self.bootstrap {
            out.push(match installed {
                Some(v) => format!("Update the Deadworks UI pack from v{v} to v{}", manifest.version),
                None => format!("Install the Deadworks UI pack v{}", manifest.version),
            });
        }
        for item in self.plan.downloads() {
            out.push(format!(
                "Download {} {}{}",
                match item.entry.kind {
                    ContentKind::Addon => "addon",
                    ContentKind::Map => "map",
                },
                item.entry.name,
                item.entry
                    .size
                    .map(|s| format!(" ({})", format_bytes(s)))
                    .unwrap_or_default()
            ));
        }
        out
    }

    pub fn needs_writes(&self) -> bool {
        !self.writes().is_empty()
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KB {
        format!("{bytes} B")
    } else if b < KB * KB {
        format!("{:.0} KB", b / KB)
    } else if b < KB * KB * KB {
        format!("{:.1} MB", b / KB / KB)
    } else {
        format!("{:.2} GB", b / KB / KB / KB)
    }
}

/// Look before joining: query the server's rules, plan its content, read gameinfo.gi and the
/// bootstrap state. Writes nothing.
pub fn check_join(game_root: &Path, address: &str, api_base: &str) -> Result<JoinCheck> {
    let addr: SocketAddr = split_address(address)
        .map(|(ip, port)| SocketAddr::new(ip, port))
        .ok_or_else(|| Error::msg(format!("{address} is not an ip:port address.")))?;
    let rules = query_rules(addr, RULES_DEADLINE).unwrap_or(None);
    let advertisement = rules.as_ref().and_then(parse_advertisement);
    let plan = match &advertisement {
        Some(ad) => plan_content(game_root, ad)?,
        None => ContentPlan::default(),
    };
    let paths = deadworks_paths(game_root);
    let gameinfo = read_deadworks_gameinfo(&paths.gameinfo)?;
    let manifest = fetch_bootstrap_manifest(api_base).map_err(|e| e.to_string());
    let bootstrap = bootstrap_state(game_root, manifest)?;
    Ok(JoinCheck {
        address: format_address(addr.ip(), addr.port()),
        advertised: advertisement.is_some(),
        plan,
        gameinfo,
        bootstrap,
        allow_private: !is_public_ip(addr.ip()),
    })
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct JoinProgress {
    pub line: String,
    /// 0 to 1 when the size is known.
    pub fraction: Option<f32>,
}

#[derive(Clone, Debug, Default)]
pub struct JoinReport {
    pub gameinfo_backup: Option<PathBuf>,
    pub bootstrap_installed: Option<u64>,
    pub installed: Vec<String>,
    /// Keys of files whose host copy differs from the advertised build. Nothing was installed for
    /// them and the join should not go ahead until the user decides.
    pub mismatched: Vec<(String, String)>,
}

/// GAME WRITE: carry out a confirmed join check. In order: `ensure_deadworks_gameinfo` (first,
/// so a locked file fails before a long download), `install_bootstrap` when needed, then
/// `install_entry` for each download. `accept_mismatch` holds the entry keys the user agreed to
/// take as the host serves them. Does not open Steam: the caller does that when the report has
/// no mismatches.
pub fn prepare_join(
    game_root: &Path,
    check: &JoinCheck,
    accept_mismatch: &[String],
    on_progress: &dyn Fn(JoinProgress),
) -> Result<JoinReport> {
    if let Some(blocker) = check.blockers().into_iter().next() {
        return Err(Error::msg(blocker));
    }
    let paths = deadworks_paths(game_root);
    let mut report = JoinReport::default();

    on_progress(JoinProgress {
        line: "Checking gameinfo.gi".to_string(),
        fraction: None,
    });
    report.gameinfo_backup = ensure_deadworks_gameinfo(&paths.gameinfo)?.backup;

    if !check.plan.hashed.is_empty() {
        let mut ledger = read_ledger(&paths);
        for (key, file) in &check.plan.hashed {
            let entry = ledger.files.entry(key.clone()).or_default();
            let ours = entry.ours;
            let advertised = entry.advertised;
            *entry = file.clone();
            entry.ours = ours;
            entry.advertised = advertised;
        }
        write_ledger(&paths, &ledger)?;
    }

    if let BootstrapState::Needed { manifest, installed } = &check.bootstrap {
        let label = format!("Deadworks UI pack v{}", manifest.version);
        let result = install_bootstrap(game_root, manifest, &mut |done, total| {
            on_progress(JoinProgress {
                line: format!("Downloading {label}"),
                fraction: total
                    .or(Some(manifest.compressed_size).filter(|s| *s > 0))
                    .map(|t| (done as f32 / t.max(1) as f32).min(1.0)),
            })
        });
        match result {
            Ok(()) => report.bootstrap_installed = Some(manifest.version),
            Err(error) => {
                let usable = installed.is_some_and(|v| v >= manifest.min_version);
                if !usable {
                    return Err(error);
                }
            }
        }
    }

    let downloads: Vec<&PlanItem> = check.plan.downloads().collect();
    let (budget, _) = check.plan.download_size();
    if budget > MAX_JOIN_BYTES {
        return Err(Error::msg("This server needs more than 16 GB of downloads. Sigil stops there."));
    }
    let fastdl = check.plan.fastdl.clone().unwrap_or_default();
    let count = downloads.len();
    // The advertised sizes are the server's word; the cap holds on what is actually written.
    let mut remaining = MAX_JOIN_BYTES;
    let paths = deadworks_paths(game_root);
    for (index, item) in downloads.into_iter().enumerate() {
        if remaining == 0 {
            return Err(Error::msg("This server needs more than 16 GB of downloads. Sigil stops there."));
        }
        let accept = accept_mismatch.contains(&item.entry.key());
        let line = if count > 1 {
            format!("Downloading {} ({} of {count})", item.entry.name, index + 1)
        } else {
            format!("Downloading {}", item.entry.name)
        };
        let advertised = item.entry.size;
        let outcome = install_entry_capped(
            game_root,
            &fastdl,
            &item.entry,
            check.allow_private,
            accept,
            remaining.min(MAX_FILE_BYTES),
            &mut |done, total| {
                on_progress(JoinProgress {
                    line: line.clone(),
                    fraction: total
                        .or(advertised)
                        .map(|t| (done as f32 / t.max(1) as f32).min(1.0)),
                })
            },
        )?;
        let written = match &outcome {
            EntryOutcome::Installed => fs::metadata(paths.dest(&item.entry)).map_or(0, |m| m.len()),
            EntryOutcome::Mismatch { got_bytes, .. } => *got_bytes,
        };
        remaining = remaining.saturating_sub(written);
        match outcome {
            EntryOutcome::Installed => report.installed.push(item.entry.key()),
            EntryOutcome::Mismatch { got_hash, .. } => {
                report.mismatched.push((item.entry.key(), got_hash))
            }
        }
    }
    on_progress(JoinProgress {
        line: "Ready".to_string(),
        fraction: Some(1.0),
    });
    Ok(report)
}


// ---------------------------------------------------------------------------------------------
// Regions (pure)
// ---------------------------------------------------------------------------------------------

/// The continent a server's country belongs to, for the grouped region filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Continent {
    Europe,
    NorthAmerica,
    SouthAmerica,
    AsiaPacific,
    Oceania,
    MiddleEast,
    Other,
}

impl Continent {
    pub const ALL: [Continent; 7] = [
        Continent::Europe,
        Continent::NorthAmerica,
        Continent::SouthAmerica,
        Continent::AsiaPacific,
        Continent::Oceania,
        Continent::MiddleEast,
        Continent::Other,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Continent::Europe => "eu",
            Continent::NorthAmerica => "na",
            Continent::SouthAmerica => "sa",
            Continent::AsiaPacific => "apac",
            Continent::Oceania => "oce",
            Continent::MiddleEast => "me",
            Continent::Other => "other",
        }
    }

    pub fn from_id(id: &str) -> Option<Continent> {
        Continent::ALL.into_iter().find(|continent| continent.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Continent::Europe => "Europe",
            Continent::NorthAmerica => "North America",
            Continent::SouthAmerica => "South America",
            Continent::AsiaPacific => "Asia Pacific",
            Continent::Oceania => "Oceania",
            Continent::MiddleEast => "Middle East",
            Continent::Other => "Other",
        }
    }

    /// The country codes in this continent, two letters, upper case.
    pub fn countries(self) -> &'static [&'static str] {
        match self {
            Continent::Europe => &[
                "EU", "AL", "AT", "BA", "BE", "BG", "BY", "CH", "CY", "CZ", "DE", "DK", "EE",
                "ES", "FI", "FR", "GB", "GR", "HR", "HU", "IE", "IS", "IT", "LT", "LU", "LV",
                "MD", "ME", "MK", "MT", "NL", "NO", "PL", "PT", "RO", "RS", "RU", "SE", "SI",
                "SK", "TR", "UA", "UK",
            ],
            Continent::NorthAmerica => &["NA", "CA", "MX", "US"],
            Continent::SouthAmerica => &["SA", "AR", "BO", "BR", "CL", "CO", "EC", "PE", "PY", "UY", "VE"],
            Continent::AsiaPacific => &[
                "AP", "ASIA", "CN", "HK", "ID", "IN", "JP", "KH", "KR", "LA", "MM", "MN", "MY",
                "PH", "PK", "SG", "TH", "TW", "VN",
            ],
            Continent::Oceania => &["OC", "AU", "FJ", "NZ"],
            Continent::MiddleEast => &["ME", "AE", "BH", "IL", "IQ", "JO", "KW", "LB", "OM", "QA", "SA", "ZA"],
            Continent::Other => &[],
        }
    }

    /// The continent of a country or region code. Unknown and empty codes are `Other`.
    pub fn of(country: &str) -> Continent {
        let code = country.trim().to_ascii_uppercase();
        if code.is_empty() {
            return Continent::Other;
        }
        for continent in Continent::ALL {
            if continent.countries().contains(&code.as_str()) {
                return continent;
            }
        }
        Continent::Other
    }
}

/// The short label for a server's region: its code, or `Unknown`.
pub fn region_label(country: &str) -> String {
    let code = country.trim().to_ascii_uppercase();
    if code.is_empty() {
        "Unknown".to_string()
    } else {
        code
    }
}

// ---------------------------------------------------------------------------------------------
// Filters and sort (pure)
// ---------------------------------------------------------------------------------------------

/// Which servers the password filter lets through.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PasswordFilter {
    #[default]
    All,
    Open,
    Password,
}

impl PasswordFilter {
    pub const ALL: [PasswordFilter; 3] = [
        PasswordFilter::All,
        PasswordFilter::Open,
        PasswordFilter::Password,
    ];

    pub fn id(self) -> &'static str {
        match self {
            PasswordFilter::All => "all",
            PasswordFilter::Open => "open",
            PasswordFilter::Password => "password",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PasswordFilter::All => "All",
            PasswordFilter::Open => "Open",
            PasswordFilter::Password => "Password",
        }
    }
}

/// Everything the Servers page filters by. Empty means every server.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerFilter {
    pub query: String,
    pub has_players: bool,
    pub has_space: bool,
    pub password: PasswordFilter,
    /// [`Server::mode_label`], matched case-insensitively.
    pub mode: Option<String>,
    pub continent: Option<Continent>,
    /// A single country inside the continent.
    pub country: Option<String>,
    pub map: Option<String>,
}

impl ServerFilter {
    /// The "Joinable" preset: no password, somebody on, room left.
    pub fn joinable() -> ServerFilter {
        ServerFilter {
            has_players: true,
            has_space: true,
            password: PasswordFilter::Open,
            ..ServerFilter::default()
        }
    }

    /// True when the preset's three axes are on and nothing else is.
    pub fn is_joinable(&self) -> bool {
        self.query.is_empty()
            && self.has_players
            && self.has_space
            && self.password == PasswordFilter::Open
            && self.mode.is_none()
            && self.continent.is_none()
            && self.country.is_none()
            && self.map.is_none()
    }

    /// The number of filters on, one per chip.
    pub fn active_count(&self) -> usize {
        [
            !self.query.trim().is_empty(),
            self.has_players,
            self.has_space,
            self.password != PasswordFilter::All,
            self.mode.is_some(),
            self.continent.is_some(),
            self.country.is_some(),
            self.map.is_some(),
        ]
        .into_iter()
        .filter(|on| *on)
        .count()
    }

    pub fn matches(&self, server: &Server) -> bool {
        if self.has_players && server.players == 0 {
            return false;
        }
        if self.has_space && server.is_full() {
            return false;
        }
        match self.password {
            PasswordFilter::All => {}
            PasswordFilter::Open if server.password => return false,
            PasswordFilter::Password if !server.password => return false,
            _ => {}
        }
        if let Some(mode) = &self.mode
            && !server.mode_label().eq_ignore_ascii_case(mode)
        {
            return false;
        }
        if let Some(continent) = self.continent
            && Continent::of(&server.country) != continent
        {
            return false;
        }
        if let Some(country) = &self.country
            && !server.country.eq_ignore_ascii_case(country)
        {
            return false;
        }
        if let Some(map) = &self.map
            && server.map != *map
        {
            return false;
        }
        let query = self.query.trim().to_lowercase();
        if query.is_empty() {
            return true;
        }
        server.name.to_lowercase().contains(&query)
            || server.map.to_lowercase().contains(&query)
            || server.address.to_lowercase().contains(&query)
            || server.country.to_lowercase() == query
            || server.mode_label().to_lowercase().contains(&query)
    }
}

/// A column the table can be sorted by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerSortKey {
    Name,
    Players,
    Map,
    Ping,
    Region,
}

impl ServerSortKey {
    pub const ALL: [ServerSortKey; 5] = [
        ServerSortKey::Name,
        ServerSortKey::Players,
        ServerSortKey::Map,
        ServerSortKey::Ping,
        ServerSortKey::Region,
    ];

    pub fn id(self) -> &'static str {
        match self {
            ServerSortKey::Name => "name",
            ServerSortKey::Players => "players",
            ServerSortKey::Map => "map",
            ServerSortKey::Ping => "ping",
            ServerSortKey::Region => "region",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ServerSortKey::Name => "Name",
            ServerSortKey::Players => "Players",
            ServerSortKey::Map => "Map",
            ServerSortKey::Ping => "Ping",
            ServerSortKey::Region => "Region",
        }
    }

    /// The direction a column starts in: busiest and closest first.
    pub fn first_direction(self) -> SortDirection {
        match self {
            ServerSortKey::Players => SortDirection::Desc,
            _ => SortDirection::Asc,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServerSort {
    pub key: ServerSortKey,
    pub direction: SortDirection,
}

/// A header press: a new column starts in its natural direction, the same column flips, the
/// third press returns to the list's own order.
pub fn next_sort(current: Option<ServerSort>, key: ServerSortKey) -> Option<ServerSort> {
    match current {
        Some(sort) if sort.key == key => {
            if sort.direction == key.first_direction() {
                Some(ServerSort {
                    key,
                    direction: if key.first_direction() == SortDirection::Asc {
                        SortDirection::Desc
                    } else {
                        SortDirection::Asc
                    },
                })
            } else {
                None
            }
        }
        _ => Some(ServerSort {
            key,
            direction: key.first_direction(),
        }),
    }
}

fn missing_last<T: Ord>(left: Option<T>, right: Option<T>, descending: bool) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (left, right) {
        (Some(left), Some(right)) => {
            let order = left.cmp(&right);
            if descending { order.reverse() } else { order }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// Sort a visible list. Ties keep the order they came in, and a server with no value for the
/// column (no ping, unknown region) stays at the bottom in both directions.
pub fn sorted_servers<'a>(
    servers: Vec<&'a Server>,
    sort: Option<ServerSort>,
    pings: &HashMap<String, Option<u32>>,
) -> Vec<&'a Server> {
    let Some(sort) = sort else {
        return servers;
    };
    let descending = sort.direction == SortDirection::Desc;
    let mut out = servers;
    out.sort_by(|left, right| match sort.key {
        ServerSortKey::Name => missing_last(
            Some(left.name.to_lowercase()),
            Some(right.name.to_lowercase()),
            descending,
        ),
        ServerSortKey::Players => missing_last(
            (left.max_players > 0).then_some(left.players),
            (right.max_players > 0).then_some(right.players),
            descending,
        ),
        ServerSortKey::Map => missing_last(
            (!left.map.is_empty()).then(|| left.map.to_lowercase()),
            (!right.map.is_empty()).then(|| right.map.to_lowercase()),
            descending,
        ),
        ServerSortKey::Ping => missing_last(
            pings.get(&left.id).copied().flatten(),
            pings.get(&right.id).copied().flatten(),
            descending,
        ),
        ServerSortKey::Region => {
            let key = |server: &Server| {
                (!server.country.trim().is_empty()).then(|| {
                    (
                        Continent::of(&server.country) as usize,
                        server.country.to_ascii_uppercase(),
                    )
                })
            };
            missing_last(key(left), key(right), descending)
        }
    });
    out
}

// ---------------------------------------------------------------------------------------------
// Favorites, recent servers and direct connect
// ---------------------------------------------------------------------------------------------

pub const SAVED_SERVERS_FILE: &str = "servers.json";
pub const SAVED_SERVERS_SCHEMA: u32 = 1;
/// Recent servers kept. Older ones fall off the end.
pub const RECENT_LIMIT: usize = 8;

/// A server the player kept, by address.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedServer {
    pub address: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub map: String,
    /// ISO time the address was saved or last joined.
    #[serde(default)]
    pub at: String,
}

impl SavedServer {
    fn new(server: &Server) -> SavedServer {
        SavedServer {
            address: server.address.clone(),
            name: server.name.clone(),
            map: server.map.clone(),
            at: crate::util::now_iso(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedServers {
    #[serde(default = "saved_schema")]
    pub schema: u32,
    #[serde(default)]
    pub favorites: Vec<SavedServer>,
    #[serde(default)]
    pub recent: Vec<SavedServer>,
}

fn saved_schema() -> u32 {
    SAVED_SERVERS_SCHEMA
}

/// `<workspace>/.dah/servers.json`, beside the profiles and snapshots.
pub fn saved_servers_path(workspace: &Path) -> PathBuf {
    workspace.join(".dah").join(SAVED_SERVERS_FILE)
}

/// Read the favorites and recent list. A missing or unreadable file is an empty list: it is a
/// convenience cache, never a source of truth about a server.
pub fn read_saved_servers(workspace: &Path) -> SavedServers {
    fs::read_to_string(saved_servers_path(workspace))
        .ok()
        .and_then(|text| serde_json::from_str::<SavedServers>(&text).ok())
        .map(|mut saved| {
            saved.favorites.retain(|entry| !entry.address.trim().is_empty());
            saved.recent.retain(|entry| !entry.address.trim().is_empty());
            saved.recent.truncate(RECENT_LIMIT);
            saved
        })
        .unwrap_or_default()
}

/// Write the favorites and recent list. Workspace only.
pub fn write_saved_servers(workspace: &Path, saved: &SavedServers) -> Result<()> {
    let mut text = serde_json::to_string_pretty(saved)?;
    text.push('\n');
    crate::util::write_atomic(&saved_servers_path(workspace), text.as_bytes())
}

pub fn is_favorite(saved: &SavedServers, address: &str) -> bool {
    saved.favorites.iter().any(|entry| entry.address == address)
}

/// Keep or forget a server. Returns true when it is now a favorite.
pub fn toggle_favorite(workspace: &Path, server: &Server) -> Result<bool> {
    let mut saved = read_saved_servers(workspace);
    let now = is_favorite(&saved, &server.address);
    if now {
        saved.favorites.retain(|entry| entry.address != server.address);
    } else {
        saved.favorites.push(SavedServer::new(server));
        saved.favorites.sort_by(|left, right| {
            left.name
                .to_lowercase()
                .cmp(&right.name.to_lowercase())
                .then_with(|| left.address.cmp(&right.address))
        });
    }
    write_saved_servers(workspace, &saved)?;
    Ok(!now)
}

/// Remember a join or a direct connect. Workspace only.
pub fn mark_recent(workspace: &Path, server: &Server) -> Result<()> {
    let mut saved = read_saved_servers(workspace);
    saved.recent.retain(|entry| entry.address != server.address);
    saved.recent.insert(0, SavedServer::new(server));
    saved.recent.truncate(RECENT_LIMIT);
    write_saved_servers(workspace, &saved)
}

pub fn forget_recent(workspace: &Path, address: &str) -> Result<()> {
    let mut saved = read_saved_servers(workspace);
    saved.recent.retain(|entry| entry.address != address);
    write_saved_servers(workspace, &saved)
}

/// Turn typed text into a server to connect to: `203.0.113.7:27015` or the same behind
/// `steam://connect/`. Refuses anything that is not an IP and a port.
pub fn parse_direct_address(input: &str) -> Result<Server> {
    let text = input.trim();
    let text = text
        .strip_prefix("steam://connect/")
        .unwrap_or(text)
        .trim_end_matches('/');
    let (ip, port) = split_address(text).ok_or_else(|| {
        Error::msg(format!(
            "Enter the server as address:port, for example 203.0.113.7:27015 ({text:?} is not one)."
        ))
    })?;
    let address = format_address(ip, port);
    Ok(Server {
        id: address.clone(),
        name: address.clone(),
        address,
        map: String::new(),
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
    })
}

// ---------------------------------------------------------------------------------------------
// Custom maps: GameBanana's Maps category, console names and the launch block
// ---------------------------------------------------------------------------------------------

/// GameBanana's Maps category for Deadlock (game 20948).
pub const MAP_CATEGORY_ID: u64 = 37_225;
pub const MAPS_PER_PAGE: usize = 24;
const GAMEBANANA_MAPS_INDEX: &str = "https://gamebanana.com/apiv11/Mod/Index";
const GAMEBANANA_MOD: &str = "https://gamebanana.com/apiv11/Mod";

/// Marks the autoexec block a "Launch Map" writes.
pub const MAP_BLOCK_BEGIN: &str = "// sigil:map begin";
pub const MAP_BLOCK_END: &str = "// sigil:map end";

/// One entry in the Maps view.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapEntry {
    pub id: u64,
    pub name: String,
    pub author: String,
    pub likes: i64,
    pub updated_unix: i64,
    pub page_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
    /// The console name, read from the description when the row is opened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub console_name: Option<String>,
    /// Whether `citadel/maps/<console>.vpk` is installed right now.
    #[serde(default)]
    pub installed: bool,
}

fn map_image(value: &Value) -> Option<String> {
    let images = value
        .get("_aPreviewMedia")?
        .get("_aImages")?
        .as_array()?;
    let image = images.first()?;
    let base = image.get("_sBaseUrl")?.as_str()?;
    let file = ["_sFile530", "_sFile100", "_sFile"]
        .iter()
        .find_map(|key| image.get(*key).and_then(Value::as_str))?;
    Some(format!("{base}/{file}"))
}

/// Parse the Maps category index.
pub fn parse_maps(body: &str) -> Result<Vec<MapEntry>> {
    let root: Value = serde_json::from_str(body)?;
    let records = root
        .get("_aRecords")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(records
        .iter()
        .filter_map(|record| {
            let id = record.get("_idRow").and_then(Value::as_u64).filter(|id| *id != 0)?;
            let name = record
                .get("_sName")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            if name.is_empty() {
                return None;
            }
            Some(MapEntry {
                id,
                name,
                author: record
                    .get("_aSubmitter")
                    .and_then(|submitter| submitter.get("_sName"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
                likes: record
                    .get("_nLikeCount")
                    .and_then(Value::as_i64)
                    .unwrap_or(0),
                updated_unix: ["_tsDateUpdated", "_tsDateModified", "_tsDateAdded"]
                    .iter()
                    .find_map(|key| record.get(*key).and_then(Value::as_i64))
                    .unwrap_or(0),
                page_url: record
                    .get("_sProfileUrl")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                thumbnail: map_image(record),
                console_name: None,
                installed: false,
            })
        })
        .collect())
}

/// Fetch one page of the Maps category. Network: `gamebanana.com` only (already allowlisted).
pub fn fetch_maps(page: usize) -> Result<Vec<MapEntry>> {
    let page = page.max(1);
    let url = format!(
        "{GAMEBANANA_MAPS_INDEX}?_nPage={page}&_nPerpage={MAPS_PER_PAGE}\
         &_aFilters%5BGeneric_Game%5D=20948\
         &_aFilters%5BGeneric_Category%5D={MAP_CATEGORY_ID}\
         &_sSort=Generic_Newest\
         &_csvProperties=_idRow,_sName,_sProfileUrl,_tsDateUpdated,_tsDateModified,_nLikeCount,_aPreviewMedia,_aSubmitter"
    );
    let body = get_text(&url)?;
    parse_maps(&body)
}

/// Fetch a map's description, where the console name usually is. Network: `gamebanana.com`.
pub fn fetch_map_description(id: u64) -> Result<Option<String>> {
    if id == 0 {
        return Err(Error::msg("A map needs its GameBanana id."));
    }
    let url = format!("{GAMEBANANA_MOD}/{id}?_csvProperties=_sText");
    let body = get_text(&url)?;
    let value: Value = serde_json::from_str(&body)?;
    Ok(value
        .get("_sText")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|text| !text.trim().is_empty()))
}

/// A map console name: letters, digits and underscores, the shape `map <name>` accepts.
pub fn valid_map_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
}

/// Find the console name in a map description: `maps/<name>.vpk` first, then a `map <name>`
/// command. Anything that does not look like a map name is ignored.
pub fn console_map_name(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find("maps/") {
        let start = from + at + "maps/".len();
        let rest = &text[start..];
        let name: String = rest
            .chars()
            .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
            .collect();
        let after = rest[name.len()..].to_ascii_lowercase();
        if valid_map_name(&name) && after.starts_with(".vpk") {
            return Some(name);
        }
        from = start;
    }
    let mut from = 0;
    while let Some(at) = lower[from..].find("map ") {
        let boundary = at == 0
            || lower[..from + at]
                .chars()
                .last()
                .is_none_or(|character| !character.is_ascii_alphanumeric());
        let start = from + at + "map ".len();
        let name: String = text[start..]
            .chars()
            .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
            .collect();
        let after = text[start + name.len()..].chars().next();
        if boundary
            && valid_map_name(&name)
            && name.len() > 1
            && after.is_none_or(|character| character.is_whitespace() || character == '`' || character == ';')
        {
            return Some(name);
        }
        from = start;
    }
    None
}

/// The maps installed under `citadel/maps`, by console name.
pub fn installed_maps(game_root: &Path) -> Vec<String> {
    let dir = game_root.join("game").join("citadel").join("maps");
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.file_type().map(|kind| kind.is_file()).unwrap_or(false))
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let stem = name
                .strip_suffix(".vpk")
                .or_else(|| name.strip_suffix(".vpk.disabled"))?;
            valid_map_name(stem).then(|| stem.to_string())
        })
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    names.dedup();
    names
}

/// The map the autoexec launch block names, when there is one.
pub fn map_launch(text: &str) -> Option<String> {
    let begin = text.find(MAP_BLOCK_BEGIN)?;
    let rest = &text[begin..];
    let end = rest.find(MAP_BLOCK_END)?;
    let block = &rest[..end];
    block.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let command = parts.next()?;
        if !command.eq_ignore_ascii_case("map") {
            return None;
        }
        let name = parts.next()?;
        valid_map_name(name).then(|| name.to_string())
    })
}

/// The autoexec text with the map launch block set to `map`, or removed when it is None.
/// None means the text already says that.
pub fn set_map_launch(text: &str, map: Option<&str>) -> Option<String> {
    if let Some(map) = map
        && !valid_map_name(map)
    {
        return None;
    }
    let without = match text.find(MAP_BLOCK_BEGIN) {
        Some(begin) => match text[begin..].find(MAP_BLOCK_END) {
            Some(relative) => {
                let end = begin + relative + MAP_BLOCK_END.len();
                let end = text[end..]
                    .find('\n')
                    .map(|offset| end + offset + 1)
                    .unwrap_or(end);
                format!("{}{}", &text[..begin], &text[end..])
            }
            None => text.to_string(),
        },
        None => text.to_string(),
    };
    let trimmed = without.trim_end().to_string();
    let next = match map {
        Some(map) => format!(
            "{trimmed}{}{MAP_BLOCK_BEGIN}\nmap {map}\n{MAP_BLOCK_END}\n",
            if trimmed.is_empty() { "" } else { "\n" }
        ),
        None => {
            if trimmed.is_empty() {
                String::new()
            } else {
                format!("{trimmed}\n")
            }
        }
    };
    (crate::autoexec::normalize_trailing_newline(&next)
        != crate::autoexec::normalize_trailing_newline(text))
    .then_some(next)
}

/// The map the current autoexec starts with, when the block is there.
pub fn read_map_launch(game_root: &Path) -> Option<String> {
    crate::autoexec::read_autoexec(game_root)
        .ok()
        .flatten()
        .and_then(|text| map_launch(&text))
}

/// GAME WRITE. Put `map <name>` into the managed autoexec block (or remove it), keeping a
/// backup of the previous file. Refuses without `approve`. The caller launches after this.
pub fn write_map_launch(
    game_root: &Path,
    map: Option<&str>,
    approve: bool,
) -> Result<Option<PathBuf>> {
    if !approve {
        return Err(Error::msg(
            "Setting the launch map was not approved. Nothing was written.",
        ));
    }
    let current = crate::autoexec::read_autoexec(game_root)?.unwrap_or_default();
    let Some(next) = set_map_launch(&current, map) else {
        return Ok(None);
    };
    let write = crate::autoexec::write_autoexec(game_root, &next, true)?;
    Ok(Some(write.backup.unwrap_or(write.path)))
}

#[cfg(test)]
mod tests;

