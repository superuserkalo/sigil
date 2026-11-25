//! German for the launch dialogs, the launch progress overlay, Stop Deadlock and the automatic
//! restore. `launch_progress::tr`
//! looks a string up here first, then in Sigil's shared tables, so these strings stay out of the
//! settings files.

use std::collections::HashMap;
use std::sync::OnceLock;

pub const DE: &[(&str, &str)] = &[
    (
        "The launch stopped: Sigil could not start a background task.",
        "Der Start wurde abgebrochen: Sigil konnte keine Hintergrundaufgabe starten.",
    ),
    ("Deadlock did not close", "Deadlock wurde nicht geschlossen"),
    (
        "Deadlock is still running (process {}). Close it from Steam or the Task Manager.",
        "Deadlock läuft noch (Prozess {}). Schließe es über Steam oder den Task-Manager.",
    ),
    ("Deadlock was closed.", "Deadlock wurde geschlossen."),
    ("Stopping Deadlock failed: {}", "Deadlock ließ sich nicht beenden: {}"),
    ("The launch could not be cancelled", "Der Start ließ sich nicht abbrechen"),
    ("Last crash", "Letzter Absturz"),
    ("Already off", "Schon aus"),
    ("Nothing queued", "Nichts in der Warteschlange"),
    ("Slow · {}", "Langsam · {}"),
    ("GameBanana link or mod id", "GameBanana-Link oder Mod-ID"),
    ("No installed mod is named in the report.", "Der Bericht nennt keinen installierten Mod."),
    ("Your mods are back", "Deine Mods sind zurück"),
    ("Your mods are still set aside", "Deine Mods liegen noch beiseite"),
    ("Your mods come back when the game closes.", "Deine Mods kommen zurück, sobald das Spiel geschlossen ist."),
    (
        "Your mods are still set aside from a vanilla launch. The game starts without them.",
        "Deine Mods liegen seit einem Vanilla-Start noch beiseite. Das Spiel startet ohne sie.",
    ),
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
    #[cfg_attr(not(feature = "languages"), ignore = "translations are parked: --features languages")]
    fn every_entry_keeps_its_holes_and_is_used() {
        let source = [include_str!("launch_progress.rs"), include_str!("launch.rs"), include_str!("launch_restore.rs"), include_str!("downloads.rs"), include_str!("../app.rs")].concat();
        for (en, de) in super::DE {
            assert_eq!(en.matches("{}").count(), de.matches("{}").count(), "{en}");
            assert!(source.contains(&format!("\"{en}\"")), "unused: {en}");
        }
    }
}
