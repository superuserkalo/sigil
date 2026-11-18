//! Settings > Diagnostics: Deadlock Mod Manager's list of network checks (2.3, behaviour only),
//! as pure functions plus one runner, and the local catalog's status and wipe.
//!
//! DMM checks, top to bottom: the network adapter, the internet, DNS, each service it talks to,
//! the local catalog, and on Windows the TCP auto-tuning level. Each result carries a status,
//! a message key, the probed address and the fixes to suggest. Sigil keeps the list and the
//! verdicts with one change: DMM proves "internet" with Cloudflare's trace endpoint, which is not
//! on Sigil's network allowlist, so Sigil's internet check is "any allowlisted service answered",
//! and its DNS check resolves the allowlisted host names without contacting anything else.
//!
//! The verdict functions ([`evaluate_network`], [`evaluate_probe`], [`evaluate_dns`],
//! [`evaluate_catalog`], [`evaluate_tcp_autotuning`]) take what was observed and return the row;
//! they never touch the network, so the UI and tests can drive them. [`run`] probes the
//! allowlisted [`SERVICES`] through the core agent (offline gate and proxy apply).
//!
//! The catalog is [`crate::catalog_mirror`]: [`catalog_status`] reads its size and age,
//! [`wipe_catalog`] deletes the mirror folder under the app data directory (never the game).

use std::net::ToSocketAddrs;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::error::Result;

/// How a check ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    Warning,
    Error,
    /// Not run here (the check does not apply on this system).
    Skipped,
}

/// One row of the diagnostics list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    /// Stable id: `network`, `internet`, `dns`, a service id, `catalog`, `tcpAutoTuning`.
    pub id: String,
    pub status: Status,
    /// What happened, as a key the UI translates: `online`, `offline`, `reachable`, `slow`,
    /// `timeout`, `unreachable`, `blocked`, `rateLimited`, `serverError`, `httpError`,
    /// `dnsFailed`, `dnsHostFailed`, `catalogReady`, `catalogStale`, `catalogEmpty`,
    /// `autoTuningOn`, `autoTuningRestricted`, `skipped`.
    pub message: String,
    pub latency_ms: Option<u64>,
    /// The address probed, so the player can see and share what failed.
    pub url: Option<String>,
    pub host: Option<String>,
    /// Fix keys, most useful first: `checkConnection`, `restartRouter`, `disableVpnFirewall`,
    /// `checkProxy`, `changeDns`, `flushDnsCache`, `fixInternetFirst`, `waitAndRetry`,
    /// `tryOtherNetwork`, `checkService`, `allowFirewall`, `syncCatalog`, `restartApp`,
    /// `enableAutoTuning`, `turnOffOffline`.
    pub fixes: Vec<String>,
}

impl Check {
    fn new(id: &str, status: Status, message: &str, fixes: &[&str]) -> Check {
        Check {
            id: id.to_string(),
            status,
            message: message.to_string(),
            latency_ms: None,
            url: None,
            host: None,
            fixes: fixes.iter().map(|f| f.to_string()).collect(),
        }
    }

    /// A plain English line for logs and a copied report.
    pub fn line(&self) -> String {
        let status = match self.status {
            Status::Ok => "ok",
            Status::Warning => "warning",
            Status::Error => "error",
            Status::Skipped => "skipped",
        };
        let mut line = format!("{}: {status} ({})", self.id, self.message);
        if let Some(ms) = self.latency_ms {
            line.push_str(&format!(", {ms} ms"));
        }
        if let Some(host) = &self.host {
            line.push_str(&format!(", {host}"));
        }
        line
    }
}

/// One allowlisted service the runner probes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Service {
    pub id: &'static str,
    pub label: &'static str,
    pub url: &'static str,
}

/// The services Sigil talks to, from the network allowlist in `docs/architecture.md`.
pub const SERVICES: [Service; 5] = [
    Service {
        id: "gamebanana",
        label: "GameBanana",
        url: "https://gamebanana.com/apiv11/Mod/Index?_nPerpage=1&_nPage=1&_aFilters[Generic_Game]=20948",
    },
    Service { id: "gamebananaFiles", label: "GameBanana files", url: "https://files.gamebanana.com/" },
    Service {
        id: "steam",
        label: "Steam patch feed",
        url: "https://api.steampowered.com/ISteamNews/GetNewsForApp/v2/?appid=1422450&count=1",
    },
    Service {
        id: "deadlockApi",
        label: "deadlock-api",
        url: "https://api.deadlock-api.com/v1/assets/heroes?only_active=true",
    },
    Service { id: "github", label: "GitHub", url: "https://api.github.com/" },
];

