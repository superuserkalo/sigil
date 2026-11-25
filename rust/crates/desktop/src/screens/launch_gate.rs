//! The gate every launch passes, modded or vanilla, from the sidebar.
//!
//! When the player presses Launch, `gameinfo.gi` is read fresh from disk
//! (`deadlock_core::deploy::launch_blocker`). A file that would crash the game (mod folders
//! mounted without `Mod citadel` / `Write citadel`, the "Unable to read default keybinding
//! configuration" crash since build 10973), a modded launch with no addons line, or a missing
//! `gameinfo.gi` stops the launch and asks first. The crash case has no "launch anyway": Fix and
//! launch repairs the file (backed up, Undo in Launch's toast) and carries on in the same press.

use rune::{El, Key, KeyEvent, row, spacer, text};

use crate::app::State;
use crate::screens::settings::i18n::tr;
use crate::ui::{ActionKind, BODY, PAGE_TITLE, Ui};
use deadlock_core::deploy::LaunchBlocker;

/// A launch waiting for the player's answer.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Pending {
    pub vanilla: bool,
    pub page: bool,
    pub blocker: LaunchBlocker,
}

/// True when the launch must wait: the gate dialog is now open. Demo and headless runs pass.
pub fn blocked(st: &mut State, vanilla: bool, page: bool) -> bool {
    if st.demo || rune::is_headless() {
        return false;
    }
    let Some(game_root) = st.game_root.clone() else {
        return false;
    };
    match deadlock_core::deploy::launch_blocker(&game_root, vanilla) {
        Some(blocker) => {
            st.launch.gate = Some(Pending { vanilla, page, blocker });
            true
        }
        None => false,
    }
}

/// Fix the file, check again, and launch when it is right. The press is the approval.
fn fix_and_launch(st: &mut State) {
    let Some(pending) = st.launch.gate else { return };
    st.launch.gate = None;
    crate::screens::launch::fix_gameinfo(st);
    if let Some(game_root) = st.game_root.clone() {
        // Belt and braces: add any Mod and Write line the repair left out.
        let _ = deadlock_core::deploy::ensure_launch_search_paths(&game_root);
        if let Some(still) = deadlock_core::deploy::launch_blocker(&game_root, pending.vanilla) {
            st.launch.gate = Some(Pending { blocker: still, ..pending });
            crate::notify::push(
                st,
                crate::notify::Notice::error(tr("gameinfo.gi could not be fixed"))
                    .detail(tr("Verify the game files in Steam, then try again.")),
            );
            return;
        }
    }
    crate::banner::recheck(st);
    crate::screens::launch_progress::begin(st, pending.vanilla, pending.page);
}

fn verify_in_steam(st: &mut State) {
    st.launch.gate = None;
    let target = "steam://validate/1422450";
    let spawn = |program: &str, args: &[&str]| deadlock_core::util::hidden_command(program).args(args).spawn().map(|_| ());
    #[cfg(target_os = "macos")]
    let opened = spawn("open", &[target]);
    #[cfg(target_os = "windows")]
    let opened = spawn("cmd", &["/C", "start", "", target]);
    #[cfg(all(unix, not(target_os = "macos")))]
    let opened = spawn("xdg-open", &[target]);
    if let Err(error) = opened {
        st.status = format!("Steam could not be opened: {error}");
    }
}

/// The dialog, drawn over every page while a launch waits.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    let pending = s.launch.gate?;
    let close = |st: &mut State| st.launch.gate = None;
    let (title, body) = match pending.blocker {
        LaunchBlocker::WillCrash => (
            tr("Deadlock will not start like this"),
            tr("gameinfo.gi mounts your mods but is missing the Mod and Write lines the game now needs, so Deadlock would close at start. Sigil can add them (the old file is kept as a backup)."),
        ),
        LaunchBlocker::ModsWontLoad => (
            tr("Your mods will not load"),
            tr("gameinfo.gi lost the addons line, usually after a game update or a file check in Steam. Sigil can put it back (the old file is kept as a backup)."),
        ),
        LaunchBlocker::GameinfoMissing => (
            tr("gameinfo.gi is missing"),
            tr("Deadlock needs this file to start. Let Steam restore it with Verify integrity of game files, then launch again."),
        ),
    };
    let mut buttons = row().w_full().gap(8.0).child(spacer()).child(
        u.action(tr("Cancel"), ActionKind::Ghost)
            .key("launch-gate-cancel")
            .cursor(rune::Cursor::Pointer)
            .on_press(close),
    );
    match pending.blocker {
        LaunchBlocker::GameinfoMissing => {
            buttons = buttons.child(
                u.action(tr("Verify in Steam"), ActionKind::Primary)
                    .key("launch-gate-verify")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(verify_in_steam),
            );
        }
        LaunchBlocker::ModsWontLoad => {
            buttons = buttons
                .child(
                    u.action(tr("Launch without mods"), ActionKind::Secondary)
                        .key("launch-gate-vanilla")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| {
                            st.launch.gate = None;
                            crate::screens::launch_progress::begin(st, true, pending.page);
                        }),
                )
                .child(
                    u.action(tr("Fix and launch"), ActionKind::Primary)
                        .key("launch-gate-fix")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(fix_and_launch),
                );
        }
        LaunchBlocker::WillCrash => {
            buttons = buttons.child(
                u.action(tr("Fix and launch"), ActionKind::Primary)
                    .key("launch-gate-fix")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(fix_and_launch),
            );
        }
    }
    Some(
        u.dialog_shell()
            .key("launch-gate")
            .role("alertdialog")
            .label(title)
            .w(480.0)
            .padding(24.0)
            .gap(16.0)
            .modal(true, close)
            .on_key(move |st: &mut State, ev: &KeyEvent, _| {
                if ev.key == Key::Escape {
                    st.launch.gate = None;
                    true
                } else {
                    false
                }
            })
            .child(text(title).font(PAGE_TITLE).color(u.ink).wrap())
            .child(text(body).font(BODY).color(u.prose).max_w(432.0).wrap())
            .child(buttons),
    )
}

/// `launch-gate`: the stop before a launch that would crash.
pub fn scene(st: &mut State) {
    crate::screens::launch::scene_checklist(st);
    st.launch.dialog = crate::screens::launch::Dialog::None;
    st.launch.gate = Some(Pending {
        vanilla: false,
        page: false,
        blocker: LaunchBlocker::WillCrash,
    });
}
