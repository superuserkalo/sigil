//! Settings > Logs and support additions from parity wave 4:
//!
//! - Log source (DMM): Sigil's log, the game's crash dumps, or both, for Copy and Ask Sigil.
//! - Ask Sigil for help (DMM's AI help, without a website): the masked logs go into Sigil's own
//!   agent composer on this computer. Nothing is sent until the player presses Send there.
//! - Include the full log in the report (Grimoire): the whole log, capped at 5 MB, instead of
//!   the last 200 lines.

use std::path::PathBuf;

use deadlock_core::game_session;
use rune::{El, Tx};

use super::i18n::{fill, tr};
use super::log;
use super::options;
use super::{Confirm, button, detail, segmented, setting_row, toggle_row, trailing};
use crate::app::State;
use crate::model::Screen;
use crate::ui::{ActionKind, Ui};

/// DMM caps what it hands an assistant at 100,000 characters; Sigil keeps the same budget.
pub const MAX_CHARS: usize = 100_000;
/// Grimoire attaches up to 5 MB of log to a report.
pub const FULL_LOG_CAP: usize = 5 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    Combined,
    Sigil,
    Dumps,
}

impl Source {
    pub fn id(self) -> &'static str {
        match self {
            Source::Combined => "combined",
            Source::Sigil => "sigil",
            Source::Dumps => "dumps",
        }
    }

    pub fn from_id(id: &str) -> Source {
        match id {
            "sigil" => Source::Sigil,
            "dumps" => Source::Dumps,
            _ => Source::Combined,
        }
    }
}

/// The newest crash dumps, newest first.
fn newest_dumps(st: &State, take: usize) -> Vec<PathBuf> {
    let mut roots = deadlock_core::discovery::default_steam_roots();
    if let Some(custom) = options::get().steam_root.clone() {
        roots.insert(0, PathBuf::from(custom));
    }
    let mut dumps = game_session::list_dumps(&game_session::dump_folders(st.game_root.as_deref(), &roots));
    dumps.sort_by_key(|a| std::cmp::Reverse(a.modified));
    dumps.into_iter().take(take).map(|d| d.path).collect()
}

/// Cut a text to its last `budget` characters on a line boundary.
pub fn tail_chars(text: &str, budget: usize) -> String {
    let count = text.chars().count();
    if count <= budget {
        return text.to_string();
    }
    let start = text.char_indices().nth(count - budget).map_or(0, |(at, _)| at);
    let rest = &text[start..];
    if text[..start].ends_with('\n') {
        return rest.to_string();
    }
    match rest.find('\n') {
        Some(at) => rest[at + 1..].to_string(),
        None => rest.to_string(),
    }
}

/// The text for a source, masked and inside the budget. Combined gives each half.
pub fn gather(st: &State, source: Source) -> String {
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default();
    let mask = |s: String| if home.len() > 1 { s.replace(&home, "~") } else { s };
    let log_budget = if source == Source::Combined { MAX_CHARS / 2 } else { MAX_CHARS };
    let mut parts = Vec::new();
    if source != Source::Dumps {
        parts.push(format!("=== SIGIL LOG ===\n{}", tail_chars(&log::tail(5_000), log_budget)));
    }
    if source != Source::Sigil {
        let dumps = if st.demo { Vec::new() } else { newest_dumps(st, 3) };
        if dumps.is_empty() {
            parts.push("=== CRASH DUMPS ===\nNo crash dumps found.".to_string());
        } else {
            for path in dumps {
                let info = game_session::read_dump_info(&path);
                parts.push(format!("=== CRASH DUMP ===\n{}", super::support::crash_text(&path, &info)));
            }
        }
    }
    let joined = mask(parts.join("\n\n"));
    log::redact(&tail_chars(&joined, MAX_CHARS))
}

/// The whole log for a report, capped (Grimoire's "Include full log").
pub fn full_log() -> String {
    let mut text = String::new();
    for file in [log::dir().join(format!("{}.1", log::FILE)), log::path()] {
        if let Ok(read) = std::fs::read_to_string(file) {
            text.push_str(&read);
            text.push('\n');
        }
    }
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default();
    let masked = if home.len() > 1 { text.replace(&home, "~") } else { text };
    log::redact(&tail_chars(&masked, FULL_LOG_CAP))
}

/// The prompt Sigil's agent gets: what to look at, then the logs. Mirrors the template DMM
/// puts in the assistant's address bar, with the logs pasted in rather than in a URL.
pub fn agent_prompt(logs: &str) -> String {
    format!(
        "{}\n\n{}\n\n{logs}",
        tr("Help me troubleshoot Sigil or Deadlock. Read these logs, say what went wrong and how to fix it."),
        tr("My issue is: "),
    )
}

