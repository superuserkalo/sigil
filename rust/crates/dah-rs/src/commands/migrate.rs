//! `dah mod migrate`: plan target paths that moved after a game update and,
//! only with `--apply`, write the new paths into `project.json`.

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::domain;
use crate::error::{CliError, Result};
use crate::paths;
use crate::util::{lossy, print_json};

pub struct MigrateArgs {
    pub workspace: Option<PathBuf>,
    pub apply: bool,
    pub json: bool,
}

pub fn run(args: &MigrateArgs) -> Result<()> {
    let project_dir = paths::project_dir(args.workspace.as_deref())?;
    let project = domain::read_project(&project_dir)?;

    if project.targets.is_empty() {
        return Err(CliError::new(
            "no_targets",
            format!(
                "Project {} has no target paths to migrate. Add them to {} first.",
                project.slug,
                project_dir.join("project.json").display()
            ),
        ));
    }

    let load = domain::load_catalog(&project_dir, false)?;
    let migrations = domain::migration_plan(&project, &load.loaded.catalog);

    if migrations.is_empty() {
        if args.json {
            print_json(&json!({
                "ok": true,
                "project": lossy(&project_dir),
                "slug": project.slug,
                "applied": false,
                "migrations": [],
            }));
            return Ok(());
        }

        println!(
            "Project {}: no moved target paths against the current game files.",
            project.slug
        );
        return Ok(());
    }

    if args.apply {
        domain::apply_migration_plan(&project_dir, &project, &migrations)?;
    }

    if args.json {
        print_json(&json!({
            "ok": true,
            "project": lossy(&project_dir),
            "slug": project.slug,
            "applied": args.apply,
            "migrations": migrations.iter().map(migration_json).collect::<Vec<_>>(),
        }));
        return Ok(());
    }

    println!(
        "Project {}: {} moved target path(s) against the current game files",
        project.slug,
        migrations.len()
    );

    for migration in &migrations {
        println!("  MOVED {}", migration.from);
        println!("     -> {}", migration.to);
    }

    if args.apply {
        println!(
            "Applied. project.json and matching staging files now use the new paths; run `Sigil cli mod baseline` to record the new CRCs."
        );
    } else {
        println!("Dry run: project.json was not changed. Re-run with `--apply` to update it.");
    }

    Ok(())
}

fn migration_json(migration: &domain::Migration) -> Value {
    json!({
        "from": migration.from,
        "to": migration.to,
    })
}
