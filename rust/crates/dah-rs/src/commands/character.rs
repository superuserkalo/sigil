//! `dah character ...`: create a stage-graph character task from a prompt or a
//! reference image, poll an asynchronous backend, print the per-stage graph,
//! and import the mesh into a character workspace.
//!
//! The command calls `deadlock_core::character` directly. Deterministic stages
//! need no key and no model. A generative stage with no configured backend is
//! reported as `unsupported`, never as success.

use std::path::{Path, PathBuf};

use serde_json::json;

use deadlock_core::character::{
    self, CharacterImport, CharacterProviderKind, CharacterRequest, CharacterStatus, MeshReport,
    Source2Plan, StageRecord, StageState, SOURCE2_COMPILE_REQUIRED,
};

use crate::error::{CliError, Result};
use crate::paths;
use crate::util::{format_bytes, lossy, print_json};

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

pub struct StagesArgs {
    /// Character slug. Without one, print the canonical graph and its rules.
    pub slug: Option<String>,
    pub workspace: Option<PathBuf>,
    pub json: bool,
}

/// Parse `--provider`. A value with a `local:` or `http:` prefix names an
/// explicit backend id, so the provider kind and the backend override come from
/// the same flag.
fn provider_kind(value: Option<&str>) -> Result<(CharacterProviderKind, Option<String>)> {
    let value = value.map(str::trim).filter(|value| !value.is_empty());

    let Some(value) = value else {
        return Ok((CharacterProviderKind::Auto, None));
    };

    if value.starts_with("local:") || value.starts_with("http:") {
        let kind = if value.starts_with("local:") {
            CharacterProviderKind::Local
        } else {
            CharacterProviderKind::Http
        };
        return Ok((kind, Some(value.to_string())));
    }

    let kind = CharacterProviderKind::parse(value).ok_or_else(|| {
        CliError::new(
            "invalid_provider",
            format!(
                "Unknown character backend {value:?}. Use `auto`, `local`, `http`, `meshy`, \
                 `mock` or `none`, or a `local:<stages>` or `http:<url>` backend id."
            ),
        )
    })?;

    Ok((kind, None))
}

fn map_core(context: &'static str, error: deadlock_core::Error) -> CliError {
    CliError::new(context, error.to_string())
}

