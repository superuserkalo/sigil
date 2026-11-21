//! German for the smalls round: the background under the status bar, the volume bar art, the
//! update download progress, the package manager note and Custom maps (`settings/smalls.rs`,
//! `screens/maps.rs`). Kept apart from the other tables so no two agents edit one file; chained
//! into `i18n`'s maps. The duplicate and coverage checks for these strings live in this file.

pub const DE: &[(&str, &str)] = &[
    // Background and art
    ("Behind the status bar", "Hinter der Statusleiste"),
    ("The image runs under the status bar at the bottom of the window.", "Das Bild reicht unter die Statusleiste am unteren Fensterrand."),
    ("The status bar is hidden in Settings, so this has no effect until it is back.", "Die Statusleiste ist in den Einstellungen ausgeblendet, daher wirkt das erst, wenn sie wieder da ist."),
    ("Volume bar", "Lautstärkeleiste"),
    ("The volume bar is the speaker at the right end of the status bar.", "Die Lautstärkeleiste ist der Lautsprecher am rechten Ende der Statusleiste."),
    // Updates
    ("{} of {} ({}%)", "{} von {} ({} %)"),
    ("Updates are managed by your package manager", "Updates kommen von deiner Paketverwaltung"),
    ("Updates for this copy come from {}. Update Sigil there.", "Updates für diese Kopie kommen von {}. Aktualisiere Sigil dort."),
    ("Updates for this copy come from your system's package manager. Update Sigil there.", "Updates für diese Kopie kommen von der Paketverwaltung deines Systems. Aktualisiere Sigil dort."),
    ("This copy came from Sigil's .deb package. Install the new .deb from the release page, or switch to the AppImage, which updates itself.", "Diese Kopie stammt aus Sigils .deb-Paket. Installiere das neue .deb von der Release-Seite oder wechsle zum AppImage, das sich selbst aktualisiert."),
    // Custom maps
    ("Maps", "Karten"),
    ("Custom maps", "Eigene Karten"),
    ("Adds a Maps page with GameBanana's maps, and How to play on map mods: launch a map and invite friends.", "Fügt eine Kartenseite mit den Karten von GameBanana hinzu und So spielst du bei Karten-Mods: Karte starten und Freunde einladen."),
    ("Custom maps from GameBanana.", "Eigene Karten von GameBanana."),
    ("Custom maps from GameBanana. Install one, then launch it and invite friends.", "Eigene Karten von GameBanana. Installiere eine, starte sie und lade Freunde ein."),
    ("How to play", "So spielst du"),
    ("Copied", "Kopiert"),
    ("Refresh", "Aktualisieren"),
    ("Launch map", "Karte starten"),
    ("Host this map", "Diese Karte hosten"),
    ("Open page", "Seite öffnen"),
    ("Invite friends", "Freunde einladen"),
    ("Stop waiting", "Nicht mehr warten"),
    ("Watch again", "Erneut warten"),
    ("Installed maps", "Installierte Karten"),
    ("On GameBanana", "Auf GameBanana"),
    ("Getting maps", "Karten werden geladen"),
    ("The list comes from GameBanana.", "Die Liste kommt von GameBanana."),
    ("Maps could not load", "Karten konnten nicht geladen werden"),
    ("No maps yet", "Noch keine Karten"),
    ("Nobody has posted a map for Deadlock.", "Noch niemand hat eine Karte für Deadlock veröffentlicht."),
    ("{}, by {}", "{}, von {}"),
    ("by {}  ·  {} likes", "von {}  ·  {} Likes"),
    ("The next launch starts {}.", "Der nächste Start lädt {}."),
    ("Every launch starts {} until you remove it.", "Jeder Start lädt {}, bis du es entfernst."),
    ("The map launch is removed.", "Der Kartenstart ist entfernt."),
    ("The map could not be set", "Die Karte konnte nicht eingetragen werden"),
    ("Deadlock is running. Close it, then launch the map.", "Deadlock läuft. Schließe es und starte dann die Karte."),
    ("Deadlock was not found. Nothing was changed.", "Deadlock wurde nicht gefunden. Nichts wurde geändert."),
    ("Open the console in Deadlock (F7) and run this, or let Sigil start the map:", "Öffne in Deadlock die Konsole (F7) und führe das aus, oder lass Sigil die Karte starten:"),
    ("Sigil could not find this map's name. Check the author's description for the command to use.", "Sigil hat den Namen dieser Karte nicht gefunden. In der Beschreibung des Autors steht, welcher Befehl gilt."),
    ("Install the map first. Friends who join need it installed too.", "Installiere die Karte zuerst. Freunde, die beitreten, brauchen sie auch."),
    ("This map is switched off. Turn it on in Installed, then launch it.", "Diese Karte ist ausgeschaltet. Schalte sie unter Installiert ein und starte sie dann."),
    ("The installed files carry no map under maps/. The map may load under another name.", "Die installierten Dateien enthalten keine Karte unter maps/. Die Karte lädt vielleicht unter einem anderen Namen."),
    ("Launch map writes the map command into autoexec.cfg (the old file is backed up) and starts Deadlock.", "Karte starten schreibt den Kartenbefehl in die autoexec.cfg (die alte Datei wird gesichert) und startet Deadlock."),
    ("Your map server is up. Friends open the console in Deadlock (F7), paste this and press Enter:", "Dein Kartenserver läuft. Freunde öffnen in Deadlock die Konsole (F7), fügen das ein und drücken Enter:"),
    ("They need the same map installed.", "Sie brauchen dieselbe Karte installiert."),
    ("Waiting for the map server. The connect command shows here when it is up.", "Warte auf den Kartenserver. Der Verbindungsbefehl erscheint hier, sobald er läuft."),
    ("No connect code showed up. Sigil reads it from console.log, which the game writes with -condebug.", "Es kam kein Verbindungscode. Sigil liest ihn aus der console.log, die das Spiel mit -condebug schreibt."),
    ("Launch the map and Sigil shows the command friends paste in their console (F7) to join you.", "Starte die Karte, und Sigil zeigt den Befehl, den Freunde in ihre Konsole (F7) einfügen, um dir beizutreten."),
    ("Sigil reads the command from the game's console log. Turn on the console log (-condebug) in Settings first.", "Sigil liest den Befehl aus dem Konsolenprotokoll des Spiels. Schalte zuerst das Konsolenprotokoll (-condebug) in den Einstellungen ein."),
    ("Reading the map's page for its name.", "Die Seite der Karte wird nach ihrem Namen durchsucht."),
    ("{} is no longer set as the start map.", "{} ist nicht mehr als Startkarte gesetzt."),
    ("Use Launch map on the Maps page to start a map again.", "Starte eine Karte erneut mit Karte starten auf der Kartenseite."),
];

