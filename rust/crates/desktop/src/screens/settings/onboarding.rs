//! First run: a short setup over the window (Grimoire's welcome steps, MIT): find the game,
//! make gameinfo.gi load mods, an autoexec.cfg, and the update check. Skip finishes it too.
//! Settings > General can show it again, and reset the page tips.
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
    /// Wave 4 (DMM's setup): a proxy or offline before Sigil goes online.
    Network,
    /// Wave 4 (DMM's setup): mods already in the addons folder.
    Mods,
    Autoexec,
    Privacy,
    /// "Help build Sigil" (ADR 0017): opt-in product data, Share or Not now.
    Share,
}

impl Step {
    const ALL: [Step; 7] = [Step::Game, Step::Gameinfo, Step::Network, Step::Mods, Step::Autoexec, Step::Privacy, Step::Share];

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
            Step::Network => "Your connection",
            Step::Mods => "Mods you already have",
            Step::Autoexec => "Your autoexec",
            Step::Privacy => "Updates and privacy",
            Step::Share => super::product::TITLE,
        }
    }
}

/// Show the setup at startup when it was never finished.
pub fn on_startup(st: &mut State) {
    if !options::get().onboarded && !st.demo && !rune::is_headless() {
        st.settings.extra.onboarding = Some(Step::Game);
        st.settings.proxy_edit = rune::TextEdit::new(st.prefs.proxy.clone().unwrap_or_default());
    }
}

