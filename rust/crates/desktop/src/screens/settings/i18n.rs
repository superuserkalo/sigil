//! Sigil's UI language. English is the source: a string is shown as `tr("Deadlock folder")` and
//! that English text is the lookup key. Each other locale is one static table of
//! `(english, translation)` pairs compiled into the binary; a string a table lacks falls back
//! to the English, so a missing translation shows English and never a key. Settings' row
//! helpers (`section`, `setting_row`, `toggle_row`, `detail`, `button`, `segmented`) call `tr`
//! themselves, so a literal handed to them is routed already.
//!
//! Adding a string: pass the literal to a helper or wrap it in `tr(..)` (use
//! `fill(tr(..), &[..])` for `{}` holes) and add the pair to every table in `locales.rs`. The
//! `every_routed_string_is_translated` test reads the routed source files and fails on a UI
//! literal a table is missing.
//!
//! Adding a locale: add a `Lang` variant (id, native name, table), a table in `locales.rs`,
//! and the variant to `Lang::ALL`. Nothing else changes.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

use super::locales;
#[path = "locales_w4a.rs"]
mod locales_w4a;
#[path = "locales_w4c.rs"]
mod locales_w4c;

/// The choice in Settings. `System` follows the OS language when Sigil has it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Lang {
    #[default]
    System,
    English,
    German,
}

impl Lang {
    pub const ALL: [Lang; 3] = [Lang::System, Lang::English, Lang::German];

    pub fn id(self) -> &'static str {
        match self {
            Lang::System => "system",
            Lang::English => "en",
            Lang::German => "de",
        }
    }

    pub fn from_id(id: &str) -> Option<Lang> {
        Lang::ALL.into_iter().find(|l| l.id() == id)
    }

    /// The language's own name, which is never translated.
    pub fn native(self) -> &'static str {
        match self {
            Lang::System => "",
            Lang::English => "English",
            Lang::German => "Deutsch",
        }
    }

    fn table(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Lang::German => locales::DE,
            _ => &[],
        }
    }

    /// The second table: strings added in the parity round (`locales_parity.rs`).
    fn more(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Lang::German => super::locales_parity::DE,
            _ => &[],
        }
    }

    /// The third table: strings added in the parity wave (`locales_wave.rs`).
    fn wave(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Lang::German => super::locales_wave::DE,
            _ => &[],
        }
    }

    /// The concrete language `System` stands for, from `LC_ALL`, `LC_MESSAGES` or `LANG`.
    pub fn resolve(self) -> Lang {
        match self {
            Lang::System => from_env(|name| std::env::var(name).ok()),
            other => other,
        }
    }
}

fn from_env(var: impl Fn(&str) -> Option<String>) -> Lang {
    for name in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Some(value) = var(name).filter(|v| !v.is_empty() && v != "C" && v != "POSIX") {
            let lower = value.to_ascii_lowercase();
            return if lower.starts_with("de") {
                Lang::German
            } else {
                Lang::English
            };
        }
    }
    Lang::English
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

fn index(lang: Lang) -> u8 {
    Lang::ALL.iter().position(|l| *l == lang).unwrap_or(0) as u8
}

/// Switch the language every `t` call reads. Called at startup and from Settings.
pub fn set(lang: Lang) {
    CURRENT.store(index(lang.resolve()), Ordering::Relaxed);
}

pub fn current() -> Lang {
    Lang::ALL[CURRENT.load(Ordering::Relaxed) as usize % Lang::ALL.len()]
}

fn maps() -> &'static [HashMap<&'static str, &'static str>; 3] {
    static MAPS: OnceLock<[HashMap<&'static str, &'static str>; 3]> = OnceLock::new();
    MAPS.get_or_init(|| Lang::ALL.map(|lang| lang.table().iter().chain(lang.more()).chain(lang.wave()).chain(if lang == Lang::German { super::locales_w4b::DE } else { &[] }).chain(if lang == Lang::German { locales_w4a::DE } else { &[] }).chain(if lang == Lang::German { locales_w4c::DE } else { &[] }).chain(if lang == Lang::German { crate::screens::servers::host::locales::DE } else { &[] }).chain(if lang == Lang::German { super::locales_smalls::DE } else { &[] }).chain(if lang == Lang::German { crate::viewer::locales::DE } else { &[] }).copied().collect()))
}