/// Literals in the routed files that are not UI text: scene fixtures (map and author names).
#[cfg(test)]
const NOT_TEXT: &[&str] = &["Street Ball", "Mindscape", "Coal Town", "Movement Practice", "Mod"];

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    /// Every English key in Sigil's other global German tables, read from their source.
    fn others() -> HashSet<String> {
        let sources = [
            include_str!("locales.rs"),
            include_str!("locales_parity.rs"),
            include_str!("locales_wave.rs"),
            include_str!("locales_w4a.rs"),
            include_str!("locales_w4b.rs"),
            include_str!("locales_w4c.rs"),
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

    /// Every UI literal in the smalls files has German here or in another table: a literal that
    /// starts with a capital and has a lowercase letter.
    #[test]
    fn every_string_of_the_round_is_translated() {
        let sources = [include_str!("smalls.rs"), include_str!("../maps.rs")];
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

    /// The new flag, page and art labels other files hand to `tr`.
    #[test]
    fn labels_from_other_files_are_here() {
        let table: HashSet<&str> = super::DE.iter().map(|(en, _)| *en).collect();
        let flag = crate::prefs::FLAGS.iter().find(|f| f.id == crate::screens::maps::FLAG).expect("flag");
        assert!(table.contains(flag.title) && table.contains(flag.line));
        assert!(table.contains(crate::model::Screen::Maps.title()));
        assert!(table.contains(crate::ui::personalize::Surface::Volume.label()));
    }

    /// Plain string literals, skipping line comments and char literals.
    fn literals(source: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut chars = source.chars().peekable();
        let mut prev = ' ';
        while let Some(c) = chars.next() {
            if c == '/' && chars.peek() == Some(&'/') && prev != ':' {
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
                prev = '\'';
                continue;
            }
            if c == '"' {
                let mut s = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            if let Some(next) = chars.next() {
                                s.push(match next {
                                    'n' => '\n',
                                    other => other,
                                });
                            }
                        }
                        '"' => break,
                        other => s.push(other),
                    }
                }
                out.push(s);
                prev = '"';
                continue;
            }
            prev = c;
        }
        out
    }
}
