//! What the player noted about a mod, and the filters that follow from it.
//!
//! The library store remembers a name, an author and a GameBanana id. Everything else the
//! Installed page needs lives in `deadlock_core::mod_extras` (`<data>/library-extra.json`): the
//! hero and category GameBanana gives a mod, the player's tags, a pin, "load first", an ignored
//! update, a local picture, an NSFW flag, a variant group, and the named mod lists.
//!
//! This module reads that store once per scan signature, fetches the missing GameBanana
//! metadata on a worker (a mod installed before Sigil recorded categories gets them the first
//! time the page sees it), and draws the hero, category, tag and list filters with their
//! dialogs. The store is app data only: nothing here touches the game folder.

use std::collections::BTreeSet;
use std::path::PathBuf;

use deadlock_core::gamebanana;
use deadlock_core::mod_extras::{self, Extras, ModExtra, SlotKey};
use rune::{El, FileDialogOptions, Tx, col, icon, row, spacer, text};
use rune::TextEdit;

use crate::app::State;
use crate::icons;
use crate::jobs::Job;
use crate::model::AddonRow;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, SMALL, Ui};

use super::{meta_for, NO_ICON};

/// The state of the extras store on this screen.
#[derive(Default)]
pub struct Notes {
    pub extras: Extras,
    pub loaded: bool,
    /// The slots the store was last read for.
    pub sig: Option<Vec<(u8, u64)>>,
    /// Mod pages read on a worker, to fill hero, category and NSFW.
    pub backfill: Job<Vec<ModExtra>>,
    /// GameBanana ids the backfill was started for.
    pub backfill_ids: Option<Vec<u64>>,
    pub hero: Option<String>,
    pub category: Option<String>,
    pub tag: Option<String>,
    pub list: Option<String>,
    pub dialog: Option<Dialog>,
    /// The tag line, mod name and list name typed in a dialog.
    pub text: TextEdit,
    pub nsfw: bool,
    /// A picture the player picked for a local mod.
    pub image: Option<PathBuf>,
    pub error: Option<String>,
}

/// The open editor.
pub enum Dialog {
    /// Tags of one slot.
    Tags(u8),
    /// Named lists and their members.
    Lists,
    /// Name, picture and NSFW flag of a local mod.
    EditLocal(u8),
    /// A new list with the given name.
    NewList,
}

/// The store path under the app data folder.
pub fn extras_file() -> PathBuf {
    mod_extras::extras_path(&crate::data::data_dir())
}

/// The key the store uses for a slot.
pub fn key(addon: &AddonRow) -> SlotKey {
    SlotKey {
        slot: addon.slot,
        size_bytes: addon.size_bytes,
        gamebanana_id: None,
    }
}

/// The key with the GameBanana id from the library entry, when there is one.
fn key_with_id(st: &State, addon: &AddonRow) -> SlotKey {
    SlotKey {
        gamebanana_id: meta_for(st, addon).and_then(|meta| meta.gamebanana_id),
        ..key(addon)
    }
}

/// The key of a slot, looked up from the page's rows.
fn slot_key(st: &State, slot: u8) -> Option<SlotKey> {
    st.addons
        .addons
        .iter()
        .find(|addon| addon.slot == slot)
        .map(|addon| key_with_id(st, addon))
}

/// The entry for a slot, reading the store as a blank one when it has no entry.
pub fn extra(st: &State, addon: &AddonRow) -> ModExtra {
    st.installed.notes.extras.notes(key_with_id(st, addon))
}

/// Change the entry of one slot and write the store. Returns the entry afterwards.
pub fn edit_slot(st: &mut State, slot: u8, f: impl FnOnce(&mut ModExtra)) -> ModExtra {
    let Some(key) = slot_key(st, slot) else {
        return ModExtra::default();
    };
    let path = extras_file();
    let entry = mod_extras::edit_extras(&path, |extras| {
        let entry = extras.entry(key);
        f(entry);
        entry.clone()
    })
    .unwrap_or_else(|error| {
        st.installed.notes.error = Some(error.to_string());
        ModExtra::default()
    });
    st.installed.notes.extras = mod_extras::read_extras(&path).unwrap_or_default();
    entry
}

fn addon_at(st: &State, slot: u8) -> Option<&AddonRow> {
    st.addons.addons.iter().find(|addon| addon.slot == slot)
}

// --- reading and the metadata backfill ---------------------------------------

/// Read the store when the slots changed, and follow them (`sync`): an update that wrote a new
/// size keeps the notes of the same GameBanana mod. Returns whether anything changed.
pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if !st.demo && st.addons.scanned {
        let sig: Vec<(u8, u64)> = st
            .addons
            .addons
            .iter()
            .map(|addon| (addon.slot, addon.size_bytes))
            .collect();
        if st.installed.notes.sig.as_ref() != Some(&sig) {
            let path = extras_file();
            let mut extras = mod_extras::read_extras(&path).unwrap_or_default();
            let keys: Vec<SlotKey> = st
                .addons
                .addons
                .iter()
                .map(|addon| key_with_id(st, addon))
                .collect();
            let mut dirty = extras.sync(&keys);
            dirty |= extras.ensure(&keys);
            if dirty {
                let _ = mod_extras::write_extras(&path, &extras);
            }
            st.installed.notes.extras = extras;
            st.installed.notes.loaded = true;
            st.installed.notes.sig = Some(sig);
            changed = true;
        }
    }
    changed |= poll_backfill(st);
    changed |= load_local_images(st);
    changed
}

