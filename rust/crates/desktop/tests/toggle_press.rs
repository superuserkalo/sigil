//! Presses the switch itself (its track, never its label) in a real headless run, one per
//! screen, and checks the knob ends on the other side: one press, one flip. Everything runs
//! against an empty home, data and game folder, so nothing touches a real install.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

/// Scene, controls to open first, and the key of the switch.
const CASES: &[(&str, &[&str], &str)] = &[
    ("locker-hero", &[], "locker-hero-3d-haze"),
    ("locker-hero", &[], "locker-hero-shuffle-haze"),
    ("viewer-controls", &[], "viewer-demo-sw-rotate"),
    ("viewer-hideout", &["viewer-demo-controls"], "viewer-demo-sw-rotate"),
    ("installed", &[], "toggle-01"),
    ("installed-lists", &[], "list-random-demo-comp"),
    ("w4b-browse-paste", &[], "paste-skip-nsfw"),
    ("crosshair", &["crosshair-groups-3"], "crosshair-switch-Static gap"),
    ("next-convars", &[], "convars-cheats"),
    ("performance-editor", &[], "perf-editor-only-changed"),
    ("next-stats-roll", &[], "stats-roll-lock"),
    ("regions-default", &[], "regions-health-bar"),
    ("host-setup", &[], "host-upnp-toggle"),
    ("profiles-list", &[], "profile-isolated-ranked"),
    ("settings-advanced", &[], "settings-unknown-alert-switch"),
    ("personalize", &[], "settings-transparent-sidebar-switch"),
];

/// Window height: tall enough that the lowest switch is on screen without scrolling.
const HEIGHT: u32 = 4000;
const OPENED: f64 = 0.8;
const PRESSED: f64 = 1.3;
const SETTLED: f64 = 2.0;

fn render(dir: &Path, stem: &str, scene: &str, events: &[Value], at: f64) -> Result<Vec<Value>, String> {
    let (png, tree, script) = (dir.join(format!("{stem}.png")), dir.join(format!("{stem}.json")), dir.join(format!("{stem}-script.json")));
    std::fs::write(&script, Value::Array(events.to_vec()).to_string()).expect("script");
    // Every run starts from nothing: a switch saved by an earlier run must not carry over.
    let (data, game) = (dir.join(format!("data-{stem}")), dir.join(format!("game-{stem}")));
    for folder in [&data, &game] {
        if folder.exists() {
            std::fs::remove_dir_all(folder).expect("clear the run's folder");
        }
    }
    let addons = game.join("game/citadel/addons");
    std::fs::create_dir_all(&addons).expect("addons folder");
    std::fs::write(game.join("game/citadel/gameinfo.gi"), "\"GameInfo\" {}\n").expect("gameinfo");
    std::fs::write(addons.join("pak01_dir.vpk"), b"fixture").expect("fixture mod");
    let run = Command::new(env!("CARGO_BIN_EXE_dah-desktop"))
        .env("HOME", dir.join("home"))
        .env("DAH_DATA_DIR", &data)
        .env("DAH_GAME_DIR", &game)
        .args(["--snap", png.to_str().unwrap(), "--scene", scene, "--size", &format!("1360x{HEIGHT}"), "--scale", "1"])
        .args(["--script", script.to_str().unwrap(), "--at", &at.to_string(), "--tree", tree.to_str().unwrap()])
        .output()
        .expect("run the scene");
    if !run.status.success() {
        return Err(format!("{scene} ({stem}): {}", String::from_utf8_lossy(&run.stderr)));
    }
    let doc: Value = serde_json::from_slice(&std::fs::read(&tree).expect("tree")).expect("json");
    Ok(doc["elements"].as_array().expect("elements").clone())
}

fn rect(e: &Value) -> Vec<f64> {
    e["rect"].as_array().expect("rect").iter().map(|v| v.as_f64().expect("number")).collect()
}

/// The switch's track and knob, inside the element carrying `key`.
fn parts<'a>(elements: &'a [Value], key: &str) -> Option<(&'a Value, &'a Value)> {
    let owner = elements.iter().find(|e| e["key"] == key)?;
    let prefix = format!("{}/", owner["path"].as_str()?);
    let under = |name: &str| elements.iter().find(|e| e["key"] == name && e["path"].as_str().is_some_and(|p| p.starts_with(&prefix)));
    Some((under("toggle-track")?, under("toggle-knob")?))
}

fn dialogs(elements: &[Value]) -> usize {
    elements.iter().filter(|e| e["role"] == "dialog").count()
}

fn check(dir: &Path, index: usize) -> Result<(), String> {
    let (scene, open, key) = CASES[index];
    let stem = format!("case-{index}");
    let opening: Vec<Value> = open
        .iter()
        .enumerate()
        .map(|(i, key)| json!({"t": 0.2 + i as f64 * 0.2, "type": "tap", "key": key}))
        .collect();
    let before = render(dir, &format!("{stem}-before"), scene, &opening, PRESSED)?;
    let (track, knob) = parts(&before, key).ok_or(format!("{scene}: no switch {key}"))?;
    let t = rect(track);
    let (x, y) = (t[0] + t[2] / 2.0, t[1] + t[3] / 2.0);
    if y >= f64::from(HEIGHT) {
        return Err(format!("{scene} {key}: below the window"));
    }
    let from = rect(knob)[0] - t[0];
    let mut events = opening;
    events.push(json!({"t": OPENED, "type": "tap", "x": x, "y": y}));
    let after = render(dir, &format!("{stem}-after"), scene, &events, SETTLED)?;
    let (track, knob) = parts(&after, key).ok_or(format!("{scene}: {key} left after the press"))?;
    let to = rect(knob)[0] - rect(track)[0];
    if (from - to).abs() < 7.5 {
        return Err(format!("{scene} {key}: knob stayed at {from} (now {to}), no single flip"));
    }
    if dialogs(&after) > dialogs(&before) {
        return Err(format!("{scene} {key}: the press also opened a dialog"));
    }
    Ok(())
}

#[test]
fn pressing_the_switch_flips_it_once() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("toggle-press");
    std::fs::create_dir_all(dir.join("home")).expect("home");
    let mut failures = Vec::new();
    for chunk in (0..CASES.len()).collect::<Vec<_>>().chunks(4) {
        let runs: Vec<_> = chunk
            .iter()
            .map(|&index| {
                let dir = dir.clone();
                std::thread::spawn(move || check(&dir, index))
            })
            .collect();
        for run in runs {
            if let Err(error) = run.join().expect("case thread") {
                failures.push(error);
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
