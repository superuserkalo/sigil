//! Search across every category. Each entry names a row by the title it shows, the category
//! that holds it, its one-line description and a few extra words people type for it. A match
//! lists the row; pressing it opens the category. The test below checks that every title here
//! is a title some category really shows, so the index cannot drift from the pages.

use rune::{El, col, row, text};

use super::Tab;
use super::i18n::tr;
use crate::app::State;
use crate::ui::{BODY_MED, SMALL, Ui};

pub struct Entry {
    pub tab: Tab,
    pub title: &'static str,
    pub line: &'static str,
    pub words: &'static str,
}

const fn e(tab: Tab, title: &'static str, line: &'static str, words: &'static str) -> Entry {
    Entry { tab, title, line, words }
}

pub const INDEX: &[Entry] = &[
    // Game
    e(Tab::Game, "Deadlock folder", "Where Deadlock is installed.", "path install steam library detect browse"),
    e(Tab::Game, "gameinfo.gi", "Lists citadel/addons so the game loads mods.", "repair fix search paths"),
    e(Tab::Game, "Addon folder", "Where installed mods live.", "addons vpk folder open"),
    e(Tab::Game, "autoexec.cfg", "Crosshairs and commands load from it.", "config cfg console"),
    e(Tab::Game, "Steam launch options", "Arguments Steam passes to Deadlock.", "launch args novid condebug"),
    e(Tab::Backups, "Backups", "Copies Sigil keeps before it edits a game or Steam file.", "restore undo copy prune"),
    // Mods
    e(Tab::Mods, "New mods start turned off", "Installs land off until you switch them on.", "enable after download install"),
    e(Tab::Mods, "Order", "How the GameBanana catalog is sorted.", "sort browse popular newest"),
    e(Tab::Mods, "Mature content", "Mods GameBanana marks as mature.", "nsfw blur hide 18+"),
    e(Tab::Mods, "Hidden mods and creators", "Mods and creators you hid in Browse.", "hidden blocked authors show again"),
    // General
    e(Tab::General, "Language", "The language Sigil's pages use.", "language deutsch english locale translation"),
    e(Tab::General, "Dates", "How Sigil writes a date.", "date format day month"),
    e(Tab::General, "Open on", "The page Sigil shows when it starts.", "startup home first page"),
    e(Tab::General, "Start with your computer", "Opens Sigil when you sign in.", "login autostart boot startup"),
    e(Tab::General, "Start minimized", "Waits in the Dock or taskbar at sign-in.", "tray hidden background"),
    e(Tab::General, "When the game starts", "Stay open, minimize or quit.", "launch close minimize"),
    e(Tab::General, "Ask before launching", "A confirm before Steam starts the game.", "confirm launch dialog"),
    e(Tab::General, "One launch button", "A single launch button with a modded or vanilla switch.", "combine launch vanilla modded"),
    // Appearance
    e(Tab::Appearance, "Theme", "Dark, light and the rest.", "dark light oled colors"),
    e(Tab::Appearance, "Accent colour", "The colour of the main action and switches.", "accent color colour custom hex"),
    e(Tab::Appearance, "Interface size", "Scales text, icons and spacing.", "zoom scale font size"),
    e(Tab::Appearance, "Transparent sidebar", "The sidebar sits on the page background.", "sidebar background"),
    // Integrations
    e(Tab::Integrations, "Discord Rich Presence", "Shows what you do in Sigil on Discord.", "discord rpc status activity"),
    e(Tab::Integrations, "Show what you are doing", "Off shows only that you use Sigil.", "discord details privacy"),
    e(Tab::Integrations, "Show your mod count", "How many mods are on, modded or vanilla.", "discord mods"),
    e(Tab::Integrations, "Steam account", "Link the Steam account Stats opens on.", "steam account profile link stats statlocker"),
    e(Tab::Integrations, "GameBanana account", "Open your GameBanana page from Sigil.", "gamebanana profile member link"),
    e(Tab::Integrations, "Open Install with Sigil links", "GameBanana's Sigil button opens the mod here.", "1-click one click deep link gamebanana install protocol"),
    e(Tab::Integrations, "Profile share codes", "Send a mod setup as a code.", "share profile code friend"),
    e(Tab::Integrations, "Community gallery", "Publishing profiles needs a service.", "social discover publish like"),
    // Privacy
    e(Tab::Privacy, "Player stats", "Public match data for the account you enter.", "deadlock-api stats"),
    e(Tab::Privacy, "Crosshair share codes", "Sends crosshair values to make a code.", "crosshair code"),
    e(Tab::Privacy, "Share match data with deadlock-api", "Match ids and keys from Steam's cache.", "salts ingest match data contribute community"),
    e(Tab::Privacy, "Where Sigil connects", "Every host Sigil talks to.", "network hosts allowlist telemetry"),
    // Models
    e(Tab::Models, "Accounts", "Sign in to ChatGPT or Grok for the agent.", "chatgpt grok codex sign in api key"),
    e(Tab::Models, "Model id", "The exact id your account offers.", "model agent llm"),
    e(Tab::Models, "Local endpoint", "Any server that speaks the OpenAI API.", "local llama ollama endpoint"),
    // Network
    e(Tab::Network, "Offline mode", "Stops every connection Sigil makes.", "offline airplane block network internet"),
    e(Tab::Network, "Check for Sigil updates", "Asks GitHub for the newest release.", "update check github"),
    e(Tab::Network, "Update check", "How often, while Sigil is open.", "update interval background"),
    e(Tab::Network, "Refresh Browse", "Reloads the GameBanana page you are on.", "catalog refresh background interval"),
    e(Tab::Network, "HTTP proxy", "Routes Browse and downloads through a proxy.", "proxy http download"),
    // Logs
    e(Tab::Logs, "Log level", "How much Sigil writes to its log.", "logging debug verbose error"),
    e(Tab::Logs, "Log file", "Open the log or its folder.", "logs folder open"),
    e(Tab::Logs, "Copy the last 200 lines", "For a bug report, keys masked.", "logs copy clipboard"),
    e(Tab::Logs, "Crash dumps", "Deadlock's crash files.", "crash dump mdmp"),
    e(Tab::Logs, "Diagnostic report", "Version, system and recent log lines.", "bug report support"),
    e(Tab::Logs, "Open an issue", "Sigil's issue tracker on GitHub.", "bug github issue support"),
    // Storage
    e(Tab::Storage, "Data folder", "Where Sigil keeps its files.", "app data folder"),
    e(Tab::Storage, "Mod workspace", "Downloads, plans and builds.", "workspace folder"),
    e(Tab::Storage, "Preview images", "Cached Browse images.", "cache images clear"),
    e(Tab::Storage, "Downloads", "Archives kept after an install.", "download cache clear"),
    e(Tab::Storage, "Leftover archives", "Zip files left in the addons folder.", "cleanup addons zip rar 7z"),
    e(Tab::Storage, "Rescan", "Find the game again.", "rescan refresh detect"),
    e(Tab::Storage, "Reset settings", "Every setting back to its default.", "reset defaults"),
    // Experimental
    e(Tab::Experimental, "Servers", "The Deadworks server browser.", "experimental deadworks servers"),
    e(Tab::Experimental, "Agent", "The agent and its mod-building tools.", "experimental agent ai"),
    e(Tab::Experimental, "Docked mod details", "Mod details beside the grid.", "experimental browse dock panel"),
    // About
    e(Tab::About, "Release channel", "Stable or preview builds.", "update channel nightly beta"),
    e(Tab::About, "Open source", "The projects Sigil is built on.", "licenses credits notices"),
    // Parity round
    e(Tab::Game, "Steam folder", "A Steam folder Sigil reads first.", "custom steam path library"),
    e(Tab::Game, "Block changes while the game runs", "Installs and toggles wait until Deadlock closes.", "game running protect lock"),
    e(Tab::Game, "Remove the mod path when the game closes", "Takes citadel/addons out of gameinfo.gi.", "cleanup exit vanilla gameinfo"),
    e(Tab::Game, "Check for crashes after a launch", "Tells you about a new crash dump.", "crash dump detect"),
    e(Tab::Game, "Fix mods before launch", "Repairs gameinfo.gi before Steam starts the game.", "auto repair launch"),
    e(Tab::Game, "Reapply mods when Sigil starts", "Puts mods back at startup.", "auto reapply startup"),
    e(Tab::Game, "Console log (-condebug)", "The game writes console.log.", "condebug console log presence"),
    e(Tab::Backups, "Back up every mod", "A full copy of the addons folder.", "create backup snapshot restore merge replace"),
    e(Tab::Mods, "Blur strength", "How much a mature preview is blurred.", "nsfw blur"),
    e(Tab::Mods, "Reveal on hover", "Pointing at a blurred card shows it.", "nsfw hover unblur"),
    e(Tab::Mods, "Always show the 18+ tag", "Mature cards keep their tag.", "nsfw tag 18+"),
    e(Tab::Mods, "Remember what you reveal", "Revealed mods stay revealed.", "nsfw per item choices clear"),
    e(Tab::Mods, "Show unclassified items", "Source mods without a mature flag.", "likely nsfw unknown"),
    e(Tab::Mods, "Download server", "Which GameBanana file server downloads use.", "mirror server speed test filecache"),
    e(Tab::Mods, "Speed test", "Measures three GameBanana servers.", "speed test mirror download"),
    e(Tab::Mods, "Faster downloads", "Large files come in parts.", "parallel segments parts download"),
    e(Tab::Mods, "Download cache", "Where downloaded archives are kept.", "move cache folder location"),
    e(Tab::Mods, "Add a source", "Community mod catalogs beside GameBanana.", "sources third party catalog registry"),
    e(Tab::Mods, "Expand Locker cards", "Locker heroes open expanded.", "locker preferences"),
    e(Tab::Mods, "Confirm before updating a profile", "Asks before a profile is overwritten.", "profile preferences"),
    e(Tab::Mods, "Show what a mod needs", "GameBanana requirements before installing.", "requirements dependencies optional"),
    e(Tab::Mods, "Warn about a second mod for one hero", "Asks which skin to keep for a hero.", "hero conflict skin swap"),
    e(Tab::Mods, "Ignore overlap by default", "The Overlap page stays empty.", "overlap conflicts"),
    e(Tab::Mods, "Switch variants", "A new file of a mod turns the old one off.", "variants files"),
    e(Tab::Mods, "Allow script mods", "Mods that change gameplay data or skeletons.", "scripts vdata unsafe review"),
    e(Tab::Mods, "Replace without asking", "Skip the replace confirm.", "skip replacement confirmation"),
    e(Tab::General, "Minimize to tray", "Minimizing hides Sigil in the tray.", "tray system background"),
    e(Tab::General, "Close to tray", "Closing hides Sigil in the tray.", "tray system background"),
    e(Tab::General, "Text editor", "The editor Sigil opens files with.", "external editor notepad code"),
    e(Tab::General, "Setup", "Run the first-run setup again.", "wizard onboarding reset welcome"),
    e(Tab::General, "Reset tips", "Shows every page tip again.", "tips hints"),
    e(Tab::Integrations, "Game presence", "Discord shows your match phase and hero.", "discord hero phase console"),
    e(Tab::Integrations, "Discord lines", "Your own Discord text per phase and hero.", "discord template custom"),
    e(Tab::Integrations, "Stat sites", "Community sites with match stats.", "tracker statlocker"),
    e(Tab::Network, "Proxy sign-in", "A user name and password for the proxy.", "proxy password authentication keychain credentials"),
    e(Tab::Network, "Skip the proxy for", "Hosts that connect directly.", "proxy bypass no_proxy"),
    e(Tab::Network, "Test connection", "Connects to GameBanana.", "proxy test"),
    e(Tab::Network, "Connection diagnostics", "Checks every host Sigil uses.", "network diagnostics probe dns"),
    e(Tab::Logs, "What went wrong", "A description for your report.", "bug report description"),
    e(Tab::Storage, "Clear all mods", "Moves every mod to Recently deleted.", "remove all mods recovery"),
    e(Tab::Storage, "Reset mod records", "Forgets names and install history.", "clear mods state library"),
    e(Tab::Storage, "Clear all mod data", "Records, cache, backups and trash.", "wipe delete data"),
    e(Tab::Storage, "Warn about unknown files", "Files in addons the game ignores.", "unknown files alert"),
    e(Tab::Experimental, "Imprint installed mods", "Mod identity inside its VPK.", "imprint metadata modinfo"),
    e(Tab::Experimental, "Copy mods to a developer folder", "Make another game folder match.", "dev developer copy"),
    e(Tab::About, "Contributors", "People who built Sigil.", "credits people github"),
];

