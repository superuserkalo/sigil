//! How the page shows the library, and how several files of one mod become one card.
//!
//! Three looks: image cards (the default), smaller cards that fit more, and a list of rows with
//! one line each. A row keeps the same switch, chips and menu as a card, so nothing is only
//! available in one look.
//!
//! Variant groups: a mod with several files (an optional recolor, a 4K and a 2K texture set)
//! ships as several slots. Cards that share a group show as one card whose switch picks one
//! file, and its picker lists every file with its own switch, label and delete. GameBanana mods
//! group by their mod id on their own; the player can join other mods with
//! [`super::notes::Dialog`]'s group field, or take a card out of its group here. Only one file
//! of a group is on at a time, so the game never loads two variants.

use rune::{Color, El, TextEdit, Tx, col, el, icon, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::model::AddonRow;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, MICRO, SMALL, Ui};

use super::notes;
use super::{meta_for, open_menu, Menu};

/// The three looks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
    /// Image cards, four across.
    #[default]
    Cards,
    /// Smaller image cards, five across.
    Compact,
    /// One row per mod.
    List,
}

impl Layout {
    pub const ALL: [Layout; 3] = [Layout::Cards, Layout::Compact, Layout::List];

    pub fn label(self) -> &'static str {
        match self {
            Layout::Cards => "Cards",
            Layout::Compact => "Small cards",
            Layout::List => "List",
        }
    }

    /// Card columns for the two card looks.
    pub fn columns(self) -> usize {
        match self {
            Layout::Cards | Layout::List => 4,
            Layout::Compact => 5,
        }
    }

    pub fn is_cards(self) -> bool {
        !matches!(self, Layout::List)
    }
}

/// One card on the page: a single mod, or every file of a variant group.
pub struct Group<'a> {
    /// The group key, `None` for a mod of its own.
    pub key: Option<String>,
    pub addons: Vec<&'a AddonRow>,
}

impl<'a> Group<'a> {
    /// The file the card speaks for: the first one that is on, else the first.
    pub fn primary(&self) -> &'a AddonRow {
        self.addons
            .iter()
            .copied()
            .find(|addon| addon.enabled)
            .unwrap_or(self.addons[0])
    }

    pub fn enabled(&self) -> bool {
        self.addons.iter().any(|addon| addon.enabled)
    }

    pub fn is_variants(&self) -> bool {
        self.addons.len() > 1
    }
}

/// Bundle the shown mods into cards, keeping the order they are already in. A group's first
/// appearance decides where its card sits.
pub fn groups<'a>(st: &State, shown: &[&'a AddonRow]) -> Vec<Group<'a>> {
    let mut cards: Vec<Group<'a>> = Vec::new();
    for addon in shown {
        let extra = notes::extra(st, addon);
        let id = meta_for(st, addon).and_then(|meta| meta.gamebanana_id);
        let key = deadlock_core::mod_extras::variant_key(Some(&extra), id);
        match key {
            Some(key) => match cards.iter_mut().find(|card| card.key.as_deref() == Some(&key)) {
                Some(card) => card.addons.push(addon),
                None => cards.push(Group {
                    key: Some(key),
                    addons: vec![addon],
                }),
            },
            None => cards.push(Group {
                key: None,
                addons: vec![addon],
            }),
        }
    }
    cards
}

/// The variant picker's state.
#[derive(Default)]
pub struct Variants {
    /// The open picker: the group key it lists.
    pub open: Option<String>,
    /// The label being renamed, by slot, and the text typed.
    pub renaming: Option<u8>,
    pub text: TextEdit,
}

/// Open the picker for a group.
pub fn open_picker(st: &mut State, key: Option<String>) {
    st.installed.variants.open = key;
    st.installed.variants.renaming = None;
    st.installed.variants.text = TextEdit::new("");
}

/// The group a key names, from the slots on disk.
pub fn group_for<'a>(st: &'a State, key: &str) -> Vec<&'a AddonRow> {
    st.addons
        .addons
        .iter()
        .filter(|addon| {
            let extra = notes::extra(st, addon);
            let id = meta_for(st, addon).and_then(|meta| meta.gamebanana_id);
            deadlock_core::mod_extras::variant_key(Some(&extra), id).as_deref() == Some(key)
        })
        .collect()
}

/// Turn one file of a group on and the others off. One game write per slot, through
/// `set_many`; the whole switch is one Undo.
pub fn enable_variant(st: &mut State, members: &[u8], slot: u8) {
    let mut wanted = std::collections::BTreeMap::new();
    for member in members {
        wanted.insert(*member, *member == slot);
    }
    set_group(st, wanted, format!("Using the file in slot {slot:02}."));
}

