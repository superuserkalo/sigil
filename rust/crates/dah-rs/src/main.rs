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
Browse: `dah browse` lists Deadlock mods from GameBanana. Read only, no key or account.\n\n\
Addons: `dah addons list` and `dah addons conflicts` read `game/citadel/addons` and never write.\n\
         `dah addons enable|disable <slot> --approve` renames one slot file.\n\n\
Profiles: `dah profiles save` captures the enabled slots, `dah profiles apply --approve` renames them back.\n\
Autoexec: `dah autoexec show` reads `game/citadel/cfg/autoexec.cfg`, `set` and `clear` write with --approve.\n\n\
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
    /// List Deadlock mod submissions from GameBanana.
    Browse {
        /// Feed page. GameBanana pages start at 1.
        #[arg(long, default_value_t = 1, value_name = "N")]
        page: usize,
        /// Records per page, 1 to 50.
        #[arg(long, default_value_t = 15, value_name = "N")]
        limit: usize,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Report installed addon slots and conflicts, or set one slot on or off.
    Addons {
        #[command(subcommand)]
        command: AddonsCommand,
    },
    /// Save and apply named sets of enabled addon slots.
    Profiles {
        #[command(subcommand)]
        command: ProfilesCommand,
    },
    /// Read and edit the game autoexec.cfg behind approval.
    Autoexec {
        #[command(subcommand)]
        command: AutoexecCommand,
    },
    /// Launch Deadlock with addons or vanilla, and restore the stash.
    Launch {
        #[command(subcommand)]
        command: LaunchCommand,
    },
    /// Read, edit and apply the crosshair convars.
    Crosshair {
        #[command(subcommand)]
        command: CrosshairCommand,
    },
    /// Download a GameBanana mod or archive, plan an install and apply it with approval.
    Install {
        #[command(subcommand)]
        command: InstallCommand,
    },
    /// Report external decompile and compile tools.
    Toolchain {
        #[command(subcommand)]
        command: ToolchainCommand,
    },
    /// Generate a character mesh from a prompt or reference image.
    Character {
        #[command(subcommand)]
        command: CharacterCommand,
    },
    /// Player profile, matches, hero performance, heatmap and leaderboard.
    Stats {
        #[command(subcommand)]
        command: StatsCommand,
    },
    /// Import local demo recordings and read the merged match summary.
    Demo {
        #[command(subcommand)]
        command: DemoCommand,
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
        /// Keep watching and write .dah/watch-state.json after each pass.
        #[arg(long)]
        watch: bool,
        /// Seconds between watch passes. Minimum 5.
        #[arg(long, default_value_t = 300, value_name = "N")]
        interval: u64,
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
enum AddonsCommand {
    /// List installed addon slots with state, size and entry count.
    List {
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Report game paths shipped by more than one enabled addon.
    Conflicts {
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Rename pakNN_dir.vpk.disabled back to pakNN_dir.vpk.
    Enable {
        /// Addon slot, 1 to 99.
        #[arg(value_name = "SLOT")]
        slot: u8,
        /// Write the rename into the game addons folder.
        #[arg(long)]
        approve: bool,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Rename pakNN_dir.vpk to pakNN_dir.vpk.disabled.
    Disable {
        /// Addon slot, 1 to 99.
        #[arg(value_name = "SLOT")]
        slot: u8,
        /// Write the rename into the game addons folder.
        #[arg(long)]
        approve: bool,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum ProfilesCommand {
    /// List saved profiles from the workspace.
    List {
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
        /// Workspace directory. Defaults to DAH_WORKSPACE_DIR or <data>/workspace.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
    },
    /// Capture the installed addon slots as a named profile.
    Save {
        /// Profile display name, for example `Bright night`.
        #[arg(value_name = "NAME")]
        name: String,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
        /// Workspace directory. Defaults to DAH_WORKSPACE_DIR or <data>/workspace.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
    },
    /// Rename slot files so the install matches a saved profile.
    Apply {
        /// Profile slug or display name.
        #[arg(value_name = "NAME")]
        name: String,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
        /// Write the renames into the game addons folder.
        #[arg(long)]
        approve: bool,
        /// Workspace directory. Defaults to DAH_WORKSPACE_DIR or <data>/workspace.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum AutoexecCommand {
    /// Print the game autoexec.cfg and its path.
    Show {
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Replace autoexec.cfg with a local file.
    Set {
        /// Local text file to write as autoexec.cfg.
        #[arg(long, value_name = "FILE")]
        file: PathBuf,
        /// Write into the game install.
        #[arg(long)]
        approve: bool,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Empty autoexec.cfg, keeping a backup of the previous file.
    Clear {
        /// Write into the game install.
        #[arg(long)]
        approve: bool,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
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
enum LaunchCommand {
    /// Launch Deadlock through Steam with the current addons.
    Modded {
        /// Start Steam.
        #[arg(long)]
        approve: bool,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Stash every addon, then launch Deadlock.
    Vanilla {
        /// Move addons and start Steam.
        #[arg(long)]
        approve: bool,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Move stashed addon files back into the game.
    Restore {
        /// Write the restore.
        #[arg(long)]
        approve: bool,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Report a pending vanilla stash.
    Status {
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum StatsCommand {
    /// Show rank, career totals and recent form for a player.
    Player {
        /// Steam account id.
        #[arg(value_name = "ACCOUNT_ID")]
        account_id: u64,
        /// Newest matches to include in the window.
        #[arg(long, default_value_t = 20, value_name = "N")]
        recent: usize,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// List recent matches.
    Matches {
        /// Steam account id.
        #[arg(value_name = "ACCOUNT_ID")]
        account_id: u64,
        /// Maximum matches to print.
        #[arg(long, default_value_t = 20, value_name = "N")]
        limit: usize,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show hero performance.
    Heroes {
        /// Steam account id.
        #[arg(value_name = "ACCOUNT_ID")]
        account_id: u64,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show the aggregate kill/death heatmap.
    Heatmap {
        /// Steam account id.
        #[arg(value_name = "ACCOUNT_ID")]
        account_id: u64,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show a regional leaderboard.
    Leaderboard {
        /// Region name, for example Europe.
        #[arg(value_name = "REGION")]
        region: String,
        /// Maximum players to print.
        #[arg(long, default_value_t = 20, value_name = "N")]
        limit: usize,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum DemoCommand {
    /// Import a Deadlock .dem into the local match store.
    Import {
        /// Demo file recorded by the game.
        #[arg(value_name = "FILE")]
        file: PathBuf,
        /// Match id for the store folder. Defaults to the file stem or a CRC32 id.
        #[arg(long, value_name = "ID")]
        id: Option<String>,
        /// Workspace directory. Defaults to DAH_WORKSPACE_DIR or <data>/workspace.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// List imported matches from the workspace store.
    List {
        /// Workspace directory. Defaults to DAH_WORKSPACE_DIR or <data>/workspace.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show the merged summary, coverage and provenance for one match.
    Show {
        /// Match id, as printed by `dah demo list`.
        #[arg(value_name = "ID")]
        id: String,
        /// Workspace directory. Defaults to DAH_WORKSPACE_DIR or <data>/workspace.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show heatmap cells from real position samples, or why they are absent.
    Heatmap {
        /// Match id, as printed by `dah demo list`.
        #[arg(value_name = "ID")]
        id: String,
        /// Workspace directory. Defaults to DAH_WORKSPACE_DIR or <data>/workspace.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Print the verified review prompt, or lint a review before storing it.
    Review {
        /// Match id, as printed by `dah demo list`.
        #[arg(value_name = "ID")]
        id: String,
        /// Review text to lint. Without it the prompt is printed.
        #[arg(long, value_name = "FILE")]
        file: Option<PathBuf>,
        /// Store review.md when every number is supported.
        #[arg(long)]
        write: bool,
        /// Workspace directory. Defaults to DAH_WORKSPACE_DIR or <data>/workspace.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Report which demo extractors are available on this build.
    Backends {
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum CharacterCommand {
    /// Create an image-to-3D or text-to-3D task.
    Create {
        /// Text prompt describing the character. Optional with --reference.
        #[arg(value_name = "PROMPT", default_value = "")]
        prompt: String,
        /// Local reference image (JPG, PNG or WebP).
        #[arg(long, value_name = "FILE")]
        reference: Option<PathBuf>,
        /// Target hero codename.
        #[arg(long, value_name = "X")]
        hero: Option<String>,
        /// Style direction.
        #[arg(long, value_name = "TEXT")]
        style: Option<String>,
        /// Provider: meshy or mock.
        #[arg(long, value_name = "NAME")]
        provider: Option<String>,
        /// Rights and consent note recorded with the task.
        #[arg(long, value_name = "TEXT")]
        rights_note: Option<String>,
        /// Consent to send the reference image to the provider.
        #[arg(long)]
        allow_reference_upload: bool,
        /// Project directory. Defaults to DAH_WORKSPACE_DIR or <data>/workspace.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show generated character tasks, optionally polling a task.
    Status {
        /// Character slug.
        #[arg(long, value_name = "SLUG")]
        slug: Option<String>,
        /// Poll the provider for a pending task.
        #[arg(long)]
        refresh: bool,
        /// Maximum tasks to print.
        #[arg(long, default_value_t = 20, value_name = "N")]
        limit: usize,
        /// Project directory.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Print the character stage graph, optionally for one recorded task.
    Stages {
        /// Character slug. Without one, print the canonical graph.
        #[arg(value_name = "SLUG")]
        slug: Option<String>,
        /// Project directory.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Import a downloaded GLB into the character workspace.
    Import {
        /// Character slug.
        #[arg(value_name = "SLUG")]
        slug: String,
        /// Project directory.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum CrosshairCommand {
    /// Print the editor crosshair and the convars it emits.
    Show {
        /// Workspace directory. Reserved for saved presets.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Read the in-game crosshair from machine_convars.vcfg.
    Import {
        /// Workspace directory. Reserved for saved presets.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Merge the managed crosshair block into autoexec.cfg.
    Apply {
        /// Write the file. Without it only the plan is printed.
        #[arg(long)]
        approve: bool,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Remove the managed crosshair block from autoexec.cfg.
    Clear {
        /// Write the file. Without it only the plan is printed.
        #[arg(long)]
        approve: bool,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum InstallCommand {
    /// Download, extract, validate and plan an install. Writes nothing to the game.
    Plan {
        /// GameBanana mod id, mod URL, https archive URL or local archive file.
        #[arg(value_name = "URL|ID")]
        target: String,
        /// Addon slot 1 to 99. Defaults to the lowest free slot.
        #[arg(long, value_name = "N")]
        slot: Option<u8>,
        /// Replace an occupied slot after backing up its files.
        #[arg(long)]
        force: bool,
        /// Extra download origin, for example a local mirror.
        #[arg(long, value_name = "ORIGIN")]
        mirror: Option<String>,
        /// Workspace directory. Defaults to DAH_WORKSPACE_DIR or <data>/workspace.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Apply a stored install plan with approval.
    Apply {
        /// Plan id printed by `dah install plan`.
        #[arg(value_name = "PLAN_ID")]
        plan_id: String,
        /// Write the payload into the game addons folder.
        #[arg(long)]
        approve: bool,
        /// Workspace directory. Defaults to DAH_WORKSPACE_DIR or <data>/workspace.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// List install journal entries and whether each file is still present.
    List {
        /// Workspace directory. Defaults to DAH_WORKSPACE_DIR or <data>/workspace.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Delete downloaded archives and extracted trees from the workspace.
    Cache {
        #[command(subcommand)]
        command: InstallCacheCommand,
    },
}

#[derive(Subcommand)]
enum InstallCacheCommand {
    /// Remove every cached download and extraction folder.
    Prune {
        /// Workspace directory. Defaults to DAH_WORKSPACE_DIR or <data>/workspace.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum ToolchainCommand {
    /// Show detected decompile and compile tools.
    Status {
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
    /// Plan or run Source2Viewer-CLI on a compiled file.
    Decompile {
        /// Compiled `_c` file inside the project.
        #[arg(value_name = "FILE")]
        input: PathBuf,
        /// Destination folder. Defaults to <project>/sources.
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
        /// Project directory, or a workspace containing one project.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
        /// Run the tool. Without it only the plan is printed.
        #[arg(long)]
        approve: bool,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Plan or run resourcecompiler.exe on a source file.
    Compile {
        /// Edited source file inside the project.
        #[arg(value_name = "FILE")]
        source: PathBuf,
        /// CSDK 12 gameinfo.gi or the folder that holds it.
        #[arg(long, value_name = "FILE")]
        gameinfo: Option<PathBuf>,
        /// Output folder. Defaults to <project>/staging.
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
        /// Project directory, or a workspace containing one project.
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
        /// Run the compiler. Without it only the plan is printed.
        #[arg(long)]
        approve: bool,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
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
            | Command::Browse { json, .. }
            | Command::Watch { json, .. } => *json,
            Command::Addons { command } => match command {
                AddonsCommand::List { json } | AddonsCommand::Conflicts { json } => *json,
                AddonsCommand::Enable { json, .. } | AddonsCommand::Disable { json, .. } => *json,
            },
            Command::Profiles { command } => match command {
                ProfilesCommand::List { json, .. }
                | ProfilesCommand::Save { json, .. }
                | ProfilesCommand::Apply { json, .. } => *json,
            },
            Command::Autoexec { command } => match command {
                AutoexecCommand::Show { json }
                | AutoexecCommand::Set { json, .. }
                | AutoexecCommand::Clear { json, .. } => *json,
            },
            Command::Launch { command } => match command {
                LaunchCommand::Modded { json, .. }
                | LaunchCommand::Vanilla { json, .. }
                | LaunchCommand::Restore { json, .. }
                | LaunchCommand::Status { json } => *json,
            },
            Command::Crosshair { command } => match command {
                CrosshairCommand::Show { json, .. }
                | CrosshairCommand::Import { json, .. }
                | CrosshairCommand::Apply { json, .. }
                | CrosshairCommand::Clear { json, .. } => *json,
            },
            Command::Install { command } => match command {
                InstallCommand::Plan { json, .. }
                | InstallCommand::Apply { json, .. }
                | InstallCommand::List { json, .. }
                | InstallCommand::Cache {
                    command: InstallCacheCommand::Prune { json, .. },
                } => *json,
            },
            Command::Toolchain { command } => match command {
                ToolchainCommand::Status { json } => *json,
            },
            Command::Character { command } => match command {
                CharacterCommand::Create { json, .. }
                | CharacterCommand::Status { json, .. }
                | CharacterCommand::Stages { json, .. }
                | CharacterCommand::Import { json, .. } => *json,
            },
            Command::Stats { command } => match command {
                StatsCommand::Player { json, .. }
                | StatsCommand::Matches { json, .. }
                | StatsCommand::Heroes { json, .. }
                | StatsCommand::Heatmap { json, .. }
                | StatsCommand::Leaderboard { json, .. } => *json,
            },
            Command::Demo { command } => match command {
                DemoCommand::Import { json, .. }
                | DemoCommand::List { json, .. }
                | DemoCommand::Show { json, .. }
                | DemoCommand::Heatmap { json, .. }
                | DemoCommand::Review { json, .. }
                | DemoCommand::Backends { json } => *json,
            },
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
                | ModCommand::Migrate { json, .. }
                | ModCommand::Decompile { json, .. }
                | ModCommand::Compile { json, .. } => *json,
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
        Command::Browse { page, limit, json } => {
            commands::browse::run(&commands::browse::BrowseArgs {
                page: *page,
                limit: *limit,
                json: *json,
            })?;
            Ok(0)
        }
        Command::Addons { command } => match command {
            AddonsCommand::List { json } => {
                commands::addons::list(*json)?;
                Ok(0)
            }
            AddonsCommand::Conflicts { json } => {
                commands::addons::conflicts(*json)?;
                Ok(0)
            }
            AddonsCommand::Enable {
                slot,
                approve,
                json,
            } => {
                commands::addons::enable(*slot, *approve, *json)?;
                Ok(0)
            }
            AddonsCommand::Disable {
                slot,
                approve,
                json,
            } => {
                commands::addons::disable(*slot, *approve, *json)?;
                Ok(0)
            }
        },
        Command::Profiles { command } => match command {
            ProfilesCommand::List { json, workspace } => {
                commands::profiles::list(*json, workspace.clone())?;
                Ok(0)
            }
            ProfilesCommand::Save {
                name,
                json,
                workspace,
            } => {
                commands::profiles::save(name, *json, workspace.clone())?;
                Ok(0)
            }
            ProfilesCommand::Apply {
                name,
                json,
                approve,
                workspace,
            } => {
                commands::profiles::apply(name, *approve, *json, workspace.clone())?;
                Ok(0)
            }
        },
        Command::Autoexec { command } => match command {
            AutoexecCommand::Show { json } => {
                commands::autoexec::show(*json)?;
                Ok(0)
            }
            AutoexecCommand::Set {
                file,
                approve,
                json,
            } => {
                commands::autoexec::set(file, *approve, *json)?;
                Ok(0)
            }
            AutoexecCommand::Clear { approve, json } => {
                commands::autoexec::clear(*approve, *json)?;
                Ok(0)
            }
        },
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
            ModCommand::Decompile {
                input,
                out,
                workspace,
                approve,
                json,
            } => {
                let paths = paths::resolve_paths(None)?;
                commands::toolchain::decompile(
                    &paths,
                    &commands::toolchain::DecompileArgs {
                        input: input.clone(),
                        out: out.clone(),
                        workspace: workspace.clone(),
                        approve: *approve,
                        json: *json,
                    },
                )?;
                Ok(0)
            }
            ModCommand::Compile {
                source,
                gameinfo,
                out,
                workspace,
                approve,
                json,
            } => {
                let paths = paths::resolve_paths(None)?;
                commands::toolchain::compile(
                    &paths,
                    &commands::toolchain::CompileArgs {
                        source: source.clone(),
                        gameinfo: gameinfo.clone(),
                        out: out.clone(),
                        workspace: workspace.clone(),
                        approve: *approve,
                        json: *json,
                    },
                )?;
                Ok(0)
            }
        },
        Command::Launch { command } => match command {
            LaunchCommand::Modded { approve, json } => {
                commands::launch::modded(*approve, *json)?;
                Ok(0)
            }
            LaunchCommand::Vanilla { approve, json } => {
                commands::launch::vanilla(*approve, *json)?;
                Ok(0)
            }
            LaunchCommand::Restore { approve, json } => {
                commands::launch::restore(*approve, *json)?;
                Ok(0)
            }
            LaunchCommand::Status { json } => {
                commands::launch::status(*json)?;
                Ok(0)
            }
        },
        Command::Crosshair { command } => match command {
            CrosshairCommand::Show { workspace, json } => {
                commands::crosshair::show(&commands::crosshair::ShowArgs {
                    workspace: workspace.clone(),
                    json: *json,
                })?;
                Ok(0)
            }
            CrosshairCommand::Import { workspace, json } => {
                commands::crosshair::import(
                    &paths::resolve_paths(None)?,
                    &commands::crosshair::ImportArgs {
                        workspace: workspace.clone(),
                        json: *json,
                    },
                )?;
                Ok(0)
            }
            CrosshairCommand::Apply { approve, json } => {
                commands::crosshair::apply(
                    &paths::resolve_paths(None)?,
                    &commands::crosshair::ApplyArgs {
                        approve: *approve,
                        json: *json,
                    },
                )?;
                Ok(0)
            }
            CrosshairCommand::Clear { approve, json } => {
                commands::crosshair::clear(
                    &paths::resolve_paths(None)?,
                    &commands::crosshair::ClearArgs {
                        approve: *approve,
                        json: *json,
                    },
                )?;
                Ok(0)
            }
        },
        Command::Install { command } => match command {
            InstallCommand::Plan {
                target,
                slot,
                force,
                mirror,
                workspace,
                json,
            } => {
                commands::install::plan(&commands::install::PlanArgs {
                    target: target.clone(),
                    slot: *slot,
                    force: *force,
                    workspace: workspace.clone(),
                    mirror: mirror.clone(),
                    json: *json,
                })?;
                Ok(0)
            }
            InstallCommand::Apply {
                plan_id,
                approve,
                workspace,
                json,
            } => {
                commands::install::apply(&commands::install::ApplyArgs {
                    plan_id: plan_id.clone(),
                    approve: *approve,
                    workspace: workspace.clone(),
                    json: *json,
                })?;
                Ok(0)
            }
            InstallCommand::List { workspace, json } => {
                commands::install::list(&commands::install::ListArgs {
                    workspace: workspace.clone(),
                    json: *json,
                })?;
                Ok(0)
            }
            InstallCommand::Cache { command } => match command {
                InstallCacheCommand::Prune { workspace, json } => {
                    commands::install::cache_prune(&commands::install::PruneArgs {
                        workspace: workspace.clone(),
                        json: *json,
                    })?;
                    Ok(0)
                }
            },
        },
        Command::Toolchain { command } => match command {
            ToolchainCommand::Status { json } => {
                commands::toolchain::status(&commands::toolchain::StatusArgs { json: *json })?;
                Ok(0)
            }
        },
        Command::Stats { command } => match command {
            StatsCommand::Player {
                account_id,
                recent,
                json,
            } => {
                commands::stats::run_player(&commands::stats::PlayerArgs {
                    account_id: *account_id,
                    recent: *recent,
                    json: *json,
                })?;
                Ok(0)
            }
            StatsCommand::Matches {
                account_id,
                limit,
                json,
            } => {
                commands::stats::run_matches(&commands::stats::MatchesArgs {
                    account_id: *account_id,
                    limit: *limit,
                    json: *json,
                })?;
                Ok(0)
            }
            StatsCommand::Heroes { account_id, json } => {
                commands::stats::run_heroes(&commands::stats::HeroesArgs {
                    account_id: *account_id,
                    json: *json,
                })?;
                Ok(0)
            }
            StatsCommand::Heatmap { account_id, json } => {
                commands::stats::run_heatmap(&commands::stats::HeatmapArgs {
                    account_id: *account_id,
                    json: *json,
                })?;
                Ok(0)
            }
            StatsCommand::Leaderboard {
                region,
                limit,
                json,
            } => {
                commands::stats::run_leaderboard(&commands::stats::LeaderboardArgs {
                    region: region.clone(),
                    limit: *limit,
                    json: *json,
                })?;
                Ok(0)
            }
        },
        Command::Demo { command } => match command {
            DemoCommand::Import {
                file,
                id,
                workspace,
                json,
            } => {
                commands::demo::run_import(&commands::demo::ImportArgs {
                    file: file.clone(),
                    id: id.clone(),
                    workspace: workspace.clone(),
                    json: *json,
                })?;
                Ok(0)
            }
            DemoCommand::List { workspace, json } => {
                commands::demo::run_list(&commands::demo::ListArgs {
                    workspace: workspace.clone(),
                    json: *json,
                })?;
                Ok(0)
            }
            DemoCommand::Show { id, workspace, json } => {
                commands::demo::run_show(&commands::demo::ShowArgs {
                    id: id.clone(),
                    workspace: workspace.clone(),
                    json: *json,
                })?;
                Ok(0)
            }
            DemoCommand::Heatmap { id, workspace, json } => {
                commands::demo::run_heatmap(&commands::demo::HeatmapArgs {
                    id: id.clone(),
                    workspace: workspace.clone(),
                    json: *json,
                })?;
                Ok(0)
            }
            DemoCommand::Review {
                id,
                file,
                write,
                workspace,
                json,
            } => {
                commands::demo::run_review(&commands::demo::ReviewArgs {
                    id: id.clone(),
                    file: file.clone(),
                    write: *write,
                    workspace: workspace.clone(),
                    json: *json,
                })?;
                Ok(0)
            }
            DemoCommand::Backends { json } => {
                commands::demo::run_backends(&commands::demo::BackendsArgs { json: *json })?;
                Ok(0)
            }
        },
        Command::Character { command } => match command {
            CharacterCommand::Create {
                prompt,
                reference,
                hero,
                style,
                provider,
                rights_note,
                allow_reference_upload,
                workspace,
                json,
            } => {
                commands::character::create(&commands::character::CreateArgs {
                    prompt: prompt.clone(),
                    reference: reference.clone(),
                    target_hero: hero.clone(),
                    style: style.clone(),
                    provider: provider.clone(),
                    rights_note: rights_note.clone(),
                    allow_reference_upload: *allow_reference_upload,
                    workspace: workspace.clone(),
                    json: *json,
                })?;
                Ok(0)
            }
            CharacterCommand::Status {
                slug,
                refresh,
                limit,
                workspace,
                json,
            } => {
                commands::character::status(&commands::character::StatusArgs {
                    workspace: workspace.clone(),
                    slug: slug.clone(),
                    refresh: *refresh,
                    limit: *limit,
                    json: *json,
                })?;
                Ok(0)
            }
            CharacterCommand::Stages {
                slug,
                workspace,
                json,
            } => {
                commands::character::stages(&commands::character::StagesArgs {
                    slug: slug.clone(),
                    workspace: workspace.clone(),
                    json: *json,
                })?;
                Ok(0)
            }
            CharacterCommand::Import {
                slug,
                workspace,
                json,
            } => {
                commands::character::import(&commands::character::ImportArgs {
                    slug: slug.clone(),
                    workspace: workspace.clone(),
                    json: *json,
                })?;
                Ok(0)
            }
        },
        Command::Watch {
            limit,
            refresh,
            watch,
            interval,
            json,
        } => {
            commands::watch::run(&commands::watch::WatchArgs {
                limit: *limit,
                refresh: *refresh,
                watch: *watch,
                interval: *interval,
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
