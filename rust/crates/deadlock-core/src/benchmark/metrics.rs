//! Frame time statistics. One list of frame times in milliseconds in, one [`Summary`] out.
//!
//! Definitions (also in `docs/design/benchmark.md` and every export):
//!
//! - Average FPS is frames over total time, never the mean of per-frame FPS.
//! - A low is 1000 over the mean of the slowest share of frames (the "average of the worst 1%"
//!   reading, as CapFrameX and Grimoire's benchmark WIP compute it).
//! - Percentiles are nearest-rank.
//! - A stutter is a frame longer than twice the median of the [`STUTTER_WINDOW`] frames before it
//!   and at least [`STUTTER_MIN_EXTRA_MS`] longer than that median.

use serde::{Deserialize, Serialize};

/// Frames a stutter is judged against.
pub const STUTTER_WINDOW: usize = 30;
/// How much longer than the local median a stutter frame must be, in ms.
pub const STUTTER_MIN_EXTRA_MS: f64 = 8.0;
/// Bins in the frame time graph.
pub const GRAPH_BINS: usize = 600;
/// Below this many frames a 0.1% low is a single frame and is not reported.
pub const POINT_ONE_MIN_FRAMES: usize = 10_000;

/// The numbers a pass or a run reports.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub frames: u64,
    pub seconds: f64,
    pub avg_fps: f64,
    /// None when only the game's summary line was read.
    pub low_1: Option<f64>,
    pub low_01: Option<f64>,
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub p99_ms: Option<f64>,
    pub p999_ms: Option<f64>,
    pub max_ms: Option<f64>,
    pub stutters: Option<u64>,
    /// Share of the run time spent in stutter frames, 0 to 1.
    pub stutter_share: Option<f64>,
    pub stdev_ms: Option<f64>,
}

/// One bin of the frame time graph.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bin {
    /// Seconds from the start of the pass to the bin's start.
    pub at: f64,
    pub fps: f64,
    pub worst_ms: f64,
}

fn clean(frames: &[f64]) -> Vec<f64> {
    frames
        .iter()
        .copied()
        .filter(|ms| ms.is_finite() && *ms > 0.0 && *ms < 10_000.0)
        .collect()
}

/// Nearest-rank percentile of sorted values, `q` from 0 to 1.
pub fn percentile(sorted: &[f64], q: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (q * sorted.len() as f64).ceil() as usize;
    Some(sorted[rank.clamp(1, sorted.len()) - 1])
}

/// 1000 over the mean of the slowest `share` of frames, at least one frame.
pub fn low(sorted: &[f64], share: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let count = ((sorted.len() as f64) * share).ceil().max(1.0) as usize;
    let worst = &sorted[sorted.len() - count.min(sorted.len())..];
    let mean = worst.iter().sum::<f64>() / worst.len() as f64;
    (mean > 0.0).then(|| 1000.0 / mean)
}

fn median_of(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    let n = values.len();
    if n % 2 == 1 {
        values[n / 2]
    } else {
        (values[n / 2 - 1] + values[n / 2]) / 2.0
    }
}

/// Stutter frames: (count, time spent in them).
pub fn stutters(frames: &[f64]) -> (u64, f64) {
    let mut count = 0;
    let mut time = 0.0;
    let mut window: Vec<f64> = Vec::with_capacity(STUTTER_WINDOW);
    for (index, ms) in frames.iter().enumerate() {
        if index >= STUTTER_WINDOW {
            window.clear();
            window.extend_from_slice(&frames[index - STUTTER_WINDOW..index]);
            let median = median_of(&mut window);
            if *ms > median * 2.0 && *ms - median >= STUTTER_MIN_EXTRA_MS {
                count += 1;
                time += ms;
            }
        }
    }
    (count, time)
}

