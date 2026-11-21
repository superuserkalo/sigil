//! Guards against jank: the real view, on real scenes, driven headless on Rune's runtime.
//!
//! - Hover must not re-run the whole view. Every commit rebuilds the window, so a card that
//!   writes hover into `State` makes moving the pointer over a grid cost one full rebuild per
//!   card crossed. Hover-only controls fade in with `hover_group` and `group_opacity` instead.
//! - Hover must not move or resize anything.
//! - A page at rest must stop asking for frames.
//! - Each core scene stays under a layout node budget, so a page can't quietly double in cost.

use rune::{Env, Runtime};

use crate::app::{self, State};

type Scene = fn(&mut State);

const BROWSE_FIXTURE: &str = include_str!("../fixtures/browse/subfeed.json");

fn scene(name: &str) -> Scene {
    app::scenes()
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, f)| *f)
        .unwrap_or_else(|| panic!("no scene {name}"))
}

fn runtime(name: &str, size: (f32, f32)) -> Runtime<State> {
    let mut s = crate::data::init_state();
    if name.starts_with("browse") {
        // Headless scenes read the stored subfeed; a test is not headless, so load it here.
        crate::data::browse_from_body(&mut s, BROWSE_FIXTURE);
        if let Some(page) = s.browse.page.as_mut() {
            page.complete = true;
        }
    }
    scene(name)(&mut s);
    let mut rt = Runtime::new(s, app::view, size, Env::new(true, 1.0));
    // Count real commits only: debug builds otherwise re-run the view after a live-only
    // event to check it, and that run counts as a commit.
    rt.check_live_only(false);
    settle(&mut rt);
    rt
}

/// Frames until nothing moves (at most four seconds at 60 Hz).
fn settle(rt: &mut Runtime<State>) -> usize {
    for n in 0..240 {
        if !rt.frame(1.0 / 60.0) && n > 2 {
            return n;
        }
    }
    240
}

/// Rects of every keyed element, by key.
fn rects(rt: &Runtime<State>) -> Vec<(String, [i32; 4])> {
    let mut out: Vec<(String, [i32; 4])> = rt
        .elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .filter_map(|e| {
            let r = e.rect.map(|v| (v * 2.0).round() as i32);
            e.key.map(|k| (k, r))
        })
        .collect();
    out.sort();
    out
}

/// Centres of the elements whose key starts with `prefix`.
fn centres(rt: &Runtime<State>, prefix: &str, skip: &[&str]) -> Vec<(f32, f32)> {
    rt.elements()
        .into_iter()
        .filter(|e| {
            e.key.as_deref().is_some_and(|k| {
                k.starts_with(prefix) && !skip.iter().any(|s| k.starts_with(s))
            })
        })
        .map(|e| (e.rect[0] + e.rect[2] / 2.0, e.rect[1] + e.rect[3] / 2.0))
        .filter(|(x, y)| *x > 0.0 && *y > 0.0)
        .collect()
}

/// Moves the pointer over each point in turn, a few frames on each, and returns how many
/// commits that took.
fn sweep(rt: &mut Runtime<State>, points: &[(f32, f32)]) -> u64 {
    let before = rt.stats.commits;
    for &(x, y) in points {
        rt.pointer_move(x, y);
        for _ in 0..4 {
            rt.frame(1.0 / 60.0);
        }
    }
    rt.pointer_leave();
    settle(rt);
    rt.stats.commits - before
}