/// The card's switch for a group: on enables the file it shows, off turns them all off.
pub fn group_switch(st: &mut State, members: &[u8], enable: bool, chosen: u8) {
    if enable {
        enable_variant(st, members, chosen);
        return;
    }
    let wanted: std::collections::BTreeMap<u8, bool> =
        members.iter().map(|slot| (*slot, false)).collect();
    set_group(st, wanted, "Turned the mod off.".to_string());
}

/// Apply a wanted set of slot switches, keep one Undo for all of them, and rescan.
fn set_group(st: &mut State, wanted: std::collections::BTreeMap<u8, bool>, line: String) {
    let Some(game_root) = super::writable_game(st) else { return };
    super::apply_switch(st, &game_root, &wanted, line);
}

/// Take one file out of its group, so it gets its own card again.
pub fn split_variant(st: &mut State, slot: u8) {
    if !st.addons.addons.iter().any(|addon| addon.slot == slot) {
        return;
    }
    // A group name of its own that only this slot has keeps the card alone.
    notes::edit_slot(st, slot, |entry| {
        entry.variant_group = Some(format!("alone-{}", entry.uid));
    });
    st.installed.variants.open = None;
    st.status = format!("Slot {slot:02} is its own card again.");
}

/// Rename one file in the picker.
pub fn rename_variant(st: &mut State, slot: u8) {
    let label = st.installed.variants.text.text.trim().to_string();
    if label.is_empty() {
        return;
    }
    if !st.addons.addons.iter().any(|addon| addon.slot == slot) {
        return;
    }
    notes::edit_slot(st, slot, |entry| entry.variant_label = Some(label));
    st.installed.variants.renaming = None;
    st.installed.variants.text = TextEdit::new("");
}

/// The picker dialog for a group.
pub fn picker_dialog(s: &State, u: &Ui) -> Option<El> {
    let key = s.installed.variants.open.as_ref()?;
    let members = group_for(s, key);
    if members.is_empty() {
        return None;
    }
    let name = meta_for(s, members[0])
        .map(|meta| meta.name.clone())
        .unwrap_or_else(|| "This mod".to_string());
    let mut list = col().w_full().gap(4.0);
    for addon in &members {
        let label = notes::label_for(s, addon);
        let on = addon.enabled;
        let slot = addon.slot;
        let slots: Vec<u8> = members.iter().map(|member| member.slot).collect();
        let renaming = s.installed.variants.renaming == Some(slot);
        let mut line = row()
            .key(format!("variant-{slot:02}"))
            .w_full()
            .h(52.0)
            .gap(10.0)
            .center_y()
            .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
            .child(
                u.toggle(on)
                    .key(format!("variant-switch-{slot:02}"))
                    .label(format!("{} {label}", if on { "Turn off" } else { "Use" }))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| enable_variant(st, &slots, slot)),
            );
        if renaming {
            line = line
                .child(
                    rune::text_input(&s.installed.variants.text, BODY)
                        .key("variant-label")
                        .single_line()
                        .label("File name")
                        .grow(1.0)
                        .min_w(0.0)
                        .color(u.ink)
                        .on_edit(rune::edit_access(|st: &mut State| {
                            &mut st.installed.variants.text
                        }))
                        .on_submit(move |st: &mut State, _tx: &mut Tx| rename_variant(st, slot)),
                )
                .child(
                    u.action("Save", ActionKind::Primary)
                        .key(format!("variant-save-{slot:02}"))
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| rename_variant(st, slot)),
                );
        } else {
            line = line.child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(2.0)
                    .child(text(label).font(BODY_MED).color(u.ink).truncate())
                    .child(
                        text(format!(
                            "Slot {slot:02}  ·  {}",
                            crate::model::format_size(addon.size_bytes)
                        ))
                        .font(SMALL)
                        .color(u.faint)
                        .truncate(),
                    ),
            );
            line = line.child(
                u.icon_button(&icons::SQUARE_PEN, "Rename this file")
                    .key(format!("variant-rename-{slot:02}"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        st.installed.variants.renaming = Some(slot);
                        let label = st
                            .addons
                            .addons
                            .iter()
                            .find(|addon| addon.slot == slot)
                            .map(|addon| notes::label_for(st, addon))
                            .unwrap_or_default();
                        st.installed.variants.text = TextEdit::new(&label);
                    }),
            );
            line = line.child(
                u.icon_button(&icons::SQUARE_CHECK, "Give this file its own card")
                    .key(format!("variant-split-{slot:02}"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| split_variant(st, slot)),
            );
            line = line.child(
                u.icon_button(&icons::TRASH_2, "Delete this file")
                    .key(format!("variant-delete-{slot:02}"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        super::ask(st, super::Confirm::Delete(vec![slot]))
                    }),
            );
        }
        list = list.child(line);
    }
    let close = |st: &mut State| {
        st.installed.variants.open = None;
        st.installed.variants.renaming = None;
    };
    Some(
        u.dialog_shell()
            .key("installed-variants")
            .role("dialog")
            .label(format!("Files of {name}"))
            .w(520.0)
            .padding(24.0)
            .gap(16.0)
            .modal(true, close)
            .child(text(name).font(H1).color(u.ink).wrap())
            .child(
                text("One file of this mod is on at a time. Pick the one you want.")
                    .font(BODY)
                    .color(u.muted)
                    .wrap(),
            )
            .child(list)
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .child(spacer())
                    .child(
                        u.action("Done", ActionKind::Secondary)
                            .key("variants-done")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(close),
                    ),
            ),
    )
}

