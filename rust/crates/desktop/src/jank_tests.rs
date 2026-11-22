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
    let _globals = crate::ui::test_globals();
    hover_check("demo-installed", (1280.0, 800.0), "addon-", &[]);
}


#[test]
fn hovering_browse_cards_neither_commits_nor_moves() {
    let _globals = crate::ui::test_globals();
    // Tall enough that the grid under the shelves is in the window.
    hover_check("browse-card-stress", (1280.0, 2000.0), "browse-card-", &["browse-card-actions", "browse-card-stat"]);
}

/// Pages at rest draw nothing: no spinner, pulse or timer keeps them awake.
#[test]
fn core_scenes_come_to_rest() {
    let _globals = crate::ui::test_globals();
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
    let _globals = crate::ui::test_globals();
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
    let _globals = crate::ui::test_globals();
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
    let _globals = crate::ui::test_globals();
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
    let _globals = crate::ui::test_globals();
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
    let _globals = crate::ui::test_globals();
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

// --- Motion guards (`motion.rs`, `.scratch/polish-climb/design/motion.md` section 7) ---------

/// One scripted input: press the element with this key, or press Escape.
#[derive(Clone, Copy)]
enum Step {
    Press(&'static str),
    Escape,
}

/// Scene, steps before the measured one, the measured step, and the window height.
const MOTION_CASES: &[(&str, &[Step], Step, f32)] = &[
    ("demo-installed", &[], Step::Press("installed-more-03"), 800.0),
    ("demo-installed", &[], Step::Press("nav-settings"), 800.0),
    ("demo-installed", &[Step::Press("installed-more-03")], Step::Escape, 800.0),
    ("settings-general", &[], Step::Press("settings-confirm-launch-switch"), 1400.0),
    ("demo-installed", &[Step::Press("nav-settings")], Step::Escape, 800.0),
    ("servers-detail", &[], Step::Press("server-panel-close"), 800.0),
    ("crosshair", &[], Step::Press("crosshair-groups-1"), 1400.0),
    ("browse-view-menu", &[], Step::Press("menu-size-large"), 800.0),
    ("demo-installed", &[], Step::Press("nav-browse"), 800.0),
];

/// Keys whose element, and everything under it, may travel or fade: overlays and the parts
/// that slide inside a fixed box. Anything else is in page flow and must land at once.
const MOVERS: &[&str] = &[
    "-menu", "-popover", "-drawer", "-bar", "toast", "-thumb", "-underline", "-modal", "-scrim",
    "toggle-knob", "-indicator", "settings-layer", "-page-", "-layer",
];

fn mover(path: &str) -> bool {
    path.split('/').any(|seg| MOVERS.iter().any(|m| seg.contains(m)))
}

fn centre_of(rt: &Runtime<State>, key: &str) -> Option<(f32, f32)> {
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .find(|e| e.key.as_deref() == Some(key))
        .map(|e| (e.rect[0] + e.rect[2] / 2.0, e.rect[1] + e.rect[3] / 2.0))
}

fn apply(rt: &mut Runtime<State>, step: Step, what: &str) {
    match step {
        Step::Escape => rt.key(rune::KeyEvent { key: rune::Key::Escape, mods: Default::default() }),
        Step::Press(key) => {
            let (x, y) = centre_of(rt, key).unwrap_or_else(|| panic!("{what}: no {key} to press"));
            rt.pointer_move(x, y);
            rt.frame(1.0 / 60.0);
            rt.pointer_down(x, y);
            rt.frame(1.0 / 60.0);
            rt.pointer_up(x, y);
        }
    }
}

fn case_name(scene: &str, step: Step) -> String {
    match step {
        Step::Press(k) => format!("{scene}: press {k}"),
        Step::Escape => format!("{scene}: Escape"),
    }
}

/// One frame after an input: whether anything visibly moved or faded since the frame before
/// (half a point, or 0.4 percent opacity), whether the window asked for another frame, and the
/// in-flow elements drawn away from where they are laid out.
struct Seen {
    visible: Vec<String>,
    asked: bool,
    off: Vec<String>,
}

fn run_case(scene_name: &str, before: &[Step], step: Step, height: f32, reduced: bool) -> Vec<Seen> {
    let mut rt = runtime(scene_name, (1280.0, height));
    if reduced {
        rt.set_system_reduced_motion(true);
        settle(&mut rt);
    }
    for &b in before {
        apply(&mut rt, b, scene_name);
        settle(&mut rt);
    }
    let name = case_name(scene_name, step);
    let commits = rt.stats.commits;
    apply(&mut rt, step, &name);
    let look = |rt: &Runtime<State>| -> std::collections::HashMap<String, ([f32; 4], f32)> {
        rt.elements().into_iter().map(|e| (format!("{}{}", e.path, if e.exiting { "~" } else { "" }), (e.rect, e.opacity))).collect()
    };
    let mut last = look(&rt);
    let mut out = Vec::new();
    for _ in 0..48 {
        let asked = rt.frame(1.0 / 60.0);
        let now = look(&rt);
        let visible: Vec<String> = now
            .iter()
            .filter(|(path, (rect, opacity))| match last.get(*path) {
                Some((r, o)) => (opacity - o).abs() > 0.004 || rect.iter().zip(r).any(|(a, b)| (a - b).abs() > 0.5),
                // Mounted since the last frame: visible unless it is already whole or gone.
                None => *opacity > 0.004 && *opacity < 0.996,
            })
            .take(3)
            .map(|(path, (rect, opacity))| format!("{path} {rect:?} {opacity:.3}"))
            .collect();
        let off: Vec<String> = rt
            .elements()
            .into_iter()
            .filter(|e| !e.exiting && !mover(&e.path) && e.rect[2] > 0.0)
            .filter(|e| e.rect.iter().zip(e.target.iter()).any(|(a, b)| (a - b).abs() > 0.5))
            .take(3)
            .map(|e| format!("{} drawn {:?} placed {:?}", e.path, e.rect, e.target))
            .collect();
        out.push(Seen { visible, asked, off });
        last = now;
    }
    assert!(rt.stats.commits > commits, "{name}: the input did nothing");
    out
}

/// Every scripted input comes to rest: nothing visibly moves or fades 20 frames (333 ms) after
/// it, at 60 Hz (the longest token is 180 ms), and the window stops drawing within 40 frames.
/// Rune rests a spring when it is within a hair of its target, about 2.5 times its visual
/// duration, and the press fill releases on Rune's own 0.25 s spring, so the last frames
/// before rest change nothing a player can see.
#[test]
fn motion_settles_fast() {
    let _globals = crate::ui::test_globals();
    let mut slow = Vec::new();
    for &(scene_name, before, step, height) in MOTION_CASES {
        let frames = run_case(scene_name, before, step, height, false);
        let still = frames.iter().rposition(|f| !f.visible.is_empty()).map_or(0, |i| i + 1);
        let rests = frames.iter().rposition(|f| f.asked).map_or(0, |i| i + 1);
        let name = case_name(scene_name, step);
        eprintln!("motion: {name}: still after {still} frames, rests after {rests}");
        if still > 20 || rests > 40 {
            let late = frames.get(still.saturating_sub(1)).map(|f| f.visible.join("; ")).unwrap_or_default();
            slow.push(format!("{name}: still after {still}, rests after {rests} frames ({late})"));
        }
    }
    assert!(slow.is_empty(), "motion did not come to rest:\n{}", slow.join("\n"));
}

/// While an input's motion runs, everything in page flow is drawn where it is laid out: only
/// overlays and the parts that slide inside a fixed box move. An eased container fails here.
#[test]
fn motion_moves_nothing_in_flow() {
    let _globals = crate::ui::test_globals();
    let mut moved = Vec::new();
    for &(scene_name, before, step, height) in MOTION_CASES {
        let frames = run_case(scene_name, before, step, height, false);
        if let Some((n, f)) = frames.iter().enumerate().find(|(_, f)| !f.off.is_empty()) {
            moved.push(format!("{} frame {n}: {}", case_name(scene_name, step), f.off.join("; ")));
        }
    }
    assert!(moved.is_empty(), "in-flow content eased:\n{}", moved.join("\n"));
}

/// Under reduced motion every input lands at once: whatever changes, changes in one step (the
/// commit's frame, or the next for hover reveals that follow it), and nothing travels or fades
/// after that.
#[test]
fn reduced_motion_is_instant() {
    let _globals = crate::ui::test_globals();
    let mut slow = Vec::new();
    for &(scene_name, before, step, height) in MOTION_CASES {
        let frames = run_case(scene_name, before, step, height, true);
        if let Some((n, f)) = frames.iter().enumerate().skip(2).find(|(_, f)| !f.visible.is_empty()) {
            slow.push(format!("{}: still moving at frame {n}: {}", case_name(scene_name, step), f.visible.join("; ")));
        }
        if let Some((n, f)) = frames.iter().enumerate().find(|(_, f)| !f.off.is_empty()) {
            slow.push(format!("{} frame {n}: {}", case_name(scene_name, step), f.off.join("; ")));
        }
    }
    assert!(slow.is_empty(), "reduced motion still animates:\n{}", slow.join("\n"));
}

/// Hover across the sidebar, a page toolbar, a settings page and an open menu: paint only, no
/// commit, and the window rests within 12 frames (200 ms) of the pointer leaving.
#[test]
fn hover_never_commits() {
    let _globals = crate::ui::test_globals();
    let cases: &[(&str, &str, f32)] = &[
        ("demo-installed", "nav-", 800.0),
        ("demo-installed", "-toolbar", 800.0),
        ("settings-general", "settings-", 1400.0),
        ("installed-menu", "menu-", 800.0),
    ];
    let mut wrong = Vec::new();
    for &(name, part, height) in cases {
        let size = (1280.0, height);
        let mut rt = runtime(name, size);
        let points: Vec<(f32, f32)> = rt
            .elements()
            .into_iter()
            .filter(|e| !e.exiting && e.key.as_deref().is_some_and(|k| k.contains(part)))
            .flat_map(|e| {
                // Its centre and a point near each end, so a toolbar's controls are crossed.
                let y = e.rect[1] + e.rect[3] / 2.0;
                [0.1, 0.5, 0.9].map(|f| (e.rect[0] + e.rect[2] * f, y))
            })
            .filter(|(x, y)| *x > 0.0 && *y > 0.0 && *x < size.0 && *y < size.1)
            .take(60)
            .collect();
        if points.len() < 3 {
            wrong.push(format!("{name}: only {} points under {part}", points.len()));
            continue;
        }
        let before = rt.stats.commits;
        for &(x, y) in &points {
            rt.pointer_move(x, y);
            for _ in 0..3 {
                rt.frame(1.0 / 60.0);
            }
        }
        rt.pointer_leave();
        let mut rest = 0;
        for n in 1..=60 {
            if !rt.frame(1.0 / 60.0) {
                rest = n;
                break;
            }
        }
        let commits = rt.stats.commits - before;
        eprintln!("motion: hover {name} {part}: {} points, {commits} commits, rests after {rest} frames", points.len());
        if commits > 0 || rest == 0 || rest > 12 {
            wrong.push(format!("{name} {part}: {commits} commits, rests after {rest} frames"));
        }
    }
    assert!(wrong.is_empty(), "hover is not paint only:\n{}", wrong.join("\n"));
}

/// The motion is there: a menu, the settings sheet and the selection thumbs visibly move or
/// fade over at least three frames after their input (a guard against tokens that collapse
/// to nothing), and land within 20.
#[test]
fn overlays_and_selections_move() {
    let _globals = crate::ui::test_globals();
    let cases: &[(&str, &[Step], Step, &str, f32)] = &[
        ("demo-installed", &[], Step::Press("installed-more-03"), "installed-menu", 800.0),
        ("demo-installed", &[], Step::Press("nav-settings"), "settings-panel", 800.0),
        ("crosshair", &[], Step::Press("crosshair-groups-1"), "crosshair-groups-indicator", 1400.0),
        ("browse-view-menu", &[], Step::Press("menu-size-large"), "menu-seg-Cards-thumb", 800.0),
        ("demo-installed", &[], Step::Press("nav-browse"), "page-browse", 800.0),
    ];
    let mut wrong = Vec::new();
    for &(scene_name, before, step, key, height) in cases {
        let mut rt = runtime(scene_name, (1280.0, height));
        for &b in before {
            apply(&mut rt, b, scene_name);
            settle(&mut rt);
        }
        let name = case_name(scene_name, step);
        apply(&mut rt, step, &name);
        let pose = |rt: &Runtime<State>| {
            rt.elements()
                .into_iter()
                .find(|e| !e.exiting && e.key.as_deref().is_some_and(|k| k.ends_with(key)))
                .map(|e| (e.rect, e.opacity))
        };
        let mut last = pose(&rt);
        let mut moving = Vec::new();
        for n in 0..40 {
            rt.frame(1.0 / 60.0);
            let now = pose(&rt);
            if let (Some((r0, o0)), Some((r1, o1))) = (last, now)
                && ((o1 - o0).abs() > 0.004 || r0.iter().zip(r1).any(|(a, b)| (a - b).abs() > 0.5))
            {
                moving.push(n);
            }
            last = now;
        }
        eprintln!("motion: {name}: {key} moved on frames {moving:?}");
        if moving.len() < 3 || moving.last().is_some_and(|&n| n >= 20) {
            wrong.push(format!("{name}: {key} moved on frames {moving:?}"));
        }
    }
    assert!(wrong.is_empty(), "expected motion is missing or long:\n{}", wrong.join("\n"));
}

/// A sidebar press leaves one page mounted: the frame after it, nothing of the old page is
/// drawn, not even fading out, while the new page fades in. A shelf's old page fades out in
/// place under the new one, which paints last.
#[test]
fn page_switch_leaves_one_page() {
    let _globals = crate::ui::test_globals();
    let mut rt = runtime("demo-installed", (1280.0, 800.0));
    apply(&mut rt, Step::Press("nav-browse"), "nav-browse");
    rt.frame(1.0 / 60.0);
    let old: Vec<String> = rt
        .elements()
        .into_iter()
        .filter(|e| e.path.split('/').any(|seg| seg == "page-installed"))
        .take(3)
        .map(|e| e.path)
        .collect();
    assert!(old.is_empty(), "the old page is still drawn after the switch: {old:?}");
    let new = rt.elements().into_iter().find(|e| e.key.as_deref() == Some("page-browse")).map(|e| e.opacity);
    assert!(new.is_some_and(|o| o > 0.0 && o < 1.0), "the new page should be fading in, at {new:?}");

}

fn shelf_view(s: &State, cx: &mut rune::Cx<State>) -> rune::El {
    crate::motion::set_reduced(cx.is_reduced_motion());
    let u = crate::ui::Ui::new();
    let tiles = (0..12).map(|i| rune::el().key(format!("tile-{i}")).size(100.0, 60.0).fill(u.hover)).collect();
    crate::motion::root(rune::col().key("root").w_full().h_full().padding(24.0)).child(
        crate::screens::browse::shelf::shelf(s, &u, 0, "test-shelf", "Shelf", None, tiles, 4, 100.0, 8.0),
    )
}

/// A shelf page comes in 32 points from the side it pages from while fading in, and the old
/// page fades out in place; the shelf, its head and its arrows stay where they are.
#[test]
fn shelf_paging_slides_the_page_only() {
    let _globals = crate::ui::test_globals();
    let mut rt = Runtime::new(crate::data::init_state(), shelf_view, (800.0, 300.0), Env::new(true, 1.0));
    rt.check_live_only(false);
    settle(&mut rt);
    let still = |rt: &Runtime<State>| -> Vec<(String, [i32; 4])> {
        rects(rt).into_iter().filter(|(k, _)| !k.contains("-page-") && !k.starts_with("tile-")).collect()
    };
    let before = still(&rt);
    apply(&mut rt, Step::Press("test-shelf-next"), "shelf next");
    let mut xs = Vec::new();
    let mut both = false;
    for n in 0..40 {
        rt.frame(1.0 / 60.0);
        let pages: Vec<(String, bool, f32, f32)> = rt
            .elements()
            .into_iter()
            .filter(|e| e.key.as_deref().is_some_and(|k| k.starts_with("test-shelf-page-")))
            .map(|e| (e.key.unwrap_or_default(), e.exiting, e.rect[0], e.opacity))
            .collect();
        if n == 1 {
            eprintln!("motion: shelf pages in tree order {pages:?}");
        }
        both |= pages.len() == 2;
        if let Some(p) = pages.iter().find(|p| p.0 == "test-shelf-page-1") {
            xs.push(p.2);
        }
        assert_eq!(still(&rt), before, "frame {n}: the shelf around the page moved");
    }
    assert!(both, "the old page should fade out while the new one comes in");
    let first = xs.first().copied().unwrap_or(0.0);
    let last = xs.last().copied().unwrap_or(0.0);
    assert!(first > last + 8.0, "page 2 should come in from the right: x {first} to {last}");
}
