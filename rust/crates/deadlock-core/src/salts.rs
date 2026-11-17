//! Match data sharing: find Deadlock match salts Steam has already downloaded and send them to
//! deadlock-api's public ingest endpoint, so the community API can serve those matches.
//!
//! When the game shows a match (a replay, a match history entry), Steam downloads its metadata
//! or replay from `replayNNN.valve.net/1422450/<match id>_<salt>.meta.bz2` (or `.dem.bz2`) and
//! keeps the response in its HTTP cache, `<Steam>/appcache/httpcache`. The cached file starts
//! with the request URL. This module reads the first bytes of each cached file, keeps the URLs
//! that match that shape and turns them into the documented `POST /v1/matches/salts` body:
//! match id, cluster id, metadata salt or replay salt, and a fixed submitter name.
//!
//! Nothing here signs in to Steam, reads an account id or sends a file. Only the four numbers
//! per match and the submitter name leave the machine, and only when the caller sends them.

use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

/// The documented ingest endpoint.
pub const INGEST_URL: &str = "https://api.deadlock-api.com/v1/matches/salts";

/// The submitter name deadlock-api records for Sigil's batches.
pub const SUBMITTER: &str = "Sigil";

const APP_ID: &str = "1422450";
const HEAD_BYTES: usize = 512;
const HOST_SUFFIX: &str = ".valve.net/";
/// A full cache can hold tens of thousands of entries; a scan stops after this many files.
const MAX_FILES: usize = 60_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SaltKind {
    Metadata,
    Replay,
}

impl SaltKind {
    pub fn label(self) -> &'static str {
        match self {
            SaltKind::Metadata => "metadata",
            SaltKind::Replay => "replay",
        }
    }
}

/// One match salt as Steam's cache names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MatchSalt {
    pub match_id: u64,
    pub cluster_id: u32,
    pub kind: SaltKind,
    pub salt: u32,
}

impl MatchSalt {
    /// A stable key for "already sent".
    pub fn key(&self) -> String {
        format!("{}:{}", self.match_id, self.kind.label())
    }

    /// The JSON object deadlock-api expects for this salt.
    pub fn to_json(&self) -> Value {
        let (metadata, replay) = match self.kind {
            SaltKind::Metadata => (Some(self.salt), None),
            SaltKind::Replay => (None, Some(self.salt)),
        };
        json!({
            "match_id": self.match_id,
            "cluster_id": self.cluster_id,
            "metadata_salt": metadata,
            "replay_salt": replay,
            "username": SUBMITTER,
        })
    }
}

/// Parse `replay<cluster>.valve.net/1422450/<match>_<salt>.(meta|dem).bz2`, with or without a
/// scheme and a query string.
pub fn parse_replay_url(url: &str) -> Option<MatchSalt> {
    let url = url.split(['?', '#']).next()?;
    let url = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .unwrap_or(url);
    let (host, path) = url.split_once('/')?;
    let cluster = host.strip_prefix("replay")?.strip_suffix(".valve.net")?;
    if cluster.is_empty() || !cluster.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (app, file) = path.split_once('/')?;
    if app != APP_ID || file.contains('/') {
        return None;
    }
    let (stem, kind) = if let Some(stem) = file.strip_suffix(".meta.bz2") {
        (stem, SaltKind::Metadata)
    } else if let Some(stem) = file.strip_suffix(".dem.bz2") {
        (stem, SaltKind::Replay)
    } else {
        return None;
    };
    let (id, salt) = stem.split_once('_')?;
    Some(MatchSalt {
        match_id: id.parse().ok().filter(|id| *id > 0)?,
        cluster_id: cluster.parse().ok()?,
        kind,
        salt: salt.parse().ok()?,
    })
}

/// The first replay URL in the head of a cached file.
pub fn find_in_head(head: &[u8]) -> Option<MatchSalt> {
    let suffix = HOST_SUFFIX.as_bytes();
    let mut from = 0;
    while let Some(offset) = find(&head[from..], suffix) {
        let at = from + offset;
        from = at + 1;
        // Walk back over the host name: letters, digits and dots.
        let mut start = at;
        while start > 0 && (head[start - 1].is_ascii_alphanumeric() || head[start - 1] == b'.') {
            start -= 1;
        }
        // And forward to the end of the URL.
        let mut end = at + suffix.len();
        while end < head.len() && is_url_byte(head[end]) {
            end += 1;
        }
        if let Ok(url) = std::str::from_utf8(&head[start..end])
            && let Some(salt) = parse_replay_url(url)
        {
            return Some(salt);
        }
    }
    None
}

