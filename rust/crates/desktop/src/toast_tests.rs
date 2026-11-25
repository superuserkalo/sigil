//! Toast overflow guards: every toast a handler can raise, the longest German strings and
//! synthetic worst cases (a 300 character sentence, a long Windows path with no spaces, a
//! newline list) laid out on the real view at 1040x680 and 1280x800. Nothing in a toast may
//! leave its card, no card may leave the window, a card stays within its line cap, and the
//! stack never covers the sidebar's launch buttons.

use rune::{Env, Runtime};

use crate::app::{self, State};
use crate::notify::{self, Notice};

const SIZES: [(f32, f32); 2] = [(1040.0, 680.0), (1280.0, 800.0)];
/// Tallest a card may be: four text lines, the gap, the padding, the button row, the border.
const CARD_MAX_H: f32 = 4.0 * 20.0 + 2.0 + 20.0 + 30.0 + 2.0;
/// What a `{}` hole gets: long enough to wrap, with a file name in it.
const HOLE: &str = "Crimson Wraith (pak04_dir.vpk)";

fn settle(rt: &mut Runtime<State>) {
    for n in 0..240 {
        if !rt.frame(1.0 / 60.0) && n > 2 {
            return;
        }
    }
}

/// Lay out the launch page with `notices` pushed and check every rule. Returns the problems.
fn check(notices: Vec<Notice<State>>, size: (f32, f32), what: &str) -> Vec<String> {
    let mut s = crate::data::init_state();
    s.prefs.reduce_motion = false;
    crate::screens::launch::scene_page(&mut s);
    for notice in notices {
        notify::push(&mut s, notice);
    }
    let mut rt = Runtime::new(s, app::view, size, Env::new(true, 1.0));
    rt.check_live_only(false);
    settle(&mut rt);
    let elements: Vec<_> = rt.elements().into_iter().filter(|e| !e.exiting).collect();
    let mut problems = Vec::new();
    let inside = |inner: [f32; 4], outer: [f32; 4]| {
        inner[0] >= outer[0] - 0.5
            && inner[1] >= outer[1] - 0.5
            && inner[0] + inner[2] <= outer[0] + outer[2] + 0.5
            && inner[1] + inner[3] <= outer[1] + outer[3] + 0.5
    };
    let window = [0.0, 0.0, size.0, size.1];
    let launch: Vec<[f32; 4]> = elements
        .iter()
        .filter(|e| matches!(e.key.as_deref(), Some("launch-modded" | "launch-vanilla")))
        .map(|e| e.rect)
        .collect();
    let cards: Vec<_> = elements
        .iter()
        .filter(|e| {
            e.key.as_deref().is_some_and(|k| {
                k.strip_prefix("toast-").is_some_and(|id| id.chars().all(|c| c.is_ascii_digit()))
            })
        })
        .collect();
    if cards.is_empty() {
        problems.push(format!("{what}: no toast drawn"));
    }
    for card in &cards {
        let key = card.key.clone().unwrap_or_default();
        if !inside(card.rect, window) {
            problems.push(format!("{what} {size:?}: {key} leaves the window: {:?}", card.rect));
        }
        if card.rect[3] > CARD_MAX_H + 0.5 {
            problems.push(format!("{what} {size:?}: {key} is {} tall", card.rect[3]));
        }
        for button in &launch {
            let [x, y, w, h] = card.rect;
            let overlaps = x < button[0] + button[2] && button[0] < x + w && y < button[1] + button[3] && button[1] < y + h;
            if overlaps {
                problems.push(format!("{what} {size:?}: {key} covers a launch button"));
            }
        }
        let prefix = format!("{}/", card.path);
        for e in elements.iter().filter(|e| e.path.starts_with(&prefix)) {
            if e.rect[2] <= 0.0 || e.rect[3] <= 0.0 {
                continue;
            }
            if !inside(e.rect, card.rect) {
                problems.push(format!(
                    "{what} {size:?}: {:?} ({:?}) leaves {key}: {:?} not in {:?}",
                    e.key, e.text.as_deref().map(|t| t.chars().take(40).collect::<String>()), e.rect, card.rect
                ));
            }
        }
    }
    problems
}

/// The string literal starting at `at` (just after its opening quote), unescaped.
fn literal(src: &str, at: usize) -> Option<String> {
    let mut out = String::new();
    let mut chars = src[at..].chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                'u' => {
                    let code: String = chars.by_ref().skip(1).take_while(|c| *c != '}').collect();
                    out.push(char::from_u32(u32::from_str_radix(&code, 16).ok()?)?);
                }
                '\n' => {
                    let rest: String = chars.clone().collect();
                    let trimmed = rest.trim_start();
                    let skip = rest.chars().count() - trimmed.chars().count();
                    for _ in 0..skip {
                        chars.next();
                    }
                }
                other => out.push(other),
            },
            c => out.push(c),
        }
    }
    None
}

/// The literal a call starts with: `"..."` or `tr("...")`, possibly behind `&fill(`.
fn first_literal(src: &str, at: usize) -> Option<String> {
    let head: String = src[at..].chars().take(40).collect();
    let quote = head.find('"')?;
    let before = &head[..quote];
    if !before.chars().all(|c| c.is_whitespace() || "tr(&fil".contains(c)) {
        return None;
    }
    literal(src, at + quote + 1)
}