#[cfg(test)]
fn lookup(lang: Lang, english: &'static str) -> &'static str {
    maps()[index(lang) as usize]
        .get(english)
        .copied()
        .unwrap_or(english)
}

/// `t` for any string: the translation, or the text itself. Settings' row helpers pass every
/// title and line through this, so a literal handed to a helper is routed without a wrapper.
pub fn tr(text: &str) -> &str {
    maps()[CURRENT.load(Ordering::Relaxed) as usize % 3]
        .get(text)
        .copied()
        .unwrap_or(text)
}

/// Fill `{}` holes in order: `fill(t("{} mods"), &[&n.to_string()])`.
pub fn fill(template: &str, values: &[&str]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut values = values.iter();
    let mut rest = template;
    while let Some(at) = rest.find("{}") {
        out.push_str(&rest[..at]);
        out.push_str(values.next().copied().unwrap_or(""));
        rest = &rest[at + 2..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_falls_back_to_the_english() {
        assert_eq!(lookup(Lang::German, "Settings"), "Einstellungen");
        assert_eq!(lookup(Lang::German, "a string no table has"), "a string no table has");
        assert_eq!(lookup(Lang::English, "Settings"), "Settings");
    }

    #[test]
    fn system_follows_the_environment() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| v.to_string())
            }
        };
        assert_eq!(from_env(env(&[("LANG", "de_DE.UTF-8")])), Lang::German);
        assert_eq!(from_env(env(&[("LC_ALL", "en_US"), ("LANG", "de_DE")])), Lang::English);
        assert_eq!(from_env(env(&[("LANG", "C")])), Lang::English);
        assert_eq!(from_env(env(&[])), Lang::English);
    }

    #[test]
    fn fill_puts_values_in_order() {
        assert_eq!(fill("{} of {} mods", &["2", "5"]), "2 of 5 mods");
        assert_eq!(fill("no holes", &["x"]), "no holes");
        assert_eq!(fill("{} and {}", &["one"]), "one and ");
    }

    #[test]
    fn tables_have_no_duplicates_and_keep_holes() {
        let mut seen = std::collections::HashSet::new();
        for (en, de) in locales::DE
            .iter()
            .chain(super::super::locales_parity::DE)
            .chain(super::super::locales_wave::DE)
            .chain(super::super::locales_w4b::DE)
            .chain(super::locales_w4a::DE)
        {
            assert!(seen.insert(*en), "duplicate key {en:?}");
            assert_eq!(en.matches("{}").count(), de.matches("{}").count(), "holes in {en:?}");
            assert!(!de.contains('\u{2014}'), "em dash in {de:?}");
        }
    }

    /// Every UI literal in the routed files has a German entry. A UI literal is a string
    /// literal that starts with a capital letter and reads as words; ids, keys, paths, hosts
    /// and format strings with named holes are not UI text.
    #[test]
    fn every_routed_string_is_translated() {
        let sources = [
            include_str!("../settings.rs"),
            include_str!("tabs.rs"),
            include_str!("search.rs"),
            include_str!("personalize.rs"),
            include_str!("../../updater.rs"),
            include_str!("../../shell.rs"),
            include_str!("downloads.rs"),
            include_str!("editor.rs"),
            include_str!("extra.rs"),
            include_str!("forge.rs"),
            include_str!("netx.rs"),
            include_str!("nsfw.rs"),
            include_str!("onboarding.rs"),
            include_str!("pages.rs"),
            include_str!("sources.rs"),
            include_str!("support.rs"),
            include_str!("templates.rs"),
            include_str!("tools.rs"),
            include_str!("tray.rs"),
            include_str!("watch.rs"),
            include_str!("../browse/quick.rs"),
            include_str!("../browse/replace.rs"),
            include_str!("../browse/multi.rs"),
            include_str!("../installed/enable.rs"),
            include_str!("../installed/drag.rs"),
            include_str!("../installed/order.rs"),
            include_str!("../installed/overlap.rs"),
            include_str!("../downloads.rs"),
            include_str!("../launch_progress.rs"),
            include_str!("../launch_gate.rs"),
            include_str!("../autoexec_flags.rs"),
            include_str!("../../statusbar.rs"),
            include_str!("../browse/strip.rs"),
            include_str!("../launch_extras.rs"),
            include_str!("../installed/req_order.rs"),
            include_str!("../locker/cursors.rs"),
            include_str!("../installed/repair.rs"),
            include_str!("telemetry.rs"),
            // Parity wave 4, UI B.
            include_str!("../w4b_scenes.rs"),
            include_str!("../../shell_w4.rs"),
            include_str!("../../shell_tips.rs"),
            include_str!("logs_w4.rs"),
            include_str!("misc_w4.rs"),
            include_str!("diag_w4.rs"),
            include_str!("../browse/w4.rs"),
            include_str!("../browse/paste.rs"),
            // Parity wave 4, UI A.
            include_str!("../../notify.rs"),
            include_str!("../../notify/feedback.rs"),
            include_str!("../w4a_scenes.rs"),
            include_str!("../installed/backups.rs"),
            include_str!("../installed/checks.rs"),
            include_str!("../locker/sync.rs"),
            include_str!("../locker/parts.rs"),
            include_str!("../locker/art.rs"),
            include_str!("../installed/safety_gate.rs"),
            include_str!("../installed/imprint.rs"),
            include_str!("../installed/selection.rs"),
            include_str!("../installed/list_sections.rs"),
            include_str!("../installed/sound.rs"),
        ];
        let table: std::collections::HashSet<&str> = locales::DE
            .iter()
            .chain(super::super::locales_parity::DE)
            .chain(super::super::locales_wave::DE)
            .chain(super::super::locales_w4b::DE)
            .chain(super::locales_w4a::DE)
            .map(|(en, _)| *en)
            .collect();
        let mut missing = Vec::new();
        for source in sources {
            let code = source.split("#[cfg(test)]").next().unwrap_or(source);
            for literal in literals(code) {
                if is_ui_text(&literal)
                    && !table.contains(literal.as_str())
                    && !NOT_TEXT.contains(&literal.as_str())
                    && !super::super::locales_parity::NOT_TEXT.contains(&literal.as_str())
                    && !super::super::locales_wave::NOT_TEXT.contains(&literal.as_str())
                    && !super::super::locales_w4b::NOT_TEXT.contains(&literal.as_str())
                    && !super::locales_w4a::NOT_TEXT.contains(&literal.as_str())
                    && !is_path_data(&literal)
                {
                    missing.push(literal);
                }
            }
        }
        missing.sort();
        missing.dedup();
        assert!(missing.is_empty(), "untranslated: {missing:#?}");
        // The nav titles are routed through `t(screen.title())` in app.rs.
        use crate::model::Screen;
        for screen in [
            Screen::Browse,
            Screen::Installed,
            Screen::Locker,
            Screen::Crosshair,
            Screen::Autoexec,
            Screen::Performance,
            Screen::Stats,
            Screen::Conflicts,
            Screen::Profiles,
            Screen::Servers,
            Screen::Agent,
            Screen::Settings,
        ] {
            assert!(table.contains(screen.title()), "nav title {:?}", screen.title());
        }
    }

    /// Literals that look like words but are not UI text: names, Discord presence lines (kept
    /// in English on purpose, friends read them), log and diagnostic report lines (read by
    /// whoever fixes the bug), demo fixtures and process arguments.
    const NOT_TEXT: &[&str] = &[
        "Sigil", "English", "Deutsch", "GitHub", "Discord", "Steam", "GameBanana", "Statlocker",
        "ChatGPT", "Grok", "DAH_GAME_DIR", "Ember", "Kalo", "Smurf", "User-Agent", "Not",
        "IMAGENAME eq deadlock.exe", "Sigil {}",
        // Discord presence
        "Using Sigil", "Playing Deadlock", "Vanilla", "Modded", "Modded, {}", "Changing settings",
        "Browsing mods", "Managing mods", "In the Locker", "Designing a crosshair",
        "Editing the autoexec", "Tuning performance", "Checking stats", "Sorting out mod overlap",
        "Managing profiles", "Browsing servers", "Getting ready to play", "Building a mod",
        // Log and report lines
        "Game: {}", "Language: {}", "Log level: {}", "Mods: {} installed, {} on",
        "Offline mode: {}", "Recent log:", "System: {} {}", "Sigil {} started on {} {}",
        "Match sharing log forgotten", "Removed {} leftover archives",
        // Key names in the shortcuts sheet
        "Click", "Enter", "Esc",
    ];

    /// SVG path data in an icon body (`M12 5v14`): letters and numbers, never words.
    fn is_path_data(s: &str) -> bool {
        s.starts_with('M') && s.chars().nth(1).is_some_and(|c| c.is_ascii_digit())
    }

    fn is_ui_text(s: &str) -> bool {
        let first = s.chars().next();
        first.is_some_and(|c| c.is_ascii_uppercase())
            && s.chars().any(|c| c.is_ascii_lowercase())
            && !s.contains("{}{")
            && !(s.contains('{') && !s.contains("{}"))
            && !s.contains("://")
            && !s.ends_with(".rs")
    }

    /// Every string literal in the source (plain `"..."`, not raw strings).
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
                // A char literal like '"' or a lifetime: skip a quoted char.
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
                        literal.push(match c {
                            'n' => '\n',
                            other => other,
                        });
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
