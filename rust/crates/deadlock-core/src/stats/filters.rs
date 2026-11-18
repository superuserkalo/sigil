//! The scope a statistics view is read in, and the request plumbing the
//! filtered views share.
//!
//! The Deadlock API selects ranked and unranked with one `match_mode` value:
//! `ranked`, `unranked`, or both joined by a comma. A start time narrows the
//! window with `min_unix_timestamp`, and `game_mode=normal` keeps the view on
//! the ordinary game rather than Street Brawl.
//!
//! The Items and Party views ask their endpoints through the one request helper
//! here, so they share the response cache in [`cache`] with
//! [`StatsClient`](crate::stats::StatsClient). A caller that needs the age of
//! the answered data uses the client directly; these free functions return the
//! parsed rows only.

use std::path::PathBuf;
use std::time::Duration;

use crate::error::{Error, Result};
use crate::stats::cache::{self, ResponseCache};
use crate::stats::{STATS_API_BASE, STATS_API_KEY_ENV, STATS_USER_AGENT};

/// How long an analytics answer is reused from disk. The API keeps one for an
/// hour on its own side, so a shorter lifetime would only repeat the request.
pub(crate) const ANALYTICS_TTL: i64 = 3_600;

const REQUEST_TIMEOUT_SECONDS: u64 = 45;
const BODY_SNIPPET_CHARS: usize = 240;

/// The scope a statistics view is read in: which match modes, and how far back.
///
/// The start time is Unix seconds. `Some(0)` is treated as no start filter,
/// because asking for all of history is a query the analytics endpoints cannot
/// answer in time; the endpoint's own default window applies instead.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatsFilter {
    /// `Some(true)` for ranked only, `Some(false)` for unranked only, `None`
    /// for both.
    pub ranked: Option<bool>,
    /// Matches that started at or after this Unix time in seconds.
    pub since: Option<u64>,
}

/// The query parameters a filtered statistics request carries.
///
/// The order is fixed so a cache key built from it is stable. `game_mode` stays
/// `normal`; the player statistics screens do not cover Street Brawl.
pub fn filter_query(filter: &StatsFilter) -> Vec<(&'static str, String)> {
    let mut query = vec![
        ("game_mode", "normal".to_string()),
        ("match_mode", filter.match_modes().to_string()),
    ];
    if let Some(since) = filter.since.filter(|since| *since > 0) {
        query.push(("min_unix_timestamp", since.to_string()));
    }
    query
}

impl StatsFilter {
    /// The modes as the API spells them: `ranked,unranked`, `ranked` or
    /// `unranked`.
    fn match_modes(&self) -> &'static str {
        match self.ranked {
            Some(true) => "ranked",
            Some(false) => "unranked",
            None => "ranked,unranked",
        }
    }
}

/// Ok for a SteamID3 account id, the form the statistics endpoints take.
pub(crate) fn ensure_account(account: u32) -> Result<()> {
    if account == 0 {
        return Err(Error::msg(
            "Account id must be a SteamID3 greater than zero. Leaderboard entries and the Steam \
             profile search show the account id.",
        ));
    }
    Ok(())
}

/// A response body, used for its own lifetime, falling back to an older copy
/// when the API cannot be reached.
///
/// The cache key is the same one [`StatsClient`](crate::stats::StatsClient)
/// builds, and the directory is the one the desktop attaches to that client, so
/// a response read on one screen answers another.
pub(crate) fn get_text(path: &str, query: &[(&str, String)], ttl: i64) -> Result<String> {
    let Some(cache) = cache_dir().map(ResponseCache::new) else {
        return get_raw(path, query);
    };
    let key = cache::key(path, query);
    let now = crate::stats::now_seconds();
    if let Some(stored) = cache.read_fresh(&key, ttl, now) {
        return Ok(stored.body);
    }
    match get_raw(path, query) {
        Ok(body) => {
            let _ = cache.write(&key, &body, now);
            Ok(body)
        }
        Err(error) => match cache.read(&key) {
            Some(stored) => Ok(stored.body),
            None => Err(error),
        },
    }
}

