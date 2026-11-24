//! Scroll bars: every scroll viewport goes through `Ui::scroll_area`, and the bar it adds shows
//! while the content overflows, hides when it fits, and never changes the content's width.

use rune::{Env, Runtime, ScrollBars};

use crate::app::{self, State};

const BROWSE_FIXTURE: &str = include_str!("../fixtures/browse/subfeed.json");

/// Viewports left without a bar on purpose, as `(file, a snippet on the viewport's line)`.
/// None today: every scroller in Sigil is long enough that a bar helps.
const BARE: &[(&str, &str)] = &[];

/// Files that build `virtual_rows`, and the wrapped viewport each one's rows mount in. A
/// virtualised block is not a viewport of its own: it reserves the whole list's height inside
/// the page's scroll viewport, whose bar measures it.
const VIRTUAL_HOSTS: &[(&str, &str)] = &[
    ("screens/installed.rs", "installed-scroll"),
    ("screens/browse/grid/menus.rs", "browse-scroll"),
];

fn sources() -> Vec<(String, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![root.clone()];
    let mut out = Vec::new();
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read src") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            if !name.ends_with(".rs") || name.ends_with("_tests.rs") || name == "tests.rs" {
                continue;
            }
            let rel = path.strip_prefix(&root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
            out.push((rel, std::fs::read_to_string(&path).unwrap_or_default()));
        }
    }
    out
}

/// The text with comments and string contents blanked, same length, so offsets still line up.
fn code_only(src: &str) -> Vec<u8> {
    let b = src.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                out[i] = b' ';
                i += 1;
            }
        } else if b[i] == b'"' {
            i += 1;
            while i < b.len() && b[i] != b'"' {
                if b[i] == b'\\' {
                    out[i] = b' ';
                    i += 1;
                }
                if i < b.len() && b[i] != b'\n' {
                    out[i] = b' ';
                }
                i += 1;
            }
            i += 1;
        } else if b[i] == b'\'' && b.get(i + 2) == Some(&b'\'') {
            out[i + 1] = b' ';
            i += 3;
        } else {
            i += 1;
        }
    }
    out
}

/// Whether the scroll call at `at` sits inside a `scroll_area(..)` or `scroll_area_x(..)`
/// call, or in a `let mut name = ..` whose `name` the file later hands to one.
fn wrapped(code: &[u8], at: usize) -> bool {
    let mut depth = 0i32;
    let mut i = at;
    while i > 0 {
        i -= 1;
        match code[i] {
            b'}' if depth == 0 => break,
            b')' | b']' | b'}' => depth += 1,
            b'(' | b'[' | b'{' if depth > 0 => depth -= 1,
            b'(' => {
                let before = std::str::from_utf8(&code[..i]).unwrap_or_default();
                if before.ends_with("scroll_area") || before.ends_with("scroll_area_x") {
                    return true;
                }
            }
            b'{' | b';' => break,
            _ => {}
        }
    }
    // The statement this call is in: `let mut name = col()...` built up, then wrapped at its use.
    let stmt = std::str::from_utf8(&code[i..at]).unwrap_or_default().trim_start_matches(['{', '}', ';']).trim_start();
    let Some(rest) = stmt.strip_prefix("let mut ") else { return false };
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    let all = std::str::from_utf8(code).unwrap_or_default();
    !name.is_empty() && (all.contains(&format!("scroll_area({name})")) || all.contains(&format!("scroll_area_x({name})")))
}

#[test]
fn every_scroll_viewport_has_a_scroll_bar() {
    let mut bare = Vec::new();
    let mut seen = 0;
    for (file, src) in sources() {
        let code = code_only(&src);
        let text = std::str::from_utf8(&code).unwrap_or_default();
        for pat in [".scroll(", ".scroll_x()", ".scroll_xy()"] {
            for (at, _) in text.match_indices(pat) {
                seen += 1;
                if wrapped(&code, at) {
                    continue;
                }
                let line_no = text[..at].matches('\n').count();
                let line = src.lines().nth(line_no).unwrap_or_default().trim();
                if BARE.iter().any(|(f, snip)| *f == file && line.contains(snip)) {
                    continue;
                }
                bare.push(format!("{file}:{}: {line}", line_no + 1));
            }
        }
        if text.contains("virtual_rows(") {
            assert!(
                VIRTUAL_HOSTS.iter().any(|(f, _)| *f == file),
                "{file} builds virtual_rows: mount them in a viewport wrapped by Ui::scroll_area and list the file in VIRTUAL_HOSTS"
            );
        }
    }
    assert!(seen > 50, "the scan found only {seen} scroll viewports: is it reading the sources?");
    assert!(
        bare.is_empty(),
        "scroll viewports without Ui::scroll_area (wrap them, or list a deliberate exception in BARE):\n{}",
        bare.join("\n")
    );
}

