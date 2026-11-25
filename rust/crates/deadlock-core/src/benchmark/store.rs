//! Benchmark results on disk and their exports. App data only:
//! `<data>/benchmarks/<run id>/run.json` and `pass-NN.txt` (one frame time in ms per line).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::compare::{Comparison, SideResult, compare};
use super::parse::TimedemoLine;
use super::session::FrameSource;
use super::system::{Machine, Settings};
use super::{CostSearch, RUN_SCHEMA, RunSpec, Side, Summary, Test, Variant};
use crate::error::{Error, Result};
use crate::util::write_atomic;

pub const DIR_NAME: &str = "benchmarks";

/// The metric definitions every export carries.
pub const DEFINITIONS: &[(&str, &str)] = &[
    ("avgFps", "frames divided by the sum of frame times"),
    ("low1", "1000 / mean of the slowest 1% of frame times"),
    ("low01", "1000 / mean of the slowest 0.1% of frame times, with 10000 frames or more"),
    ("p50Ms, p95Ms, p99Ms, p999Ms", "nearest-rank percentiles of frame time in ms"),
    ("stutters", "frames over twice the median of the 30 frames before them and at least 8 ms longer"),
    ("result", "the median of the measured passes; the warm-up pass is left out"),
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PassRecord {
    pub index: u32,
    pub variant: Variant,
    pub side: Side,
    pub warmup: bool,
    pub source: FrameSource,
    pub summary: Option<Summary>,
    pub timedemo: Option<TimedemoLine>,
    pub fps_cap: Option<f64>,
    pub started_at: String,
    pub wall_seconds: f64,
    pub error: Option<String>,
}

impl PassRecord {
    pub fn ok(&self) -> bool {
        self.error.is_none() && self.summary.as_ref().is_some_and(|s| s.avg_fps > 0.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "reason")]
pub enum Outcome {
    Complete,
    Stopped,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    pub schema: u32,
    pub spec: RunSpec,
    pub machine: Machine,
    pub settings: Settings,
    pub passes: Vec<PassRecord>,
    pub cost: Option<CostSearch>,
    pub outcome: Option<Outcome>,
    pub finished_at: Option<String>,
    /// The player's own name for the run ("before driver 591").
    #[serde(default)]
    pub note: String,
    /// Notes added after the run, oldest first. Appended, never rewritten.
    #[serde(default)]
    pub notes: Vec<RunNote>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunNote {
    pub at: String,
    pub text: String,
}

impl RunRecord {
    pub fn new(spec: RunSpec, machine: Machine, settings: Settings) -> Self {
        let cost = match &spec.test {
            Test::CostSearch { pool } => Some(CostSearch::new(pool)),
            _ => None,
        };
        RunRecord { schema: RUN_SCHEMA, spec, machine, settings, passes: Vec::new(), cost, outcome: None, finished_at: None, note: String::new(), notes: Vec::new() }
    }

    /// The measured, successful passes of one side.
    pub fn measured(&self, side: Side) -> Vec<&Summary> {
        self.passes
            .iter()
            .filter(|p| !p.warmup && p.side == side && p.ok())
            .filter_map(|p| p.summary.as_ref())
            .collect()
    }

    /// The result of a one-sided run, or the B side ("with mods", "mod on") of a comparison.
    /// A cost search reports its base passes.
    pub fn headline(&self) -> Option<SideResult> {
        let (side, label) = match self.spec.test.sides() {
            Some((_, b)) => (Side::B, b),
            None => (Side::A, "Result"),
        };
        let mut passes = self.measured(side);
        if self.cost.is_some() {
            passes = self
                .passes
                .iter()
                .filter(|p| !p.warmup && p.variant == Variant::AsIs && p.ok())
                .filter_map(|p| p.summary.as_ref())
                .collect();
        }
        (!passes.is_empty()).then(|| SideResult::from_passes(label, &passes))
    }

    /// The comparison of a two-sided run.
    pub fn comparison(&self) -> Option<Comparison> {
        let (a_label, b_label) = self.spec.test.sides()?;
        let (a, b) = (self.measured(Side::A), self.measured(Side::B));
        if a.is_empty() || b.is_empty() {
            return None;
        }
        Some(compare(SideResult::from_passes(a_label, &a), SideResult::from_passes(b_label, &b)))
    }

    /// Where the frames came from, when every measured pass agrees; else None.
    pub fn source(&self) -> Option<FrameSource> {
        let mut sources = self.passes.iter().filter(|p| !p.warmup && p.ok()).map(|p| p.source);
        let first = sources.next()?;
        sources.all(|s| s == first).then_some(first)
    }

    /// History groups runs by this: the replay and the game build.
    pub fn series_key(&self) -> String {
        let demo = self.spec.demo.as_ref().map(|d| d.label.as_str()).unwrap_or("play");
        format!("{demo}@{}", self.settings.build.as_deref().unwrap_or("?"))
    }
}

/// Two runs as a comparison: A is the older one.
pub fn compare_runs(a: &RunRecord, b: &RunRecord) -> Option<Comparison> {
    let (mut sa, mut sb) = (a.headline()?, b.headline()?);
    sa.label = run_label(a);
    sb.label = run_label(b);
    Some(compare(sa, sb))
}

/// A short name for a run: its note, else the test and the date.
pub fn run_label(run: &RunRecord) -> String {
    if !run.note.trim().is_empty() {
        return run.note.trim().to_string();
    }
    let day = run.spec.created_at.get(..10).unwrap_or("");
    format!("{} {day}", run.spec.test.label())
}

pub fn runs_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(DIR_NAME)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 40 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The run's folder: `run.json`, the frames and the logs (Open logs).
pub fn run_dir(data_dir: &Path, id: &str) -> Result<PathBuf> {
    if !valid_id(id) {
        return Err(Error::msg(format!("{id:?} is not a benchmark run id.")));
    }
    Ok(runs_dir(data_dir).join(id))
}

/// App data write.
pub fn save_run(data_dir: &Path, run: &RunRecord) -> Result<()> {
    let dir = run_dir(data_dir, &run.spec.id)?;
    fs::create_dir_all(&dir)?;
    let mut text = serde_json::to_string_pretty(run)?;
    text.push('\n');
    write_atomic(&dir.join("run.json"), text.as_bytes())
}

/// App data write: the frame times of one pass.
pub fn save_frames(data_dir: &Path, id: &str, pass: u32, frames: &[f64]) -> Result<()> {
    let dir = run_dir(data_dir, id)?;
    fs::create_dir_all(&dir)?;
    let mut text = String::with_capacity(frames.len() * 7);
    for ms in frames {
        text.push_str(&format!("{ms:.3}\n"));
    }
    write_atomic(&dir.join(format!("pass-{pass:02}.txt")), text.as_bytes())
}

pub fn load_frames(data_dir: &Path, id: &str, pass: u32) -> Vec<f64> {
    let Ok(dir) = run_dir(data_dir, id) else { return Vec::new() };
    fs::read_to_string(dir.join(format!("pass-{pass:02}.txt")))
        .map(|t| t.lines().filter_map(|l| l.trim().parse().ok()).collect())
        .unwrap_or_default()
}

/// Every stored run, newest first. A run that cannot be read is skipped.
pub fn list_runs(data_dir: &Path) -> Vec<RunRecord> {
    let Ok(entries) = fs::read_dir(runs_dir(data_dir)) else { return Vec::new() };
    let mut runs: Vec<RunRecord> = entries
        .flatten()
        .filter_map(|e| fs::read_to_string(e.path().join("run.json")).ok())
        // A demo kept in the data folder before its rename (`crate::app_data`) names the old one.
        .filter_map(|t| crate::app_data::parse_in_data::<RunRecord>(&t, data_dir).ok())
        .filter(|r| r.schema == RUN_SCHEMA)
        .collect();
    runs.sort_by(|a, b| b.spec.id.cmp(&a.spec.id));
    runs
}

/// App data write: append a note to a stored run. Earlier notes are kept as they are.
pub fn add_note(data_dir: &Path, id: &str, text: &str, at: &str) -> Result<RunRecord> {
    let text = text.trim();
    if text.is_empty() {
        return Err(Error::msg("A note needs some text."));
    }
    let path = run_dir(data_dir, id)?.join("run.json");
    let mut run: RunRecord = crate::app_data::parse_in_data(&fs::read_to_string(&path)?, data_dir)?;
    run.notes.push(RunNote { at: at.to_string(), text: text.chars().take(500).collect() });
    save_run(data_dir, &run)?;
    Ok(run)
}

/// App data write: a log file in the run's folder (`pass-02-console.log`, `pass-02.cfg`).
pub fn save_log(data_dir: &Path, id: &str, name: &str, bytes: &[u8]) -> Result<PathBuf> {
    if name.contains(['/', '\\']) || name.starts_with('.') {
        return Err(Error::msg(format!("{name:?} is not a log file name.")));
    }
    let dir = run_dir(data_dir, id)?;
    fs::create_dir_all(&dir)?;
    let path = dir.join(name);
    write_atomic(&path, bytes)?;
    Ok(path)
}

/// App data write: forget a run and its frames.
pub fn delete_run(data_dir: &Path, id: &str) -> Result<()> {
    let dir = run_dir(data_dir, id)?;
    match fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// `run.json` with the metric definitions, for sharing.
pub fn export_json(run: &RunRecord) -> Result<String> {
    let mut value = serde_json::to_value(run)?;
    if let Some(map) = value.as_object_mut() {
        let defs: serde_json::Map<String, serde_json::Value> =
            DEFINITIONS.iter().map(|(k, v)| (k.to_string(), serde_json::Value::String(v.to_string()))).collect();
        map.insert("definitions".into(), serde_json::Value::Object(defs));
        map.insert("generator".into(), serde_json::Value::String("Sigil benchmark".into()));
    }
    let mut text = serde_json::to_string_pretty(&value)?;
    text.push('\n');
    Ok(text)
}

fn variant_label(variant: &Variant) -> String {
    match variant {
        Variant::AsIs => "as-is".into(),
        Variant::Vanilla => "vanilla".into(),
        Variant::Without(slots) => {
            format!("without {}", slots.iter().map(|s| format!("pak{s:02}")).collect::<Vec<_>>().join(" "))
        }
    }
}

fn opt(v: Option<f64>) -> String {
    v.map(|v| format!("{v:.2}")).unwrap_or_default()
}

/// One row per pass.
pub fn export_summary_csv(run: &RunRecord) -> String {
    let mut out = String::from("run,pass,variant,side,warmup,source,frames,seconds,avg_fps,low_1,low_01,p50_ms,p95_ms,p99_ms,max_ms,stutters,error\n");
    for p in &run.passes {
        let s = p.summary.clone().unwrap_or_default();
        out.push_str(&format!(
            "{},{},{},{:?},{},{:?},{},{:.3},{:.2},{},{},{},{},{},{},{},{}\n",
            run.spec.id,
            p.index,
            variant_label(&p.variant),
            p.side,
            p.warmup,
            p.source,
            s.frames,
            s.seconds,
            s.avg_fps,
            opt(s.low_1),
            opt(s.low_01),
            opt(s.p50_ms),
            opt(s.p95_ms),
            opt(s.p99_ms),
            opt(s.max_ms),
            s.stutters.map(|n| n.to_string()).unwrap_or_default(),
            p.error.as_deref().unwrap_or("").replace([',', '\n'], " "),
        ));
    }
    out
}

/// One row per frame of every pass.
pub fn export_frames_csv(data_dir: &Path, run: &RunRecord) -> String {
    let mut out = String::from("run,pass,variant,warmup,frame,ms\n");
    for p in &run.passes {
        let variant = variant_label(&p.variant);
        for (i, ms) in load_frames(data_dir, &run.spec.id, p.index).iter().enumerate() {
            out.push_str(&format!("{},{},{},{},{},{ms:.3}\n", run.spec.id, p.index, variant, p.warmup, i + 1));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmark::{Length, metrics::summarize};

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dah-bench-store-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    pub(crate) fn run(id: &str, test: Test) -> RunRecord {
        let spec = RunSpec {
            id: id.into(),
            created_at: "2026-11-21T10:00:00Z".into(),
            test,
            demo: None,
            length: Length::Standard,
            window: None,
            passes: 2,
            warmup: true,
            display: crate::benchmark::Display::Game,
        };
        RunRecord::new(spec, Machine::default(), Settings { build: Some("6857".into()), ..Settings::default() })
    }

    fn pass(index: u32, variant: Variant, side: Side, warmup: bool, ms: f64) -> PassRecord {
        PassRecord {
            index,
            variant,
            side,
            warmup,
            source: FrameSource::TimedemoProfile,
            summary: Some(summarize(&vec![ms; 1000])),
            timedemo: None,
            fps_cap: Some(0.0),
            started_at: String::new(),
            wall_seconds: 60.0,
            error: None,
        }
    }

    #[test]
    fn a_comparison_leaves_the_warmup_out() {
        let mut r = run("b1", Test::ModsVsVanilla);
        r.passes = vec![
            pass(1, Variant::AsIs, Side::B, true, 10.0),
            pass(2, Variant::Vanilla, Side::A, false, 5.0),
            pass(3, Variant::AsIs, Side::B, false, 6.25),
            pass(4, Variant::Vanilla, Side::A, false, 5.0),
            pass(5, Variant::AsIs, Side::B, false, 6.25),
        ];
        let c = r.comparison().unwrap();
        assert!((c.a.summary.avg_fps - 200.0).abs() < 1e-6);
        assert!((c.b.summary.avg_fps - 160.0).abs() < 1e-6);
        assert_eq!(c.verdict, crate::benchmark::compare::Verdict::Slower);
        assert_eq!(r.headline().unwrap().label, "With mods");
        assert_eq!(r.source(), Some(FrameSource::TimedemoProfile));
        assert_eq!(r.series_key(), "play@6857");
    }

    #[test]
    fn runs_and_frames_round_trip() {
        let dir = temp("round");
        let mut r = run("b20261121-100000", Test::Measure);
        r.passes.push(pass(1, Variant::AsIs, Side::A, false, 5.0));
        save_run(&dir, &r).unwrap();
        save_frames(&dir, &r.spec.id, 1, &[5.0, 6.5]).unwrap();
        let older = run("b20261120-100000", Test::Measure);
        save_run(&dir, &older).unwrap();
        let runs = list_runs(&dir);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0], r, "newest first");
        assert_eq!(load_frames(&dir, &r.spec.id, 1), vec![5.0, 6.5]);
        let frames = export_frames_csv(&dir, &r);
        assert!(frames.contains("b20261121-100000,1,as-is,false,2,6.500"));
        let summary = export_summary_csv(&r);
        assert_eq!(summary.lines().count(), 2);
        assert!(export_json(&r).unwrap().contains("\"definitions\""));
        let noted = add_note(&dir, &r.spec.id, "  first  ", "2026-11-21T11:00:00Z").unwrap();
        let noted2 = add_note(&dir, &r.spec.id, "second", "2026-11-21T12:00:00Z").unwrap();
        assert_eq!(noted.notes.len(), 1);
        assert_eq!(noted2.notes.iter().map(|n| n.text.as_str()).collect::<Vec<_>>(), vec!["first", "second"]);
        assert!(add_note(&dir, &r.spec.id, "  ", "x").is_err());
        let log = save_log(&dir, &r.spec.id, "pass-01-console.log", b"hello").unwrap();
        assert_eq!(fs::read(&log).unwrap(), b"hello");
        assert!(save_log(&dir, &r.spec.id, "../x", b"").is_err());
        delete_run(&dir, &r.spec.id).unwrap();
        assert_eq!(list_runs(&dir).len(), 1);
        assert!(save_frames(&dir, "../x", 1, &[]).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_runs_compare_by_their_headlines() {
        let mut a = run("b1", Test::Measure);
        a.passes.push(pass(1, Variant::AsIs, Side::A, false, 5.0));
        let mut b = run("b2", Test::Measure);
        b.note = "New driver".into();
        b.passes.push(pass(1, Variant::AsIs, Side::A, false, 4.0));
        let c = compare_runs(&a, &b).unwrap();
        assert_eq!(c.b.label, "New driver");
        assert_eq!(c.verdict, crate::benchmark::compare::Verdict::Faster);
        assert!(c.low_confidence);
    }
}
