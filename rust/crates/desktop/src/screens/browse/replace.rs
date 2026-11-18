//! Replace an installed file with another upload of the same mod, from the details' file rows
//! (Grimoire's manual replacement, MIT).
//!
//! A file row of an installed mod offers Install (a new slot) and Replace (into the slot of a
//! file already installed, through Installed's update queue: the old file is backed up and on
//! or off stays). Settings > Mods > "Replace first" makes Replace the main button; "Replace
//! without asking" skips the confirm when only one installed file can be replaced. With several
//! installed files the confirm always asks which one.

use rune::{Color, El, Key, KeyEvent, Tx, col, row, spacer, text};

use super::*;
use crate::screens::settings::i18n::{fill, tr};
use crate::screens::settings::options;

/// A Replace waiting for its confirm.
#[derive(Clone, Debug, PartialEq)]
pub struct Pending {
    pub id: u64,
    pub name: String,
    /// What the download queue reads: the mod id for its newest file, else the file's link.
    pub target: String,
    pub file_id: u64,
    pub file_name: String,
    /// The installed slots that can be replaced, and the one picked.
    pub slots: Vec<u8>,
    pub chosen: u8,
    pub dont_ask: bool,
}

/// The installed slots of mod `id` that are on disk.
pub fn candidates(s: &State, id: u64) -> Vec<u8> {
    let mut slots: Vec<u8> = s
        .addons
        .addons
        .iter()
        .filter(|addon| {
            crate::screens::installed::meta_for(s, addon).and_then(|meta| meta.gamebanana_id) == Some(id)
        })
        .map(|addon| addon.slot)
        .collect();
    slots.sort_unstable();
    slots
}

/// The buttons a file row shows: Install alone, or Install and Replace for an installed mod, in
/// the order Settings chose.
pub fn file_actions(s: &State, u: &Ui, d: &Details, profile: &ModProfile, file: &ModFile) -> El {
    let install = quick::file_button(s, u, d, profile, file);
    let slots = candidates(s, d.id);
    let busy = s.browse_view.installs.items.get(&d.id).is_some_and(quick::Entry::busy);
    if slots.is_empty() || file.archived || d.section != BrowseSection::Mods {
        return install;
    }
    let first = options::swap_install_replace();
    let pending = Pending {
        id: d.id,
        name: d.name.clone(),
        target: quick::file_target(d, profile, file),
        file_id: file.id,
        file_name: file.name.clone(),
        chosen: slots[0],
        slots,
        dont_ask: false,
    };
    let mut replace = u
        .icon_action(&icons::REFRESH_CW, tr("Replace"), if first { ActionKind::Primary } else { ActionKind::Secondary })
        .key(format!("file-replace-{}", file.id))
        .label(fill(tr("Replace an installed file with {}"), &[&file.name]));
    replace = if busy {
        replace.opacity(0.45).on_press(|_st: &mut State| {})
    } else {
        replace.cursor(rune::Cursor::Pointer).on_press(move |st: &mut State| press(st, pending.clone()))
    };
    let mut r = row().gap(8.0).center_y();
    r = if first { r.child(replace).child(install) } else { r.child(install).child(replace) };
    r
}

/// Replace at once when Settings says so and the slot is clear, else ask.
pub fn press(st: &mut State, pending: Pending) {
    if !asks(options::skip_replace_confirm(), pending.slots.len()) {
        run(st, pending);
    } else {
        st.browse_view.replace = Some(pending);
    }
}

/// Whether a Replace opens its confirm: always with a choice to make, else unless Settings
/// skips it.
pub fn asks(skip_confirm: bool, slots: usize) -> bool {
    !skip_confirm || slots != 1
}