fn hover_check(name: &str, prefix: &str, skip: &[&str]) {
    let mut rt = runtime(name, (1280.0, 800.0));
    let points = centres(&rt, prefix, skip);
    assert!(points.len() >= 3, "{name}: expected cards under {prefix}, found {}", points.len());
    let before = rects(&rt);
    // Rest on each card (under the 0.5 s tooltip delay: a tooltip may commit), then check
    // nothing moved while the pointer was there.
    let mut moved = Vec::new();
    let commits_before = rt.stats.commits;
    for &(x, y) in &points {
        rt.pointer_move(x, y);
        for _ in 0..20 {
            rt.frame(1.0 / 60.0);
        }
        let now = rects(&rt);
        if now != before {
            let diff: Vec<_> = now
                .iter()
                .filter(|item| !before.contains(item))
                .take(4)
                .map(|(k, r)| format!("{k} {r:?}"))
                .collect();
            moved.push(format!("hover at ({x:.0}, {y:.0}) moved: {}", diff.join(", ")));
        }
    }
    let commits = rt.stats.commits - commits_before;
    assert!(moved.is_empty(), "{name}: hover changed layout:\n{}", moved.join("\n"));
    assert_eq!(
        commits, 0,
        "{name}: hovering {} cards re-ran the whole view {commits} times; paint hover with \
         hover_group/group_opacity instead of keeping it in State",
        points.len()
    );
    // A hover handler that skips the commit (`Tx::live_only`) must leave nothing stale: debug
    // builds re-run the view to compare, and list what differed.
    let mut checked = runtime(name, (1280.0, 800.0));
    checked.check_live_only(true);
    sweep(&mut checked, &points);
    let stale = checked.live_only_warnings();
    assert!(stale.is_empty(), "{name}: a live-only hover left the view stale:\n{}", stale.join("\n"));
}

#[test]
fn hovering_installed_cards_neither_commits_nor_moves() {
    hover_check("demo-installed", "addon-", &[]);
}


/// Pages at rest draw nothing: no spinner, pulse or timer keeps them awake.
#[test]
fn core_scenes_come_to_rest() {
    for name in ["demo-installed", "installed-list", "browse-card-stress", "locker", "stats", "launch-page", "performance"] {
        let mut rt = runtime(name, (1280.0, 800.0));
        let mut awake = 0;
        for _ in 0..120 {
            if rt.frame(1.0 / 60.0) {
                awake += 1;
            }
        }
        assert!(awake == 0, "{name}: {awake} of 120 frames at rest still animated");
    }
}

/// Layout nodes per scene at 1280x800. A budget, not a target: raise one only with a reason.
#[test]
fn core_scenes_stay_inside_their_node_budget() {
    let budgets: &[(&str, usize)] = &[
        ("demo-installed", 480),
        ("browse-card-stress", 700),
        ("locker", 400),
        ("stats", 400),
        ("launch-page", 500),
        ("performance", 600),
    ];
    let mut over = Vec::new();
    for &(name, budget) in budgets {
        let rt = runtime(name, (1280.0, 800.0));
        let nodes = rt.stats.layout_nodes;
        eprintln!("jank: {name} {nodes} layout nodes (budget {budget})");
        if nodes > budget {
            over.push(format!("{name}: {nodes} layout nodes, budget {budget}"));
        }
    }
    assert!(over.is_empty(), "{}", over.join("\n"));
}

/// The update banner arrives over every page and goes from found, to downloading, to ready
/// within a minute of launch. It must keep one height through those, or the page jumps twice.
#[test]
fn update_banner_keeps_one_height_through_its_states() {
    let mut heights = Vec::new();
    for state in ["available", "downloading", "ready"] {
        let mut s = crate::data::init_state();
        scene("demo-installed")(&mut s);
        crate::updater::stage_fixture(&mut s, state);
        s.shell.w4.update_hidden = false;
        let mut rt = Runtime::new(s, app::view, (1280.0, 800.0), Env::new(true, 1.0));
        rt.check_live_only(false);
        settle(&mut rt);
        let height = rects(&rt)
            .into_iter()
            .find(|(key, _)| key == "update-banner")
            .map(|(_, r)| r[3] as f32 / 2.0);
        assert!(height.is_some(), "no update banner in the {state} state");
        heights.push((state, height.unwrap_or(0.0)));
    }
    assert!(
        heights.windows(2).all(|w| (w[0].1 - w[1].1).abs() < 0.5),
        "the update banner changes height: {heights:?}"
    );
}
