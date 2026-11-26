//! Where a player's averages sit among the matches played at their rank.
//!
//! Source: `GET /v1/analytics/player-stats/metrics?min_average_badge=&max_average_badge=`, one
//! request per rank tier, kept 24 hours. The answer describes single matches (each player in each
//! match is one sample): an average, a spread and the 1st to 99th percentiles of each metric. A
//! player's average placed in it reads "beats N% of matches at your rank", not "of players".

use std::collections::HashMap;

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::stats::filters::get_text;

/// How long a tier's distribution is kept.
pub const BRACKET_TTL: i64 = 24 * 3_600;

/// One metric across the tier's matches.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct Distribution {
    #[serde(default)]
    pub avg: f64,
    #[serde(default)]
    pub percentile1: f64,
    #[serde(default)]
    pub percentile5: f64,
    #[serde(default)]
    pub percentile10: f64,
    #[serde(default)]
    pub percentile25: f64,
    #[serde(default)]
    pub percentile50: f64,
    #[serde(default)]
    pub percentile75: f64,
    #[serde(default)]
    pub percentile90: f64,
    #[serde(default)]
    pub percentile95: f64,
    #[serde(default)]
    pub percentile99: f64,
}

impl Distribution {
    /// The known points, (percentile, value), lowest first.
    fn points(&self) -> [(f64, f64); 9] {
        [
            (1.0, self.percentile1),
            (5.0, self.percentile5),
            (10.0, self.percentile10),
            (25.0, self.percentile25),
            (50.0, self.percentile50),
            (75.0, self.percentile75),
            (90.0, self.percentile90),
            (95.0, self.percentile95),
            (99.0, self.percentile99),
        ]
    }

    /// The share of matches with a value at or below `value`, 0 to 100, read between the known
    /// percentiles. Flat stretches (many matches with the same count, such as 0 denies) give the
    /// middle of the stretch, so a common value is not ranked at its top.
    pub fn percentile_of(&self, value: f64) -> f64 {
        let points = self.points();
        if value < points[0].1 {
            return 0.5;
        }
        if value > points[8].1 {
            return 99.5;
        }
        // Every known point holding exactly this value: the middle of that run.
        let equal: Vec<f64> = points.iter().filter(|(_, v)| (*v - value).abs() < 1e-9).map(|(p, _)| *p).collect();
        if !equal.is_empty() {
            return (equal[0] + equal[equal.len() - 1]) / 2.0;
        }
        for pair in points.windows(2) {
            let ((p0, v0), (p1, v1)) = (pair[0], pair[1]);
            if value >= v0 && value <= v1 && v1 > v0 {
                return p0 + (value - v0) / (v1 - v0) * (p1 - p0);
            }
        }
        50.0
    }
}

/// Parse a metrics answer into metric name to distribution. Entries that are not a
/// distribution are skipped.
pub fn parse_metrics(json: &str) -> Result<HashMap<String, Distribution>> {
    let raw: HashMap<String, serde_json::Value> = serde_json::from_str(json)
        .map_err(|error| Error::msg(format!("The rank metrics response did not parse: {error}")))?;
    Ok(raw
        .into_iter()
        .filter_map(|(name, value)| serde_json::from_value::<Distribution>(value).ok().map(|d| (name, d)))
        .collect())
}

/// The distributions for the tier holding `badge` (all six subranks). `None` for an unranked
/// badge.
pub fn tier_metrics(badge: u32) -> Result<Option<HashMap<String, Distribution>>> {
    if badge < 10 {
        return Ok(None);
    }
    let tier = badge / 10;
    let query = [("min_average_badge", (tier * 10 + 1).to_string()), ("max_average_badge", (tier * 10 + 6).to_string())];
    parse_metrics(&get_text("/v1/analytics/player-stats/metrics", &query, BRACKET_TTL)?).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KILLS: &str = r#"{"kills": {"avg": 6.3, "std": 4.0, "percentile1": 0.0, "percentile5": 1.0,
        "percentile10": 2.0, "percentile25": 3.0, "percentile50": 6.0, "percentile75": 9.0,
        "percentile90": 12.0, "percentile95": 14.0, "percentile99": 18.0},
        "denies": {"avg": 2.6, "percentile1": 0.0, "percentile5": 0.0, "percentile10": 0.0,
        "percentile25": 1.0, "percentile50": 2.0, "percentile75": 4.0, "percentile90": 6.0,
        "percentile95": 8.0, "percentile99": 11.0},
        "note": "not a distribution"}"#;

    #[test]
    fn percentiles_read_between_the_known_points() {
        let m = parse_metrics(KILLS).unwrap();
        assert_eq!(m.len(), 2, "the note is skipped");
        let kills = m["kills"];
        assert!((kills.percentile_of(6.0) - 50.0).abs() < 1e-9);
        assert!((kills.percentile_of(7.5) - 62.5).abs() < 1e-9);
        assert!((kills.percentile_of(20.0) - 99.5).abs() < 1e-9);
        let denies = m["denies"];
        assert!((denies.percentile_of(0.0) - 5.5).abs() < 1e-9, "a flat run gives its middle");
        assert!((denies.percentile_of(3.0) - 62.5).abs() < 1e-9);
        assert!(parse_metrics("[").is_err());
    }

    #[test]
    fn an_unranked_badge_asks_for_nothing() {
        assert!(tier_metrics(0).unwrap().is_none());
    }
}
