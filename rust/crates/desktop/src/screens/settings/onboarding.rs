//! First run: a short setup over the window (Grimoire's welcome steps, MIT). Three steps before
//! the first install: find the game, make gameinfo.gi load mods, and one privacy step with the
//! choices that must be asked (the update check and Help build Sigil, both off by default).
//! Skip finishes it too. Settings > General can show it again, and reset the page tips.
//!
//! Everything else lives in Settings and its pages: the proxy and offline mode in Settings >
//! Network, mods already in the addons folder in Installed, and autoexec.cfg in Autoexec (saving
//! a command there creates the file).
//!
//! Tips: the first visit to a page shows one line about it as a notification, once. They can
//! be turned off and shown again from Settings.

use deadlock_core::deploy;
use rune::{El, Key, KeyEvent, Tx, col, el, row, spacer, text};

use super::i18n::{fill, tr};
use super::options;
use super::{button, say, section, setting_row, toggle_row};
use crate::app::State;
use crate::model::Screen;
use crate::notify::{self, Notice};
use crate::ui::{ActionKind, BODY, BODY_MED, DISPLAY, MONO, SMALL, Ui};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Step {
    #[default]
    Game,
    Gameinfo,
    /// The update check and "Help build Sigil" (ADR 0017): opt-in product data, answered with
    /// Share or Not now, which also finishes the setup.
    Privacy,
}

/// Height the privacy step adds above the Help build Sigil list (the intro, the update switch,
/// the rule and the heading), so the list scrolls instead of pushing the buttons off a short
/// window.
const PRIVACY_EXTRA_H: f32 = 150.0;

impl Step {
    pub const ALL: [Step; 3] = [Step::Game, Step::Gameinfo, Step::Privacy];

    fn index(self) -> usize {
        Step::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    fn next(self) -> Option<Step> {
        Step::ALL.get(self.index() + 1).copied()
    }

    fn back(self) -> Option<Step> {
        self.index().checked_sub(1).map(|i| Step::ALL[i])
    }

    fn title(self) -> &'static str {
        match self {
            Step::Game => "Find Deadlock",
            Step::Gameinfo => "Let the game load mods",
            Step::Privacy => "Updates and privacy",
        }
    }
}

/// Show the setup at startup when it was never finished.
pub fn on_startup(st: &mut State) {
    if !options::get().onboarded && !st.demo && !rune::is_headless() {
        st.settings.extra.onboarding = Some(Step::Game);
    }
}

fn finish(st: &mut State) {
    options::edit(st, |o| o.onboarded = true);
    st.settings.extra.onboarding = None;
    st.go(Screen::Browse);
}

fn gameinfo_state(st: &State) -> (bool, bool) {
    // (present, configured)
    let Some(root) = &st.game_root else { return (false, false) };
    let path = deploy::gameinfo_path(root);
    match std::fs::read_to_string(&path) {
        Ok(text) => (true, text.lines().any(|l| {
            let w: Vec<&str> = l.split("//").next().unwrap_or("").split_whitespace().collect();
            w.len() == 2 && w[0].eq_ignore_ascii_case("Game") && w[1].eq_ignore_ascii_case("citadel/addons")
        })),
        Err(_) => (false, false),
    }
}

fn fix_gameinfo(st: &mut State) {
    if st.demo {
        return;
    }
    let Some(root) = st.game_root.clone() else { return };
    match deploy::repair_gameinfo(&deploy::gameinfo_path(&root), true) {
        Ok(_) => crate::data::rescan_game(st),
        Err(e) => st.settings.extra.onboarding_error = Some(e.to_string()),
    }
}

fn choose(_st: &mut State, tx: &mut Tx) {
    let options = rune::FileDialogOptions {
        title: tr("Choose the Deadlock folder").into(),
        directory: super::editor::dialog_dir(),
        ..Default::default()
    };
    tx.open_folder(options, |st: &mut State, result, _tx| {
        if let Ok(Some(path)) = result {
            super::editor::remember(&path);
            super::set_game_dir(st, path);
        }
    });
}

