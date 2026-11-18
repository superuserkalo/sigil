//! The gameinfo.gi banner. Game updates and Steam's file check put back the game's own
//! gameinfo.gi, which drops the `Game citadel/addons` search path, and from then on the game
//! ignores every mod. When the game is found and the file no longer lists that path, a banner
//! sits at the top of every page with a Fix (`deploy::repair_gameinfo` behind a confirm) and a
//! way to the Game settings.
//!
//! The file is read on a worker (`Job`), never in a frame: at startup, after a launch, when the
//! window comes back to the front, and after a rescan.

use std::path::PathBuf;

use deadlock_core::discovery::GameInfoStatus;
use rune::{El, Key, KeyEvent, Tx, col, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::jobs::Job;
use crate::model::Screen;
use crate::notify::{self, Notice};
use crate::ui::{ActionKind, BODY, H1, PAGE_X, Ui};

/// What the last read found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Gameinfo {
    /// Not read yet, no game found, or the read failed: no banner.
    #[default]
    Unknown,
    /// The addons search path is listed: no banner.
    Loads,
    /// The file is there but does not list the addons path: the banner with Fix.
    NotListed { path: PathBuf },
    /// The file itself is gone: the banner, without Fix (Steam has to restore it).
    Missing { path: PathBuf },
    /// Mods load, but lines a modded game needs are missing (the Mod and Write paths, without
    /// which it can crash at launch, or the language mounts): the banner with Fix.
    Incomplete { path: PathBuf, missing: Vec<String> },
    /// No Deadlock install was found: the banner points at Settings > Game to choose it.
    NoGame,
}

#[derive(Default)]
pub struct Banner {
    pub state: Gameinfo,
    pub job: Job<(PathBuf, GameInfoStatus)>,
    /// A recheck asked for while one ran: read again when it lands.
    again: bool,
    /// The Fix confirm is open.
    pub confirm: bool,
}

/// Read gameinfo.gi again on a worker. Does nothing in demo data, headless renders (their
/// pictures must not depend on this machine's game) or without a game folder.
pub fn recheck(st: &mut State) {
    if st.demo || rune::is_headless() {
        return;
    }
    let Some(root) = st.game_root.clone() else {
        st.gameinfo.state = Gameinfo::NoGame;
        return;
    };
    if st.gameinfo.job.is_running() {
        st.gameinfo.again = true;
        return;
    }
    let wake = st.wake.clone();
    st.gameinfo.job.start(wake, move || {
        let path = deadlock_core::deploy::gameinfo_path(&root);
        deadlock_core::discovery::read_game_info_status(&path)
            .map(|status| (path, status))
            .map_err(|error| error.to_string())
    });
}

/// From the shell's poll. True when the banner changed.
pub fn poll(st: &mut State) -> bool {
    let Some(result) = st.gameinfo.job.try_take() else {
        return false;
    };
    let before = st.gameinfo.state.clone();
    st.gameinfo.state = match result {
        Ok((path, status)) => state_for(path, &status),
        Err(_) => Gameinfo::Unknown,
    };
    if std::mem::take(&mut st.gameinfo.again) {
        recheck(st);
    }
    st.gameinfo.state != before
}

pub fn state_for(path: PathBuf, status: &GameInfoStatus) -> Gameinfo {
    if !status.present {
        Gameinfo::Missing { path }
    } else if status.addons_search_path && status.missing_paths.is_empty() {
        Gameinfo::Loads
    } else if status.addons_search_path {
        Gameinfo::Incomplete { path, missing: status.missing_paths.clone() }
    } else {
        Gameinfo::NotListed { path }
    }
}