/// Every toast title and detail written as a literal in the app's source, with `{}` filled.
fn raised_messages() -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let src = std::fs::read_to_string(&path).unwrap();
            for kind in ["Notice::error(", "Notice::warning(", "Notice::info(", "Notice::success("] {
                for (at, _) in src.match_indices(kind) {
                    let start = at + kind.len();
                    let Some(title) = first_literal(&src, start) else { continue };
                    let window: String = src[start..].chars().take(400).collect();
                    let detail = window
                        .find(".detail(")
                        .and_then(|d| first_literal(&window, d + ".detail(".len()));
                    out.push((title, detail));
                }
            }
        }
    }
    let fill = |t: &str| t.replace("{}", HOLE);
    let mut all = Vec::new();
    for (title, detail) in out {
        all.push((fill(&title), detail.as_deref().map(fill)));
        let de_title = crate::screens::settings::i18n::german_of(&title).map(fill);
        let de_detail = detail.as_deref().and_then(crate::screens::settings::i18n::german_of).map(fill);
        if de_title.is_some() || de_detail.is_some() {
            all.push((
                de_title.unwrap_or_else(|| fill(&title)),
                de_detail.or_else(|| detail.as_deref().map(fill)),
            ));
        }
    }
    all.sort();
    all.dedup();
    all
}

fn notice((title, detail): &(String, Option<String>)) -> Notice<State> {
    let notice = Notice::error(title.clone());
    match detail {
        Some(d) => notice.detail(d.clone()),
        None => notice,
    }
}

#[test]
fn worst_case_toasts_stay_inside_their_cards_and_the_window() {
    let mut problems = Vec::new();
    for size in SIZES {
        for (i, worst) in notify::worst_cases().into_iter().enumerate() {
            problems.extend(check(vec![worst], size, &format!("worst case {i}")));
        }
        // A full stack of the worst: three on screen and the rest counted.
        let mut stack = notify::worst_cases();
        stack.truncate(5);
        problems.extend(check(stack, size, "worst stack"));
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn every_raised_toast_fits_in_english_and_german() {
    let messages = raised_messages();
    assert!(messages.len() > 50, "the sweep found only {} toasts", messages.len());
    let mut problems = Vec::new();
    // Three a time: the stack's cap, so each batch is one full stack at the tight size.
    for (n, batch) in messages.chunks(3).enumerate() {
        problems.extend(check(batch.iter().map(notice).collect(), SIZES[0], &format!("batch {n}")));
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
#[cfg_attr(not(feature = "languages"), ignore = "translations are parked: --features languages")]
fn the_longest_german_strings_fit_as_title_and_detail() {
    let mut german = crate::screens::settings::i18n::german_strings();
    german.sort_by_key(|s| std::cmp::Reverse(s.chars().count()));
    let longest: Vec<&str> = german.into_iter().take(6).collect();
    let mut problems = Vec::new();
    for size in SIZES {
        for pair in longest.chunks(2) {
            let notices = vec![
                Notice::warning(pair[0]).detail(pair.get(1).copied().unwrap_or(pair[0])),
                Notice::success(pair[0]).undo(|_st: &mut State| {}),
            ];
            problems.extend(check(notices, size, "longest German"));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn a_cut_toast_offers_details_and_a_short_one_does_not() {
    let mut s = crate::data::init_state();
    crate::screens::launch::scene_page(&mut s);
    notify::push(&mut s, Notice::error("Could not apply the profile").detail(notify::SCENE_LONG_SENTENCE));
    notify::push(&mut s, Notice::error("Install failed").detail("Disk full."));
    let mut rt = Runtime::new(s, app::view, SIZES[0], Env::new(true, 1.0));
    settle(&mut rt);
    let keys: Vec<String> = rt.elements().into_iter().filter_map(|e| e.key).filter(|k| k.ends_with("-details")).collect();
    assert_eq!(keys.len(), 1, "{keys:?}");
}

#[test]
fn launching_while_a_launch_runs_is_refused_and_the_buttons_are_off() {
    let mut s = crate::data::init_state();
    crate::screens::launch::scene_page(&mut s);
    crate::screens::launch_progress::stage_waiting(&mut s, false);
    assert!(crate::screens::launch_progress::busy(&s).is_some());
    let steps = s.launch.progress.as_ref().map(|p| p.steps.clone());
    crate::screens::launch_progress::begin(&mut s, true, false);
    assert_eq!(s.launch.progress.as_ref().map(|p| p.steps.clone()), steps, "the running launch is kept");
    assert_eq!(s.notices.visible().count(), 1);
    let mut rt = Runtime::new(s, app::view, SIZES[0], Env::new(true, 1.0));
    settle(&mut rt);
    let vanilla = rt.elements().into_iter().find(|e| e.key.as_deref() == Some("launch-vanilla")).expect("the button");
    assert!(vanilla.help.as_deref().is_some_and(|h| h.contains("already starting")), "{:?}", vanilla.help);
}

#[test]
fn a_held_file_reads_as_a_short_line() {
    let line = crate::screens::launch_progress::vanilla_failure(notify::SCENE_LAUNCH_BUSY_RAW);
    assert!(line.len() < 100 && !line.contains('\\'), "{line}");
    assert_eq!(crate::screens::launch_progress::vanilla_failure("Addon folder X does not exist."), "Addon folder X does not exist.");
    let notice = crate::screens::launch_progress::vanilla_held_notice(notify::SCENE_LAUNCH_BUSY_RAW).expect("a held file");
    assert_eq!(notice.kind, notify::Kind::Warning);
    assert!(crate::screens::launch_progress::vanilla_held_notice("Addon folder X does not exist.").is_none());
}