fn step_body(s: &State, u: &Ui, step: Step) -> El {
    let line = |t: &str| text(tr(t).to_string()).font(BODY).color(u.muted).wrap().w_full();
    let status = |ok: bool, t: String| {
        row()
            .w_full()
            .gap(8.0)
            .center_y()
            .child(
                rune::icon(if ok { &crate::icons::CIRCLE_CHECK } else { &crate::icons::TRIANGLE_ALERT }, 16.0)
                    .color(if ok { u.teal } else { u.amber })
                    .a11y_hidden(),
            )
            .child(text(t).font(BODY_MED).color(u.ink).grow(1.0).min_w(0.0).truncate())
    };
    match step {
        Step::Game => {
            let found = s.game_detected;
            col()
                .w_full()
                .gap(12.0)
                .child(line("Sigil installs mods into the game's addons folder. It looks in your Steam libraries first."))
                .child(status(
                    found,
                    if found { s.paths.game.clone() } else { tr("Not found in your Steam libraries").to_string() },
                ))
                .child(
                    row()
                        .gap(8.0)
                        .child(button(u, "onboarding-detect", "Detect", ActionKind::Secondary).on_press(|st: &mut State| {
                            crate::data::rescan_game(st)
                        }))
                        .child(button(u, "onboarding-choose", "Choose folder", ActionKind::Secondary).on_press_tx(choose)),
                )
                .child(text(tr("Usually Steam/steamapps/common/Deadlock")).font(MONO).color(u.faint))
        }
        Step::Gameinfo => {
            let (present, ok) = gameinfo_state(s);
            col()
                .w_full()
                .gap(12.0)
                .child(line("The game reads mods only when gameinfo.gi lists citadel/addons. Game updates reset it; Sigil keeps a backup before any change."))
                .child(status(
                    ok,
                    if ok {
                        tr("gameinfo.gi is set up").to_string()
                    } else if present {
                        tr("gameinfo.gi needs the addons line").to_string()
                    } else {
                        tr("No gameinfo.gi yet. Find the game first.").to_string()
                    },
                ))
                .child(row().child(super::disabled(
                    button(u, "onboarding-fix", "Fix", ActionKind::Primary).on_press(fix_gameinfo),
                    ok || !present,
                )))
        }
        Step::Privacy => col()
            .w_full()
            .gap(12.0)
            .child(line("Sigil goes online for GameBanana, the features you use and, if you want it, the update check. Crash reports, usage events, unknown-mod lookup and Help build Sigil are sent only if you turn them on. Settings > Privacy lists every host and switch."))
            .child(
                row()
                    .w_full()
                    .gap(12.0)
                    .center_y()
                    .child(text(tr("Check for Sigil updates")).font(BODY_MED).color(u.ink).grow(1.0))
                    .child(
                        u.toggle(s.prefs.check_updates)
                            .key("onboarding-updates")
                            .label(tr("Check for Sigil updates"))
                            .cursor(rune::Cursor::Pointer)
                            .on_press(|st: &mut State| crate::prefs::edit(st, |p| p.check_updates = !p.check_updates)),
                    ),
            )
            .child(el().w_full().h(1.0).fill(u.hair))
            .child(text(tr(super::product::TITLE).to_string()).font(BODY_MED).color(u.ink))
            .child(super::product::consent_body_reserving(u, PRIVACY_EXTRA_H)),
    }
}

/// The setup over the window, while it runs.
pub fn overlay(s: &State, u: &Ui) -> Option<El> {
    let step = s.settings.extra.onboarding?;
    let mut dots = row().gap(6.0).center_y();
    for (i, _) in Step::ALL.iter().enumerate() {
        dots = dots.child(
            el().size(if i == step.index() { 18.0 } else { 6.0 }, 6.0)
                .radius(crate::ui::r(3.0))
                .fill(if i <= step.index() { u.accent } else { u.hair_strong }),
        );
    }
    let last = step.next().is_none();
    let ready = s.game_detected;
    let mut card = u
        .dialog_shell()
        .key("onboarding")
        .role("dialog")
        .label(tr("Welcome to Sigil"))
        .w(520.0)
        .padding(32.0)
        .gap(20.0)
        .modal(true, |_st: &mut State| {})
        .on_key(|st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                finish(st);
                true
            } else {
                false
            }
        })
        .child(
            row()
                .w_full()
                .center_y()
                .child(text(fill(tr("Step {} of {}"), &[&(step.index() + 1).to_string(), &Step::ALL.len().to_string()])).font(SMALL).color(u.muted))
                .child(spacer())
                .child(dots),
        )
        .child(text(tr(step.title()).to_string()).font(DISPLAY).color(u.ink))
        .child(step_body(s, u, step));
    if let Some(error) = &s.settings.extra.onboarding_error {
        card = card.child(crate::notify::feedback::error_line(u, "onboarding-error", error));
    }
    let mut buttons = row().w_full().gap(8.0).pt(8.0).center_y().child(
        button(u, "onboarding-skip", "Skip for now", ActionKind::Ghost).on_press(finish),
    );
    buttons = buttons.child(spacer());
    if let Some(back) = step.back() {
        buttons = buttons.child(button(u, "onboarding-back", "Back", ActionKind::Secondary).on_press(move |st: &mut State| {
            st.settings.extra.onboarding = Some(back);
            st.settings.extra.onboarding_error = None;
        }));
    }
    buttons = buttons.child(if last {
        // The answer finishes the setup: "Not now" and "Share" weigh the same.
        super::product::answer_buttons(u, finish)
    } else {
        super::disabled(
            button(u, "onboarding-next", "Next", ActionKind::Primary).on_press(move |st: &mut State| {
                st.settings.extra.onboarding = step.next();
                st.settings.extra.onboarding_error = None;
            }),
            step == Step::Game && !ready,
        )
    });
    Some(
        el().key("onboarding-layer")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .child(el().absolute(0.0, 0.0).w_full().h_full().fill(u.scrim))
            .child(el().w_full().h_full().center().child(card.child(buttons))),
    )
}

