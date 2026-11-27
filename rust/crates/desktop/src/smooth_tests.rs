//! What the shell's transitions cost and what moves in them: the sidebar collapse and expand
//! and a page switch, on the real view, headless on Rune's runtime.

use rune::{Env, Runtime};

use crate::app::{self, State};

const BROWSE_FIXTURE: &str = include_str!("../fixtures/browse/subfeed.json");

fn runtime(name: &str, size: (f32, f32)) -> Runtime<State> {
    let mut s = crate::data::init_state();
    s.prefs.reduce_motion = false;
    if name.starts_with("browse") {
        crate::data::browse_from_body(&mut s, BROWSE_FIXTURE);
        if let Some(page) = s.browse.page.as_mut() {
            page.complete = true;
        }
    }
    let scene = app::scenes()
        .iter()
        .chain(crate::bench_scenes::SCENES.iter())
        .find(|(n, _)| *n == name)
        .map(|(_, f)| *f)
        .unwrap_or_else(|| panic!("no scene {name}"));
    scene(&mut s);
    let mut rt = Runtime::new(s, app::view, size, Env::new(true, 1.0));
    rt.check_live_only(false);
    settle(&mut rt);
    rt
}

fn settle(rt: &mut Runtime<State>) {
    for n in 0..240 {
        if !rt.frame(1.0 / 60.0) && n > 2 {
            return;
        }
    }
}

fn press(rt: &mut Runtime<State>, key: &str) {
    let e = rt
        .elements()
        .into_iter()
        .find(|e| !e.exiting && e.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("no {key}"));
    let (x, y) = (e.rect[0] + e.rect[2] / 2.0, e.rect[1] + e.rect[3] / 2.0);
    rt.pointer_move(x, y);
    rt.frame(1.0 / 60.0);
    rt.pointer_down(x, y);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(x, y);
}

/// One frame after an input: wall time, commits, the sidebar's width, elements part faded.
struct Frame {
    ms: f64,
    commits: u64,
    sidebar: f32,
    fading: Vec<String>,
    shaped: u32,
}

fn frames_after(rt: &mut Runtime<State>, n: usize) -> Vec<Frame> {
    let mut out = Vec::new();
    for _ in 0..n {
        let before = rt.stats.commits;
        let start = std::time::Instant::now();
        rt.frame(1.0 / 60.0);
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        let els = rt.elements();
        let sidebar = els.iter().find(|e| e.key.as_deref() == Some("sidebar")).map_or(0.0, |e| e.rect[2]);
        let fading = els
            .iter()
            .filter(|e| e.opacity > 0.01 && e.opacity < 0.99)
            .filter_map(|e| e.key.clone())
            .take(4)
            .collect();
        out.push(Frame { ms, commits: rt.stats.commits - before, sidebar, fading, shaped: rt.stats.shaped });
    }
    out
}

fn report(what: &str, frames: &[Frame]) {
    let busy = frames.iter().rposition(|f| f.commits > 0 || !f.fading.is_empty()).map_or(0, |i| i + 1);
    eprintln!("probe: {what}: {busy} frames until still");
    for (i, f) in frames.iter().take(busy.max(2)).enumerate() {
        eprintln!("  {i:2} {:6.2} ms  commits {} sidebar {:5.1}  {:?}", f.ms, f.commits, f.sidebar, f.fading);
    }
}

/// Prints what the sidebar and a page switch do frame by frame. A measurement, not a guard:
/// run it by name with `--ignored --nocapture`.
#[test]
#[ignore]
fn probe_shell_transitions() {
    let _globals = crate::ui::test_globals();
    for scene in ["browse-dashboard", "demo-installed"] {
        let mut rt = runtime(scene, (1280.0, 800.0));
        press(&mut rt, "sidebar-collapse");
        report(&format!("{scene} collapse"), &frames_after(&mut rt, 40));
        settle(&mut rt);
        press(&mut rt, "sidebar-collapse");
        report(&format!("{scene} expand"), &frames_after(&mut rt, 40));
        settle(&mut rt);
    }
    let mut rt = runtime("demo-installed", (1280.0, 800.0));
    for nav in ["nav-browse", "nav-installed", "nav-locker", "nav-stats", "nav-browse"] {
        press(&mut rt, nav);
        report(&format!("switch {nav}"), &frames_after(&mut rt, 30));
        settle(&mut rt);
    }
}