pub fn create(args: &CreateArgs) -> Result<()> {
    if args.prompt.trim().is_empty() && args.reference.is_none() {
        return Err(CliError::new(
            "empty_request",
            "Character request needs a prompt or --reference image.",
        ));
    }

    let workspace = paths::workspace_root(args.workspace.as_deref())?;
    let (provider, backend) = provider_kind(args.provider.as_deref())?;
    let mut request = CharacterRequest::new(args.prompt.clone(), provider);
    request.backend = backend;

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

    if request.reference_image.is_some() && !request.allow_reference_upload {
        eprintln!(
            "The reference image stays local. Only a remote provider needs \
             --allow-reference-upload; a local backend reads the file in place."
        );
    }

    let record =
        character::create_character(&workspace, &request).map_err(|e| map_core("character_create", e))?;

    let directory = character::character_dir(&workspace, &record.slug);
    let stages = record.stages();
    let summary = stage_summary(&stages);
    let reference = record.reference.clone();
    let mesh = record.mesh.clone();

    if args.json {
        print_json(&json!({
            "ok": true,
            "slug": record.slug,
            "taskId": record.task.id,
            "status": record.task.status,
            "progress": record.task.progress,
            "stage": record.task.stage,
            "provider": record.request.provider,
            "backend": record.backend,
            "targetHero": record.request.target_hero,
            "directory": directory.to_string_lossy(),
            "taskPath": character::task_path(&workspace, &record.slug).to_string_lossy(),
            "source2Compile": SOURCE2_COMPILE_REQUIRED,
            "inGameReady": false,
            "rights": record.rights,
            "reference": reference,
            "mesh": mesh,
            "summary": {
                "total": stages.len(),
                "succeeded": summary.succeeded,
                "skipped": summary.skipped,
                "unsupported": summary.unsupported,
                "failed": summary.failed,
                "pending": summary.pending,
            },
            "stages": stages,
        }));
        return Ok(());
    }

    println!("Character task {} recorded: {}", record.task.id, record.slug);
    println!("  Backend:  {}", record.backend);
    print_stage_rows(&stages);
    println!();
    println!(
        "  {} of {} stages succeeded, {} skipped, {} unsupported, {} failed, {} pending",
        summary.succeeded,
        stages.len(),
        summary.skipped,
        summary.unsupported,
        summary.failed,
        summary.pending
    );

    if let Some(reference) = &reference {
        println!(
            "  Reference: {}x{} {}, alpha channel: {}",
            reference.width,
            reference.height,
            reference.format,
            if reference.has_alpha_channel { "yes" } else { "no" }
        );
    }

    if let Some(mesh) = &mesh {
        println!("  Mesh:      {}", describe_mesh(mesh));
    }

    println!("  Folder:    {}", directory.display());
    println!();

    match record.task.status {
        CharacterStatus::Blocked => {
            println!(
                "No generative backend is configured, so nothing was generated. That is not a \
                 failure and it is not a mesh."
            );
            println!(
                "Configure DAH_MESHGEN_COMMAND or DAH_MESHGEN_URL, or pass --provider meshy, then \
                 run: dah character status --refresh --slug {}",
                record.slug
            );
        }
        CharacterStatus::Pending | CharacterStatus::InProgress => {
            println!(
                "Next: dah character status --refresh --slug {}",
                record.slug
            );
        }
        _ => {
            println!("Next: dah character import {}", record.slug);
        }
    }

    println!("Source 2 compile is required. No stage here claims a game-ready model.");
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

    let mut tasks =
        character::character_status(&workspace).map_err(|e| map_core("character_status", e))?;

    if let Some(slug) = args.slug.as_deref() {
        tasks.retain(|task| task.slug == slug);
        if tasks.is_empty() {
            return Err(CliError::new(
                "character_not_found",
                format!(
                    "No character task {slug} under {}. Run `Sigil cli character create` first.",
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
        println!("Run `Sigil cli character create --reference image.png` to start one.");
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
        println!("    task {} (backend {})", task.task.id, task.backend);

        let stages = task.stages();
        let summary = stage_summary(&stages);
        println!(
            "    stages: {} succeeded, {} skipped, {} unsupported, {} failed, {} pending",
            summary.succeeded, summary.skipped, summary.unsupported, summary.failed, summary.pending
        );

        if let Some(mesh) = &task.mesh {
            println!("    mesh: {}", describe_mesh(mesh));
        }
        if let Some(error) = &task.task.error {
            println!("    note: {error}");
        }
        if task.task.status == CharacterStatus::Succeeded {
            println!("    next: dah character import {}", task.slug);
        } else if task.task.status == CharacterStatus::Blocked {
            println!("    next: dah character stages {}", task.slug);
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

/// Print the stage graph. Without a slug this is the canonical definition; with
/// a slug it is the recorded state, including the reason each stage is
/// unsupported.
pub fn stages(args: &StagesArgs) -> Result<()> {
    let workspace = paths::workspace_root(args.workspace.as_deref())?;

    let (records, task) = match args.slug.as_deref() {
        Some(slug) => {
            let record = character::read_task_record(&workspace, slug)
                .map_err(|error| map_core("character_stages", error))?;
            (record.stages(), Some(record))
        }
        None => (
            character::stage_graph()
                .into_iter()
                .map(|spec| {
                    let mut record = StageRecord {
                        id: spec.stage,
                        state: StageState::Pending,
                        backend: "-".to_string(),
                        inputs: Vec::new(),
                        outputs: Vec::new(),
                        bytes: None,
                        hash: None,
                        notes: Vec::new(),
                        evidence: spec.evidence.iter().map(|id| (*id).to_string()).collect(),
                        task_id: None,
                        credits: None,
                        updated_at: String::new(),
                    };
                    record.notes.push(spec.title.to_string());
                    record
                })
                .collect(),
            None,
        ),
    };

    if args.json {
        print_json(&json!({
            "ok": true,
            "slug": args.slug,
            "task": task.as_ref().map(|record| &record.task),
            "backend": task.as_ref().map(|record| record.backend.clone()),
            "stages": records,
        }));
        return Ok(());
    }

    match &task {
        Some(record) => {
            println!(
                "Stage graph for {} (task {}, backend {}):",
                record.slug, record.task.id, record.backend
            );
        }
        None => {
            println!("Character stage graph (canonical order):");
        }
    }

    print_stage_rows(&records);
    println!();

    if task.is_none() {
        println!(
            "Deterministic stages run with no key and no model: {}.",
            deterministic_stages()
        );
        println!(
            "Generative stages need DAH_MESHGEN_COMMAND (a local executable), DAH_MESHGEN_URL (a \
             JSON model server) or --provider meshy. Without one they are `unsupported`, with a \
             reason, never skipped silently."
        );
        return Ok(());
    }

    let summary = stage_summary(&records);
    println!(
        "  {} of {} stages succeeded, {} skipped, {} unsupported, {} failed, {} pending",
        summary.succeeded,
        records.len(),
        summary.skipped,
        summary.unsupported,
        summary.failed,
        summary.pending
    );

    // Only print notes for stages that carry more than a refusal boilerplate.
    for record in &records {
        if record.state == StageState::Unsupported && !record.notes.is_empty() {
            println!();
            println!("  {} [{}]", record.id.id(), record.state);
            for note in &record.notes {
                println!("    {note}");
            }
        }
    }

    println!();
    println!("Source 2 compile: {SOURCE2_COMPILE_REQUIRED}. In game ready: no.");
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
    println!("  Backend:  {}", import.backend);
    if let Some(hero) = &import.target_hero {
        println!("  Hero:     {hero}");
    }
    println!(
        "  GLB:      {} ({} bytes, crc32 {})",
        import.asset.path, import.asset.bytes, import.asset.crc32
    );
    println!("  Task:     {}", import.task_id);

    if let Some(mesh) = &import.mesh {
        println!("  Mesh:     {}", describe_mesh(mesh));
        for problem in &mesh.problems {
            println!("    problem: {problem}");
        }
    }

    println!();
    print_stage_rows(&import.stages);

    if let Some(plan) = &import.source2_plan {
        println!();
        print_source2_plan(plan);
    }

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

fn print_source2_plan(plan: &Source2Plan) {
    println!(
        "Source 2 plan (hero {}), {} artifact(s):",
        plan.hero.as_deref().unwrap_or("<none>"),
        plan.artifacts.len()
    );
    for artifact in &plan.artifacts {
        let compiled = if artifact.compiled.is_empty() {
            "-".to_string()
        } else {
            artifact.compiled.clone()
        };
        println!(
            "  {:<10} {:<52} -> {}{}",
            artifact.kind,
            if artifact.source.is_empty() {
                "(from the game)".to_string()
            } else {
                artifact.source.clone()
            },
            compiled,
            if artifact.shipped_by_mod {
                ""
            } else {
                "  (not shipped)"
            }
        );
    }
    println!("  skeleton: {}", plan.skeleton.skeleton);
    println!("  animgraph: {}", plan.anim_graph.graph);
}

fn print_stage_rows(records: &[StageRecord]) {
    for record in records {
        let bytes = record
            .bytes
            .map(|bytes| format!("  {}", format_bytes(bytes)))
            .unwrap_or_default();
        println!(
            "  {:<20} {:<12} {}{}",
            record.id.id(),
            record.state.as_str(),
            record.backend,
            bytes
        );
    }
}

fn deterministic_stages() -> String {
    character::stage_graph()
        .into_iter()
        .filter(|spec| !spec.generative)
        .map(|spec| spec.id)
        .collect::<Vec<_>>()
        .join(", ")
}

struct StageSummary {
    succeeded: usize,
    skipped: usize,
    unsupported: usize,
    failed: usize,
    pending: usize,
}

fn stage_summary(records: &[StageRecord]) -> StageSummary {
    let mut summary = StageSummary {
        succeeded: 0,
        skipped: 0,
        unsupported: 0,
        failed: 0,
        pending: 0,
    };

    for record in records {
        match record.state {
            StageState::Succeeded => summary.succeeded += 1,
            StageState::Skipped => summary.skipped += 1,
            StageState::Unsupported => summary.unsupported += 1,
            StageState::Failed => summary.failed += 1,
            StageState::Pending | StageState::Running => summary.pending += 1,
        }
    }

    summary
}

fn describe_mesh(mesh: &MeshReport) -> String {
    let bounds = match mesh.bounds_size() {
        Some(size) => format!(", bounds {:.2}x{:.2}x{:.2}", size[0], size[1], size[2]),
        None => String::new(),
    };

    let attributes = [
        ("NORMAL", mesh.has_normals),
        ("TEXCOORD_0", mesh.has_uvs),
        ("TANGENT", mesh.has_tangents),
        ("skin", mesh.has_skin),
    ]
    .into_iter()
    .filter_map(|(name, present)| present.then_some(name))
    .collect::<Vec<_>>()
    .join("+");

    let attributes = if attributes.is_empty() {
        "no normals, UVs or skin".to_string()
    } else {
        attributes
    };

    format!(
        "{} triangles, {} vertices, {} primitive(s), {} material(s), {} texture(s), {}{}",
        mesh.triangles,
        mesh.vertices,
        mesh.primitives,
        mesh.materials,
        mesh.textures,
        attributes,
        bounds
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn temp_workspace(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "dah-cli-character-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    // The `stages` clap arm is owned by the main.rs change. Until it lands, the
    // command body is exercised through the module so the printed graph is real
    // output from the same function.
    #[test]
    fn stages_prints_the_canonical_graph() {
        let workspace = temp_workspace("graph");
        stages(&StagesArgs {
            slug: None,
            workspace: Some(workspace.clone()),
            json: false,
        })
        .unwrap();
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn stages_prints_a_recorded_task() {
        // `DAH_CHARACTER_TEST_WORKSPACE` plus `DAH_CHARACTER_TEST_SLUG` point
        // this at a real workspace so the printed graph can be inspected.
        if let (Ok(workspace), Ok(slug)) = (
            std::env::var("DAH_CHARACTER_TEST_WORKSPACE"),
            std::env::var("DAH_CHARACTER_TEST_SLUG"),
        ) {
            stages(&StagesArgs {
                slug: Some(slug),
                workspace: Some(PathBuf::from(workspace)),
                json: false,
            })
            .unwrap();
            return;
        }

        let workspace = temp_workspace("recorded");
        let mut request =
            CharacterRequest::new("a stage graph test character", CharacterProviderKind::Auto);
        request.backend = Some("none".to_string());
        let record = character::create_character(&workspace, &request).unwrap();

        stages(&StagesArgs {
            slug: Some(record.slug.clone()),
            workspace: Some(workspace.clone()),
            json: false,
        })
        .unwrap();

        let _ = fs::remove_dir_all(workspace);
    }
}

pub struct BackendsArgs {
    pub json: bool,
}

pub struct ScaffoldArgs {
    pub slug: String,
    pub force: bool,
    pub workspace: Option<PathBuf>,
    pub json: bool,
}

/// `Sigil cli character backends`: what this machine can run, and how to set it up.
pub fn backends(args: &BackendsArgs) -> Result<()> {
    let options = character::backend_options();

    if args.json {
        print_json(&json!({
            "ok": true,
            "platform": std::env::consts::OS,
            "backends": options,
            "note": "An LLM can write a Blender or Python script for the blender-code lane. Image or text to 3D is a separate trained model, local or hosted.",
        }));
        return Ok(());
    }

    println!("Mesh backends on {}", std::env::consts::OS);
    for option in &options {
        println!(
            "  {:<16} {:<12} {}",
            option.id,
            if option.available { "available" } else { "unavailable" },
            option.kind
        );
        println!("    {}", option.reason);
        if option.available {
            println!("    stages: {}", option.stages.join(", "));
        }
        println!("    setup:  {}", option.setup);
    }
    println!();
    println!("An LLM cannot emit a production mesh as tokens, but it can write and iterate Blender code.");
    println!("Run `Sigil cli character scaffold <slug>` to give it a real script contract.");
    Ok(())
}

/// `Sigil cli character scaffold <slug>`: write a Blender script an LLM can edit.
pub fn scaffold(args: &ScaffoldArgs) -> Result<()> {
    let workspace = paths::workspace_root(args.workspace.as_deref())?;
    let path = character::scaffold_mesh_script(&workspace, &args.slug, args.force)
        .map_err(|error| map_core("character_scaffold", error))?;
    let blender = character::blender_binary();

    if args.json {
        print_json(&json!({
            "ok": true,
            "path": lossy(&path),
            "blender": blender.as_ref().map(|path| lossy(path)),
            "next": [
                "edit mesh.py",
                "set DAH_MESHGEN_COMMAND to the Blender binary",
                "set DAH_MESHGEN_ARGS to a JSON array with --background --factory-startup --python <mesh.py>",
                "run `Sigil cli character create \"<prompt>\" --provider local`",
            ],
        }));
        return Ok(());
    }

    println!("Wrote the Blender mesh scaffold:");
    println!("  {}", path.display());
    println!();
    println!("Blender: {}", match &blender {
        Some(path) => lossy(path),
        None => "not found. Set DAH_BLENDER or put blender on PATH.".to_string(),
    });
    println!();
    println!("Next:");
    println!("  1. Edit the script. Replace the primitives with the brief; keep the");
    println!("     model.glb export and the turnaround render.");
    println!("  2. Point the local backend at Blender:");
    println!(
        "     DAH_MESHGEN_COMMAND=<blender> DAH_MESHGEN_ARGS='[\"--background\",\"--factory-startup\",\"--python\",\"{}\"]'",
        path.display()
    );
    println!("  3. Run `Sigil cli character create \"<prompt>\" --provider local`.");
    println!("     The script gets DAH_CHARACTER_DIR, DAH_STAGE_DIR and DAH_PROMPT.");
    println!();
    println!("An LLM or coding agent can edit mesh.py, look at turnaround.png, and iterate.");
    Ok(())
}