fn finish(st: &mut State) {
    options::edit(st, |o| o.onboarded = true);
    st.settings.extra.onboarding = None;
    let next = st.settings.extra.after_onboarding.take().unwrap_or(Screen::Browse);
    st.go(next);
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

fn create_autoexec(st: &mut State) {
    if st.demo {
        return;
    }
    let Some(root) = st.game_root.clone() else { return };
    if deadlock_core::autoexec_path(&root).is_file() {
        return;
    }
    match deadlock_core::write_autoexec(&root, "// Sigil: commands here run when Deadlock starts.\n", true) {
        Ok(_) => st.autoexec.loaded = false,
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
        Step::Network => {
            let typed = s.settings.proxy_edit.text.trim().to_string();
            let valid = typed.is_empty() || deadlock_core::net::validate_proxy(&typed).is_ok();
            col()
                .w_full()
                .gap(12.0)
                .child(line("Sigil reads GameBanana and downloads mods. Behind a proxy or a firewall, set the proxy here; HTTP, HTTPS and SOCKS5 work. You can change it later in Settings > Network."))
                .child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .center_y()
                        .child(
                            super::field(
                                rune::text_input(&s.settings.proxy_edit, MONO)
                                    .key("onboarding-proxy")
                                    .label(tr("Proxy address"))
                                    .single_line()
                                    .grow(1.0)
                                    .color(u.ink)
                                    .placeholder("socks5://127.0.0.1:1080", u.faint)
                                    .caret(u.accent, u.accent.alpha(0.22))
                                    .on_edit(|st: &mut State| &mut st.settings.proxy_edit),
                                u,
                            )
                            .grow(1.0)
                            .min_w(0.0),
                        )
                        .child(super::disabled(
                            button(u, "onboarding-proxy-save", "Save", ActionKind::Secondary).on_press(|st: &mut State| {
                                let typed = st.settings.proxy_edit.text.trim().to_string();
                                let value = (!typed.is_empty()).then_some(typed);
                                match deadlock_core::net::set_proxy(value.clone()) {
                                    Ok(()) => crate::prefs::edit(st, |p| p.proxy = value),
                                    Err(error) => {
                                        let error = tr(&error).to_string();
                                        say(st, error, true)
                                    }
                                }
                            }),
                            !valid || typed == s.prefs.proxy.clone().unwrap_or_default(),
                        )),
                )
                .child(status(
                    s.prefs.proxy.is_none() || valid,
                    match &s.prefs.proxy {
                        Some(proxy) => fill(tr("Using {} for Browse and mod downloads."), &[proxy]),
                        None => tr("Browse and mod downloads connect directly.").to_string(),
                    },
                ))
                .child(
                    row()
                        .w_full()
                        .gap(12.0)
                        .center_y()
                        .child(text(tr("Offline mode")).font(BODY_MED).color(u.ink).grow(1.0))
                        .child(
                            u.toggle(s.prefs.offline)
                                .key("onboarding-offline")
                                .label(tr("Offline mode"))
                                .cursor(rune::Cursor::Pointer)
                                .on_press(|st: &mut State| {
                                    crate::prefs::edit(st, |p| p.offline = !p.offline);
                                    super::apply_offline(st.prefs.offline);
                                }),
                        ),
                )
        }
        Step::Mods => {
            let count = s.addons.addons.len();
            col()
                .w_full()
                .gap(12.0)
                .child(line("Mods already in the addons folder stay where they are. Installed lists them; mods Sigil does not know yet can be named there by looking them up."))
                .child(status(
                    s.addons.scanned,
                    if !s.addons.scanned {
                        tr("Looking in the addons folder").to_string()
                    } else if count == 0 {
                        tr("No mods in the addons folder yet").to_string()
                    } else {
                        fill(tr("{} mods found in the addons folder"), &[&count.to_string()])
                    },
                ))
                .child(
                    row()
                        .w_full()
                        .gap(12.0)
                        .center_y()
                        .child(text(tr("Open Installed when done")).font(BODY_MED).color(u.ink).grow(1.0))
                        .child(super::disabled(
                            u.toggle(s.settings.extra.after_onboarding == Some(Screen::Installed))
                                .key("onboarding-mods-open")
                                .label(tr("Open Installed when done"))
                                .cursor(rune::Cursor::Pointer)
                                .on_press(|st: &mut State| {
                                    let next = &mut st.settings.extra.after_onboarding;
                                    *next = if next.is_some() { None } else { Some(Screen::Installed) };
                                }),
                            count == 0,
                        )),
                )
        }
        Step::Autoexec => {
            let exists = s.game_root.as_ref().is_some_and(|r| deadlock_core::autoexec_path(r).is_file());
            col()
                .w_full()
                .gap(12.0)
                .child(line("autoexec.cfg holds console commands the game runs at start: crosshairs, binds, settings. Optional."))
                .child(status(exists, if exists { tr("autoexec.cfg is there").to_string() } else { tr("No autoexec.cfg yet").to_string() }))
                .child(row().child(super::disabled(
                    button(u, "onboarding-autoexec", "Create", ActionKind::Secondary).on_press(create_autoexec),
                    exists || s.game_root.is_none(),
                )))
        }
        Step::Share => super::product::consent_body(u),
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
            .child(
                row().child(
                    u.action(tr("What Sigil sends"), ActionKind::Ghost)
                        .key("onboarding-privacy-doc")
                        .role("link")
                        .cursor(rune::Cursor::Pointer)
                        .on_press_tx(|st: &mut State, tx: &mut Tx| super::tabs::open_url(st, tx, super::telemetry::PRIVACY_DOC)),
                ),
            ),
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
    buttons = buttons.child(if step == Step::Share {
        // The answer finishes the setup: "Not now" and "Share" weigh the same.
        super::product::answer_buttons(u, finish)
    } else if last {
        button(u, "onboarding-done", "Get started", ActionKind::Primary).on_press(finish)
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
                super::detail(u, "Walk through finding the game, gameinfo.gi and your autoexec again."),
                button(u, "settings-setup-run", "Run setup", ActionKind::Secondary).on_press(|st: &mut State| {
                    options::edit(st, |o| o.onboarded = false);
                    super::close(st);
                    st.settings.extra.onboarding = Some(Step::Game);
                    st.settings.proxy_edit = rune::TextEdit::new(st.prefs.proxy.clone().unwrap_or_default());
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
        assert_eq!(Step::Privacy.next(), Some(Step::Share));
        assert_eq!(Step::Share.next(), None);
        assert_eq!(Step::Game.back(), None);
        assert_eq!(Step::Gameinfo.next(), Some(Step::Network));
        assert_eq!(Step::Autoexec.back(), Some(Step::Mods));
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
