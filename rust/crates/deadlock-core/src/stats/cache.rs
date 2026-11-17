//! A small disk cache for the public Deadlock API so a player screen opens without
//! waiting on the network again.
//!
//! One file per request: `<dir>/<hash>.httpcache`, whose first line is
//! `<fetched_at unix seconds>\t<key>` and whose rest is the response body. Every
//! endpoint has its own lifetime ([`TTL_*`]); an entry inside its lifetime is used
//! without a request, and an older entry is the fallback when the API cannot be
//! reached. The cache never holds credentials: keys are paths and public account ids.
//!
//! The directory is the app data directory (`cache/stats`), so deleting it costs a
//! refetch and nothing else.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::util::write_atomic;

/// How long a stored response is used without asking the API again.
/// Match history: 10 minutes.
pub const TTL_MATCH_HISTORY: i64 = 10 * 60;
/// One match's metadata never changes, so it is kept for a week.
pub const TTL_MATCH_METADATA: i64 = 7 * 86_400;
/// Hero stats: 10 minutes.
pub const TTL_HERO_STATS: i64 = 10 * 60;
/// Rank: 30 minutes.
pub const TTL_RANK: i64 = 30 * 60;
/// Popularity per badge allows 5 requests a minute, so it is kept for 12 hours.
pub const TTL_RANK_DISTRIBUTION: i64 = 12 * 3_600;
/// Steam profiles: 24 hours.
pub const TTL_STEAM_PROFILE: i64 = 24 * 3_600;
/// Teammates and opponents: 1 hour.
pub const TTL_SOCIAL: i64 = 3_600;
/// Leaderboards: 1 hour (Valve refreshes its own board hourly).
pub const TTL_LEADERBOARD: i64 = 3_600;
/// Hero, rank, season and item assets: 7 days.
pub const TTL_ASSETS: i64 = 7 * 86_400;
/// Entries older than this are deleted when the cache grows past [`MAX_FILES`].
const MAX_AGE_SECONDS: i64 = 30 * 86_400;
const MAX_FILES: usize = 512;

/// How old the newest load's data is.
///
/// `seconds` is the oldest response body the load used: zero when every request
/// reached the API, larger when a stored copy answered. `stale` is true when the
/// API failed and a copy past its lifetime was used instead.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Age {
    pub seconds: i64,
    pub stale: bool,
}

impl Age {
    /// Keep the oldest age seen and remember any failure.
    pub fn merge(&mut self, other: Age) {
        self.seconds = self.seconds.max(other.seconds);
        self.stale |= other.stale;
    }

    pub fn is_cached(&self) -> bool {
        self.seconds > 0
    }
}

/// A body read back from disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stored {
    pub body: String,
    pub fetched_at: i64,
}

/// One directory of stored responses.
#[derive(Clone, Debug)]
pub struct ResponseCache {
    dir: PathBuf,
}

impl ResponseCache {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        ResponseCache { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{:016x}.httpcache", hash(key)))
    }

    /// The stored body for `key`, whatever its age.
    pub fn read(&self, key: &str) -> Option<Stored> {
        let text = fs::read_to_string(self.path(key)).ok()?;
        let (header, body) = text.split_once('\n')?;
        let (at, stored_key) = header.split_once('\t')?;
        // Two different keys can share a hash; keep the file honest about which it holds.
        if stored_key != key {
            return None;
        }
        Some(Stored {
            body: body.to_string(),
            fetched_at: at.parse().ok()?,
        })
    }

    /// The stored body when it is no older than `ttl` seconds.
    pub fn read_fresh(&self, key: &str, ttl: i64, now: i64) -> Option<Stored> {
        let stored = self.read(key)?;
        (now - stored.fetched_at <= ttl.max(0)).then_some(stored)
    }

    /// Store one body, replacing any older copy. Failure is not an error for the
    /// caller: a cache that cannot be written only means the next load asks again.
    pub fn write(&self, key: &str, body: &str, now: i64) -> Result<()> {
        let text = format!("{now}\t{key}\n{body}");
        write_atomic(&self.path(key), text.as_bytes())?;
        self.prune(now);
        Ok(())
    }

    /// Remove entries past [`MAX_AGE_SECONDS`] once the directory is large.
    fn prune(&self, now: i64) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        let mut files: Vec<(i64, PathBuf)> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("httpcache") {
                continue;
            }
            let at = fs::read_to_string(&path)
                .ok()
                .and_then(|text| {
                    text.split('\n')
                        .next()
                        .and_then(|header| header.split_once('\t'))
                        .and_then(|(at, _)| at.parse::<i64>().ok())
                })
                .unwrap_or(0);
            files.push((at, path));
        }
        if files.len() <= MAX_FILES {
            return;
        }
        for (at, path) in files {
            if now - at > MAX_AGE_SECONDS {
                let _ = fs::remove_file(path);
            }
        }
    }
}

/// The cache key for one request: the path and its query, in the order sent.
pub fn key(path: &str, query: &[(&str, String)]) -> String {
    let mut key = path.to_string();
    for (index, (name, value)) in query.iter().enumerate() {
        key.push(if index == 0 { '?' } else { '&' });
        key.push_str(name);
        key.push('=');
        key.push_str(value);
    }
    key
}

/// FNV-1a, enough to name a file after a key.
fn hash(key: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in key.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("dah-stats-cache-{name}-{stamp}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn keys_carry_the_query_in_order() {
        let query = vec![("account_ids", "1".to_string()), ("limit", "5".to_string())];
        assert_eq!(
            key("/v1/players/steam", &query),
            "/v1/players/steam?account_ids=1&limit=5"
        );
        assert_eq!(key("/v1/assets/heroes", &[]), "/v1/assets/heroes");
    }

    #[test]
    fn a_stored_body_reads_back_inside_its_lifetime() {
        let dir = temp_dir("fresh");
        let cache = ResponseCache::new(&dir);
        cache.write("/v1/x", "{\"a\":1}", 1_000).expect("write");
        let stored = cache.read("/v1/x").expect("stored");
        assert_eq!(stored.body, "{\"a\":1}");
        assert_eq!(stored.fetched_at, 1_000);
        assert!(cache.read_fresh("/v1/x", 60, 1_030).is_some());
        assert!(cache.read_fresh("/v1/x", 60, 1_061).is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn another_key_with_the_same_file_is_not_served() {
        let dir = temp_dir("foreign");
        let cache = ResponseCache::new(&dir);
        cache.write("/v1/x", "body", 5).expect("write");
        let path = cache.path("/v1/x");
        fs::write(&path, "5\t/v1/other\nbody").expect("rewrite");
        assert!(cache.read("/v1/x").is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_and_corrupt_files_read_as_none() {
        let dir = temp_dir("corrupt");
        let cache = ResponseCache::new(&dir);
        assert!(cache.read("/v1/x").is_none());
        fs::write(cache.path("/v1/x"), "no header").expect("write");
        assert!(cache.read("/v1/x").is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn age_keeps_the_oldest_copy_and_any_failure() {
        let mut age = Age::default();
        assert!(!age.is_cached());
        age.merge(Age {
            seconds: 30,
            stale: false,
        });
        age.merge(Age {
            seconds: 120,
            stale: true,
        });
        age.merge(Age {
            seconds: 5,
            stale: false,
        });
        assert_eq!(
            age,
            Age {
                seconds: 120,
                stale: true
            }
        );
        assert!(age.is_cached());
    }
}
