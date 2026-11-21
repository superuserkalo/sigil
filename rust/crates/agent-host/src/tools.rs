//! The Deadlock tool surface. Every tool calls `dah-rs` except
//! `validate_payload`, which is a pure path check. Nothing here writes to a
//! game install, launches a process outside `dah-rs`, or reaches a network
//! host directly.

use std::sync::Arc;

use serde_json::{Value, json};
use tiny_agent::{Error, Result, Tool, ToolContext, ToolDefinition, ToolReturn};

use crate::{dah_rs::DahRsCli, payload};

pub fn deadlock_tools(cli: Arc<DahRsCli>) -> Result<Vec<Tool>> {
    Ok(vec![
        game_status(cli.clone())?,
        game_index(cli.clone())?,
        search_paths(cli.clone())?,
        extract_paths(cli.clone())?,
        project_check(cli.clone())?,
        project_validate(cli.clone())?,
        project_pack(cli.clone())?,
        project_release(cli.clone())?,
        deploy_plan(cli.clone())?,
        project_migrate(cli.clone())?,
        watch_projects(cli.clone())?,
        publish_prepare(cli.clone())?,
        patch_notes(cli.clone())?,
        player_stats(cli.clone())?,
        player_heatmap(cli.clone())?,
        hero_performance(cli.clone())?,
        launch_status(cli.clone())?,
        toolchain_status(cli.clone())?,
        demo_matches(cli.clone())?,
        demo_import(cli.clone())?,
        demo_review_prompt(cli.clone())?,
        demo_review_write(cli.clone())?,
        install_status(cli.clone())?,
        install_plan(cli.clone())?,
        profiles_list(cli.clone())?,
        locker_heroes(cli.clone())?,
        recolor_brief(cli.clone())?,
        game_textures(cli.clone())?,
        game_sounds(cli.clone())?,
        files_list(cli.clone())?,
        files_show(cli.clone())?,
        files_search(cli.clone())?,
        files_compare(cli.clone())?,
        files_export(cli.clone())?,
        validate_payload()?,
    ])
}

fn definition(name: &str, description: &str, parameters: Value) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: description.to_string(),
        parameters,
    }
}

fn empty_parameters() -> Value {
    json!({"type": "object", "properties": {}, "additionalProperties": false})
}

