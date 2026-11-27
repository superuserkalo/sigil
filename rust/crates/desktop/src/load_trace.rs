//! How fast Browse fills, for a person judging it: with `SIGIL_LOAD_TRACE=<file>` set, Sigil
//! appends one line per milestone to that file, milliseconds since the process started:
//!
//! - `first-frame`: the window drew its first frame;
//! - `browse-cards`: the first catalog cards are in Browse (from the network, the saved
//!   catalog or a search);
//! - `pictures-shown`: every picture Browse shows at that moment (the landing's covers on
//!   screen and the grid's visible cards) is decoded and drawn;
//! - `billboard-sharp`: the billboard's own large picture is drawn (it shows the card
//!   thumbnail until then);
//! - `page <name> pictures-shown`: the same for a later visit to Browse, from the press.
//!
//! Unset, every call is one relaxed load and returns. Nothing leaves the machine; the file is
//! written by this process only, and only where the variable points.

use std::collections::HashSet;
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use crate::app::State;

struct Trace {
    out: Mutex<std::fs::File>,
    done: Mutex<HashSet<String>>,
    /// A visit to Browse after a press: when, and whether its pictures were seen ready.
    visit: Mutex<Option<(Instant, u32)>>,
    visits: Mutex<u32>,
}

fn start() -> Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now)
}

fn trace() -> Option<&'static Trace> {
    static TRACE: OnceLock<Option<Trace>> = OnceLock::new();
    TRACE
        .get_or_init(|| {
            let path = std::env::var_os("SIGIL_LOAD_TRACE")?;
            let out = std::fs::OpenOptions::new().create(true).append(true).open(path).ok()?;
            Some(Trace {
                out: Mutex::new(out),
                done: Mutex::default(),
                visit: Mutex::default(),
                visits: Mutex::default(),
            })
        })
        .as_ref()
}

/// From `main`, first thing: the clock starts and the variable is read once.
pub fn begin() {
    start();
    let _ = trace();
}

fn write(t: &Trace, ms: f64, what: &str) {
    if let Ok(mut out) = t.out.lock() {
        let _ = writeln!(out, "{ms:.1} {what}");
    }
}

/// Writes `what` the first time it is reached.
pub fn once(what: &str) {
    let Some(t) = trace() else { return };
    let fresh = t.done.lock().map(|mut done| done.insert(what.to_string())).unwrap_or(false);
    if fresh {
        write(t, start().elapsed().as_secs_f64() * 1000.0, what);
    }
}

/// The grid's visible card pictures, as `browse::store::track_visible` last saw them.
static VISIBLE: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `browse::store::track_visible`: the grid's visible card pictures (a dozen strings, only
/// when the visible rows change).
pub fn note_visible(urls: Vec<String>) {
    if let Ok(mut visible) = VISIBLE.lock() {
        *visible = urls;
    }
}

/// The pictures Browse shows now (the landing's covers on screen and the grid's visible
/// cards), and how many of them are decoded or held by an atlas.
pub fn shown(s: &State) -> (usize, usize) {
    let (now, _) = crate::screens::browse::top::cover_urls(s);
    let grid = VISIBLE.lock().map(|v| v.clone()).unwrap_or_default();
    let mut wanted: Vec<String> = now.into_iter().chain(grid).collect();
    wanted.sort();
    wanted.dedup();
    let ready = wanted.iter().filter(|url| s.thumbnails.get(*url).is_some_and(rune::Pixels::is_ready)).count();
    (ready, wanted.len())
}

/// A sidebar press opened Browse again: its pictures are timed from now.
pub fn visit_browse() {
    let Some(t) = trace() else { return };
    let n = t.visits.lock().map(|mut n| {
        *n += 1;
        *n
    });
    if let (Ok(n), Ok(mut visit)) = (n, t.visit.lock()) {
        *visit = Some((Instant::now(), n));
    }
}

/// From `poll`: checks the milestones not reached yet.
pub fn poll(s: &State) {
    let Some(t) = trace() else { return };
    if crate::first_show::drew_frame() {
        once("first-frame");
    }
    if s.screen != crate::model::Screen::Browse {
        return;
    }
    if s.browse.page.as_ref().is_some_and(|p| !p.mods.is_empty()) {
        once("browse-cards");
    }
    let (ready, total) = shown(s);
    if total > 0 && ready == total {
        once("pictures-shown");
        if let Ok(mut visit) = t.visit.lock()
            && let Some((at, n)) = visit.take()
        {
            write(t, at.elapsed().as_secs_f64() * 1000.0, &format!("page browse visit {n} pictures-shown"));
        }
    }
    // The billboard's large picture: the current pick's `image`, the `BILLBOARD_MARK` url.
    let (now, _) = crate::screens::browse::top::cover_urls(s);
    let ready = |url: &String| s.thumbnails.get(url).is_some_and(rune::Pixels::is_ready);
    if now.iter().filter(|url| url.ends_with(crate::data::BILLBOARD_MARK)).any(|url| ready(url)) {
        once("billboard-sharp");
    }
}