/// Download the file, then hand the slot to Installed's update queue with that archive.
fn run(st: &mut State, pending: Pending) {
    st.browse_view.replace = None;
    if pending.dont_ask {
        options::edit(st, |o| o.skip_replace_confirm = true);
    }
    st.browse_view.installs.items.remove(&pending.id);
    quick::start(st, pending.id, &pending.name, pending.target, Some(pending.file_id), vec![pending.chosen]);
}

fn close(st: &mut State) {
    st.browse_view.replace = None;
}

/// The confirm: which installed file the upload replaces.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    let p = s.browse_view.replace.as_ref()?;
    let mut list = col().w_full().border_sides([1.0, 0.0, 0.0, 0.0], u.hair);
    for slot in &p.slots {
        let slot = *slot;
        let on = p.chosen == slot;
        let (label, state) = crate::screens::installed::file_label(s, slot);
        list = list.child(
            row()
                .key(format!("replace-slot-{slot:02}"))
                .w_full()
                .min_h(44.0)
                .py(6.0)
                .gap(10.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .focusable(true)
                .role("radio")
                .checked(on)
                .label(label.clone())
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    if let Some(p) = st.browse_view.replace.as_mut() {
                        p.chosen = slot;
                    }
                })
                .child(
                    rune::el()
                        .size(16.0, 16.0)
                        .radius(8.0)
                        .center()
                        .border(1.5, if on { u.accent } else { u.hair_strong })
                        .child(rune::el().size(8.0, 8.0).radius(4.0).fill(if on { u.accent } else { Color::TRANSPARENT }))
                        .a11y_hidden(),
                )
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(text(label).font(BODY_MED).color(u.ink).truncate())
                        .child(text(state).font(SMALL).color(u.faint).truncate()),
                ),
        );
    }
    let go = p.clone();
    let dont_ask = p.dont_ask;
    let title = tr("Replace an installed file?");
    Some(
        u.dialog_shell()
            .key("browse-replace-dialog")
            .role("dialog")
            .label(title)
            .w(480.0)
            .padding(24.0)
            .gap(16.0)
            .modal(true, close)
            .on_key(|st: &mut State, ev: &KeyEvent, _tx: &mut Tx| {
                if ev.key == Key::Escape {
                    close(st);
                    true
                } else {
                    false
                }
            })
            .child(text(title).font(H1).color(u.ink))
            .child(
                text(fill(tr("{} goes into the slot of the file you pick. The old file is backed up first, and on or off stays as it is."), &[&p.file_name]))
                    .font(BODY)
                    .color(u.muted)
                    .wrap(),
            )
            .child(list)
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(
                        row()
                            .key("replace-dont-ask")
                            .h(32.0)
                            .gap(8.0)
                            .center_y()
                            .focusable(true)
                            .role("checkbox")
                            .checked(dont_ask)
                            .label(tr("Don't ask again"))
                            .cursor(rune::Cursor::Pointer)
                            .on_press(|st: &mut State| {
                                if let Some(p) = st.browse_view.replace.as_mut() {
                                    p.dont_ask = !p.dont_ask;
                                }
                            })
                            .child(crate::screens::installed::check_box_el(u, dont_ask))
                            .child(text(tr("Don't ask again")).font(BODY).color(u.muted).a11y_hidden()),
                    )
                    .child(spacer())
                    .child(
                        u.action(tr("Cancel"), ActionKind::Ghost)
                            .key("replace-cancel")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(close),
                    )
                    .child(
                        u.action(tr("Replace"), ActionKind::Primary)
                            .key("replace-confirm")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(move |st: &mut State| {
                                let mut go = go.clone();
                                if let Some(p) = st.browse_view.replace.as_ref() {
                                    go.chosen = p.chosen;
                                    go.dont_ask = p.dont_ask;
                                }
                                run(st, go);
                            }),
                    ),
            ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_replace_asks_unless_settings_say_not_to() {
        assert!(asks(false, 1), "asks by default");
        assert!(asks(true, 2), "two files to pick from still ask");
        assert!(!asks(true, 1), "one file and the switch on: no confirm");
    }
}
