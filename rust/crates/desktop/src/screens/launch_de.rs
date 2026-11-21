//! German for the launch progress overlay and Stop game. `launch_progress.rs` looks a string up
//! here first, then in Sigil's shared tables, so these strings stay out of the settings files.

use std::collections::HashMap;
use std::sync::OnceLock;

pub const DE: &[(&str, &str)] = &[
    ("Deadlock did not close", "Deadlock wurde nicht geschlossen"),
    (
        "Deadlock is still running (process {}). Close it from Steam or the Task Manager.",
        "Deadlock läuft noch (Prozess {}). Schließe es über Steam oder den Task-Manager.",
    ),
    ("Deadlock was closed.", "Deadlock wurde geschlossen."),
    ("Stopping Deadlock failed: {}", "Deadlock ließ sich nicht beenden: {}"),
    ("The launch could not be cancelled", "Der Start ließ sich nicht abbrechen"),
];

fn map() -> &'static HashMap<&'static str, &'static str> {
    static MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| DE.iter().copied().collect())
}

pub(super) fn get(text: &str) -> Option<&'static str> {
    map().get(text).copied()
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_entry_keeps_its_holes_and_is_used() {
        let source = include_str!("launch_progress.rs");
        for (en, de) in super::DE {
            assert_eq!(en.matches("{}").count(), de.matches("{}").count(), "{en}");
            assert!(source.contains(&format!("\"{en}\"")), "unused: {en}");
        }
    }
}
