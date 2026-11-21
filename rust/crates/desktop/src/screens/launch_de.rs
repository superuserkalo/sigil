//! German for the Launch page, the launch progress overlay and Stop game. `launch_progress::tr`
//! looks a string up here first, then in Sigil's shared tables, so these strings stay out of the
//! settings files.

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
    ("Ready to launch", "Bereit zum Start"),
    ("Deadlock is not running", "Deadlock läuft nicht"),
    ("Restore mods to launch", "Mods zurücklegen, um zu starten"),
    ("Restore the mods set aside first.", "Lege zuerst die beiseitegelegten Mods zurück."),
    (
        "Moves your mods aside, then starts the game. Restore them here afterwards.",
        "Legt deine Mods beiseite und startet dann das Spiel. Lege sie danach hier zurück.",
    ),
    ("{} of {} mod on", "{} von {} Mod an"),
    ("{} of {} mods on", "{} von {} Mods an"),
    ("gameinfo.gi does not load mods", "gameinfo.gi lädt keine Mods"),
    ("gameinfo.gi loads mods", "gameinfo.gi lädt Mods"),
    ("{} file set aside", "{} Datei beiseitegelegt"),
    ("{} files set aside", "{} Dateien beiseitegelegt"),
    ("Vanilla stash", "Beiseitegelegt"),
    ("Empty", "Leer"),
    ("Last crash", "Letzter Absturz"),
    ("Already off", "Schon aus"),
    ("Nothing queued", "Nichts in der Warteschlange"),
    ("Slow · {}", "Langsam · {}"),
    ("GameBanana link or mod id", "GameBanana-Link oder Mod-ID"),
    ("No installed mod is named in the report.", "Der Bericht nennt keinen installierten Mod."),
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
        let source = [include_str!("launch_progress.rs"), include_str!("launch.rs"), include_str!("downloads.rs")].concat();
        for (en, de) in super::DE {
            assert_eq!(en.matches("{}").count(), de.matches("{}").count(), "{en}");
            assert!(source.contains(&format!("\"{en}\"")), "unused: {en}");
        }
    }
}
