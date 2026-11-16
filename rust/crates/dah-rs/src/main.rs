//! `dah`: the native command line surface over the Deadlock domain core.

mod commands;
mod domain;
mod error;
mod paths;
mod util;

use std::path::PathBuf;
use std::process;

use clap::{Parser, Subcommand};
use serde_json::json;

use crate::error::CliError;

#[derive(Parser)]
#[command(
    name = "dah",
    version,
    about = "Deadlock modding workbench CLI",
    long_about = "Inspect the Deadlock install, index game paths, extract files and manage mod projects.\n\n\
Data directory: DAH_DATA_DIR, else the platform data directory.\n\
Workspace: --workspace, else DAH_WORKSPACE_DIR, else <data>/workspace.",
    disable_help_subcommand = true,
    propagate_version = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Report the game install, gameinfo.gi and addon slot status.
    Doctor {
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Build or reuse the game path catalog and report what changed.
    Index {
        /// Rebuild even when the cache matches the VPK files.
        #[arg(long)]
        refresh: bool,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Search game paths by text, hero, type, folder or VPK.
    Find {
        /// Case-insensitive text that must appear in the path.
        #[arg(value_name = "QUERY")]
        query: String,
        /// Hero codename, for example billy or punkgoat.
        #[arg(long, value_name = "X")]
        hero: Option<String>,
        /// Compiled extension, for example vmdl_c or vtex_c.
        #[arg(long = "type", value_name = "EXT")]
        extension: Option<String>,
        /// Path prefix, for example models/heroes_wip.
        #[arg(long, value_name = "PREFIX")]
        folder: Option<String>,
        /// Restrict to one catalog source: citadel, core or maps/<name>.
        #[arg(long, value_name = "ID")]
        vpk: Option<String>,
        /// Maximum paths to print.
        #[arg(long, default_value_t = 50, value_name = "N")]
        limit: usize,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Copy exact game paths out of a game VPK into the workspace.
    Extract {
        /// Exact game paths from `dah find`.
        #[arg(value_name = "PATH", required = true, num_args = 1..)]
        paths: Vec<String>,
        /// Catalog source to read: citadel, core or maps/<name>.
        #[arg(long, default_value = "citadel", value_name = "ID")]
        vpk: String,
        /// Destination directory inside the workspace. Default `extracted`.
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
        /// Replace files that already exist.
        #[arg(long)]
        overwrite: bool,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// List official Steam patch posts and flag new ones.
    Patches {
        /// Number of posts to list.
        #[arg(long, default_value_t = 10, value_name = "N")]
        limit: usize,
        /// Fetch from Steam even when the cache is fresh.
        #[arg(long)]
        refresh: bool,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
        /// Print the full text of the newest post.
        #[arg(long)]
        full: bool,
    },
    /// Create and maintain a mod project.
    Mod {
        #[command(subcommand)]
        command: ModCommand,
    },
}

#[derive(Subcommand)]
enum ModCommand {
    /// Create a mod project under the workspace.
    New {
        /// Directory name and default display name.
        #[arg(value_name = "SLUG")]
        slug: String,
        /// Display name. Defaults to the slug.
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
        /// Parent workspace. Defaults to DAH_WORKSPACE_DIR or <data>/workspace.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Record the current game files as the compatibility baseline.
    Baseline {
        /// Mod project directory, or a workspace containing one project.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Compare the project targets with the current game files.
    Check {
        /// Mod project directory, or a workspace containing one project.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Bump the project version and record a note.
    Version {
        /// New version, for example 1.2.0.
        #[arg(value_name = "VERSION")]
        new_version: String,
        /// Note recorded with the version.
        #[arg(long, value_name = "TEXT")]
        note: Option<String>,
        /// Mod project directory, or a workspace containing one project.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
    },
}

impl Cli {
    fn wants_json(&self) -> bool {
        match &self.command {
            Command::Doctor { json }
            | Command::Index { json, .. }
            | Command::Find { json, .. }
            | Command::Extract { json, .. }
            | Command::Patches { json, .. } => *json,
            Command::Mod { command } => match command {
                ModCommand::New { json, .. }
                | ModCommand::Baseline { json, .. }
                | ModCommand::Check { json, .. } => *json,
                ModCommand::Version { .. } => false,
            },
        }
    }
}

fn main() {
    let cli = Cli::parse();
    let json_output = cli.wants_json();

    match run(&cli) {
        Ok(code) => process::exit(code),
        Err(error) => {
            report_error(&error, json_output);
            process::exit(1);
        }
    }
}

fn run(cli: &Cli) -> error::Result<i32> {
    match &cli.command {
        Command::Doctor { json } => {
            let paths = paths::resolve_paths(None)?;
            commands::doctor::run(&paths, *json)?;
            Ok(0)
        }
        Command::Index { refresh, json } => {
            let paths = paths::resolve_paths(None)?;
            commands::index::run(
                &paths,
                &commands::index::IndexArgs {
                    refresh: *refresh,
                    json: *json,
                },
            )?;
            Ok(0)
        }
        Command::Find {
            query,
            hero,
            extension,
            folder,
            vpk,
            limit,
            json,
        } => {
            let paths = paths::resolve_paths(None)?;
            commands::find::run(
                &paths,
                &commands::find::FindArgs {
                    query: query.clone(),
                    hero: hero.clone(),
                    extension: extension.clone(),
                    folder: folder.clone(),
                    vpk: vpk.clone(),
                    limit: *limit,
                    json: *json,
                },
            )?;
            Ok(0)
        }
        Command::Extract {
            paths: entry_paths,
            vpk,
            out,
            overwrite,
            json,
        } => {
            let paths = paths::resolve_paths(None)?;
            let ok = commands::extract::run(
                &paths,
                &commands::extract::ExtractArgs {
                    paths: entry_paths.clone(),
                    vpk: vpk.clone(),
                    out: out.clone(),
                    overwrite: *overwrite,
                    json: *json,
                },
            )?;
            Ok(if ok { 0 } else { 1 })
        }
        Command::Patches {
            limit,
            refresh,
            json,
            full,
        } => {
            let paths = paths::resolve_paths(None)?;
            commands::patches::run(
                &paths,
                &commands::patches::PatchArgs {
                    limit: *limit,
                    refresh: *refresh,
                    json: *json,
                    full: *full,
                },
            )?;
            Ok(0)
        }
        Command::Mod { command } => match command {
            ModCommand::New {
                slug,
                name,
                workspace,
                json,
            } => {
                commands::project::new_project(&commands::project::NewArgs {
                    slug: slug.clone(),
                    name: name.clone(),
                    workspace: workspace.clone(),
                    json: *json,
                })?;
                Ok(0)
            }
            ModCommand::Baseline { workspace, json } => {
                commands::project::baseline(&commands::project::ProjectArgs {
                    workspace: workspace.clone(),
                    json: *json,
                })?;
                Ok(0)
            }
            ModCommand::Check { workspace, json } => {
                let ok = commands::project::check(&commands::project::ProjectArgs {
                    workspace: workspace.clone(),
                    json: *json,
                })?;
                Ok(if ok { 0 } else { 1 })
            }
            ModCommand::Version {
                new_version,
                note,
                workspace,
            } => {
                commands::project::version(&commands::project::VersionArgs {
                    version: new_version.clone(),
                    note: note.clone(),
                    workspace: workspace.clone(),
                })?;
                Ok(0)
            }
        },
    }
}

fn report_error(error: &CliError, json_output: bool) {
    if json_output {
        let value = json!({
            "ok": false,
            "error": {
                "code": error.code,
                "message": error.message,
            },
        });
        match serde_json::to_string_pretty(&value) {
            Ok(text) => eprintln!("{text}"),
            Err(_) => eprintln!("dah: {}", error.message),
        }
        return;
    }

    eprintln!("dah: {}", error.message);
}
