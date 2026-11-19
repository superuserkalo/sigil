//! The helpers Sigil starts as child processes: the `dah` CLI and the TinyAgent host.
//!
//! Both live in this executable. `Sigil cli ...` runs the CLI and `Sigil agent-host` runs the
//! host, each still in its own process with the same stdin and stdout protocol. An explicit
//! variable (`DAH_RS_BIN`, `DAH_AGENT_BIN`) still points at a standalone build for development.

use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;

/// The subcommand that runs the dah CLI.
pub const CLI: &str = "cli";
/// The subcommand that runs the TinyAgent JSONL host.
pub const AGENT_HOST: &str = "agent-host";

/// A program and the words that come before its own arguments.
#[derive(Clone, Debug)]
pub struct Helper {
    pub program: PathBuf,
    pub prefix: Option<&'static str>,
}

impl Helper {
    /// A command for this helper, with the subcommand already added.
    pub fn command(&self) -> Command {
        let mut command = deadlock_core::util::hidden_command(&self.program);
        if let Some(prefix) = self.prefix {
            command.arg(prefix);
        }
        command
    }

    /// The program and subcommand, for labels and errors.
    pub fn label(&self) -> String {
        match self.prefix {
            Some(prefix) => format!("{} {prefix}", self.program.display()),
            None => self.program.display().to_string(),
        }
    }
}

/// The dah CLI: `DAH_RS_BIN`, else this executable with `cli`.
pub fn cli() -> Result<Helper, String> {
    resolve("DAH_RS_BIN", CLI)
}

/// The agent host: `DAH_AGENT_BIN`, else this executable with `agent-host`.
pub fn agent_host() -> Result<Helper, String> {
    resolve("DAH_AGENT_BIN", AGENT_HOST)
}

fn resolve(variable: &str, subcommand: &'static str) -> Result<Helper, String> {
    if let Some(value) = env::var_os(variable).filter(|value| !value.is_empty()) {
        let program = PathBuf::from(value);
        if program.is_file() {
            return Ok(Helper {
                program,
                prefix: None,
            });
        }
        return Err(format!(
            "{variable} points at {}, which is not a file. Correct the variable or unset it to use Sigil's own {subcommand}.",
            program.display()
        ));
    }
    let program = env::current_exe()
        .map_err(|error| format!("Sigil could not locate its own executable: {error}"))?;
    Ok(Helper {
        program,
        prefix: Some(subcommand),
    })
}

/// Run a helper subcommand when this process was started as one. `None` means a normal launch,
/// which opens the window. Nothing here runs before the check, so the window path pays one
/// argument comparison.
pub fn dispatch() -> Option<std::process::ExitCode> {
    let mut args = env::args_os();
    args.next();
    let first = args.next()?;
    if first == CLI {
        attach_console();
        let rest = std::iter::once(OsString::from("dah")).chain(args);
        let code = dah_rs::run_from(rest);
        return Some(std::process::ExitCode::from(code.clamp(0, 255) as u8));
    }
    if first == AGENT_HOST {
        return Some(dah_agent::run(dah_agent::CliLaunch::SelfCli));
    }
    None
}

/// Windows: Sigil.exe is a GUI-subsystem program, so a terminal that runs `Sigil.exe cli doctor`
/// gives it no console. Attach to the parent's console so the output shows. A parent that piped
/// stdio (the agent host, the app itself) keeps its pipes: Windows only fills the standard
/// handles that are unset.
#[cfg(windows)]
fn attach_console() {
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    unsafe extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
    }
    // SAFETY: AttachConsole takes a process id and has no other preconditions. A failure (no
    // parent console) leaves the process as it was.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(windows))]
fn attach_console() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_executable_runs_the_subcommand() {
        let helper = resolve("DAH_TEST_HELPER_UNSET_VARIABLE", CLI).expect("current exe");
        assert_eq!(helper.prefix, Some(CLI));
        let command = helper.command();
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args, vec![std::ffi::OsStr::new("cli")]);
        assert!(helper.label().ends_with(" cli"));
    }

    #[test]
    fn an_explicit_binary_takes_no_subcommand() {
        let exe = env::current_exe().expect("current exe");
        // SAFETY: test-only variable read by this test alone.
        unsafe { env::set_var("DAH_TEST_HELPER_BIN", &exe) };
        let helper = resolve("DAH_TEST_HELPER_BIN", AGENT_HOST).expect("explicit");
        assert_eq!(helper.prefix, None);
        assert_eq!(helper.program, exe);
        unsafe { env::set_var("DAH_TEST_HELPER_BIN", "/nonexistent/sigil-helper") };
        assert!(resolve("DAH_TEST_HELPER_BIN", AGENT_HOST).is_err());
        unsafe { env::remove_var("DAH_TEST_HELPER_BIN") };
    }
}
