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
        patch_notes(cli.clone())?,
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
