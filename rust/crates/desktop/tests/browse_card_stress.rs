//! Renders the worst-case Browse cards headless and reads the laid-out tree: every card shows
//! all three counts, nothing leaves its card, and all cards are the same height.

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

const COUNTS: [&str; 3] = ["likes", "views", "downloads"];
const CARDS: u64 = 8;
const FIRST: u64 = 9_000_000;

fn rect(e: &Value) -> [f64; 4] {
    let r = e["rect"].as_array().expect("rect");
    [0, 1, 2, 3].map(|i| r[i].as_f64().expect("number"))
}

fn inside(inner: [f64; 4], outer: [f64; 4]) -> bool {
    const E: f64 = 0.5;
    inner[0] >= outer[0] - E
        && inner[1] >= outer[1] - E
        && inner[0] + inner[2] <= outer[0] + outer[2] + E
        && inner[1] + inner[3] <= outer[1] + outer[3] + E
}

fn check(scene: &str, width: u32) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("browse-card-stress");
    std::fs::create_dir_all(&dir).expect("output folder");
    let stem = format!("{scene}-{width}");
    let (png, tree) = (dir.join(format!("{stem}.png")), dir.join(format!("{stem}.json")));
    let run = Command::new(env!("CARGO_BIN_EXE_dah-desktop"))
        .env("DAH_DATA_DIR", dir.join("data"))
        // Tall enough that the virtual rows build all eight cards.
        .args(["--snap", png.to_str().unwrap(), "--scene", scene, "--size", &format!("{width}x1500")])
        .args(["--at", "1", "--scale", "1", "--tree", tree.to_str().unwrap()])
        .output()
        .expect("run the scene");
    assert!(run.status.success(), "{stem}: {}", String::from_utf8_lossy(&run.stderr));
    let doc: Value = serde_json::from_slice(&std::fs::read(&tree).expect("tree")).expect("json");
    let elements = doc["elements"].as_array().expect("elements");
    let find = |key: &str| elements.iter().find(|e| e["key"] == key);

    let mut height = None;
    for id in FIRST..FIRST + CARDS {
        let card = find(&format!("browse-card-{id}")).unwrap_or_else(|| panic!("{stem}: card {id}"));
        let card_rect = rect(card);
        let h = *height.get_or_insert(card_rect[3]);
        assert!((card_rect[3] - h).abs() < 0.5, "{stem}: card {id} is {} tall, not {h}", card_rect[3]);

        for count in COUNTS {
            let key = format!("card-stat-{count}-{id}");
            let cell = find(&key).unwrap_or_else(|| panic!("{stem}: no {key}"));
            let cell_rect = rect(cell);
            assert!(inside(cell_rect, card_rect), "{stem}: {key} leaves the card");
            let path = format!("{}/", cell["path"].as_str().expect("path"));
            let number = elements
                .iter()
                .find(|e| e["role"] == "text" && e["path"].as_str().is_some_and(|p| p.starts_with(&path)))
                .unwrap_or_else(|| panic!("{stem}: {key} has no number"));
            assert!(!number["text"].as_str().unwrap_or("").is_empty(), "{stem}: {key} is empty");
            assert!(inside(rect(number), cell_rect), "{stem}: {key} number leaves its column");
        }
        for key in [format!("card-actions-{id}"), format!("install-{id}")] {
            let e = find(&key).unwrap_or_else(|| panic!("{stem}: no {key}"));
            assert!(inside(rect(e), card_rect), "{stem}: {key} leaves the card");
        }
    }
}

#[test]
fn every_card_keeps_its_counts_and_actions() {
    for scene in ["browse-card-stress", "browse-card-stress-de"] {
        for width in [1040, 1360, 1920] {
            check(scene, width);
        }
    }
    // 1112 wide puts Small cards at their 192 point minimum.
    for width in [1112, 1360] {
        check("browse-card-stress-small", width);
    }
    check("browse-card-stress-large", 1360);
}
