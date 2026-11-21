//! Where the player's recent matches were played, per relay location: the "Where I play" preset
//! and the match count beside each location on the Regions tab.
//!
//! deadlock-api keeps a per-account history of matches by month and datacenter
//! (`/v1/players/{id}/region`). It is read only for the primary Stats account and only while
//! Player and match stats is allowed (Settings > Privacy); the Regions page asks, this module
//! counts. The datacenter codes there are not Valve's relay ids: a partner-hosted site reads
//! `<site>-<host>` (`fra-dpkt`, `hel-hetz`), a numbered building `fra2`. [`location_of`] maps a
//! code onto the catalog. Studied from Deadlock Mod Manager's Server Picker (GPL-3.0, behaviour
//! only); the code and words here are Sigil's.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use serde_json::Value;

use super::Catalog;

/// Calendar months counted, the current one included.
pub const RECENT_MONTHS: u32 = 3;

/// One month of one datacenter in the history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionMonth {
    /// Unix seconds of the month's start (UTC), as the API gives it.
    pub month: i64,
    pub datacenter: Option<String>,
    pub matches: u64,
}

/// Read the API's answer. Rows are read one by one, so one odd month does not drop the rest; a
/// body that is not the expected object reads as no history.
pub fn parse(body: &str) -> Vec<RegionMonth> {
    #[derive(Deserialize)]
    struct Raw {
        #[serde(default)]
        history: Vec<Value>,
    }
    let Ok(raw) = serde_json::from_str::<Raw>(body) else {
        return Vec::new();
    };
    raw.history
        .iter()
        .filter_map(|row| {
            let month = row.get("month")?.as_i64()?;
            let matches = row.get("matches").and_then(Value::as_u64).unwrap_or(0);
            let datacenter = row
                .get("datacenter")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|code| !code.is_empty())
                .map(str::to_string);
            Some(RegionMonth { month, datacenter, matches })
        })
        .collect()
}

/// Partner hosts in deadlock-api's codes and the prefix Valve's catalog gives the same site.
const HOSTS: [(&str, &str); 2] = [("dpkt", "d"), ("hetz", "")];

/// The catalog location a datacenter code belongs to. A list of codes (`"fra2, ams"`) takes the
/// first one that maps. A numbered building falls back to its site unless the catalog lists the
/// numbered id itself (`sto2`).
pub fn location_of(code: &str, ids: &BTreeSet<String>) -> Option<String> {
    for part in code.split(',') {
        let part = part.trim().to_ascii_lowercase();
        if part.is_empty() {
            continue;
        }
        let (site, host) = match part.split_once('-') {
            Some((site, host)) => (site, Some(host)),
            None => (part.as_str(), None),
        };
        let id = match host {
            None => site.to_string(),
            Some(host) => {
                let prefix = HOSTS.iter().find(|(name, _)| *name == host).map_or("", |(_, prefix)| *prefix);
                format!("{prefix}{site}")
            }
        };
        if ids.contains(&id) {
            return Some(id);
        }
        let site_only = id.trim_end_matches(|c: char| c.is_ascii_digit());
        if site_only != id && ids.contains(site_only) {
            return Some(site_only.to_string());
        }
    }
    None
}

/// The first second (UTC) of the calendar month `back` months before the one holding `now`.
fn month_start(now: i64, back: u32) -> i64 {
    use chrono::{Datelike, TimeZone, Utc};
    let Some(at) = Utc.timestamp_opt(now, 0).single() else {
        return 0;
    };
    let total = at.year() * 12 + at.month0() as i32 - back as i32;
    let (year, month0) = (total.div_euclid(12), total.rem_euclid(12) as u32);
    Utc.with_ymd_and_hms(year, month0 + 1, 1, 0, 0, 0)
        .single()
        .map_or(0, |start| start.timestamp())
}

/// Matches per catalog location over the last [`RECENT_MONTHS`] calendar months.
pub fn matches_by_location(history: &[RegionMonth], catalog: &Catalog, now: i64) -> BTreeMap<String, u64> {
    let ids: BTreeSet<String> = catalog.locations.iter().map(|l| l.id.clone()).collect();
    let since = month_start(now, RECENT_MONTHS - 1);
    let mut counts = BTreeMap::new();
    for row in history.iter().filter(|row| row.month >= since && row.matches > 0) {
        let Some(code) = &row.datacenter else { continue };
        if let Some(id) = location_of(code, &ids) {
            *counts.entry(id).or_insert(0) += row.matches;
        }
    }
    counts
}