/// Fetch the GameBanana page of every mod that has an id but no metadata yet. One worker run
/// at a time, at most every time the set of ids changes.
fn poll_backfill(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.installed.notes.backfill.try_take() {
        if let Ok(entries) = result {
            let path = extras_file();
            let _ = mod_extras::edit_extras(&path, |extras| {
                for entry in entries {
                    let at = extras
                        .mods
                        .iter()
                        .position(|known| known.uid == entry.uid);
                    match at {
                        Some(at) => {
                            let old = &mut extras.mods[at];
                            old.hero = entry.hero.clone();
                            old.category = entry.category.clone();
                            old.root_category = entry.root_category.clone();
                            old.remote_nsfw = entry.remote_nsfw;
                            old.remote_files = entry.remote_files.clone();
                            old.fetched = true;
                        }
                        None => extras.mods.push(entry),
                    }
                }
            });
            st.installed.notes.extras = mod_extras::read_extras(&path).unwrap_or_default();
        }
        changed = true;
    }

    if st.demo || rune::is_headless() || !st.addons.scanned || st.prefs.offline {
        return changed;
    }
    let mut wanted: Vec<(String, u8, u64, u64)> = Vec::new();
    for addon in &st.addons.addons {
        let Some(id) = meta_for(st, addon).and_then(|meta| meta.gamebanana_id) else {
            continue;
        };
        let entry = extra(st, addon);
        if entry.fetched || entry.uid.is_empty() {
            continue;
        }
        wanted.push((entry.uid, addon.slot, addon.size_bytes, id));
    }
    let ids: Vec<u64> = wanted.iter().map(|(_, _, _, id)| *id).collect();
    if wanted.is_empty()
        || st.installed.notes.backfill.is_running()
        || st.installed.notes.backfill_ids.as_ref() == Some(&ids)
    {
        return changed;
    }
    st.installed.notes.backfill_ids = Some(ids);
    let heroes = mod_extras::known_heroes(&st.installed.notes.extras);
    let wake = st.wake.clone();
    st.installed.notes.backfill.start(wake, move || {
        let mut done = Vec::new();
        for (uid, slot, size, id) in wanted {
            let Ok(profile) = gamebanana::fetch_mod_profile(id) else {
                // A page that cannot be read now is tried again next session.
                continue;
            };
            let mut entry = ModExtra {
                slot,
                size_bytes: size,
                gamebanana_id: Some(id),
                uid,
                ..ModExtra::default()
            };
            apply_profile(&mut entry, &profile, &heroes);
            done.push(entry);
        }
        Ok(done)
    });
    changed
}

/// Copy a GameBanana page's category facts onto an entry.
pub fn apply_profile(entry: &mut ModExtra, profile: &gamebanana::ModProfile, heroes: &BTreeSet<String>) {
    let (hero, category, root) =
        mod_extras::classify(&profile.category, &profile.super_category, heroes);
    entry.hero = hero;
    entry.category = category;
    entry.root_category = root;
    entry.remote_nsfw = profile.nsfw;
    entry.remote_files = profile.files.iter().map(|file| file.name.clone()).collect();
    entry.fetched = true;
}

/// Decode a picture the player chose for a local mod. The image cache decodes on its own
/// thread; headless scenes collect the bytes for `setup`.
fn load_local_images(st: &mut State) -> bool {
    let mut changed = false;
    for entry in st.installed.notes.extras.mods.clone() {
        let Some(path) = entry.image else { continue };
        let key = path.to_string_lossy().into_owned();
        if st.thumbnails.contains_key(&key) {
            continue;
        }
        match st.images.as_ref() {
            Some(cache) => {
                if path.is_file() {
                    let image = cache.file(&path, 1.0, 1.0);
                    st.thumbnails.insert(key, image.pixels().clone());
                    changed = true;
                }
            }
            None => {
                if !st.pending_thumbs.iter().any(|(known, _)| *known == key)
                    && let Ok(bytes) = std::fs::read(&path)
                {
                    st.pending_thumbs.push((key, bytes));
                }
            }
        }
    }
    changed
}

// --- filters -----------------------------------------------------------------

/// Hero names in the library, sorted.
pub fn heroes(st: &State) -> Vec<String> {
    let mut names: BTreeSet<String> = st
        .installed
        .notes
        .extras
        .mods
        .iter()
        .filter_map(|entry| entry.hero.clone())
        .collect();
    names.retain(|name| !name.trim().is_empty());
    names.into_iter().collect()
}