fn is_url_byte(b: u8) -> bool {
    b.is_ascii_graphic() && !matches!(b, b'"' | b'\'' | b'<' | b'>')
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// `<root>/appcache/httpcache` for every Steam root that has one.
pub fn cache_dirs(steam_roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for root in steam_roots {
        let dir = root.join("appcache").join("httpcache");
        if dir.is_dir() && !out.contains(&dir) {
            out.push(dir);
        }
    }
    out
}

/// What a scan found.
#[derive(Clone, Debug, Default)]
pub struct Scan {
    pub salts: Vec<MatchSalt>,
    pub files: usize,
}

/// Read the head of every file under the cache folders and keep the replay salts, one per
/// match and kind.
pub fn scan(dirs: &[PathBuf]) -> Scan {
    let mut found: BTreeSet<MatchSalt> = BTreeSet::new();
    let mut files = 0;
    for dir in dirs {
        walk(dir, 0, &mut files, &mut found);
    }
    let mut seen = BTreeSet::new();
    let salts = found
        .into_iter()
        .filter(|s| seen.insert((s.match_id, s.kind)))
        .collect();
    Scan { salts, files }
}

fn walk(dir: &Path, depth: usize, files: &mut usize, found: &mut BTreeSet<MatchSalt>) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if *files >= MAX_FILES {
            return;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            walk(&path, depth + 1, files, found);
        } else if kind.is_file() {
            *files += 1;
            let mut head = [0u8; HEAD_BYTES];
            let Ok(mut file) = fs::File::open(&path) else {
                continue;
            };
            let read = file.read(&mut head).unwrap_or(0);
            if let Some(salt) = find_in_head(&head[..read]) {
                found.insert(salt);
            }
        }
    }
}

/// The request body for a batch.
pub fn payload(salts: &[MatchSalt]) -> Value {
    Value::Array(salts.iter().map(MatchSalt::to_json).collect())
}

/// POST one batch to deadlock-api. Goes through the offline gate.
pub fn send(salts: &[MatchSalt]) -> Result<(), String> {
    if salts.is_empty() {
        return Ok(());
    }
    let body = payload(salts).to_string();
    let response = crate::net::agent(Duration::from_secs(30))
        .post(INGEST_URL)
        .header("Content-Type", "application/json")
        .header("User-Agent", "Sigil")
        .send(body.as_bytes());
    match response {
        Ok(_) => Ok(()),
        Err(ureq::Error::StatusCode(code)) => Err(format!("deadlock-api answered {code}")),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_metadata_and_replay_urls() {
        let meta = parse_replay_url("http://replay404.valve.net/1422450/37959196_937530290.meta.bz2")
            .unwrap();
        assert_eq!(meta.match_id, 37_959_196);
        assert_eq!(meta.cluster_id, 404);
        assert_eq!(meta.kind, SaltKind::Metadata);
        assert_eq!(meta.salt, 937_530_290);
        let dem = parse_replay_url("replay183.valve.net/1422450/42476710_428480166.dem.bz2?v=2")
            .unwrap();
        assert_eq!(dem.kind, SaltKind::Replay);
        assert_eq!(dem.cluster_id, 183);
    }

    #[test]
    fn rejects_other_games_hosts_and_shapes() {
        for url in [
            "http://replay404.valve.net/570/37959196_937530290.meta.bz2",
            "http://cdn.valve.net/1422450/37959196_937530290.meta.bz2",
            "http://replay.valve.net/1422450/37959196_937530290.meta.bz2",
            "http://replay404.valve.net/1422450/37959196.meta.bz2",
            "http://replay404.valve.net/1422450/x_937530290.meta.bz2",
            "http://replay404.valve.net/1422450/37959196_937530290.zip",
            "http://replay404.valve.net/1422450/a/37959196_937530290.meta.bz2",
        ] {
            assert!(parse_replay_url(url).is_none(), "{url}");
        }
    }

    #[test]
    fn finds_the_url_inside_a_cache_header() {
        let mut head = b"\x00\x01KEY\x00http://replay12.valve.net/1422450/100_200.meta.bz2\x00rest".to_vec();
        head.extend_from_slice(&[0xff; 32]);
        let salt = find_in_head(&head).unwrap();
        assert_eq!((salt.match_id, salt.cluster_id, salt.salt), (100, 12, 200));
        assert!(find_in_head(b"http://example.com/a.valve.net/").is_none());
    }

    #[test]
    fn scans_a_cache_folder_and_dedupes() {
        let root = std::env::temp_dir().join(format!("sigil-salts-{}", std::process::id()));
        let cache = root.join("appcache").join("httpcache").join("0a");
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("a"), b"http://replay1.valve.net/1422450/5_6.meta.bz2\0").unwrap();
        fs::write(cache.join("b"), b"http://replay1.valve.net/1422450/5_6.meta.bz2\0").unwrap();
        fs::write(cache.join("c"), b"http://replay2.valve.net/1422450/5_9.dem.bz2\0").unwrap();
        fs::write(cache.join("d"), b"nothing here").unwrap();
        let dirs = cache_dirs(&[root.clone(), root.join("missing")]);
        assert_eq!(dirs.len(), 1);
        let scan = scan(&dirs);
        assert_eq!(scan.files, 4);
        assert_eq!(scan.salts.len(), 2);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_payload_has_exactly_the_documented_fields() {
        let salt = MatchSalt {
            match_id: 1,
            cluster_id: 2,
            kind: SaltKind::Replay,
            salt: 3,
        };
        let body = payload(&[salt]);
        let object = body[0].as_object().unwrap();
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(
            keys,
            ["cluster_id", "match_id", "metadata_salt", "replay_salt", "username"]
        );
        assert_eq!(body[0]["metadata_salt"], Value::Null);
        assert_eq!(body[0]["replay_salt"], 3);
        assert_eq!(body[0]["username"], SUBMITTER);
    }
}
