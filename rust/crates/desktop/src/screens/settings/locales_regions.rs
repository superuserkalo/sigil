//! German for the Regions tab (Servers > Regions, the relay region picker). Kept apart from the
//! other tables so no two agents edit one file; declared in `screens/servers_regions/mod.rs` and
//! chained into `i18n`'s maps. The duplicate and routed-file checks for these strings live here.

pub const DE: &[(&str, &str)] = &[
    // Search, sort, Where I play and relay health (round 2)
    ("Fastest: {} ({} ms)", "Am schnellsten: {} ({} ms)"),
    ("Loaded Where I play. Press Apply to block the locations you did not play on.", "Wo ich spiele geladen. Drücke Übernehmen, um die Standorte zu sperren, auf denen du nicht gespielt hast."),
    ("Measure again", "Erneut messen"),
    ("Measuring relay health", "Relay-Zustand wird gemessen"),
    ("Measuring...", "Wird gemessen..."),
    ("My matches", "Meine Matches"),
    ("Name", "Name"),
    ("No location matches {}", "Kein Standort passt zu {}"),
    ("Ping", "Ping"),
    (
        "Ping from this computer to Valve's relays. It says how far each region is, not whether Deadlock's servers are up.",
        "Ping von diesem Computer zu Valves Relays. Er zeigt, wie weit jede Region entfernt ist, nicht ob Deadlocks Server laufen.",
    ),
    ("Relay health", "Relay-Zustand"),
    ("Relay health in the status bar", "Relay-Zustand in der Statusleiste"),
    ("Relays {}/{}", "Relays {}/{}"),
    ("Relays: measuring", "Relays: wird gemessen"),
    ("Search by city, country or the location's short id.", "Suche nach Stadt, Land oder dem Kürzel des Standorts."),
    ("Search locations", "Standorte suchen"),
    ("Shift-click ticks a range", "Umschalt-Klick markiert einen Bereich"),
    ("Sort", "Sortieren"),
    ("Where I play", "Wo ich spiele"),
    (
        "Where I play: block every location you had no match on in the last {} months",
        "Wo ich spiele: jeden Standort sperren, auf dem du in den letzten {} Monaten kein Match hattest",
    ),
    ("{} of {} relay locations answer. Show relay health", "{} von {} Relay-Standorten antworten. Relay-Zustand zeigen"),
    ("{} open", "{} offen"),
    ("1 match", "1 Match"),
    ("{} matches", "{} Matches"),
    ("Regions", "Regionen"),
    ("North America", "Nordamerika"),
    ("South America", "Südamerika"),
    ("Europe", "Europa"),
    ("Middle East", "Naher Osten"),
    ("Africa", "Afrika"),
    ("Asia", "Asien"),
    ("Oceania", "Ozeanien"),
    ("Other locations", "Weitere Standorte"),
    ("A preset name is 1 to 60 characters.", "Ein Vorlagenname hat 1 bis 60 Zeichen."),
    ("Sigil keeps at most 50 presets. Delete one first.", "Sigil speichert höchstens 50 Vorlagen. Lösche zuerst eine."),
    ("Loaded {}. Press Apply to block these regions.", "{} geladen. Drücke Übernehmen, um diese Regionen zu sperren."),
    ("Offline mode is on. Turn it off in Settings to measure pings.", "Der Offlinemodus ist an. Schalte ihn in den Einstellungen aus, um Pings zu messen."),
    ("Set the game folder in Settings first: the rules apply to Deadlock only.", "Lege zuerst den Spielordner in den Einstellungen fest: Die Regeln gelten nur für Deadlock."),
    ("Default routing is back. No relay region is blocked.", "Das normale Routing ist zurück. Keine Relay-Region ist gesperrt."),
    ("Some relay rules are still there.", "Einige Relay-Regeln sind noch da."),
    (
        "Press Reset again. If it keeps failing, remove the Sigil-Relay rules in Windows Defender Firewall.",
        "Drücke erneut Zurücksetzen. Klappt es weiterhin nicht, entferne die Sigil-Relay-Regeln in der Windows Defender Firewall.",
    ),
    ("{} relay locations blocked for Deadlock.", "{} Relay-Standorte für Deadlock gesperrt."),
    (
        "Restart Deadlock if it is running. Reset in Servers > Regions brings default routing back.",
        "Starte Deadlock neu, falls es läuft. Zurücksetzen unter Server > Regionen stellt das normale Routing wieder her.",
    ),
    ("Windows Firewall did not take every rule.", "Die Windows-Firewall hat nicht jede Regel übernommen."),
    ("Press Apply again, or Reset to remove what is there.", "Drücke erneut Übernehmen oder Zurücksetzen, um das Vorhandene zu entfernen."),
    ("Windows Firewall is off for this network", "Die Windows-Firewall ist für dieses Netzwerk aus"),
    ("The rules are in place but do nothing until the firewall is on.", "Die Regeln sind gesetzt, wirken aber erst, wenn die Firewall an ist."),
    ("{} relay locations are blocked", "{} Relay-Standorte sind gesperrt"),
    (
        "Matchmaking only uses the open regions, so queues can be longer. Reset in Servers > Regions.",
        "Das Matchmaking nutzt nur die offenen Regionen, daher kann die Suche länger dauern. Zurücksetzen unter Server > Regionen.",
    ),
    ("Open Regions", "Regionen öffnen"),
    ("{} relay locations blocked", "{} Relay-Standorte gesperrt"),
    ("{} relay locations blocked. Open Servers, Regions", "{} Relay-Standorte gesperrt. Server, Regionen öffnen"),
    ("Valve's relay list could not be loaded", "Valves Relay-Liste konnte nicht geladen werden"),
    ("Loading Valve's relay locations", "Valves Relay-Standorte werden geladen"),
    (
        "Queues can take longer: fewer regions means fewer players to match with.",
        "Die Suche kann länger dauern: Weniger Regionen heißt weniger Spieler für ein Match.",
    ),
    (
        "A block you forget about looks like broken matchmaking. The status bar shows it on every page while one is on.",
        "Eine vergessene Sperre sieht aus wie kaputtes Matchmaking. Die Statusleiste zeigt sie auf jeder Seite, solange eine aktiv ist.",
    ),
    (
        "It steers matchmaking; it does not pick a server. Only Deadlock is affected, not other games.",
        "Es lenkt das Matchmaking, wählt aber keinen Server. Betroffen ist nur Deadlock, keine anderen Spiele.",
    ),
    (
        "Deadlock reaches its matches through Valve's relay network. Tick the locations you do not want, press Apply, and Sigil blocks their relays in Windows Firewall so matchmaking places you in the open regions. Use it to avoid far regions, language barriers, or to agree on a region with friends.",
        "Deadlock erreicht seine Matches über Valves Relay-Netz. Hake die Standorte an, die du nicht willst, drücke Übernehmen, und Sigil sperrt ihre Relays in der Windows-Firewall, sodass das Matchmaking dich in die offenen Regionen setzt. Nutze es gegen weit entfernte Regionen, Sprachbarrieren oder um dich mit Freunden auf eine Region zu einigen.",
    ),
    ("Before you block a region", "Bevor du eine Region sperrst"),
    ("Blocking needs Windows", "Sperren braucht Windows"),
    (
        "Sigil blocks relays with Windows Firewall rules for deadlock.exe, so it can only block on Windows. The list and the pings work here, to see which regions are close.",
        "Sigil sperrt Relays mit Windows-Firewall-Regeln für deadlock.exe, daher geht das Sperren nur unter Windows. Die Liste und die Pings funktionieren hier, um zu sehen, welche Regionen nah sind.",
    ),
    ("Waiting for Windows", "Warten auf Windows"),
    (
        "Press Yes in Windows' admin prompt to change the firewall. No leaves it as it is.",
        "Drücke Ja in der Administratorabfrage von Windows, um die Firewall zu ändern. Nein lässt sie, wie sie ist.",
    ),
    ("Reading Windows Firewall", "Windows-Firewall wird gelesen"),
    ("Firewall not checked yet", "Firewall noch nicht geprüft"),
    ("Windows Firewall could not be read", "Die Windows-Firewall konnte nicht gelesen werden"),
    ("Default routing: every region is open", "Normales Routing: Jede Region ist offen"),
    ("Sigil has no relay rules in Windows Firewall.", "Sigil hat keine Relay-Regeln in der Windows-Firewall."),
    (
        "Windows Firewall is off for this network, so the rules do nothing.",
        "Die Windows-Firewall ist für dieses Netzwerk aus, daher wirken die Regeln nicht.",
    ),
    (
        "Only some relays are blocked for {}: Valve changed its list or a rule was removed. Press Apply again or Reset.",
        "Für {} sind nur einige Relays gesperrt: Valve hat seine Liste geändert oder eine Regel wurde entfernt. Drücke erneut Übernehmen oder Zurücksetzen.",
    ),
    (
        "Rules for locations Valve no longer lists: {}. Apply or Reset removes them.",
        "Regeln für Standorte, die Valve nicht mehr führt: {}. Übernehmen oder Zurücksetzen entfernt sie.",
    ),
    (
        "Some Sigil rules are turned off in Windows Firewall: {}. Apply turns them on again.",
        "Einige Sigil-Regeln sind in der Windows-Firewall ausgeschaltet: {}. Übernehmen schaltet sie wieder ein.",
    ),
    (
        "The last firewall change did not report back. What is shown is what Windows holds now.",
        "Die letzte Firewall-Änderung hat sich nicht zurückgemeldet. Angezeigt wird, was Windows jetzt enthält.",
    ),
    ("Measuring {} of {}", "{} von {} gemessen"),
    ("Pings measured {}", "Pings gemessen {}"),
    ("Pings not measured yet", "Pings noch nicht gemessen"),
    ("Fastest open: {} ({} ms)", "Schnellster offener: {} ({} ms)"),
    ("Valve could not be reached: showing the saved list.", "Valve war nicht erreichbar: Die gespeicherte Liste wird gezeigt."),
    ("Valve's list from {}", "Valves Liste von {}"),
    ("Measure ping", "Ping messen"),
    ("Load preset {}", "Vorlage {} laden"),
    ("Delete preset {}", "Vorlage {} löschen"),
    ("Preset name", "Name der Vorlage"),
    ("Save as preset", "Als Vorlage speichern"),
    ("{} blocked", "{} gesperrt"),
    ("Save a selection to switch to it in one press.", "Speichere eine Auswahl, um mit einem Druck zu ihr zu wechseln."),
    ("All {} open", "Alle {} offen"),
    ("All {} blocked", "Alle {} gesperrt"),
    ("{} of {} blocked", "{} von {} gesperrt"),
    ("Block all", "Alle sperren"),
    ("Allow all", "Alle erlauben"),
    ("Partly blocked", "Teilweise gesperrt"),
    ("Blocked", "Gesperrt"),
    ("Will block", "Wird gesperrt"),
    ("Will open", "Wird geöffnet"),
    ("No answer", "Keine Antwort"),
    ("Block {}", "{} sperren"),
    (
        "Leave at least one location open, or Deadlock cannot find a match.",
        "Lass mindestens einen Standort offen, sonst findet Deadlock kein Match.",
    ),
    ("Waiting for Windows' admin prompt...", "Warten auf die Administratorabfrage von Windows ..."),
    (
        "Sigil's rules do not match your ticks. Press Apply to set them right.",
        "Sigils Regeln passen nicht zu deinen Haken. Drücke Übernehmen, um sie richtigzustellen.",
    ),
    ("No changes. Tick a location to block it.", "Keine Änderungen. Hake einen Standort an, um ihn zu sperren."),
    ("{} changes not applied yet", "{} Änderungen noch nicht angewendet"),
    ("Sigil removes its relay rules, so every region is open again.", "Sigil entfernt seine Relay-Regeln, sodass jede Region wieder offen ist."),
    (
        "Sigil adds Windows Firewall rules that stop deadlock.exe from reaching the relays of {} locations. Nothing else changes, and other programs are not affected.",
        "Sigil fügt Windows-Firewall-Regeln hinzu, die deadlock.exe von den Relays von {} Standorten fernhalten. Sonst ändert sich nichts, und andere Programme sind nicht betroffen.",
    ),
    ("Blocks: {}", "Sperrt: {}"),
    ("Opens again: {}", "Öffnet wieder: {}"),
    (
        "Queues can take longer, and the block stays after Sigil closes until you press Reset. Restart Deadlock if it is running.",
        "Die Suche kann länger dauern, und die Sperre bleibt nach dem Schließen von Sigil, bis du Zurücksetzen drückst. Starte Deadlock neu, falls es läuft.",
    ),
    (
        "Windows shows its admin prompt next. Press Yes there to change the rules; No leaves the firewall as it is.",
        "Als Nächstes zeigt Windows seine Administratorabfrage. Drücke dort Ja, um die Regeln zu ändern; Nein lässt die Firewall, wie sie ist.",
    ),
    ("Block relay regions", "Relay-Regionen sperren"),
    (
        "Sigil removes all {} of its relay rules from Windows Firewall. Every region is open again and matchmaking goes back to normal.",
        "Sigil entfernt alle {} seiner Relay-Regeln aus der Windows-Firewall. Jede Region ist wieder offen und das Matchmaking läuft wieder normal.",
    ),
    ("Unblock every region", "Jede Region entsperren"),
];

/// Literals in the Regions files that are not UI text.
#[cfg(test)]
const NOT_TEXT: &[&str] = &[];

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
            include_str!("locales_viewer.rs"),
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
    #[cfg_attr(not(feature = "languages"), ignore = "translations are parked: --features languages")]
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

    /// Every UI literal in the Regions files has German here or in another table, and so does
    /// every region name and preset error the core hands the page.
    #[test]
    #[cfg_attr(not(feature = "languages"), ignore = "translations are parked: --features languages")]
    fn every_string_on_the_regions_tab_is_translated() {
        let sources = [include_str!("../servers_regions/mod.rs"), include_str!("../servers_regions/view.rs")];
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
        for region in deadlock_core::relays::Region::ALL {
            if !table.contains(region.label()) {
                missing.push(region.label().to_string());
            }
        }
        for error in ["A preset name is 1 to 60 characters.", "Sigil keeps at most 50 presets. Delete one first."] {
            if !table.contains(error) {
                missing.push(error.to_string());
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