#[test]
fn launch_args_ask_for_shown_scroll_bars_unless_chosen() {
    assert_eq!(crate::ui::scroll_bars_shown(vec![]), ["--scroll-bars", "always"]);
    let chosen = vec!["--scroll-bars".to_owned(), "when-scrolling".to_owned()];
    assert_eq!(crate::ui::scroll_bars_shown(chosen.clone()), chosen);
}

fn scene(name: &str) -> fn(&mut State) {
    app::scenes()
        .iter()
        .chain(crate::bench_scenes::SCENES.iter())
        .find(|(n, _)| *n == name)
        .map(|(_, f)| *f)
        .unwrap_or_else(|| panic!("no scene {name}"))
}

fn runtime(name: &str, size: (f32, f32), bars: ScrollBars) -> Runtime<State> {
    let mut s = crate::data::init_state();
    s.prefs.reduce_motion = false;
    if name.starts_with("browse") {
        crate::data::browse_from_body(&mut s, BROWSE_FIXTURE);
        if let Some(page) = s.browse.page.as_mut() {
            page.complete = true;
        }
    }
    scene(name)(&mut s);
    let mut rt = Runtime::new(s, app::view, size, Env::new(true, 1.0));
    rt.set_system_scroll_bars(bars);
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

/// The thumb of the bar on the viewport keyed `viewport`: its drawn rect and opacity.
fn thumb(rt: &Runtime<State>, viewport: &str) -> Option<([f32; 4], f32)> {
    let area = format!("{viewport}-scroll-area/");
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting && e.key.as_deref() == Some("thumb") && e.path.contains(&area))
        .map(|e| (e.rect, e.opacity))
        .next()
}

fn rect(rt: &Runtime<State>, key: &str) -> [f32; 4] {
    rt.elements()
        .into_iter()
        .find(|e| !e.exiting && e.key.as_deref() == Some(key))
        .map(|e| e.rect)
        .unwrap_or_else(|| panic!("no element {key}"))
}

fn assert_shown(name: &str, size: (f32, f32), viewport: &str, content: &str) {
    let _globals = crate::ui::test_globals();
    let rt = runtime(name, size, ScrollBars::Always);
    let (r, opacity) = thumb(&rt, viewport).unwrap_or_else(|| panic!("{name}: no scroll bar on {viewport}"));
    assert!(r[3] >= 24.0 && opacity > 0.9, "{name}: an overflowing {viewport} shows its thumb: {r:?} at {opacity}");
    assert!(r[2] <= 10.5, "{name}: the thumb is 6 points wide at rest: {r:?}");
    // The bar overlays the viewport's edge: the content is as wide with it as with the bar
    // hidden at rest (overlay bars), so its showing never reflows the page.
    let shown = rect(&rt, content);
    let hidden = rect(&runtime(name, size, ScrollBars::WhenScrolling), content);
    assert_eq!(shown, hidden, "{name}: showing the bar moved or resized {content}");
}

#[test]
fn installed_stress_shows_a_scroll_bar() {
    assert_shown("installed-stress", (1280.0, 800.0), "installed-scroll", "installed-page");
}

#[test]
fn browse_card_stress_shows_a_scroll_bar() {
    // Its cards fill a 1280 by 800 window without overflowing; a shorter one scrolls.
    assert_shown("browse-card-stress", (1280.0, 560.0), "browse-scroll", "browse-page");
}

#[test]
fn settings_general_shows_a_scroll_bar() {
    let _globals = crate::ui::test_globals();
    // A short window, so the General pane overflows.
    let rt = runtime("settings-general", (1040.0, 680.0), ScrollBars::Always);
    let bars: Vec<_> = rt
        .elements()
        .into_iter()
        .filter(|e| !e.exiting && e.key.as_deref() == Some("thumb") && e.path.contains("settings") && e.rect[3] >= 24.0)
        .collect();
    assert!(!bars.is_empty(), "settings-general: the overflowing pane shows a thumb");
}

#[test]
fn content_that_fits_shows_no_thumb() {
    let _globals = crate::ui::test_globals();
    for (name, viewport) in [("installed-empty", "installed-scroll"), ("browse-card-stress", "browse-scroll")] {
        let rt = runtime(name, (1280.0, 800.0), ScrollBars::Always);
        if let Some((r, _)) = thumb(&rt, viewport) {
            assert!(r[3] < 1.0, "{name} fits, yet its thumb is {r:?}");
        }
    }
}

#[test]
fn scrolling_moves_the_thumb_without_a_commit() {
    let _globals = crate::ui::test_globals();
    let mut rt = runtime("installed-stress", (1280.0, 800.0), ScrollBars::Always);
    let before = thumb(&rt, "installed-scroll").expect("a thumb").0;
    let commits = rt.stats.commits;
    rt.wheel(640.0, 400.0, -40.0);
    settle(&mut rt);
    let after = thumb(&rt, "installed-scroll").expect("a thumb").0;
    assert!(after[1] > before[1], "the thumb follows the scroll: {before:?} to {after:?}");
    assert_eq!(rt.stats.commits, commits, "scrolling committed the view");
}
