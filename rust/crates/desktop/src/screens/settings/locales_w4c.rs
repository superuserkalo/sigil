//! German for the strings parity wave 4 (UI C) added: the Locker's Game art, Game sounds, sound
//! swaps and In-game card. Kept apart from the other tables so no two agents edit one file;
//! `i18n` reads it next to the others (declared there with `#[path]`). The duplicate and
//! routed-file checks for these strings live in this file's tests.

pub const DE: &[(&str, &str)] = &[
// BEGIN TABLE
    // The In-game card dialog's 3D view (`locker/card_art.rs`).
    ("Use the 3D view", "3D-Ansicht verwenden"),
    ("Use the 3D view for {}", "3D-Ansicht für {} verwenden"),
    ("Open the hero's 3D view first, then try again.", "Öffne zuerst die 3D-Ansicht des Helden und versuche es dann erneut."),
    ("Cards and portraits", "Karten und Porträts"),
    ("Ability icons", "Fähigkeitssymbole"),
    ("Skin textures", "Skin-Texturen"),
    ("Ability effects", "Fähigkeitseffekte"),
    ("Ability", "Fähigkeit"),
    ("Weapon", "Waffe"),
    ("Movement", "Bewegung"),
    ("Melee", "Nahkampf"),
    ("Other sounds", "Andere Sounds"),
    ("ULT", "ULT"),
    ("The file could not be read.", "Die Datei konnte nicht gelesen werden."),
    ("Game art", "Spielgrafik"),
    ("Game sounds", "Spielsounds"),
    ("In-game card", "Karte im Spiel"),
    ("{} textures", "{} Texturen"),
    ("Reading the game's textures...", "Texturen des Spiels werden gelesen..."),
    ("The game's textures could not be read", "Die Texturen des Spiels konnten nicht gelesen werden"),
    ("The game files have no textures for {}.", "Die Spieldateien haben keine Texturen für {}."),
    ("No textures match", "Keine Textur passt"),
    ("Try another kind or a shorter search.", "Versuche eine andere Art oder eine kürzere Suche."),
    ("Showing {} of {}", "{} von {} angezeigt"),
    ("Show more", "Mehr anzeigen"),
    ("Search textures", "Texturen suchen"),
    ("Texture kind", "Texturart"),
    ("Open {}", "{} öffnen"),
    ("Native size {} × {}", "Originalgröße {} × {}"),
    ("Copy path", "Pfad kopieren"),
    ("Path copied", "Pfad kopiert"),
    ("Replace {}", "{} ersetzen"),
    ("The file picker could not open", "Die Dateiauswahl konnte nicht geöffnet werden"),
    ("{} sounds", "{} Sounds"),
    ("Reading sounds from the game files...", "Sounds werden aus den Spieldateien gelesen..."),
    ("The game's sounds could not be read", "Die Sounds des Spiels konnten nicht gelesen werden"),
    ("The game files have no sounds for {}.", "Die Spieldateien haben keine Sounds für {}."),
    ("Search sounds", "Sounds suchen"),
    ("No sounds match the search.", "Kein Sound passt zur Suche."),
    ("{}, {} sounds", "{}, {} Sounds"),
    ("Showing the first {}. Search to narrow the list.", "Die ersten {} werden angezeigt. Suche, um die Liste einzugrenzen."),
    ("Stop {}", "{} stoppen"),
    ("Play {}", "{} abspielen"),
    ("{} clips", "{} Clips"),
    ("Replaced", "Ersetzt"),
    ("Replace {} with your MP3", "{} durch deine MP3 ersetzen"),
    ("Loop: as the game", "Schleife: wie im Spiel"),
    ("Loop: on", "Schleife: an"),
    ("Loop: off", "Schleife: aus"),
    ("Your swaps", "Deine Tausche"),
    ("Remove the swap of {}", "Tausch von {} entfernen"),
    ("Every clip of a swapped sound plays your audio ({} clips in all).", "Jeder Clip eines getauschten Sounds spielt dein Audio ({} Clips insgesamt)."),
    ("Mod name", "Mod-Name"),
    ("Building...", "Wird gebaut..."),
    ("Build mod", "Mod bauen"),
    ("Choose an image for {}", "Bild für {} wählen"),
    ("That file is not a PNG, JPEG or WebP image.", "Diese Datei ist kein PNG-, JPEG- oder WebP-Bild."),
    ("Choose an image for at least one card.", "Wähle ein Bild für mindestens eine Karte."),
    ("{} card art", "{} Kartengrafik"),
    ("Your image is {} × {}, so it will look soft.", "Dein Bild ist {} × {} groß und wirkt daher unscharf."),
    ("Clear the image for {}", "Bild für {} entfernen"),
    ("In-game card for {}", "Karte im Spiel für {}"),
    ("Choose an image for each card the game shows. It is cropped to the card's shape from the centre. Cards you leave empty keep the game's art.", "Wähle ein Bild für jede Karte, die das Spiel zeigt. Es wird von der Mitte aus auf die Form der Karte zugeschnitten. Leere Karten behalten die Grafik des Spiels."),
    ("Reading the hero's cards from the game files...", "Karten des Helden werden aus den Spieldateien gelesen..."),
    ("The hero's cards could not be read", "Die Karten des Helden konnten nicht gelesen werden"),
    ("The game files have no card art for {}.", "Die Spieldateien haben keine Kartengrafik für {}."),
    ("Sigil packs the cards into a new mod and opens the import review. It lands switched off.", "Sigil packt die Karten in eine neue Mod und öffnet die Importprüfung. Sie kommt ausgeschaltet an."),
    ("{} of the cards will change. The mod lands switched off.", "{} der Karten ändern sich. Die Mod kommt ausgeschaltet an."),
    ("Save the preview as PNG", "Vorschau als PNG speichern"),
    ("Save preview", "Vorschau speichern"),
    ("Preview saved", "Vorschau gespeichert"),
    ("The preview could not be saved", "Die Vorschau konnte nicht gespeichert werden"),
    ("The preview is a copy of at most 256 pixels a side. Save preview writes that copy.", "Die Vorschau ist eine Kopie mit höchstens 256 Pixeln pro Seite. Vorschau speichern schreibt diese Kopie."),
    ("Sound saved", "Sound gespeichert"),
    ("The sound could not be saved", "Der Sound konnte nicht gespeichert werden"),
    ("Save the sound", "Sound speichern"),
    ("Audio", "Audio"),
    ("MP3 or WAV", "MP3 oder WAV"),
    ("Replace in game", "Im Spiel ersetzen"),
    ("Replace this icon", "Dieses Symbol ersetzen"),
    ("The game files could not be opened.", "Die Spieldateien konnten nicht geöffnet werden."),
    ("The texture could not be read.", "Die Textur konnte nicht gelesen werden."),
    ("Wolf {}", "Wolf {}"),
    ("Save {} as a file", "{} als Datei speichern"),
    ("Change the volume of {}", "Lautstärke von {} ändern"),
    ("Volume", "Lautstärke"),
    ("The game's sound, louder or quieter", "Der Sound des Spiels, lauter oder leiser"),
    ("Volume of {}", "Lautstärke von {}"),
    ("Quieter", "Leiser"),
    ("Louder", "Lauter"),
    ("Checking...", "Wird geprüft..."),
    ("Build anyway", "Trotzdem bauen"),
    ("A sound marked above is shared: other heroes play the same clips, so the swap changes it for them too. Build anyway, or remove that swap.", "Ein oben markierter Sound wird geteilt: Andere Helden spielen dieselben Clips, also ändert der Tausch ihn auch für sie. Baue trotzdem oder entferne diesen Tausch."),
    ("Replace takes an MP3. A 16-bit WAV works only for sounds the game stores uncompressed.", "Ersetzen nimmt eine MP3. Eine 16-Bit-WAV geht nur für Sounds, die das Spiel unkomprimiert speichert."),
    ("Swaps take an MP3 (or a 16-bit WAV for sounds the game stores uncompressed). Convert OGG, FLAC and other audio to MP3 first.", "Tausche nehmen eine MP3 (oder eine 16-Bit-WAV für Sounds, die das Spiel unkomprimiert speichert). Wandle OGG, FLAC und anderes Audio zuerst in MP3 um."),
    ("Each swap becomes its own mod, named after the sound. Sigil opens the import review; they land switched off.", "Jeder Tausch wird eine eigene Mod, benannt nach dem Sound. Sigil öffnet die Importprüfung; sie kommen ausgeschaltet an."),
    ("Sigil packs your swap into a new mod and opens the import review. It lands switched off.", "Sigil packt deinen Tausch in eine neue Mod und öffnet die Importprüfung. Sie kommt ausgeschaltet an."),
    ("Other heroes play these clips too.", "Andere Helden spielen diese Clips auch."),
    ("Also plays for {}.", "Spielt auch für {}."),
    ("Also plays for {} and {} more.", "Spielt auch für {} und {} weitere."),
    ("Takes a PNG. A JPEG or WebP is turned into a PNG first.", "Nimmt ein PNG. Ein JPEG oder WebP wird vorher in ein PNG umgewandelt."),
    ("Choose an image. It is cropped to the texture's shape from the centre and replaces the game's art.", "Wähle ein Bild. Es wird von der Mitte aus auf die Form der Textur zugeschnitten und ersetzt die Grafik des Spiels."),
    ("Replace {} in game", "{} im Spiel ersetzen"),
// END TABLE
];

