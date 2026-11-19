//! External toolchain detection plus the decompile and compile backends.
//!
//! Nothing here downloads, bundles or redistributes a tool. Every backend is
//! resolved from an explicit environment setting first and from `PATH` second.
//! An absent tool is reported as unavailable with the reason, never as a
//! disabled feature without an explanation.
//!
//! - Decompile: `Source2Viewer-CLI` turns a compiled `_c` asset into editable
//!   source. It is cross-platform.
//! - Compile: CSDK 12 `resourcecompiler.exe` turns edited source back into a
//!   compiled `_c` asset. It is Windows only. CSDK 12 has no license, so it is
//!   never shipped or fetched by this crate.
//!
//! Path containment for decompile follows the product rule: the input and the
//! output both stay inside a mod workspace. Compile keeps the same "no game
//! writes" rule through the caller but does not require a project manifest,
//! because a CSDK content tree is a legitimate source location.

use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::util::absolute;

/// Override for the `Source2Viewer-CLI` binary.
pub const SOURCE2VIEWER_CLI_ENV: &str = "DAH_SOURCE2VIEWER_CLI";
/// Override for the `vpkeditcli` binary.
pub const VPKEDITCLI_ENV: &str = "DAH_VPKEDITCLI";
/// Override for the `vpkmerge` binary.
pub const VPKMERGE_ENV: &str = "DAH_VPKMERGE";
/// A local CSDK 12 root. `resourcecompiler.exe` is found below it.
pub const CSDK_PATH_ENV: &str = "DAH_CSDK_PATH";
/// A direct path to `resourcecompiler.exe`.
pub const RESOURCECOMPILER_ENV: &str = "DAH_RESOURCECOMPILER";
/// A workspace root, used for path containment when no `project.json` exists.
pub const WORKSPACE_ENV: &str = "DAH_WORKSPACE_DIR";

/// Wall clock cap for one decompile run.
pub const DECOMPILE_TIMEOUT: Duration = Duration::from_secs(120);
/// Wall clock cap for one compile run.
pub const COMPILE_TIMEOUT: Duration = Duration::from_secs(300);
/// Version probes are best effort and must never hold up a status report.
const VERSION_TIMEOUT: Duration = Duration::from_secs(3);
/// Captured process output is capped so a chatty tool cannot flood the caller.
const MAX_LOG_BYTES: usize = 64 * 1024;