// --- tips ----------------------------------------------------------------------------------

/// One line per page, shown the first time the page opens.
pub const TIPS: [(&str, &str); 7] = [
    ("browse", "Press a mod for its pages, files and comments. Right-click hides a mod or its creator."),
    ("installed", "Drag mods to change their order: a lower slot wins when two mods change the same file."),
    ("locker", "The Locker groups skins by hero. Turn one on and Sigil turns the hero's other skins off."),
    ("profiles", "A profile saves which mods are on. Share it as a code from its menu."),
    ("autoexec", "Commands here run every time Deadlock starts. Sigil keeps a backup before each save."),
    ("performance", "Pinned values stay when a config updates. Discard them to go back to the author's."),
    ("crosshair", "Copy a code to share your crosshair, or paste one to try someone else's."),
];

pub fn tip_for(key: &str) -> Option<&'static str> {
    TIPS.iter().find(|(k, _)| *k == key).map(|(_, t)| *t)
}

/// Runs every poll: a page opened for the first time shows its tip.
pub fn poll_tips(st: &mut State) -> bool {
    let key = st.screen.key();
    if st.settings.extra.last_screen.as_deref() == Some(key) {
        return false;
    }
    st.settings.extra.last_screen = Some(key.to_string());
    let o = options::get();
    if !o.tips || !o.onboarded || st.settings.extra.onboarding.is_some() || st.demo || rune::is_headless() {
        return false;
    }
    if o.tips_seen.iter().any(|k| k == key) {
        return false;
    }
    let Some(tip) = tip_for(key) else { return false };
    let owned = key.to_string();
    options::edit(st, |o| o.tips_seen.push(owned));
    notify::push(st, Notice::info(tr("Tip")).detail(tr(tip)));
    true
}

pub fn rows(_s: &State, u: &Ui) -> El {
    let o = options::get();
    section(
        u,
        "Setup and tips",
        super::misc_w4::tip_rows(u)
            .into_iter()
            .chain(vec![
            toggle_row(u, "settings-tips", "Show page tips", "One line about a page, the first time you open it.", o.tips, |st| {
                options::edit(st, |o| o.tips = !o.tips)
            }),
            setting_row(
                u,
                "settings-tips-reset",
                "Reset tips",
                super::detail(u, ""),
                super::disabled(
                    button(u, "settings-tips-reset-run", "Reset", ActionKind::Secondary).on_press(|st: &mut State| {
                        options::edit(st, |o| {
                            o.tips_seen.clear();
                            o.tips = true;
                        });
                        say(st, tr("Tips will show again."), false);
                    }),
                    o.tips_seen.is_empty() && o.tips,
                ),
            ),
            setting_row(
                u,
                "settings-setup-again",
                "Setup",
                super::detail(u, "Walk through finding the game, gameinfo.gi and the privacy choices again."),
                button(u, "settings-setup-run", "Run setup", ActionKind::Secondary).on_press(|st: &mut State| {
                    options::edit(st, |o| o.onboarded = false);
                    super::close(st);
                    st.settings.extra.onboarding = Some(Step::Game);
                }),
            ),
        ])
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_walk_forward_and_back() {
        assert_eq!(Step::Game.next(), Some(Step::Gameinfo));
        assert_eq!(Step::Gameinfo.next(), Some(Step::Privacy));
        assert_eq!(Step::Privacy.next(), None);
        assert_eq!(Step::Game.back(), None);
        assert_eq!(Step::Gameinfo.back(), Some(Step::Game));
        assert_eq!(Step::Privacy.back(), Some(Step::Gameinfo));
    }

    /// Three steps before the first install, and the last one asks Help build Sigil.
    #[test]
    fn setup_is_three_steps_ending_on_privacy() {
        assert_eq!(Step::ALL, [Step::Game, Step::Gameinfo, Step::Privacy]);
        assert_eq!(Step::default(), Step::Game);
        let source = include_str!("onboarding.rs");
        let privacy = &source[source.find("Step::Privacy => col()").expect("privacy body")..];
        let privacy = &privacy[..privacy.find("\n    }\n}").expect("end of body")];
        assert!(privacy.contains("product::consent_body_reserving"), "the privacy step asks Help build Sigil");
        assert!(privacy.contains("onboarding-updates"), "the privacy step holds the update check");
        assert!(source.contains("super::product::answer_buttons(u, finish)"), "Share or Not now finishes the setup");
    }

    /// The consent switches the setup asks are off until the player turns them on.
    #[test]
    fn consent_switches_start_off() {
        assert!(!crate::prefs::Prefs::default().check_updates);
        assert!(!deadlock_core::telemetry::Consent::default().product);
    }

    #[test]
    fn every_tip_names_a_page() {
        for (key, _) in TIPS {
            assert!(
                [Screen::Browse, Screen::Installed, Screen::Locker, Screen::Profiles, Screen::Autoexec, Screen::Performance, Screen::Crosshair]
                    .iter()
                    .any(|s| s.key() == key),
                "{key}"
            );
        }
    }
}
