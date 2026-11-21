//! German for the Autoexec page. `autoexec.rs` looks a string up here first, then in
//! Sigil's shared tables (`settings/locales*.rs`), so this page's strings stay out of the
//! shared files.

use std::collections::HashMap;
use std::sync::OnceLock;

pub const DE: &[(&str, &str)] = &[
    ("More", "Mehr"),
    ("Search premade commands", "Vorgefertigte Befehle suchen"),
    ("Search", "Suchen"),
    ("No premade command matches", "Kein vorgefertigter Befehl passt"),
    ("Premade", "Vorgefertigt"),
    ("Add", "Hinzufügen"),
    ("All convars", "Alle Konsolenvariablen"),
    ("Add custom", "Eigenen hinzufügen"),
    ("Autoexec", "Autoexec"),
    ("Cancel", "Abbrechen"),
    ("Clear all", "Alle löschen"),
    ("Clear all commands?", "Alle Befehle löschen?"),
    ("Commands", "Befehle"),
    ("Console commands the game runs every time it starts.", "Konsolenbefehle, die das Spiel bei jedem Start ausführt."),
    ("Copied", "Kopiert"),
    ("Copy all", "Alle kopieren"),
    ("Deadlock was not found", "Deadlock wurde nicht gefunden"),
    ("Edit in editor", "Im Editor bearbeiten"),
    ("Empty", "Leer"),
    ("In Steam now", "Jetzt in Steam"),
    ("Launch options", "Startoptionen"),
    ("Launch options not written", "Startoptionen nicht geschrieben"),
    ("Launch options written to Steam", "Startoptionen in Steam geschrieben"),
    ("No autoexec.cfg yet. Add a command and save to create it.", "Noch keine autoexec.cfg. Füge einen Befehl hinzu und speichere, um sie anzulegen."),
    ("No commands yet. Pick one above, or write your own.", "Noch keine Befehle. Wähle oben einen aus oder schreibe deinen eigenen."),
    ("No launch options saved in Sigil yet.", "In Sigil sind noch keine Startoptionen gespeichert."),
    ("Open", "Öffnen"),
    ("Open Settings", "Einstellungen öffnen"),
    ("Passed to Deadlock when Steam starts it. Sigil writes them into Steam right before it launches the game.", "Werden an Deadlock übergeben, wenn Steam es startet. Sigil schreibt sie direkt vor dem Spielstart in Steam."),
    ("Premade commands", "Fertige Befehle"),
    ("Removes all {} lines from the list. autoexec.cfg stays as it is until you save.", "Entfernt alle {} Zeilen aus der Liste. autoexec.cfg bleibt unverändert, bis du speicherst."),
    ("Removes the one line from the list. autoexec.cfg stays as it is until you save.", "Entfernt die eine Zeile aus der Liste. autoexec.cfg bleibt unverändert, bis du speicherst."),
    ("Save", "Speichern"),
    ("Save autoexec.cfg", "autoexec.cfg speichern"),
    ("Save mine", "Meine speichern"),
    ("Saved. Sigil writes them into Steam the next time it launches the game.", "Gespeichert. Sigil schreibt sie beim nächsten Spielstart in Steam."),
    ("Set the game folder in Settings first.", "Lege zuerst den Spielordner in den Einstellungen fest."),
    ("Set the game folder in Settings to edit autoexec.cfg.", "Lege den Spielordner in den Einstellungen fest, um autoexec.cfg zu bearbeiten."),
    ("Something else wrote to it after you opened it. Use the file as it is now, or save yours over it.", "Etwas anderes hat nach dem Öffnen hineingeschrieben. Nimm die Datei, wie sie jetzt ist, oder speichere deine darüber."),
    ("Steam already has these launch options.", "Steam hat diese Startoptionen bereits."),
    ("Steam config not found. Start Deadlock from Steam once, then come back.", "Steam-Konfiguration nicht gefunden. Starte Deadlock einmal über Steam und komm dann zurück."),
    ("Steam config not found. Start Deadlock from Steam once, then try again.", "Steam-Konfiguration nicht gefunden. Starte Deadlock einmal über Steam und versuche es dann erneut."),
    ("Steam is running and would undo the change when it closes. Quit Steam, then try again.", "Steam läuft und würde die Änderung beim Beenden rückgängig machen. Beende Steam und versuche es erneut."),
    ("Steam is running. Quit it before launching from Sigil so the change sticks.", "Steam läuft. Beende es vor dem Start aus Sigil, damit die Änderung bleibt."),
    ("Steam was already running. Quit Steam before launching from Sigil so they apply.", "Steam lief bereits. Beende Steam vor dem Start aus Sigil, damit sie greifen."),
    ("The current file is backed up first. The game reads it on the next start.", "Die aktuelle Datei wird zuerst gesichert. Das Spiel liest sie beim nächsten Start."),
    ("There is no autoexec.cfg yet. Save a command first, then open it in your editor.", "Es gibt noch keine autoexec.cfg. Speichere zuerst einen Befehl und öffne sie dann in deinem Editor."),
    ("This creates the file. The game reads it on the next start.", "Damit wird die Datei angelegt. Das Spiel liest sie beim nächsten Start."),
    ("Unsaved changes", "Ungespeicherte Änderungen"),
    ("Use the file", "Datei übernehmen"),
    ("Write to Steam now", "Jetzt in Steam schreiben"),
    ("Writing", "Wird geschrieben"),
    ("Written to Steam. The old localconfig.vdf is backed up next to it.", "In Steam geschrieben. Die alte localconfig.vdf liegt als Sicherung daneben."),
    ("Your commands", "Deine Befehle"),
    ("Your saved value replaces it the next time you launch from Sigil.", "Dein gespeicherter Wert ersetzt ihn beim nächsten Start aus Sigil."),
    ("autoexec.cfg changed", "autoexec.cfg wurde geändert"),
    ("autoexec.cfg could not be read", "autoexec.cfg konnte nicht gelesen werden"),
];

