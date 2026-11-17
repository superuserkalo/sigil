//! GameBanana's file servers: the public directory (which `filecacheN` mirrors are up and how
//! busy they are) and a speed test against a real file. Ported from Grimoire's
//! `gamebananaFileServers.ts` (MIT): the directory endpoint, the 256 KiB range probe and the
//! three-server limit are Grimoire's.
//!
//! Hosts: `gamebanana.com` (directory, the test file) and `*.gamebanana.com` file servers,
//! all already on the network allowlist under GameBanana.

use std::io::Read;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::error::{Error, Result};

pub const DIRECTORY_URL: &str = "https://gamebanana.com/apiv11/Util/Fileservers?_nPage=1";
pub const STATUS_PAGE: &str = "https://gamebanana.com/fileservers";
pub const CANONICAL_HOST: &str = "files.gamebanana.com";
/// Bytes one probe reads.
pub const PROBE_BYTES: u64 = 256 * 1024;
/// Servers a test measures.
pub const PROBE_COUNT: usize = 3;

#[derive(Debug, Clone, PartialEq)]
pub struct Server {
    /// `filecache12`.
    pub name: String,
    pub up: bool,
    /// Bytes per second over the last 10 minutes, as GameBanana reports it.
    pub rate: f64,
}

impl Server {
    pub fn host(&self) -> String {
        format!("{}.gamebanana.com", self.name)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Directory {
    /// Servers with a rate, fastest first.
    pub servers: Vec<Server>,
    pub available: usize,
    pub total: usize,
}

impl Directory {
    /// Every server online and reporting.
    pub fn healthy(&self) -> bool {
        self.available > 0 && self.available == self.total && self.servers.iter().filter(|s| s.up).count() == self.available
    }
}

/// `filecacheN` and nothing else.
pub fn valid_name(name: &str) -> bool {
    name.strip_prefix("filecache")
        .is_some_and(|n| !n.is_empty() && n.len() <= 4 && n.chars().all(|c| c.is_ascii_digit()))
}

pub fn parse_directory(body: &str) -> Result<Directory> {
    let v: Value = serde_json::from_str(body).map_err(|e| Error::msg(format!("Not JSON: {e}")))?;
    let records = v
        .get("_aRecords")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::msg("The file server list has no records."))?;
    let mut dir = Directory::default();
    for r in records {
        let domain = r.get("_sDomain").and_then(Value::as_str).unwrap_or("");
        let name = domain.trim_end_matches(".gamebanana.com");
        if !valid_name(name) {
            continue;
        }
        let state = r.get("_sState").and_then(Value::as_str).unwrap_or("");
        if state == "terminated" {
            continue;
        }
        dir.total += 1;
        let up = state == "up";
        if up {
            dir.available += 1;
        }
        let rate = r
            .get("_aStats")
            .and_then(|s| s.get("_a10min"))
            .and_then(|s| s.get("_fRate"))
            .and_then(Value::as_f64)
            .filter(|r| r.is_finite() && *r >= 0.0);
        if let Some(rate) = rate {
            dir.servers.push(Server {
                name: name.to_string(),
                up,
                rate,
            });
        }
    }
    dir.servers
        .sort_by(|a, b| b.rate.partial_cmp(&a.rate).unwrap_or(std::cmp::Ordering::Equal).then(a.name.cmp(&b.name)));
    Ok(dir)
}

fn agent(timeout: u64) -> ureq::Agent {
    crate::net::with_proxy(ureq::Agent::config_builder())
        .middleware(crate::net::gate)
        .timeout_global(Some(Duration::from_secs(timeout)))
        .max_redirects(0)
        .http_status_as_error(false)
        .build()
        .new_agent()
}

pub fn fetch_directory() -> Result<Directory> {
    let mut r = agent(10)
        .get(DIRECTORY_URL)
        .header("User-Agent", crate::gamebanana::GAMEBANANA_USER_AGENT)
        .header("Accept", "application/json")
        .call()
        .map_err(|e| Error::Network(e.to_string()))?;
    if !r.status().is_success() {
        return Err(Error::Network(format!("GameBanana answered HTTP {}", r.status())));
    }
    let body = r.body_mut().read_to_string().map_err(|e| Error::Network(e.to_string()))?;
    parse_directory(&body)
}

/// One measured server. None speed means it failed.
#[derive(Debug, Clone, PartialEq)]
pub struct Probe {
    pub name: String,
    pub bytes_per_second: Option<f64>,
}

/// `Content-Range: bytes 0-N/T` within the probe size; the byte count it promises.
pub fn probe_range_ok(header: &str) -> Option<u64> {
    let rest = header.trim().strip_prefix("bytes")?.trim_start();
    let (span, _) = rest.split_once('/')?;
    let (a, b) = span.split_once('-')?;
    if a.trim() != "0" {
        return None;
    }
    let end: u64 = b.trim().parse().ok()?;
    (end < PROBE_BYTES).then_some(end + 1)
}

/// Read the first 256 KiB of `url` and time it.
pub fn probe(url: &str) -> Option<f64> {
    let started = Instant::now();
    let mut r = agent(6)
        .get(url)
        .header("User-Agent", crate::gamebanana::GAMEBANANA_USER_AGENT)
        .header("Range", &format!("bytes=0-{}", PROBE_BYTES - 1))
        .call()
        .ok()?;
    if r.status().as_u16() != 206 {
        return None;
    }
    let expected = probe_range_ok(r.headers().get("content-range")?.to_str().ok()?)?;
    let mut buf = Vec::with_capacity(expected as usize);
    r.body_mut().with_config().limit(PROBE_BYTES).reader().read_to_end(&mut buf).ok()?;
    if buf.len() as u64 != expected {
        return None;
    }
    let ms = started.elapsed().as_secs_f64().max(0.001);
    Some(buf.len() as f64 / ms)
}

/// The path of a real Deadlock file on the canonical host, found from the newest file of
/// GameBanana's most popular Deadlock mod: its `/dl/<id>` link redirects to a file server.
pub fn test_path() -> Result<String> {
    let mods = crate::gamebanana::fetch_deadlock_mods(1, 1)?;
    let first = mods.first().ok_or_else(|| Error::msg("GameBanana listed no mods to test with."))?;
    let file = crate::install::fetch_gamebanana_file(first.id)?;
    let url = file.download_url.clone();
    let r = agent(10)
        .head(&url)
        .header("User-Agent", crate::gamebanana::GAMEBANANA_USER_AGENT)
        .call()
        .map_err(|e| Error::Network(e.to_string()))?;
    let location = if r.status().is_redirection() {
        r.headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
            .ok_or_else(|| Error::Network("The file link did not redirect.".into()))?
    } else {
        url
    };
    let rest = location
        .strip_prefix("https://")
        .ok_or_else(|| Error::Network("The file link left https.".into()))?;
    let (_, path) = rest.split_once('/').ok_or_else(|| Error::Network("The file link has no path.".into()))?;
    Ok(format!("/{path}"))
}

/// Measure up to three servers, fastest by GameBanana's numbers first.
pub fn test_servers(dir: &Directory) -> Result<Vec<Probe>> {
    let path = test_path()?;
    let picks: Vec<&Server> = dir.servers.iter().filter(|s| s.up).take(PROBE_COUNT).collect();
    let results: Vec<Probe> = std::thread::scope(|scope| {
        let handles: Vec<_> = picks
            .iter()
            .map(|s| {
                let url = format!("https://{}{}", s.host(), path);
                let name = s.name.clone();
                scope.spawn(move || Probe {
                    name,
                    bytes_per_second: probe(&url),
                })
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    });
    if results.iter().all(|p| p.bytes_per_second.is_none()) {
        return Err(Error::Network("No server answered the test.".into()));
    }
    Ok(results)
}

/// The fastest server that passed.
pub fn fastest(probes: &[Probe]) -> Option<&Probe> {
    probes
        .iter()
        .filter(|p| p.bytes_per_second.is_some())
        .max_by(|a, b| a.bytes_per_second.partial_cmp(&b.bytes_per_second).unwrap_or(std::cmp::Ordering::Equal))
}

/// `2.4 MB/s` or `830 KB/s`.
pub fn speed_label(bytes_per_second: f64) -> String {
    let kb = bytes_per_second / 1024.0;
    if kb >= 1024.0 {
        format!("{:.1} MB/s", kb / 1024.0)
    } else {
        format!("{} KB/s", kb.round() as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_keeps_rated_servers_fastest_first() {
        let body = r#"{"_aRecords":[
          {"_sDomain":"filecache2.gamebanana.com","_sState":"up","_aStats":{"_a10min":{"_fRate":100.5}}},
          {"_sDomain":"filecache9.gamebanana.com","_sState":"up","_aStats":{"_a10min":{"_fRate":900}}},
          {"_sDomain":"filecache3.gamebanana.com","_sState":"down","_aStats":{}},
          {"_sDomain":"filecache4.gamebanana.com","_sState":"terminated"},
          {"_sDomain":"evil.example.com","_sState":"up","_aStats":{"_a10min":{"_fRate":5000}}}
        ]}"#;
        let d = parse_directory(body).unwrap();
        assert_eq!(d.total, 3);
        assert_eq!(d.available, 2);
        assert_eq!(d.servers.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["filecache9", "filecache2"]);
        assert!(!d.healthy());
        assert_eq!(d.servers[0].host(), "filecache9.gamebanana.com");
    }

    #[test]
    fn probes_and_labels() {
        assert_eq!(probe_range_ok("bytes 0-262143/9999999"), Some(262_144));
        assert_eq!(probe_range_ok("bytes 0-262144/9999999"), None);
        assert_eq!(probe_range_ok("bytes 5-10/20"), None);
        assert_eq!(speed_label(2.5 * 1024.0 * 1024.0), "2.5 MB/s");
        assert_eq!(speed_label(830.0 * 1024.0), "830 KB/s");
        let probes = vec![
            Probe { name: "a".into(), bytes_per_second: Some(10.0) },
            Probe { name: "b".into(), bytes_per_second: None },
            Probe { name: "c".into(), bytes_per_second: Some(30.0) },
        ];
        assert_eq!(fastest(&probes).unwrap().name, "c");
        assert!(valid_name("filecache12"));
        assert!(!valid_name("files"));
    }
}