fn object_schema(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

/// A tool that runs one `dah-rs` call with fixed or argument-derived words and
/// returns its JSON. The argument builder never receives a shell string.
fn cli_tool(
    cli: Arc<DahRsCli>,
    name: &'static str,
    description: &'static str,
    parameters: Value,
    idempotent: bool,
    build: fn(&Value) -> Result<Vec<String>>,
) -> Result<Tool> {
    Tool::builder(definition(name, description, parameters))
        .idempotent(idempotent)
        .timeout_seconds(150)
        .handler(move |arguments: Value, context: ToolContext| {
            let cli = cli.clone();
            async move {
                let args = build(&arguments)?;
                let text = cli.json(&args, &context.cancellation).await?;
                Ok(ToolReturn::Output(text))
            }
        })
}

fn game_status(cli: Arc<DahRsCli>) -> Result<Tool> {
    Tool::builder(definition(
        "game_status",
        "Report the Deadlock install, the gameinfo.gi SearchPaths status and the addon slots in use. Read-only.",
        empty_parameters(),
    ))
    .idempotent(true)
    .timeout_seconds(150)
    .handler(move |_arguments: Value, context: ToolContext| {
        let cli = cli.clone();
        async move {
            let args = vec!["doctor".to_string(), "--json".to_string()];
            let text = cli.json(&args, &context.cancellation).await?;
            Ok(ToolReturn::Output(text))
        }
    })
}

fn game_index(cli: Arc<DahRsCli>) -> Result<Tool> {
    Tool::builder(definition(
        "game_index",
        "Build or load the cached index of game VPK paths. Reports entry counts, sources and any change since the previous index. Read-only toward the game install.",
        object_schema(
            json!({
                "refresh": {
                    "type": ["boolean", "null"],
                    "description": "Rebuild the index even when the cache matches the game files. Pass null to keep the cache.",
                },
            }),
            &["refresh"],
        ),
    ))
    .idempotent(false)
    .timeout_seconds(150)
    .handler(move |arguments: Value, context: ToolContext| {
        let cli = cli.clone();
        async move {
            let refresh = arguments
                .get("refresh")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let mut args = vec!["index".to_string()];
            if refresh {
                args.push("--refresh".to_string());
            }
            args.push("--json".to_string());
            let text = cli.json(&args, &context.cancellation).await?;
            Ok(ToolReturn::Output(text))
        }
    })
}

fn search_paths(cli: Arc<DahRsCli>) -> Result<Tool> {
    Tool::builder(definition(
        "search_paths",
        "Search the game index for exact game paths with sizes. Filter by text, hero codename, compiled file type or folder. Read-only.",
        object_schema(
            json!({
                "query": {
                    "type": ["string", "null"],
                    "description": "Case-insensitive text that must appear in the path. Pass null with a filter.",
                },
                "hero": {
                    "type": ["string", "null"],
                    "description": "Hero codename, for example billy or punkgoat. Pass null to skip.",
                },
                "type": {
                    "type": ["string", "null"],
                    "description": "Compiled extension, for example vmdl_c or vtex_c. Pass null to skip.",
                },
                "folder": {
                    "type": ["string", "null"],
                    "description": "Path prefix, for example models/heroes_wip. Pass null to skip.",
                },
                "limit": {
                    "type": ["integer", "null"],
                    "minimum": 1,
                    "maximum": 1000,
                    "description": "Maximum paths to return. Pass null for the default of 50.",
                },
            }),
            &["query", "hero", "type", "folder", "limit"],
        ),
    ))
    .idempotent(true)
    .timeout_seconds(150)
    .handler(move |arguments: Value, context: ToolContext| {
        let cli = cli.clone();
        async move {
            let query = optional_string(arguments.get("query"));
            let hero = optional_string(arguments.get("hero"));
            let file_type = optional_string(arguments.get("type"));
            let folder = optional_string(arguments.get("folder"));

            if query.is_none() && hero.is_none() && file_type.is_none() && folder.is_none() {
                return Err(Error::Tool(
                    "Provide query, hero, type or folder. At least one filter is required.".into(),
                ));
            }

            let mut args = vec!["find".to_string(), query.unwrap_or_default()];
            if let Some(hero) = hero {
                args.push("--hero".to_string());
                args.push(hero);
            }
            if let Some(file_type) = file_type {
                args.push("--type".to_string());
                args.push(file_type);
            }
            if let Some(folder) = folder {
                args.push("--folder".to_string());
                args.push(folder);
            }
            if let Some(limit) = arguments.get("limit").and_then(Value::as_u64) {
                args.push("--limit".to_string());
                args.push(limit.to_string());
            }
            args.push("--json".to_string());

            let text = cli.json(&args, &context.cancellation).await?;
            Ok(ToolReturn::Output(text))
        }
    })
}

fn extract_paths(cli: Arc<DahRsCli>) -> Result<Tool> {
    Tool::builder(definition(
        "extract_paths",
        "Extract exact game paths from the game VPK into the mod workspace. It writes inside the workspace only and keeps compiled files compiled. Not a game install write.",
        object_schema(
            json!({
                "paths": {
                    "type": "array",
                    "items": {"type": "string"},
                    "minItems": 1,
                    "description": "Exact game paths from search_paths.",
                },
                "destination": {
                    "type": ["string", "null"],
                    "description": "Folder inside the mod workspace. Pass null for `extracted`, with game paths preserved below it.",
                },
                "overwrite": {
                    "type": ["boolean", "null"],
                    "description": "Replace files that already exist. Pass null or false to keep them.",
                },
            }),
            &["paths", "destination", "overwrite"],
        ),
    ))
    .idempotent(false)
    .timeout_seconds(150)
    .handler(move |arguments: Value, context: ToolContext| {
        let cli = cli.clone();
        async move {
            let paths = string_list(arguments.get("paths"), "paths")?;
            if paths.is_empty() {
                return Err(Error::Tool("Pass at least one exact game path.".into()));
            }
            let destination = optional_string(arguments.get("destination"));
            let overwrite = arguments
                .get("overwrite")
                .and_then(Value::as_bool)
                .unwrap_or(false);

            let mut args = vec!["extract".to_string()];
            args.extend(paths);
            if let Some(destination) = destination {
                args.push("--out".to_string());
                args.push(destination);
            }
            if overwrite {
                args.push("--overwrite".to_string());
            }
            args.push("--json".to_string());

            let text = cli.json(&args, &context.cancellation).await?;
            Ok(ToolReturn::Output(text))
        }
    })
}

fn project_check(cli: Arc<DahRsCli>) -> Result<Tool> {
    Tool::builder(definition(
        "project_check",
        "Check the current mod project against the current game files: changed, missing and moved paths, gameinfo.gi and the addon slot. Read-only.",
        empty_parameters(),
    ))
    .idempotent(true)
    .timeout_seconds(150)
    .handler(move |_arguments: Value, context: ToolContext| {
        let cli = cli.clone();
        async move {
            let args = vec![
                "mod".to_string(),
                "check".to_string(),
                "--json".to_string(),
            ];
            let text = cli.json(&args, &context.cancellation).await?;
            Ok(ToolReturn::Output(text))
        }
    })
}

fn project_validate(cli: Arc<DahRsCli>) -> Result<Tool> {
    cli_tool(
        cli,
        "project_validate",
        "Check the project staging tree against the payload rules without packing. Read-only. Reports the project, the staging folder, the entry count and every error and warning.",
        empty_parameters(),
        true,
        |_arguments| {
            Ok(vec![
                "mod".to_string(),
                "validate".to_string(),
                "--json".to_string(),
            ])
        },
    )
}

fn project_pack(cli: Arc<DahRsCli>) -> Result<Tool> {
    cli_tool(
        cli,
        "project_pack",
        "Validate the staging tree and pack it as pakNN_dir.vpk in the project builds folder. The addon slot comes from project.json unless slot is passed. Writes inside the mod workspace only.",
        object_schema(
            json!({
                "slot": {
                    "type": ["integer", "null"],
                    "minimum": 1,
                    "maximum": 99,
                    "description": "Addon slot 1 to 99. Pass null to use the project slot, else 01.",
                },
            }),
            &["slot"],
        ),
        false,
        |arguments| {
            let mut args = vec!["mod".to_string(), "pack".to_string()];
            if let Some(slot) = slot_argument(arguments)? {
                args.push("--slot".to_string());
                args.push(slot.to_string());
            }
            args.push("--json".to_string());
            Ok(args)
        },
    )
}

fn project_release(cli: Arc<DahRsCli>) -> Result<Tool> {
    cli_tool(
        cli,
        "project_release",
        "Pack the staging tree into builds/<version>/, write release.json and record the release in project history. Bump the version with the project first. Writes inside the mod workspace only.",
        object_schema(
            json!({
                "slot": {
                    "type": ["integer", "null"],
                    "minimum": 1,
                    "maximum": 99,
                    "description": "Addon slot 1 to 99. Pass null to use the project slot, else 01.",
                },
                "note": {
                    "type": ["string", "null"],
                    "description": "Note stored in the release manifest and project history. Pass null for none.",
                },
            }),
            &["slot", "note"],
        ),
        false,
        |arguments| {
            let mut args = vec!["mod".to_string(), "release".to_string()];
            if let Some(slot) = slot_argument(arguments)? {
                args.push("--slot".to_string());
                args.push(slot.to_string());
            }
            if let Some(note) = optional_string(arguments.get("note")) {
                args.push("--note".to_string());
                args.push(note);
            }
            args.push("--json".to_string());
            Ok(args)
        },
    )
}

fn deploy_plan(cli: Arc<DahRsCli>) -> Result<Tool> {
    cli_tool(
        cli,
        "deploy_plan",
        "Plan an install of the latest release into game/citadel/addons: target slot, overlapping paths, gameinfo.gi state and whether a replace is required. Read-only, it never writes to the game install and never approves a deploy.",
        empty_parameters(),
        true,
        |_arguments| {
            Ok(vec![
                "mod".to_string(),
                "deploy".to_string(),
                "--json".to_string(),
            ])
        },
    )
}

fn project_migrate(cli: Arc<DahRsCli>) -> Result<Tool> {
    cli_tool(
        cli,
        "project_migrate",
        "Plan target paths that moved after a game update, with the exact new path for each one. Read-only: project.json is not changed, because applying a migration is an app action.",
        empty_parameters(),
        true,
        |_arguments| {
            Ok(vec![
                "mod".to_string(),
                "migrate".to_string(),
                "--json".to_string(),
            ])
        },
    )
}

fn watch_projects(cli: Arc<DahRsCli>) -> Result<Tool> {
    cli_tool(
        cli,
        "watch_projects",
        "One read-only pass over every mod project in the workspace as a review queue after a game update. Reports changed, missing, moved and unbaselined targets, slot conflicts, gameinfo.gi state and planned migrations.",
        object_schema(
            json!({
                "limit": {
                    "type": ["integer", "null"],
                    "minimum": 1,
                    "maximum": 200,
                    "description": "Maximum projects to report. Pass null for the default of 20.",
                },
            }),
            &["limit"],
        ),
        true,
        |arguments| {
            let mut args = vec!["watch".to_string()];
            if let Some(limit) = arguments.get("limit").and_then(Value::as_u64) {
                args.push("--limit".to_string());
                args.push(limit.to_string());
            }
            args.push("--json".to_string());
            Ok(args)
        },
    )
}

fn publish_prepare(cli: Arc<DahRsCli>) -> Result<Tool> {
    cli_tool(
        cli,
        "publish_prepare",
        "Copy the latest release, its metadata.json and install notes into a publish folder inside the workspace. It never uploads anything, because uploading is an app action behind approval.",
        empty_parameters(),
        false,
        |_arguments| {
            Ok(vec![
                "publish".to_string(),
                "prepare".to_string(),
                "--json".to_string(),
            ])
        },
    )
}

fn patch_notes(cli: Arc<DahRsCli>) -> Result<Tool> {
    Tool::builder(definition(
        "patch_notes",
        "Fetch or read cached official Deadlock patch posts from Steam. Reports titles, dates and which posts are new. Does not replace the game index or project check.",
        object_schema(
            json!({
                "limit": {
                    "type": ["integer", "null"],
                    "minimum": 1,
                    "maximum": 50,
                    "description": "Maximum posts to return. Pass null for the default of 10.",
                },
                "refresh": {
                    "type": ["boolean", "null"],
                    "description": "Fetch from Steam even when the cache is fresh. Pass null to use the cache.",
                },
            }),
            &["limit", "refresh"],
        ),
    ))
    .idempotent(false)
    .timeout_seconds(150)
    .handler(move |arguments: Value, context: ToolContext| {
        let cli = cli.clone();
        async move {
            let mut args = vec!["patches".to_string()];
            if let Some(limit) = arguments.get("limit").and_then(Value::as_u64) {
                args.push("--limit".to_string());
                args.push(limit.to_string());
            }
            if arguments
                .get("refresh")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                args.push("--refresh".to_string());
            }
            args.push("--json".to_string());

            let text = cli.json(&args, &context.cancellation).await?;
            Ok(ToolReturn::Output(text))
        }
    })
}

