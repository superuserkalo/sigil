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
//! [`super::notes::Dialog`]'s group field, or take a card out of its group here. Any set of a
//! group's files can be on at once (a model and its voice pack, two HUD parts); each loads like a
//! mod of its own, so the picker also sets their order inside the group (lower slot wins).
//! Settings > Mods > "Open the mod page for multi-file mods" makes a press on such a card open
//! its GameBanana page instead of the picker.

use rune::{Color, El, TextEdit, Tx, col, el, icon, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::screens::settings::i18n::{fill, tr};
use crate::screens::settings::options;
use crate::model::AddonRow;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, SMALL, Ui};

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
    groups_in(st, &super::index::Index::new(st), shown)
}

/// [`groups`] with the page's lookups.
pub(super) fn groups_in<'a>(st: &State, idx: &super::index::Index, shown: &[&'a AddonRow]) -> Vec<Group<'a>> {
    let mut cards: Vec<Group<'a>> = Vec::new();
    // Where each group's card is, so a group finds its card at once.
    let mut at: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for addon in shown {
        let extra = idx.extra(st, addon);
        let id = idx.meta(st, addon).and_then(|meta| meta.gamebanana_id);
        let key = deadlock_core::mod_extras::variant_key(Some(&extra), id);
        match key {
            Some(key) => match at.get(&key) {
                Some(&i) => cards[i].addons.push(addon),
                None => {
                    at.insert(key.clone(), cards.len());
                    cards.push(Group {
                        key: Some(key),
                        addons: vec![addon],
                    });
                }
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
    pub renaming: Option<u16>,
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

/// The card's switch for a group: on turns on the file it shows (through the same questions
/// as any switch), off turns every file of the group off.
pub fn group_switch(st: &mut State, members: &[u16], enable: bool, chosen: u16) {
    if enable {
        super::enable::switch(st, chosen, true);
        return;
    }
    let wanted: std::collections::BTreeMap<u16, bool> =
        members.iter().map(|slot| (*slot, false)).collect();
    set_group(st, wanted, "Turned the mod off.".to_string());
}

/// Apply a wanted set of slot switches, keep one Undo for all of them, and rescan.
fn set_group(st: &mut State, wanted: std::collections::BTreeMap<u16, bool>, line: String) {
    let Some(game_root) = super::writable_game(st) else { return };
    super::apply_switch(st, &game_root, &wanted, line);
}

/// A press on a multi-file card: its file list, or its GameBanana page when Settings asks for
/// that and the mod has one.
pub fn open_group(st: &mut State, key: Option<String>, slot: u16) {
    if options::installed_open_page()
        && let Some(id) = st
            .addons
            .addons
            .iter()
            .find(|addon| addon.slot == slot)
            .and_then(|addon| meta_for(st, addon))
            .and_then(|meta| meta.gamebanana_id)
    {
        crate::screens::browse::open_submission(st, deadlock_core::gamebanana::BrowseSection::Mods, id);
        return;
    }
    open_picker(st, key);
}

/// The new order of a group's slots with the file at `index` moved one place up or down.
/// `None` at either end.
pub fn nudge(slots: &[u16], index: usize, up: bool) -> Option<Vec<u16>> {
    let to = if up { index.checked_sub(1)? } else { index + 1 };
    if to >= slots.len() || index >= slots.len() {
        return None;
    }
    let mut order = slots.to_vec();
    order.swap(index, to);
    Some(order)
}

/// Take one file out of its group, so it gets its own card again.
pub fn split_variant(st: &mut State, slot: u16) {
    if !st.addons.addons.iter().any(|addon| addon.slot == slot) {
        return;
    }
    // A group name of its own that only this slot has keeps the card alone.
    notes::edit_slot(st, slot, |entry| {
        entry.variant_group = Some(format!("alone-{}", entry.uid));
    });
    st.installed.variants.open = None;
    st.status = format!("Slot {} is its own card again.", deadlock_core::addons::slot_label(slot));
}

/// Rename one file in the picker.
pub fn rename_variant(st: &mut State, slot: u16) {
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
        let slots: Vec<u16> = members.iter().map(|member| member.slot).collect();
        let index = slots.iter().position(|s| *s == slot).unwrap_or(0);
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
                    .label(fill(if on { tr("Turn off {}") } else { tr("Turn on {}") }, &[&label]))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| super::enable::switch(st, slot, !on)),
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
            for (up, glyph, words) in [(true, &icons::ARROW_UP, tr("Load earlier")), (false, &ARROW_DOWN, tr("Load later"))] {
                let order = nudge(&slots, index, up);
                let group = slots.clone();
                let mut b = u
                    .icon_button(glyph, words)
                    .key(format!("variant-{}-{slot:02}", if up { "up" } else { "down" }));
                b = match order {
                    Some(order) => b
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| super::ask_reorder(st, &group, &order)),
                    None => b.disabled(true).opacity(0.35),
                };
                line = line.child(b);
            }
            // A file of a skin, in 3D on its hero, on or off.
            if !crate::viewer::dialog::heroes_of(s, slot).is_empty() {
                let file_name = format!("{name}: {}", notes::label_for(s, addon));
                line = line.child(
                    u.icon_button(&icons::BOX, tr(crate::viewer::dialog::LABEL))
                        .key(format!("variant-3d-{slot:02}"))
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| crate::viewer::dialog::open(st, slot, &file_name)),
                );
            }
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
                text(tr("Turn on any of its files. Each one loads like a mod of its own, and the higher one in this list wins where two change the same file."))
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
pub fn list_row(s: &State, u: &Ui, idx: &super::index::Index, card: &Group) -> El {
    let addon = card.primary();
    let slot = addon.slot;
    let name = idx.meta(s, addon)
        .map(|meta| meta.name.clone())
        .unwrap_or_else(|| "Unnamed mod".to_string());
    let mut detail = match idx.meta(s, addon).and_then(|meta| meta.author.clone()) {
        Some(author) => format!("by {author}"),
        None => super::slot_file_name(addon),
    };
    if card.is_variants() {
        detail = format!("{} files  ·  {detail}", card.addons.len());
    }
    // When it was installed: the cards leave it out, the list keeps it.
    if let Some(date) = super::install_date(s, addon) {
        detail = format!("{detail}  ·  {date}");
    }
    let key = notes::preview_key(s, addon);
    let pixels = key.as_ref().and_then(|key| s.thumbnails.get(key));
    let thumb = match pixels {
        Some(pixels) => el()
            .size(56.0, 32.0)
            .corners([crate::ui::r(4.0); 4])
            .clip_corners([crate::ui::r(4.0); 4])
            .pixels_fit(pixels, rune::SurfaceFit::Fill),
        None => el()
            .size(56.0, 32.0)
            .radius(crate::ui::r(4.0))
            .fill(u.hover)
            .center()
            .child(icon(&icons::PACKAGE, 16.0).color(u.faint).a11y_hidden()),
    };
    let selected = s.installed.selected.contains(&slot);
    let menu_open = s.installed.menu == Some(Menu::Card(slot));
    let enable = !addon.enabled;
    // Two fixed columns on the right, so every row's chips and marks share their edges: the
    // word chips (right aligned, cut at the column's edge) and the marks.
    let wide = crate::ui::fit() == crate::ui::Fit::Full;
    // Marks: the same as the card's, warnings first, at most three.
    // The flat look: an outlined square per mark, not the dark plate the preview needs.
    let mut all: Vec<El> = super::warning_buttons(s, u, addon, false);
    all.extend(super::card_marks(s, u, addon, false));
    let mut marks = row().w(MARKS_W).gap(4.0).center_y().justify_end();
    for mark in all.into_iter().take(3) {
        marks = marks.child(mark);
    }
    // The top tag and the size, at Full width only.
    let tag = row().w(TAG_W).center_y().children(notes::top_tag(s, addon).map(|label| {
        row()
            .h(24.0)
            .px(8.0)
            .max_w(TAG_W)
            .center_y()
            .radius(crate::ui::r(4.0))
            .fill(u.over.alpha(0.06))
            .child(text(label.clone()).font(crate::ui::LABEL).color(u.prose).truncate().help_when_truncated(label))
    }));
    let size = text(crate::model::format_size(addon.size_bytes))
        .font(crate::ui::SMALL_NUM)
        .color(u.muted)
        .w(SIZE_W)
        .align(rune::Align::End);
    let dragging = s.installed.drag.is_some_and(|d| d.slot == slot);
    let drop = super::drag::drop_mark(s, slot, addon.enabled);
    let mut line = row()
        .key(format!("addon-row-{slot:02}"))
        .w_full()
        .h(ROW_H)
        .gap(12.0)
        .center_y()
        .px(8.0)
        .radius(crate::ui::r(8.0))
        .fill(if selected { u.selected } else if dragging { u.well } else { Color::TRANSPARENT })
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .hover(u.hover)
        // The grip and the box fade in on hover without a commit (`super::hover_reveal`).
        .hover_group();
    // The grip keeps its box in every sort, so rows never slide sideways.
    line = line.child(super::drag::grip(s, u, slot, addon.enabled, [0.0, ROW_H], 1, dragging, s.installed.hovered == Some(slot)));
    line = line
        .child(super::hover_reveal(
            el()
                .size(18.0, 18.0)
                .center()
                .focusable(true)
                .role("checkbox")
                .checked(selected)
                .label(format!("Select {name}"))
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                    super::pick(st, slot, tx.mods().shift)
                })
                .child(super::check_box(u, selected, false)),
            selected || s.installed.hovered == Some(slot),
        ))
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
                .child(text(detail).font(SMALL).color(u.muted).truncate()),
        )
        .child(marks)
        .children(wide.then_some(tag))
        .children(wide.then_some(size))
        .child(
            text(deadlock_core::addons::slot_label(slot))
                .font(crate::ui::SMALL_NUM)
                .color(u.muted)
                .w(28.0)
                .align(rune::Align::End),
        );
    // The variants button keeps its slot on every row, so the menu and switch line up.
    line = line.child(if card.is_variants() {
        let group_key = card.key.clone();
        u.icon_button(&icons::LAYERS, "Choose the file")
            .size(28.0, 28.0)
            .key(format!("row-variants-{slot:02}"))
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| open_group(st, group_key.clone(), slot))
    } else {
        el().size(28.0, 28.0).a11y_hidden()
    });
    let group_key = card.key.clone();
    line = line
        .child(
            u.icon_button(&icons::ELLIPSIS, &format!("More for {name}"))
                .size(28.0, 28.0)
                .key(format!("installed-more-{slot:02}"))
                .fill(if menu_open { u.selected } else { Color::TRANSPARENT })
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| open_menu(st, Menu::Card(slot))),
        )
        .child({
            let members: Vec<u16> = card.addons.iter().map(|addon| addon.slot).collect();
            let is_variants = card.is_variants();
            u.toggle(addon.enabled)
                .key(format!("toggle-{slot:02}"))
                // No focus on press: that was a second commit per toggle (`installed::card_view`).
                .keeps_text_selection()
                .label(format!(
                    "{} {name}",
                    if addon.enabled { "Turn off" } else { "Turn on" }
                ))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    let _ = &group_key;
                    if is_variants {
                        group_switch(st, &members, enable, slot)
                    } else {
                        super::enable::switch(st, slot, enable)
                    }
                })
        });
    // A drop target gets a 2 pt insertion line on its top or bottom hairline, over the row,
    // so nothing moves.
    if let Some(after) = drop {
        let mark = el().key(format!("installed-drop-line-{slot:02}")).h(2.0).fill(u.accent).a11y_hidden();
        line = line.child(if after {
            mark.pin(Some(0.0), None, Some(0.0), Some(-1.0))
        } else {
            mark.pin(Some(0.0), Some(-1.0), Some(0.0), None)
        });
    }
    super::drag::lift(line, dragging)
}

/// One row of the list look: fixed, so the virtual list steps by it.
pub const ROW_H: f32 = 48.0;
/// A list row's marks column (three 28 marks), and its tag and size columns (Full only).
const MARKS_W: f32 = 92.0;
const TAG_W: f32 = 96.0;
const SIZE_W: f32 = 64.0;

/// Lucide arrow-down (ISC), drawn like `icons.rs`.
static ARROW_DOWN: rune::Icon = rune::Icon {
    name: "sigil-variants-arrow-down",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M12 5v14"/><path d="m19 12-7 7-7-7"/></g>"#,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_moves_one_place_inside_its_group() {
        assert_eq!(nudge(&[3, 7, 9], 1, true), Some(vec![7, 3, 9]));
        assert_eq!(nudge(&[3, 7, 9], 1, false), Some(vec![3, 9, 7]));
        assert_eq!(nudge(&[3, 7, 9], 0, true), None, "the first cannot go up");
        assert_eq!(nudge(&[3, 7, 9], 2, false), None, "the last cannot go down");
        assert_eq!(nudge(&[3], 4, false), None);
    }
}
