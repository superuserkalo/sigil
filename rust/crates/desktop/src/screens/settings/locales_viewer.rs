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
    // The Controls panel and the recordings (`viewer/controls.rs`).
    ("{}% done", "{} % erledigt"),
    ("Saved a recording to {}", "Aufnahme gespeichert unter {}"),
    ("Auto-rotate", "Automatisch drehen"),
    ("Controls", "Steuerung"),
    ("Stop recording", "Aufnahme beenden"),
    ("Record a GIF", "GIF aufnehmen"),
    ("Record a video (up to 10 seconds)", "Video aufnehmen (bis zu 10 Sekunden)"),
    ("Idle", "Stehen"),
    ("Combat", "Kampf"),
    ("Lighting", "Beleuchtung"),
    ("Studio", "Studio"),
    ("Midtown dusk", "Midtown in der Dämmerung"),
    ("Transparent", "Transparent"),
    ("My picture", "Mein Bild"),
    ("Open a picture...", "Bild öffnen ..."),
    ("Choose a background picture", "Hintergrundbild wählen"),
    ("Pictures", "Bilder"),
    ("That picture is over 12 MB.", "Das Bild ist größer als 12 MB."),
    ("Cloth", "Stoff"),
    ("Particles", "Partikel"),
    ("Bloom", "Leuchten"),
    ("Shadows", "Schatten"),
    ("GIF {} of {} frames", "GIF {} von {} Bildern"),
    ("GIF position", "GIF-Position"),
    ("Review the GIF", "GIF prüfen"),
    ("GIF preview", "GIF-Vorschau"),
    ("Saving...", "Wird gespeichert ..."),
    ("Save GIF", "GIF speichern"),
    ("Retake", "Neu aufnehmen"),
    // Backgrounds: Deadlock Mod Manager's studio gradient and Grimoire's soul backdrops.
    ("Studio gradient", "Studio-Verlauf"),
    ("Charcoal spotlight", "Kohle mit Spot"),
    ("Warm ember", "Warme Glut"),
    ("Cool abyss blue", "Kühles Abgrundblau"),
    ("Violet soul", "Violette Seele"),
    ("Deep forest", "Tiefer Wald"),
    ("Crimson dusk", "Purpurne Dämmerung"),
    ("Teal nebula", "Petrol-Nebel"),
    ("Amber halo", "Bernsteinschein"),
    ("Indigo-to-rose dawn", "Indigo-Rosé-Morgen"),
    ("Ice steel", "Eisstahl"),
    (
        "Drag to turn, scroll to zoom, Shift-drag to move, double-click to reset.",
        "Ziehen zum Drehen, Scrollen zum Zoomen, Umschalt-Ziehen zum Verschieben, Doppelklick zum Zurücksetzen.",
    ),
    // Where the viewer appears (`viewer/prepare.rs`, `viewer/dialog.rs`, Locker, Installed).
    ("Reading your mods for {}", "Deine Mods für {} werden gelesen"),
    ("Building the Hideout with your mods", "Das Hideout wird mit deinen Mods gebaut"),
    ("Preparing {} with your mods", "{} wird mit deinen Mods vorbereitet"),
    ("Preparing {} without mods", "{} wird ohne Mods vorbereitet"),
    ("No installed mod changes {}.", "Kein installierter Mod verändert {}."),
    ("Preparing {} with this mod", "{} wird mit diesem Mod vorbereitet"),
    ("Preparing {} without this mod", "{} wird ohne diesen Mod vorbereitet"),
    (
        "This mod is off: you see it as it would look switched on.",
        "Dieser Mod ist aus: Du siehst ihn so, wie er eingeschaltet aussähe.",
    ),
    ("{} does not change how {} looks.", "{} verändert nicht, wie {} aussieht."),
    ("{} changes nothing this view draws.", "{} verändert nichts, was diese Ansicht zeigt."),
    ("The preview worker stopped before it finished.", "Die Vorschau wurde beendet, bevor sie fertig war."),
    ("With this mod", "Mit diesem Mod"),
    ("Without this mod", "Ohne diesen Mod"),
    (
        "The first time takes a while; after that it opens at once.",
        "Beim ersten Mal dauert es etwas, danach öffnet es sofort.",
    ),
    ("Set the game folder in Settings to see heroes in 3D.", "Lege den Spielordner in den Einstellungen fest, um Helden in 3D zu sehen."),
    ("The 3D preview could not be made.", "Die 3D-Vorschau konnte nicht erstellt werden."),
    ("{} does not change a hero model.", "{} verändert kein Heldenmodell."),
    ("Preview in 3D", "In 3D ansehen"),
    ("Changes", "Verändert"),
    ("3D view", "3D-Ansicht"),
    ("Preview {} in 3D", "{} in 3D ansehen"),
    ("while off", "solange aus"),
    (
        "No 3D on this machine: it has no GPU adapter Sigil can use.",
        "Kein 3D auf diesem Rechner: Er hat keinen Grafikadapter, den Sigil nutzen kann.",
    ),
    // Loading and failure on the stage (`viewer/mod.rs`, `viewer/worker.rs`).
    ("Reading the room's models: {} of {}", "Modelle des Raums werden gelesen: {} von {}"),
    ("Lighting the room", "Der Raum wird beleuchtet"),
    ("Drawing the first frame", "Das erste Bild wird gezeichnet"),
    ("Exposure", "Belichtung"),
    ("Reset exposure", "Belichtung zurücksetzen"),
    ("Pictures and recordings", "Bilder und Aufnahmen"),
    ("Ask where to save", "Beim Speichern nach dem Ort fragen"),
    ("Save as", "Speichern unter"),
    ("Wireframe", "Drahtgitter"),
    ("The 3D view is not on screen.", "Die 3D-Ansicht ist nicht sichtbar."),
    ("Use this view as the mod's cover", "Diese Ansicht als Titelbild der Mod verwenden"),
    ("Saved the view as this mod's cover.", "Die Ansicht wurde als Titelbild dieser Mod gespeichert."),
    ("Use the 3D view", "3D-Ansicht verwenden"),
    ("Use the 3D view for {}", "3D-Ansicht für {} verwenden"),
    ("Open the hero's 3D view first, then try again.", "Öffne zuerst die 3D-Ansicht des Helden und versuche es dann erneut."),
    ("Putting the room on the graphics card: {} of {}", "Der Raum wird auf die Grafikkarte geladen: {} von {}"),
    ("Elapsed", "Vergangen"),
    ("The graphics card reported an error: {}", "Die Grafikkarte hat einen Fehler gemeldet: {}"),
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
        let sources = [
            include_str!("../../viewer/mod.rs"),
            include_str!("../../viewer/worker.rs"),
            include_str!("../../viewer/prepare.rs"),
            include_str!("../../viewer/dialog.rs"),
            include_str!("../../viewer/controls.rs"),
        ];
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