/// A probe slower than this is a warning.
pub const SLOW_MS: u64 = 3_000;
/// A probe gives up after this.
pub const TIMEOUT: Duration = Duration::from_secs(8);

/// What one probe saw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// An HTTP status came back.
    Response { status: u16, latency_ms: u64 },
    Timeout { latency_ms: u64 },
    /// No answer. `dns` is true when the host name did not resolve.
    Failed { latency_ms: u64, dns: bool },
}

impl Outcome {
    fn latency(self) -> u64 {
        match self {
            Outcome::Response { latency_ms, .. }
            | Outcome::Timeout { latency_ms }
            | Outcome::Failed { latency_ms, .. } => latency_ms,
        }
    }
}

fn host_of(url: &str) -> String {
    url.split("://").nth(1).unwrap_or(url).split(['/', '?']).next().unwrap_or("").to_string()
}

/// The network row: offline mode on (Sigil's switch) or not. DMM reads the adapter state here;
/// a desktop app cannot read that portably, so the offline switch and the probes stand in.
pub fn evaluate_network(offline: bool) -> Check {
    if offline {
        Check::new("network", Status::Error, "offline", &["turnOffOffline"])
    } else {
        Check::new("network", Status::Ok, "online", &[])
    }
}

/// One service's row from its probe, DMM's verdicts: a 2xx is reachable (slow above
/// [`SLOW_MS`]), 403 blocked, 429 rate limited, 5xx a server error, other codes an HTTP error, a
/// failed name lookup a DNS problem, no answer unreachable or a timeout. `internet_up` turns the
/// fixes for an unreachable service into "fix the internet first" when nothing answered.
pub fn evaluate_probe(service: &Service, outcome: Outcome, internet_up: bool, proxy: bool) -> Check {
    let host = host_of(service.url);
    let with_proxy = |fixes: &[&'static str]| -> Vec<&'static str> {
        let mut out: Vec<&str> = Vec::new();
        if proxy {
            out.push("checkProxy");
        }
        out.extend_from_slice(fixes);
        out
    };
    let (status, message, fixes): (Status, &str, Vec<&'static str>) = match outcome {
        Outcome::Failed { dns: true, .. } => {
            (Status::Error, "dnsHostFailed", vec!["changeDns", "flushDnsCache", "restartRouter"])
        }
        Outcome::Failed { .. } | Outcome::Timeout { .. } => {
            let message = if matches!(outcome, Outcome::Timeout { .. }) { "timeout" } else { "unreachable" };
            let fixes = if internet_up {
                with_proxy(&["disableVpnFirewall", "allowFirewall", "checkService"])
            } else {
                vec!["fixInternetFirst"]
            };
            (Status::Error, message, fixes)
        }
        Outcome::Response { status, latency_ms } => match status {
            200..=399 if latency_ms > SLOW_MS => {
                (Status::Warning, "slow", vec!["tryOtherNetwork", "disableVpnFirewall"])
            }
            200..=399 => (Status::Ok, "reachable", vec![]),
            403 => (Status::Error, "blocked", vec!["waitAndRetry", "tryOtherNetwork"]),
            429 => (Status::Warning, "rateLimited", vec!["waitAndRetry"]),
            500.. => (Status::Error, "serverError", vec!["checkService", "waitAndRetry"]),
            _ => (Status::Error, "httpError", vec!["checkService"]),
        },
    };
    let mut check = Check::new(service.id, status, message, &fixes);
    check.latency_ms = Some(outcome.latency());
    check.url = Some(service.url.to_string());
    check.host = Some(host);
    check
}

/// The internet row from the service rows: up when any service answered with an HTTP status.
pub fn evaluate_internet(services: &[Check], proxy: bool) -> Check {
    let answered = services
        .iter()
        .any(|check| !matches!(check.message.as_str(), "timeout" | "unreachable" | "dnsHostFailed"));
    if answered {
        Check::new("internet", Status::Ok, "online", &[])
    } else {
        let mut fixes = vec!["checkConnection", "restartRouter", "disableVpnFirewall"];
        if proxy {
            fixes.insert(0, "checkProxy");
        }
        Check::new("internet", Status::Error, "offline", &fixes)
    }
}

/// The DNS row: how many of the service host names resolved.
pub fn evaluate_dns(resolved: usize, total: usize, internet_up: bool) -> Check {
    if total == 0 || resolved == total {
        Check::new("dns", Status::Ok, "reachable", &[])
    } else if resolved == 0 {
        let fixes: &[&str] = if internet_up {
            &["changeDns", "flushDnsCache", "restartRouter"]
        } else {
            &["fixInternetFirst"]
        };
        Check::new("dns", Status::Error, "dnsFailed", fixes)
    } else {
        Check::new("dns", Status::Warning, "dnsHostFailed", &["changeDns", "flushDnsCache"])
    }
}

/// What the local catalog holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CatalogStatus {
    pub entries: usize,
    /// Unix seconds of the last finished sync step; 0 when it never synced.
    pub synced_at: i64,
    /// Bytes on disk.
    pub bytes: u64,
}