/// Leaf categories in the library, sorted.
pub fn categories(st: &State) -> Vec<String> {
    let mut names: BTreeSet<String> = st
        .installed
        .notes
        .extras
        .mods
        .iter()
        .filter_map(|entry| entry.category.clone())
        .collect();
    names.retain(|name| !name.trim().is_empty());
    names.into_iter().collect()
}

/// Does a mod pass the hero, category, tag and list filters?
pub fn matches(st: &State, addon: &AddonRow) -> bool {
    let notes = &st.installed.notes;
    if notes.hero.is_none() && notes.category.is_none() && notes.tag.is_none() && notes.list.is_none()
    {
        return true;
    }
    let entry = extra(st, addon);
    if let Some(hero) = &notes.hero
        && entry.hero.as_deref() != Some(hero.as_str())
    {
        return false;
    }
    if let Some(category) = &notes.category
        && entry.category.as_deref() != Some(category.as_str())
    {
        return false;
    }
    if let Some(tag) = &notes.tag
        && !entry.tags.iter().any(|known| known.eq_ignore_ascii_case(tag))
    {
        return false;
    }
    if let Some(list) = &notes.list {
        let member = st
            .installed
            .notes
            .extras
            .list(list)
            .is_some_and(|list| list.members.iter().any(|uid| uid == &entry.uid));
        if !member {
            return false;
        }
    }
    true
}

/// Clear every filter this module owns.
pub fn clear(st: &mut State) {
    let notes = &mut st.installed.notes;
    notes.hero = None;
    notes.category = None;
    notes.tag = None;
    notes.list = None;
}

/// The filter buttons and the active filter values, under the status switch.
pub fn filter_row(s: &State, u: &Ui) -> Option<El> {
    let heroes = heroes(s);
    let categories = categories(s);
    let tags = s.installed.notes.extras.all_tags();
    let lists = &s.installed.notes.extras.lists;
    if heroes.is_empty() && categories.is_empty() && tags.is_empty() && lists.is_empty() {
        return None;
    }
    let notes = &s.installed.notes;
    let mut line = row().key("installed-notes-filters").w_full().gap(8.0).center_y();
    let mut any = false;
    for (key, label, value, menu) in [
        (
            "installed-hero",
            "Hero",
            notes.hero.clone(),
            super::Menu::Hero,
        ),
        (
            "installed-category",
            "Category",
            notes.category.clone(),
            super::Menu::Category,
        ),
        ("installed-tag", "Tag", notes.tag.clone(), super::Menu::Tag),
        (
            "installed-list",
            "List",
            notes
                .list
                .as_ref()
                .and_then(|id| s.installed.notes.extras.list(id))
                .map(|list| list.name.clone()),
            super::Menu::List,
        ),
    ] {
        if value.is_none() {
            continue;
        }
        any = true;
        let text_value = value.clone().unwrap_or_default();
        let shown = format!("{label}: {text_value}");
        let close: fn(&mut State) = match menu {
            super::Menu::Hero => |st: &mut State| st.installed.notes.hero = None,
            super::Menu::Category => |st: &mut State| st.installed.notes.category = None,
            super::Menu::Tag => |st: &mut State| st.installed.notes.tag = None,
            _ => |st: &mut State| st.installed.notes.list = None,
        };
        line = line.child(
            row()
                .key(key)
                .h(28.0)
                .pl(10.0)
                .pr(6.0)
                .gap(4.0)
                .center_y()
                .radius(crate::ui::r(6.0))
                .fill(u.selected)
                .border(1.0, u.hair)
                .hover(u.hover)
                .focusable(true)
                .role("button")
                .label(format!("Remove the {label} filter"))
                .cursor(rune::Cursor::Pointer)
                .on_press(close)
                .child(text(shown).font(SMALL).color(u.prose).a11y_hidden())
                .child(icon(&icons::X, 12.0).color(u.muted).a11y_hidden()),
        );
    }
    if !any {
        // No filter is on: the buttons that open one.
        line = line.child(
            dropdown(u, "installed-hero", "Hero", &icons::USER)
                .on_press(|st: &mut State| super::open_menu(st, super::Menu::Hero)),
        );
        if !categories.is_empty() {
            line = line.child(
                dropdown(u, "installed-category", "Category", &icons::LAYERS)
                    .on_press(|st: &mut State| super::open_menu(st, super::Menu::Category)),
            );
        }
        if !tags.is_empty() {
            line = line.child(
                dropdown(u, "installed-tag", "Tag", &icons::FILTER)
                    .on_press(|st: &mut State| super::open_menu(st, super::Menu::Tag)),
            );
        }
        if !lists.is_empty() {
            line = line.child(
                dropdown(u, "installed-list", "List", &icons::LIST)
                    .on_press(|st: &mut State| super::open_menu(st, super::Menu::List)),
            );
        }
    } else {
        line = line.child(
            row()
                .key("installed-notes-clear")
                .h(28.0)
                .px(8.0)
                .center_y()
                .radius(crate::ui::r(6.0))
                .hover(u.hover)
                .focusable(true)
                .role("button")
                .label("Clear the filters")
                .cursor(rune::Cursor::Pointer)
                .on_press(clear)
                .child(text("Clear").font(SMALL).color(u.muted).a11y_hidden()),
        );
    }
    Some(line)
}

