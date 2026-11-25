//! German for "Help build Sigil" (`settings/product.rs`, ADR 0017). Kept apart from the other
//! tables so no two agents edit one file; chained into `i18n`'s maps. The coverage check reads
//! `product.rs` and fails on any UI string without German.

pub const DE: &[(&str, &str)] = &[
    ("Help build Sigil", "Hilf mit, Sigil zu verbessern"),
    (
        "Share how you use Sigil so it can suggest mods, warn about mods that break after a patch and show what frame rates to expect. Nothing is sent unless you choose Share, and you can turn it off any time in Settings, Privacy.",
        "Teile, wie du Sigil nutzt, damit es Mods vorschlagen, vor Mods warnen kann, die nach einem Patch kaputtgehen, und zeigen kann, welche Bildraten du erwarten kannst. Gesendet wird nur, wenn du Teilen wählst, und du kannst es jederzeit in Einstellungen, Datenschutz ausschalten.",
    ),
    (
        "Mods from GameBanana: the mod id when you install, remove, turn on or off, update, open, save or hide one, and which ones you use together.",
        "Mods von GameBanana: die Mod-ID, wenn du einen installierst, entfernst, ein- oder ausschaltest, aktualisierst, öffnest, speicherst oder ausblendest, und welche du zusammen nutzt.",
    ),
    (
        "Mod health: failed installs, overlaps, conflicts and repairs, by mod id and game build.",
        "Mod-Zustand: fehlgeschlagene Installationen, Überschneidungen, Konflikte und Reparaturen, nach Mod-ID und Spielversion.",
    ),
    (
        "Browse: filters, sort, your search text (short, lower case, never emails, paths, links or long numbers) and which suggestions you open.",
        "Durchsuchen: Filter, Sortierung, dein Suchtext (kurz, kleingeschrieben, nie E-Mails, Pfade, Links oder lange Zahlen) und welche Vorschläge du öffnest.",
    ),
    (
        "Play: your ten most played heroes in rough hours, and your rank tier.",
        "Spielen: deine zehn meistgespielten Helden in groben Stunden und deine Rangstufe.",
    ),
    (
        "Performance: configs you apply, benchmark frame rates, your GPU and CPU model, RAM rounded and screen size.",
        "Leistung: Konfigurationen, die du anwendest, Benchmark-Bildraten, dein GPU- und CPU-Modell, gerundeter Arbeitsspeicher und Bildschirmgröße.",
    ),
    (
        "App use: pages you open, theme, language, launches, how long Sigil stays open, and the Sigil, system and game version.",
        "App-Nutzung: Seiten, die du öffnest, Design, Sprache, Spielstarts, wie lange Sigil offen ist, und die Version von Sigil, System und Spiel.",
    ),
    (
        "Never sent: your Steam id or name, friends, matches, chat, file names or paths, local mods, mod files, keys, your IP address, or anything from the AI agent. Sent with a random id the server keeps only as a hash, and kept until you delete it with Delete my data in Settings. After two years with nothing new it is cut loose from your id.",
        "Nie gesendet: deine Steam-ID oder dein Name, Freunde, Matches, Chat, Dateinamen oder Pfade, lokale Mods, Mod-Dateien, Schlüssel, deine IP-Adresse oder irgendetwas vom KI-Agenten. Gesendet mit einer zufälligen ID, die der Server nur als Hash speichert, und aufbewahrt, bis du es mit Meine Daten löschen in den Einstellungen löschst. Nach zwei Jahren ohne Neues wird es von deiner ID getrennt.",
    ),
    (
        "Never sent: your Steam id or name, friends, matches, chat, file names or paths, local mods, mod files, keys or your IP address. Sent with a random id the server keeps only as a hash, and kept until you delete it with Delete my data in Settings. After two years with nothing new it is cut loose from your id.",
        "Nie gesendet: deine Steam-ID oder dein Name, Freunde, Matches, Chat, Dateinamen oder Pfade, lokale Mods, Mod-Dateien, Schlüssel oder deine IP-Adresse. Gesendet mit einer zufälligen ID, die der Server nur als Hash speichert, und aufbewahrt, bis du es mit Meine Daten löschen in den Einstellungen löschst. Nach zwei Jahren ohne Neues wird es von deiner ID getrennt.",
    ),
    ("Not now", "Jetzt nicht"),
    ("Share", "Teilen"),
    ("Share product data", "Produktdaten teilen"),
    (
        "Mods, Browse, play, performance and app use, to Sigil's own server, kept until you delete it. Off unless you turn it on.",
        "Mods, Durchsuchen, Spielen, Leistung und App-Nutzung, an Sigils eigenen Server, aufbewahrt bis du es löschst. Aus, bis du es einschaltest.",
    ),
    ("What Sigil shared", "Was Sigil geteilt hat"),
    ("The last records sent from this computer.", "Die letzten Einträge, die von diesem Computer gesendet wurden."),
    ("Delete my data", "Meine Daten löschen"),
    (
        "Deletes the records Sigil's server keeps from this computer, clears what waits to be sent and starts a new id. Totals already counted with no id stay.",
        "Löscht die Einträge, die Sigils Server von diesem Computer hat, leert, was noch auf das Senden wartet, und startet eine neue ID. Bereits gezählte Summen ohne ID bleiben.",
    ),
    ("Deleting", "Wird gelöscht"),
    ("Your data was deleted", "Deine Daten wurden gelöscht"),
    (
        "Sigil's server deleted the records from this computer. A new id is made if you share again.",
        "Sigils Server hat die Einträge von diesem Computer gelöscht. Wenn du wieder teilst, wird eine neue ID erstellt.",
    ),
    ("Your data was not deleted", "Deine Daten wurden nicht gelöscht"),
    ("Try again later: {}", "Versuche es später noch einmal: {}"),
    ("Nothing has been sent yet.", "Bisher wurde nichts gesendet."),
    (
        "The last {} records sent from this computer, newest first. Sigil keeps this list only here.",
        "Die letzten {} Einträge, die von diesem Computer gesendet wurden, neueste zuerst. Sigil speichert diese Liste nur hier.",
    ),
];

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    /// String literals in `source` that look like UI text: start with a capital, have a lowercase
    /// letter, and are not format strings for code.
    fn ui_literals(source: &str) -> Vec<String> {
        let code = source.split("#[cfg(test)]").next().unwrap_or(source);
        let mut out = Vec::new();
        let mut rest = code;
        while let Some(start) = rest.find('"') {
            let after = &rest[start + 1..];
            let Some(end) = after.find('"') else { break };
            let literal = &after[..end];
            rest = &after[end + 1..];
            let line_start = code.len() - rest.len();
            let _ = line_start;
            let ui = literal.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                && literal.chars().any(|c| c.is_ascii_lowercase())
                && !literal.contains("{:")
                && !literal.contains("://");
            if ui {
                out.push(literal.to_string());
            }
        }
        out
    }

    #[test]
    #[cfg_attr(not(feature = "languages"), ignore = "translations are parked: --features languages")]
    fn every_string_of_help_build_sigil_has_german() {
        let mut missing: Vec<String> = ui_literals(include_str!("product.rs"))
            .into_iter()
            .filter(|s| super::super::i18n::german_of(s).is_none())
            .collect();
        missing.sort();
        missing.dedup();
        assert!(missing.is_empty(), "untranslated: {missing:#?}");
    }

    #[test]
    #[cfg_attr(not(feature = "languages"), ignore = "translations are parked: --features languages")]
    fn the_table_has_no_duplicates_keeps_holes_and_no_em_dash() {
        let mut seen = HashSet::new();
        for (en, de) in super::DE {
            assert!(seen.insert(*en), "duplicate key {en:?}");
            assert_eq!(en.matches("{}").count(), de.matches("{}").count(), "holes in {en:?}");
            assert!(!de.contains('\u{2014}') && !en.contains('\u{2014}'), "em dash in {en:?}");
        }
    }

    /// Sigil has opt-in telemetry (ADR 0014) and product data (ADR 0017), so the app must not
    /// claim it has none. Fails on "no telemetry" or "never sends" style absolutes in any string
    /// of the desktop app, English or German. Comments are not checked.
    #[test]
    fn no_app_string_claims_sigil_sends_nothing() {
        const ABSOLUTES: [&str; 8] = [
            "no telemetry",
            "keine telemetrie",
            "no analytics",
            "keine analyse",
            "no crash reporting",
            "keine absturzmeldungen",
            "never sends",
            "sendet nie",
        ];
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![root];
        let mut found = Vec::new();
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") || path.ends_with("locales_product.rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                for (n, line) in text.lines().enumerate() {
                    let trimmed = line.trim_start();
                    if trimmed.starts_with("//") {
                        continue;
                    }
                    let lower = line.to_lowercase();
                    if ABSOLUTES.iter().any(|a| lower.contains(a)) {
                        found.push(format!("{}:{}", path.display(), n + 1));
                    }
                }
            }
        }
        assert!(found.is_empty(), "absolute privacy claims: {found:#?}");
    }

    /// A key here that another table also has would make one of them win at random.
    #[test]
    #[cfg_attr(not(feature = "languages"), ignore = "translations are parked: --features languages")]
    fn keys_are_not_in_another_table() {
        let ours: HashSet<&str> = super::DE.iter().map(|(en, _)| *en).collect();
        for (en, de) in super::DE {
            assert_eq!(super::super::i18n::german_of(en), Some(*de), "{en:?} is translated elsewhere too");
        }
        assert_eq!(ours.len(), super::DE.len());
    }
}
