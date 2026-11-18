//! Mod lists as sections, with an icon each and an order (Grimoire's list icons and order, Deadlock
//! Mod Manager 2.0's groups as sections).
//!
//! - Deadlock Mod Manager: groups shown as sections with covers, turn a group on or off.
//! - Grimoire: an icon per list and drag to reorder lists.
//! - Sigil: View > Sections by list shows one section per mod list, in the lists' order, with its
//!   icon, count and Turn on / Turn off, then "Not in a list". In the Mod lists dialog each list
//!   gets an icon (press to pick the next one) and Move up / Move down. Icons live in
//!   `<data>/mod-list-icons.json`; the order is the store's own list order (`mod_extras`).

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use deadlock_core::mod_extras;
use rune::{El, Icon, col, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::model::AddonRow;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{ActionKind, BODY_MED, SMALL, Ui};

/// The icons a list can wear, in the order a press steps through them.
pub const CHOICES: [(&str, &Icon); 10] = [
    ("layers", &icons::LAYERS),
    ("bookmark", &icons::BOOKMARK),
    ("heart", &icons::HEART),
    ("sparkles", &icons::SPARKLES),
    ("crosshair", &icons::CROSSHAIR),
    ("shirt", &icons::SHIRT),
    ("zap", &icons::ZAP),
    ("shield", &icons::SHIELD),
    ("globe", &icons::GLOBE),
    ("box", &icons::BOX),
];

fn store() -> &'static Mutex<Option<BTreeMap<String, String>>> {
    static STORE: OnceLock<Mutex<Option<BTreeMap<String, String>>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(None))
}

fn path() -> std::path::PathBuf {
    crate::data::data_dir().join("mod-list-icons.json")
}

fn icons_by_list() -> BTreeMap<String, String> {
    let mut guard = match store().lock() {
        Ok(guard) => guard,
        Err(_) => return BTreeMap::new(),
    };
    if guard.is_none() {
        let read = if rune::is_headless() || cfg!(test) {
            BTreeMap::new()
        } else {
            std::fs::read_to_string(path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
        };
        *guard = Some(read);
    }
    guard.clone().unwrap_or_default()
}

/// The icon a list wears: its pick, else the first choice.
pub fn icon_of(id: &str) -> &'static Icon {
    let name = icons_by_list().get(id).cloned().unwrap_or_default();
    CHOICES.iter().find(|(n, _)| *n == name).map(|(_, i)| *i).unwrap_or(CHOICES[0].1)
}

/// The next icon after `current`, wrapping.
pub fn next_choice(current: &str) -> &'static str {
    let at = CHOICES.iter().position(|(n, _)| *n == current).map_or(0, |i| (i + 1) % CHOICES.len());
    CHOICES[at].0
}

pub fn cycle_icon(st: &mut State, id: &str) {
    let mut map = icons_by_list();
    let next = next_choice(map.get(id).map(String::as_str).unwrap_or(CHOICES[0].0));
    map.insert(id.to_string(), next.to_string());
    if !st.demo && !rune::is_headless()
        && let Ok(text) = serde_json::to_string_pretty(&map)
    {
        let _ = std::fs::write(path(), text);
    }
    if let Ok(mut guard) = store().lock() {
        *guard = Some(map);
    }
}

/// The list order after `id` moves by `delta`, clamped.
pub fn moved(ids: &[String], id: &str, delta: isize) -> Vec<String> {
    let mut out = ids.to_vec();
    if let Some(at) = out.iter().position(|x| x == id) {
        let to = (at as isize + delta).clamp(0, out.len() as isize - 1) as usize;
        let item = out.remove(at);
        out.insert(to, item);
    }
    out
}

pub fn move_list(st: &mut State, id: &str, delta: isize) {
    let reorder = |extras: &mut mod_extras::Extras| {
        let ids: Vec<String> = extras.lists.iter().map(|l| l.id.clone()).collect();
        let order = moved(&ids, id, delta);
        extras.lists.sort_by_key(|l| order.iter().position(|x| *x == l.id).unwrap_or(usize::MAX));
    };
    if st.demo {
        reorder(&mut st.installed.notes.extras);
        return;
    }
    let path = super::notes::extras_file();
    let _ = mod_extras::edit_extras(&path, reorder);
    st.installed.notes.extras = mod_extras::read_extras(&path).unwrap_or_default();
}

