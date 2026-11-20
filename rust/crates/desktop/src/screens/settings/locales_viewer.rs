//! German for the 3D viewer's viewport and control strip (`crate::viewer`). Kept apart from
//! the other tables so no two agents edit one file; declared in `viewer/mod.rs` and chained
//! into `i18n`'s maps. The duplicate and routed-file checks for these strings live here.

pub const DE: &[(&str, &str)] = &[
    ("Saved a picture to {}", "Bild gespeichert unter {}"),
    ("Could not save the picture: {}", "Das Bild konnte nicht gespeichert werden: {}"),
    ("Shown full screen", "Im Vollbild angezeigt"),
    ("3D view of {}", "3D-Ansicht von {}"),
    ("Loading the 3D view", "3D-Ansicht wird geladen"),
    ("The 3D view needs a working graphics driver.", "Die 3D-Ansicht braucht einen funktionierenden Grafiktreiber."),
    ("This machine has no GPU adapter Sigil can use.", "Dieser Rechner hat keinen Grafikadapter, den Sigil nutzen kann."),
    ("The 3D view could not open.", "Die 3D-Ansicht konnte nicht geöffnet werden."),
    ("Seek", "Spulen"),
    ("Playback speed", "Wiedergabegeschwindigkeit"),
    ("No animation", "Keine Animation"),
    ("Animation", "Animation"),
    ("With mods", "Mit Mods"),
    ("Without mods", "Ohne Mods"),
    ("Split", "Geteilt"),
    ("Reset camera", "Kamera zurücksetzen"),
    ("Save a picture", "Bild speichern"),
    ("Exit full screen", "Vollbild beenden"),
    ("Full screen", "Vollbild"),
    (
        "Drag to turn, scroll to zoom, Shift-drag to move, double-click to reset.",
        "Ziehen zum Drehen, Scrollen zum Zoomen, Umschalt-Ziehen zum Verschieben, Doppelklick zum Zurücksetzen.",
    ),
];

/// Literals in the viewer files that are not UI text: demo titles (data, like a hero name) and
/// folder names on disk.
#[cfg(test)]
const NOT_TEXT: &[&str] = &["Stand-in hero", "Hideout", "Pictures", "Sigil"];

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    /// Every English key in Sigil's other German tables, read from their source.
    fn others() -> HashSet<String> {
        let sources = [
            include_str!("locales.rs"),
            include_str!("locales_parity.rs"),
            include_str!("locales_wave.rs"),
            include_str!("locales_w4a.rs"),
            include_str!("locales_w4b.rs"),
            include_str!("locales_w4c.rs"),
            include_str!("locales_smalls.rs"),
            include_str!("locales_host.rs"),
        ];
        let mut keys = HashSet::new();
        for source in sources {
            for line in source.lines().filter(|l| l.trim_start().starts_with("(\"")) {
                keys.extend(literals(line).into_iter().step_by(2));
            }
        }
        keys
    }

    #[test]
    fn the_table_has_no_duplicates_and_keeps_holes() {
        let others = others();
        let mut seen = HashSet::new();
        for (en, de) in super::DE {
            assert!(seen.insert(*en), "duplicate key {en:?}");
            assert!(!others.contains(*en), "{en:?} is already in another table");
            assert_eq!(en.matches("{}").count(), de.matches("{}").count(), "holes in {en:?}");
            assert!(!de.contains('\u{2014}') && !en.contains('\u{2014}'), "em dash in {en:?}");
        }
    }

    /// Every UI literal in the viewer files has German here or in another table.
    #[test]
    fn every_string_in_the_viewer_is_translated() {
        let sources = [include_str!("../../viewer/mod.rs"), include_str!("../../viewer/worker.rs")];
        let mut table = others();
        table.extend(super::DE.iter().map(|(en, _)| en.to_string()));
        let mut missing = Vec::new();
        for source in sources {
            let code = source.split("#[cfg(test)]").next().unwrap_or(source);
            for literal in literals(code) {
                let ui = literal.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                    && literal.chars().any(|c| c.is_ascii_lowercase())
                    && !(literal.contains('{') && !literal.contains("{}"))
                    && !literal.contains("://");
                if ui && !table.contains(&literal) && !super::NOT_TEXT.contains(&literal.as_str()) {
                    missing.push(literal);
                }
            }
        }
        missing.sort();
        missing.dedup();
        assert!(missing.is_empty(), "untranslated: {missing:#?}");
    }

    /// Plain string literals, skipping comments, raw strings and char literals.
    fn literals(source: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut chars = source.chars().peekable();
        let mut prev = ' ';
        while let Some(c) = chars.next() {
            if c == '/' && chars.peek() == Some(&'/') {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
                prev = '\n';
                continue;
            }
            if c == '\'' && prev != 'b' {
                let rest: String = chars.clone().take(3).collect();
                if rest.len() >= 2 && rest.chars().nth(1) == Some('\'') {
                    chars.next();
                    chars.next();
                } else if rest.starts_with('\\') {
                    chars.next();
                    chars.next();
                    chars.next();
                }
                prev = c;
                continue;
            }
            if c == '"' && prev != 'r' && prev != '#' {
                let mut literal = String::new();
                let mut escaped = false;
                for c in chars.by_ref() {
                    if escaped {
                        literal.push(c);
                        escaped = false;
                    } else if c == '\\' {
                        escaped = true;
                    } else if c == '"' {
                        break;
                    } else {
                        literal.push(c);
                    }
                }
                out.push(literal);
                prev = '"';
                continue;
            }
            prev = c;
        }
        out
    }
}
