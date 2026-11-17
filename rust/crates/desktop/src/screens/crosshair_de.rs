//! German for the Crosshair page. `crosshair.rs` looks a string up here first, then in
//! Sigil's shared tables (`settings/locales*.rs`), so this page's strings stay out of the
//! shared files.

use std::collections::HashMap;
use std::sync::OnceLock;

pub const DE: &[(&str, &str)] = &[
    ("Apply", "Anwenden"),
    ("Apply crosshair", "Fadenkreuz anwenden"),
    ("Blue", "Blau"),
    ("Cancel", "Abbrechen"),
    ("Color", "Farbe"),
    ("Console", "Konsole"),
    ("Copied", "Kopiert"),
    ("Copy code", "Code kopieren"),
    ("Copy commands", "Befehle kopieren"),
    ("Copying", "Wird kopiert"),
    ("Could not undo the removal", "Das Entfernen konnte nicht rückgängig gemacht werden"),
    ("Crosshair", "Fadenkreuz"),
    ("Crosshair removed from autoexec", "Fadenkreuz aus autoexec entfernt"),
    ("Disable hero crosshair", "Helden-Fadenkreuz deaktivieren"),
    ("Dot", "Punkt"),
    ("Game", "Spiel"),
    ("Gap", "Abstand"),
    ("Green", "Grün"),
    ("Import", "Importieren"),
    ("Keep the window above other windows", "Fenster über anderen Fenstern halten"),
    ("Length", "Länge"),
    ("Lines", "Linien"),
    ("Lines and dot", "Linien und Punkt"),
    ("No Deadlock install was found. Nothing was written.", "Keine Deadlock-Installation gefunden. Es wurde nichts geschrieben."),
    ("Opacity", "Deckkraft"),
    ("Outline", "Umriss"),
    ("Outline blue", "Umriss Blau"),
    ("Outline color", "Umrissfarbe"),
    ("Outline gap", "Umrissabstand"),
    ("Outline green", "Umriss Grün"),
    ("Outline opacity", "Umrissdeckkraft"),
    ("Outline red", "Umriss Rot"),
    ("Pin window", "Fenster anheften"),
    ("Pinned", "Angeheftet"),
    ("Presets", "Vorlagen"),
    ("Put the previous autoexec.cfg back", "Vorherige autoexec.cfg wiederhergestellt"),
    ("Put the previous autoexec.cfg back.", "Die vorherige autoexec.cfg ist wieder da."),
    ("Red", "Rot"),
    ("Remove", "Entfernen"),
    ("Remove from autoexec", "Aus autoexec entfernen"),
    ("Remove the crosshair from autoexec?", "Fadenkreuz aus autoexec entfernen?"),
    ("Removed the crosshair from autoexec.cfg.", "Das Fadenkreuz wurde aus autoexec.cfg entfernt."),
    ("Reset", "Zurücksetzen"),
    ("Save current", "Aktuelles speichern"),
    ("Size", "Größe"),
    ("Static gap", "Fester Abstand"),
    ("Takes Sigil's crosshair out of autoexec.cfg. Every other line stays, and the file is backed up first.", "Nimmt Sigils Fadenkreuz aus autoexec.cfg. Jede andere Zeile bleibt, und die Datei wird zuerst gesichert."),
    ("The game uses its own crosshair settings on the next start.", "Das Spiel nutzt beim nächsten Start seine eigenen Fadenkreuz-Einstellungen."),
    ("Thickness", "Dicke"),
    ("This build cannot pin the window yet.", "Diese Version kann das Fenster noch nicht anheften."),
    ("Tune your crosshair and read it against a real scene.", "Stimme dein Fadenkreuz ab und prüfe es vor einer echten Szene."),
    ("Unpin the window", "Fenster lösen"),
    ("Writes these settings to autoexec.cfg. The current file is backed up first.", "Schreibt diese Einstellungen in autoexec.cfg. Die aktuelle Datei wird zuerst gesichert."),
    ("autoexec.cfg has no crosshair from Sigil. Nothing changed.", "autoexec.cfg enthält kein Fadenkreuz von Sigil. Nichts wurde geändert."),
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
        let extra = [super::super::DESCRIPTION, "Copying", "Copied", "Copy code"];
        let mut missing: Vec<String> = routed(include_str!("crosshair.rs"))
            .into_iter()
            .chain(extra.iter().map(|text| text.to_string()))
            .filter(|text| get(text).is_none())
            .collect();
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
