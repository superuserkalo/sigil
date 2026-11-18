//! The overlap badge on Installed cards (Deadlock Mod Manager's quiet grey "Overlap" badge, with
//! Grimoire's list of who overlaps): a mark on an enabled mod that ships a game path another
//! enabled mod also ships. Its press opens a short list of those mods with the files each pair
//! shares and who wins (the lower slot loads first and wins), and a way on to Overlap.
//!
//! The data is the scan's conflict rows (`AddonLibrary::conflicts`), minus the overlaps the
//! player ignored on the Overlap page; nothing reads a VPK here. [`refresh`] runs with
//! Installed's poll and keeps the result in `View::overlaps`, so a frame only looks it up. A mod
//! whose model another mod hides shows the hidden-model mark instead, as in DMM.

use std::collections::{BTreeMap, BTreeSet};

use rune::{Color, El, col, el, icon, row, text};

use crate::app::State;
use crate::icons;
use crate::model::{ConflictRow, Screen};
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{BODY, MICRO, SMALL, Ui};

use super::{Menu, display_name};

/// For each enabled slot that overlaps: the other enabled slots it shares paths with, and how
/// many paths each pair shares.
pub type Overlaps = BTreeMap<u16, BTreeMap<u16, usize>>;

/// Who overlaps whom. `hidden` says whether a row is ignored on the Overlap page.
pub fn by_slot(conflicts: &[ConflictRow], enabled: &BTreeSet<u16>, hidden: impl Fn(&ConflictRow) -> bool) -> Overlaps {
    let mut out = Overlaps::new();
    for row in conflicts {
        if hidden(row) {
            continue;
        }
        let on: Vec<u16> = row.slots.iter().copied().filter(|slot| enabled.contains(slot)).collect();
        for a in &on {
            for b in &on {
                if a != b {
                    *out.entry(*a).or_default().entry(*b).or_default() += 1;
                }
            }
        }
    }
    out
}

/// The winner of a pair: the lower slot loads first, and the game keeps its copy.
pub fn winner(a: u16, b: u16) -> u16 {
    a.min(b)
}

/// Recompute after a scan or an ignore change. Cheap: the rows are already in memory.
pub fn refresh(st: &mut State) -> bool {
    let enabled: BTreeSet<u16> = st.addons.addons.iter().filter(|a| a.enabled).map(|a| a.slot).collect();
    let ignores = crate::screens::conflicts::ignore_list();
    let fresh = by_slot(&st.addons.conflicts, &enabled, |row| ignores.hides(&row.path, &row.slots, row.winner));
    if fresh != st.installed.overlaps {
        st.installed.overlaps = fresh;
        return true;
    }
    false
}

fn name_of(s: &State, slot: u16) -> String {
    s.addons
        .addons
        .iter()
        .find(|a| a.slot == slot)
        .map(|a| display_name(s, a))
        .unwrap_or_else(|| format!("{slot:02}"))
}

/// The words the badge carries for a screen reader, and the line the popover heads with.
pub fn summary(s: &State, slot: u16) -> Option<String> {
    let others = s.installed.overlaps.get(&slot)?;
    let names: Vec<String> = others.keys().map(|other| name_of(s, *other)).collect();
    Some(fill(tr("Shares files with {}. The lower slot loads first and wins."), &[&names.join(", ")]))
}

/// The badge: on an image corner for cards, or as a small chip in a list row. None when the mod
/// does not overlap, or its model is hidden (that mark says more).
pub fn badge(s: &State, u: &Ui, slot: u16, on_image: bool) -> Option<El> {
    if s.installed.hidden.contains(&slot) {
        return None;
    }
    let label = summary(s, slot)?;
    let open = s.installed.menu == Some(Menu::Overlap(slot));
    let base = el()
        .key(format!("installed-overlap-{slot:02}"))
        .focusable(true)
        .role("button")
        .label(label)
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| super::open_menu(st, Menu::Overlap(slot)));
    Some(if on_image {
        base.size(28.0, 28.0)
            .center()
            .radius(crate::ui::r(8.0))
            .fill(Color::BLACK.alpha(if open { 0.78 } else { 0.6 }))
            .hover(Color::BLACK.alpha(0.78))
            .child(icon(&icons::LAYERS, 14.0).color(Color::WHITE.alpha(0.72)).a11y_hidden())
    } else {
        base.size(24.0, 24.0)
            .center()
            .radius(crate::ui::r(6.0))
            .fill(if open { u.selected } else { Color::TRANSPARENT })
            .border(1.0, u.hair_strong)
            .hover(u.hover)
            .child(icon(&icons::LAYERS, 13.0).color(u.muted).a11y_hidden())
    })
}

