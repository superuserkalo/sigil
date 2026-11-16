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
Mod flow: `dah mod validate`, `dah mod pack`, `dah mod release`, `dah mod deploy`, `dah publish prepare`.\n\
Deploy writes to the game install only with --approve; publish prepare never uploads.\n\n\
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
    /// Report every mod project as a review queue after a game update.
    Watch {
        /// Maximum projects to print.
        #[arg(long, default_value_t = 20, value_name = "N")]
        limit: usize,
        /// Fetch the official patch feed even when a cache exists.
        #[arg(long)]
        refresh: bool,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Prepare a release folder for upload. Never uploads.
    Publish {
        #[command(subcommand)]
        command: PublishCommand,
    },
    /// Inspect a VPK directory file and optionally verify its entries.
    Vpk {
        #[command(subcommand)]
        command: VpkCommand,
    },
}

#[derive(Subcommand)]
enum VpkCommand {
    /// Print the directory tree and check CRCs.
    Inspect {
        /// Directory file, for example pak06_dir.vpk.
        #[arg(value_name = "VPK")]
        path: PathBuf,
        /// Maximum entries to print.
        #[arg(long, default_value_t = 50, value_name = "N")]
        limit: usize,
        /// Read every entry and verify its CRC32.
        #[arg(long)]
        verify: bool,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
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
    /// Check the staging tree against the payload rules.
    Validate {
        /// Mod project directory, or a workspace containing one project.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Validate staging and pack it as pakNN_dir.vpk.
    Pack {
        /// Mod project directory, or a workspace containing one project.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
        /// Addon slot 1 to 99. Defaults to project.slot or 01.
        #[arg(long, value_name = "N")]
        slot: Option<u8>,
        /// Output directory. Defaults to <project>/builds.
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Pack into builds/<version>/ and write release.json.
    Release {
        /// Mod project directory, or a workspace containing one project.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
        /// Addon slot 1 to 99. Defaults to project.slot or 01.
        #[arg(long, value_name = "N")]
        slot: Option<u8>,
        /// Note stored in the release manifest and project history.
        #[arg(long, value_name = "TEXT")]
        note: Option<String>,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Plan or apply the latest release against the game install.
    Deploy {
        /// Mod project directory, or a workspace containing one project.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
        /// Addon slot 1 to 99. Defaults to the release manifest.
        #[arg(long, value_name = "N")]
        slot: Option<u8>,
        /// Write the VPK and gameinfo repair into the game install.
        #[arg(long)]
        approve: bool,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Update target paths that moved after a game update.
    Migrate {
        /// Mod project directory, or a workspace containing one project.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
        /// Write the new paths into project.json.
        #[arg(long)]
        apply: bool,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum PublishCommand {
    /// Copy the latest release, metadata and install notes into a folder.
    Prepare {
        /// Mod project directory, or a workspace containing one project.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
        /// Destination folder. Defaults to <project>/publish/<version>.
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Plan or approve the GameBanana upload handoff for the prepared release.
    Upload {
        /// Mod project directory, or a workspace containing one project.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
        /// Approve the handoff and write the upload record.
        #[arg(long)]
        approve: bool,
        /// Open the page in the default browser.
        #[arg(long)]
        open: bool,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Record and verify the submission URL after the browser upload.
    Confirm {
        /// Mod project directory, or a workspace containing one project.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
        /// Public submission URL, for example https://gamebanana.com/mods/123456.
        #[arg(long, value_name = "URL")]
        url: Option<String>,
        /// Record the URL without fetching the page.
        #[arg(long)]
        skip_verify: bool,
        /// Allow http:// for a local test server.
        #[arg(long)]
        allow_http: bool,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show the recorded GameBanana upload state.
    Status {
        /// Mod project directory, or a workspace containing one project.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
}

impl Cli {
    fn wants_json(&self) -> bool {
        match &self.command {
            Command::Doctor { json }
            | Command::Index { json, .. }
            | Command::Find { json, .. }
            | Command::Extract { json, .. }
            | Command::Patches { json, .. }
            | Command::Watch { json, .. } => *json,
            Command::Publish { command } => match command {
                PublishCommand::Prepare { json, .. }
                | PublishCommand::Upload { json, .. }
                | PublishCommand::Confirm { json, .. }
                | PublishCommand::Status { json, .. } => *json,
            },
            Command::Vpk { command } => match command {
                VpkCommand::Inspect { json, .. } => *json,
            },
            Command::Mod { command } => match command {
                ModCommand::New { json, .. }
                | ModCommand::Baseline { json, .. }
                | ModCommand::Check { json, .. }
                | ModCommand::Validate { json, .. }
                | ModCommand::Pack { json, .. }
                | ModCommand::Release { json, .. }
                | ModCommand::Deploy { json, .. }
                | ModCommand::Migrate { json, .. } => *json,
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
            ModCommand::Validate { workspace, json } => {
                let ok = commands::package::validate(&commands::package::ValidateArgs {
                    workspace: workspace.clone(),
                    json: *json,
                })?;
                Ok(if ok { 0 } else { 1 })
            }
            ModCommand::Pack {
                workspace,
                slot,
                out,
                json,
            } => commands::package::pack(&commands::package::PackArgs {
                workspace: workspace.clone(),
                slot: *slot,
                out: out.clone(),
                json: *json,
            }),
            ModCommand::Release {
                workspace,
                slot,
                note,
                json,
            } => {
                commands::release::release(&commands::release::ReleaseArgs {
                    workspace: workspace.clone(),
                    slot: *slot,
                    note: note.clone(),
                    json: *json,
                })?;
                Ok(0)
            }
            ModCommand::Deploy {
                workspace,
                slot,
                approve,
                json,
            } => {
                commands::release::deploy(&commands::release::DeployArgs {
                    workspace: workspace.clone(),
                    slot: *slot,
                    approve: *approve,
                    json: *json,
                })?;
                Ok(0)
            }
            ModCommand::Migrate {
                workspace,
                apply,
                json,
            } => {
                commands::migrate::run(&commands::migrate::MigrateArgs {
                    workspace: workspace.clone(),
                    apply: *apply,
                    json: *json,
                })?;
                Ok(0)
            }
        },
        Command::Watch {
            limit,
            refresh,
            json,
        } => {
            commands::watch::run(&commands::watch::WatchArgs {
                limit: *limit,
                refresh: *refresh,
                json: *json,
            })?;
            Ok(0)
        }
        Command::Vpk { command } => match command {
            VpkCommand::Inspect {
                path,
                limit,
                verify,
                json,
            } => {
                let ok = commands::vpk::inspect(&commands::vpk::InspectArgs {
                    path: path.clone(),
                    limit: *limit,
                    verify: *verify,
                    json: *json,
                })?;
                Ok(if ok { 0 } else { 1 })
            }
        },
        Command::Publish { command } => match command {
            PublishCommand::Prepare {
                workspace,
                out,
                json,
            } => {
                commands::publish::prepare(&commands::publish::PrepareArgs {
                    workspace: workspace.clone(),
                    out: out.clone(),
                    json: *json,
                })?;
                Ok(0)
            }
            PublishCommand::Upload {
                workspace,
                approve,
                open,
                json,
            } => {
                commands::publish::upload(&commands::publish::UploadArgs {
                    workspace: workspace.clone(),
                    approve: *approve,
                    open: *open,
                    json: *json,
                })?;
                Ok(0)
            }
            PublishCommand::Confirm {
                workspace,
                url,
                skip_verify,
                allow_http,
                json,
            } => {
                commands::publish::confirm(&commands::publish::ConfirmArgs {
                    workspace: workspace.clone(),
                    url: url.clone(),
                    skip_verify: *skip_verify,
                    allow_http: *allow_http,
                    json: *json,
                })?;
                Ok(0)
            }
            PublishCommand::Status { workspace, json } => {
                commands::publish::status(&commands::publish::StatusArgs {
                    workspace: workspace.clone(),
                    json: *json,
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