/// A 28 point filter chip with a glyph and a chevron.
fn dropdown(u: &Ui, key: &str, label: &str, glyph: &'static rune::Icon) -> El {
    row()
        .key(key)
        .h(28.0)
        .pl(8.0)
        .pr(6.0)
        .gap(6.0)
        .center_y()
        .radius(crate::ui::r(6.0))
        .border(1.0, u.hair_strong)
        .hover(u.hover)
        .focusable(true)
        .role("button")
        .label(label)
        .cursor(rune::Cursor::Pointer)
        .child(icon(glyph, 14.0).color(u.muted).a11y_hidden())
        .child(text(label.to_string()).font(SMALL).color(u.prose).a11y_hidden())
        .child(icon(&icons::CHEVRON_DOWN, 12.0).color(u.faint).a11y_hidden())
}

// --- menus -------------------------------------------------------------------

/// The rows of one filter menu.
pub fn menu_rows(s: &State, u: &Ui, menu: super::Menu) -> Vec<El> {
    let notes = &s.installed.notes;
    let mut rows = Vec::new();
    let any_row = |rows: &mut Vec<El>, value: Option<String>, label: &str, key: String| {
        let on = match menu {
            super::Menu::Hero => notes.hero == value,
            super::Menu::Category => notes.category == value,
            super::Menu::Tag => notes.tag == value,
            _ => notes.list == value,
        };
        rows.push(
            super::menu_item(
                u,
                &key,
                Some(if on { &icons::CHECK } else { &NO_ICON }),
                label,
                "",
                true,
            )
            .on_press(move |st: &mut State| {
                st.installed.menu = None;
                match menu {
                    super::Menu::Hero => st.installed.notes.hero = value.clone(),
                    super::Menu::Category => st.installed.notes.category = value.clone(),
                    super::Menu::Tag => st.installed.notes.tag = value.clone(),
                    _ => st.installed.notes.list = value.clone(),
                }
            }),
        );
    };
    any_row(&mut rows, None, "Any", format!("menu-{menu:?}-any"));
    match menu {
        super::Menu::Hero => {
            for hero in heroes(s) {
                any_row(&mut rows, Some(hero.clone()), &hero, format!("menu-hero-{hero}"));
            }
        }
        super::Menu::Category => {
            for category in categories(s) {
                any_row(
                    &mut rows,
                    Some(category.clone()),
                    &category,
                    format!("menu-category-{category}"),
                );
            }
        }
        super::Menu::Tag => {
            for tag in s.installed.notes.extras.all_tags() {
                any_row(&mut rows, Some(tag.clone()), &tag, format!("menu-tag-{tag}"));
            }
        }
        super::Menu::List => {
            for list in &s.installed.notes.extras.lists {
                let label = format!("{} ({})", list.name, list.members.len());
                any_row(&mut rows, Some(list.id.clone()), &label, format!("menu-list-{}", list.id));
            }
            rows.push(super::menu_rule(u));
            rows.push(
                super::menu_item(u, "menu-lists-manage", Some(&icons::LIST), "Mod lists", "", true)
                    .on_press(|st: &mut State| {
                        st.installed.menu = None;
                        st.installed.notes.dialog = Some(Dialog::Lists);
                    }),
            );
        }
        _ => {}
    }
    rows
}

// --- dialogs -----------------------------------------------------------------

pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    match s.installed.notes.dialog.as_ref()? {
        Dialog::Tags(slot) => Some(tags_dialog(s, u, *slot)),
        Dialog::Lists => Some(lists_dialog(s, u)),
        Dialog::EditLocal(slot) => Some(edit_dialog(s, u, *slot)),
        Dialog::NewList => Some(new_list_dialog(s, u)),
    }
}

fn close(st: &mut State) {
    st.installed.notes.dialog = None;
    st.installed.notes.error = None;
}