/// GAME WRITE, from the confirm's Fix button only: insert the addons line into gameinfo.gi
/// (`deploy::repair_gameinfo` copies the file aside first), then rescan. The success toast
/// offers Undo, which restores that copy.
fn fix(st: &mut State) {
    st.gameinfo.confirm = false;
    let path = match st.gameinfo.state.clone() {
        Gameinfo::NotListed { path } | Gameinfo::Incomplete { path, .. } => path,
        _ => return,
    };
    if st.demo {
        st.gameinfo.state = Gameinfo::Loads;
        notify::push(st, Notice::info("Demo data: nothing was changed."));
        return;
    }
    // Wave 4: the owner's repair keeps mounts the player or another tool added (DMM), from
    // Sigil's newest copy; on an error it falls back to adding Sigil's own lines only.
    if let Some(game_root) = st.game_root.clone() {
        match deadlock_core::game_apply::apply_gameinfo_repair(&game_root, true) {
            Ok(done) => {
                st.gameinfo.state = Gameinfo::Loads;
                crate::data::rescan_game(st);
                let notice = match done {
                    Some(done) => {
                        let mut notice = Notice::success("Fixed. Mods load again");
                        if !done.restored.is_empty() {
                            notice = notice.detail(crate::screens::settings::i18n::fill(
                                crate::screens::settings::i18n::tr("Kept {} lines another tool had added."),
                                &[&done.restored.len().to_string()],
                            ));
                        }
                        let backup = done.backup;
                        notice.undo(move |st: &mut State| undo_fix(st, backup))
                    }
                    None => Notice::success("Already fixed. Nothing changed"),
                };
                notify::push(st, notice);
                return;
            }
            Err(error) => crate::screens::settings::log::error(format!("gameinfo.gi repair, keeping mounts: {error}")),
        }
    }
    match deadlock_core::deploy::repair_gameinfo(&path, true) {
        Ok(repair) => {
            st.gameinfo.state = Gameinfo::Loads;
            crate::data::rescan_game(st);
            let mut notice = Notice::success(if repair.changed {
                "Fixed. Mods load again"
            } else {
                "Already fixed. Nothing changed"
            });
            if let Some(backup) = repair.backup {
                notice = notice.undo(move |st: &mut State| undo_fix(st, backup));
            }
            notify::push(st, notice);
        }
        Err(error) => notify::push(
            st,
            Notice::error("Could not fix gameinfo.gi")
                .detail(error.to_string())
                .action("Details", details),
        ),
    }
}

/// GAME WRITE, from the Undo button: put back the gameinfo.gi the fix copied aside.
fn undo_fix(st: &mut State, backup: PathBuf) {
    match deadlock_core::backups::restore(&backup, true) {
        Ok(_) => {
            crate::data::rescan_game(st);
            notify::push(st, Notice::info("Put gameinfo.gi back as it was"));
        }
        Err(error) => notify::push(
            st,
            Notice::error("Could not undo the fix").detail(error.to_string()),
        ),
    }
}

/// Settings, on its Game page.
fn details(st: &mut State) {
    st.settings.tab = crate::screens::settings::Tab::Game;
    st.go(Screen::Settings);
}

// --- view -----------------------------------------------------------------------------------

/// The strip under the titlebar. None while gameinfo.gi is fine or unknown.
pub fn view(s: &State, u: &Ui) -> Option<El> {
    let (line, can_fix) = match &s.gameinfo.state {
        Gameinfo::NotListed { .. } => (
            "Mods are off. A game update or a Steam file check reset gameinfo.gi, so the game skips your mods.",
            true,
        ),
        Gameinfo::Missing { .. } => (
            "Mods are off. gameinfo.gi is missing. Verify the game files in Steam, then come back.",
            false,
        ),
        Gameinfo::Incomplete { .. } => (
            "gameinfo.gi is missing lines a modded game needs. Without them Deadlock can crash at launch.",
            true,
        ),
        Gameinfo::NoGame => ("Deadlock wasn't found. Choose its folder to manage your mods.", false),
        Gameinfo::Unknown | Gameinfo::Loads => return None,
    };
    // Hidden with its X until the next launch (Grimoire); a different problem still shows.
    if crate::shell_w4::gameinfo_hidden(s, line) {
        return None;
    }
    let details_label = if s.gameinfo.state == Gameinfo::NoGame { "Choose folder" } else { "Details" };
    let mut bar = row()
        .key("gameinfo-banner")
        .role("alert")
        .label(line)
        .w_full()
        .h(48.0)
        .px(PAGE_X)
        .gap(12.0)
        .center_y()
        .fill(u.amber.alpha(0.07))
        .border_sides([1.0, 0.0, 1.0, 0.0], u.amber.alpha(0.22))
        .child(
            rune::icon(&icons::TRIANGLE_ALERT, 16.0)
                .color(u.amber)
                .a11y_hidden(),
        )
        .child(
            text(line)
                .font(BODY)
                .color(u.ink)
                .grow(1.0)
                .shrink()
                .min_w(0.0)
                .truncate()
                .a11y_hidden(),
        )
        .child(
            u.action(details_label, ActionKind::Ghost)
                .key("gameinfo-details")
                .cursor(rune::Cursor::Pointer)
                .on_press(details),
        );
    if can_fix {
        bar = bar.child(
            u.action("Fix", ActionKind::Secondary)
                .key("gameinfo-fix")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| st.gameinfo.confirm = true),
        );
    }
    Some(bar.child(crate::shell_w4::gameinfo_hide_button(u, line.to_string())))
}

