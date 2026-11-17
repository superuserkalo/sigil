//! `dah-agent`: the standalone host binary. Sigil runs the same entry as `Sigil agent-host`.

fn main() -> std::process::ExitCode {
    dah_agent::run(dah_agent::CliLaunch::Search)
}