fn tags_dialog(s: &State, u: &Ui, slot: u8) -> El {
    let addon = addon_at(s, slot);
    let entry = addon.map(|addon| extra(s, addon)).unwrap_or_default();
    let mut tags = col().w_full().gap(6.0);
    for tag in &entry.tags {
        let tag = tag.clone();
        tags = tags.child(
            row()
                .key(format!("tag-{tag}"))
                .h(32.0)
                .pl(10.0)
                .pr(6.0)
                .gap(8.0)
                .center_y()
                .radius(crate::ui::r(6.0))
                .border(1.0, u.hair)
                .child(text(tag.clone()).font(BODY).color(u.prose).grow(1.0).min_w(0.0).truncate())
                .child(
                    u.icon_button(&icons::X, &format!("Remove {tag}"))
                        .key(format!("tag-remove-{tag}"))
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| remove_tag(st, slot, &tag)),
                ),
        );
    }
    if entry.tags.is_empty() {
        tags = tags.child(text("No tags yet.").font(SMALL).color(u.faint));
    }
    let field = rune::text_input(&s.installed.notes.text, BODY)
        .key("tags-input")
        .single_line()
        .label("Tags, separated by commas")
        .grow(1.0)
        .min_w(0.0)
        .color(u.ink)
        .placeholder("Add a tag, like comp or ranked", u.faint)
        .caret(u.accent, u.accent.alpha(0.20))
        .on_edit(rune::edit_access(|st: &mut State| &mut st.installed.notes.text))
        .on_submit(move |st: &mut State, _tx: &mut Tx| add_typed_tags(st, slot));
    let name = addon
        .map(|addon| super::display_name(s, addon))
        .unwrap_or_else(|| format!("Slot {slot:02}"));
    dialog_shell(u, "installed-tags", &format!("Tags for {name}"))
        .child(text("Tags").font(H1).color(u.ink))
        .child(
            text("Your own labels. Filter the page by one, or use them in a mod list.")
                .font(BODY)
                .color(u.muted)
                .wrap(),
        )
        .child(tags)
        .child(field)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(error_line(s, u))
                .child(spacer())
                .child(
                    u.action("Done", ActionKind::Secondary)
                        .key("tags-done")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(close),
                )
                .child(
                    u.action("Add", ActionKind::Primary)
                        .key("tags-add")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| add_typed_tags(st, slot)),
                ),
        )
}

fn lists_dialog(s: &State, u: &Ui) -> El {
    let extras = &s.installed.notes.extras;
    let mut list = col().w_full().gap(4.0);
    if extras.lists.is_empty() {
        list = list.child(
            text("A list turns several mods on or off together, and filters the page.")
                .font(BODY)
                .color(u.faint)
                .wrap(),
        );
    }
    for entry in &extras.lists {
        let members = entry.members.len();
        let id = entry.id.clone();
        let id_show = id.clone();
        let id_on = id.clone();
        let id_off = id.clone();
        let name = entry.name.clone();
        list = list.child(
            row()
                .key(format!("list-{}", entry.id))
                .w_full()
                .h(52.0)
                .gap(8.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(text(name.clone()).font(BODY_MED).color(u.ink).truncate())
                        .child(
                            text(format!("{members} mod{}", if members == 1 { "" } else { "s" }))
                                .font(SMALL)
                                .color(u.faint),
                        ),
                )
                .child(
                    u.action("Show", ActionKind::Ghost)
                        .key(format!("list-show-{}", entry.id))
                        .disabled(members == 0)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| {
                            st.installed.notes.list = Some(id_show.clone());
                            close(st);
                        }),
                )
                .child(
                    u.action("Turn on", ActionKind::Secondary)
                        .key(format!("list-on-{}", entry.id))
                        .disabled(members == 0)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| lists_switch(st, &id_on, true)),
                )
                .child(
                    u.action("Turn off", ActionKind::Secondary)
                        .key(format!("list-off-{}", entry.id))
                        .disabled(members == 0)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| lists_switch(st, &id_off, false)),
                )
                .child(
                    u.icon_button(&icons::TRASH_2, &format!("Delete {name}"))
                        .key(format!("list-delete-{}", entry.id))
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| delete_list(st, &id)),
                ),
        );
    }
    dialog_shell(u, "installed-lists", "Mod lists")
        .child(text("Mod lists").font(H1).color(u.ink))
        .child(
            text("Pick mods on the page, then make a list from the selection.")
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
                .child(error_line(s, u))
                .child(spacer())
                .child(
                    u.action("Cancel", ActionKind::Ghost)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(close),
                )
                .child(
                    u.action("Make a list", ActionKind::Primary)
                        .key("lists-new")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| {
                            st.installed.notes.error = None;
                            st.installed.notes.text = TextEdit::new("");
                            st.installed.notes.dialog = Some(Dialog::NewList);
                        }),
                ),
        )
}

fn new_list_dialog(s: &State, u: &Ui) -> El {
    let picked = s.installed.selected.len();
    let field = rune::text_input(&s.installed.notes.text, BODY)
        .key("list-name")
        .single_line()
        .label("List name")
        .w_full()
        .color(u.ink)
        .placeholder("Name the list", u.faint)
        .caret(u.accent, u.accent.alpha(0.20))
        .on_edit(rune::edit_access(|st: &mut State| &mut st.installed.notes.text));
    dialog_shell(u, "installed-list-name", "New list")
        .child(text("New list").font(H1).color(u.ink))
        .child(
            text(format!(
                "Holds the {picked} mod{} picked now. You can add more later.",
                if picked == 1 { "" } else { "s" }
            ))
            .font(BODY)
            .color(u.muted)
            .wrap(),
        )
        .child(field)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(error_line(s, u))
                .child(spacer())
                .child(
                    u.action("Cancel", ActionKind::Ghost)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| {
                            st.installed.notes.dialog = Some(Dialog::Lists)
                        }),
                )
                .child(
                    u.action("Make the list", ActionKind::Primary)
                        .key("list-name-save")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(save_new_list),
                ),
        )
}

