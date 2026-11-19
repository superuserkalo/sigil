//! German for the Host tab (Servers > Host, ADR 0015). Kept apart from the other tables so no two
//! agents edit one file; declared in `screens/servers_host/mod.rs` and chained into `i18n`'s
//! maps. The duplicate and routed-file checks for these strings live in this file's tests.

pub const DE: &[(&str, &str)] = &[
    // BEGIN TABLE
    ("Host", "Hosten"),
    ("Check again", "Erneut prüfen"),
    (
        "Run a Deadworks server on this PC for you and your friends. Sigil sets it up, starts and stops it, and shows its console.",
        "Betreibe einen Deadworks-Server auf diesem PC für dich und deine Freunde. Sigil richtet ihn ein, startet und stoppt ihn und zeigt seine Konsole.",
    ),
    (
        "Hosting needs Windows, about 35 GB of free space, the .NET 10 runtime and a Steam account that owns Deadlock. You sign in to Steam in SteamCMD's own window; Sigil never sees your password.",
        "Hosten braucht Windows, etwa 35 GB freien Speicher, die .NET-10-Laufzeit und ein Steam-Konto, das Deadlock besitzt. Du meldest dich in SteamCMDs eigenem Fenster bei Steam an; Sigil sieht dein Passwort nie.",
    ),
    ("Windows 10 or 11, 64-bit", "Windows 10 oder 11, 64 Bit"),
    (
        "About 35 GB of free space for the server files",
        "Etwa 35 GB freier Speicher für die Serverdateien",
    ),
    (
        "The .NET 10 runtime (x64) from Microsoft",
        "Die .NET-10-Laufzeit (x64) von Microsoft",
    ),
    (
        "A Steam account that owns Deadlock, signed in inside SteamCMD's own window",
        "Ein Steam-Konto, das Deadlock besitzt, angemeldet in SteamCMDs eigenem Fenster",
    ),
    ("Hosting needs Windows", "Hosten braucht Windows"),
    (
        "The Deadlock dedicated server and Deadworks run on Windows only, so Sigil can host from a Windows PC. Joining servers works here as usual.",
        "Der dedizierte Deadlock-Server und Deadworks laufen nur unter Windows, daher kann Sigil nur auf einem Windows-PC hosten. Server beitreten funktioniert hier wie gewohnt.",
    ),
    ("What hosting needs", "Was Hosten braucht"),
    (
        "On Linux, the Deadworks guide shows a Docker image that runs the Windows server under Wine.",
        "Für Linux zeigt die Deadworks-Anleitung ein Docker-Image, das den Windows-Server unter Wine ausführt.",
    ),
    ("Open the Deadworks guide", "Deadworks-Anleitung öffnen"),
    (
        "The page could not be opened: {}",
        "Die Seite konnte nicht geöffnet werden: {}",
    ),
    (
        "Server files are older than your game (server build {}, game build {}). Players on the new build cannot join until you update.",
        "Die Serverdateien sind älter als dein Spiel (Server-Build {}, Spiel-Build {}). Spieler mit dem neuen Build können erst nach dem Update beitreten.",
    ),
    (
        "Deadworks {} is in the server folder. Sigil's pinned release is {}.",
        "Im Serverordner liegt Deadworks {}. Sigils festgelegte Version ist {}.",
    ),
    ("Checking this PC", "Dieser PC wird geprüft"),
    ("{} of {} done", "{} von {} erledigt"),
    (
        "{} free here. A new server needs about {}.",
        "{} frei. Ein neuer Server braucht etwa {}.",
    ),
    (
        "Only {} free here. The server files need about {}: pick a folder on a drive with more room.",
        "Nur {} frei. Die Serverdateien brauchen etwa {}: wähle einen Ordner auf einem Laufwerk mit mehr Platz.",
    ),
    (
        "Free space could not be read.",
        "Der freie Speicher konnte nicht gelesen werden.",
    ),
    ("Server folder", "Serverordner"),
    (".NET 10 runtime", ".NET-10-Laufzeit"),
    ("Found {}.", "{} gefunden."),
    (
        "Not found. Install the .NET 10 Runtime (x64) from Microsoft, then press Check again.",
        "Nicht gefunden. Installiere die .NET 10 Runtime (x64) von Microsoft und drücke dann Erneut prüfen.",
    ),
    ("Get .NET 10", ".NET 10 holen"),
    ("SteamCMD", "SteamCMD"),
    (
        "Downloading SteamCMD from Valve...",
        "SteamCMD wird von Valve geladen...",
    ),
    (
        "Valve's command-line Steam client. It downloads the server files. About 4 MB from Valve.",
        "Valves Steam-Client für die Kommandozeile. Er lädt die Serverdateien. Etwa 4 MB von Valve.",
    ),
    ("Install SteamCMD", "SteamCMD installieren"),
    (
        "SteamCMD is open in its own window. Sign in there with your Steam account; Steam Guard asks there too. Close the window when it says the download is done.",
        "SteamCMD ist in einem eigenen Fenster offen. Melde dich dort mit deinem Steam-Konto an; auch Steam Guard fragt dort. Schließe das Fenster, wenn es meldet, dass der Download fertig ist.",
    ),
    ("Server files", "Serverdateien"),
    ("Build {}, app 1422450.", "Build {}, App 1422450."),
    ("Install SteamCMD first.", "Installiere zuerst SteamCMD."),
    (
        "Not downloaded yet. SteamCMD opens in its own window, you sign in there, and it downloads about 35 GB.",
        "Noch nicht geladen. SteamCMD öffnet sich in einem eigenen Fenster, du meldest dich dort an, und es lädt etwa 35 GB.",
    ),
    ("Deadworks", "Deadworks"),
    (
        "Downloading Deadworks {} from GitHub and checking its hash...",
        "Deadworks {} wird von GitHub geladen und sein Hash geprüft...",
    ),
    (
        "Version {}, the plugin framework the server runs.",
        "Version {}, das Plugin-Framework, mit dem der Server läuft.",
    ),
    (
        "Download the server files first. Deadworks goes on top of them.",
        "Lade zuerst die Serverdateien. Deadworks kommt darüber.",
    ),
    (
        "The plugin framework the server runs (MIT). Sigil downloads its pinned release from GitHub and checks the hash before unpacking.",
        "Das Plugin-Framework, mit dem der Server läuft (MIT). Sigil lädt die festgelegte Version von GitHub und prüft den Hash vor dem Entpacken.",
    ),
    ("Stopped", "Gestoppt"),
    ("Starting", "Startet"),
    ("Stopping", "Stoppt"),
    ("Did not start", "Nicht gestartet"),
    ("Running on port {}", "Läuft auf Port {}"),
    ("Stop", "Stoppen"),
    ("Start server", "Server starten"),
    ("Server", "Server"),
    ("Server name", "Servername"),
    ("My Deadlock server", "Mein Deadlock-Server"),
    ("Map", "Karte"),
    ("Port", "Port"),
    ("Max players", "Max. Spieler"),
    ("None: anyone can join", "Keins: jeder kann beitreten"),
    ("Extra launch options", "Zusätzliche Startoptionen"),
    (
        "Finish the setup above to start the server.",
        "Schließe die Einrichtung oben ab, um den Server zu starten.",
    ),
    (
        "The server stopped on its own. Its last lines are in the console below.",
        "Der Server hat sich selbst beendet. Seine letzten Zeilen stehen unten in der Konsole.",
    ),
    (
        "The server stopped on its own: {}",
        "Der Server hat sich selbst beendet: {}",
    ),
    (
        "Stop the server to change its settings.",
        "Stoppe den Server, um seine Einstellungen zu ändern.",
    ),
    ("Console", "Konsole"),
    ("{} lines", "{} Zeilen"),
    ("Copy the console", "Konsole kopieren"),
    ("Console copied.", "Konsole kopiert."),
    (
        "The server's output shows here once it starts.",
        "Die Ausgabe des Servers erscheint hier, sobald er startet.",
    ),
    (
        "Checking Windows Firewall...",
        "Windows-Firewall wird geprüft...",
    ),
    (
        "Windows Firewall lets TCP and UDP port {} in.",
        "Die Windows-Firewall lässt TCP und UDP auf Port {} herein.",
    ),
    (
        "Only part of the rule for port {} is there. Add it again to open both TCP and UDP.",
        "Für Port {} ist die Regel nur zum Teil da. Füge sie erneut hinzu, um TCP und UDP zu öffnen.",
    ),
    (
        "No rule for port {} yet. Other PCs cannot reach the server until there is one.",
        "Noch keine Regel für Port {}. Andere PCs erreichen den Server erst, wenn es eine gibt.",
    ),
    (
        "Adding the rule shows Windows' admin prompt. Sigil changes nothing else.",
        "Beim Hinzufügen der Regel zeigt Windows seine Administrator-Abfrage. Sigil ändert sonst nichts.",
    ),
    ("Firewall", "Firewall"),
    ("Add firewall rule", "Firewall-Regel hinzufügen"),
    ("Remove rule", "Regel entfernen"),
    (
        "No local network address found. Is this PC connected?",
        "Keine Adresse im lokalen Netzwerk gefunden. Ist dieser PC verbunden?",
    ),
    ("Copy the connect line", "Verbindungszeile kopieren"),
    ("Connect line copied.", "Verbindungszeile kopiert."),
    ("Join from this PC", "Von diesem PC beitreten"),
    (
        "Friends on the same network type this in Deadlock's console (F7), or use Connect on the Servers tab.",
        "Freunde im selben Netzwerk geben das in Deadlocks Konsole (F7) ein oder nutzen Verbinden im Tab Server.",
    ),
    ("Same network", "Selbes Netzwerk"),
    ("this PC", "diesen PC"),
    (
        "Open your router's settings page. Its address is often on a sticker on the router, like 192.168.1.1.",
        "Öffne die Einstellungsseite deines Routers. Die Adresse steht oft auf einem Aufkleber am Router, etwa 192.168.1.1.",
    ),
    (
        "Forward TCP and UDP port {} to {}.",
        "Leite TCP- und UDP-Port {} an {} weiter.",
    ),
    (
        "Your public IP is on the router's status page. Friends join with connect <public IP>:{}.",
        "Deine öffentliche IP steht auf der Statusseite des Routers. Freunde treten mit connect <öffentliche IP>:{} bei.",
    ),
    (
        "Friends outside your network",
        "Freunde außerhalb deines Netzwerks",
    ),
    (
        "Your router has to pass the port on to this PC. Sigil does not change router settings or look up your public IP.",
        "Dein Router muss den Port an diesen PC weiterleiten. Sigil ändert keine Router-Einstellungen und fragt deine öffentliche IP nicht ab.",
    ),
    ("Let friends join", "Freunde beitreten lassen"),
    ("Plugins", "Plugins"),
    (
        "Plugins are .NET code that runs on this PC with the server's rights. Turn on only plugins you trust. A change applies on the next start; Deadworks reloads a replaced plugin while it runs.",
        "Plugins sind .NET-Code, der auf diesem PC mit den Rechten des Servers läuft. Schalte nur Plugins ein, denen du vertraust. Eine Änderung gilt ab dem nächsten Start; ein ersetztes Plugin lädt Deadworks im laufenden Betrieb neu.",
    ),
    (
        "Plugins show here once Deadworks is installed.",
        "Plugins erscheinen hier, sobald Deadworks installiert ist.",
    ),
    (
        "No plugins in the server's plugins folder.",
        "Keine Plugins im Plugin-Ordner des Servers.",
    ),
    ("Turn {} on or off", "{} ein- oder ausschalten"),
    ("Update the server files", "Serverdateien aktualisieren"),
    ("Download the server files", "Serverdateien laden"),
    (
        "SteamCMD opens in its own window and brings Deadlock's dedicated server (app 1422450) up to the newest build in:",
        "SteamCMD öffnet sich in einem eigenen Fenster und bringt den dedizierten Deadlock-Server (App 1422450) auf den neuesten Build in:",
    ),
    (
        "SteamCMD opens in its own window and downloads Deadlock's dedicated server (app 1422450, about 35 GB) into:",
        "SteamCMD öffnet sich in einem eigenen Fenster und lädt den dedizierten Deadlock-Server (App 1422450, etwa 35 GB) nach:",
    ),
    (
        "Sign in there with a Steam account that owns Deadlock. SteamCMD asks for the password and any Steam Guard code in its own window. Sigil never sees, asks for or keeps them.",
        "Melde dich dort mit einem Steam-Konto an, das Deadlock besitzt. SteamCMD fragt in seinem eigenen Fenster nach dem Passwort und einem Steam-Guard-Code. Sigil sieht, erfragt und speichert sie nie.",
    ),
    ("Steam account name", "Steam-Kontoname"),
    (
        "The name you sign in with, not your profile name",
        "Der Name zum Anmelden, nicht dein Profilname",
    ),
    (
        "Run it again after every Deadlock patch so friends on the new build can join.",
        "Führe es nach jedem Deadlock-Patch erneut aus, damit Freunde mit dem neuen Build beitreten können.",
    ),
    ("Open SteamCMD", "SteamCMD öffnen"),
    (
        "Sigil asks Windows to let incoming TCP and UDP on port {} reach the server. Nothing else changes.",
        "Sigil bittet Windows, eingehendes TCP und UDP auf Port {} zum Server durchzulassen. Sonst ändert sich nichts.",
    ),
    (
        "Windows shows its admin prompt next. Press Yes there to add the rule; No leaves the firewall as it is.",
        "Als Nächstes zeigt Windows seine Administrator-Abfrage. Drücke dort Ja, um die Regel hinzuzufügen; Nein lässt die Firewall, wie sie ist.",
    ),
    ("Add rule", "Regel hinzufügen"),
    ("Add a firewall rule", "Eine Firewall-Regel hinzufügen"),
    (
        "The port must be a number from 1024 to 65535.",
        "Der Port muss eine Zahl von 1024 bis 65535 sein.",
    ),
    (
        "The map is a console name like dl_midtown: letters, digits and _ only.",
        "Die Karte ist ein Konsolenname wie dl_midtown: nur Buchstaben, Ziffern und _.",
    ),
    (
        "Choose a folder for the server files",
        "Ordner für die Serverdateien wählen",
    ),
    (
        "Type the Steam account name you sign in with.",
        "Gib den Steam-Kontonamen ein, mit dem du dich anmeldest.",
    ),
    (
        "Deadworks {} is installed.",
        "Deadworks {} ist installiert.",
    ),
    ("SteamCMD is installed.", "SteamCMD ist installiert."),
    ("Console command", "Konsolenbefehl"),
    ("Copied in at Start", "Wird beim Start kopiert"),
    (
        "Custom map {}: Start copies {} into the server folder first, then loads it.",
        "Eigene Karte {}: Starten kopiert zuerst {} in den Serverordner und lädt sie dann.",
    ),
    ("Deadlock's maps", "Deadlocks Karten"),
    (
        "Installed outside Sigil. The plugin framework the server runs.",
        "Außerhalb von Sigil installiert. Das Plugin-Framework, mit dem der Server läuft.",
    ),
    (
        "Max players must be a number from 1 to 64, or empty for the server's default.",
        "Max. Spieler muss eine Zahl von 1 bis 64 sein oder leer für den Standard des Servers.",
    ),
    (
        "None installed. Get maps on the Maps tab.",
        "Keine installiert. Karten gibt es im Tab Karten.",
    ),
    ("Pick a map", "Karte wählen"),
    ("Server default", "Standard des Servers"),
    (
        "SteamCMD closed with an error ({}). Its window showed what went wrong; try again.",
        "SteamCMD wurde mit einem Fehler beendet ({}). Sein Fenster hat gezeigt, was schiefging; versuche es erneut.",
    ),
    (
        "The Deadworks in the server folder was not installed by Sigil. Sigil's pinned release is {}.",
        "Das Deadworks im Serverordner wurde nicht von Sigil installiert. Sigils festgelegte Version ist {}.",
    ),
    (
        "The server runs your custom map {}. Friends who use Sigil get the map offered when they join. Everyone else installs the map first, then types the line below in Deadlock's console (F7).",
        "Der Server läuft mit deiner eigenen Karte {}. Freunden mit Sigil wird die Karte beim Beitreten angeboten. Alle anderen installieren zuerst die Karte und geben dann die Zeile unten in Deadlocks Konsole (F7) ein.",
    ),
    (
        "Type a console command, like status",
        "Gib einen Konsolenbefehl ein, etwa status",
    ),
    (
        "Type status to list the players on the server.",
        "Gib status ein, um die Spieler auf dem Server aufzulisten.",
    ),
    (
        "Valve's command-line Steam client, kept in Sigil's app data.",
        "Valves Steam-Client für die Kommandozeile, in Sigils App-Daten.",
    ),
    ("Your custom maps", "Deine eigenen Karten"),
    (
        "Deadworks does not support this Deadlock build yet. Wait for a Deadworks update, then install it here.",
        "Deadworks unterstützt diesen Deadlock-Build noch nicht. Warte auf ein Deadworks-Update und installiere es dann hier.",
    ),
    ("On the server", "Auf dem Server"),
    // END TABLE
];

