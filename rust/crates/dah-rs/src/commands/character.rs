//! `dah character ...`: create a generation task from a prompt or reference
//! image, poll it, and import the downloaded GLB into a character workspace.
//!
//! This module calls `deadlock_core::character` directly. The `character`
//! subcommand and `pub mod character;` are wired by the CLI change; nothing
//! here touches `main.rs`.

use std::path::{Path, PathBuf};

use serde_json::json;

use deadlock_core::character::{
    self, CharacterImport, CharacterProviderKind, CharacterRequest, CharacterStatus,
    SOURCE2_COMPILE_REQUIRED,
};

use crate::error::{CliError, Result};
use crate::paths;
use crate::util::{format_bytes, print_json};

pub struct CreateArgs {
    pub prompt: String,
    pub reference: Option<PathBuf>,
    pub target_hero: Option<String>,
    pub style: Option<String>,
    pub provider: Option<String>,
    pub rights_note: Option<String>,
    pub allow_reference_upload: bool,
    pub workspace: Option<PathBuf>,
    pub json: bool,
}

pub struct StatusArgs {
    pub workspace: Option<PathBuf>,
    pub slug: Option<String>,
    pub refresh: bool,
    pub limit: usize,
    pub json: bool,
}

pub struct ImportArgs {
    pub slug: String,
    pub workspace: Option<PathBuf>,
    pub json: bool,
}

fn provider_kind(value: Option<&str>) -> Result<CharacterProviderKind> {
    let value = value.unwrap_or("meshy");
    CharacterProviderKind::parse(value).ok_or_else(|| {
        CliError::new(
            "invalid_provider",
            format!("Unknown character provider {value:?}. Use `meshy` or `mock`."),
        )
    })
}

fn map_core(context: &'static str, error: deadlock_core::Error) -> CliError {
    CliError::new(context, error.to_string())
}

pub fn create(args: &CreateArgs) -> Result<()> {
    if args.prompt.trim().is_empty() {
        return Err(CliError::new(
            "empty_prompt",
            "Character prompt must not be empty.",
        ));
    }

    let workspace = paths::workspace_root(args.workspace.as_deref())?;
    let mut request = CharacterRequest::new(
        args.prompt.clone(),
        provider_kind(args.provider.as_deref())?,
    );

    request.reference_image = args.reference.clone();
    request.target_hero = args
        .target_hero
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    request.allow_reference_upload = args.allow_reference_upload;

    if let Some(style) = args
        .style
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        request.style = style.to_string();
    }

    if let Some(note) = args
        .rights_note
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        request.rights_note = note.to_string();
    }

    if request.reference_image.is_some()
        && !request.allow_reference_upload
        && request.provider == CharacterProviderKind::Meshy
    {
        eprintln!(
            "The reference image stays local. Pass --allow-reference-upload to send it to the \
             provider, or use --provider mock."
        );
    }

    let record = character::create_character(&workspace, &request)
        .map_err(|error| map_core("character_create", error))?;

    let directory = character::character_dir(&workspace, &record.slug);

    if args.json {
        print_json(&json!({
            "ok": true,
            "slug": record.slug,
            "taskId": record.task.id,
            "status": record.task.status,
            "progress": record.task.progress,
            "stage": record.task.stage,
            "provider": record.request.provider,
            "targetHero": record.request.target_hero,
            "directory": directory.to_string_lossy(),
            "taskPath": character::task_path(&workspace, &record.slug).to_string_lossy(),
            "source2Compile": SOURCE2_COMPILE_REQUIRED,
            "inGameReady": false,
            "rights": record.rights,
        }));
        return Ok(());
    }

    println!("Character task {} created: {}", record.task.id, record.slug);
    println!(
        "  Provider: {} ({})",
        record.request.provider.as_str(),
        record.task.stage
    );
    println!(
        "  Status:   {} {}%",
        record.task.status, record.task.progress
    );
    if let Some(hero) = &record.request.target_hero {
        println!("  Hero:     {hero}");
    }
    println!("  Folder:   {}", directory.display());
    println!();
    println!("The mesh is not a Deadlock hero model. Source 2 compile is required.");
    println!(
        "Next: dah character status --refresh --slug {}",
        record.slug
    );

    Ok(())
}