fn edit_dialog(s: &State, u: &Ui, slot: u8) -> El {
    let addon = addon_at(s, slot);
    let entry = addon.map(|addon| extra(s, addon)).unwrap_or_default();
    let image_name = s
        .installed
        .notes
        .image
        .as_ref()
        .and_then(|path| path.file_name().map(|name| name.to_string_lossy().into_owned()))
        .or_else(|| {
            entry
                .image
                .as_ref()
                .and_then(|path| path.file_name().map(|name| name.to_string_lossy().into_owned()))
        })
        .unwrap_or_else(|| "No picture".to_string());
    let field = rune::text_input(&s.installed.notes.text, BODY)
        .key("edit-name")
        .single_line()
        .label("Name")
        .w_full()
        .color(u.ink)
        .placeholder("Name this mod", u.faint)
        .caret(u.accent, u.accent.alpha(0.20))
        .on_edit(rune::edit_access(|st: &mut State| &mut st.installed.notes.text));
    let nsfw = s.installed.notes.nsfw;
    let name = addon
        .map(|addon| super::display_name(s, addon))
        .unwrap_or_else(|| format!("Slot {slot:02}"));
    dialog_shell(u, "installed-edit", &format!("Edit {name}"))
        .child(text("Edit this mod").font(H1).color(u.ink))
        .child(
            text("Only the name, picture and flag in Sigil change. The mod file stays as it is.")
                .font(BODY)
                .color(u.muted)
                .wrap(),
        )
        .child(u.field_label("Name"))
        .child(field)
        .child(
            row()
                .w_full()
                .h(44.0)
                .gap(12.0)
                .center_y()
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(text("Picture").font(BODY).color(u.ink))
                        .child(text(image_name).font(SMALL).color(u.faint).truncate()),
                )
                .child(
                    u.action("Choose a picture", ActionKind::Secondary)
                        .key("edit-image")
                        .cursor(rune::Cursor::Pointer)
                        .on_press_tx(pick_image),
                ),
        )
        .child(
            row()
                .w_full()
                .h(44.0)
                .gap(12.0)
                .center_y()
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(text("Mature").font(BODY).color(u.ink))
                        .child(
                            text("Blurs the picture while Mature mods are hidden or blurred.")
                                .font(SMALL)
                                .color(u.faint),
                        ),
                )
                .child(
                    u.toggle(nsfw)
                        .key("edit-nsfw")
                        .label("Mark as mature")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| st.installed.notes.nsfw = !st.installed.notes.nsfw),
                ),
        )
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(error_line(s, u))
                .child(spacer())
                .child(
                    u.action("Cancel", ActionKind::Ghost)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(close),
                )
                .child(
                    u.action("Save", ActionKind::Primary)
                        .key("edit-save")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| save_edit(st, slot)),
                ),
        )
}

fn dialog_shell(u: &Ui, key: &str, label: &str) -> El {
    u.dialog_shell()
        .key(key)
        .role("dialog")
        .label(label)
        .w(480.0)
        .padding(24.0)
        .gap(16.0)
        .modal(true, close)
}

fn error_line(s: &State, u: &Ui) -> El {
    text(s.installed.notes.error.clone().unwrap_or_default())
        .font(SMALL)
        .color(u.rose)
        .wrap()
}

// --- handlers ----------------------------------------------------------------

/// Open the editor for a slot with its current name, picture and flag.
pub fn open_edit(st: &mut State, slot: u8) {
    let Some(addon) = addon_at(st, slot) else { return };
    let entry = extra(st, addon);
    let name = meta_for(st, addon)
        .map(|meta| meta.name.clone())
        .unwrap_or_default();
    st.installed.notes.text = TextEdit::new(&name);
    st.installed.notes.nsfw = entry.is_nsfw();
    st.installed.notes.image = entry.image.clone();
    st.installed.notes.error = None;
    st.installed.notes.dialog = Some(Dialog::EditLocal(slot));
}

fn pick_image(_st: &mut State, tx: &mut Tx) {
    let options = FileDialogOptions {
        title: "Choose a picture".to_string(),
        ..FileDialogOptions::default()
    }
    .filter("Pictures", &["png", "jpg", "jpeg", "gif", "webp"]);
    tx.open_file(options, |st: &mut State, result, _tx| match result {
        Ok(Some(path)) => st.installed.notes.image = Some(path),
        Ok(None) => {}
        Err(error) => st.installed.notes.error = Some(format!("The file picker could not open: {error}")),
    });
}