/// Literals in the routed files that read as words but are not UI text.
#[cfg(test)]
const NOT_TEXT: &[&str] = &[];

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    /// Every English key in Sigil's other German tables, read from their source so private
    /// tables count too.
    fn others() -> HashSet<String> {
        let sources = [
            include_str!("locales.rs"),
            include_str!("locales_parity.rs"),
            include_str!("locales_wave.rs"),
            include_str!("locales_w4a.rs"),
            include_str!("locales_w4b.rs"),
            include_str!("locales_w4c.rs"),
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
            assert_eq!(
                en.matches("{}").count(),
                de.matches("{}").count(),
                "holes in {en:?}"
            );
            assert!(
                !de.contains('\u{2014}') && !en.contains('\u{2014}'),
                "em dash in {en:?}"
            );
        }
    }

    /// Every UI literal in the Host files has German here or in another table. Same rule as
    /// `i18n::every_routed_string_is_translated`: a literal that starts with a capital and has a
    /// lowercase letter.
    #[test]
    fn every_string_on_the_host_tab_is_translated() {
        let sources = [
            include_str!("../servers_host/mod.rs"),
            include_str!("../servers_host/view.rs"),
            include_str!("../servers_host/bridge.rs"),
        ];
        let mut table = others();
        table.extend(super::DE.iter().map(|(en, _)| en.to_string()));
        let mut missing = Vec::new();
        for source in sources {
            let code = source.split("#[cfg(test)]").next().unwrap_or(source);
            for literal in literals(code) {
                let ui = literal
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_uppercase())
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
