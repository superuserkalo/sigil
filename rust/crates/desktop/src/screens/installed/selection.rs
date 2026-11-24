//! What the selection can do beyond on, off and delete: group the picked files as one mod
//! (Grimoire's "Group and add variants") and file them in a Locker category (Grimoire's bulk
//! Locker tag).
//!
//! - Grimoire: group two or more local mods as one card, add files to an existing group; tag the
//!   selection for a hero or General in the Locker.
//! - Deadlock Mod Manager: mod groups, no Locker tag.
//! - Sigil: the selection bar's More menu. Group as one mod writes one `variant_group` for every
//!   picked file and the files already grouped with them, so picking a group card and a new file
//!   adds the file to it. Add to a Locker category files the picked mods in one of the Locker's
//!   own categories (General page), by the content key the Locker reads.

use rune::El;

use crate::app::State;
use crate::icons;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::Ui;

/// Every slot that shares a variant card with one of `picked`, `picked` first.
fn with_groups(st: &State, picked: &[u16]) -> Vec<u16> {
    let key_of = |slot: u16| {
        st.addons.addons.iter().find(|a| a.slot == slot).and_then(|addon| {
            let extra = super::notes::extra(st, addon);
            let id = super::meta_for(st, addon).and_then(|m| m.gamebanana_id);
            deadlock_core::mod_extras::variant_key(Some(&extra), id)
        })
    };
    let keys: Vec<String> = picked.iter().filter_map(|slot| key_of(*slot)).collect();
    let mut out: Vec<u16> = picked.to_vec();
    for addon in &st.addons.addons {
        if !out.contains(&addon.slot) && key_of(addon.slot).is_some_and(|k| keys.contains(&k)) {
            out.push(addon.slot);
        }
    }
    out
}

/// The group name: one the picked files already share, else a new one from the first file.
pub fn group_name(existing: &[Option<String>], first_uid: &str) -> String {
    existing
        .iter()
        .flatten()
        .find(|name| !name.trim().is_empty() && !name.starts_with("alone-"))
        .cloned()
        .unwrap_or_else(|| format!("group-{first_uid}"))
}

/// Group the picked files (and their current card mates) as one mod card.
pub fn group(st: &mut State) {
    st.installed.menu = None;
    let picked: Vec<u16> = st.installed.selected.iter().copied().collect();
    if picked.len() < 2 {
        return;
    }
    let slots = with_groups(st, &picked);
    let existing: Vec<Option<String>> = slots
        .iter()
        .filter_map(|slot| st.addons.addons.iter().find(|a| a.slot == *slot))
        .map(|addon| super::notes::extra(st, addon).variant_group)
        .collect();
    let first_uid = st
        .addons
        .addons
        .iter()
        .find(|a| a.slot == slots[0])
        .map(|addon| super::notes::extra(st, addon).uid)
        .unwrap_or_else(|| slots[0].to_string());
    let name = group_name(&existing, &first_uid);
    for slot in &slots {
        let name = name.clone();
        super::notes::edit_slot(st, *slot, move |entry| entry.variant_group = Some(name));
    }
    st.installed.selected.clear();
    st.status = fill(tr("Grouped {} files as one mod."), &[&slots.len().to_string()]);
}

/// File the picked mods in a Locker category.
fn add_to_category(st: &mut State, id: &str) {
    st.installed.menu = None;
    let keys: Vec<String> = st
        .installed
        .selected
        .iter()
        .filter_map(|slot| st.locker.mods.iter().find(|m| m.slot == *slot).map(|m| m.key.clone()))
        .collect();
    let mut next = st.locker.categories.clone();
    for key in &keys {
        next = deadlock_core::locker::add_category_membership(&next, id, key);
    }
    crate::data::save_locker_categories(st, next);
    let name = st.locker.categories.iter().find(|c| c.id == id).map(|c| c.name.clone()).unwrap_or_default();
    st.status = fill(tr("Added {} mods to {} in the Locker."), &[&keys.len().to_string(), &name]);
}

/// The selection's More menu items.
pub fn menu_items(s: &State, u: &Ui) -> Vec<El> {
    let mut items = Vec::new();
    let count = s.installed.selected.len();
    items.push(
        super::menu_item(
            u,
            "selection-make-pack",
            Some(&icons::PACKAGE),
            &crate::screens::packs::from_selection_label(count),
            "",
            count > 0,
        )
        .on_press(|st: &mut State| {
            let slots: Vec<u16> = st.installed.selected.iter().copied().collect();
            crate::screens::packs::open_create(st, crate::screens::packs::Source::Selection(slots), None);
        }),
    );
    items.push(
        super::menu_item(u, "selection-group", Some(&icons::LAYERS), tr("Group as one mod"), "", count >= 2)
            .on_press(group),
    );
    items.push(super::menu_rule(u));
    if !s.locker.loaded {
        items.push(
            super::menu_item(u, "selection-locker-load", Some(&icons::FOLDER_OPEN), tr("Read the Locker's categories"), "", true)
                .on_press(|st: &mut State| crate::data::start_locker_load(st)),
        );
    } else if s.locker.categories.is_empty() {
        items.push(super::menu_item(u, "selection-locker-none", None, tr("No Locker categories yet"), tr("make one on the Locker's General page"), false));
    } else {
        for category in &s.locker.categories {
            let id = category.id.clone();
            items.push(
                super::menu_item(
                    u,
                    &format!("selection-locker-{}", category.id),
                    Some(&icons::PLUS),
                    &fill(tr("Add to {} in the Locker"), &[&category.name]),
                    "",
                    count > 0,
                )
                .on_press(move |st: &mut State| add_to_category(st, &id)),
            );
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_group_takes_an_existing_name_first() {
        assert_eq!(group_name(&[None, Some("haze-set".into())], "u1"), "haze-set");
        assert_eq!(group_name(&[None, Some("alone-u2".into())], "u1"), "group-u1");
        assert_eq!(group_name(&[], "u1"), "group-u1");
    }
}
