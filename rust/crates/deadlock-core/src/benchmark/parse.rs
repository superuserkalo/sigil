//! What the game writes during a pass: `console.log` lines and the two per-frame CSV files.
//!
//! The patterns are the engine's and client's own format strings from the build 6857 string
//! tables (`GameTracking-Deadlock`, `engine2_strings.txt` and `client_strings.txt`), so a line is
//! matched by what the game prints, not by a guess. A console line may carry a channel or time
//! prefix, so the patterns are not anchored to the start of the line.

use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// `%i frames %5.3f seconds %5.2f fps (%5.2f ms/f) %5.3f fps variability, P5/50/95 frame time
/// %.1f/%.1f/%.1f ms`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimedemoLine {
    pub frames: u64,
    pub seconds: f64,
    pub fps: f64,
    pub ms_per_frame: f64,
    pub variability: f64,
    pub p5_ms: Option<f64>,
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
}

/// One thing the log said that a pass cares about.
#[derive(Debug, Clone, PartialEq)]
pub enum LogEvent {
    /// The run's own `echo`: the cfg ran. (run id, pass number)
    Marker(String, u32),
    /// `Requesting timedemo of %s`.
    TimedemoRequested(String),
    Timedemo(TimedemoLine),
    /// `Demo playback finished ( %.1f seconds, %i render frames, %.2f fps).`
    PlaybackFinished { seconds: f64, frames: u64, fps: f64 },
    /// `FPS: Avg=%.1f, P1=%.1f`.
    AvgP1 { avg: f64, p1: f64 },
    /// `fps_max` printed with no value: the frame cap. 0 is none.
    FpsCap(f64),
    /// `Avg FPS %3i, Frame MS %5.1f, Frame Server MS %5.1f log: %s`: where `cl_showfps 4` wrote.
    ShowFpsLog(String),
    /// The engine refused or could not read the replay.
    DemoError(String),
}

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("benchmark pattern"))
}

fn num(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

/// The timedemo result in a line, if it holds one.
pub fn timedemo_line(line: &str) -> Option<TimedemoLine> {
    static CELL: OnceLock<Regex> = OnceLock::new();
    let caps = re(
        &CELL,
        r"(\d+) frames\s+([\d.]+) seconds\s+([\d.]+) fps\s*\(\s*([\d.]+) ms/f\)\s+([\d.]+) fps variability(?:,\s*P5/50/95 frame time\s*([\d.]+)/([\d.]+)/([\d.]+) ms)?",
    )
    .captures(line)?;
    let opt = |i: usize| caps.get(i).and_then(|m| num(m.as_str()));
    Some(TimedemoLine {
        frames: caps[1].parse().ok()?,
        seconds: num(&caps[2])?,
        fps: num(&caps[3])?,
        ms_per_frame: num(&caps[4])?,
        variability: num(&caps[5])?,
        p5_ms: opt(6),
        p50_ms: opt(7),
        p95_ms: opt(8),
    })
}

const DEMO_ERRORS: &[&str] = &[
    "can't playdemo",
    "demo recorded for game",
    "addons, currently have",
    "demo has invalid",
    "pre-dates demo versioning",
    "couldn't open demo",
    "failed to open demo",
    "unable to open demo",
    "is not a valid demo",
];

/// What one console line says, if anything a pass cares about.
pub fn scan_line(line: &str) -> Option<LogEvent> {
    static MARKER: OnceLock<Regex> = OnceLock::new();
    static REQUEST: OnceLock<Regex> = OnceLock::new();
    static FINISHED: OnceLock<Regex> = OnceLock::new();
    static AVG_P1: OnceLock<Regex> = OnceLock::new();
    static CAP: OnceLock<Regex> = OnceLock::new();
    static SHOWFPS: OnceLock<Regex> = OnceLock::new();
    if let Some(caps) = re(&MARKER, r"SIGIL_BENCHMARK ([A-Za-z0-9_-]+) (\d+)").captures(line) {
        return Some(LogEvent::Marker(caps[1].to_string(), caps[2].parse().ok()?));
    }
    if let Some(found) = timedemo_line(line) {
        return Some(LogEvent::Timedemo(found));
    }
    if let Some(caps) = re(&REQUEST, r"Requesting timedemo of (\S+)").captures(line) {
        return Some(LogEvent::TimedemoRequested(caps[1].to_string()));
    }
    if let Some(caps) = re(
        &FINISHED,
        r"Demo playback finished \(\s*([\d.]+) seconds, (\d+) render frames, ([\d.]+) fps\)",
    )
    .captures(line)
    {
        return Some(LogEvent::PlaybackFinished {
            seconds: num(&caps[1])?,
            frames: caps[2].parse().ok()?,
            fps: num(&caps[3])?,
        });
    }
    if let Some(caps) = re(&AVG_P1, r"FPS: Avg=([\d.]+), P1=([\d.]+)").captures(line) {
        return Some(LogEvent::AvgP1 { avg: num(&caps[1])?, p1: num(&caps[2])? });
    }
    if let Some(caps) = re(&CAP, r#"(?:^|\]\s*|\s)fps_max\s*=\s*"?([\d.]+)"#).captures(line) {
        return Some(LogEvent::FpsCap(num(&caps[1])?));
    }
    if let Some(caps) = re(
        &SHOWFPS,
        r"Avg FPS\s*\d+, Frame MS\s*[\d.]+, Frame Server MS\s*[\d.]+ log: (\S.*?)\s*$",
    )
    .captures(line)
    {
        return Some(LogEvent::ShowFpsLog(caps[1].to_string()));
    }
    let lower = line.to_ascii_lowercase();
    if DEMO_ERRORS.iter().any(|needle| lower.contains(needle)) {
        return Some(LogEvent::DemoError(line.trim().to_string()));
    }
    None
}

fn split_csv(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    for c in line.chars() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => fields.push(std::mem::take(&mut field)),
            _ => field.push(c),
        }
    }
    fields.push(field);
    fields.into_iter().map(|f| f.trim().to_string()).collect()
}

