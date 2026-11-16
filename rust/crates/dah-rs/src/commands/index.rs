//! `dah index`: build or reuse the game catalog and report what changed.

use serde_json::{json, Value};

use crate::domain;
use crate::error::Result;
use crate::paths::CliPaths;
use crate::util::print_json;

pub struct IndexArgs {
    pub refresh: bool,
    pub json: bool,
}

pub fn run(paths: &CliPaths, args: &IndexArgs) -> Result<()> {
    let load = domain::load_catalog(&paths.workspace_dir, args.refresh)?;
    let catalog = &load.loaded.catalog;
    let total = catalog.entries.len();
    let diff = diff_catalog(&load.loaded);
    let cache = domain::cache_dir(&paths.workspace_dir);
    let cache_reused = !load.loaded.meta.is_empty()
        && load.loaded.meta.values().all(|meta| meta.from_cache);

    if args.json {
        print_json(&render_json(catalog, &load.loaded, total, &diff, cache_reused));
        return Ok(());
    }

    println!(
        "Game catalog: {} VPK(s), {} entries",
        catalog.sources.len(),
        total
    );

    for source in &catalog.sources {
        println!("  {}: {} entries ({})", source.id, source.entries, source.path);
    }

    for error in &load.loaded.errors {
        println!("  {}: skipped ({})", error.id, error.message);
    }

    println!("Cache: {}", cache.display());

    match diff {
        None if cache_reused => println!("Index diff: cache reused, nothing rebuilt."),
        None => println!("Index diff: no previous cache, this build is the baseline."),
        Some(source_diffs) if source_diffs.is_empty() => {
            println!("Index diff: no path changes since the previous cache.");
        }
        Some(source_diffs) => {
            println!("Index diff since the previous cache:");
            for source in source_diffs {
                println!(
                    "  {}: +{} added, -{} removed, {} changed, {} moved",
                    source.id, source.added, source.removed, source.changed, source.moved
                );
                for path in &source.added_sample {
                    println!("    + {path}");
                }
                for path in &source.removed_sample {
                    println!("    - {path}");
                }
                for moved in &source.moved_sample {
                    println!("    > {} -> {}", moved.from, moved.to);
                }
                if source.added > source.added_sample.len()
                    || source.removed > source.removed_sample.len()
                    || source.moved > source.moved_sample.len()
                {
                    println!("    (samples truncated)");
                }
            }
        }
    }

    Ok(())
}

pub struct SourceDiff {
    pub id: String,
    pub added: usize,
    pub removed: usize,
    pub changed: usize,
    pub moved: usize,
    pub added_sample: Vec<String>,
    pub removed_sample: Vec<String>,
    pub moved_sample: Vec<MoveSample>,
}

pub struct MoveSample {
    pub from: String,
    pub to: String,
}

const SAMPLE_LIMIT: usize = 5;

/// Compare the per-VPK indexes the core kept from the previous build with the
/// freshly loaded ones.
pub fn diff_catalog(loaded: &domain::LoadGameCatalogResult) -> Option<Vec<SourceDiff>> {
    if loaded.previous.is_empty() {
        return None;
    }

    let mut diffs = Vec::new();

    for (id, previous) in &loaded.previous {
        let diff = match loaded.indexes.get(id) {
            Some(current) => domain::diff_game_index(previous, current),
            None => domain::IndexDiff {
                removed: previous.entries.clone(),
                ..domain::IndexDiff::default()
            },
        };

        if diff.is_empty() {
            continue;
        }

        diffs.push(SourceDiff {
            id: id.clone(),
            added: diff.added.len(),
            removed: diff.removed.len(),
            changed: diff.changed.len(),
            moved: diff.moved.len(),
            added_sample: diff
                .added
                .iter()
                .take(SAMPLE_LIMIT)
                .map(|entry| entry.path.clone())
                .collect(),
            removed_sample: diff
                .removed
                .iter()
                .take(SAMPLE_LIMIT)
                .map(|entry| entry.path.clone())
                .collect(),
            moved_sample: diff
                .moved
                .iter()
                .take(SAMPLE_LIMIT)
                .map(|moved| MoveSample {
                    from: moved.from.clone(),
                    to: moved.to.clone(),
                })
                .collect(),
        });
    }

    diffs.sort_by(|left, right| left.id.cmp(&right.id));
    Some(diffs)
}

fn render_json(
    catalog: &domain::GameCatalog,
    loaded: &domain::LoadGameCatalogResult,
    total: usize,
    diff: &Option<Vec<SourceDiff>>,
    cache_reused: bool,
) -> Value {
    json!({
        "ok": true,
        "builtAt": catalog.built_at,
        "totalEntries": total,
        "cacheReused": cache_reused,
        "sources": catalog
            .sources
            .iter()
            .map(|source| json!({
                "id": source.id,
                "path": source.path,
                "entries": source.entries,
                "size": source.size,
                "mtimeMs": source.mtime_ms,
            }))
            .collect::<Vec<_>>(),
        "errors": loaded
            .errors
            .iter()
            .map(|error| json!({
                "id": error.id,
                "message": error.message,
            }))
            .collect::<Vec<_>>(),
        "diff": diff.as_ref().map(|sources| json!({
            "sources": sources
                .iter()
                .map(|source| json!({
                    "id": source.id,
                    "added": source.added,
                    "removed": source.removed,
                    "changed": source.changed,
                    "moved": source.moved,
                    "addedSample": source.added_sample,
                    "removedSample": source.removed_sample,
                    "movedSample": source
                        .moved_sample
                        .iter()
                        .map(|moved| json!({
                            "from": moved.from,
                            "to": moved.to,
                        }))
                        .collect::<Vec<_>>(),
                }))
                .collect::<Vec<_>>(),
        })),
    })
}