fn save_edit(st: &mut State, slot: u8) {
    if addon_at(st, slot).is_none() {
        return;
    }
    let name = st.installed.notes.text.text.trim().to_string();
    if name.is_empty() {
        st.installed.notes.error = Some("Give the mod a name first.".to_string());
        return;
    }
    // The picture is copied into Sigil's own data folder, never left pointing at wherever the
    // player picked it from.
    let mut stored = None;
    if let Some(path) = st.installed.notes.image.clone() {
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("png")
            .to_ascii_lowercase();
        let folder = crate::data::data_dir().join("mod-images");
        let _ = std::fs::create_dir_all(&folder);
        let target = folder.join(format!("slot{:02}-{}.{}", slot, mod_extras::new_uid(), extension));
        match std::fs::read(&path) {
            Ok(bytes) => match std::fs::write(&target, bytes) {
                Ok(()) => stored = Some(target),
                Err(error) => {
                    st.installed.notes.error = Some(format!("The picture could not be saved: {error}"));
                    return;
                }
            },
            Err(error) => {
                st.installed.notes.error = Some(format!("The picture could not be read: {error}"));
                return;
            }
        }
    }
    let nsfw = st.installed.notes.nsfw;
    edit_slot(st, slot, |entry| {
        entry.nsfw = Some(nsfw);
        if stored.is_some() {
            entry.image = stored.clone();
        }
    });
    let library = super::library_file();
    let meta = st
        .addons
        .addons
        .iter()
        .find(|addon| addon.slot == slot)
        .and_then(|addon| meta_for(st, addon))
        .cloned();
    if let Some(mut meta) = meta {
        meta.name = name;
        let _ = deadlock_core::import::record_meta(&library, meta);
    }
    st.installed.library_sig = None;
    st.status = format!("Saved slot {slot:02}.");
    close(st);
}

fn add_typed_tags(st: &mut State, slot: u8) {
    let line = st.installed.notes.text.text.clone();
    let tags = mod_extras::parse_tags(&line);
    if tags.is_empty() {
        st.installed.notes.error = Some("Type a tag first.".to_string());
        return;
    }
    edit_slot(st, slot, |entry| {
        for tag in &tags {
            mod_extras::add_tag(entry, tag);
        }
    });
    st.installed.notes.text = TextEdit::new("");
    st.installed.notes.error = None;
}

fn remove_tag(st: &mut State, slot: u8, tag: &str) {
    edit_slot(st, slot, |entry| {
        entry.tags.retain(|known| !known.eq_ignore_ascii_case(tag));
    });
    if st.installed.notes.tag.as_deref() == Some(tag) {
        st.installed.notes.tag = None;
    }
}

/// Turn every mod of a list on or off. One write per mod, one Undo for the lot.
pub fn lists_switch(st: &mut State, id: &str, enable: bool) {
    if crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    let Some(list) = st.installed.notes.extras.list(id) else { return };
    let name = list.name.clone();
    let members = list.members.clone();
    let mut wanted: std::collections::BTreeMap<u8, bool> = std::collections::BTreeMap::new();
    let mut found = 0;
    for uid in &members {
        if let Some(slot) = st.installed.notes.extras.slot_for_uid(uid)
            && st.addons.addons.iter().any(|addon| addon.slot == slot)
        {
            wanted.insert(slot, enable);
            found += 1;
        }
    }
    if wanted.is_empty() {
        st.status = format!("Every mod of {name} is gone from the library.");
        return;
    }
    let missing = members.len() - found;
    let Some(game_root) = super::writable_game(st) else { return };
    super::apply_switch(
        st,
        &game_root,
        &wanted,
        format!(
            "Turned {} {found} mod{} of {name}.{}",
            if enable { "on" } else { "off" },
            if found == 1 { "" } else { "s" },
            if missing == 0 {
                String::new()
            } else {
                format!(" {missing} of its mods are gone.")
            }
        ),
    );
}

fn delete_list(st: &mut State, id: &str) {
    let path = extras_file();
    let _ = mod_extras::edit_extras(&path, |extras| extras.remove_list(id));
    st.installed.notes.extras = mod_extras::read_extras(&path).unwrap_or_default();
    if st.installed.notes.list.as_deref() == Some(id) {
        st.installed.notes.list = None;
    }
    st.status = "List deleted. The mods stay as they are.".to_string();
}

/// Turn a mod's pin on or off. Pinned mods sit at the top of the disabled list.
pub fn toggle_pin(st: &mut State, slot: u8) {
    let Some(addon) = addon_at(st, slot) else { return };
    let entry = extra(st, addon);
    let pinned = !entry.pinned;
    edit_slot(st, slot, |entry| entry.pinned = pinned);
    st.status = if pinned {
        format!("Pinned slot {slot:02} to the top of the disabled mods.")
    } else {
        format!("Slot {slot:02} is no longer pinned.")
    };
}