pub fn get(text: &str) -> Option<&'static str> {
    static MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| DE.iter().copied().collect()).get(text).copied()
}

/// Every `tr("..")` literal in `source`, plus both arms of `tr(if .. { ".." } else { ".." })`.
#[cfg(test)]
pub fn routed(source: &str) -> Vec<String> {
    fn literal(rest: &str) -> Option<String> {
        let rest = rest.trim_start().strip_prefix('"')?;
        let mut out = String::new();
        let mut chars = rest.chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => out.push(chars.next()?),
                '"' => return Some(out),
                c => out.push(c),
            }
        }
        None
    }
    let mut found = Vec::new();
    for (at, _) in source.match_indices("tr(") {
        if at > 0 && source[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let rest = &source[at + 3..];
        if let Some(text) = literal(rest) {
            found.push(text);
        } else if let Some(cond) = rest.trim_start().strip_prefix("if ")
            && let Some(open) = cond.find('{')
            && let Some(first) = literal(&cond[open + 1..])
        {
            found.push(first);
            if let Some(else_at) = cond.find("else")
                && let Some(open) = cond[else_at..].find('{')
                && let Some(second) = literal(&cond[else_at + open + 1..])
            {
                found.push(second);
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_routed_string_has_german() {
        let mut missing: Vec<String> = routed(include_str!("autoexec.rs"))
            .into_iter()
            .filter(|text| get(text).is_none())
            .collect();
        missing.extend(
            [super::super::DESCRIPTION]
                .iter()
                .filter(|text| get(text).is_none())
                .map(|text| text.to_string()),
        );
        missing.sort();
        missing.dedup();
        assert!(missing.is_empty(), "untranslated: {missing:#?}");
    }

    #[test]
    fn table_is_clean() {
        let mut seen = std::collections::HashSet::new();
        for (en, de) in DE {
            assert!(seen.insert(*en), "duplicate {en:?}");
            assert_eq!(en.matches("{}").count(), de.matches("{}").count(), "holes in {en:?}");
            assert!(!en.contains('\u{2014}') && !de.contains('\u{2014}'), "em dash in {en:?}");
        }
    }
}