/// Entries whose title, line or words contain every typed word, in either language.
pub fn matches(query: &str) -> Vec<&'static Entry> {
    let words: Vec<String> = query
        .split_whitespace()
        .map(|w| w.to_lowercase())
        .collect();
    if words.is_empty() {
        return Vec::new();
    }
    INDEX
        .iter()
        .filter(|entry| {
            let hay = format!(
                "{} {} {} {} {} {} {}",
                entry.title,
                tr(entry.title),
                entry.line,
                tr(entry.line),
                entry.words,
                entry.tab.label(),
                tr(entry.tab.label())
            )
            .to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .collect()
}

/// The results pane: one row per match, pressing it opens the category.
pub fn results(s: &State, u: &Ui) -> El {
    let found = matches(&s.settings.search.text);
    if found.is_empty() {
        return u
            .empty_state("", tr("No settings match"), tr("Try another word, like proxy, Discord or language."), None)
            .py(48.0);
    }
    let mut list = col()
        .key("settings-search-results")
        .w_full()
        .border_sides([1.0, 0.0, 0.0, 0.0], u.hair);
    for (index, entry) in found.into_iter().enumerate() {
        let tab = entry.tab;
        list = list.child(
            row()
                .key(format!("settings-search-hit-{index}"))
                .w_full()
                .min_h(56.0)
                .py(10.0)
                .gap(16.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .hover(u.hover)
                .focusable(true)
                .role("button")
                .label(tr(entry.title))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    st.settings.search = rune::TextEdit::new("");
                    super::select(st, tab);
                })
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(text(tr(entry.title)).font(BODY_MED).color(u.ink))
                        .child(text(tr(entry.line)).font(SMALL).color(u.muted).truncate()),
                )
                .child(text(tr(tab.label())).font(SMALL).color(u.faint)),
        );
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_index_title_is_on_a_page() {
        let pages = [
            include_str!("../settings.rs"),
            include_str!("tabs.rs"),
            include_str!("../../prefs.rs"),
            include_str!("support.rs"),
            include_str!("watch.rs"),
            include_str!("extra.rs"),
            include_str!("tools.rs"),
            include_str!("nsfw.rs"),
            include_str!("downloads.rs"),
            include_str!("sources.rs"),
            include_str!("pages.rs"),
            include_str!("tray.rs"),
            include_str!("onboarding.rs"),
            include_str!("netx.rs"),
            include_str!("forge.rs"),
            include_str!("templates.rs"),
        ]
        .concat();
        for entry in INDEX {
            assert!(
                pages.contains(&format!("\"{}\"", entry.title)),
                "{:?} is in the search index but no page shows it",
                entry.title
            );
        }
    }

    #[test]
    fn search_needs_every_word() {
        assert!(matches("").is_empty());
        let hits = matches("discord");
        assert!(hits.iter().any(|e| e.title == "Discord Rich Presence"));
        assert!(matches("proxy").iter().all(|e| e.tab == Tab::Network));
        assert!(matches("offline zzz").is_empty());
    }
}