/// Frame times in ms from `timedemo_profile.csv`
/// (`demo tick,frame start time,frame start delta,frame swap time,frame swap delta,FPS
/// variability`). The `frame start delta` column; `frame swap delta` when the start column is
/// empty. Seconds are turned into ms when every value is that small.
pub fn timedemo_profile(text: &str) -> Vec<f64> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let Some(header) = lines.by_ref().find(|l| l.to_ascii_lowercase().contains("frame start delta")) else {
        return Vec::new();
    };
    let names: Vec<String> = split_csv(header).into_iter().map(|n| n.to_ascii_lowercase()).collect();
    let start = names.iter().position(|n| n == "frame start delta");
    let swap = names.iter().position(|n| n == "frame swap delta");
    let mut values: Vec<f64> = lines
        .filter_map(|line| {
            let fields = split_csv(line);
            let pick = |i: Option<usize>| i.and_then(|i| fields.get(i)).and_then(|f| num(f));
            pick(start).or_else(|| pick(swap))
        })
        .filter(|v| *v > 0.0)
        .collect();
    if let Some(max) = values.iter().copied().reduce(f64::max)
        && max < 0.5
    {
        // Seconds: a frame of 500 ms or more would show as 0.5.
        values.iter_mut().for_each(|v| *v *= 1000.0);
    }
    values
}