fn validate_payload() -> Result<Tool> {
    Tool::builder(definition(
        "validate_payload",
        "Validate a proposed VPK entry list against the Deadlock payload rules before packing. Pure function, no file access. Reports errors and warnings.",
        object_schema(
            json!({
                "entries": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "VPK entry paths, relative to the game root.",
                },
                "stagingFolder": {
                    "type": ["string", "null"],
                    "description": "Staging folder name that must not appear as the first path segment. Pass null to skip.",
                },
            }),
            &["entries", "stagingFolder"],
        ),
    ))
    .idempotent(true)
    .handler(move |arguments: Value, _context: ToolContext| async move {
        let entries = string_list(arguments.get("entries"), "entries")?;
        let staging = optional_string(arguments.get("stagingFolder"));
        let issues = payload::validate(&entries, staging.as_deref());
        let ok = !payload::has_errors(&issues);
        Ok(ToolReturn::Output(
            json!({"ok": ok, "issues": issues}).to_string(),
        ))
    })
}

fn optional_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn slot_argument(arguments: &Value) -> Result<Option<u8>> {
    let Some(value) = arguments.get("slot") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let slot = value
        .as_u64()
        .filter(|slot| (1..=99).contains(slot))
        .ok_or_else(|| Error::Tool("slot must be an integer from 1 to 99.".into()))?;
    Ok(Some(slot as u8))
}