/// Relative locations of `resourcecompiler.exe` inside a CSDK 12 install.
const CSDK_RESOURCECOMPILER_PATHS: &[&str] = &[
    "game/bin_tools/win64/resourcecompiler.exe",
    "bin_tools/win64/resourcecompiler.exe",
    "game/bin/win64/resourcecompiler.exe",
    "resourcecompiler.exe",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolKind {
    #[serde(rename = "source2viewer-cli")]
    Source2ViewerCli,
    #[serde(rename = "vpkeditcli")]
    VpkEditCli,
    #[serde(rename = "vpkmerge")]
    VpkMerge,
    #[serde(rename = "resourcecompiler")]
    ResourceCompiler,
}

impl ToolKind {
    pub fn all() -> [ToolKind; 4] {
        [
            ToolKind::Source2ViewerCli,
            ToolKind::ResourceCompiler,
            ToolKind::VpkEditCli,
            ToolKind::VpkMerge,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            ToolKind::Source2ViewerCli => "Source 2 Viewer CLI",
            ToolKind::VpkEditCli => "vpkeditcli",
            ToolKind::VpkMerge => "vpkmerge",
            ToolKind::ResourceCompiler => "resourcecompiler.exe",
        }
    }

    fn env_vars(self) -> &'static [&'static str] {
        match self {
            ToolKind::Source2ViewerCli => &[SOURCE2VIEWER_CLI_ENV],
            ToolKind::VpkEditCli => &[VPKEDITCLI_ENV],
            ToolKind::VpkMerge => &[VPKMERGE_ENV],
            ToolKind::ResourceCompiler => &[RESOURCECOMPILER_ENV],
        }
    }

    fn executable_names(self) -> &'static [&'static str] {
        match self {
            ToolKind::Source2ViewerCli => &["Source2Viewer-CLI", "Source2Viewer-CLI.exe"],
            ToolKind::VpkEditCli => &["vpkeditcli", "vpkeditcli.exe"],
            ToolKind::VpkMerge => &["vpkmerge", "vpkmerge.exe"],
            ToolKind::ResourceCompiler => &["resourcecompiler.exe", "resourcecompiler"],
        }
    }

    fn missing_reason(self) -> String {
        match self {
            ToolKind::Source2ViewerCli => format!(
                "Source2Viewer-CLI was not found on PATH. Set {SOURCE2VIEWER_CLI_ENV} to the CLI binary. Nothing is downloaded."
            ),
            ToolKind::VpkEditCli => format!(
                "vpkeditcli was not found on PATH. Set {VPKEDITCLI_ENV} to the binary. Nothing is downloaded."
            ),
            ToolKind::VpkMerge => format!(
                "vpkmerge was not found on PATH. Set {VPKMERGE_ENV} to the binary. Nothing is downloaded."
            ),
            ToolKind::ResourceCompiler => {
                if cfg!(windows) {
                    format!(
                        "resourcecompiler.exe was not found. Set {RESOURCECOMPILER_ENV} or {CSDK_PATH_ENV} to a local CSDK 12 install. CSDK 12 is never bundled or downloaded."
                    )
                } else {
                    format!(
                        "resourcecompiler.exe is Windows only and was not found. Set {RESOURCECOMPILER_ENV} or {CSDK_PATH_ENV} to a local CSDK 12 install or a wrapper that runs here. CSDK 12 is never bundled or downloaded."
                    )
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatus {
    pub kind: ToolKind,
    pub label: String,
    pub available: bool,
    pub path: Option<PathBuf>,
    /// Why the tool is unavailable. Always set when `available` is false.
    pub reason: Option<String>,
    /// First line of `--version`, when the tool answers it. Best effort.
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolchainStatus {
    /// `std::env::consts::OS`: macos, windows or linux.
    pub platform: String,
    pub tools: Vec<ToolStatus>,
    pub decompile_available: bool,
    pub decompile_reason: Option<String>,
    /// True on Windows with a working `resourcecompiler.exe`, or on another
    /// host when the configured compiler is an executable for that host.
    pub compile_available: bool,
    pub compile_reason: Option<String>,
}

impl ToolchainStatus {
    pub fn tool(&self, kind: ToolKind) -> Option<&ToolStatus> {
        self.tools.iter().find(|tool| tool.kind == kind)
    }
}

/// Detect every known tool. Reads the environment on each call, so a changed
/// `DAH_*` variable is picked up without a restart.
pub fn toolchain_status() -> ToolchainStatus {
    let tools: Vec<ToolStatus> = ToolKind::all().into_iter().map(detect_tool).collect();

    let source2viewer = tools
        .iter()
        .find(|tool| tool.kind == ToolKind::Source2ViewerCli)
        .cloned()
        .unwrap_or_else(|| unavailable(ToolKind::Source2ViewerCli, "not probed"));
    let resourcecompiler = tools
        .iter()
        .find(|tool| tool.kind == ToolKind::ResourceCompiler)
        .cloned()
        .unwrap_or_else(|| unavailable(ToolKind::ResourceCompiler, "not probed"));

    let decompile_available = source2viewer.available;
    let decompile_reason = if decompile_available {
        None
    } else {
        source2viewer.reason.clone()
    };

    let compile_available = resourcecompiler.available;
    let compile_reason = if compile_available {
        None
    } else {
        Some(compile_unavailable_reason(&resourcecompiler))
    };

    ToolchainStatus {
        platform: env::consts::OS.to_string(),
        tools,
        decompile_available,
        decompile_reason,
        compile_available,
        compile_reason,
    }
}

fn detect_tool(kind: ToolKind) -> ToolStatus {
    detect_tool_with_version(kind, true)
}

fn detect_tool_with_version(kind: ToolKind, probe_version: bool) -> ToolStatus {
    if kind == ToolKind::ResourceCompiler {
        if let Some(value) = env_value(RESOURCECOMPILER_ENV) {
            return from_explicit_file(
                kind,
                RESOURCECOMPILER_ENV,
                PathBuf::from(value),
                probe_version,
            );
        }
        if let Some(value) = env_value(CSDK_PATH_ENV) {
            return from_csdk_root(kind, PathBuf::from(value), probe_version);
        }
    } else if let Some(var) = kind.env_vars().first()
        && let Some(value) = env_value(var)
    {
        return from_explicit_file(kind, var, PathBuf::from(value), probe_version);
    }

    match find_on_path(kind) {
        PathSearch::Runnable(path) => available(kind, path, probe_version),
        PathSearch::Unusable(path, reason) => unavailable_with_path(kind, path, reason),
        PathSearch::Missing => unavailable(kind, kind.missing_reason()),
    }
}

enum PathSearch {
    Runnable(PathBuf),
    Unusable(PathBuf, String),
    Missing,
}

fn from_explicit_file(kind: ToolKind, var: &str, path: PathBuf, probe_version: bool) -> ToolStatus {
    let resolved = absolute(&path);
    match check_runnable(&resolved) {
        Ok(()) => available(kind, resolved, probe_version),
        Err(reason) => unavailable(
            kind,
            format!("{var} points to {}, which {reason}.", resolved.display()),
        ),
    }
}

fn from_csdk_root(kind: ToolKind, root: PathBuf, probe_version: bool) -> ToolStatus {
    let resolved = absolute(&root);

    if !resolved.is_dir() {
        return unavailable(
            kind,
            format!(
                "{CSDK_PATH_ENV} points to {}, which is not a folder.",
                resolved.display()
            ),
        );
    }

    for relative in CSDK_RESOURCECOMPILER_PATHS {
        let candidate = resolved.join(relative);
        if candidate.is_file() {
            return match check_runnable(&candidate) {
                Ok(()) => available(kind, candidate, probe_version),
                Err(reason) => unavailable(
                    kind,
                    format!(
                        "{CSDK_PATH_ENV} resolved to {}, which {reason}.",
                        candidate.display()
                    ),
                ),
            };
        }
    }

    unavailable(
        kind,
        format!(
            "{CSDK_PATH_ENV} points to {}, but no resourcecompiler.exe was found below it. Expected game/bin_tools/win64/resourcecompiler.exe.",
            resolved.display()
        ),
    )
}

fn available(kind: ToolKind, path: PathBuf, probe_version: bool) -> ToolStatus {
    let version = if probe_version && kind != ToolKind::ResourceCompiler {
        probe_version_line(&path)
    } else {
        None
    };

    ToolStatus {
        kind,
        label: kind.label().to_string(),
        available: true,
        path: Some(path),
        reason: None,
        version,
    }
}

fn unavailable(kind: ToolKind, reason: impl Into<String>) -> ToolStatus {
    ToolStatus {
        kind,
        label: kind.label().to_string(),
        available: false,
        path: None,
        reason: Some(reason.into()),
        version: None,
    }
}

fn unavailable_with_path(kind: ToolKind, path: PathBuf, reason: String) -> ToolStatus {
    ToolStatus {
        kind,
        label: kind.label().to_string(),
        available: false,
        path: None,
        reason: Some(format!("{} {reason}.", path.display())),
        version: None,
    }
}

fn compile_unavailable_reason(tool: &ToolStatus) -> String {
    match &tool.reason {
        Some(reason) => reason.clone(),
        None => format!("{} is unavailable.", tool.label),
    }
}

fn env_value(name: &str) -> Option<String> {
    match env::var(name) {
        Ok(value) if !value.trim().is_empty() => Some(value.trim().to_string()),
        _ => None,
    }
}

fn find_on_path(kind: ToolKind) -> PathSearch {
    let Some(path_value) = env::var_os("PATH") else {
        return PathSearch::Missing;
    };

    let mut unusable = None;

    for directory in env::split_paths(&path_value) {
        for name in kind.executable_names() {
            let candidate = directory.join(name);
            if !candidate.is_file() {
                continue;
            }

            match check_runnable(&candidate) {
                Ok(()) => return PathSearch::Runnable(candidate),
                Err(reason) => {
                    if unusable.is_none() {
                        unusable = Some((candidate, reason));
                    }
                }
            }
        }
    }

    match unusable {
        Some((path, reason)) => PathSearch::Unusable(path, reason),
        None => PathSearch::Missing,
    }
}

/// True when the file can plausibly be executed on this host. A Windows PE
/// file on macOS or Linux is reported as unavailable instead of pretending a
/// path found on `PATH` is runnable.
fn check_runnable(path: &Path) -> std::result::Result<(), String> {
    let metadata = fs::metadata(path).map_err(|error| format!("cannot be read: {error}"))?;

    if !metadata.is_file() {
        return Err("is not a file".to_string());
    }

    if metadata.len() == 0 {
        return Err("is empty".to_string());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        if metadata.permissions().mode() & 0o111 == 0 {
            return Err("is not executable".to_string());
        }
    }

    #[cfg(not(windows))]
    {
        let mut file = fs::File::open(path).map_err(|error| format!("cannot be read: {error}"))?;
        let mut magic = [0_u8; 2];
        let count = file.read(&mut magic).unwrap_or(0);

        if count == 2 && magic == *b"MZ" {
            return Err(format!(
                "is a Windows PE binary and cannot run on {} without a wrapper",
                env::consts::OS
            ));
        }
    }

    Ok(())
}

fn probe_version_line(program: &Path) -> Option<String> {
    let args = [OsString::from("--version")];
    let output = run_child(program, &args, None, VERSION_TIMEOUT).ok()?;

    if output.timed_out || output.exit_code != Some(0) {
        return None;
    }

    let text = if output.stdout.trim().is_empty() {
        output.stderr
    } else {
        output.stdout
    };

    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(120).collect())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DecompileRequest {
    /// A compiled `_c` file, usually copied out of the game VPK.
    pub input: PathBuf,
    /// Destination folder inside the same mod workspace.
    pub output_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DecompileResult {
    /// Files created or changed by this run. Empty when the CLI wrote nothing.
    pub outputs: Vec<PathBuf>,
    pub log: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DecompilePlan {
    pub tool: ToolStatus,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub input: PathBuf,
    pub output_dir: PathBuf,
    pub workspace: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompileRequest {
    /// Editable source file such as `billy.vmat` or `jacket.png`.
    pub source: PathBuf,
    /// `gameinfo.gi` for the CSDK install, or the folder that holds it.
    pub gameinfo: PathBuf,
    /// Working folder and scan root for produced files.
    pub output_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompileResult {
    /// Files created or changed by this run, wherever the compiler wrote them.
    pub outputs: Vec<PathBuf>,
    pub log: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompilePlan {
    pub tool: ToolStatus,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub source: PathBuf,
    pub gameinfo: PathBuf,
    pub output_dir: PathBuf,
}

/// Validate a decompile request and resolve the exact command without running
/// it. This is what a dry run prints.
pub fn plan_decompile(request: &DecompileRequest) -> Result<DecompilePlan> {
    let tool = detect_tool_with_version(ToolKind::Source2ViewerCli, false);

    let program = available_program(&tool).map_err(|reason| {
        Error::msg(format!(
            "Decompile needs Source2Viewer-CLI. {reason} Install the tool yourself and point {SOURCE2VIEWER_CLI_ENV} at it."
        ))
    })?;

    let input = absolute(&request.input);
    if !input.is_file() {
        return Err(Error::msg(format!(
            "Decompile input {} does not exist or is not a file.",
            input.display()
        )));
    }

    let canonical_input = fs::canonicalize(&input).map_err(|error| {
        Error::msg(format!(
            "Could not resolve decompile input {}: {error}",
            input.display()
        ))
    })?;

    let output_dir = absolute(&request.output_dir);
    let workspace = workspace_root_for(&output_dir).ok_or_else(|| {
        Error::msg(format!(
            "Decompile output {} is not inside a mod workspace. Run the command for a mod project, or set {WORKSPACE_ENV} to the workspace root.",
            output_dir.display()
        ))
    })?;

    if !canonical_input.starts_with(&workspace) {
        return Err(Error::msg(format!(
            "Decompile input {} is outside the workspace {}. Decompile reads files inside the mod workspace only.",
            canonical_input.display(),
            workspace.display()
        )));
    }

    if !canonical_lenient(&output_dir).starts_with(&workspace) {
        return Err(Error::msg(format!(
            "Decompile output {} is outside the workspace {}. Decompile writes inside the mod workspace only.",
            output_dir.display(),
            workspace.display()
        )));
    }

    let args = decompile_args(&canonical_input, &output_dir);

    Ok(DecompilePlan {
        tool,
        program,
        args,
        input: canonical_input,
        output_dir,
        workspace,
    })
}

/// Decompile one compiled asset into a workspace folder.
pub fn decompile(request: &DecompileRequest) -> Result<DecompileResult> {
    let plan = plan_decompile(request)?;

    fs::create_dir_all(&plan.output_dir)?;
    let before = snapshot(&plan.output_dir, true)?;

    let output = run_child(&plan.program, &os_args(&plan.args), None, DECOMPILE_TIMEOUT)?;
    let log = render_log(&plan.program, &os_args(&plan.args), &output);

    if output.timed_out {
        return Err(Error::msg(format!(
            "Source2Viewer-CLI timed out after {} seconds.\n{log}",
            DECOMPILE_TIMEOUT.as_secs()
        )));
    }

    if output.exit_code != Some(0) {
        return Err(Error::msg(format!("Source2Viewer-CLI failed.\n{log}")));
    }

    let after = snapshot(&plan.output_dir, true)?;
    let outputs = changed_paths(&before, &after);

    Ok(DecompileResult { outputs, log })
}

/// Validate a compile request and resolve the exact command without running
/// it. Availability is reported honestly on every platform.
pub fn plan_compile(request: &CompileRequest) -> Result<CompilePlan> {
    let tool = detect_tool_with_version(ToolKind::ResourceCompiler, false);

    let program = available_program(&tool).map_err(|reason| {
        Error::msg(format!(
            "Compile is unavailable. {reason} CSDK 12 is never bundled, downloaded or redistributed."
        ))
    })?;

    let source = absolute(&request.source);
    if !source.is_file() {
        return Err(Error::msg(format!(
            "Compile source {} does not exist or is not a file.",
            source.display()
        )));
    }

    let gameinfo = resolve_gameinfo(&request.gameinfo)?;
    let output_dir = absolute(&request.output_dir);

    let args = compile_args(&source, &gameinfo);

    Ok(CompilePlan {
        tool,
        program,
        args,
        source,
        gameinfo,
        output_dir,
    })
}

/// Compile one edited source file with the locally installed CSDK 12.
///
/// `resourcecompiler.exe` chooses where it writes the compiled `_c` file. The
/// result reports the files this run created or changed in `output_dir` and
/// next to the source, and never predicts a game path.
pub fn compile(request: &CompileRequest) -> Result<CompileResult> {
    let plan = plan_compile(request)?;

    fs::create_dir_all(&plan.output_dir)?;
    let output_before = snapshot(&plan.output_dir, true)?;
    let source_parent = plan
        .source
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| plan.source.clone());
    let source_before = snapshot(&source_parent, false)?;

    let output = run_child(
        &plan.program,
        &os_args(&plan.args),
        Some(&plan.output_dir),
        COMPILE_TIMEOUT,
    )?;
    let log = render_log(&plan.program, &os_args(&plan.args), &output);

    if output.timed_out {
        return Err(Error::msg(format!(
            "resourcecompiler.exe timed out after {} seconds.\n{log}",
            COMPILE_TIMEOUT.as_secs()
        )));
    }

    if output.exit_code != Some(0) {
        return Err(Error::msg(format!("resourcecompiler.exe failed.\n{log}")));
    }

    let output_after = snapshot(&plan.output_dir, true)?;
    let source_after = snapshot(&source_parent, false)?;

    let mut outputs = changed_paths(&output_before, &output_after);
    for path in changed_paths(&source_before, &source_after) {
        if path
            .file_name()
            .map(|name| name.to_string_lossy().ends_with("_c"))
            .unwrap_or(false)
        {
            outputs.push(path);
        }
    }
    outputs.sort();
    outputs.dedup();

    Ok(CompileResult { outputs, log })
}

fn available_program(tool: &ToolStatus) -> std::result::Result<PathBuf, String> {
    if tool.available
        && let Some(path) = &tool.path
    {
        return Ok(path.clone());
    }

    Err(tool
        .reason
        .clone()
        .unwrap_or_else(|| format!("{} is unavailable.", tool.label)))
}

fn resolve_gameinfo(path: &Path) -> Result<PathBuf> {
    let resolved = absolute(path);
    let resolved = if resolved.is_dir() {
        resolved.join("gameinfo.gi")
    } else {
        resolved
    };

    if !resolved.is_file() {
        return Err(Error::msg(format!(
            "Compile needs a gameinfo.gi file, but {} does not exist. Point --gameinfo at the CSDK 12 gameinfo.gi.",
            resolved.display()
        )));
    }

    fs::canonicalize(&resolved).map_err(|error| {
        Error::msg(format!(
            "Could not resolve gameinfo.gi {}: {error}",
            resolved.display()
        ))
    })
}

fn decompile_args(input: &Path, output_dir: &Path) -> Vec<String> {
    vec![
        "-i".to_string(),
        input.to_string_lossy().into_owned(),
        "-o".to_string(),
        output_dir.to_string_lossy().into_owned(),
        "-d".to_string(),
    ]
}

fn compile_args(source: &Path, gameinfo: &Path) -> Vec<String> {
    vec![
        "-i".to_string(),
        source.to_string_lossy().into_owned(),
        "-game".to_string(),
        gameinfo.to_string_lossy().into_owned(),
    ]
}

fn os_args(args: &[String]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

/// Workspace root is the nearest ancestor with `project.json`, else
/// `DAH_WORKSPACE_DIR` when the output lives below it.
fn workspace_root_for(output_dir: &Path) -> Option<PathBuf> {
    let resolved = canonical_lenient(output_dir);

    for ancestor in resolved.ancestors() {
        if ancestor.join("project.json").is_file() {
            return Some(ancestor.to_path_buf());
        }
    }

    if let Some(value) = env_value(WORKSPACE_ENV) {
        let root = canonical_lenient(&absolute(Path::new(&value)));
        if resolved.starts_with(&root) {
            return Some(root);
        }
    }

    None
}

/// Canonicalize the deepest existing ancestor and keep the missing tail. This
/// keeps containment checks sound for an output folder that does not exist yet.
fn canonical_lenient(path: &Path) -> PathBuf {
    let mut current = absolute(path);
    let mut tail: Vec<OsString> = Vec::new();

    while !current.exists() {
        match current.file_name() {
            Some(name) => tail.push(name.to_os_string()),
            None => break,
        }
        if !current.pop() {
            break;
        }
    }

    let mut resolved = fs::canonicalize(&current).unwrap_or(current);
    for part in tail.iter().rev() {
        resolved.push(part);
    }

    resolved
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    length: u64,
    modified: Option<SystemTime>,
}

type Snapshot = BTreeMap<PathBuf, FileStamp>;

fn snapshot(directory: &Path, recursive: bool) -> Result<Snapshot> {
    let mut entries = Snapshot::new();
    collect_snapshot(directory, recursive, &mut entries)?;
    Ok(entries)
}

fn collect_snapshot(directory: &Path, recursive: bool, entries: &mut Snapshot) -> Result<()> {
    let reader = match fs::read_dir(directory) {
        Ok(reader) => reader,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };

    for entry in reader {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let path = entry.path();

        if file_type.is_symlink() {
            continue;
        }

        if file_type.is_dir() {
            if recursive {
                collect_snapshot(&path, true, entries)?;
            }
            continue;
        }

        if !file_type.is_file() {
            continue;
        }

        let metadata = entry.metadata()?;
        entries.insert(
            path,
            FileStamp {
                length: metadata.len(),
                modified: metadata.modified().ok(),
            },
        );
    }

    Ok(())
}

fn changed_paths(before: &Snapshot, after: &Snapshot) -> Vec<PathBuf> {
    after
        .iter()
        .filter(|(path, stamp)| before.get(*path) != Some(*stamp))
        .map(|(path, _)| path.clone())
        .collect()
}

struct ChildOutput {
    timed_out: bool,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_child(
    program: &Path,
    args: &[OsString],
    current_dir: Option<&Path>,
    timeout: Duration,
) -> Result<ChildOutput> {
    let mut command = crate::util::hidden_command(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Some(directory) = current_dir {
        command.current_dir(directory);
    }

    let mut child = command
        .spawn()
        .map_err(|error| Error::msg(format!("Could not run {}: {error}", program.display())))?;

    let stdout_pipe = child
        .stdout
        .take()
        .ok_or_else(|| Error::msg("Could not capture tool stdout."))?;
    let stderr_pipe = child
        .stderr
        .take()
        .ok_or_else(|| Error::msg("Could not capture tool stderr."))?;

    let stdout_thread = thread::spawn(move || read_pipe(stdout_pipe));
    let stderr_thread = thread::spawn(move || read_pipe(stderr_pipe));

    let started = Instant::now();
    let mut status = None;

    loop {
        match child.try_wait() {
            Ok(Some(finished)) => {
                status = Some(finished);
                break;
            }
            Ok(None) => {}
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_thread.join();
                let _ = stderr_thread.join();
                return Err(Error::msg(format!(
                    "Could not wait for {}: {error}",
                    program.display()
                )));
            }
        }

        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            break;
        }

        thread::sleep(Duration::from_millis(25));
    }

    let stdout = stdout_thread.join().unwrap_or_default();
    let stderr = stderr_thread.join().unwrap_or_default();

    Ok(ChildOutput {
        timed_out: status.is_none(),
        exit_code: status.and_then(|value| value.code()),
        stdout,
        stderr,
    })
}

fn read_pipe(mut pipe: impl Read) -> String {
    let mut bytes = Vec::new();
    let _ = pipe.read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

fn render_log(program: &Path, args: &[OsString], output: &ChildOutput) -> String {
    let mut log = String::new();
    log.push_str("$ ");
    log.push_str(&format_invocation(program, args));
    log.push('\n');

    if output.timed_out {
        log.push_str("timed out\n");
    } else {
        match output.exit_code {
            Some(code) => log.push_str(&format!("exit code: {code}\n")),
            None => log.push_str("terminated by signal\n"),
        }
    }

    if !output.stdout.is_empty() {
        log.push_str(&output.stdout);
        if !output.stdout.ends_with('\n') {
            log.push('\n');
        }
    }

    if !output.stderr.is_empty() {
        log.push_str("stderr:\n");
        log.push_str(&output.stderr);
        if !output.stderr.ends_with('\n') {
            log.push('\n');
        }
    }

    truncate_log(log)
}

fn format_invocation(program: &Path, args: &[OsString]) -> String {
    let mut parts = vec![quote(&program.to_string_lossy())];
    for arg in args {
        parts.push(quote(&arg.to_string_lossy()));
    }
    parts.join(" ")
}

fn quote(value: &str) -> String {
    if value.is_empty() || value.chars().any(|character| character.is_whitespace()) {
        return format!("{value:?}");
    }

    value.to_string()
}

fn truncate_log(mut log: String) -> String {
    if log.len() <= MAX_LOG_BYTES {
        return log;
    }

    let mut cut = MAX_LOG_BYTES;
    while cut > 0 && !log.is_char_boundary(cut) {
        cut -= 1;
    }
    log.truncate(cut);
    log.push_str("\n[log truncated]\n");
    log
}

/// Human-readable status with one line per tool and an explicit capability
/// line for each backend.
pub fn format_toolchain_status(status: &ToolchainStatus) -> String {
    let mut lines = vec![format!("Toolchain on {}", status.platform)];

    for tool in &status.tools {
        let state = if tool.available {
            match (&tool.path, &tool.version) {
                (Some(path), Some(version)) => {
                    format!("available at {} (version {version})", path.display())
                }
                (Some(path), None) => format!("available at {}", path.display()),
                (None, _) => "available".to_string(),
            }
        } else {
            format!(
                "unavailable ({})",
                inline_reason(tool.reason.as_deref().unwrap_or("no reason reported"))
            )
        };

        lines.push(format!("  {}: {state}", tool.label));
    }

    lines.push(capability_line(
        "Decompile",
        status.decompile_available,
        status.decompile_reason.as_deref(),
    ));
    lines.push(capability_line(
        "Compile",
        status.compile_available,
        status.compile_reason.as_deref(),
    ));
    lines.push(
        "CSDK 12 is used from a local install and is never bundled or downloaded.".to_string(),
    );

    lines.join("\n")
}

fn capability_line(label: &str, available: bool, reason: Option<&str>) -> String {
    if available {
        return format!("{label}: available");
    }

    format!(
        "{label}: unavailable ({})",
        inline_reason(reason.unwrap_or("no backend configured"))
    )
}

fn inline_reason(reason: &str) -> String {
    reason.trim_end_matches('.').to_string()
}
