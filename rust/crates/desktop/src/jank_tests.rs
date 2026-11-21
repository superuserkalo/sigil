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
        .chain(crate::bench_scenes::SCENES.iter())
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
/// Only those inside the window, where the pointer can reach them.
fn centres(rt: &Runtime<State>, size: (f32, f32), prefix: &str, skip: &[&str]) -> Vec<(f32, f32)> {
    rt.elements()
        .into_iter()
        .filter(|e| {
            e.key.as_deref().is_some_and(|k| {
                k.starts_with(prefix) && !skip.iter().any(|s| k.starts_with(s))
            })
        })
        .map(|e| (e.rect[0] + e.rect[2] / 2.0, e.rect[1] + e.rect[3] / 2.0))
        .filter(|(x, y)| *x > 0.0 && *y > 0.0 && *x < size.0 && *y < size.1)
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

fn hover_check(name: &str, size: (f32, f32), prefix: &str, skip: &[&str]) {
    let mut rt = runtime(name, size);
    let points = centres(&rt, size, prefix, skip);
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
    let mut checked = runtime(name, size);
    checked.check_live_only(true);
    sweep(&mut checked, &points);
    let stale = checked.live_only_warnings();
    assert!(stale.is_empty(), "{name}: a live-only hover left the view stale:\n{}", stale.join("\n"));
}

#[test]
fn hovering_installed_cards_neither_commits_nor_moves() {
    hover_check("demo-installed", (1280.0, 800.0), "addon-", &[]);
}


#[test]
fn hovering_browse_cards_neither_commits_nor_moves() {
    // Tall enough that the grid under the shelves is in the window.
    hover_check("browse-card-stress", (1280.0, 2000.0), "browse-card-", &["browse-card-actions", "browse-card-stat"]);
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
        // 96 mods. Before rows were virtual it built every card: 2685 and 2106 nodes.
        ("installed-stress", 2000),
        ("installed-list-stress", 1700),
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

/// Targets of every keyed element, by key.
fn targets(rt: &Runtime<State>) -> Vec<(String, [i32; 4])> {
    let mut out: Vec<(String, [i32; 4])> = rt
        .elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .filter_map(|e| e.key.map(|k| (k, e.target.map(|v| (v * 2.0).round() as i32))))
        .collect();
    out.sort();
    out
}

/// The first frame is laid out at the window's real size. A page that learns its width from a
/// layout handler gets it a frame late, so its grid is built for the launch width first and
/// jumps on frame two, on every open and every resize.
#[test]
fn first_frame_is_laid_out_at_the_real_width() {
    let wrong = same_language(first_frame_moves);
    assert!(wrong.is_empty(), "laid out again after the first frame:\n{}", wrong.join("\n"));
}

/// Runs `check` until no other test switched the interface language while it ran: tests run
/// in parallel and the language is process-wide, so a switch mid-run moves text for reasons
/// that have nothing to do with the check.
fn same_language(check: impl Fn() -> Vec<String>) -> Vec<String> {
    let mut found = Vec::new();
    for _ in 0..4 {
        let before = crate::screens::settings::i18n::current();
        found = check();
        if crate::screens::settings::i18n::current() == before {
            break;
        }
    }
    found
}

fn first_frame_moves() -> Vec<String> {
    let mut wrong = Vec::new();
    for name in ["browse-card-stress", "locker", "demo-installed", "stats"] {
        for size in [(1040.0, 680.0), (1600.0, 900.0)] {
            let mut s = crate::data::init_state();
            if name.starts_with("browse") {
                crate::data::browse_from_body(&mut s, BROWSE_FIXTURE);
                if let Some(page) = s.browse.page.as_mut() {
                    page.complete = true;
                }
            }
            scene(name)(&mut s);
            let mut rt = Runtime::new(s, app::view, size, Env::new(true, 1.0));
            rt.check_live_only(false);
            rt.frame(0.0);
            let first = targets(&rt);
            settle(&mut rt);
            let last = targets(&rt);
            let moved: Vec<String> = last
                .iter()
                .filter(|item| first.iter().any(|f| f.0 == item.0) && !first.contains(item))
                .take(3)
                .map(|(k, r)| format!("{k} -> {r:?}"))
                .collect();
            if !moved.is_empty() {
                wrong.push(format!("{name} at {size:?}: {}", moved.join(", ")));
            }
        }
    }
    wrong
}

/// Dragging the window narrower than a column break must not scroll the page. The grid's
/// virtual rows were keyed by their column count too, so every key changed at the break, and
/// the page at its top jumped about 400 pt down.
#[test]
fn resizing_across_a_column_break_keeps_the_page_where_it_is() {
    let mut rt = runtime("browse-card-stress", (1360.0, 680.0));
    let scrolled = |rt: &Runtime<State>| {
        rt.elements().into_iter().find(|e| e.key.as_deref() == Some("browse-page")).map(|e| e.scrolled[1])
    };
    assert_eq!(scrolled(&rt), Some(0.0));
    for width in (1040..=1360).rev().step_by(16) {
        rt.set_viewport((width as f32, 680.0), 1.0);
        rt.frame(1.0 / 60.0);
        let at = scrolled(&rt).unwrap_or(0.0);
        assert!(at.abs() < 0.5, "at {width} pt wide the page scrolled {at} pt by itself");
    }
}


/// Installed builds the rows near the window only (and about two windows either side), so past
/// that a larger library costs the same to build. Before rows were virtual, every card was
/// built: 2685 layout nodes at 96 mods, about 8000 at 300.
#[test]
fn installed_cost_does_not_grow_with_the_library() {
    let nodes = |mods: u16| {
        let mut s = crate::data::init_state();
        crate::bench_scenes::installed_stress_of(&mut s, mods);
        let mut rt = Runtime::new(s, app::view, (1280.0, 800.0), Env::new(true, 1.0));
        rt.check_live_only(false);
        settle(&mut rt);
        rt.stats.layout_nodes
    };
    let (small, large) = (nodes(300), nodes(600));
    eprintln!("jank: installed at 300 mods {small} nodes, at 600 mods {large}");
    assert!(small < 3000, "300 mods built {small} layout nodes");
    assert!(large < small + small / 10, "600 mods built {large} layout nodes against {small} for 300");
}