pub fn status(args: &StatusArgs) -> Result<()> {
    if args.limit == 0 || args.limit > 200 {
        return Err(CliError::new(
            "invalid_limit",
            "Limit must be from 1 to 200 character tasks.",
        ));
    }

    let workspace = paths::workspace_root(args.workspace.as_deref())?;

    if args.refresh {
        refresh_tasks(&workspace, args.slug.as_deref())?;
    }

    let mut tasks = character::character_status(&workspace)
        .map_err(|error| map_core("character_status", error))?;

    if let Some(slug) = args.slug.as_deref() {
        tasks.retain(|task| task.slug == slug);
        if tasks.is_empty() {
            return Err(CliError::new(
                "character_not_found",
                format!(
                    "No character task {slug} under {}. Run `dah character create` first.",
                    character::characters_dir(&workspace).display()
                ),
            ));
        }
    }

    tasks.truncate(args.limit);

    if args.json {
        print_json(&json!({
            "ok": true,
            "workspace": workspace.to_string_lossy(),
            "count": tasks.len(),
            "tasks": tasks,
        }));
        return Ok(());
    }

    if tasks.is_empty() {
        println!(
            "No character tasks under {}.",
            character::characters_dir(&workspace).display()
        );
        println!("Run `dah character create --prompt \"...\"` to start one.");
        return Ok(());
    }

    println!(
        "Character tasks under {}:",
        character::characters_dir(&workspace).display()
    );

    for task in &tasks {
        let asset = match &task.asset {
            Some(asset) => format!(", {} GLB", format_bytes(asset.bytes)),
            None => String::new(),
        };
        println!(
            "  {:<40} {:<12} {:>3}%  {}{}",
            task.slug,
            task.task.status.as_str(),
            task.task.progress,
            task.updated_at,
            asset
        );
        println!(
            "    task {} ({})",
            task.task.id,
            task.request.provider.as_str()
        );
        if let Some(error) = &task.task.error {
            println!("    error: {error}");
        }
        if task.task.status == CharacterStatus::Succeeded {
            println!("    next: dah character import {}", task.slug);
        }
    }

    Ok(())
}

fn refresh_tasks(workspace: &Path, slug: Option<&str>) -> Result<()> {
    let targets = match slug {
        Some(slug) => vec![slug.to_string()],
        None => {
            let tasks = character::character_status(workspace)
                .map_err(|error| map_core("character_status", error))?;
            tasks
                .into_iter()
                .filter(|task| !task.task.status.is_final())
                .map(|task| task.slug)
                .collect()
        }
    };

    for slug in targets {
        character::refresh_character(workspace, &slug)
            .map_err(|error| map_core("character_refresh", error))?;
    }

    Ok(())
}

pub fn import(args: &ImportArgs) -> Result<()> {
    character::validate_slug(&args.slug)
        .map_err(|error| CliError::new("invalid_slug", error.to_string()))?;

    let workspace = paths::workspace_root(args.workspace.as_deref())?;
    let import = character::import_character(&workspace, &args.slug)
        .map_err(|error| map_core("character_import", error))?;
    let path = character::character_manifest_path(&workspace, &args.slug);

    if args.json {
        print_json(&json!({
            "ok": true,
            "path": path.to_string_lossy(),
            "character": import,
        }));
        return Ok(());
    }

    print_import(&import, &path);
    Ok(())
}

fn print_import(import: &CharacterImport, path: &Path) {
    println!("Imported character {} to {}", import.slug, path.display());
    println!("  Prompt:   {}", import.prompt);
    println!("  Style:    {}", import.style);
    println!("  Provider: {}", import.provider);
    if let Some(hero) = &import.target_hero {
        println!("  Hero:     {hero}");
    }
    println!(
        "  GLB:      {} ({} bytes, crc32 {})",
        import.asset.path, import.asset.bytes, import.asset.crc32
    );
    println!("  Task:     {}", import.task_id);
    println!();
    println!("In game ready: no");
    println!("Source 2 compile: {}", import.source2_compile);
    println!("Still required:");
    for requirement in &import.compile_requirements {
        println!("  - {requirement}");
    }
    println!();
    for note in &import.notes {
        println!("{note}");
    }
    println!();
    println!("This record never claims the character loads in Deadlock.");
}