/// The icon button and the two move buttons a row of the Mod lists dialog gets.
pub fn row_controls(u: &Ui, id: &str, name: &str, first: bool, last: bool) -> (El, El) {
    let (a, b, c) = (id.to_string(), id.to_string(), id.to_string());
    let icon_button = u
        .icon_button(icon_of(id), &fill(tr("Change the icon of {}"), &[name]))
        .key(format!("list-icon-{id}"))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| cycle_icon(st, &a));
    let up = u.icon_button(&icons::ARROW_UP, &fill(tr("Move {} up"), &[name])).key(format!("list-up-{id}"));
    let down = u.icon_button(&icons::CHEVRON_DOWN, &fill(tr("Move {} down"), &[name])).key(format!("list-down-{id}"));
    let moves = row()
        .gap(2.0)
        .child(if first { up.disabled(true).opacity(0.35) } else { up.cursor(rune::Cursor::Pointer).on_press(move |st: &mut State| move_list(st, &b, -1)) })
        .child(if last { down.disabled(true).opacity(0.35) } else { down.cursor(rune::Cursor::Pointer).on_press(move |st: &mut State| move_list(st, &c, 1)) });
    (icon_button, moves)
}

/// The page body as sections by list. `cards` draws a set of addons as the page's cards or rows.
pub fn body<'a>(s: &State, u: &Ui, shown: &[&'a AddonRow], cards: impl Fn(&[&'a AddonRow]) -> El) -> El {
    let extras = &s.installed.notes.extras;
    let mut out = col().w_full().gap(24.0).pb(32.0);
    let mut listed: std::collections::BTreeSet<u16> = Default::default();
    for list in &extras.lists {
        let members: Vec<&AddonRow> = shown
            .iter()
            .copied()
            .filter(|addon| list.members.iter().any(|uid| extras.slot_for_uid(uid) == Some(addon.slot)))
            .collect();
        if members.is_empty() {
            continue;
        }
        listed.extend(members.iter().map(|a| a.slot));
        let on = members.iter().filter(|a| a.enabled).count();
        let (id_on, id_off) = (list.id.clone(), list.id.clone());
        let head = row()
            .key(format!("section-list-{}", list.id))
            .w_full()
            .h(32.0)
            .gap(8.0)
            .center_y()
            .child(rune::icon(icon_of(&list.id), 16.0).color(u.muted).a11y_hidden())
            .child(text(fill(tr("{} ({})"), &[&list.name, &members.len().to_string()])).font(BODY_MED).color(u.muted))
            .child(text(fill(tr("{} on"), &[&on.to_string()])).font(SMALL).color(u.faint))
            .child(spacer())
            .child(
                u.action(tr("Turn on"), ActionKind::Ghost)
                    .key(format!("section-list-on-{}", list.id))
                    .disabled(on == members.len())
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| super::notes::lists_switch(st, &id_on, true)),
            )
            .child(
                u.action(tr("Turn off"), ActionKind::Ghost)
                    .key(format!("section-list-off-{}", list.id))
                    .disabled(on == 0)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| super::notes::lists_switch(st, &id_off, false)),
            );
        out = out.child(col().w_full().gap(12.0).child(head).child(cards(&members)));
    }
    let rest: Vec<&AddonRow> = shown.iter().copied().filter(|a| !listed.contains(&a.slot)).collect();
    if !rest.is_empty() {
        let head = row()
            .key("section-list-none")
            .w_full()
            .h(32.0)
            .gap(8.0)
            .center_y()
            .child(text(fill(tr("Not in a list ({})"), &[&rest.len().to_string()])).font(BODY_MED).color(u.muted));
        out = out.child(col().w_full().gap(12.0).child(head).child(cards(&rest)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_move_and_icons_cycle() {
        let ids: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        assert_eq!(moved(&ids, "c", -1), vec!["a", "c", "b"]);
        assert_eq!(moved(&ids, "a", -1), ids, "the top stays the top");
        assert_eq!(moved(&ids, "a", 5), vec!["b", "c", "a"]);
        assert_eq!(next_choice("layers"), "bookmark");
        assert_eq!(next_choice("box"), "layers");
        assert_eq!(next_choice("unknown"), "layers");
    }
}
