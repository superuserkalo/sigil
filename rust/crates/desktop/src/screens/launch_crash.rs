//! The crash card's evidence: the exact file the game was loading, the console lines written
//! around the crash, and for a HUD mod, whether its layout is behind the game's.
//!
//! The reading is `deadlock_core::impact::crash_cause` (after Deadlock Mod Manager's crash
//! resource attribution, behaviour only, and Grimoire's crash evidence) and
//! `deadlock_core::addon_checks::hud_check` for the one layout the crash named. `launch.rs`
//! draws the card and calls in here for its Details block.

use deadlock_core::addon_checks::HudCheck;
use deadlock_core::impact::{Reason, Suspect};
use rune::{El, col, icon, row, text};

use crate::icons::TRIANGLE_ALERT;

use crate::screens::settings::i18n::fill;
use crate::ui::{MONO, SMALL, SMALL_NUM, Ui};

const DE: &[(&str, &str)] = &[
    ("The crash report names {}, which this mod replaces.", "Der Absturzbericht nennt {}, das dieser Mod ersetzt."),
    ("The crash report names a file this mod replaces.", "Der Absturzbericht nennt eine Datei, die dieser Mod ersetzt."),
    ("The game's console names {}, which this mod replaces.", "Die Konsole des Spiels nennt {}, das dieser Mod ersetzt."),
    ("The game's console names a file this mod replaces.", "Die Konsole des Spiels nennt eine Datei, die dieser Mod ersetzt."),
    ("Installed {} and not changed since.", "Installiert {} und seitdem unverändert."),
    ("Installed shortly before the crash.", "Kurz vor dem Absturz installiert."),
    ("Loading", "Geladen wurde"),
    ("No mod replaces this file: the game's own copy was loading.", "Kein Mod ersetzt diese Datei: Das Spiel lud seine eigene Kopie."),
    ("Replaced by the suspect above.", "Ersetzt durch den verdächtigen Mod oben."),
    ("Crash report", "Absturzbericht"),
    ("HUD layout", "HUD-Layout"),
    (
        "This layout lacks {} panels the current game build has. The mod is behind the game: update it or turn it off.",
        "Diesem Layout fehlen {} Panels, die die aktuelle Spielversion hat. Der Mod ist hinter dem Spiel zurück: Aktualisiere ihn oder schalte ihn aus.",
    ),
    ("This layout has every panel the current game build has.", "Dieses Layout hat alle Panels der aktuellen Spielversion."),
    ("This is the mod's own layout. The game has none to compare with.", "Das ist ein eigenes Layout des Mods. Das Spiel hat keins zum Vergleichen."),
    ("This layout could not be compared with the game's.", "Dieses Layout ließ sich nicht mit dem des Spiels vergleichen."),
    ("Missing: {}", "Fehlt: {}"),
    ("Console around the crash", "Konsole rund um den Absturz"),
    ("Game build {}", "Spielversion {}"),
    ("{} seconds after start", "{} Sekunden nach dem Start"),
];

fn tr(text: &str) -> &str {
    use crate::screens::settings::i18n::{self, Lang};
    if i18n::current() == Lang::German
        && let Some((_, de)) = DE.iter().find(|(en, _)| *en == text)
    {
        return de;
    }
    i18n::tr(text)
}

/// The line under the suspect's name, in the player's language.
pub fn reason_line(suspect: &Suspect, now: i64) -> String {
    match (suspect.reason, suspect.file.as_deref(), suspect.change.as_ref()) {
        (Reason::DumpNamesFile, Some(file), _) => fill(tr("The crash report names {}, which this mod replaces."), &[file]),
        (Reason::DumpNamesFile, None, _) => tr("The crash report names a file this mod replaces.").to_string(),
        (Reason::ConsoleNamesFile, Some(file), _) => fill(tr("The game's console names {}, which this mod replaces."), &[file]),
        (Reason::ConsoleNamesFile, None, _) => tr("The game's console names a file this mod replaces.").to_string(),
        (Reason::NewestChange, _, Some(change)) => fill(tr("Installed {} and not changed since."), &[&change.ago(now)]),
        (Reason::NewestChange, _, None) => tr("Installed shortly before the crash.").to_string(),
    }
}

/// What the crash evidence adds to the card. Read on the launch worker.
#[derive(Clone, Debug, Default)]
pub struct Evidence {
    /// The first resource the dump or console names, mod or not.
    pub resource: Option<String>,
    /// Console error lines written around the crash, newest first.
    pub console: Vec<String>,
    /// The HUD check of the layout the suspect replaces, when it is a layout.
    pub hud: Option<HudCheck>,
    pub build: Option<String>,
    pub uptime_seconds: Option<f64>,
}

/// One labelled group in the Details block: the label in small muted text, then its lines.
fn group(u: &Ui, key: &str, label: &str) -> El {
    col()
        .key(key.to_string())
        .w_full()
        .gap(2.0)
        .child(text(label.to_string()).font(SMALL).color(u.muted))
}

fn mono(u: &Ui, line: String) -> El {
    text(line).font(MONO).color(u.prose).wrap_anywhere().selectable(true)
}