fn string_list(value: Option<&Value>, name: &str) -> Result<Vec<String>> {
    let Some(Value::Array(items)) = value else {
        return Err(Error::Tool(format!("{name} must be an array of strings.")));
    };
    items
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_string)
                .ok_or_else(|| Error::Tool(format!("{name} must be an array of strings.")))
        })
        .collect()
}

fn player_stats(cli: Arc<DahRsCli>) -> Result<Tool> {
    Tool::builder(definition(
        "player_stats",
        "Report a Deadlock player's rank, percentile, career totals, recent form and top heroes from the public Deadlock API. Read-only and keyless. Numbers are computed from the API payload; never invent a statistic that is absent.",
        object_schema(
            json!({
                "account_id": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Steam account id. Ask the user when unknown; do not guess.",
                },
                "recent": {
                    "type": ["integer", "null"],
                    "minimum": 1,
                    "maximum": 100,
                    "description": "Newest matches to include in the window. Pass null for 20.",
                },
            }),
            &["account_id", "recent"],
        ),
    ))
    .idempotent(true)
    .timeout_seconds(150)
    .handler(move |arguments: Value, context: ToolContext| {
        let cli = cli.clone();
        async move {
            let account = arguments
                .get("account_id")
                .and_then(Value::as_u64)
                .ok_or_else(|| Error::Tool("account_id is required.".into()))?;
            let mut args = vec![
                "stats".to_string(),
                "player".to_string(),
                account.to_string(),
                "--json".to_string(),
            ];
            if let Some(recent) = arguments.get("recent").and_then(Value::as_u64) {
                args.push("--recent".to_string());
                args.push(recent.to_string());
            }
            let text = cli.json(&args, &context.cancellation).await?;
            Ok(ToolReturn::Output(text))
        }
    })
}

fn player_heatmap(cli: Arc<DahRsCli>) -> Result<Tool> {
    cli_tool(
        cli,
        "player_heatmap",
        "Report aggregate kill and death map cells for a player over the API's 30 day window. The cells are real positions; without a returned cell, say the data is absent instead of describing a pattern.",
        object_schema(
            json!({
                "account_id": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Steam account id.",
                },
            }),
            &["account_id"],
        ),
        true,
        |arguments| {
            let account = arguments
                .get("account_id")
                .and_then(Value::as_u64)
                .ok_or_else(|| Error::Tool("account_id is required.".into()))?;
            Ok(vec![
                "stats".to_string(),
                "heatmap".to_string(),
                account.to_string(),
                "--json".to_string(),
            ])
        },
    )
}

fn hero_performance(cli: Arc<DahRsCli>) -> Result<Tool> {
    cli_tool(
        cli,
        "hero_performance",
        "Report per-hero career rows for a player, sorted by sample size, with Wilson intervals for win rate. Read-only.",
        object_schema(
            json!({
                "account_id": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Steam account id.",
                },
            }),
            &["account_id"],
        ),
        true,
        |arguments| {
            let account = arguments
                .get("account_id")
                .and_then(Value::as_u64)
                .ok_or_else(|| Error::Tool("account_id is required.".into()))?;
            Ok(vec![
                "stats".to_string(),
                "heroes".to_string(),
                account.to_string(),
                "--json".to_string(),
            ])
        },
    )
}

fn launch_status(cli: Arc<DahRsCli>) -> Result<Tool> {
    cli_tool(
        cli,
        "launch_status",
        "Report whether a vanilla stash is pending and where it lives. Launching and restoring are app actions behind approval and are not available as agent tools.",
        empty_parameters(),
        true,
        |_arguments| {
            Ok(vec![
                "launch".to_string(),
                "status".to_string(),
                "--json".to_string(),
            ])
        },
    )
}

fn toolchain_status(cli: Arc<DahRsCli>) -> Result<Tool> {
    cli_tool(
        cli,
        "toolchain_status",
        "Report which external decompile and compile tools are available, with the path, version and exact reason when one is missing. Nothing is downloaded.",
        empty_parameters(),
        true,
        |_arguments| {
            Ok(vec![
                "toolchain".to_string(),
                "status".to_string(),
                "--json".to_string(),
            ])
        },
    )
}