/// Every number for one list of frame times. Bad values (zero, negative, not finite, over ten
/// seconds) are dropped first.
pub fn summarize(frames: &[f64]) -> Summary {
    let frames = clean(frames);
    if frames.is_empty() {
        return Summary::default();
    }
    let total: f64 = frames.iter().sum();
    let mut sorted = frames.clone();
    sorted.sort_by(f64::total_cmp);
    let n = frames.len();
    let mean = total / n as f64;
    let variance = frames.iter().map(|ms| (ms - mean).powi(2)).sum::<f64>() / n as f64;
    let (stutter_count, stutter_time) = stutters(&frames);
    Summary {
        frames: n as u64,
        seconds: total / 1000.0,
        avg_fps: if total > 0.0 { n as f64 * 1000.0 / total } else { 0.0 },
        low_1: low(&sorted, 0.01),
        low_01: (n >= POINT_ONE_MIN_FRAMES).then(|| low(&sorted, 0.001)).flatten(),
        p50_ms: percentile(&sorted, 0.50),
        p95_ms: percentile(&sorted, 0.95),
        p99_ms: percentile(&sorted, 0.99),
        p999_ms: (n >= POINT_ONE_MIN_FRAMES).then(|| percentile(&sorted, 0.999)).flatten(),
        max_ms: sorted.last().copied(),
        stutters: Some(stutter_count),
        stutter_share: Some(if total > 0.0 { stutter_time / total } else { 0.0 }),
        stdev_ms: Some(variance.sqrt()),
    }
}

/// The frame time graph: the pass cut into at most `bins` bins of equal time.
pub fn graph(frames: &[f64], bins: usize) -> Vec<Bin> {
    let frames = clean(frames);
    let total: f64 = frames.iter().sum();
    if frames.is_empty() || bins == 0 || total <= 0.0 {
        return Vec::new();
    }
    let width = (total / bins as f64).max(1e-9);
    let mut out: Vec<Bin> = Vec::with_capacity(bins);
    let (mut start, mut count, mut sum, mut worst, mut clock) = (0.0, 0u32, 0.0, 0.0f64, 0.0);
    for ms in frames {
        count += 1;
        sum += ms;
        worst = worst.max(ms);
        clock += ms;
        if clock - start >= width && out.len() + 1 < bins {
            out.push(Bin { at: start / 1000.0, fps: count as f64 * 1000.0 / sum, worst_ms: worst });
            start = clock;
            count = 0;
            sum = 0.0;
            worst = 0.0;
        }
    }
    if count > 0 {
        out.push(Bin { at: start / 1000.0, fps: count as f64 * 1000.0 / sum, worst_ms: worst });
    }
    out
}

/// The median of each metric across passes. Metrics a pass lacks are left out of that median;
/// a metric no pass has stays None.
pub fn median_summary(passes: &[&Summary]) -> Summary {
    fn med(values: Vec<f64>) -> Option<f64> {
        if values.is_empty() {
            return None;
        }
        let mut values = values;
        Some(median_of(&mut values))
    }
    let pick = |f: &dyn Fn(&Summary) -> Option<f64>| med(passes.iter().filter_map(|s| f(s)).collect());
    Summary {
        frames: med(passes.iter().map(|s| s.frames as f64).collect()).unwrap_or(0.0).round() as u64,
        seconds: pick(&|s| Some(s.seconds)).unwrap_or(0.0),
        avg_fps: pick(&|s| Some(s.avg_fps)).unwrap_or(0.0),
        low_1: pick(&|s| s.low_1),
        low_01: pick(&|s| s.low_01),
        p50_ms: pick(&|s| s.p50_ms),
        p95_ms: pick(&|s| s.p95_ms),
        p99_ms: pick(&|s| s.p99_ms),
        p999_ms: pick(&|s| s.p999_ms),
        max_ms: pick(&|s| s.max_ms),
        stutters: pick(&|s| s.stutters.map(|n| n as f64)).map(|n| n.round() as u64),
        stutter_share: pick(&|s| s.stutter_share),
        stdev_ms: pick(&|s| s.stdev_ms),
    }
}