/// The Fix confirm, mounted with the other modal layers.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    if !s.gameinfo.confirm {
        return None;
    }
    let close = |st: &mut State| st.gameinfo.confirm = false;
    let body = col()
        .w_full()
        .gap(8.0)
        .child(text("Fix gameinfo.gi").font(H1).color(u.ink))
        .child(
            text(match &s.gameinfo.state {
                Gameinfo::Incomplete { missing, .. } => format!(
                    "Sigil adds the lines the game needs: {}. The current file is copied aside first, so you can undo it.",
                    missing.join(", ")
                ),
                _ => "Sigil adds back the one line that tells Deadlock to load mods. The current file is copied aside first, so you can undo it.".to_string(),
            })
                .font(BODY)
                .color(u.muted)
                .wrap(),
        )
        .child(rune::el().h(12.0))
        .child(
            row()
                .w_full()
                .gap(8.0)
                .child(spacer())
                .child(
                    u.action("Cancel", ActionKind::Ghost)
                        .key("gameinfo-cancel")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(close),
                )
                .child(
                    u.icon_action(&icons::WRENCH, "Fix", ActionKind::Primary)
                        .key("gameinfo-confirm")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(fix),
                ),
        );
    Some(
        u.dialog_shell()
            .key("gameinfo-dialog")
            .role("dialog")
            .label("Fix gameinfo.gi")
            .w(440.0)
            .padding(24.0)
            .modal(true, close)
            .on_key(|st: &mut State, ev: &KeyEvent, _tx: &mut Tx| {
                if ev.key == Key::Escape {
                    st.gameinfo.confirm = false;
                    true
                } else {
                    false
                }
            })
            .child(body),
    )
}

/// `gameinfo-banner`: the banner over Installed, as after a game update.
pub fn scene_banner(s: &mut State) {
    crate::app::scene_demo_installed(s);
    s.gameinfo.state = Gameinfo::NotListed {
        path: PathBuf::from("game/citadel/gameinfo.gi"),
    };
}

/// `gameinfo-fix`: the banner's confirm.
pub fn scene_fix(s: &mut State) {
    scene_banner(s);
    s.gameinfo.confirm = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(present: bool, listed: bool) -> GameInfoStatus {
        GameInfoStatus {
            present,
            addons_search_path: listed,
            line: listed.then_some(40),
            missing_paths: Vec::new(),
        }
    }

    #[test]
    fn missing_mod_or_language_lines_offer_a_fix() {
        let mut status = status(true, true);
        status.missing_paths = vec!["Mod citadel".into(), "Write core".into()];
        let path = PathBuf::from("gameinfo.gi");
        match state_for(path.clone(), &status) {
            Gameinfo::Incomplete { path: p, missing } => {
                assert_eq!(p, path);
                assert_eq!(missing.len(), 2);
            }
            other => panic!("expected Incomplete, got {other:?}"),
        }
    }

    #[test]
    fn the_banner_shows_only_when_mods_are_off() {
        let path = PathBuf::from("gameinfo.gi");
        assert_eq!(state_for(path.clone(), &status(true, true)), Gameinfo::Loads);
        assert_eq!(
            state_for(path.clone(), &status(true, false)),
            Gameinfo::NotListed { path: path.clone() }
        );
        assert_eq!(
            state_for(path.clone(), &status(false, false)),
            Gameinfo::Missing { path }
        );
        assert_eq!(Banner::default().state, Gameinfo::Unknown);
    }
}