/// The directory the desktop attaches to the stats client: `<data>/cache/stats`.
/// A machine that cannot resolve its data directory only loses the cache.
fn cache_dir() -> Option<PathBuf> {
    crate::subscription::data_dir()
        .ok()
        .map(|dir| dir.join("cache").join("stats"))
}

fn get_raw(path: &str, query: &[(&str, String)]) -> Result<String> {
    let url = format!("{}{}", STATS_API_BASE.trim_end_matches('/'), path);
    let agent = crate::net::builder()
        .middleware(crate::net::gate)
        .timeout_global(Some(Duration::from_secs(REQUEST_TIMEOUT_SECONDS)))
        .http_status_as_error(false)
        .build()
        .new_agent();
    let mut request = agent
        .get(&url)
        .header("User-Agent", STATS_USER_AGENT)
        .header("Accept", "application/json");

    if let Some(key) = read_api_key() {
        request = request.header("X-API-KEY", key);
    }

    for (name, value) in query {
        request = request.query(*name, value);
    }

    let mut response = request.call().map_err(|error| {
        Error::Network(format!(
            "Deadlock API request to {url} failed: {error}. Check the network connection and \
             retry."
        ))
    })?;

    let status = response.status().as_u16();
    let body = response.body_mut().read_to_string().map_err(|error| {
        Error::Network(format!(
            "Deadlock API response for {url} could not be read: {error}"
        ))
    })?;

    if !(200..300).contains(&status) {
        let hint = status_hint(status);
        return Err(Error::Network(format!(
            "Deadlock API returned HTTP {status} for {url}.{hint} {}",
            snippet(&body)
        )));
    }

    Ok(body)
}

/// The optional API key only raises the shared rate limits. It is sent as a
/// header and never written to the cache or an error.
fn read_api_key() -> Option<String> {
    std::env::var(STATS_API_KEY_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn status_hint(status: u16) -> String {
    match status {
        429 => {
            " Rate limited. The analytics endpoints allow 200 requests a minute per IP.".to_string()
        }
        503 => " The database queue is busy. Wait a moment and retry.".to_string(),
        504 => " The query hit the time limit. Narrow the window instead of retrying it unchanged."
            .to_string(),
        401 | 403 => format!(
            " The endpoint refused the request. If it needs a key, set {STATS_API_KEY_ENV}."
        ),
        404 => " The API has no record for this identifier.".to_string(),
        _ => String::new(),
    }
}

fn snippet(body: &str) -> String {
    let collapsed = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return "The response body was empty.".to_string();
    }

    let mut text: String = collapsed.chars().take(BODY_SNIPPET_CHARS).collect();
    if collapsed.chars().count() > BODY_SNIPPET_CHARS {
        text.push_str("...");
    }
    format!("Response starts with: {text}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_scope_is_both_modes_and_the_api_window() {
        let query = filter_query(&StatsFilter::default());
        assert_eq!(
            query,
            vec![
                ("game_mode", "normal".to_string()),
                ("match_mode", "ranked,unranked".to_string()),
            ]
        );
    }

    #[test]
    fn ranked_and_unranked_are_separate_mode_values() {
        let ranked = filter_query(&StatsFilter {
            ranked: Some(true),
            since: None,
        });
        assert_eq!(ranked[1], ("match_mode", "ranked".to_string()));

        let unranked = filter_query(&StatsFilter {
            ranked: Some(false),
            since: None,
        });
        assert_eq!(unranked[1], ("match_mode", "unranked".to_string()));
    }

    #[test]
    fn a_start_time_becomes_min_unix_timestamp() {
        let query = filter_query(&StatsFilter {
            ranked: Some(true),
            since: Some(1_700_000_000),
        });
        assert_eq!(query[2], ("min_unix_timestamp", "1700000000".to_string()));
    }

    #[test]
    fn a_zero_start_time_is_left_to_the_endpoint_default() {
        let query = filter_query(&StatsFilter {
            ranked: None,
            since: Some(0),
        });
        assert_eq!(query.len(), 2);
        assert!(!query.iter().any(|(name, _)| *name == "min_unix_timestamp"));
    }

    #[test]
    fn the_account_floor_rejects_zero() {
        assert!(ensure_account(1).is_ok());
        assert!(ensure_account(0).is_err());
    }
}
