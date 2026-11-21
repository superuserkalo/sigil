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
use deadlock_core::mod_extras::{self, Extras, ModExtra, SlotKey, TagKind};
use rune::{El, FileDialogOptions, Tx, col, icon, row, spacer, text};
use rune::TextEdit;

use crate::app::State;
use crate::icons;
use crate::jobs::Job;
use crate::model::AddonRow;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, SMALL, Ui};

use super::{meta_for, NO_ICON};

#[path = "hero_guess.rs"]
pub mod hero_guess;

/// One tag in the Installed tag menu: a tag the player typed, a GameBanana category or a hero.
pub type TagRef = (TagKind, String);

/// The `Prefs::flags` id of Settings > Mods > Cycle tag filters (off by default).
pub const CYCLE_TAG_FILTERS: &str = "cycle_tag_filters";

/// The state of the extras store on this screen.
#[derive(Default)]
pub struct Notes {
    pub extras: Extras,
    pub loaded: bool,
    /// The slots the store was last read for.
    pub sig: Option<Vec<(u16, u64)>>,
    /// Mod pages read on a worker, to fill hero, category and NSFW.
    pub backfill: Job<Vec<ModExtra>>,
    /// GameBanana ids the backfill was started for.
    pub backfill_ids: Option<Vec<u64>>,
    pub hero: Option<String>,
    pub category: Option<String>,
    /// Tags a mod must have one of (green plus).
    pub tags_in: Vec<TagRef>,
    /// Tags a mod must not have (red minus).
    pub tags_out: Vec<TagRef>,
    pub list: Option<String>,
    /// Heroes read from the files of mods GameBanana filed under no hero.
    pub guess: hero_guess::Guess,
    pub dialog: Option<Dialog>,
    /// The tag line, mod name and list name typed in a dialog.
    pub text: TextEdit,
    pub nsfw: bool,
    /// A picture the player picked for a local mod.
    pub image: Option<PathBuf>,
    pub error: Option<String>,
    /// Pictures a mod names that were not on disk, and when that was seen: poll looked each
    /// one up again on every frame.
    pub missing_images: std::collections::HashMap<PathBuf, std::time::Instant>,
}

/// How long a missing picture waits before poll looks for it again.
const MISSING_IMAGE_RETRY: std::time::Duration = std::time::Duration::from_secs(30);

