//! `dah find`: search the cached game catalog.

use serde_json::{json, Value};

use crate::domain;
use crate::error::Result;
use crate::paths::CliPaths;
use crate::util::print_json;

pub struct FindArgs {
    pub query: String,
    pub hero: Option<String>,
    pub extension: Option<String>,
    pub folder: Option<String>,
    pub vpk: Option<String>,
    pub limit: usize,
    pub json: bool,
}

pub fn run(paths: &CliPaths, args: &FindArgs) -> Result<()> {
    let limit = args.limit.clamp(1, 1000);
    let load = domain::load_catalog(&paths.workspace_dir, false)?;
    let options = domain::SearchGameCatalogOptions {
        query: Some(args.query.clone()),
        hero: args.hero.clone(),
        extension: args.extension.clone(),
        folder: args.folder.clone(),
        vpk: args.vpk.clone(),
        limit: Some(limit),
    };
    let result = domain::search_game_catalog(&load.loaded.catalog, options);

    if args.json {
        print_json(&render_json(&result, args, limit));
        return Ok(());
    }

    if result.total == 0 {
        println!("No matching paths in the game catalog.");
        return Ok(());
    }

    let mut filters: Vec<String> = Vec::new();
    if !args.query.is_empty() {
        filters.push(format!("query {:?}", args.query));
    }
    if let Some(hero) = &args.hero {
        filters.push(format!("hero {hero}"));
    }
    if let Some(extension) = &args.extension {
        filters.push(format!("type {extension}"));
    }
    if let Some(folder) = &args.folder {
        filters.push(format!("folder {folder}"));
    }
    if let Some(vpk) = &args.vpk {
        filters.push(format!("vpk {vpk}"));
    }

    if filters.is_empty() {
        println!("Showing {} of {} paths", result.entries.len(), result.total);
    } else {
        println!(
            "Showing {} of {} paths matching {}",
            result.entries.len(),
            result.total,
            filters.join(", ")
        );
    }

    for entry in &result.entries {
        println!(
            "{}: {} ({} bytes)",
            entry.vpk,
            entry.path,
            entry.total_length()
        );
    }

    if result.truncated {
        println!("Output truncated. Narrow the query or raise --limit.");
    }

    Ok(())
}

fn render_json(result: &domain::SearchGameCatalogResult, args: &FindArgs, limit: usize) -> Value {
    json!({
        "ok": true,
        "total": result.total,
        "returned": result.entries.len(),
        "truncated": result.truncated,
        "limit": limit,
        "filters": {
            "query": args.query,
            "hero": args.hero,
            "type": args.extension,
            "folder": args.folder,
            "vpk": args.vpk,
        },
        "entries": result
            .entries
            .iter()
            .map(|entry| json!({
                "vpk": entry.vpk,
                "path": entry.path,
                "bytes": entry.total_length(),
                "archiveIndex": entry.archive_index,
            }))
            .collect::<Vec<_>>(),
    })
}