/// A catalog older than this is stale.
pub const CATALOG_STALE_SECONDS: i64 = 7 * 24 * 60 * 60;

/// The catalog row: ready, stale (older than a week) or empty.
pub fn evaluate_catalog(catalog: CatalogStatus, now: i64, internet_up: bool) -> Check {
    let sync: &[&str] = if internet_up { &["syncCatalog"] } else { &["fixInternetFirst"] };
    if catalog.entries == 0 {
        let fixes: &[&str] = if internet_up { &["syncCatalog", "restartApp"] } else { &["fixInternetFirst"] };
        return Check::new("catalog", Status::Error, "catalogEmpty", fixes);
    }
    if catalog.synced_at <= 0 || now - catalog.synced_at > CATALOG_STALE_SECONDS {
        return Check::new("catalog", Status::Warning, "catalogStale", sync);
    }
    Check::new("catalog", Status::Ok, "catalogReady", &[])
}

/// Windows TCP receive window auto-tuning, as `netsh interface tcp show global` prints it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoTuning {
    Disabled,
    HighlyRestricted,
    Restricted,
    Normal,
    Experimental,
}

/// The level from `netsh interface tcp show global` output ("Receive Window Auto-Tuning Level :
/// normal"). None when the line is not there.
pub fn parse_netsh_autotuning(output: &str) -> Option<AutoTuning> {
    let line = output.lines().find(|line| line.to_ascii_lowercase().contains("auto-tuning"))?;
    let value = line.rsplit(':').next()?.trim().to_ascii_lowercase().replace([' ', '_'], "");
    Some(match value.as_str() {
        "disabled" => AutoTuning::Disabled,
        "highlyrestricted" => AutoTuning::HighlyRestricted,
        "restricted" => AutoTuning::Restricted,
        "normal" => AutoTuning::Normal,
        "experimental" => AutoTuning::Experimental,
        _ => return None,
    })
}

/// DMM's auto-tuning row: disabled or restricted levels slow large downloads.
pub fn evaluate_tcp_autotuning(level: Option<AutoTuning>) -> Check {
    match level {
        None => Check::new("tcpAutoTuning", Status::Skipped, "skipped", &[]),
        Some(AutoTuning::Disabled | AutoTuning::HighlyRestricted | AutoTuning::Restricted) => {
            Check::new("tcpAutoTuning", Status::Warning, "autoTuningRestricted", &["enableAutoTuning"])
        }
        Some(_) => Check::new("tcpAutoTuning", Status::Ok, "autoTuningOn", &[]),
    }
}

/// The fix command a player runs in an administrator terminal for `enableAutoTuning`.
pub const ENABLE_AUTOTUNING_COMMAND: &str = "netsh interface tcp set global autotuninglevel=normal";