/// The open editor.
pub enum Dialog {
    /// Tags of one slot.
    Tags(u16),
    /// Named lists and their members.
    Lists,
    /// Name, picture and NSFW flag of a local mod.
    EditLocal(u16),
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
pub fn key_with_id(st: &State, addon: &AddonRow) -> SlotKey {
    SlotKey {
        gamebanana_id: meta_for(st, addon).and_then(|meta| meta.gamebanana_id),
        ..key(addon)
    }
}

/// The key of a slot, looked up from the page's rows.
fn slot_key(st: &State, slot: u16) -> Option<SlotKey> {
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
pub fn edit_slot(st: &mut State, slot: u16, f: impl FnOnce(&mut ModExtra)) -> ModExtra {
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

fn addon_at(st: &State, slot: u16) -> Option<&AddonRow> {
    st.addons.addons.iter().find(|addon| addon.slot == slot)
}

// --- reading and the metadata backfill ---------------------------------------

/// Read the store when the slots changed, and follow them (`sync`): an update that wrote a new
/// size keeps the notes of the same GameBanana mod. Returns whether anything changed.
pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if !st.demo && st.addons.scanned {
        let sig: Vec<(u16, u64)> = st
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
    changed |= hero_guess::poll(st);
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
    let mut wanted: Vec<(String, u16, u64, u64)> = Vec::new();
    let idx = super::index::Index::new(st);
    for addon in &st.addons.addons {
        let Some(id) = idx.meta(st, addon).and_then(|meta| meta.gamebanana_id) else {
            continue;
        };
        // A blank entry has no uid, so a mod without one is skipped as before.
        let Some(entry) = idx.extra_ref(st, addon) else { continue };
        if entry.fetched || entry.uid.is_empty() {
            continue;
        }
        wanted.push((entry.uid.clone(), addon.slot, addon.size_bytes, id));
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
    // Only the pictures not loaded yet, and not one seen missing a moment ago: this ran
    // through a copy of the whole notes store and asked the disk about every missing picture
    // on every poll.
    let notes = &st.installed.notes;
    let wanted: Vec<PathBuf> = notes
        .extras
        .mods
        .iter()
        .filter_map(|entry| entry.image.clone())
        .filter(|path| !st.thumbnails.contains_key(path.to_string_lossy().as_ref()))
        .filter(|path| notes.missing_images.get(path).is_none_or(|seen| seen.elapsed() >= MISSING_IMAGE_RETRY))
        .collect();
    for path in wanted {
        let key = path.to_string_lossy().into_owned();
        match st.images.as_ref() {
            Some(cache) => {
                if path.is_file() {
                    let image = cache.file(&path, 1.0, 1.0);
                    st.thumbnails.insert(key, image.pixels().clone());
                    st.installed.notes.missing_images.remove(&path);
                    changed = true;
                } else {
                    st.installed.notes.missing_images.insert(path, std::time::Instant::now());
                }
            }
            None => {
                if !st.pending_thumbs.iter().any(|(known, _)| *known == key) {
                    match std::fs::read(&path) {
                        Ok(bytes) => st.pending_thumbs.push((key, bytes)),
                        Err(_) => {
                            st.installed.notes.missing_images.insert(path, std::time::Instant::now());
                        }
                    }
                }
            }
        }
    }
    changed
}

// --- filters -----------------------------------------------------------------

/// Hero names in the library, sorted: GameBanana's hero categories and the heroes guessed
/// for the rest (`hero_guess`).
pub fn heroes(st: &State) -> Vec<String> {
    let mut names: BTreeSet<String> = st
        .addons
        .addons
        .iter()
        .flat_map(|addon| hero_guess::heroes_of(st, addon))
        .collect();
    names.extend(
        st.installed
            .notes
            .extras
            .mods
            .iter()
            .filter_map(|entry| entry.hero.clone()),
    );
    names.retain(|name| !name.trim().is_empty());
    names.into_iter().collect()
}

/// `!heroes(st).is_empty()`, stopping at the first hero.
pub(super) fn has_heroes(st: &State) -> bool {
    st.installed.notes.extras.mods.iter().any(|entry| entry.hero.as_ref().is_some_and(|h| !h.trim().is_empty()))
        || st
            .addons
            .addons
            .iter()
            .any(|addon| hero_guess::heroes_of(st, addon).iter().any(|name| !name.trim().is_empty()))
}

/// `!tag_options(st).is_empty()`, stopping at the first tag.
pub(super) fn has_tags(st: &State) -> bool {
    st.addons.addons.iter().any(|addon| !tags_of(st, addon).is_empty())
        || !st.installed.notes.extras.all_tags().is_empty()
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

/// Every tag of one mod: the player's own, its GameBanana categories (root and leaf, with
/// the label fixes of `normalize_category_label`) and its heroes. The order is the tag menu's.
pub fn tags_of(st: &State, addon: &AddonRow) -> Vec<TagRef> {
    let entry = extra(st, addon);
    let mut tags: Vec<TagRef> = Vec::new();
    let push = |tags: &mut Vec<TagRef>, kind: TagKind, label: String| {
        let label = label.trim().to_string();
        if !label.is_empty() && !tags.iter().any(|tag| same_tag(tag, &(kind, label.clone()))) {
            tags.push((kind, label));
        }
    };
    for tag in &entry.tags {
        push(&mut tags, TagKind::Custom, tag.clone());
    }
    let heroes = hero_guess::heroes_of(st, addon);
    for label in [entry.root_category.clone(), entry.category.clone()].into_iter().flatten() {
        let label = mod_extras::normalize_category_label(&label);
        // A hero leaf is the hero tag, not a category.
        if !heroes.iter().any(|hero| hero.eq_ignore_ascii_case(&label))
            && mod_extras::category_hero(&label).is_none()
        {
            push(&mut tags, TagKind::Base, label);
        }
    }
    for hero in heroes {
        push(&mut tags, TagKind::Hero, hero);
    }
    mod_extras::sort_tags(&st.installed.notes.extras.tag_order, &mut tags);
    tags
}

fn same_tag(a: &TagRef, b: &TagRef) -> bool {
    a.0 == b.0 && a.1.eq_ignore_ascii_case(&b.1)
}

/// Every tag in the library, in the player's display priority.
pub fn tag_options(st: &State) -> Vec<TagRef> {
    let mut all: Vec<TagRef> = Vec::new();
    for addon in &st.addons.addons {
        for tag in tags_of(st, addon) {
            if !all.iter().any(|known| same_tag(known, &tag)) {
                all.push(tag);
            }
        }
    }
    // Tags typed on mods that are not on disk now still show, so they can be filtered later.
    for tag in st.installed.notes.extras.all_tags() {
        let tag = (TagKind::Custom, tag);
        if !all.iter().any(|known| same_tag(known, &tag)) {
            all.push(tag);
        }
    }
    mod_extras::sort_tags(&st.installed.notes.extras.tag_order, &mut all);
    all
}

/// Does a mod pass the hero, category, tag and list filters? With included tags it must have
/// one of them; it must have none of the excluded ones (Grimoire's tag filter).
pub fn matches(st: &State, addon: &AddonRow) -> bool {
    let notes = &st.installed.notes;
    if notes.hero.is_none()
        && notes.category.is_none()
        && notes.tags_in.is_empty()
        && notes.tags_out.is_empty()
        && notes.list.is_none()
    {
        return true;
    }
    let entry = extra(st, addon);
    if let Some(hero) = &notes.hero
        && !hero_guess::heroes_of(st, addon).iter().any(|known| known == hero)
    {
        return false;
    }
    if let Some(category) = &notes.category
        && entry.category.as_deref() != Some(category.as_str())
    {
        return false;
    }
    if !notes.tags_in.is_empty() || !notes.tags_out.is_empty() {
        let tags = tags_of(st, addon);
        let has = |want: &TagRef| tags.iter().any(|tag| same_tag(tag, want));
        if !notes.tags_in.is_empty() && !notes.tags_in.iter().any(has) {
            return false;
        }
        if notes.tags_out.iter().any(has) {
            return false;
        }
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

/// Left click on a tag in the menu. Without cycling it includes the tag, or clears it when it
/// is on; with Cycle tag filters it walks off, included, excluded, off.
pub fn tag_left_click(st: &mut State, tag: &TagRef) {
    let cycle = st.prefs.flag(CYCLE_TAG_FILTERS);
    let notes = &mut st.installed.notes;
    let included = notes.tags_in.iter().position(|known| same_tag(known, tag));
    let excluded = notes.tags_out.iter().position(|known| same_tag(known, tag));
    match (included, excluded, cycle) {
        (Some(at), _, true) => {
            notes.tags_in.remove(at);
            notes.tags_out.push(tag.clone());
        }
        (Some(at), _, false) => {
            notes.tags_in.remove(at);
        }
        (None, Some(at), _) => {
            notes.tags_out.remove(at);
        }
        (None, None, _) => notes.tags_in.push(tag.clone()),
    }
}

/// Right click on a tag in the menu. Without cycling it excludes the tag (an included one
/// switches to excluded, an excluded one clears); with Cycle tag filters it clears either.
pub fn tag_right_click(st: &mut State, tag: &TagRef) {
    let cycle = st.prefs.flag(CYCLE_TAG_FILTERS);
    let notes = &mut st.installed.notes;
    let included = notes.tags_in.iter().position(|known| same_tag(known, tag));
    let excluded = notes.tags_out.iter().position(|known| same_tag(known, tag));
    match (included, excluded) {
        (Some(at), _) => {
            notes.tags_in.remove(at);
            if !cycle {
                notes.tags_out.push(tag.clone());
            }
        }
        (None, Some(at)) => {
            notes.tags_out.remove(at);
        }
        (None, None) => {
            if !cycle {
                notes.tags_out.push(tag.clone());
            }
        }
    }
}

/// Move a tag up or down in the display priority and save it.
fn move_tag(st: &mut State, tag: &TagRef, step: isize) {
    let shown = tag_options(st);
    let Some(order) = mod_extras::move_tag(&shown, (tag.0, &tag.1), step) else {
        return;
    };
    let path = extras_file();
    if let Err(error) = mod_extras::edit_extras(&path, |extras| extras.tag_order = order) {
        st.installed.notes.error = Some(error.to_string());
    }
    st.installed.notes.extras = mod_extras::read_extras(&path).unwrap_or_default();
}

/// Back to the default order: custom tags, then categories, then heroes, by name.
fn reset_tag_order(st: &mut State) {
    let path = extras_file();
    let _ = mod_extras::edit_extras(&path, |extras| extras.tag_order.clear());
    st.installed.notes.extras = mod_extras::read_extras(&path).unwrap_or_default();
}

fn tag_kind_label(kind: TagKind) -> &'static str {
    match kind {
        TagKind::Custom => "Tag",
        TagKind::Base => "Category",
        TagKind::Hero => "Hero",
    }
}

/// Clear every filter this module owns.
pub fn clear(st: &mut State) {
    let notes = &mut st.installed.notes;
    notes.hero = None;
    notes.category = None;
    notes.tags_in.clear();
    notes.tags_out.clear();
    notes.list = None;
}


/// The tag filter chip's value: the one tag with its sign, or the counts.
pub(super) fn tag_summary(notes: &Notes) -> Option<String> {
    let (inc, exc) = (notes.tags_in.len(), notes.tags_out.len());
    match (inc, exc) {
        (0, 0) => None,
        (1, 0) => Some(format!("+{}", notes.tags_in[0].1)),
        (0, 1) => Some(format!("\u{2212}{}", notes.tags_out[0].1)),
        _ => {
            let mut parts = Vec::new();
            if inc > 0 {
                parts.push(format!("+{inc}"));
            }
            if exc > 0 {
                parts.push(format!("\u{2212}{exc}"));
            }
            Some(parts.join(" "))
        }
    }
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
                    _ => st.installed.notes.list = value.clone(),
                }
            }),
        );
    };
    if menu == super::Menu::Tag {
        return tag_menu_rows(s, u);
    }
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

/// The one tag menu: the player's tags, GameBanana categories and heroes in the display
/// priority. Left click includes (or cycles, with Cycle tag filters on), right click excludes
/// (or clears). The arrows move a tag up or down; the order also sorts the tags on the cards.
fn tag_menu_rows(s: &State, u: &Ui) -> Vec<El> {
    use crate::screens::settings::i18n::tr;
    let notes = &s.installed.notes;
    let options = tag_options(s);
    let mut rows = Vec::new();
    let count = options.len();
    for (at, tag) in options.into_iter().enumerate() {
        let included = notes.tags_in.iter().any(|known| same_tag(known, &tag));
        let excluded = notes.tags_out.iter().any(|known| same_tag(known, &tag));
        let key = mod_extras::tag_order_key(tag.0, &tag.1);
        let (glyph, tone) = if included {
            (&icons::PLUS, u.teal)
        } else if excluded {
            (&icons::MINUS, u.rose)
        } else {
            (&NO_ICON, u.muted)
        };
        let hint = if excluded {
            tr("Excluded. Click to clear.")
        } else if included {
            tr("Included. Right click to exclude.")
        } else {
            tr("Click to include, right click to exclude.")
        };
        let left = tag.clone();
        let right = tag.clone();
        let up = tag.clone();
        let down = tag.clone();
        let arrow = |glyph_key: String, label: &str, flip: bool, enabled: bool| {
            let mut glyph = icon(&icons::ARROW_UP, 14.0).color(if enabled { u.muted } else { u.faint });
            if flip {
                glyph = glyph.rotate(std::f32::consts::PI);
            }
            let mut b = row()
                .key(glyph_key)
                .w(24.0)
                .h(24.0)
                .center()
                .radius(crate::ui::r(5.0))
                .role("button")
                .label(label.to_string())
                .child(glyph.a11y_hidden());
            if enabled {
                b = b.hover(u.hover).focusable(true).cursor(rune::Cursor::Pointer);
            } else {
                b = b.disabled(true);
            }
            b
        };
        rows.push(
            row()
                .key(format!("menu-tag-{key}"))
                .w_full()
                .h(crate::ui::CONTROL_L)
                .pl(10.0)
                .pr(4.0)
                .gap(10.0)
                .center_y()
                .radius(crate::ui::r(6.0))
                .role("menuitemcheckbox")
                .label(format!("{} ({}): {hint}", tag.1, tr(tag_kind_label(tag.0))))
                .focusable(true)
                .hover(u.hover)
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| tag_left_click(st, &left))
                .on_secondary_press(move |st: &mut State, _press, _tx: &mut Tx| tag_right_click(st, &right))
                .child(icon(glyph, 16.0).color(tone).a11y_hidden())
                .child(text(tag.1.clone()).font(BODY).color(u.prose).grow(1.0).min_w(0.0).truncate().a11y_hidden())
                .child(text(tr(tag_kind_label(tag.0)).to_string()).font(SMALL).color(u.faint).a11y_hidden())
                .child(
                    arrow(format!("menu-tag-up-{key}"), tr("Move up"), false, at > 0)
                        .on_press(move |st: &mut State| move_tag(st, &up, -1)),
                )
                .child(
                    arrow(format!("menu-tag-down-{key}"), tr("Move down"), true, at + 1 < count)
                        .on_press(move |st: &mut State| move_tag(st, &down, 1)),
                ),
        );
    }
    if rows.is_empty() {
        rows.push(super::menu_item(u, "menu-tag-none", None, tr("No tags yet"), "", false));
    }
    rows.push(super::menu_rule(u));
    let filtered = !notes.tags_in.is_empty() || !notes.tags_out.is_empty();
    rows.push(
        super::menu_item(u, "menu-tag-clear", Some(&icons::X), tr("Clear tag filters"), "", filtered)
            .on_press(|st: &mut State| {
                st.installed.notes.tags_in.clear();
                st.installed.notes.tags_out.clear();
            }),
    );
    rows.push(
        super::menu_item(
            u,
            "menu-tag-reset-order",
            Some(&icons::ROTATE_CCW),
            tr("Reset to default order"),
            "",
            !notes.extras.tag_order.is_empty(),
        )
        .on_press(reset_tag_order),
    );
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

fn tags_dialog(s: &State, u: &Ui, slot: u16) -> El {
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
        .unwrap_or_else(|| format!("Slot {}", deadlock_core::addons::slot_label(slot)));
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
    let list_count = extras.lists.len();
    for (index, entry) in extras.lists.iter().enumerate() {
        let (icon_button, moves) = super::list_sections::row_controls(u, &entry.id, &entry.name, index == 0, index + 1 == list_count);
        let members = entry.members.len();
        let id = entry.id.clone();
        let id_show = id.clone();
        let id_on = id.clone();
        let id_off = id.clone();
        let id_random = id.clone();
        let random = entry.random_each_launch;
        let name = entry.name.clone();
        // Two lines a list: who it is (icon, name, order, delete), then what it does.
        let top = row()
            .w_full()
            .gap(8.0)
            .center_y()
            .child(icon_button)
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(2.0)
                    .child(text(name.clone()).font(BODY_MED).color(u.ink).truncate())
                    .child(
                        text(if random {
                            crate::screens::settings::i18n::fill(
                                crate::screens::settings::i18n::tr("{} mods, random each launch"),
                                &[&members.to_string()],
                            )
                        } else {
                            format!("{members} mod{}", if members == 1 { "" } else { "s" })
                        })
                        .font(SMALL)
                        .color(u.faint)
                        .truncate(),
                    ),
            )
            .child(moves)
            .child(
                u.icon_button(&icons::TRASH_2, &format!("Delete {name}"))
                    .key(format!("list-delete-{}", entry.id))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| delete_list(st, &id)),
            );
        let actions = row()
            .w_full()
            .gap(8.0)
            .center_y()
            // Deadlock Mod Manager 2.4: one member on at random each launch.
            .child(
                row()
                    .key(format!("list-random-{}", entry.id))
                    .gap(6.0)
                    .center_y()
                    .focusable(true)
                    .role("switch")
                    .checked(random)
                    .label(crate::screens::settings::i18n::tr("One at random each launch"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| toggle_random(st, &id_random))
                    .child(text(crate::screens::settings::i18n::tr("Random")).font(SMALL).color(u.muted).a11y_hidden())
                    .child(u.toggle_mark(random)),
            )
            .child(spacer())
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
            );
        list = list.child(
            col()
                .key(format!("list-{}", entry.id))
                .w_full()
                .py(8.0)
                .gap(6.0)
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(top)
                .child(actions),
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

fn edit_dialog(s: &State, u: &Ui, slot: u16) -> El {
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
        .unwrap_or_else(|| format!("Slot {}", deadlock_core::addons::slot_label(slot)));
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
    match s.installed.notes.error.as_deref().filter(|error| !error.trim().is_empty()) {
        Some(error) => crate::notify::feedback::error_box(u, "notes-error", error).grow(1.0).shrink().min_w(0.0),
        None => rune::el(),
    }
}

// --- handlers ----------------------------------------------------------------

/// Open the editor for a slot with its current name, picture and flag.
pub fn open_edit(st: &mut State, slot: u16) {
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

fn save_edit(st: &mut State, slot: u16) {
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
    st.status = format!("Saved slot {}.", deadlock_core::addons::slot_label(slot));
    close(st);
}

fn add_typed_tags(st: &mut State, slot: u16) {
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

fn remove_tag(st: &mut State, slot: u16, tag: &str) {
    edit_slot(st, slot, |entry| {
        entry.tags.retain(|known| !known.eq_ignore_ascii_case(tag));
    });
    st.installed.notes
        .tags_in
        .retain(|known| !(known.0 == TagKind::Custom && known.1.eq_ignore_ascii_case(tag)));
    st.installed.notes
        .tags_out
        .retain(|known| !(known.0 == TagKind::Custom && known.1.eq_ignore_ascii_case(tag)));
}

/// Turn every mod of a list on or off. One write per mod, one Undo for the lot.
pub fn lists_switch(st: &mut State, id: &str, enable: bool) {
    if crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    let Some(list) = st.installed.notes.extras.list(id) else { return };
    let name = list.name.clone();
    let members = list.members.clone();
    let mut wanted: std::collections::BTreeMap<u16, bool> = std::collections::BTreeMap::new();
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

/// Turn "One at random each launch" on or off for a list. A modded launch then turns one of its
/// members on and the rest off (`screens::launch_extras`).
fn toggle_random(st: &mut State, id: &str) {
    let flip = |extras: &mut mod_extras::Extras| {
        if let Some(list) = extras.lists.iter_mut().find(|l| l.id == id) {
            list.random_each_launch = !list.random_each_launch;
        }
    };
    if st.demo {
        flip(&mut st.installed.notes.extras);
        return;
    }
    let path = extras_file();
    let _ = mod_extras::edit_extras(&path, flip);
    st.installed.notes.extras = mod_extras::read_extras(&path).unwrap_or_default();
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
/// Is the mod in this slot saved? Installed and the Locker share the flag (Grimoire #593).
pub fn is_favorite(st: &State, slot: u16) -> bool {
    addon_at(st, slot).is_some_and(|addon| extra(st, addon).favorite)
}

/// Save a mod or take it out of the saved ones. Saved mods sort first on Installed and in the
/// Locker's General lists.
pub fn toggle_favorite(st: &mut State, slot: u16) {
    let Some(addon) = addon_at(st, slot) else { return };
    let on = !extra(st, addon).favorite;
    let name = crate::screens::installed::display_name(st, addon);
    edit_slot(st, slot, |entry| entry.favorite = on);
    crate::screens::profile_favorites::sync_active(st);
    st.status = if on {
        crate::screens::settings::i18n::fill(crate::screens::settings::i18n::tr("Saved {}."), &[&name])
    } else {
        crate::screens::settings::i18n::fill(crate::screens::settings::i18n::tr("{} is no longer saved."), &[&name])
    };
}

pub fn toggle_pin(st: &mut State, slot: u16) {
    let Some(addon) = addon_at(st, slot) else { return };
    let entry = extra(st, addon);
    let pinned = !entry.pinned;
    edit_slot(st, slot, |entry| entry.pinned = pinned);
    st.status = if pinned {
        format!("Pinned slot {} to the top of the disabled mods.", deadlock_core::addons::slot_label(slot))
    } else {
        format!("Slot {} is no longer pinned.", deadlock_core::addons::slot_label(slot))
    };
}

/// Turn "load first" on or off. On, the mod moves to the lowest slot of its group, where it
/// wins every file it shares with another mod.
pub fn toggle_always_first(st: &mut State, slot: u16) {
    if crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    let Some(addon) = addon_at(st, slot) else { return };
    let entry = extra(st, addon);
    let first = !entry.always_first;
    edit_slot(st, slot, |entry| entry.always_first = first);
    if !first || super::writable_game(st).is_none() {
        if !first {
            st.status = format!("Slot {} no longer loads first.", deadlock_core::addons::slot_label(slot));
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
                st.status = format!("Slot {} loads first from now on.", deadlock_core::addons::slot_label(slot));
                super::rescan(st);
            }
            Err(error) => st.status = error.to_string(),
        },
        Err(error) => st.status = error.to_string(),
    }
}

/// Remember that the player does not want the newer file of this mod.
pub fn ignore_update(st: &mut State, slot: u16, file_id: Option<u64>) {
    edit_slot(st, slot, |entry| entry.ignored_update = file_id.or(Some(0)));
    st.status = "Update ignored. You can ask for it again from the update chip.".to_string();
}

/// Forget an ignored update.
pub fn unignore_update(st: &mut State, slot: u16) {
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


/// The tag the player ranked highest that is not a hero (the hero is in the detail line).
pub fn top_tag(s: &State, addon: &AddonRow) -> Option<String> {
    tags_of(s, addon).into_iter().find(|(kind, _)| *kind != TagKind::Hero).map(|(_, label)| label)
}