/// Lowest and highest average FPS across passes.
pub fn spread(passes: &[&Summary]) -> Option<(f64, f64)> {
    let mut values = passes.iter().map(|s| s.avg_fps).filter(|v| *v > 0.0);
    let first = values.next()?;
    Some(values.fold((first, first), |(lo, hi), v| (lo.min(v), hi.max(v))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn average_fps_is_frames_over_time_not_mean_fps() {
        // 1 frame of 100 ms and 9 of 10 ms: 10 frames in 190 ms.
        let mut frames = vec![10.0; 9];
        frames.push(100.0);
        let s = summarize(&frames);
        assert_eq!(s.frames, 10);
        assert!((s.avg_fps - 10.0 * 1000.0 / 190.0).abs() < 1e-9);
        assert!((s.seconds - 0.19).abs() < 1e-9);
    }

    #[test]
    fn lows_average_the_slowest_share() {
        // 1000 frames: 990 at 5 ms, 10 at 20 ms. The slowest 1% is the 10 at 20 ms.
        let mut frames = vec![5.0; 990];
        frames.extend(vec![20.0; 10]);
        let s = summarize(&frames);
        assert!((s.low_1.unwrap() - 50.0).abs() < 1e-9);
        assert_eq!(s.low_01, None, "too few frames for a 0.1% low");
        assert_eq!(s.p50_ms, Some(5.0));
        assert_eq!(s.p99_ms, Some(5.0));
        assert_eq!(s.max_ms, Some(20.0));
    }

    #[test]
    fn point_one_low_needs_ten_thousand_frames() {
        let mut frames = vec![4.0; 9_990];
        frames.extend(vec![40.0; 10]);
        let s = summarize(&frames);
        assert!((s.low_01.unwrap() - 25.0).abs() < 1e-9);
        assert_eq!(s.p999_ms, Some(4.0));
    }

    #[test]
    fn percentiles_are_nearest_rank() {
        let sorted: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(percentile(&sorted, 0.95), Some(95.0));
        assert_eq!(percentile(&sorted, 0.0), Some(1.0));
        assert_eq!(percentile(&sorted, 1.0), Some(100.0));
        assert_eq!(percentile(&[], 0.5), None);
    }

    #[test]
    fn a_stutter_stands_out_from_its_neighbours() {
        let mut frames = vec![6.0; 100];
        frames[50] = 30.0; // 5x the median and 24 ms longer
        frames[70] = 11.0; // under 2x
        frames[10] = 50.0; // inside the first window: not judged
        let (count, time) = stutters(&frames);
        assert_eq!(count, 1);
        assert_eq!(time, 30.0);
        // A slow but steady section is not stuttering.
        assert_eq!(stutters(&[40.0; 200]).0, 0);
        // At high frame rates a doubling under 8 ms is not a stutter.
        let mut fast = vec![2.0; 100];
        fast[60] = 6.0;
        assert_eq!(stutters(&fast).0, 0);
    }

    #[test]
    fn bad_values_are_dropped() {
        let s = summarize(&[f64::NAN, -1.0, 0.0, 10.0, 10.0, f64::INFINITY, 50_000.0]);
        assert_eq!(s.frames, 2);
        assert_eq!(summarize(&[]), Summary::default());
    }

    #[test]
    fn the_graph_has_at_most_the_bins_asked_for() {
        let frames: Vec<f64> = (0..10_000).map(|i| if i % 100 == 0 { 30.0 } else { 5.0 }).collect();
        let bins = graph(&frames, 600);
        assert!(bins.len() <= 600 && bins.len() > 500, "{}", bins.len());
        assert!(bins.iter().all(|b| b.fps > 0.0 && b.worst_ms >= 5.0));
        assert!(bins.windows(2).all(|w| w[1].at > w[0].at));
        assert_eq!(graph(&[], 10), Vec::new());
        assert_eq!(graph(&[5.0, 5.0], 600).len(), 2);
    }

    #[test]
    fn the_median_of_passes_skips_missing_metrics() {
        let a = Summary { avg_fps: 100.0, low_1: Some(60.0), ..Summary::default() };
        let b = Summary { avg_fps: 110.0, low_1: None, ..Summary::default() };
        let c = Summary { avg_fps: 90.0, low_1: Some(70.0), ..Summary::default() };
        let m = median_summary(&[&a, &b, &c]);
        assert_eq!(m.avg_fps, 100.0);
        assert_eq!(m.low_1, Some(65.0));
        assert_eq!(m.p99_ms, None);
        assert_eq!(spread(&[&a, &b, &c]), Some((90.0, 110.0)));
        assert_eq!(spread(&[]), None);
    }
}