/// How long Browse's landing takes from shown to every picture on screen decoded, with the
/// pictures in the disk cache (warm) or not (`SMOOTH_COLD=1`: from GameBanana). Real image
/// cache, real fetch crew, real decoder threads; frames paced at 60 Hz. A measurement, not a
/// guard: run it by name with `--ignored --nocapture`, with `SIGIL_DATA_DIR` naming a folder
/// of its own.
#[test]
#[ignore]
fn measure_browse_pictures() {
    let _globals = crate::ui::test_globals();
    let cold = std::env::var_os("SMOOTH_COLD").is_some();
    // The scene reads its pages from the fixture and fills the disk cache on the way.
    let mut rt = runtime("browse-dashboard", (1280.0, 800.0));
    let images = crate::data::data_dir().join("cache").join("images");
    if cold {
        let _ = std::fs::remove_dir_all(&images);
    }
    rt.update(|s, _| {
        s.pending_thumbs.clear();
        s.thumbnails.clear();
        s.thumb_crew.clear();
        s.images = Some(rune::ImageCache::new(std::sync::Arc::new(|| {})).decode_when_drawn(true));
    });
    let decodes = rune::image_memory().decodes;
    let commits = rt.stats.commits;
    let start = std::time::Instant::now();
    let tick = std::time::Duration::from_micros(16_667);
    let (mut first, mut all, mut sharp) = (None, None, None);
    let mut frames = 0;
    let mut blank_billboard = 0;
    while start.elapsed() < std::time::Duration::from_secs(if cold { 30 } else { 10 }) {
        let due = start + tick * (frames + 1);
        let t = start.elapsed().as_secs_f64();
        // As the app's loop does: the view runs again only when the poll says something changed.
        rt.update(|s, tx| {
            if !crate::app::poll(s, t, tx) {
                tx.live_only();
            }
        });
        rt.frame(1.0 / 60.0);
        frames += 1;
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        let (ready, total) = crate::load_trace::shown(&rt.state);
        if ready > 0 && first.is_none() {
            first = Some(ms);
        }
        if total > 0 && ready == total && all.is_none() {
            all = Some((ms, total));
        }
        let (blank, large) = crate::screens::browse::top::billboard_now(&rt.state).unwrap_or((false, false));
        if sharp.is_none() && large {
            sharp = Some(ms);
        }
        // The billboard draws a picture that is not decoded yet: it shows nothing there.
        if blank {
            blank_billboard += 1;
        }
        if all.is_some() && sharp.is_some() {
            break;
        }
        if let Some(wait) = due.checked_duration_since(std::time::Instant::now()) {
            std::thread::sleep(wait);
        }
    }
    eprintln!(
        "pictures ({}): first {:?} ms, all shown {:?}, billboard sharp {:?} ms, {} frames, {} commits, {} decodes, frames without a billboard picture {}",
        if cold { "cold" } else { "warm" },
        first.map(|m| m.round()),
        all.map(|(m, n)| (m.round(), n)),
        sharp.map(|m| m.round()),
        frames,
        rt.stats.commits - commits,
        rune::image_memory().decodes - decodes,
        blank_billboard,
    );
}

