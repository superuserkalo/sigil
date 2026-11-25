//! The mod agent is out of builds without the cargo feature `agent` (ROADMAP.md): a settings file
//! with the Agent switch on still shows no Agent anywhere, routes nothing to it and starts no
//! host. With the feature on, the switch works as before.

#[cfg(not(feature = "agent"))]
use rune::{Env, Runtime};

use crate::app::{self, State};
use crate::model::Screen;
use crate::prefs::{self, AGENT, AGENT_FLAG, Prefs};
#[cfg(not(feature = "agent"))]
use crate::screens::settings::{self, Tab};

/// A demo library whose stored settings have the Agent switch on, as an older build or an owner
/// build with the feature writes them.
fn state_with_agent_on() -> State {
    let mut s = State::default();
    s.load_demo();
    s.screen = Screen::Installed;
    s.prefs = Prefs::from_json(r#"{"schema":1,"flags":{"agent":true}}"#);
    s.prefs.reduce_motion = true;
    s
}

#[cfg(not(feature = "agent"))]
fn settle(rt: &mut Runtime<State>) {
    for n in 0..120 {
        if !rt.frame(1.0 / 60.0) && n > 2 {
            return;
        }
    }
}

#[cfg(not(feature = "agent"))]
/// Every text and label on screen that names the agent.
fn agent_words(s: State, what: &str) -> Vec<String> {
    let mut rt = Runtime::new(s, app::view, (1360.0, 860.0), Env::new(true, 1.0));
    rt.check_live_only(false);
    settle(&mut rt);
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .flat_map(|e| [e.text.clone(), e.label.clone(), e.key.clone()])
        .flatten()
        // The data folder's old name may still show where this machine has not moved it yet.
        .filter(|t| {
            let mut t = t.clone();
            for leaf in deadlock_core::app_data::LEGACY_LEAVES {
                t = t.replace(leaf, "");
            }
            t.to_lowercase().contains("agent")
        })
        .map(|t| format!("{what}: {t}"))
        .collect()
}

#[cfg(not(feature = "agent"))]
const TABS: [Tab; 13] = [
    Tab::Game,
    Tab::Backups,
    Tab::Mods,
    Tab::General,
    Tab::Appearance,
    Tab::Integrations,
    Tab::Privacy,
    Tab::Models,
    Tab::Network,
    Tab::Logs,
    Tab::Storage,
    Tab::Experimental,
    Tab::About,
];

#[test]
fn the_stored_switch_is_kept_whatever_the_build() {
    let p = state_with_agent_on().prefs;
    assert_eq!(p.flag(AGENT_FLAG), AGENT);
    assert_eq!(p.page_on(Screen::Agent), AGENT);
    // The stored value survives a save, so a build with the agent sees it again.
    assert!(p.flags.get(AGENT_FLAG) == Some(&true));
    let again = Prefs::from_json(&p.to_json());
    assert!(again.flags.get(AGENT_FLAG) == Some(&true));
    assert_eq!(prefs::experimental_flags().any(|f| f.id == AGENT_FLAG), AGENT);
    assert_eq!(crate::shell_tips::tour_stops().len(), if AGENT { 4 } else { 3 });
}

#[cfg(not(feature = "agent"))]
#[test]
fn no_agent_anywhere_without_the_feature() {
    // No route: the shortcut, a link or a button that opens Agent or a page under it does nothing.
    let mut s = state_with_agent_on();
    for screen in [Screen::Agent, Screen::Projects, Screen::Index, Screen::Patches, Screen::Toolchain, Screen::Release] {
        s.go(screen);
        assert!(s.screen == Screen::Installed, "{} opened", screen.key());
    }
    for key in ['7', '8', '9'] {
        if let Some(screen) = Screen::from_command(key) {
            s.go(screen);
        }
        assert!(s.screen == Screen::Installed, "the {key} shortcut opened a page");
    }
    // No host process, whatever asks for one.
    s.ensure_host();
    s.start_host();
    assert!(!s.agent.has_process(), "the agent host started");

    // Nothing on screen names it: the sidebar, every Settings category (Models included, which
    // opens General), the shortcuts sheet and the experimental tour.
    let mut found = agent_words(state_with_agent_on(), "page");
    for tab in TABS {
        let mut s = state_with_agent_on();
        s.settings.tab = tab;
        settings::open(&mut s);
        assert!(s.settings.tab.shown());
        found.extend(agent_words(s, tab.key()));
    }
    let mut s = state_with_agent_on();
    s.shell.shortcuts = true;
    found.extend(agent_words(s, "shortcuts"));
    for stop in 0..crate::shell_tips::tour_stops().len() {
        let mut s = state_with_agent_on();
        settings::open(&mut s);
        s.shell.tips.tour = Some(stop);
        found.extend(agent_words(s, "tour"));
    }
    assert!(found.is_empty(), "agent text on screen:\n{}", found.join("\n"));

    // Settings search finds none of its rows.
    for query in ["agent", "model", "sign in", "chatgpt", "ask sigil", "llm"] {
        let hits: Vec<_> = settings::search::matches(query).iter().map(|e| e.title).collect();
        assert!(
            hits.iter().all(|t| !t.to_lowercase().contains("agent") && *t != "Accounts" && *t != "Model id" && *t != "Local endpoint" && *t != "Ask Sigil for help"),
            "{query}: {hits:?}"
        );
    }
}

#[cfg(not(feature = "agent"))]
#[test]
fn agent_scenes_are_not_listed_without_the_feature() {
    let scenes = crate::all_scenes();
    for name in app::AGENT_SCENES {
        assert!(scenes.iter().all(|(n, _)| n != name), "{name} is listed");
    }
    // Running every scene changes process-wide state (the language, caches), so the sweep runs
    // in a process of its own.
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["agent_gate_tests::listed_scenes_stay_off_the_agent", "--exact", "--ignored", "--test-threads=1"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success() && stdout.contains("1 passed"), "{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
}

/// Every listed scene stays off the agent's pages and Models. A scene that needs fixtures a bare
/// state lacks is skipped (it panics on the missing fixture, not on the agent). Run by
/// `agent_scenes_are_not_listed_without_the_feature` in its own process.
#[cfg(not(feature = "agent"))]
#[test]
#[ignore = "run in its own process by agent_scenes_are_not_listed_without_the_feature"]
fn listed_scenes_stay_off_the_agent() {
    let scenes = crate::all_scenes();
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let scratch = std::env::temp_dir().join(format!("sigil-agent-gate-scenes-{}", std::process::id()));
    let mut checked = 0;
    let mut wrong = Vec::new();
    for (name, scene) in scenes {
        let run = std::panic::catch_unwind(|| {
            let mut s = State::default();
            // Scenes that write fixtures write them here, not under the placeholder `~` path.
            s.paths.data = scratch.display().to_string();
            scene(&mut s);
            (s.screen, s.settings.open && s.settings.tab == Tab::Models)
        });
        if let Ok((screen, models)) = run {
            checked += 1;
            if app::under_agent(screen) || models {
                wrong.push(format!("{name} shows {}", if models { "Models" } else { screen.key() }));
            }
        }
    }
    std::panic::set_hook(hook);
    let _ = std::fs::remove_dir_all(&scratch);
    assert!(wrong.is_empty(), "{wrong:?}");
    assert!(checked > scenes.len() / 2, "only {checked} of {} scenes ran", scenes.len());
}

#[cfg(feature = "agent")]
#[test]
fn the_feature_lists_the_agent_scenes_and_routes_to_agent() {
    let scenes = crate::all_scenes();
    for name in app::AGENT_SCENES {
        assert!(scenes.iter().any(|(n, _)| n == name), "{name} is missing");
    }
    let mut s = state_with_agent_on();
    s.go(Screen::Agent);
    assert!(s.screen == Screen::Agent);
}