/// Read the auto-tuning level on Windows (`netsh`, read only). None elsewhere or on failure.
pub fn read_tcp_autotuning() -> Option<AutoTuning> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let output = std::process::Command::new("netsh")
            .args(["interface", "tcp", "show", "global"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()?;
        parse_netsh_autotuning(&String::from_utf8_lossy(&output.stdout))
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// The local catalog's size and age, from its files in the app data directory.
pub fn catalog_status(data_dir: &Path) -> CatalogStatus {
    let mirror = crate::catalog_mirror::Mirror::load(data_dir);
    let bytes = [crate::catalog_mirror::entries_path(data_dir), crate::catalog_mirror::meta_path(data_dir)]
        .iter()
        .filter_map(|path| std::fs::metadata(path).ok())
        .map(|meta| meta.len())
        .sum();
    CatalogStatus { entries: mirror.entries.len(), synced_at: mirror.meta.synced_at, bytes }
}

/// Delete the local catalog (the mirror folder under the app data directory). The next
/// background sync starts over. Returns whether anything was there.
pub fn wipe_catalog(data_dir: &Path) -> Result<bool> {
    let dir = crate::catalog_mirror::mirror_dir(data_dir);
    if !dir.exists() {
        return Ok(false);
    }
    std::fs::remove_dir_all(&dir)?;
    Ok(true)
}

fn probe(service: &Service) -> Outcome {
    let started = Instant::now();
    let elapsed = || started.elapsed().as_millis() as u64;
    let host = host_of(service.url);
    if (host.as_str(), 443).to_socket_addrs().map(|mut a| a.next().is_none()).unwrap_or(true) {
        return Outcome::Failed { latency_ms: elapsed(), dns: true };
    }
    let agent = crate::net::with_proxy(crate::net::builder())
        .middleware(crate::net::gate)
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .build()
        .new_agent();
    match agent.get(service.url).header("User-Agent", crate::gamebanana::GAMEBANANA_USER_AGENT).call() {
        Ok(response) => Outcome::Response { status: response.status().as_u16(), latency_ms: elapsed() },
        Err(ureq::Error::Timeout(_)) => Outcome::Timeout { latency_ms: elapsed() },
        Err(ureq::Error::HostNotFound) => Outcome::Failed { latency_ms: elapsed(), dns: true },
        Err(_) => Outcome::Failed { latency_ms: elapsed(), dns: false },
    }
}

/// Run every check in DMM's order: network, internet, DNS, each service, catalog, TCP
/// auto-tuning. Later checks run even when earlier ones fail. Blocks for up to a few seconds per
/// service; run it on a worker. Offline mode skips the probes.
pub fn run(data_dir: &Path, now: i64) -> Vec<Check> {
    let offline = crate::net::is_offline();
    let proxy = crate::net::current_proxy().is_some();
    let mut out = vec![evaluate_network(offline)];
    let services: Vec<Check> = if offline {
        Vec::new()
    } else {
        let outcomes: Vec<(Service, Outcome)> = std::thread::scope(|scope| {
            let handles: Vec<_> = SERVICES.iter().map(|s| scope.spawn(move || (*s, probe(s)))).collect();
            handles.into_iter().filter_map(|h| h.join().ok()).collect()
        });
        let up = outcomes.iter().any(|(_, o)| matches!(o, Outcome::Response { .. }));
        outcomes.iter().map(|(s, o)| evaluate_probe(s, *o, up, proxy)).collect()
    };
    let internet = evaluate_internet(&services, proxy);
    let internet_up = internet.status == Status::Ok && !offline;
    let resolved = services.iter().filter(|c| c.message != "dnsHostFailed").count();
    out.push(internet);
    out.push(evaluate_dns(resolved, services.len(), internet_up));
    out.extend(services);
    out.push(evaluate_catalog(catalog_status(data_dir), now, internet_up));
    out.push(evaluate_tcp_autotuning(read_tcp_autotuning()));
    out
}

/// The results as plain text for "Copy results".
pub fn report(checks: &[Check]) -> String {
    checks.iter().map(Check::line).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_verdicts_follow_dmm() {
        let s = &SERVICES[0];
        let ok = evaluate_probe(s, Outcome::Response { status: 200, latency_ms: 40 }, true, false);
        assert_eq!((ok.status, ok.message.as_str()), (Status::Ok, "reachable"));
        assert_eq!(ok.host.as_deref(), Some("gamebanana.com"));
        let slow = evaluate_probe(s, Outcome::Response { status: 204, latency_ms: 5_000 }, true, false);
        assert_eq!(slow.message, "slow");
        assert_eq!(evaluate_probe(s, Outcome::Response { status: 403, latency_ms: 1 }, true, false).message, "blocked");
        let limited = evaluate_probe(s, Outcome::Response { status: 429, latency_ms: 1 }, true, false);
        assert_eq!((limited.status, limited.message.as_str()), (Status::Warning, "rateLimited"));
        assert_eq!(evaluate_probe(s, Outcome::Response { status: 502, latency_ms: 1 }, true, false).message, "serverError");
        assert_eq!(evaluate_probe(s, Outcome::Response { status: 404, latency_ms: 1 }, true, false).message, "httpError");
        let dns = evaluate_probe(s, Outcome::Failed { latency_ms: 1, dns: true }, true, false);
        assert_eq!(dns.message, "dnsHostFailed");
        let down = evaluate_probe(s, Outcome::Timeout { latency_ms: 8000 }, false, true);
        assert_eq!((down.message.as_str(), down.fixes.clone()), ("timeout", vec!["fixInternetFirst".to_string()]));
        let blocked = evaluate_probe(s, Outcome::Failed { latency_ms: 1, dns: false }, true, true);
        assert_eq!(blocked.fixes[0], "checkProxy");
    }

    #[test]
    fn summary_rows() {
        assert_eq!(evaluate_network(true).status, Status::Error);
        let rows = vec![evaluate_probe(&SERVICES[0], Outcome::Timeout { latency_ms: 1 }, false, false)];
        assert_eq!(evaluate_internet(&rows, false).status, Status::Error);
        let rows = vec![evaluate_probe(&SERVICES[0], Outcome::Response { status: 500, latency_ms: 1 }, true, false)];
        assert_eq!(evaluate_internet(&rows, false).status, Status::Ok, "a server error still proves the internet");
        assert_eq!(evaluate_dns(5, 5, true).status, Status::Ok);
        assert_eq!(evaluate_dns(0, 5, true).message, "dnsFailed");
        assert_eq!(evaluate_dns(3, 5, true).status, Status::Warning);
        let now = 1_000_000_000;
        assert_eq!(evaluate_catalog(CatalogStatus::default(), now, true).message, "catalogEmpty");
        let fresh = CatalogStatus { entries: 10, synced_at: now - 60, bytes: 1 };
        assert_eq!(evaluate_catalog(fresh, now, true).message, "catalogReady");
        let old = CatalogStatus { synced_at: now - CATALOG_STALE_SECONDS - 1, ..fresh };
        assert_eq!(evaluate_catalog(old, now, false).fixes, vec!["fixInternetFirst".to_string()]);
        assert!(report(&[evaluate_network(false)]).starts_with("network: ok"));
    }

    #[test]
    fn reads_netsh_output() {
        let text = "TCP Global Parameters\n----\nReceive-Side Scaling State          : enabled\nReceive Window Auto-Tuning Level    : highlyrestricted\n";
        assert_eq!(parse_netsh_autotuning(text), Some(AutoTuning::HighlyRestricted));
        assert_eq!(evaluate_tcp_autotuning(Some(AutoTuning::HighlyRestricted)).message, "autoTuningRestricted");
        assert_eq!(evaluate_tcp_autotuning(Some(AutoTuning::Normal)).status, Status::Ok);
        assert_eq!(evaluate_tcp_autotuning(None).status, Status::Skipped);
        assert_eq!(parse_netsh_autotuning("nothing"), None);
    }

    #[test]
    fn catalog_status_and_wipe_use_the_mirror_folder() {
        let dir = std::env::temp_dir().join(format!("sigil-diag-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(crate::catalog_mirror::mirror_dir(&dir)).unwrap();
        assert_eq!(catalog_status(&dir).entries, 0);
        assert!(wipe_catalog(&dir).unwrap());
        assert!(!crate::catalog_mirror::mirror_dir(&dir).exists());
        assert!(!wipe_catalog(&dir).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