// --- list rows -----------------------------------------------------------------

/// One mod as a list row: a 64 by 36 preview, the name and detail, chips, the switch and the
/// card's own menu. Used by the list look in place of [`super::card`].
pub fn list_row(s: &State, u: &Ui, card: &Group) -> El {
    let addon = card.primary();
    let slot = addon.slot;
    let name = meta_for(s, addon)
        .map(|meta| meta.name.clone())
        .unwrap_or_else(|| "Unnamed mod".to_string());
    let mut detail = match meta_for(s, addon).and_then(|meta| meta.author.clone()) {
        Some(author) => format!("by {author}"),
        None => super::slot_file_name(addon),
    };
    if card.is_variants() {
        detail = format!("{} files  ·  {detail}", card.addons.len());
    }
    let key = notes::preview_key(s, addon);
    let pixels = key.as_ref().and_then(|key| s.thumbnails.get(key));
    let thumb = match pixels {
        Some(pixels) => el()
            .size(64.0, 36.0)
            .corners([6.0, 6.0, 6.0, 6.0])
            .clip_corners([6.0, 6.0, 6.0, 6.0])
            .pixels_fit(pixels, rune::SurfaceFit::Fill),
        None => el()
            .size(64.0, 36.0)
            .radius(crate::ui::r(6.0))
            .fill(u.hover)
            .center()
            .child(icon(&icons::PACKAGE, 16.0).color(u.faint).a11y_hidden()),
    };
    let selected = s.installed.selected.contains(&slot);
    let menu_open = s.installed.menu == Some(Menu::Card(slot));
    let enable = !addon.enabled;
    let mut chips = row().gap(6.0).center_y();
    for chip in notes::card_chips(s, u, addon) {
        chips = chips.child(chip);
    }
    let mut line = row()
        .key(format!("addon-row-{slot:02}"))
        .w_full()
        .h(52.0)
        .gap(12.0)
        .center_y()
        .px(8.0)
        .radius(crate::ui::r(8.0))
        .fill(if selected { u.selected } else { Color::TRANSPARENT })
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .hover(u.hover)
        .child(
            el()
                .size(18.0, 18.0)
                .center()
                .opacity(if selected || s.installed.hovered == Some(slot) { 1.0 } else { 0.0 })
                .focusable(true)
                .role("checkbox")
                .checked(selected)
                .label(format!("Select {name}"))
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                    super::pick(st, slot, tx.mods().shift)
                })
                .child(super::check_box(u, selected, false)),
        )
        .child(thumb)
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(
                    text(name.clone())
                        .font(BODY_MED)
                        .color(if addon.enabled { u.ink } else { u.prose })
                        .truncate(),
                )
                .child(text(detail).font(SMALL).color(u.faint).truncate()),
        )
        .child(chips)
        .child(
            text(format!("{slot:02}")).font(MICRO).color(u.faint).w(20.0),
        );
    if card.is_variants() {
        let group_key = card.key.clone();
        line = line.child(
            u.icon_button(&icons::LAYERS, "Choose the file")
                .key(format!("row-variants-{slot:02}"))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| open_picker(st, group_key.clone())),
        );
    }
    let group_key = card.key.clone();
    line = line
        .child(
            u.icon_button(&icons::ELLIPSIS, &format!("More for {name}"))
                .key(format!("installed-more-{slot:02}"))
                .fill(if menu_open { u.selected } else { Color::TRANSPARENT })
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| open_menu(st, Menu::Card(slot))),
        )
        .child(
            u.toggle(addon.enabled)
                .key(format!("toggle-{slot:02}"))
                .label(format!(
                    "{} {name}",
                    if addon.enabled { "Turn off" } else { "Turn on" }
                ))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    let _ = group_key;
                    super::toggle_one(st, slot, enable)
                }),
        );
    line
}
