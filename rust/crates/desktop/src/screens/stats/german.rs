//! German for the Stats page. `stats` looks a string up here first, then in
//! Sigil's shared tables (`settings/locales*.rs`), so this page's strings stay
//! out of the shared files.

use std::collections::HashMap;
use std::sync::OnceLock;

pub const DE: &[(&str, &str)] = &[
    (", middle lane", ", mittlere Lane"),
    (", side lane", ", Seitenlane"),
    ("Asia", "Asien"),
    ("Best teammates", "Beste Mitspieler"),
    ("Both teams, final builds and the souls curve", "Beide Teams, Endbuilds und der Seelenverlauf"),
    ("Bots", "Bots"),
    ("By sitting. Open a match for its detail", "Nach Sitzung. Öffne ein Match für die Details"),
    ("Check and save", "Prüfen und speichern"),
    ("Close", "Schließen"),
    ("Compared with the matches before", "Im Vergleich zu den Matches davor"),
    ("Damage", "Schaden"),
    ("Deaths", "Tode"),
    ("Deaths a match", "Tode pro Match"),
    ("Demo file path", "Pfad zur Demo-Datei"),
    ("Denies", "Denies"),
    ("Each point is your win rate over the {} matches before it", "Jeder Punkt ist deine Siegquote über die {} Matches davor"),
    ("Europe", "Europa"),
    ("Facts", "Fakten"),
    ("Fewer to more {}. {} map cells from the API.", "Weniger bis mehr {}. {} Kartenzellen von der API."),
    ("Form", "Form"),
    ("Found on this computer", "Auf diesem Computer gefunden"),
    ("Frequent opponents", "Häufige Gegner"),
    ("From a demo file you import", "Aus einer Demo, die du importierst"),
    ("Generate", "Erzeugen"),
    ("Generate a review or paste one. Numbers the match does not back up are refused.", "Erzeuge eine Analyse oder füge eine ein. Zahlen, die das Match nicht belegt, werden abgelehnt."),
    ("Generating", "Wird erzeugt"),
    ("Going well", "Läuft gut"),
    ("Guardian", "Guardian"),
    ("Healing", "Heilung"),
    ("Hero", "Held"),
    ("Hero Labs", "Heldenlabor"),
    ("Hero {}", "Held {}"),
    ("Heroes", "Helden"),
    ("Import", "Importieren"),
    ("K / D / A", "K / T / A"),
    ("Import a demo to go from a match to its moments, with your model's review.", "Importiere eine Demo, um von einem Match zu seinen Momenten zu kommen, mit der Analyse deines Modells."),
    ("Kills", "Kills"),
    ("Kills per death", "Kills pro Tod"),
    ("Last hits", "Last Hits"),
    ("Last signed in", "Zuletzt angemeldet"),
    ("Leaderboard", "Rangliste"),
    ("Length", "Dauer"),
    ("Likely range", "Wahrscheinlicher Bereich"),
    ("Load {}", "{} laden"),
    ("Loading both teams", "Beide Teams werden geladen"),
    ("Loading teammates and opponents", "Mitspieler und Gegner werden geladen"),
    ("Loading the board", "Rangliste wird geladen"),
    ("Looking for \u{201c}{}\u{201d}", "Suche nach \u{201e}{}\u{201c}"),
    ("Loss", "Niederlage"),
    ("Lost", "Verloren"),
    ("MVP", "MVP"),
    ("Make this the primary account", "Als Hauptkonto festlegen"),
    ("Match", "Match"),
    ("Match reviews", "Match-Analysen"),
    ("Match scoreboard", "Match-Übersicht"),
    ("Match {}", "Match {}"),
    ("Matches", "Matches"),
    ("Mode", "Modus"),
    ("Moment by moment review needs this match's demo. Import it under Match reviews below.", "Die Analyse Zug um Zug braucht die Demo dieses Matches. Importiere sie unten unter Match-Analysen."),
    ("NAmerica", "Nordamerika"),
    ("Near the enemy Guardian, middle lane", "Beim gegnerischen Guardian, mittlere Lane"),
    ("Near {} {}{}", "Bei {} {}{}"),
    ("Next", "Als Nächstes"),
    ("No active player by that name. Paste the profile link or the account id instead.", "Kein aktiver Spieler mit diesem Namen. Füge den Profillink oder die Account-ID ein."),
    ("No hero rows for this account yet.", "Noch keine Heldendaten für dieses Konto."),
    ("No items recorded", "Keine Items erfasst"),
    ("No matches for {} yet", "Noch keine Matches für {}"),
    ("No matches yet. New matches show up a little while after they end.", "Noch keine Matches. Neue erscheinen kurz nach dem Ende."),
    ("No model", "Kein Modell"),
    ("No model selected", "Kein Modell ausgewählt"),
    ("No name", "Kein Name"),
    ("No one is on this board yet. It fills as the season is played.", "Noch ist niemand auf dieser Rangliste. Sie füllt sich, wenn die Saison gespielt wird."),
    ("Nobody has three shared matches yet. It fills in as you play.", "Noch niemand mit drei gemeinsamen Matches. Es füllt sich, wenn du spielst."),
    ("None yet", "Noch keine"),
    ("Not in the demo", "Nicht in der Demo"),
    ("Note", "Hinweis"),
    ("Oceania", "Ozeanien"),
    ("Open map", "Freie Karte"),
    ("Open map, enemy half", "Freie Karte, gegnerische Hälfte"),
    ("Open map, your half", "Freie Karte, deine Hälfte"),
    ("Open the stats for {}", "Statistik für {} öffnen"),
    ("Other", "Andere"),
    ("Paste the reply here", "Füge die Antwort hier ein"),
    ("Paste your Steam profile link, SteamID or Deadlock account id.", "Füge deinen Steam-Profillink, deine SteamID oder Deadlock-Account-ID ein."),
    ("Path to a .dem file", "Pfad zu einer .dem-Datei"),
    ("Patron", "Patron"),
    ("Place", "Platz"),
    ("Placement", "Platzierung"),
    ("Player", "Spieler"),
    ("Player stats are turned off in Settings, Privacy.", "Spielerstatistiken sind in den Einstellungen unter Datenschutz aus."),
    ("Players with that name", "Spieler mit diesem Namen"),
    ("Players you meet", "Spieler, die du triffst"),
    ("Primary", "Hauptkonto"),
    ("Private lobby", "Private Lobby"),
    ("Profile links work in both forms: /profiles/ with the number, or /id/ with a name.", "Profillinks funktionieren in beiden Formen: /profiles/ mit der Nummer oder /id/ mit einem Namen."),
    ("Progress", "Verlauf"),
    ("Prompt", "Prompt"),
    ("Queued together", "Zusammen in der Gruppe"),
    ("Rank", "Rang"),
    ("Ranked", "Ranked"),
    ("Recent", "Zuletzt"),
    ("Refresh", "Aktualisieren"),
    ("Refresh teammates and opponents", "Mitspieler und Gegner aktualisieren"),
    ("Refreshing", "Wird aktualisiert"),
    ("Reload demos", "Demos neu laden"),
    ("Result", "Ergebnis"),
    ("Review", "Analyse"),
    ("Review a match", "Match analysieren"),
    ("Review reply", "Antwort der Analyse"),
    ("Review this match", "Dieses Match analysieren"),
    ("Rolling win rate, now {} percent", "Laufende Siegquote, jetzt {} Prozent"),
    ("SAmerica", "Südamerika"),
    ("Scoreboard", "Übersicht"),
    ("See how you are playing", "Sieh, wie du spielst"),
    ("Set a local model endpoint to generate", "Lege einen lokalen Modell-Endpunkt fest, um zu erzeugen"),
    ("Show all", "Alle anzeigen"),
    ("Show all {}", "Alle {} anzeigen"),
    ("Show earlier sittings ({})", "Frühere Sitzungen anzeigen ({})"),
    ("Show fewer", "Weniger anzeigen"),
    ("Show leaderboard", "Rangliste anzeigen"),
    ("Show more", "Mehr anzeigen"),
    ("Show players", "Spieler anzeigen"),
    ("Show stats", "Statistik anzeigen"),
    ("Showing", "Wird angezeigt"),
    ("Showing an older copy", "Ältere Kopie wird angezeigt"),
    ("Signed in before", "Früher angemeldet"),
    ("Soul lead", "Seelenvorsprung"),
    ("Soul lead through the match", "Seelenvorsprung im Match"),
    ("Souls", "Seelen"),
    ("Souls a minute", "Seelen pro Minute"),
    ("Souls ahead of the other team, sampled every three minutes. Ended {}.", "Seelen vor dem anderen Team, alle drei Minuten erfasst. Am Ende {}."),
    ("Stats", "Statistiken"),
    ("Steam profile link, account id or name", "Steam-Profillink, Account-ID oder Name"),
    ("Stop tracking this player", "Diesen Spieler nicht mehr verfolgen"),
    ("Switch player", "Spieler wechseln"),
    ("Team 0", "Team 0"),
    ("Team 0 won", "Team 0 hat gewonnen"),
    ("Team 1", "Team 1"),
    ("Team 1 won", "Team 1 hat gewonnen"),
    ("Teammates and opponents, three shared matches or more", "Mitspieler und Gegner, drei gemeinsame Matches oder mehr"),
    ("The API only knows matches that finished. New ones show up a little while after they end.", "Die API kennt nur beendete Matches. Neue erscheinen kurz nach dem Ende."),
    ("The kill and death map did not load.", "Die Karte der Kills und Tode wurde nicht geladen."),
    ("The other team", "Das andere Team"),
    ("The region's ranked board this season", "Die Rangliste deiner Region in dieser Saison"),
    ("The rolling win rate needs more than 20 decided matches in the window.", "Die laufende Siegquote braucht mehr als 20 gewertete Matches im Fenster."),
    ("This season, {} ranked matches. Last season ended at {}", "Diese Saison {} Ranked-Matches. Die letzte Saison endete bei {}"),
    ("Time", "Zeit"),
    ("Track", "Verfolgen"),
    ("Track and show", "Verfolgen und anzeigen"),
    ("Two ranked matches draw the ladder. Play ranked to see it move.", "Zwei Ranked-Matches zeichnen die Leiter. Spiele Ranked, um sie zu sehen."),
    ("Unknown map", "Unbekannte Karte"),
    ("Unranked", "Unranked"),
    ("Walker", "Walker"),
    ("Watch", "Beobachten"),
    ("Where fights happen", "Wo Kämpfe passieren"),
    ("Where you die most", "Wo du am meisten stirbst"),
    ("Where you get kills", "Wo du Kills holst"),
    ("Where you sit against everyone else in your region.", "Wo du im Vergleich zu deiner Region stehst."),
    ("Who you queue with, who you keep facing, and how those games went.", "Mit wem du spielst, gegen wen du immer wieder antrittst und wie diese Spiele liefen."),
    ("Win", "Sieg"),
    ("Win rate", "Siegquote"),
    ("Win rate, with its likely range", "Siegquote mit wahrscheinlichem Bereich"),
    ("Won", "Gewonnen"),
    ("You", "Du"),
    ("You are not on this board yet.", "Du bist noch nicht auf dieser Rangliste."),
    ("Your last {}", "Deine letzten {}"),
    ("Your overall win rate", "Deine Gesamtsiegquote"),
    ("Your place: {}", "Dein Platz: {}"),
    ("Your team", "Dein Team"),
    ("Your team lost", "Dein Team hat verloren"),
    ("Your team won", "Dein Team hat gewonnen"),
    ("Your win rate", "Deine Siegquote"),
    ("base", "Basis"),
    ("deaths", "Tode"),
    ("kills", "Kills"),
    ("losses", "Niederlagen"),
    ("objective", "Ziel"),
    ("ranked matches", "Ranked-Matches"),
    ("the enemy", "dem gegnerischen"),
    ("wins", "Siege"),
    ("your", "deinem"),
    ("{} W {} L in your last {}", "{} S {} N in deinen letzten {}"),
    ("{} matches in the last 30 days", "{} Matches in den letzten 30 Tagen"),
    ("{} minutes", "{} Minuten"),
    ("{} played, by matches", "{} gespielt, nach Matches"),
    ("{} players on the hero", "{} Spieler auf dem Helden"),
    ("{} souls", "{} Seelen"),
    ("{} together", "{} zusammen"),
    ("{} {}", "{} {}"),
    ("{} {} in a row now", "{} {} in Folge"),
    ("{}, {}", "{}, {}"),
    ("{}% against them", "{} Prozent gegen sie"),
    ("{}% with them", "{} Prozent mit ihnen"),
    ("{}, every match, both sides of the map", "{}, jedes Match, beide Seiten der Karte"),
    // Wave 3: filter, Items, Party
    ("Show", "Zeigen"),
    ("Ranked and unranked", "Ranked und Unranked"),
    ("Any time", "Jederzeit"),
    ("Ranked seasons", "Ranked-Saisons"),
    ("Loading seasons", "Saisons werden geladen"),
    ("The API lists no ranked season yet.", "Die API listet noch keine Ranked-Saison."),
    ("Try again", "Erneut versuchen"),
    ("Last 7 days", "Letzte 7 Tage"),
    ("Last 30 days", "Letzte 30 Tage"),
    ("Last 90 days", "Letzte 90 Tage"),
    ("Filters Matches, Items and Party", "Filtert Matches, Items und Party"),
    ("Items", "Items"),
    ("Your win rate with each item against everyone's, and when you buy it", "Deine Siegquote mit jedem Item gegen die aller anderen, und wann du es kaufst"),
    ("Show items", "Items zeigen"),
    ("Loading items", "Items werden geladen"),
    ("No item has five matches in this filter yet.", "Noch kein Item hat fünf Matches in diesem Filter."),
    ("Item", "Item"),
    ("Against everyone", "Gegen alle"),
    ("Bought at", "Gekauft bei"),
    ("Than everyone", "Als alle"),
    ("Item timeline", "Item-Verlauf"),
    ("Items in the order you bought them", "Items in der Reihenfolge, in der du sie gekauft hast"),
    ("Loading the item timeline", "Item-Verlauf wird geladen"),
    ("No purchases were recorded for you in this match.", "Für dich wurden in diesem Match keine Käufe erfasst."),
    ("sold", "verkauft"),
    ("Load a player first.", "Lade zuerst einen Spieler."),
    ("Party", "Party"),
    ("Every lineup your party can queue, from each member's most played heroes", "Jede Aufstellung, die deine Party spielen kann, aus den meistgespielten Helden jedes Mitglieds"),
    ("Track a friend's account or show the players you meet; premades and tracked players can join the party here.", "Verfolge das Konto eines Freundes oder zeige die Spieler, die du triffst; Premades und verfolgte Spieler können hier der Party beitreten."),
    ("Pick at least one other player for the party.", "Wähle mindestens einen weiteren Spieler für die Party."),
    ("Rate lineups", "Aufstellungen bewerten"),
    ("Rating lineups", "Aufstellungen werden bewertet"),
    ("No lineup to rate: the party has no heroes in this filter.", "Keine Aufstellung zu bewerten: Die Party hat in diesem Filter keine Helden."),
    ("Lineup", "Aufstellung"),
    ("about {}", "etwa {}"),
    ("Across these lineups the party wins about {}% of its games. Grey rows have few matches behind them.", "Über diese Aufstellungen gewinnt die Party etwa {} Prozent ihrer Spiele. Graue Zeilen beruhen auf wenigen Matches."),
    ("Open on Statlocker", "Auf Statlocker öffnen"),
    ("The page could not be opened: {}", "Die Seite ließ sich nicht öffnen: {}"),
    ("\u{00b7} \u{00b7} \u{00b7}", "\u{00b7} \u{00b7} \u{00b7}"),
    ("Add player", "Spieler hinzufügen"),
    ("Each player offers up to 3 heroes, as in queue. Press a player, then heroes below.", "Jeder Spieler bietet bis zu 3 Helden an, wie in der Warteschlange. Drück einen Spieler, dann Helden unten."),
    ("Everyone's games in the filter's window, 30 days when none is set. Items need 500 matches to show.", "Die Spiele aller im Zeitraum des Filters, 30 Tage ohne Zeitraum. Gegenstände brauchen 500 Matches, um zu erscheinen."),
    ("Filters every section below", "Filtert jeden Abschnitt darunter"),
    ("Give at least two players a hero.", "Gib mindestens zwei Spielern einen Helden."),
    ("No heroes yet", "Noch keine Helden"),
    ("No item has enough matches in this window.", "Kein Gegenstand hat in diesem Zeitraum genug Matches."),
    ("Overview and Heroes show this filter.", "Übersicht und Helden zeigen diesen Filter."),
    ("Player {}", "Spieler {}"),
    ("Reading your stats for this filter", "Deine Statistiken für diesen Filter werden gelesen"),
    ("Remove player", "Spieler entfernen"),
    ("Show item meta", "Item-Meta zeigen"),
    ("Take out {}", "{} herausnehmen"),
    ("The filtered stats did not load: {}", "Die gefilterten Statistiken haben nicht geladen: {}"),
    ("Item meta", "Item-Meta"),
    ("Win rate and buy time of every item across all players", "Siegquote und Kaufzeit jedes Gegenstands über alle Spieler"),
    ("Most played heroes", "Meistgespielte Helden"),
    ("Pick heroes by hand", "Helden selbst wählen"),
    ("Every lineup your party can queue, rated from pair and trio records", "Jede Aufstellung, die deine Gruppe spielen kann, bewertet aus Paar- und Trio-Daten"),
    ("This patch", "Dieser Patch"),
    ("The filter's window", "Zeitraum des Filters"),
    ("Everyone's games since the patch on {}. Items need 500 matches to show.", "Die Spiele aller seit dem Patch vom {}. Gegenstände brauchen 500 Matches, um zu erscheinen."),
    ("No patch date was found, so this is the last 30 days. Items need 500 matches to show.", "Kein Patchdatum gefunden, daher die letzten 30 Tage. Gegenstände brauchen 500 Matches, um zu erscheinen."),
];