/// The sidebar's collapse and expand on Browse: one commit (the press), then a glide that lays
/// nothing out and shapes no text, and the sidebar ends at its width with the page beside it.
#[test]
fn the_sidebar_glides_in_one_commit_and_lays_nothing_out_while_it_moves() {
    let _globals = crate::ui::test_globals();
    let mut rt = runtime("browse-dashboard", (1280.0, 800.0));
    let mut wrong = Vec::new();
    for (label, end) in [("collapse", 48.0), ("expand", crate::ui::SIDEBAR_W)] {
        let commits = rt.stats.commits;
        press(&mut rt, "sidebar-collapse");
        let frames = frames_after(&mut rt, 30);
        let after = rt.stats.commits - commits;
        // The press's down and up frames, and the commit they make.
        if after > 2 {
            wrong.push(format!("{label}: {after} commits"));
        }
        let widths: Vec<f32> = frames.iter().map(|f| f.sidebar).collect();
        if !widths.windows(2).any(|w| (w[1] - w[0]).abs() > 1.0) {
            wrong.push(format!("{label}: the sidebar did not glide: {widths:?}"));
        }
        if (widths.last().copied().unwrap_or(0.0) - end).abs() > 0.5 {
            wrong.push(format!("{label}: the sidebar ends at {widths:?}"));
        }
        let shaped: Vec<u32> = frames.iter().skip(1).map(|f| f.shaped).filter(|n| *n > 0).collect();
        if !shaped.is_empty() {
            wrong.push(format!("{label}: text shaped while it moved: {shaped:?}"));
        }
        settle(&mut rt);
        let main = rt.elements().into_iter().find(|e| !e.exiting && e.key.as_deref() == Some("main-glide")).map(|e| e.rect[0]);
        if main.is_none_or(|x| (x - end).abs() > 0.5) {
            wrong.push(format!("{label}: the page starts at {main:?}, not at the sidebar's edge {end}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Under reduced motion the sidebar lands at its width in the press's commit.
#[test]
fn the_sidebar_lands_at_once_under_reduced_motion() {
    let _globals = crate::ui::test_globals();
    let mut rt = runtime("demo-installed", (1280.0, 800.0));
    rt.set_system_reduced_motion(true);
    settle(&mut rt);
    press(&mut rt, "sidebar-collapse");
    let frames = frames_after(&mut rt, 3);
    assert!(frames.iter().all(|f| (f.sidebar - 48.0).abs() < 0.5), "{:?}", frames.iter().map(|f| f.sidebar).collect::<Vec<_>>());
}

/// Opening Browse from the sidebar is one commit: its laid-out width is read in that commit,
/// and storing it for the polls makes no second one.
#[test]
fn opening_browse_is_one_commit() {
    let _globals = crate::ui::test_globals();
    let mut rt = runtime("demo-installed", (1280.0, 800.0));
    press(&mut rt, "nav-browse");
    // The release lands in the first frame after the press.
    let frames = frames_after(&mut rt, 10);
    let commits: Vec<u64> = frames.iter().map(|f| f.commits).collect();
    assert_eq!(commits.iter().sum::<u64>(), 1, "commits per frame after the release: {commits:?}");
}


fn ready_pixels() -> rune::Pixels {
    let px = rune::Pixels::new();
    px.set(4, 4, vec![200; 64]);
    px
}

/// The owner's "the picture is there, goes grey, comes back" in a mod's details: the details
/// open on the card's own picture, and when the page lands the gallery's larger copy (sized from
/// its header, not decoded) must not replace it before it can draw. Every frame shows a picture;
/// the larger one takes over in place once decoded.
#[test]
fn a_shown_details_picture_never_goes_back_to_its_empty_box() {
    let _globals = crate::ui::test_globals();
    let mut rt = runtime("browse", (1280.0, 800.0));
    let card = "https://images.gamebanana.com/img/ss/mods/530-90_smooth.jpg".to_string();
    let gallery = "https://images.gamebanana.com/img/ss/mods/800-90_smooth.jpg".to_string();
    rt.update(|s, _| {
        let Some(first) = s.browse.page.as_ref().and_then(|p| p.mods.first()) else { return };
        let mut card_now = crate::screens::browse::clone_card(first);
        card_now.thumbnail_url = Some(card.clone());
        card_now.nsfw = false;
        s.thumbnails.insert(card.clone(), ready_pixels());
        s.browse_view.details = Some(crate::screens::browse::Details::from_card(&card_now));
    });
    settle(&mut rt);
    let carousel = |rt: &Runtime<State>| -> (bool, bool) {
        let els = rt.elements();
        let shown = els.iter().any(|e| !e.exiting && e.key.as_deref() == Some("carousel-0"));
        let empty = els.iter().any(|e| !e.exiting && e.key.as_deref() == Some("carousel-0-missing"));
        (shown, empty)
    };
    assert_eq!(carousel(&rt), (true, false), "the details open on the card's picture");
    // The page lands: the gallery's first picture is the 800 pixel copy, only sized so far.
    rt.update(|s, _| {
        let details = s.browse_view.details.as_mut().expect("details");
        details.profile = Some(deadlock_core::gamebanana::ModProfile {
            images: vec![deadlock_core::gamebanana::ModImage { url: gallery.clone(), full_url: gallery.replace("800-90_", ""), aspect: 16.0 / 9.0, ..Default::default() }],
            ..Default::default()
        });
        s.thumbnails.insert(gallery.clone(), rune::Pixels::new());
    });
    for n in 0..6 {
        rt.frame(1.0 / 60.0);
        assert_eq!(carousel(&rt), (true, false), "frame {n} after the page landed: the box went empty");
    }
    // Decoded: it takes over in the same element, with no empty frame between.
    rt.update(|s, _| {
        s.thumbnails.insert(gallery.clone(), ready_pixels());
    });
    for n in 0..6 {
        rt.frame(1.0 / 60.0);
        assert_eq!(carousel(&rt), (true, false), "frame {n} after the decode");
    }
}
