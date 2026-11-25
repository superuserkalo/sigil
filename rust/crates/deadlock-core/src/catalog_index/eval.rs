//! The search review's evaluation (316 player-style queries with the mod ids a player expects)
//! run against the real downloaded index. The data is about 900 KB, so it stays out of the
//! repository and the test is ignored by default:
//!
//! ```text
//! SIGIL_SEARCH_EVAL=<dir with evalset.json and data/index.json> \
//!   cargo test --release -p deadlock-core search_eval -- --ignored --nocapture
//! ```
//!
//! Without the variable it reads `.scratch/search-review` at the repository root. It prints
//! hit@5 and recall@10 per query type and fails when a type drops below its floor.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::*;

#[derive(Deserialize)]
struct Case {
    q: String,
    #[serde(rename = "type")]
    kind: String,
    expect: Vec<u64>,
}

fn eval_dir() -> PathBuf {
    std::env::var_os("SIGIL_SEARCH_EVAL").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../.scratch/search-review")
    })
}

/// Per type: (queries, hit@5, recall@10), in percent.
pub(super) struct Table {
    rows: BTreeMap<String, (usize, f64, f64)>,
    none_empty: usize,
    none_total: usize,
    none_mean: f64,
    per_query: Duration,
}

fn score(cases: &[Case], run: &mut dyn FnMut(&str) -> Vec<u64>) -> Table {
    let mut sums: BTreeMap<String, (usize, f64, f64)> = BTreeMap::new();
    let (mut none_empty, mut none_total, mut none_results) = (0, 0, 0);
    let start = Instant::now();
    for case in cases {
        let found = run(&case.q);
        if case.expect.is_empty() {
            none_total += 1;
            none_results += found.len();
            if found.is_empty() {
                none_empty += 1;
            }
            continue;
        }
        let want: HashSet<u64> = case.expect.iter().copied().collect();
        let within = |k: usize| found.iter().take(k).filter(|id| want.contains(id)).count();
        let hit5 = if within(5) > 0 { 1.0 } else { 0.0 };
        let recall10 = within(10) as f64 / want.len().min(10) as f64;
        for key in [case.kind.as_str(), "ALL"] {
            let row = sums.entry(key.to_string()).or_default();
            row.0 += 1;
            row.1 += hit5;
            row.2 += recall10;
        }
    }
    let per_query = start.elapsed() / cases.len().max(1) as u32;
    let rows = sums
        .into_iter()
        .map(|(kind, (n, h, r))| (kind, (n, 100.0 * h / n as f64, 100.0 * r / n as f64)))
        .collect();
    Table {
        rows,
        none_empty,
        none_total,
        none_mean: none_results as f64 / none_total.max(1) as f64,
        per_query,
    }
}

fn print(name: &str, table: &Table) {
    eprintln!("\n### {name} ({:?} per query)", table.per_query);
    eprintln!("type          n  hit@5  rec@10");
    let mut kinds: Vec<&String> = table.rows.keys().filter(|k| *k != "ALL").collect();
    kinds.push(table.rows.keys().find(|k| *k == "ALL").unwrap());
    for kind in kinds {
        let (n, h, r) = table.rows[kind];
        eprintln!("{kind:<11} {n:>4}  {h:>5.0}  {r:>6.0}");
    }
    eprintln!(
        "none       {:>4}  correctly empty {}/{}, mean results {:.0}",
        table.none_total, table.none_empty, table.none_total, table.none_mean
    );
}

