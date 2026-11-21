//! German for the upstream round (2026-11-21): the mod safety review list, custom mod sources
//! in Browse, GameBanana notices with their own words, hidden mods and creators with what they
//! were, profiles keeping the Locker shuffle, and Discord's Hide party size. Kept apart from
//! the other tables so no two agents edit one file; chained into `i18n`'s maps. The duplicate
//! and coverage checks for the new files live here.

pub const DE: &[(&str, &str)] = &[
    // Discord
    ("Hide party size", "Gruppengröße ausblenden"),
    ("Leave out how many players are in your party, such as \"Party of 3\". Lines you wrote yourself show what you wrote.", "Lässt weg, wie viele Spieler in deiner Gruppe sind, etwa \"Party of 3\". Zeilen, die du selbst geschrieben hast, zeigen, was du geschrieben hast."),
    ("Leave the party size out of Discord.", "Die Gruppengröße nicht an Discord zeigen."),
    // GameBanana notices
    ("GameBanana staff withheld this mod. Its files may not download until the creator fixes it.", "Das GameBanana-Team hat diese Mod zurückgehalten. Ihre Dateien lassen sich womöglich nicht laden, bis der Ersteller sie korrigiert."),
    ("Withheld by {} on {}.", "Zurückgehalten von {} am {}."),
    ("Withheld by {}.", "Zurückgehalten von {}."),
    ("Withheld on {}.", "Zurückgehalten am {}."),
    ("Only the creator can see this mod right now.", "Nur der Ersteller kann diese Mod gerade sehen."),
    ("The creator marked this mod outdated. A newer mod may replace it.", "Der Ersteller hat diese Mod als veraltet markiert. Eine neuere Mod ersetzt sie vielleicht."),
    // Hidden mods and creators
    ("{} mods and {} creators hidden.", "{} Mods und {} Ersteller ausgeblendet."),
    ("Hidden {} from {}", "Ausgeblendet am {} bei {}"),
    ("Hidden {}", "Ausgeblendet am {}"),
    ("Hidden mod", "Ausgeblendete Mod"),
    ("1 upload", "1 Upload"),
    ("{} uploads", "{} Uploads"),
    ("Add a note", "Notiz hinzufügen"),
    ("Edit note", "Notiz bearbeiten"),
    ("Note for {}", "Notiz zu {}"),
    ("Why did you hide this?", "Warum hast du das ausgeblendet?"),

    // Mod safety review list
    ("Mod safety", "Mod-Sicherheit"),
    ("review and allowed mods", "zu prüfende und erlaubte Mods"),
    ("{} is off and back in Review.", "{} ist aus und wieder zur Prüfung."),
    ("Allowed and turned on {} script mods.", "{} Skript-Mods erlaubt und eingeschaltet."),
    ("1 mod needs attention", "1 Mod braucht deine Aufmerksamkeit"),
    ("{} mods need attention", "{} Mods brauchen deine Aufmerksamkeit"),
    ("They ship programs, scripts or network calls. Look at each one before it runs.", "Sie bringen Programme, Skripte oder Netzwerkaufrufe mit. Sieh dir jede an, bevor sie läuft."),
    ("Review mods", "Mods prüfen"),
    ("Dismiss mod safety notice", "Hinweis zur Mod-Sicherheit schließen"),
    ("Checking installed mods.", "Installierte Mods werden geprüft."),
    ("The check could not finish. Check installed mods again before playing.", "Die Prüfung wurde nicht fertig. Prüfe die installierten Mods vor dem Spielen erneut."),
    ("No mods need review.", "Keine Mod muss geprüft werden."),
    ("Allow script mods is on in Settings, so nothing asks before it turns on. The list still shows what each mod ships.", "Skript-Mods erlauben ist in den Einstellungen an, daher fragt nichts vor dem Einschalten. Die Liste zeigt trotzdem, was jede Mod mitbringt."),
    ("Sigil asks before turning on a mod that ships a program, a script the game does not run, a script mod or a network call. Allow only mods you trust.", "Sigil fragt, bevor eine Mod eingeschaltet wird, die ein Programm, ein Skript, das das Spiel nicht ausführt, eine Skript-Mod oder einen Netzwerkaufruf mitbringt. Erlaube nur Mods, denen du vertraust."),
    ("Check installed mods", "Installierte Mods prüfen"),
    ("Allows and turns on every mod below whose only finding is a script mod. Mods with other findings are allowed one at a time.", "Erlaubt und schaltet jede Mod unten ein, deren einziger Fund eine Skript-Mod ist. Mods mit anderen Funden werden einzeln erlaubt."),
    ("Allow {} script mods", "{} Skript-Mods erlauben"),
    ("Allowed", "Erlaubt"),
    ("View risks and actions for {}", "Risiken und Aktionen für {} ansehen"),
    ("You allowed this version. Disallow turns it off and asks again before it turns on.", "Du hast diese Version erlaubt. Nicht mehr erlauben schaltet sie aus und fragt vor dem nächsten Einschalten erneut."),
    ("This mod changed since you allowed it. Look at it again before allowing this version.", "Diese Mod hat sich geändert, seit du sie erlaubt hast. Sieh sie dir erneut an, bevor du diese Version erlaubst."),
    ("This mod is on, and nobody allowed this version. Keep it off, or allow it if you trust where it came from.", "Diese Mod ist an, und niemand hat diese Version erlaubt. Lass sie aus, oder erlaube sie, wenn du ihrer Herkunft vertraust."),
    ("Allowing this version turns the mod on with these findings.", "Wenn du diese Version erlaubst, wird die Mod mit diesen Funden eingeschaltet."),
    ("Disallow", "Nicht mehr erlauben"),
    ("Keep it off", "Ausgeschaltet lassen"),
    ("Allow this version", "Diese Version erlauben"),
    // Custom mod sources in Browse
    ("Mod source", "Mod-Quelle"),
    ("This source did not answer", "Diese Quelle hat nicht geantwortet"),
    ("Try another word.", "Versuche ein anderes Wort."),
    ("{} more. Search to narrow the list.", "{} weitere. Suche, um die Liste einzugrenzen."),
    ("Search {}", "{} durchsuchen"),
    ("Image {}", "Bild {}"),
    ("Updated {}", "Aktualisiert {}"),
    ("This release has no file to install.", "Diese Version hat keine Datei zum Installieren."),
    ("sha256 checked", "sha256 geprüft"),
    ("no checksum", "keine Prüfsumme"),
    ("Release history", "Versionsverlauf"),
    ("Reading the release history.", "Der Versionsverlauf wird gelesen."),
    ("No releases listed.", "Keine Versionen aufgeführt."),
    ("current", "aktuell"),
    ("Open the mod's page", "Seite der Mod öffnen"),
    // Street Brawl in Stats
    ("Street Brawl", "Street Brawl"),
    ("With Street Brawl", "Mit Street Brawl"),
    ("Without Street Brawl", "Ohne Street Brawl"),
    ("Street Brawl only", "Nur Street Brawl"),
];

/// Literals in the round's files that are not UI text.
#[cfg(test)]
const NOT_TEXT: &[&str] = &[
    "Mod", "Sound", "Wip", "GameBanana",
    // Scene fixtures: a source's own names and words
    "Abrams Voice Pack", "Clean HUD", "Community Mods", "First release.", "Haze Night Coat", "Kalo", "Red variant.",
];

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
            include_str!("locales_smalls.rs"),
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

    /// Every UI literal in the round's new files has German here or in another table: a
    /// literal that starts with a capital and has a lowercase letter.
    #[test]
    fn every_string_of_the_round_is_translated() {
        let sources = [include_str!("../browse/hidden.rs"), include_str!("../installed/safety_review.rs"), include_str!("../browse/sources.rs")];
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
