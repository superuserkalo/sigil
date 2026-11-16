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
