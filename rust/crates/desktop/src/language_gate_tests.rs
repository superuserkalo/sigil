//! The translations are parked behind the cargo feature `languages` (ROADMAP.md): a build without
//! it is English everywhere. A settings file that stores German (an older build, or a build with
//! the feature) keeps that choice on disk, but nothing German reaches the screen and Settings
//! offers no language. With the feature on, German works as before.

use crate::app::State;
use crate::model::Screen;
use crate::prefs::{LANGUAGES, Prefs};
use crate::screens::settings::i18n::{self, Lang};

/// A demo library whose stored settings choose German, as a build with the feature writes them.
fn state_in_german() -> State {
    let mut s = State::default();
    s.load_demo();
    s.screen = Screen::Installed;
    s.prefs = Prefs::from_json(r#"{"schema":1,"language":"de"}"#);
    s.prefs.reduce_motion = true;
    // What startup does with the stored choice (`settings::on_startup`). Only without the
    // feature: with it, the language is process-wide and would turn other tests German.
    if !LANGUAGES {
        i18n::set(s.prefs.language);
    }
    s
}

#[test]
fn the_stored_language_is_kept_whatever_the_build() {
    let p = state_in_german().prefs;
    assert_eq!(p.language, Lang::German, "the stored choice is read");
    let again = Prefs::from_json(&p.to_json());
    assert_eq!(again.language, Lang::German, "and written back unchanged");
    if !LANGUAGES {
        assert_eq!(i18n::current(), Lang::English);
        assert_eq!(i18n::tr("Settings"), "Settings");
    }
}

#[cfg(not(feature = "languages"))]
mod without_the_feature {
    use rune::{Env, Runtime};

    use super::*;
    use crate::app;
    use crate::screens::settings::{self, Tab};

    fn settle(rt: &mut Runtime<State>) {
        for n in 0..120 {
            if !rt.frame(1.0 / 60.0) && n > 2 {
                return;
            }
        }
    }

    /// Every text, label and key on screen that is German or the language choice.
    fn german_on_screen(s: State, what: &str) -> Vec<String> {
        let german: std::collections::HashSet<&str> = i18n::german_only().into_iter().collect();
        let mut rt = Runtime::new(s, app::view, (1360.0, 860.0), Env::new(true, 1.0));
        rt.check_live_only(false);
        settle(&mut rt);
        let mut found = Vec::new();
        for e in rt.elements().into_iter().filter(|e| !e.exiting) {
            if let Some(key) = &e.key
                && key.starts_with("settings-language")
            {
                found.push(format!("{what}: the language choice ({key})"));
            }
            for t in [e.text.clone(), e.label.clone()].into_iter().flatten() {
                if t.contains("Deutsch") || t.contains(['ä', 'ö', 'ü', 'Ä', 'Ö', 'Ü', 'ß']) || german.contains(t.as_str()) {
                    found.push(format!("{what}: {t}"));
                }
            }
        }
        found
    }

    #[test]
    fn english_only_without_the_feature() {
        // The sidebar and the Installed page under it.
        let mut found = german_on_screen(state_in_german(), "sidebar");
        // Settings: General, where the language choice sat, and About.
        for tab in [Tab::General, Tab::About] {
            let mut s = state_in_german();
            s.settings.tab = tab;
            settings::open(&mut s);
            found.extend(german_on_screen(s, tab.key()));
        }
        // A dialog: Before you launch, with a reset gameinfo.gi.
        let mut s = state_in_german();
        s.gameinfo.state = crate::banner::Gameinfo::NotListed {
            path: std::path::PathBuf::from("~/Steam/steamapps/common/Deadlock/game/citadel/gameinfo.gi"),
        };
        s.launch.dialog = crate::screens::launch::Dialog::Checklist;
        found.extend(german_on_screen(s, "dialog"));
        assert!(found.is_empty(), "German or the language choice on screen:\n{}", found.join("\n"));

        // Settings search finds no language row.
        for query in ["language", "deutsch", "sprache", "translation"] {
            let hits: Vec<_> = settings::search::matches(query).iter().map(|e| e.title).collect();
            assert!(!hits.contains(&"Language"), "{query}: {hits:?}");
        }
        // No German scene is listed.
        assert!(crate::all_scenes().iter().all(|(name, _)| !app::is_german_scene(name)));
    }
}