/// Literals in the routed files that read as words but are not UI text: scene fixtures.
#[cfg_attr(not(test), allow(dead_code))]
pub const NOT_TEXT: &[&str] = &[
    "Haze",
    "Haze sounds",
    "Sleep Dagger",
    "Smoke Bomb",
    "Bullet Dance",
    "Dagger throw",
    "Dagger hit",
    "Smoke bomb",
    "Bullet dance start",
    "Bullet dance loop",
    "Pistol shot",
    "Reload",
    "Melee swing",
    "Footstep",
    "Card",
    "Vertical",
    "Low HP",
    "Gloat",
    "Minimap",
    "Small",
    "Haze.SleepDagger.Cast",
    "Haze.SleepDagger.Impact",
    "Haze.SmokeBomb.Activate",
    "Haze.BulletDance.Start",
    "Haze.BulletDance.Loop",
    "Haze.Weapon.Fire",
    "Haze.Weapon.Reload",
    "Haze.Melee.Swing",
    "Haze.Footstep",
    "Wraith",
    "Vindicta",
];

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    fn others() -> HashSet<&'static str> {
        super::super::super::locales::DE
            .iter()
            .chain(super::super::super::locales_parity::DE)
            .chain(super::super::super::locales_wave::DE)
            .chain(super::super::super::locales_w4b::DE)
            .chain(super::super::locales_w4a::DE)
            .map(|(en, _)| *en)
            .collect()
    }

    #[test]
    fn the_table_has_no_duplicates_and_keeps_holes() {
        let others = others();
        let mut seen = HashSet::new();
        for (en, de) in super::DE {
            assert!(seen.insert(*en), "duplicate key {en:?}");
            assert!(!others.contains(en), "{en:?} is already in another table");
            assert_eq!(en.matches("{}").count(), de.matches("{}").count(), "holes in {en:?}");
            assert!(!de.contains('\u{2014}') && !en.contains('\u{2014}'), "em dash in {en:?}");
        }
    }

    /// Every UI literal in this wave's files has a German entry somewhere. Same rule as
    /// `i18n::every_routed_string_is_translated`: a literal that starts with a capital and has a
    /// lowercase letter.
    #[test]
    fn every_string_in_the_new_screens_is_translated() {
        let sources = [
            include_str!("../locker/assets.rs"),
            include_str!("../locker/catalog.rs"),
            include_str!("../locker/game_sounds.rs"),
            include_str!("../locker/card_art.rs"),
            include_str!("../w4c_scenes.rs"),
        ];
        let mut table = others();
        table.extend(super::DE.iter().map(|(en, _)| *en));
        let mut missing = Vec::new();
        for source in sources {
            let code = source.split("#[cfg(test)]").next().unwrap_or(source);
            for literal in literals(code) {
                let ui = literal.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                    && literal.chars().any(|c| c.is_ascii_lowercase())
                    && !(literal.contains('{') && !literal.contains("{}"))
                    && !literal.contains("://");
                if ui && !table.contains(literal.as_str()) && !super::NOT_TEXT.contains(&literal.as_str()) {
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
