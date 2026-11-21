//! Two results side by side, and whether the gap is bigger than the noise.
//!
//! Noise is the larger of the two spreads of average FPS (highest pass minus lowest), never less
//! than [`NOISE_FLOOR_SHARE`] of the slower side. A gap inside the noise is "Same".

use serde::{Deserialize, Serialize};

use super::metrics::{Summary, median_summary, spread};

/// The smallest noise band, as a share of the slower side's average FPS.
pub const NOISE_FLOOR_SHARE: f64 = 0.015;

/// One side: the median of its measured passes and their spread.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SideResult {
    pub label: String,
    pub summary: Summary,
    pub spread: Option<(f64, f64)>,
    pub passes: usize,
}

impl SideResult {
    pub fn from_passes(label: &str, passes: &[&Summary]) -> Self {
        SideResult {
            label: label.to_string(),
            summary: median_summary(passes),
            spread: spread(passes),
            passes: passes.len(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    /// B is faster than A by more than the noise.
    Faster,
    Slower,
    /// Inside the noise.
    Same,
}

/// One metric of both sides.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricDelta {
    pub key: String,
    pub a: Option<f64>,
    pub b: Option<f64>,
    /// B minus A as a share of A.
    pub change: Option<f64>,
    pub higher_is_better: bool,
}

impl MetricDelta {
    /// B is better on this metric (by any amount).
    pub fn b_better(&self) -> Option<bool> {
        let (a, b) = (self.a?, self.b?);
        if (a - b).abs() < f64::EPSILON {
            return None;
        }
        Some((b > a) == self.higher_is_better)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    pub a: SideResult,
    pub b: SideResult,
    /// B minus A, average FPS.
    pub gap_fps: f64,
    /// B minus A as a share of A.
    pub gap_share: f64,
    pub noise_fps: f64,
    pub verdict: Verdict,
    /// A side has one pass: the noise is the floor, not measured.
    pub low_confidence: bool,
    pub metrics: Vec<MetricDelta>,
}

fn delta(key: &'static str, a: Option<f64>, b: Option<f64>, higher_is_better: bool) -> MetricDelta {
    let change = match (a, b) {
        (Some(a), Some(b)) if a.abs() > f64::EPSILON => Some((b - a) / a),
        _ => None,
    };
    MetricDelta { key: key.to_string(), a, b, change, higher_is_better }
}

/// The comparison of B against A.
pub fn compare(a: SideResult, b: SideResult) -> Comparison {
    let (fa, fb) = (a.summary.avg_fps, b.summary.avg_fps);
    let width = |s: &Option<(f64, f64)>| s.map(|(lo, hi)| hi - lo).unwrap_or(0.0);
    let noise = width(&a.spread).max(width(&b.spread)).max(fa.min(fb) * NOISE_FLOOR_SHARE);
    let gap = fb - fa;
    let verdict = if gap.abs() <= noise {
        Verdict::Same
    } else if gap > 0.0 {
        Verdict::Faster
    } else {
        Verdict::Slower
    };
    let (sa, sb) = (&a.summary, &b.summary);
    let metrics = vec![
        delta("avgFps", Some(fa), Some(fb), true),
        delta("low1", sa.low_1, sb.low_1, true),
        delta("low01", sa.low_01, sb.low_01, true),
        delta("p50", sa.p50_ms, sb.p50_ms, false),
        delta("p95", sa.p95_ms, sb.p95_ms, false),
        delta("p99", sa.p99_ms, sb.p99_ms, false),
        delta("stutters", sa.stutters.map(|n| n as f64), sb.stutters.map(|n| n as f64), false),
    ];
    Comparison {
        gap_fps: gap,
        gap_share: if fa > 0.0 { gap / fa } else { 0.0 },
        noise_fps: noise,
        verdict,
        low_confidence: a.passes < 2 || b.passes < 2,
        metrics,
        a,
        b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn side(label: &str, fps: &[f64]) -> SideResult {
        let summaries: Vec<Summary> = fps
            .iter()
            .map(|f| Summary { avg_fps: *f, low_1: Some(f * 0.6), p99_ms: Some(1500.0 / f), ..Summary::default() })
            .collect();
        let refs: Vec<&Summary> = summaries.iter().collect();
        SideResult::from_passes(label, &refs)
    }

    #[test]
    fn a_gap_inside_the_spread_is_the_same() {
        let c = compare(side("A", &[200.0, 204.0, 198.0]), side("B", &[201.0, 205.0, 199.0]));
        assert_eq!(c.verdict, Verdict::Same);
        assert!((c.noise_fps - 6.0).abs() < 1e-9);
        assert!(!c.low_confidence);
    }

    #[test]
    fn a_clear_gap_is_faster_or_slower() {
        let c = compare(side("Vanilla", &[200.0, 201.0, 199.0]), side("Mods", &[180.0, 181.0, 179.0]));
        assert_eq!(c.verdict, Verdict::Slower);
        assert!((c.gap_fps + 20.0).abs() < 1e-9);
        assert!((c.gap_share + 0.1).abs() < 1e-9);
        let low = c.metrics.iter().find(|m| m.key == "low1").unwrap();
        assert_eq!(low.b_better(), Some(false));
        let p99 = c.metrics.iter().find(|m| m.key == "p99").unwrap();
        assert_eq!(p99.b_better(), Some(false), "a longer p99 frame is worse");
        let back = compare(side("Mods", &[180.0, 181.0]), side("Vanilla", &[200.0, 201.0]));
        assert_eq!(back.verdict, Verdict::Faster);
    }

    #[test]
    fn one_pass_uses_the_floor_and_says_so() {
        let c = compare(side("A", &[100.0]), side("B", &[101.0]));
        assert!(c.low_confidence);
        assert!((c.noise_fps - 1.5).abs() < 1e-9);
        assert_eq!(c.verdict, Verdict::Same);
        let c = compare(side("A", &[100.0]), side("B", &[103.0]));
        assert_eq!(c.verdict, Verdict::Faster);
    }
}