/// The index as downloaded, and the same index with the optional per-mod keys (`l`, `n`, `a`)
/// filled from the review's GameBanana corpus, as the server will send them.
fn load() -> Option<(Vec<Case>, CatalogIndex, Option<CatalogIndex>)> {
    let dir = eval_dir();
    let cases: Vec<Case> = serde_json::from_slice(&fs::read(dir.join("evalset.json")).ok()?).ok()?;
    let body = fs::read(dir.join("data/index.json")).ok()?;
    let index = parse(&body).ok()?;
    let with_keys = fs::read(dir.join("data/corpus.json")).ok().and_then(|corpus| {
        let corpus: Vec<serde_json::Value> = serde_json::from_slice(&corpus).ok()?;
        let side: std::collections::HashMap<u64, &serde_json::Value> =
            corpus.iter().filter_map(|m| Some((m["id"].as_u64()?, m))).collect();
        let mut raw: serde_json::Value = serde_json::from_slice(&body).ok()?;
        for m in raw["mods"].as_array_mut()? {
            let Some(extra) = m["i"].as_u64().and_then(|id| side.get(&id)) else { continue };
            m["l"] = extra["likes"].clone();
            m["a"] = extra["added"].clone();
            if extra["rated"].as_bool() == Some(true) {
                m["n"] = 1.into();
            }
        }
        parse(&serde_json::to_vec(&raw).ok()?).ok()
    });
    Some((cases, index, with_keys))
}

/// Mod-kind words the desktop app derives from its German labels (`browse::search::kind_words`,
/// whose test checks it still yields these).
pub(super) const APP_KIND_WORDS: [(&str, &str); 3] = [("fadenkreuz", "crosshair"), ("karte", "map"), ("karten", "maps")];

/// Floors a little under the measured numbers (hit@5, recall@10 per type, in percent).
const FLOORS: [(&str, f64, f64); 13] = [
    ("creator", 95.0, 95.0),
    ("franchise", 95.0, 75.0),
    ("hero", 95.0, 95.0),
    ("implied", 68.0, 50.0),
    ("kind", 95.0, 78.0),
    ("multi", 90.0, 80.0),
    ("nickname", 85.0, 80.0),
    ("nonenglish", 80.0, 65.0),
    ("partial", 95.0, 80.0),
    ("style", 95.0, 82.0),
    ("title", 95.0, 95.0),
    ("typo", 85.0, 78.0),
    ("ALL", 91.0, 79.0),
];

fn check(table: &Table) {
    for (kind, hit5, recall10) in FLOORS {
        let (_, h, r) = table.rows[kind];
        assert!(h >= hit5 && r >= recall10, "{kind}: hit@5 {h:.0} (floor {hit5}), recall@10 {r:.0} (floor {recall10})");
    }
    // A search that should find nothing mostly shows the empty state.
    assert!(table.none_empty >= 8 && table.none_mean <= 12.0, "no-answer queries: {}/{} empty, mean {:.0}", table.none_empty, table.none_total, table.none_mean);
}

fn matcher_for(index: &CatalogIndex) -> Matcher {
    let mut matcher = Matcher::new(index);
    matcher.set_kind_words(APP_KIND_WORDS.iter().map(|(a, b)| (a.to_string(), b.to_string())));
    matcher
}

#[test]
#[ignore]
fn search_eval() {
    let Some((cases, index, with_keys)) = load() else {
        eprintln!("no evaluation data at {}", eval_dir().display());
        return;
    };
    let start = Instant::now();
    let matcher = matcher_for(&index);
    eprintln!("prepared {} mods in {:?}", matcher.len(), start.elapsed());
    let mut run = |q: &str| matcher.search(q, 1_000).hits.into_iter().map(|hit| hit.id).collect();
    let table = score(&cases, &mut run);
    print("index as downloaded", &table);
    if let Some(keyed) = with_keys {
        let matcher = matcher_for(&keyed);
        let mut run = |q: &str| matcher.search(q, 1_000).hits.into_iter().map(|hit| hit.id).collect();
        print("with likes, rating and added keys", &score(&cases, &mut run));
    }
    // One keystroke: the slowest prefix of every query, as typed.
    let mut slowest = Duration::ZERO;
    let mut sum = Duration::ZERO;
    let mut count = 0_u32;
    for case in &cases {
        for end in (1..=case.q.chars().count()).map(|k| case.q.char_indices().nth(k).map_or(case.q.len(), |(i, _)| i)) {
            let start = Instant::now();
            let _ = matcher.search(&case.q[..end], 1_000);
            let took = start.elapsed();
            slowest = slowest.max(took);
            sum += took;
            count += 1;
        }
    }
    eprintln!("keystrokes: {count}, mean {:?}, slowest {slowest:?}", sum / count.max(1));
    check(&table);
}