fn demo_matches(cli: Arc<DahRsCli>) -> Result<Tool> {
    cli_tool(
        cli,
        "demo_matches",
        "List locally imported Deadlock match demos with source, map, duration, event count and heatmap cell count. Read-only. A match with zero cells has no real position samples; never describe movement for it.",
        empty_parameters(),
        true,
        |_arguments| {
            Ok(vec![
                "demo".to_string(),
                "list".to_string(),
                "--json".to_string(),
            ])
        },
    )
}

fn demo_import(cli: Arc<DahRsCli>) -> Result<Tool> {
    Tool::builder(definition(
        "demo_import",
        "Copy one Source 2 .dem file into the local match store, parse its container header and provenance, and write a summary. It refuses a file without the PBDEMS2 magic. It never uploads the demo.",
        object_schema(
            json!({
                "path": {
                    "type": "string",
                    "description": "Absolute path to a .dem file on this machine.",
                },
                "match_id": {
                    "type": ["string", "null"],
                    "description": "Optional match id. Pass null to derive one.",
                },
            }),
            &["path", "match_id"],
        ),
    ))
    .idempotent(false)
    .timeout_seconds(150)
    .handler(move |arguments: Value, context: ToolContext| {
        let cli = cli.clone();
        async move {
            let path = arguments
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Tool("path is required.".into()))?;
            let mut args = vec![
                "demo".to_string(),
                "import".to_string(),
                path.to_string(),
                "--json".to_string(),
            ];
            if let Some(id) = arguments.get("match_id").and_then(Value::as_str)
                && !id.trim().is_empty()
            {
                args.push("--id".to_string());
                args.push(id.to_string());
            }
            let text = cli.json(&args, &context.cancellation).await?;
            Ok(ToolReturn::Output(text))
        }
    })
}

fn install_status(cli: Arc<DahRsCli>) -> Result<Tool> {
    cli_tool(
        cli,
        "install_status",
        "List install journal entries from the Browse pipeline and whether each file is still present in the addons folder. Read-only.",
        empty_parameters(),
        true,
        |_arguments| {
            Ok(vec![
                "install".to_string(),
                "list".to_string(),
                "--json".to_string(),
            ])
        },
    )
}

fn install_plan(cli: Arc<DahRsCli>) -> Result<Tool> {
    Tool::builder(definition(
        "install_plan",
        "Download a GameBanana mod or archive into the workspace, extract it, validate the payload and plan a free addon slot. It writes only inside the workspace. Applying the plan to the game is an app action behind approval and is not available to the agent.",
        object_schema(
            json!({
                "target": {
                    "type": "string",
                    "description": "GameBanana mod id, mod URL or https archive URL.",
                },
                "slot": {
                    "type": ["integer", "null"],
                    "minimum": 1,
                    "maximum": 99,
                    "description": "Requested slot. Pass null for the lowest free slot.",
                },
                "force": {
                    "type": ["boolean", "null"],
                    "description": "Allow replacing an occupied slot. Pass null for false.",
                },
            }),
            &["target", "slot", "force"],
        ),
    ))
    .idempotent(false)
    .timeout_seconds(300)
    .handler(move |arguments: Value, context: ToolContext| {
        let cli = cli.clone();
        async move {
            let target = arguments
                .get("target")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Tool("target is required.".into()))?;
            let mut args = vec![
                "install".to_string(),
                "plan".to_string(),
                target.to_string(),
                "--json".to_string(),
            ];
            if let Some(slot) = arguments.get("slot").and_then(Value::as_u64) {
                args.push("--slot".to_string());
                args.push(slot.to_string());
            }
            if arguments.get("force").and_then(Value::as_bool) == Some(true) {
                args.push("--force".to_string());
            }
            let text = cli.json(&args, &context.cancellation).await?;
            Ok(ToolReturn::Output(text))
        }
    })
}

fn profiles_list(cli: Arc<DahRsCli>) -> Result<Tool> {
    cli_tool(
        cli,
        "profiles_list",
        "List saved addon slot profiles in the workspace with their enabled and total slot counts. Read-only. Applying a profile changes files in the game install and is an app action, not a tool.",
        empty_parameters(),
        true,
        |_arguments| {
            Ok(vec![
                "profiles".to_string(),
                "list".to_string(),
                "--json".to_string(),
            ])
        },
    )
}

fn demo_review_prompt(cli: Arc<DahRsCli>) -> Result<Tool> {
    cli_tool(
        cli,
        "demo_review_prompt",
        "Return the verified review prompt and fact set for one imported match. Use only those facts in a review. The prompt names every absent field. No provider is called by this tool.",
        object_schema(
            json!({
                "match_id": {
                    "type": "string",
                    "description": "Match id from demo_matches.",
                },
            }),
            &["match_id"],
        ),
        true,
        |arguments| {
            let id = arguments
                .get("match_id")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Tool("match_id is required.".into()))?;
            Ok(vec![
                "demo".to_string(),
                "review".to_string(),
                id.to_string(),
                "--json".to_string(),
            ])
        },
    )
}