/// The popover rows: who this mod overlaps, how many files, who wins, then Overlap.
pub fn menu_items(s: &State, u: &Ui, slot: u16) -> Vec<El> {
    let Some(others) = s.installed.overlaps.get(&slot) else {
        return Vec::new();
    };
    let mut items = vec![
        col()
            .w_full()
            .px(10.0)
            .pt(8.0)
            .pb(6.0)
            .child(
                text(tr("Shares files with other mods. The lower slot loads first and wins."))
                    .font(SMALL)
                    .color(u.faint)
                    .wrap(),
            ),
    ];
    for (other, files) in others {
        let theirs = winner(slot, *other) == *other;
        let note = if theirs { tr("Wins") } else { tr("This one wins") };
        let count = if *files == 1 {
            tr("1 file").to_string()
        } else {
            fill(tr("{} files"), &[&files.to_string()])
        };
        items.push(
            row()
                .key(format!("overlap-with-{other:02}"))
                .w_full()
                .h(36.0)
                .px(10.0)
                .gap(10.0)
                .center_y()
                .child(text(format!("{other:02}")).font(MICRO).color(u.faint).w(18.0))
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .child(text(name_of(s, *other)).font(BODY).color(u.ink).truncate())
                        .child(text(count).font(SMALL).color(u.faint)),
                )
                .child(text(note.to_string()).font(SMALL).color(if theirs { u.amber } else { u.muted })),
        );
    }
    items.push(super::menu_rule(u));
    items.push(
        super::menu_item(u, "menu-overlap-open", Some(&icons::GIT_COMPARE), tr("Review in Overlap"), "", true).on_press(
            |st: &mut State| {
                st.installed.menu = None;
                st.go(Screen::Conflicts);
            },
        ),
    );
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(path: &str, slots: &[u16]) -> ConflictRow {
        ConflictRow { path: path.into(), slots: slots.to_vec(), winner: *slots.iter().min().unwrap() }
    }

    #[test]
    fn every_enabled_pair_counts_its_shared_files() {
        let conflicts = vec![
            row("materials/a.vmat_c", &[2, 5]),
            row("materials/b.vtex_c", &[2, 5, 9]),
            row("sounds/c.vsnd_c", &[3, 4]),
        ];
        let enabled: BTreeSet<u16> = [2, 3, 4, 5].into_iter().collect();
        let found = by_slot(&conflicts, &enabled, |_| false);
        assert_eq!(found[&2].iter().map(|(k, v)| (*k, *v)).collect::<Vec<_>>(), vec![(5, 2)], "9 is off");
        assert_eq!(found[&5][&2], 2);
        assert_eq!(found[&3][&4], 1);
        assert!(!found.contains_key(&9), "an off mod has no badge");
    }

    #[test]
    fn ignored_rows_leave_no_badge() {
        let conflicts = vec![row("materials/a.vmat_c", &[2, 5]), row("sounds/c.vsnd_c", &[3, 4])];
        let enabled: BTreeSet<u16> = [2, 3, 4, 5].into_iter().collect();
        let found = by_slot(&conflicts, &enabled, |r| r.path.starts_with("sounds/"));
        assert!(found.contains_key(&2) && found.contains_key(&5));
        assert!(!found.contains_key(&3) && !found.contains_key(&4));
    }

    #[test]
    fn the_lower_slot_wins() {
        assert_eq!(winner(7, 3), 3);
        assert_eq!(winner(3, 7), 3);
    }
}
