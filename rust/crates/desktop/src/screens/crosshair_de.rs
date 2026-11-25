//! German for the Crosshair page. `crosshair.rs` looks a string up here first, then in
//! Sigil's shared tables (`settings/locales*.rs`), so this page's strings stay out of the
//! shared files.

use std::collections::HashMap;
use std::sync::OnceLock;

pub const DE: &[(&str, &str)] = &[
    ("Plain", "Schlicht"),
    ("The game already has this crosshair", "Das Spiel hat dieses Fadenkreuz schon"),
    ("Console commands", "Konsolenbefehle"),
    ("Rename preset", "Vorlage umbenennen"),
    ("Delete preset", "Vorlage löschen"),
    ("Scaled to fit,", "Eingepasst,"),
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
    // Wave 4: the hero settings switch.
    ("Applies the crosshair as the game's default and keeps hero overrides. Turning it off restores your previous settings. Close Deadlock first.", "Wendet das Fadenkreuz als Standard des Spiels an und behält Helden-Ausnahmen. Ausschalten stellt deine vorherigen Einstellungen wieder her. Schließ Deadlock vorher."),
    ("Write to the game's hero settings", "In die Helden-Einstellungen des Spiels schreiben"),
    ("Writing the hero settings", "Helden-Einstellungen werden geschrieben"),
    ("On: {} of {} hero settings files take the crosshair. Apply writes it again; turning it off restores your previous settings.", "An: {} von {} Helden-Einstellungsdateien übernehmen das Fadenkreuz. Anwenden schreibt es erneut; Ausschalten stellt deine vorherigen Einstellungen wieder her."),
    ("On: the crosshair is the game's default, hero overrides stay. Turning it off restores your previous settings.", "An: Das Fadenkreuz ist der Standard des Spiels, Helden-Ausnahmen bleiben. Ausschalten stellt deine vorherigen Einstellungen wieder her."),
    ("Crosshair written to the hero settings", "Fadenkreuz in die Helden-Einstellungen geschrieben"),
    ("Launch Deadlock to use it.", "Starte Deadlock, um es zu nutzen."),
    ("Hero settings restored", "Helden-Einstellungen wiederhergestellt"),
    ("Your previous crosshair settings are back.", "Deine vorherigen Fadenkreuz-Einstellungen sind zurück."),
    ("The hero settings were not changed", "Die Helden-Einstellungen wurden nicht geändert"),
    ("Find the game folder first.", "Finde zuerst den Spielordner."),
    // Import from game and the page-off cleanup (`crosshair_game.rs`, Grimoire #504).
    ("No Deadlock install found, so there is no in-game crosshair to import.", "Keine Deadlock-Installation gefunden, daher gibt es kein Fadenkreuz aus dem Spiel zu importieren."),
    ("Read the in-game crosshair from {}.", "Das Fadenkreuz aus dem Spiel wurde aus {} gelesen."),
    ("The game has not saved a crosshair yet, so the defaults are shown.", "Das Spiel hat noch kein Fadenkreuz gespeichert, daher werden die Standardwerte gezeigt."),
    ("The Crosshair page is off, so the game keeps the crosshair you set in game.", "Die Fadenkreuz-Seite ist aus, daher behält das Spiel das Fadenkreuz, das du im Spiel eingestellt hast."),
    ("Could not take the crosshair out of autoexec", "Das Fadenkreuz konnte nicht aus autoexec entfernt werden"),
    ("autoexec.cfg still sets the crosshair Sigil wrote, so the game resets your in-game crosshair at every start.", "autoexec.cfg setzt noch das Fadenkreuz, das Sigil geschrieben hat, daher setzt das Spiel dein im Spiel eingestelltes Fadenkreuz bei jedem Start zurück."),
    ("1 use", "1 Nutzung"),
    ("{} uses", "{} Nutzungen"),
    ("by Sigil", "von Sigil"),
    ("Community", "Community"),
    ("Could not remove it from the library", "Konnte nicht aus der Bibliothek entfernt werden"),
    ("Could not save the library file.", "Die Bibliotheksdatei konnte nicht gespeichert werden."),
    ("Everyone using Sigil can find it under Community.", "Alle, die Sigil nutzen, finden es unter Community."),
    ("Everyone using Sigil will see this crosshair and its name. You can remove it from the library later.", "Alle, die Sigil nutzen, sehen dieses Fadenkreuz und seinen Namen. Du kannst es später wieder aus der Bibliothek entfernen."),
    ("It stays in the community library. Use Remove from library first to take it down.", "Es bleibt in der Community-Bibliothek. Nutze zuerst Aus Bibliothek entfernen, um es herauszunehmen."),
    ("Load into the editor", "In den Editor laden"),
    ("Load more", "Mehr laden"),
    ("Loading", "Wird geladen"),
    ("Loading the library", "Bibliothek wird geladen"),
    ("Most used", "Meistgenutzt"),
    ("Name", "Name"),
    ("Newest", "Neueste"),
    ("No crosshairs in the library yet.", "Noch keine Fadenkreuze in der Bibliothek."),
    ("Publish", "Veröffentlichen"),
    ("Publish to the library", "In der Bibliothek veröffentlichen"),
    ("Published to the library", "In der Bibliothek veröffentlicht"),
    ("Published, but Sigil could not keep the removal key", "Veröffentlicht, aber Sigil konnte den Schlüssel zum Entfernen nicht speichern"),
    ("Publishing", "Wird veröffentlicht"),
    ("Remove from library", "Aus Bibliothek entfernen"),
    ("Removed from the library", "Aus der Bibliothek entfernt"),
    ("Removing", "Wird entfernt"),
    ("Report", "Melden"),
    ("Report this crosshair?", "Dieses Fadenkreuz melden?"),
    ("Thanks", "Danke"),
    ("That preset no longer exists.", "Diese Vorlage gibt es nicht mehr."),
    ("The crosshair is hidden from your list.", "Das Fadenkreuz ist in deiner Liste ausgeblendet."),
    ("The crosshair library needs a connection. Your presets still work.", "Die Fadenkreuz-Bibliothek braucht eine Verbindung. Deine Vorlagen funktionieren weiter."),
    ("This crosshair is already in the library", "Dieses Fadenkreuz ist schon in der Bibliothek"),
    ("Try again", "Erneut versuchen"),
    ("{} is hidden from your list at once. Crosshairs that several players report are taken down for review.", "{} wird sofort aus deiner Liste ausgeblendet. Fadenkreuze, die mehrere Spieler melden, werden zur Prüfung herausgenommen."),
    ("The crosshair library needs a connection.", "Die Fadenkreuz-Bibliothek braucht eine Verbindung."),
    ("This crosshair is already in the library.", "Dieses Fadenkreuz ist schon in der Bibliothek."),
    ("You can publish 3 crosshairs a day. Try again tomorrow.", "Du kannst 3 Fadenkreuze am Tag veröffentlichen. Versuch es morgen wieder."),
    ("The library took all it can for today. Try again tomorrow.", "Die Bibliothek nimmt heute nichts mehr an. Versuch es morgen wieder."),
    ("That name is not allowed. Pick another one.", "Dieser Name ist nicht erlaubt. Wähle einen anderen."),
    ("Use 2 to 32 letters, digits, spaces or . - _ ' ! +", "Nutze 2 bis 32 Buchstaben, Ziffern, Leerzeichen oder . - _ ' ! +"),
    ("The library refused one of this crosshair's settings.", "Die Bibliothek hat eine Einstellung dieses Fadenkreuzes abgelehnt."),
    ("Too many requests. Wait a minute and try again.", "Zu viele Anfragen. Warte eine Minute und versuch es erneut."),
    ("The library is full right now.", "Die Bibliothek ist gerade voll."),
    ("That crosshair is no longer in the library.", "Dieses Fadenkreuz ist nicht mehr in der Bibliothek."),
    ("The library did not accept the removal key for this crosshair.", "Die Bibliothek hat den Schlüssel zum Entfernen für dieses Fadenkreuz nicht angenommen."),
    ("You reported 5 crosshairs today. Try again tomorrow.", "Du hast heute 5 Fadenkreuze gemeldet. Versuch es morgen wieder."),
    ("The crosshair library is not answering. Try again later.", "Die Fadenkreuz-Bibliothek antwortet nicht. Versuch es später erneut."),
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
        let extra = ["Copying", "Copied", "Copy code", "Unpin the window", "Keep the window above other windows"];
        // The library's messages come from deadlock-core and go through `tr` as values.
        use deadlock_core::crosshair_library::{LibraryError as E, name_rule};
        let library: Vec<String> = [
            E::Offline, E::Duplicate(String::new()), E::PublishLimit, E::DailyLimit, E::BlockedName, E::BadName,
            E::BadConfig(String::new()), E::RateLimited, E::LibraryFull, E::NotFound, E::WrongToken,
            E::ReportLimit, E::NotConfigured, E::Failed(String::new()),
        ]
        .iter()
        .map(E::message)
        .chain([name_rule().to_string()])
        .collect();
        let mut missing: Vec<String> = routed(include_str!("crosshair.rs"))
            .into_iter()
            .chain(routed(include_str!("crosshair_game.rs")))
            .chain(routed(include_str!("crosshair_community.rs")))
            .chain(extra.iter().map(|text| text.to_string()))
            .chain(library)
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