fn demo_review_write(cli: Arc<DahRsCli>) -> Result<Tool> {
    Tool::builder(definition(
        "demo_review_write",
        "Lint a match review against the verified facts and store review.md only when every number is supported. It refuses invented statistics and never writes the game install.",
        object_schema(
            json!({
                "match_id": {
                    "type": "string",
                    "description": "Match id from demo_matches.",
                },
                "body": {
                    "type": "string",
                    "description": "The Markdown review. Cite only facts from demo_review_prompt.",
                },
            }),
            &["match_id", "body"],
        ),
    ))
    .idempotent(false)
    .timeout_seconds(60)
    .handler(move |arguments: Value, context: ToolContext| {
        let cli = cli.clone();
        async move {
            let id = arguments
                .get("match_id")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Tool("match_id is required.".into()))?;
            let body = arguments
                .get("body")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Tool("body is required.".into()))?;
            let path = std::env::temp_dir().join(format!("dah-review-{id}-{}.md", std::process::id()));
            std::fs::write(&path, body)
                .map_err(|error| Error::Tool(format!("Could not stage the review: {error}")))?;
            let args = vec![
                "demo".to_string(),
                "review".to_string(),
                id.to_string(),
                "--file".to_string(),
                path.display().to_string(),
                "--write".to_string(),
                "--json".to_string(),
            ];
            let result = cli.json(&args, &context.cancellation).await;
            let _ = std::fs::remove_file(&path);
            let text = result?;
            Ok(ToolReturn::Output(text))
        }
    })
}

fn locker_heroes(cli: Arc<DahRsCli>) -> Result<Tool> {
    cli_tool(
        cli,
        "locker_heroes",
        "List heroes with local art, catalog path counts, installed mod counts and recolorable material paths. Read-only. An art-only hero has no game path to edit.",
        object_schema(
            json!({
                "modded": {
                    "type": ["boolean", "null"],
                    "description": "Only heroes touched by an installed addon. Pass null for every hero.",
                },
            }),
            &["modded"],
        ),
        true,
        |arguments| {
            let mut args = vec!["locker".to_string(), "list".to_string(), "--json".to_string()];
            if arguments.get("modded").and_then(Value::as_bool) == Some(true) {
                args.push("--modded".to_string());
            }
            Ok(args)
        },
    )
}

fn recolor_brief(cli: Arc<DahRsCli>) -> Result<Tool> {
    Tool::builder(definition(
        "recolor_brief",
        "Record a recolor work order for one hero in the workspace: exact catalog targets, a palette and the steps still required. It does not edit an asset and never writes the game install. It refuses a hero with no catalog material.",
        object_schema(
            json!({
                "hero": {
                    "type": "string",
                    "description": "Hero codename or display name from locker_heroes.",
                },
                "palette": {
                    "type": ["string", "null"],
                    "description": "ember, frost, toxic or void. Pass null for ember.",
                },
                "hue": {
                    "type": ["integer", "null"],
                    "minimum": -180,
                    "maximum": 180,
                    "description": "Custom hue shift in degrees. Pass null to use the preset.",
                },
                "note": {
                    "type": ["string", "null"],
                    "description": "Why the recolor exists. Pass null for none.",
                },
            }),
            &["hero", "palette", "hue", "note"],
        ),
    ))
    .idempotent(false)
    .timeout_seconds(60)
    .handler(move |arguments: Value, context: ToolContext| {
        let cli = cli.clone();
        async move {
            let hero = arguments
                .get("hero")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Tool("hero is required.".into()))?;
            let mut args = vec![
                "locker".to_string(),
                "recolor".to_string(),
                hero.to_string(),
                "--json".to_string(),
            ];
            if let Some(palette) = arguments.get("palette").and_then(Value::as_str) {
                args.push("--palette".to_string());
                args.push(palette.to_string());
            }
            if let Some(hue) = arguments.get("hue").and_then(Value::as_i64) {
                args.push("--hue".to_string());
                args.push(hue.to_string());
            }
            if let Some(note) = arguments.get("note").and_then(Value::as_str)
                && !note.trim().is_empty()
            {
                args.push("--note".to_string());
                args.push(note.to_string());
            }
            let text = cli.json(&args, &context.cancellation).await?;
            Ok(ToolReturn::Output(text))
        }
    })
}

// ----- game art, sounds and the file explorer ------------------------------------------------

const SOURCE_DESCRIPTION: &str = "game (citadel pak01), core, an installed addon file such as pak03_dir.vpk, or a path inside the workspace such as mods/<slug>/staging.";

fn required_string(arguments: &Value, name: &str) -> Result<String> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| Error::Tool(format!("{name} is required.")))
}

fn push_option(args: &mut Vec<String>, arguments: &Value, name: &str, flag: &str) {
    match arguments.get(name) {
        Some(Value::String(text)) if !text.trim().is_empty() => {
            args.push(flag.to_string());
            args.push(text.trim().to_string());
        }
        Some(Value::Number(number)) => {
            args.push(flag.to_string());
            args.push(number.to_string());
        }
        _ => {}
    }
}

fn game_textures_args(arguments: &Value) -> Result<Vec<String>> {
    let mut args = vec!["locker".to_string(), "textures".to_string(), "--json".to_string()];
    push_option(&mut args, arguments, "hero", "--hero");
    push_option(&mut args, arguments, "kind", "--kind");
    push_option(&mut args, arguments, "search", "--search");
    push_option(&mut args, arguments, "limit", "--limit");
    Ok(args)
}

