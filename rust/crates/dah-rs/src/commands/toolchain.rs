//! `dah toolchain status`, `dah mod decompile` and `dah mod compile`.
//!
//! The commands are plain run functions. Wiring in `main.rs` is intentionally
//! left to the caller:
//!
//! - `toolchain status` maps to [`status`].
//! - `mod decompile` maps to [`decompile`].
//! - `mod compile` maps to [`compile`].
//!
//! Decompile and compile are dry-run plans unless `--approve` is passed. The
//! plan is resolved by the domain core, so a plan and a run use the same
//! argument array. No shell is involved.

use std::path::{Path, PathBuf};

use serde_json::json;

use crate::error::{CliError, Result};
use crate::paths::{self, CliPaths};
use crate::util::{lossy, print_json};

use deadlock_core::toolchain;

pub struct StatusArgs {
    pub json: bool,
}

pub struct DecompileArgs {
    /// Compiled `_c` file inside the mod project.
    pub input: PathBuf,
    /// Destination folder. Defaults to `<project>/sources`.
    pub out: Option<PathBuf>,
    /// Project directory, workspace, or the usual override.
    pub workspace: Option<PathBuf>,
    /// Run the tool. Without it the command only prints the plan.
    pub approve: bool,
    pub json: bool,
}

pub struct CompileArgs {
    /// Edited source file inside the mod project.
    pub source: PathBuf,
    /// CSDK 12 `gameinfo.gi`, or the folder that holds it. Defaults to
    /// `$DAH_CSDK_PATH/gameinfo.gi`.
    pub gameinfo: Option<PathBuf>,
    /// Output folder. Defaults to `<project>/staging`.
    pub out: Option<PathBuf>,
    /// Project directory, workspace, or the usual override.
    pub workspace: Option<PathBuf>,
    /// Run the compiler. Without it the command only prints the plan.
    pub approve: bool,
    pub json: bool,
}

/// Report detected external tools and which backends are usable.
pub fn status(args: &StatusArgs) -> Result<()> {
    let status = toolchain::toolchain_status();

    if args.json {
        let value = json!({
            "ok": true,
            "toolchain": serde_json::to_value(&status)?,
        });
        print_json(&value);
        return Ok(());
    }

    println!("{}", toolchain::format_toolchain_status(&status));

    Ok(())
}

/// Plan or run a decompile inside a mod project.
pub fn decompile(paths: &CliPaths, args: &DecompileArgs) -> Result<()> {
    let project = resolve_project(paths, args.workspace.as_deref())?;
    let input = resolve_inside(&project, &args.input);
    let output_dir = match &args.out {
        Some(path) => resolve_inside(&project, path),
        None => project.join("sources"),
    };

    let request = toolchain::DecompileRequest {
        input,
        output_dir: output_dir.clone(),
    };
    let plan = toolchain::plan_decompile(&request).map_err(core_error)?;

    if !args.approve {
        if args.json {
            let value = json!({
                "ok": true,
                "applied": false,
                "plan": serde_json::to_value(&plan)?,
            });
            print_json(&value);
        } else {
            print_decompile_plan(&plan);
        }
        return Ok(());
    }

    let result = toolchain::decompile(&request).map_err(core_error)?;

    if args.json {
        let value = json!({
            "ok": true,
            "applied": true,
            "outputs": paths_json(&result.outputs),
            "log": result.log,
        });
        print_json(&value);
        return Ok(());
    }

    if result.outputs.is_empty() {
        println!(
            "Source2Viewer-CLI exited 0, but no files changed under {}.",
            lossy(&output_dir)
        );
    } else {
        println!(
            "Decompiled {} file(s) into {}",
            result.outputs.len(),
            lossy(&output_dir)
        );
        for output in &result.outputs {
            println!("  {}", lossy(output));
        }
    }

    print_log(&result.log);

    Ok(())
}