/// The "Where I play" preset: every location without a recent match is blocked. None when there
/// is no recent match at all (blocking everything would leave nowhere to play).
pub fn where_i_play(catalog: &Catalog, counts: &BTreeMap<String, u64>) -> Option<Vec<String>> {
    if counts.values().all(|n| *n == 0) {
        return None;
    }
    let mut blocked: Vec<String> = catalog
        .locations
        .iter()
        .filter(|l| counts.get(&l.id).copied().unwrap_or(0) == 0)
        .map(|l| l.id.clone())
        .collect();
    blocked.sort();
    Some(blocked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relays::{Location, Region};

    fn catalog(ids: &[&str]) -> Catalog {
        Catalog {
            revision: 1,
            fetched_at: 0,
            locations: ids
                .iter()
                .map(|id| Location {
                    id: id.to_string(),
                    description: String::new(),
                    city: id.to_string(),
                    country: None,
                    region: Region::Europe,
                    relays: Vec::new(),
                })
                .collect(),
        }
    }

    fn ids(list: &[&str]) -> BTreeSet<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn codes_map_onto_valve_ids() {
        let known = ids(&["fra", "dfra", "hel", "sto2", "sto", "iad"]);
        assert_eq!(location_of("fra", &known).as_deref(), Some("fra"));
        assert_eq!(location_of("fra2", &known).as_deref(), Some("fra"), "a building falls back to its site");
        assert_eq!(location_of("sto2", &known).as_deref(), Some("sto2"), "a listed numbered id stays");
        assert_eq!(location_of("fra-dpkt", &known).as_deref(), Some("dfra"));
        assert_eq!(location_of("HEL-hetz", &known).as_deref(), Some("hel"));
        assert_eq!(location_of("zzz, iad", &known).as_deref(), Some("iad"), "the first code that maps");
        assert_eq!(location_of("lhr", &known), None);
        assert_eq!(location_of("", &known), None);
    }

    #[test]
    fn the_answer_is_read_row_by_row() {
        let body = r#"{"history":[
            {"month":1761955200,"datacenter":"fra2","matches":12},
            {"month":"bad","datacenter":"ams","matches":3},
            {"month":1759276800,"datacenter":null,"matches":4},
            {"month":1756684800,"datacenter":"  ","matches":1}
        ]}"#;
        let rows = parse(body);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], RegionMonth { month: 1_761_955_200, datacenter: Some("fra2".into()), matches: 12 });
        assert_eq!(rows[1].datacenter, None);
        assert_eq!(rows[2].datacenter, None, "a blank code is no code");
        assert!(parse("not json").is_empty());
        assert!(parse("[]").is_empty());
    }

    #[test]
    fn only_the_last_three_calendar_months_count() {
        // 2026-11-21 12:00 UTC.
        let now = 1_795_262_400;
        let nov = month_start(now, 0);
        let sep = month_start(now, 2);
        let aug = month_start(now, 3);
        assert!(nov > sep && sep > aug);
        let history = vec![
            RegionMonth { month: nov, datacenter: Some("fra".into()), matches: 5 },
            RegionMonth { month: sep, datacenter: Some("fra2".into()), matches: 2 },
            RegionMonth { month: sep, datacenter: Some("hel-hetz".into()), matches: 1 },
            RegionMonth { month: aug, datacenter: Some("iad".into()), matches: 40 },
            RegionMonth { month: nov, datacenter: Some("lhr".into()), matches: 9 },
        ];
        let cat = catalog(&["fra", "hel", "iad", "sgp"]);
        let counts = matches_by_location(&history, &cat, now);
        assert_eq!(counts.get("fra"), Some(&7));
        assert_eq!(counts.get("hel"), Some(&1));
        assert_eq!(counts.get("iad"), None, "August is outside the window");
        let preset = where_i_play(&cat, &counts).unwrap();
        assert_eq!(preset, vec!["iad".to_string(), "sgp".to_string()]);
        assert_eq!(where_i_play(&cat, &BTreeMap::new()), None, "no history, no preset");
    }

    #[test]
    fn month_starts_cross_the_year() {
        // 2026-01-15 UTC, two months back is 2025-11-01.
        let now = 1_768_435_200;
        assert_eq!(month_start(now, 2), 1_761_955_200);
    }
}