fn game_textures(cli: Arc<DahRsCli>) -> Result<Tool> {
    cli_tool(
        cli,
        "game_textures",
        "List the game's textures (.vtex_c) with exact paths, kind and hero: hero cards, portraits, ability and item icons, skin textures and ability effects. Read-only; reads the pak directory, no pixels.",
        object_schema(
            json!({
                "hero": {"type": ["string", "null"], "description": "Hero display name or codename. Pass null for every hero."},
                "kind": {"type": ["string", "null"], "enum": ["card", "portrait", "ability", "item", "skin", "effect", "other", null], "description": "Texture kind. Pass null for every kind."},
                "search": {"type": ["string", "null"], "description": "Words that must all appear in the label or path. Pass null to skip."},
                "limit": {"type": ["integer", "null"], "minimum": 1, "maximum": 2000, "description": "Pass null for 200."},
            }),
            &["hero", "kind", "search", "limit"],
        ),
        true,
        game_textures_args,
    )
}

fn game_sounds_args(arguments: &Value) -> Result<Vec<String>> {
    let mut args = vec!["locker".to_string(), "sounds".to_string(), "--json".to_string()];
    push_option(&mut args, arguments, "hero", "--hero");
    push_option(&mut args, arguments, "slot", "--slot");
    push_option(&mut args, arguments, "search", "--search");
    push_option(&mut args, arguments, "limit", "--limit");
    Ok(args)
}

fn game_sounds(cli: Arc<DahRsCli>) -> Result<Tool> {
    cli_tool(
        cli,
        "game_sounds",
        "List the game's sound events for one hero (event name, ability, slot, clip paths, soundevents file), or with hero null the heroes that have sounds. Read-only toward the game; the first call indexes the soundevents into the workspace cache.",
        object_schema(
            json!({
                "hero": {"type": ["string", "null"], "description": "Hero display name or codename. Pass null to list the heroes."},
                "slot": {"type": ["integer", "null"], "minimum": 1, "maximum": 4, "description": "Ability slot. Pass null for every sound."},
                "search": {"type": ["string", "null"], "description": "Words that must all appear in the label, event or ability. Pass null to skip."},
                "limit": {"type": ["integer", "null"], "minimum": 1, "maximum": 2000, "description": "Pass null for 150."},
            }),
            &["hero", "slot", "search", "limit"],
        ),
        true,
        game_sounds_args,
    )
}

fn files_list_args(arguments: &Value) -> Result<Vec<String>> {
    let mut args = vec!["files".to_string(), "list".to_string(), required_string(arguments, "source")?];
    push_option(&mut args, arguments, "folder", "--folder");
    args.push("--json".to_string());
    Ok(args)
}

fn files_list(cli: Arc<DahRsCli>) -> Result<Tool> {
    cli_tool(
        cli,
        "files_list",
        "List one folder level of a VPK or workspace folder: subfolders with file counts and sizes, then files. Read-only.",
        object_schema(
            json!({
                "source": {"type": "string", "description": SOURCE_DESCRIPTION},
                "folder": {"type": ["string", "null"], "description": "Folder inside the source, for example scripts. Pass null for the root."},
            }),
            &["source", "folder"],
        ),
        true,
        files_list_args,
    )
}

fn files_show_args(arguments: &Value) -> Result<Vec<String>> {
    Ok(vec![
        "files".to_string(),
        "show".to_string(),
        required_string(arguments, "source")?,
        required_string(arguments, "path")?,
        "--json".to_string(),
    ])
}

fn files_show(cli: Arc<DahRsCli>) -> Result<Tool> {
    cli_tool(
        cli,
        "files_show",
        "Preview one file as text: source text, compiled KV3 data (vdata, vsndevts, vpcf) as KV3 text, compiled Panorama layouts and styles, a text DMX summary, or a hex dump. Lists the resource's references. Read-only; capped at 24 KB.",
        object_schema(
            json!({
                "source": {"type": "string", "description": SOURCE_DESCRIPTION},
                "path": {"type": "string", "description": "Exact file path inside the source."},
            }),
            &["source", "path"],
        ),
        true,
        files_show_args,
    )
}

fn files_search_args(arguments: &Value) -> Result<Vec<String>> {
    let mut args = vec![
        "files".to_string(),
        "search".to_string(),
        required_string(arguments, "source")?,
        required_string(arguments, "query")?,
    ];
    push_option(&mut args, arguments, "under", "--under");
    push_option(&mut args, arguments, "max_hits", "--max-hits");
    if arguments.get("case_sensitive").and_then(Value::as_bool) == Some(true) {
        args.push("--case-sensitive".to_string());
    }
    args.push("--json".to_string());
    Ok(args)
}

fn files_search(cli: Arc<DahRsCli>) -> Result<Tool> {
    cli_tool(
        cli,
        "files_search",
        "Search the contents of every text and compiled data file in a source, decoded first, and return matching lines. Read-only. Narrow a game-wide search with under.",
        object_schema(
            json!({
                "source": {"type": "string", "description": SOURCE_DESCRIPTION},
                "query": {"type": "string", "description": "Text to find."},
                "under": {"type": ["string", "null"], "description": "Only paths containing this text, for example scripts/abilities. Pass null for every file."},
                "case_sensitive": {"type": ["boolean", "null"], "description": "Pass null for a case-insensitive search."},
                "max_hits": {"type": ["integer", "null"], "minimum": 1, "maximum": 200, "description": "Pass null for 60."},
            }),
            &["source", "query", "under", "case_sensitive", "max_hits"],
        ),
        true,
        files_search_args,
    )
}