pub fn get(text: &str) -> Option<&'static str> {
    static MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| DE.iter().chain(super::live_de::DE).chain(super::roll_de::DE).copied().collect())
        .get(text)
        .copied()
}

/// The UI text in the player's language: this page's German table first, then
/// Sigil's shared tables.
pub(super) fn tr(text: &str) -> &str {
    use crate::screens::settings::i18n::{self, Lang};
    if i18n::current() == Lang::German
        && let Some(de) = get(text)
    {
        return de;
    }
    i18n::tr(text)
}

/// Every `tr("..")` literal in `source`, plus both arms of
/// `tr(if .. { ".." } else { ".." })`.
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

    const SOURCES: [&str; 14] = [
        include_str!("roll.rs"),
        include_str!("notes.rs"),
        include_str!("more.rs"),
        include_str!("mod.rs"),
        include_str!("brief.rs"),
        include_str!("heroes.rs"),
        include_str!("matches.rs"),
        include_str!("board.rs"),
        include_str!("social.rs"),
        include_str!("leaderboard.rs"),
        include_str!("map.rs"),
        include_str!("demos.rs"),
        include_str!("w4.rs"),
        include_str!("live.rs"),
    ];

    #[test]
    fn every_routed_string_has_german() {
        let mut missing: Vec<String> = SOURCES
            .iter()
            .flat_map(|source| routed(&source.split("#[cfg(test)]").next().unwrap_or(source)))
            .filter(|text| get(text).is_none())
            .collect();
        missing.sort();
        missing.dedup();
        assert!(missing.is_empty(), "untranslated: {missing:#?}");
    }

    #[test]
    fn table_is_clean() {
        let mut seen = std::collections::HashSet::new();
        for (en, de) in DE.iter().chain(super::super::live_de::DE).chain(super::super::roll_de::DE) {
            assert!(seen.insert(*en), "duplicate {en:?}");
            assert_eq!(en.matches("{}").count(), de.matches("{}").count(), "holes in {en:?}");
            assert!(!en.contains('\u{2014}') && !de.contains('\u{2014}'), "em dash in {en:?}");
        }
    }
}
