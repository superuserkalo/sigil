//! The Crosshair page's ties to the game's own files (Grimoire #504).
//!
//! - Import from game reads the crosshair from `citadel_hero_settings.lst` (its `base` object),
//!   where the game keeps it since crosshairs became per hero, and falls back to
//!   `machine_convars.vcfg`, where older builds kept it.
//! - Deadlock runs `autoexec.cfg` at every launch by itself now. While the Crosshair page is off
//!   nothing in Sigil shows the crosshair block Sigil wrote there, yet it would reset the
//!   crosshair the player sets in game at every start. Turning the page off takes the block out
//!   (the player's switch is the trigger, with a backup and Undo), and Settings > General > Pages
//!   offers a Remove button when a block is left from before.
//!
//! Only the block between Sigil's own markers is touched; a crosshair typed into autoexec.cfg by
//! hand stays.

use std::cell::Cell;
use std::path::PathBuf;

use deadlock_core::crosshair::Crosshair;
use rune::{El, row, text};

use crate::app::State;
use crate::model::Screen;
use crate::ui::{ActionKind, SMALL, Tone, Ui};

use super::tr;

thread_local! {
    /// Whether the install's autoexec.cfg still holds Sigil's crosshair block, read once and
    /// dropped after a removal or a page switch. UI thread only.
    static LEFTOVER: Cell<Option<bool>> = const { Cell::new(None) };
}

/// A file that may hold the crosshair, and how to read it.
type Source = (PathBuf, fn(&str) -> Option<Crosshair>);

/// Where the game keeps the crosshair, newest first: every `citadel_hero_settings.lst` (the
/// game folder's copy and each Steam account's cloud copy) by modified time, then
/// `machine_convars.vcfg`.
fn sources(st: &State) -> Vec<Source> {
    let Some(root) = st.game_root.clone() else {
        return Vec::new();
    };
    let mut hero: Vec<(std::time::SystemTime, PathBuf)> = super::hero_settings_paths(st)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|path| Some((std::fs::metadata(&path).ok()?.modified().ok()?, path)))
        .collect();
    hero.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    let mut out: Vec<Source> = hero
        .into_iter()
        .map(|(_, path)| (path, deadlock_core::crosshair::parse_hero_settings as fn(&str) -> Option<Crosshair>))
        .collect();
    out.push((
        root.join("game").join("citadel").join("cfg").join("machine_convars.vcfg"),
        deadlock_core::crosshair::parse_machine_convars,
    ));
    out
}

/// Import from game: the first file that holds a crosshair wins. Read only.
pub fn import(st: &mut State) {
    if st.game_root.is_none() {
        st.crosshair.log = Some(tr("No Deadlock install found, so there is no in-game crosshair to import.").to_string());
        st.crosshair.tone = Tone::Warn;
        return;
    }
    for (path, parse) in sources(st) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Some(crosshair) = parse(&text) {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            st.crosshair.settings = crosshair;
            st.crosshair.source = format!("in-game {name}");
            st.crosshair.log = Some(crate::screens::settings::i18n::fill(
                tr("Read the in-game crosshair from {}."),
                &[&path.display().to_string()],
            ));
            st.crosshair.tone = Tone::Ok;
            return;
        }
    }
    st.crosshair.log = Some(tr("The game has not saved a crosshair yet, so the defaults are shown.").to_string());
    st.crosshair.tone = Tone::Warn;
}

/// Whether autoexec.cfg holds Sigil's crosshair block, cached. Read only.
fn leftover(st: &State) -> bool {
    if let Some(known) = LEFTOVER.with(Cell::get) {
        return known;
    }
    let found = !st.demo
        && st
            .game_root
            .as_deref()
            .is_some_and(deadlock_core::crosshair::applied_in_install);
    LEFTOVER.with(|cell| cell.set(Some(found)));
    found
}

/// Called after the player flips the Crosshair page or Mods only in Settings > General > Pages.
/// When the page is now off and Sigil's block is in autoexec.cfg, the block comes out.
pub fn after_page_switch(st: &mut State) {
    LEFTOVER.with(|cell| cell.set(None));
    if st.prefs.page_on(Screen::Crosshair) || !leftover(st) {
        return;
    }
    remove(st);
}

/// GAME WRITE, player triggered (the Pages switch or the Remove button under it):
/// `deadlock_core::crosshair::remove_from_install` takes only Sigil's block out of autoexec.cfg,
/// copying the old file aside first. The toast's Undo puts it back.
fn remove(st: &mut State) {
    use crate::notify::{self, Notice};
    LEFTOVER.with(|cell| cell.set(None));
    let Some(root) = st.game_root.clone() else {
        return;
    };
    if st.demo || rune::is_headless() {
        return;
    }
    match deadlock_core::crosshair::remove_from_install(&root) {
        Ok(Some(result)) => {
            st.autoexec.loaded = false;
            let notice = Notice::success(tr("Crosshair removed from autoexec"))
                .detail(tr("The Crosshair page is off, so the game keeps the crosshair you set in game."));
            let notice = match result.backup {
                Some(backup) => notice.undo(move |st: &mut State| super::undo_remove(st, backup)),
                None => notice,
            };
            notify::push(st, notice);
        }
        Ok(None) => {}
        Err(error) => notify::push(
            st,
            Notice::error(tr("Could not take the crosshair out of autoexec")).detail(error.to_string()),
        ),
    }
}

/// Under the Crosshair row in Settings > General > Pages: a note and a Remove button while the
/// page is off and Sigil's block is still in autoexec.cfg. None otherwise.
pub fn pages_note(s: &State, u: &Ui) -> Option<El> {
    if s.prefs.page_on(Screen::Crosshair) || !leftover(s) {
        return None;
    }
    Some(
        row()
            .key("settings-page-crosshair-leftover")
            .w_full()
            .gap(12.0)
            .py(6.0)
            .center_y()
            .child(
                text(tr("autoexec.cfg still sets the crosshair Sigil wrote, so the game resets your in-game crosshair at every start."))
                    .font(SMALL)
                    .color(u.muted)
                    .wrap()
                    .grow(1.0)
                    .min_w(0.0),
            )
            .child(
                u.action(tr("Remove"), ActionKind::Secondary)
                    .key("settings-page-crosshair-leftover-remove")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(remove),
            ),
    )
}