/// Frame times in ms from a `cl_showfps 4` log (`Time,Player 1 Position,Smooth FPS,Frame FPS,
/// Smooth MS,Frame MS,Server Frame MS`). A position with commas in it makes a row longer than
/// the header, so the column is counted from the end.
pub fn showfps_log(text: &str) -> Vec<f64> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let Some(header) = lines.by_ref().find(|l| l.to_ascii_lowercase().contains("frame ms")) else {
        return Vec::new();
    };
    let names: Vec<String> = split_csv(header).into_iter().map(|n| n.to_ascii_lowercase()).collect();
    let Some(at) = names.iter().position(|n| n == "frame ms") else {
        return Vec::new();
    };
    let from_end = names.len() - 1 - at;
    lines
        .filter_map(|line| {
            let fields: Vec<&str> = line.split(',').collect();
            let index = fields.len().checked_sub(from_end + 1)?;
            num(fields[index])
        })
        .filter(|v| *v > 0.0)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_timedemo_line_is_read_from_the_engine_format() {
        // `%i frames %5.3f seconds %5.2f fps (%5.2f ms/f) %5.3f fps variability, P5/50/95 ...`
        let line = "5760 frames 31.204 seconds 184.59 fps ( 5.42 ms/f) 1.284 fps variability, P5/50/95 frame time 4.1/5.2/7.9 ms";
        let found = timedemo_line(line).unwrap();
        assert_eq!(found.frames, 5760);
        assert_eq!(found.fps, 184.59);
        assert_eq!(found.ms_per_frame, 5.42);
        assert_eq!((found.p5_ms, found.p50_ms, found.p95_ms), (Some(4.1), Some(5.2), Some(7.9)));
        let older = timedemo_line("[Client] 100 frames  2.000 seconds 50.00 fps (20.00 ms/f) 0.500 fps variability").unwrap();
        assert_eq!(older.p95_ms, None);
        assert!(timedemo_line("loaded 100 frames of animation").is_none());
    }

    #[test]
    fn console_lines_become_events() {
        assert_eq!(scan_line("SIGIL_BENCHMARK b1x2 3"), Some(LogEvent::Marker("b1x2".into(), 3)));
        assert_eq!(
            scan_line("Requesting timedemo of replays/sigil_benchmark"),
            Some(LogEvent::TimedemoRequested("replays/sigil_benchmark".into()))
        );
        assert_eq!(
            scan_line("Demo playback finished ( 90.0 seconds, 16612 render frames, 184.58 fps)."),
            Some(LogEvent::PlaybackFinished { seconds: 90.0, frames: 16612, fps: 184.58 })
        );
        assert_eq!(scan_line("FPS: Avg=184.6, P1=101.2"), Some(LogEvent::AvgP1 { avg: 184.6, p1: 101.2 }));
        assert_eq!(scan_line("fps_max = 240 ( def. \"120\" ) archive"), Some(LogEvent::FpsCap(240.0)));
        assert_eq!(scan_line("[Console] fps_max = 0"), Some(LogEvent::FpsCap(0.0)));
        assert_eq!(scan_line("fps_max_ui = 60"), None);
        assert_eq!(
            scan_line("Avg FPS 184, Frame MS   5.4, Frame Server MS   0.0 log: prof_dl_midtown.csv"),
            Some(LogEvent::ShowFpsLog("prof_dl_midtown.csv".into()))
        );
        assert!(matches!(scan_line("can't playdemo 'replays/x', disallowed"), Some(LogEvent::DemoError(_))));
        assert!(matches!(
            scan_line("demo recorded with '' addons, currently have 'x' addons"),
            Some(LogEvent::DemoError(_))
        ));
        assert_eq!(scan_line("Host_NewGame on map dl_midtown"), None);
    }

    #[test]
    fn the_timedemo_profile_reads_frame_start_deltas() {
        let csv = "demo tick,frame start time,frame start delta,frame swap time,frame swap delta,FPS variability\n\n\
                   100,1.000,5.5,1.004,5.4,0.1\n101,1.0055,6.5,1.010,6.6,0.1\n102,1.012,,1.017,7.0,0.1\n";
        assert_eq!(timedemo_profile(csv), vec![5.5, 6.5, 7.0]);
        let seconds = "demo tick,frame start time,frame start delta\n1,0,0.005\n2,0.005,0.0065\n";
        let ms = timedemo_profile(seconds);
        assert!((ms[0] - 5.0).abs() < 1e-9 && (ms[1] - 6.5).abs() < 1e-9);
        assert!(timedemo_profile("nothing,here\n1,2\n").is_empty());
    }

    #[test]
    fn the_showfps_log_counts_its_column_from_the_end() {
        let csv = "Time,Player 1 Position,Smooth FPS,Frame FPS,Smooth MS,Frame MS,Server Frame MS\n\
                   0.01,-120.5 340.2 64.0,180,182,5.5,5.49,0.0\n\
                   0.02,-120.5, 340.2, 64.0,181,150,5.5,6.67,0.0\n";
        assert_eq!(showfps_log(csv), vec![5.49, 6.67]);
        assert!(showfps_log("Time,FPS\n1,2\n").is_empty());
    }
}