/// Plan or run a compile with a locally installed CSDK 12.
pub fn compile(paths: &CliPaths, args: &CompileArgs) -> Result<()> {
    let project = resolve_project(paths, args.workspace.as_deref())?;
    let source = resolve_inside(&project, &args.source);
    let output_dir = match &args.out {
        Some(path) => resolve_inside(&project, path),
        None => project.join("staging"),
    };
    let gameinfo = resolve_gameinfo(args.gameinfo.as_deref())?;

    let request = toolchain::CompileRequest {
        source: source.clone(),
        gameinfo,
        output_dir: output_dir.clone(),
    };
    let plan = toolchain::plan_compile(&request).map_err(core_error)?;

    if !args.approve {
        if args.json {
            let value = json!({
                "ok": true,
                "applied": false,
                "plan": serde_json::to_value(&plan)?,
            });
            print_json(&value);
        } else {
            print_compile_plan(&plan);
        }
        return Ok(());
    }

    let result = toolchain::compile(&request).map_err(core_error)?;

    if args.json {
        let value = json!({
            "ok": true,
            "applied": true,
            "outputs": paths_json(&result.outputs),
            "log": result.log,
        });
        print_json(&value);
        return Ok(());
    }

    if result.outputs.is_empty() {
        println!(
            "resourcecompiler.exe exited 0, but no compiled files were observed under {} or next to {}.",
            lossy(&output_dir),
            lossy(&source)
        );
    } else {
        println!(
            "Compiled. {} file(s) created or changed:",
            result.outputs.len()
        );
        for output in &result.outputs {
            println!("  {}", lossy(output));
        }
        println!(
            "The compiler chooses the output location. Files are reported where they were observed."
        );
    }

    print_log(&result.log);

    Ok(())
}

fn resolve_project(paths: &CliPaths, explicit: Option<&Path>) -> Result<PathBuf> {
    match explicit {
        Some(path) => paths::project_dir(Some(path)),
        None => paths::project_dir(Some(&paths.workspace_dir)),
    }
}

/// Resolve a user path against the project. Absolute and `~` paths are
/// normalized as given; relative paths are project relative.
fn resolve_inside(project: &Path, path: &Path) -> PathBuf {
    if let Some(text) = path.to_str()
        && (text == "~" || text.starts_with("~/"))
    {
        return paths::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    }

    if path.is_absolute() {
        let normalized = paths::normalize_lexical(path);
        return normalized;
    }

    paths::normalize_lexical(&project.join(path))
}

fn resolve_gameinfo(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return paths::absolute(path);
    }

    if let Some(root) = std::env::var_os(toolchain::CSDK_PATH_ENV) {
        let candidate = PathBuf::from(root).join("gameinfo.gi");
        if candidate.is_file() {
            return paths::absolute(&candidate);
        }
    }

    Err(CliError::new(
        "gameinfo_required",
        format!(
            "Pass --gameinfo with the CSDK 12 gameinfo.gi, or set {} to the CSDK root that contains it.",
            toolchain::CSDK_PATH_ENV
        ),
    ))
}

fn print_decompile_plan(plan: &toolchain::DecompilePlan) {
    println!("Decompile plan (not run)");
    println!(
        "  Tool:      {} at {}",
        plan.tool.label,
        lossy(&plan.program)
    );
    println!(
        "  Command:   {}",
        display_command(&plan.program, &plan.args)
    );
    println!("  Input:     {}", lossy(&plan.input));
    println!("  Output:    {}", lossy(&plan.output_dir));
    println!("  Workspace: {}", lossy(&plan.workspace));
    println!("Nothing was run. Pass --approve to decompile.");
}

fn print_compile_plan(plan: &toolchain::CompilePlan) {
    println!("Compile plan (not run)");
    println!(
        "  Tool:     {} at {}",
        plan.tool.label,
        lossy(&plan.program)
    );
    println!("  Command:  {}", display_command(&plan.program, &plan.args));
    println!("  Source:   {}", lossy(&plan.source));
    println!("  Gameinfo: {}", lossy(&plan.gameinfo));
    println!("  Output:   {}", lossy(&plan.output_dir));
    println!(
        "Nothing was run. Pass --approve to compile. CSDK 12 stays local and is never redistributed."
    );
}

fn display_command(program: &Path, args: &[String]) -> String {
    let mut parts = vec![quote(&lossy(program))];
    for arg in args {
        parts.push(quote(arg));
    }
    parts.join(" ")
}

fn quote(value: &str) -> String {
    if value.is_empty() || value.chars().any(char::is_whitespace) {
        return format!("{value:?}");
    }

    value.to_string()
}

fn print_log(log: &str) {
    if log.trim().is_empty() {
        return;
    }

    println!();
    print!("{log}");
    if !log.ends_with('\n') {
        println!();
    }
}

fn paths_json(paths: &[PathBuf]) -> Vec<String> {
    paths.iter().map(|path| lossy(path)).collect()
}

fn core_error(error: deadlock_core::Error) -> CliError {
    CliError::new("toolchain", error.to_string())
}
