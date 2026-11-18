//! A mod's backups (C10): every install backup of one slot, newest first, with when it was taken
//! and what it held.
//!
//! - Deadlock Mod Manager: keeps a backup of each mod an update replaces and lists them by name.
//! - Grimoire: keeps no backup per mod.
//! - Sigil: the card's More actions > Backups lists `deadlock_core::backups::for_slot` ("Was X,
//!   replaced by Y on date", size). Put this version back runs the existing update undo
//!   (`trash::undo_update` with `approve`, through `installed::apply_undo`) after an inline
//!   confirm; Delete removes the backup from Sigil's data folder (`backups::delete`), never a
//!   game file.

use deadlock_core::backups::{self, SlotBackup};
use rune::{El, Tx, col, row, spacer, text};

use crate::app::State;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{ActionKind, BODY, BODY_MED, H1, SMALL, Ui};

pub struct Dialog {
    pub name: String,
    pub list: Vec<SlotBackup>,
    /// The row waiting for its inline confirm: (index, restore or delete).
    pub confirm: Option<(usize, bool)>,
    pub error: Option<String>,
}

/// Open the list for one slot. The journal read is small and runs on the press.
pub fn open(st: &mut State, slot: u16) {
    let name = st
        .addons
        .addons
        .iter()
        .find(|a| a.slot == slot)
        .map(|a| super::display_name(st, a))
        .unwrap_or_else(|| deadlock_core::addons::slot_label(slot));
    let list = if st.demo || rune::is_headless() {
        Vec::new()
    } else {
        backups::for_slot(&crate::data::workspace_dir(), slot)
    };
    st.installed.menu = None;
    st.installed.backups = Some(Dialog { name, list, confirm: None, error: None });
}

fn close(st: &mut State) {
    st.installed.backups = None;
}

/// "2026-11-18 14:05" from an RFC 3339 stamp.
pub fn when(stamp: &str) -> String {
    let date = stamp.get(..10).unwrap_or(stamp);
    match stamp.get(11..16) {
        Some(time) if stamp.as_bytes().get(10) == Some(&b'T') => format!("{date} {time}"),
        _ => date.to_string(),
    }
}

fn restore(st: &mut State, index: usize) {
    let Some(dialog) = st.installed.backups.as_ref() else { return };
    let Some(backup) = dialog.list.get(index) else { return };
    let undo = super::Undo::Update { slot: backup.slot, backups: backup.files.clone(), meta: None };
    st.installed.backups = None;
    if st.demo {
        st.status = tr("Demo library: nothing was changed.").to_string();
        return;
    }
    super::apply_undo(st, undo);
}

fn delete(st: &mut State, index: usize) {
    let workspace = crate::data::workspace_dir();
    let Some(dialog) = st.installed.backups.as_mut() else { return };
    let Some(backup) = dialog.list.get(index).cloned() else { return };
    dialog.confirm = None;
    match backups::delete(&backup.folder, Some(&workspace)) {
        Ok(()) => {
            dialog.list.remove(index);
            dialog.error = None;
        }
        Err(error) => dialog.error = Some(error.to_string()),
    }
}

pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    let d = s.installed.backups.as_ref()?;
    let mut rows = col().key("backups-rows").w_full().gap(8.0);
    if d.list.is_empty() {
        rows = rows.child(
            text(tr("No backups of this mod. Sigil keeps one when an install or update replaces its files."))
                .font(BODY)
                .color(u.muted)
                .wrap(),
        );
    }
    for (index, backup) in d.list.iter().enumerate() {
        let title = match &backup.previous_label {
            Some(label) if !label.trim().is_empty() => fill(tr("Was {}"), &[label]),
            _ => tr("A file Sigil did not install").to_string(),
        };
        let line = fill(
            tr("Replaced by {} on {}. {}."),
            &[&backup.replaced_by, &when(&backup.taken_at), &crate::model::format_size(backup.bytes)],
        );
        let mut actions = row().gap(6.0).center_y();
        match d.confirm {
            Some((at, restoring)) if at == index => {
                actions = actions
                    .child(text(if restoring { tr("Put it back?") } else { tr("Delete it?") }).font(SMALL).color(u.muted))
                    .child(
                        u.action(tr("Cancel"), ActionKind::Ghost)
                            .key(format!("backup-{index}-no"))
                            .cursor(rune::Cursor::Pointer)
                            .on_press(|st: &mut State| {
                                if let Some(d) = st.installed.backups.as_mut() {
                                    d.confirm = None;
                                }
                            }),
                    )
                    .child(
                        u.action(if restoring { tr("Put it back") } else { tr("Delete") }, if restoring { ActionKind::Primary } else { ActionKind::Danger })
                            .key(format!("backup-{index}-yes"))
                            .cursor(rune::Cursor::Pointer)
                            .on_press(move |st: &mut State| if restoring { restore(st, index) } else { delete(st, index) }),
                    );
            }
            _ => {
                let folder = backup.folder.clone();
                actions = actions
                    .child(
                        u.action(tr("Show in folder"), ActionKind::Ghost)
                            .key(format!("backup-{index}-reveal"))
                            .cursor(rune::Cursor::Pointer)
                            .on_press(move |st: &mut State| super::reveal(&mut st.status, folder.clone())),
                    )
                    .child(
                        u.action(tr("Delete"), ActionKind::Ghost)
                            .key(format!("backup-{index}-delete"))
                            .cursor(rune::Cursor::Pointer)
                            .on_press(move |st: &mut State| {
                                if let Some(d) = st.installed.backups.as_mut() {
                                    d.confirm = Some((index, false));
                                }
                            }),
                    )
                    .child(
                        u.action(tr("Put this version back"), ActionKind::Secondary)
                            .key(format!("backup-{index}-restore"))
                            .cursor(rune::Cursor::Pointer)
                            .on_press(move |st: &mut State| {
                                if let Some(d) = st.installed.backups.as_mut() {
                                    d.confirm = Some((index, true));
                                }
                            }),
                    );
            }
        }
        rows = rows.child(
            col()
                .key(format!("backup-{index}"))
                .w_full()
                .gap(2.0)
                .padding(12.0)
                .radius(crate::ui::r(8.0))
                .fill(u.well)
                .border(1.0, u.hair)
                .child(text(title).font(BODY_MED).color(u.ink).wrap())
                .child(text(line).font(SMALL).color(u.muted).wrap())
                .child(row().w_full().pt(8.0).child(spacer()).child(actions)),
        );
    }
    let mut body = u
        .dialog_shell()
        .key("installed-backups")
        .role("dialog")
        .label(tr("Backups"))
        .w(640.0)
        .padding(24.0)
        .gap(14.0)
        .modal(true, close)
        .child(text(fill(tr("Backups of {}"), &[&d.name])).font(H1).color(u.ink).wrap())
        .child(
            text(tr("What an install or update replaced in this slot, newest first. Putting one back moves the current files to the trash."))
                .font(BODY)
                .color(u.muted)
                .wrap(),
        )
        .child(col().w_full().max_h(380.0).scroll(rune::ScrollSpec { stick_bottom: false }).child(rows));
    if let Some(error) = &d.error {
        body = body.child(crate::notify::feedback::error_box(u, "backups-error", error));
    }
    Some(
        body.child(
            row().w_full().child(spacer()).child(
                u.action(tr("Close"), ActionKind::Primary)
                    .key("backups-close")
                    .cursor(rune::Cursor::Pointer)
                    .on_press_tx(|st: &mut State, _tx: &mut Tx| close(st)),
            ),
        ),
    )
}

/// The scene: slot 03 with two backups, the second asking before it is put back.
pub fn stage_scene(st: &mut State) {
    use std::path::PathBuf;
    let item = |label: Option<&str>, by: &str, at: &str, bytes: u64| SlotBackup {
        slot: 3,
        folder: PathBuf::from("backups/x"),
        files: vec![PathBuf::from("backups/x/pak03_dir.vpk")],
        bytes,
        taken_at: at.to_string(),
        previous_label: label.map(str::to_string),
        replaced_by: by.to_string(),
        plan_id: String::new(),
    };
    st.installed.backups = Some(Dialog {
        name: "Random Respawn Dances".to_string(),
        list: vec![
            item(Some("Random Respawn Dances 1.2"), "Random Respawn Dances 1.3", "2026-11-17T21:04:11Z", 48_400_000),
            item(Some("Random Respawn Dances 1.1"), "Random Respawn Dances 1.2", "2026-10-02T09:15:40Z", 47_900_000),
            item(None, "Random Respawn Dances 1.1", "2026-08-30T18:22:03Z", 12_300_000),
        ],
        confirm: Some((1, true)),
        error: None,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamps_read_as_date_and_minute() {
        assert_eq!(when("2026-11-18T14:05:33.120Z"), "2026-11-18 14:05");
        assert_eq!(when("2026-11-18"), "2026-11-18");
        assert_eq!(when("bad"), "bad");
    }
}