fn files_compare_args(arguments: &Value) -> Result<Vec<String>> {
    let mut args = vec![
        "files".to_string(),
        "compare".to_string(),
        required_string(arguments, "left")?,
        required_string(arguments, "right")?,
    ];
    push_option(&mut args, arguments, "path", "--path");
    args.push("--json".to_string());
    Ok(args)
}

fn files_compare(cli: Arc<DahRsCli>) -> Result<Tool> {
    cli_tool(
        cli,
        "files_compare",
        "Compare two sources: files only one side has and files that differ. With path, the changed lines of that file's decoded text. Read-only.",
        object_schema(
            json!({
                "left": {"type": "string", "description": SOURCE_DESCRIPTION},
                "right": {"type": "string", "description": SOURCE_DESCRIPTION},
                "path": {"type": ["string", "null"], "description": "One file to diff line by line. Pass null to compare the trees."},
            }),
            &["left", "right", "path"],
        ),
        true,
        files_compare_args,
    )
}

fn files_export_args(arguments: &Value) -> Result<Vec<String>> {
    let mut args = vec!["files".to_string(), "export".to_string(), required_string(arguments, "source")?];
    let paths = string_list(arguments.get("paths"), "paths")?;
    if paths.is_empty() {
        return Err(Error::Tool("paths needs at least one file or folder.".into()));
    }
    args.extend(paths);
    if arguments.get("editable").and_then(Value::as_bool) == Some(true) {
        args.push("--editable".to_string());
    }
    if arguments.get("overwrite").and_then(Value::as_bool) == Some(true) {
        args.push("--overwrite".to_string());
    }
    args.push("--json".to_string());
    Ok(args)
}

fn files_export(cli: Arc<DahRsCli>) -> Result<Tool> {
    cli_tool(
        cli,
        "files_export",
        "Copy files or folders from a source into the current mod project: originals into staging/ (they replace that exact game path) or, with editable, decoded text into sources/. Writes only inside the mod project, never the game install.",
        object_schema(
            json!({
                "source": {"type": "string", "description": SOURCE_DESCRIPTION},
                "paths": {"type": "array", "items": {"type": "string"}, "minItems": 1, "maxItems": 200, "description": "Files or folders inside the source."},
                "editable": {"type": ["boolean", "null"], "description": "Export a decoded text copy into sources/. Pass null for the original bytes."},
                "overwrite": {"type": ["boolean", "null"], "description": "Replace files the project already has. Pass null to keep them."},
            }),
            &["source", "paths", "editable", "overwrite"],
        ),
        false,
        files_export_args,
    )
}

#[cfg(test)]
mod contract_tests {
    use super::*;

    fn tools() -> Vec<Tool> {
        let cli = Arc::new(DahRsCli::new(
            Some("/opt/dah".to_string()),
            crate::CliLaunch::SelfCli,
            std::path::PathBuf::from("."),
            std::path::PathBuf::from("."),
        ));
        deadlock_tools(cli).expect("tools build")
    }

    #[test]
    fn asset_and_file_tools_are_reads_except_export() {
        let tools = tools();
        let idempotent = |name: &str| {
            tools
                .iter()
                .find(|tool| tool.name() == name)
                .unwrap_or_else(|| panic!("{name} is registered"))
                .idempotent()
        };
        for name in ["game_textures", "game_sounds", "files_list", "files_show", "files_search", "files_compare"] {
            assert!(idempotent(name), "{name} reads only");
        }
        assert!(!idempotent("files_export"), "export writes into the project");
        for tool in &tools {
            let schema = tool.definition().parameters;
            assert_eq!(schema["type"], "object", "{} has an object schema", tool.name());
            assert_eq!(schema["additionalProperties"], false, "{} rejects extra fields", tool.name());
        }
    }

    #[test]
    fn file_tools_build_argument_arrays_not_shell_strings() {
        let args = files_search_args(&json!({
            "source": "game", "query": "MaxHealth; rm -rf /", "under": "scripts", "case_sensitive": true, "max_hits": 5,
        }))
        .unwrap();
        assert_eq!(
            args,
            ["files", "search", "game", "MaxHealth; rm -rf /", "--under", "scripts", "--max-hits", "5", "--case-sensitive", "--json"]
        );
        assert!(files_show_args(&json!({"source": "game", "path": ""})).is_err());
        assert!(files_export_args(&json!({"source": "game", "paths": []})).is_err());
        let export = files_export_args(&json!({"source": "pak03_dir.vpk", "paths": ["sounds/x.vsnd_c"], "editable": null, "overwrite": true})).unwrap();
        assert_eq!(export, ["files", "export", "pak03_dir.vpk", "sounds/x.vsnd_c", "--overwrite", "--json"]);
        // The export never names a destination: dah-rs resolves the mod project itself.
        assert!(!export.iter().any(|word| word.contains("addons") || word == "--project"));
        let sounds = game_sounds_args(&json!({"hero": "Haze", "slot": 2, "search": null, "limit": null})).unwrap();
        assert_eq!(sounds, ["locker", "sounds", "--json", "--hero", "Haze", "--slot", "2"]);
        let textures = game_textures_args(&json!({"hero": null, "kind": "skin", "search": null, "limit": 20})).unwrap();
        assert_eq!(textures, ["locker", "textures", "--json", "--kind", "skin", "--limit", "20"]);
    }
}