/// Put the logs in the agent's composer and open the agent. The agent page turns on if it was
/// off; nothing is sent until Send.
pub fn ask_agent(st: &mut State) {
    // A build without the agent has nothing to hand the logs to.
    if !crate::prefs::AGENT {
        return;
    }
    let source = Source::from_id(&options::get().log_source);
    let logs = gather(st, source);
    if !st.prefs.flag("agent") {
        crate::prefs::edit(st, |p| p.toggle_flag("agent"));
        super::apply_flags(st);
    }
    st.composer = rune::TextEdit::new(agent_prompt(&logs));
    st.go(Screen::Agent);
}

/// Rows under the log file rows.
pub fn rows(_s: &State, u: &Ui) -> Vec<El> {
    let o = options::get();
    let source = Source::from_id(&o.log_source);
    let source_line = if crate::prefs::AGENT { "What Copy logs and Ask Sigil read." } else { "What Copy logs reads." };
    let mut rows = vec![
        setting_row(
            u,
            "settings-log-source",
            "Log source",
            detail(u, source_line),
            segmented(
                u,
                "settings-log-source-pick",
                &[(Source::Combined, "Combined"), (Source::Sigil, "Sigil log"), (Source::Dumps, "Crash dumps")],
                source,
                |st, pick| options::edit(st, |o| o.log_source = pick.id().to_string()),
            ),
        ),
        setting_row(
            u,
            "settings-log-copy-source",
            "Copy logs",
            detail(u, "The chosen source, masked, up to 100,000 characters."),
            button(u, "settings-log-copy-source-run", "Copy", ActionKind::Secondary).on_press_tx(
                |st: &mut State, tx: &mut Tx| {
                    let text = gather(st, Source::from_id(&options::get().log_source));
                    let chars = text.chars().count().to_string();
                    tx.copy(text);
                    super::say(st, fill(tr("Copied {} characters."), &[&chars]), false);
                },
            ),
        ),
    ];
    // Ask Sigil hands the logs to the agent, which only a build with the agent has.
    if crate::prefs::AGENT {
        rows.push(setting_row(
            u,
            "settings-log-ask",
            "Ask Sigil for help",
            detail(u, "Hands the masked logs to Sigil's own agent on this computer. No website, and nothing is sent until you press Send."),
            trailing(vec![
                button(u, "settings-log-ask-run", "Ask Sigil", ActionKind::Secondary)
                    .on_press(|st: &mut State| st.settings.confirm = Some(Confirm::AskAgent)),
            ]),
        ));
    }
    rows
}

/// "Include the full log" for the report section.
pub fn full_log_row(u: &Ui) -> El {
    toggle_row(
        u,
        "settings-report-full-log",
        "Include the full log",
        "Up to 5 MB of log in the report instead of the last 200 lines.",
        options::get().full_log,
        |st| options::edit(st, |o| o.full_log = !o.full_log),
    )
}

/// The confirm's text (DMM: "Share Log Files with AI?").
pub fn confirm_text(agent_on: bool) -> String {
    let base = tr("Logs can hold file paths and user names. Sigil masks your home folder and keys, puts the logs in its agent's message box and opens the agent. Your model provider sees them only when you press Send.");
    if agent_on {
        base.to_string()
    } else {
        format!("{base} {}", tr("This turns on the Agent page."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_text_keeps_its_last_whole_lines() {
        let text = "first line\nsecond line\nthird";
        assert_eq!(tail_chars(text, 100), text);
        assert_eq!(tail_chars(text, 14), "third");
        assert_eq!(tail_chars(text, 18), "second line\nthird");
        assert_eq!(tail_chars(text, 17), "second line\nthird");
        assert_eq!(tail_chars(text, 16), "third");
        assert_eq!(tail_chars("sec\nond line\nthird", 16), "ond line\nthird");
    }

    #[test]
    fn sources_round_trip_and_default_to_combined() {
        for source in [Source::Combined, Source::Sigil, Source::Dumps] {
            assert_eq!(Source::from_id(source.id()), source);
        }
        assert_eq!(Source::from_id("other"), Source::Combined);
    }

    #[test]
    fn the_prompt_puts_the_logs_last() {
        let prompt = agent_prompt("=== SIGIL LOG ===\nline");
        assert!(prompt.ends_with("=== SIGIL LOG ===\nline"));
        assert!(prompt.contains("My issue is"));
    }
}
