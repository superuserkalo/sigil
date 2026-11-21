//! Content search: find text inside the files of a source, compiled data included (decoded to
//! KV3 text first). Runs on a worker with a shared [`SearchControl`] the UI reads for progress
//! and live hits, and sets to pause or stop.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use serde::Serialize;

use super::preview::{self, NO_TEXT_COMPILED};
use super::{Listing, extension};

/// Defaults, after DMM's caps: 200 hits shown, 3 lines per file, 4 MB per file.
pub const DEFAULT_MAX_HITS: usize = 200;
pub const DEFAULT_LINES_PER_FILE: usize = 3;
pub const DEFAULT_MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// A query longer than this is refused.
pub const MAX_QUERY_CHARS: usize = 4096;
/// A hit line is cut to this many characters around the match.
const SNIPPET_BEFORE: usize = 80;
const SNIPPET_AFTER: usize = 240;

#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub query: String,
    pub case_sensitive: bool,
    /// Only paths containing this text (case-insensitive) are searched.
    pub path_filter: Option<String>,
    pub max_hits: usize,
    pub lines_per_file: usize,
    pub max_file_bytes: u64,
}

impl SearchOptions {
    pub fn new(query: impl Into<String>) -> Self {
        SearchOptions {
            query: query.into(),
            case_sensitive: false,
            path_filter: None,
            max_hits: DEFAULT_MAX_HITS,
            lines_per_file: DEFAULT_LINES_PER_FILE,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub path: String,
    /// 1-based line number in the decoded text.
    pub line: usize,
    /// The line, cut around the match.
    pub text: String,
    /// Byte range of the match inside `text`.
    pub start: usize,
    pub end: usize,
    /// Matching lines in this file, shown or not.
    pub file_matches: usize,
}

/// Shared between the worker and the UI.
#[derive(Debug, Default)]
pub struct SearchControl {
    stop: AtomicBool,
    paused: AtomicBool,
    pub done: AtomicUsize,
    pub total: AtomicUsize,
    hits: Mutex<Vec<SearchHit>>,
}

impl SearchControl {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    pub fn paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    /// A copy of the hits so far.
    pub fn hits(&self) -> Vec<SearchHit> {
        self.hits.lock().map(|hits| hits.clone()).unwrap_or_default()
    }

    pub fn hit_count(&self) -> usize {
        self.hits.lock().map(|hits| hits.len()).unwrap_or(0)
    }

    pub fn progress(&self) -> (usize, usize) {
        (self.done.load(Ordering::Relaxed), self.total.load(Ordering::Relaxed))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchReport {
    pub hits: Vec<SearchHit>,
    pub files_searched: usize,
    pub files_matched: usize,
    /// Files with no text (pixels, sound), over the size cap, or unreadable.
    pub files_skipped: usize,
    pub stopped: bool,
    /// The hit cap was reached before every file was searched.
    pub capped: bool,
}

/// True when a file can hold searchable text, judged by its name alone.
pub fn searchable(path: &str) -> bool {
    let ext = extension(path);
    if ext.ends_with("_c") {
        return !NO_TEXT_COMPILED.contains(&ext.as_str());
    }
    !preview::RASTER_EXTENSIONS.contains(&ext.as_str())
        && !preview::AUDIO_EXTENSIONS.contains(&ext.as_str())
        && !matches!(ext.as_str(), "vpk" | "psd" | "fbx" | "exe" | "dll" | "zip" | "7z" | "rar")
}

/// Search every file of a listing. `wake` runs now and then so a UI can redraw progress.
pub fn search(
    listing: &Listing,
    options: &SearchOptions,
    control: &SearchControl,
    wake: &dyn Fn(),
) -> Result<SearchReport, String> {
    let query = options.query.trim();
    if query.is_empty() {
        return Err("Type something to search for.".to_string());
    }
    if query.chars().count() > MAX_QUERY_CHARS {
        return Err(format!("The search text is longer than {MAX_QUERY_CHARS} characters."));
    }
    let needle = if options.case_sensitive {
        query.to_string()
    } else {
        query.to_lowercase()
    };
    let filter = options
        .path_filter
        .as_deref()
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .map(str::to_lowercase);
    let candidates: Vec<&super::SourceFile> = listing
        .files
        .iter()
        .filter(|file| {
            filter
                .as_deref()
                .is_none_or(|f| file.path.to_lowercase().contains(f))
        })
        .collect();
    control.total.store(candidates.len(), Ordering::Relaxed);
    control.done.store(0, Ordering::Relaxed);
    if let Ok(mut hits) = control.hits.lock() {
        hits.clear();
    }

    let mut report = SearchReport {
        hits: Vec::new(),
        files_searched: 0,
        files_matched: 0,
        files_skipped: 0,
        stopped: false,
        capped: false,
    };
    for (i, file) in candidates.iter().enumerate() {
        while control.paused() && !control.stopped() {
            std::thread::sleep(Duration::from_millis(50));
        }
        if control.stopped() {
            report.stopped = true;
            break;
        }
        if report.hits.len() >= options.max_hits {
            report.capped = true;
            break;
        }
        control.done.store(i, Ordering::Relaxed);
        if i % 256 == 0 {
            wake();
        }
        if !searchable(&file.path) || file.size > options.max_file_bytes {
            report.files_skipped += 1;
            continue;
        }
        let Ok(bytes) = listing.read(&file.path) else {
            report.files_skipped += 1;
            continue;
        };
        let Some(decoded) = preview::decoded_text(&file.path, &bytes) else {
            report.files_skipped += 1;
            continue;
        };
        report.files_searched += 1;
        let found = find_lines(&file.path, &decoded.text, query, &needle, options);
        if found.is_empty() {
            continue;
        }
        report.files_matched += 1;
        let room = options.max_hits - report.hits.len();
        let found: Vec<SearchHit> = found.into_iter().take(room).collect();
        if let Ok(mut hits) = control.hits.lock() {
            hits.extend(found.iter().cloned());
        }
        report.hits.extend(found);
        wake();
    }
    control.done.store(candidates.len(), Ordering::Relaxed);
    wake();
    Ok(report)
}

fn find_lines(path: &str, text: &str, query: &str, needle: &str, options: &SearchOptions) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    let mut matches = 0;
    for (number, line) in text.lines().enumerate() {
        let at = if options.case_sensitive {
            line.find(needle)
        } else {
            find_insensitive(line, needle)
        };
        let Some(at) = at else {
            continue;
        };
        matches += 1;
        if hits.len() < options.lines_per_file {
            let end = at + query.len().min(line.len() - at);
            let (snippet, start) = snippet(line, at, end);
            let snippet_end = (start + (end - at)).min(snippet.len());
            hits.push(SearchHit {
                path: path.to_string(),
                line: number + 1,
                text: snippet,
                start,
                end: snippet_end,
                file_matches: 0,
            });
        }
    }
    for hit in &mut hits {
        hit.file_matches = matches;
    }
    hits
}

/// The byte offset in `line` of `needle` (already lowercased), ignoring case.
fn find_insensitive(line: &str, needle: &str) -> Option<usize> {
    if line.is_ascii() && needle.is_ascii() {
        let hay = line.as_bytes();
        let pin = needle.as_bytes();
        if pin.len() > hay.len() {
            return None;
        }
        return (0..=hay.len() - pin.len())
            .find(|&i| hay[i..i + pin.len()].eq_ignore_ascii_case(pin));
    }
    // Lowercasing can change byte lengths; map char positions back.
    let lowered: Vec<(usize, char)> = line
        .char_indices()
        .flat_map(|(i, c)| c.to_lowercase().map(move |l| (i, l)))
        .collect();
    let target: Vec<char> = needle.chars().collect();
    if target.len() > lowered.len() {
        return None;
    }
    (0..=lowered.len() - target.len())
        .find(|&s| lowered[s..s + target.len()].iter().map(|(_, c)| *c).eq(target.iter().copied()))
        .map(|s| lowered[s].0)
}

/// A line cut to a window around the match. Returns the text and where the match starts in it.
fn snippet(line: &str, start: usize, end: usize) -> (String, usize) {
    let mut from = start.saturating_sub(SNIPPET_BEFORE);
    while from > 0 && !line.is_char_boundary(from) {
        from -= 1;
    }
    let mut to = (end + SNIPPET_AFTER).min(line.len());
    while to < line.len() && !line.is_char_boundary(to) {
        to += 1;
    }
    let lead = line[from..start].trim_start();
    let offset = lead.len();
    let text = format!("{lead}{}", &line[start..to]);
    (text.trim_end().to_string(), offset)
}