/// Turn "load first" on or off. On, the mod moves to the lowest slot of its group, where it
/// wins every file it shares with another mod.
pub fn toggle_always_first(st: &mut State, slot: u8) {
    if crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    let Some(addon) = addon_at(st, slot) else { return };
    let entry = extra(st, addon);
    let first = !entry.always_first;
    edit_slot(st, slot, |entry| entry.always_first = first);
    if !first || super::writable_game(st).is_none() {
        if !first {
            st.status = format!("Slot {slot:02} no longer loads first.");
        }
        return;
    }
    if st.demo {
        return;
    }
    let dir = super::addons_folder(st);
    let plan = deadlock_core::load_order::read_slot_files(&dir)
        .and_then(|files| deadlock_core::load_order::plan_slot_to_front(&dir, &files, slot, &deadlock_core::load_order::new_token()));
    match plan {
        Ok(plan) => match deadlock_core::load_order::apply_reorder(&plan, true) {
            Ok(report) => {
                let library = super::library_file();
                let mut lib = deadlock_core::import::read_library(&library).unwrap_or_default();
                deadlock_core::load_order::remap_library(&mut lib, &report.moves);
                let _ = deadlock_core::import::write_library(&library, &lib);
                st.installed.library_sig = None;
                st.installed.notes.sig = None;
                st.installed.last_undo = Some(super::Undo::Reorder(plan));
                st.installed.undo_fresh = true;
                st.status = format!("Slot {slot:02} loads first from now on.");
                super::rescan(st);
            }
            Err(error) => st.status = error.to_string(),
        },
        Err(error) => st.status = error.to_string(),
    }
}

/// Remember that the player does not want the newer file of this mod.
pub fn ignore_update(st: &mut State, slot: u8, file_id: Option<u64>) {
    edit_slot(st, slot, |entry| entry.ignored_update = file_id.or(Some(0)));
    st.status = "Update ignored. You can ask for it again from the update chip.".to_string();
}

/// Forget an ignored update.
pub fn unignore_update(st: &mut State, slot: u8) {
    edit_slot(st, slot, |entry| entry.ignored_update = None);
    st.status = "The update shows again.".to_string();
}

/// Save the name typed in the "New list" dialog for the mods picked now.
pub fn save_new_list(st: &mut State) {
    let name = st.installed.notes.text.text.trim().to_string();
    if name.is_empty() {
        st.installed.notes.error = Some("Name the list first.".to_string());
        return;
    }
    if st.installed.selected.is_empty() {
        st.installed.notes.error = Some("Pick the mods the list should hold first.".to_string());
        return;
    }
    let members: Vec<String> = st
        .installed
        .selected
        .iter()
        .filter_map(|slot| addon_at(st, *slot))
        .map(|addon| extra(st, addon).uid)
        .filter(|uid| !uid.is_empty())
        .collect();
    if members.is_empty() {
        st.installed.notes.error = Some("The picked mods have no record yet. Scan again.".to_string());
        return;
    }
    let path = extras_file();
    let id = mod_extras::edit_extras(&path, |extras| extras.add_list(&name, members))
        .unwrap_or_else(|error| {
            st.installed.notes.error = Some(error.to_string());
            String::new()
        });
    st.installed.notes.extras = mod_extras::read_extras(&path).unwrap_or_default();
    if !id.is_empty() {
        st.installed.notes.list = Some(id);
        st.installed.notes.dialog = None;
        st.status = format!("Made the list {name}.");
    }
}

/// A slot's label in the variant picker and the card facts, for other modules.
pub fn label_for(st: &State, addon: &AddonRow) -> String {
    let entry = extra(st, addon);
    mod_extras::variant_label(Some(&entry), &addon.file, addon.slot)
}

/// The preview image key for a card: the player's own picture first, then GameBanana's.
pub fn preview_key(st: &State, addon: &AddonRow) -> Option<String> {
    let entry = extra(st, addon);
    if let Some(path) = entry.image {
        return Some(path.to_string_lossy().into_owned());
    }
    meta_for(st, addon).and_then(|meta| meta.thumbnail_url.clone())
}

/// A card's small chips: pinned, load first, mature.
pub fn card_chips(s: &State, u: &Ui, addon: &AddonRow) -> Vec<El> {
    let entry = extra(s, addon);
    let mut chips = Vec::new();
    if addon.enabled && entry.always_first {
        chips.push(chip(u, &icons::ARROW_UP, "Loads first"));
    } else if entry.pinned {
        chips.push(chip(u, &icons::PIN, "Pinned"));
    }
    if entry.is_nsfw() {
        chips.push(chip(u, &icons::EYE, "Mature"));
    }
    chips
}

fn chip(u: &Ui, glyph: &'static rune::Icon, label: &str) -> El {
    row()
        .h(20.0)
        .pl(5.0)
        .pr(7.0)
        .gap(4.0)
        .center_y()
        .radius(crate::ui::r(6.0))
        .fill(u.raised.alpha(0.92))
        .border(1.0, u.hair)
        .child(icon(glyph, 12.0).color(u.muted).a11y_hidden())
        .child(text(label.to_string()).font(crate::ui::MICRO).color(u.prose).a11y_hidden())
}