/// The Details block of the crash card: the file the game was loading, the HUD check, the
/// console around the crash and the dump's own words, as labelled groups.
pub fn groups(u: &Ui, evidence: &Evidence, suspect: Option<&Suspect>, comment: Option<&str>) -> Vec<El> {
    let mut out = Vec::new();
    if let Some(resource) = &evidence.resource {
        let replaced = suspect
            .and_then(|suspect| suspect.file.as_deref())
            .is_some_and(|file| file.eq_ignore_ascii_case(resource));
        let mut facts = Vec::new();
        if let Some(build) = &evidence.build {
            facts.push(fill(tr("Game build {}"), &[build]));
        }
        if let Some(uptime) = evidence.uptime_seconds {
            facts.push(fill(tr("{} seconds after start"), &[&format!("{uptime:.0}")]));
        }
        let mut body = group(u, "launch-crash-resource", tr("Loading"))
            .child(mono(u, resource.clone()))
            .child(
                text(if replaced {
                    tr("Replaced by the suspect above.")
                } else {
                    tr("No mod replaces this file: the game's own copy was loading.")
                })
                .font(SMALL)
                .color(u.prose)
                .wrap(),
            );
        if !facts.is_empty() {
            body = body.child(text(facts.join(" · ")).font(SMALL_NUM).color(u.muted));
        }
        out.push(body);
    }
    if let Some(hud) = &evidence.hud {
        use deadlock_core::addon_checks::HudStatus;
        let (line, warn) = match hud.status {
            HudStatus::Compared if hud.is_outdated() => (
                fill(
                    tr("This layout lacks {} panels the current game build has. The mod is behind the game: update it or turn it off."),
                    &[&hud.missing_panels.len().to_string()],
                ),
                true,
            ),
            HudStatus::Compared => (tr("This layout has every panel the current game build has.").to_string(), false),
            HudStatus::NoBaseline => (tr("This is the mod's own layout. The game has none to compare with.").to_string(), false),
            HudStatus::Unsupported => (tr("This layout could not be compared with the game's.").to_string(), false),
        };
        let mut sentence = row().w_full().gap(6.0).items_start();
        if warn {
            sentence = sentence.child(icon(&TRIANGLE_ALERT, 12.0).color(u.amber).mt(2.0).a11y_hidden());
        }
        sentence = sentence.child(text(line).font(SMALL).color(u.prose).wrap().grow(1.0).min_w(0.0));
        let mut body = group(u, "launch-crash-hud", tr("HUD layout")).child(sentence);
        if !hud.missing_panels.is_empty() {
            let shown: Vec<&str> = hud.missing_panels.iter().take(8).map(String::as_str).collect();
            body = body.child(mono(u, fill(tr("Missing: {}"), &[&shown.join(", ")])));
        }
        out.push(body);
    }
    if !evidence.console.is_empty() {
        let mut body = group(u, "launch-crash-console", tr("Console around the crash"));
        for line in evidence.console.iter().take(4) {
            body = body.child(mono(u, line.clone()));
        }
        out.push(body);
    }
    if let Some(comment) = comment {
        let words: String = comment.trim().chars().take(400).collect();
        out.push(group(u, "launch-crash-comment", tr("Crash report")).child(mono(u, words)));
    }
    out
}

/// Read the evidence for one dump: the comment's resource and facts, the console tail when it
/// was written around the crash, and the HUD check when the suspect replaces a layout.
pub fn read(
    comment: Option<&str>,
    game_root: Option<&std::path::Path>,
    crash_secs: i64,
) -> Evidence {
    use deadlock_core::impact::crash_cause;
    let console = game_root
        .map(deadlock_core::game_session::console_log_path)
        .and_then(|path| crash_cause::read_console_tail(&path, crash_secs))
        .map(|tail| crash_cause::console_evidence(&tail))
        .unwrap_or_default();
    let facts = comment.map(crash_cause::parse_comment).unwrap_or_default();
    let resource = crash_cause::named_resources(comment, &console).into_iter().next().map(|(path, _)| path);
    Evidence {
        resource,
        console,
        hud: None,
        build: facts.build,
        uptime_seconds: facts.uptime_seconds,
    }
}

/// The HUD check of the one layout the suspect replaces. `vpk` is the suspect's directory VPK.
pub fn hud_for(suspect: &Suspect, vpk: &std::path::Path, game_root: &std::path::Path, scratch: &std::path::Path) -> Option<HudCheck> {
    let file = suspect.file.as_deref()?;
    let lower = file.to_ascii_lowercase();
    if !(lower.starts_with("panorama/layout/") && lower.ends_with(".vxml_c")) {
        return None;
    }
    let citadel = game_root.join("game").join("citadel");
    deadlock_core::addon_checks::hud_check(
        &[(suspect.slot, vpk.to_path_buf())],
        &citadel.join("pak01_dir.vpk"),
        &citadel.join("console.log"),
        scratch,
    )
    .ok()?
    .into_iter()
    .find(|check| check.layout.eq_ignore_ascii_case(file))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_german_table_is_clean() {
        let mut seen = std::collections::HashSet::new();
        for (en, de) in DE {
            assert!(seen.insert(*en), "duplicate {en:?}");
            assert_eq!(en.matches("{}").count(), de.matches("{}").count(), "holes in {en:?}");
            assert!(!en.contains('\u{2014}') && !de.contains('\u{2014}'));
        }
        let source = include_str!("launch_crash.rs");
        let body = source.split("#[cfg(test)]").next().unwrap();
        for (at, _) in body.match_indices("tr(\"") {
            let rest = &body[at + 4..];
            let literal = &rest[..rest.find('"').unwrap()];
            assert!(DE.iter().any(|(en, _)| *en == literal), "untranslated {literal:?}");
        }
    }

    #[test]
    fn evidence_reads_the_comment() {
        let comment = "error\nAbortMessage({FATAL ERROR: Unable to load layout file 'file://{resources}/layout/hud.xml'})\nUptime( 12.5 )\nBuild: 6100\n";
        let evidence = read(Some(comment), None, 0);
        assert_eq!(evidence.resource.as_deref(), Some("panorama/layout/hud.vxml_c"));
        assert_eq!(evidence.build.as_deref(), Some("6100"));
        assert_eq!(evidence.uptime_seconds, Some(12.5));
        assert!(evidence.console.is_empty());
    }
}
