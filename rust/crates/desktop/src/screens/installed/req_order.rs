//! The requirement load-order fix (Grimoire's "Recolor won't show: the base skin is hiding it.
//! Fix it", MIT). A mod whose GameBanana page lists another mod as a requirement usually
//! overrides part of it, a recolor over a base skin. When the dependent loads after its base
//! (a higher slot), the base wins every shared path and the dependent does nothing.
//!
//! `deadlock_core::requirements::order_problems` finds those pairs from the requirement pages
//! Installed already read (`enable::Gates::known`) and returns the moves that put the dependent
//! directly before its base. The card shows an amber mark; its popover names the base and offers
//! "Fix it", which goes through the same plan, renames, library remap and Undo as a drag
//! (`installed::apply_moves`). A base kept "always first" cannot be passed; the popover says so.
//! Deadlock Mod Manager has no such check.

use std::collections::BTreeMap;
use std::path::PathBuf;

use deadlock_core::requirements::{ModRequirements, OrderProblem, OrderedMod, order_problems, order_problems_with_files};
use rune::{Color, El, col, el, icon, row, text};


use crate::app::State;
use crate::icons;
use crate::jobs::Job;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{SMALL, Ui};

use super::{Menu, display_name, meta_for, notes};

/// Grimoire's texture-overlay narrowing (C6): the game paths of the mods in a problem, by
/// GameBanana id, with the file size they were read at. Read on a worker, never per frame.
#[derive(Default)]
pub struct Files {
    pub lists: BTreeMap<u64, (u64, Vec<String>)>,
    /// Ids already sent to the worker at this size, so a broken file is read once.
    pub tried: BTreeMap<u64, u64>,
    pub job: Job<Vec<(u64, u64, Vec<String>)>>,
    /// Bumped when lists arrive; with the plain problems it keys the last narrowed answer.
    pub generation: usize,
    pub last: Option<(Vec<OrderProblem>, usize, Vec<OrderProblem>)>,
}

/// Work the problems out again from the library, the extras and the requirement pages read so
/// far. True when they changed. Called in Installed's poll; it walks memory only, and starts a
/// worker for the file lists the narrowing still needs.
pub fn refresh(st: &mut State) -> bool {
    if let Some(Ok(found)) = st.installed.order_files.job.try_take() {
        for (id, size, list) in found {
            st.installed.order_files.lists.insert(id, (size, list));
        }
        st.installed.order_files.generation += 1;
    }
    let mods: Vec<OrderedMod> = st
        .addons
        .addons
        .iter()
        .filter_map(|addon| {
            let id = meta_for(st, addon)?.gamebanana_id?;
            Some(OrderedMod {
                gamebanana_id: id,
                slot: addon.slot,
                enabled: addon.enabled,
                always_first: notes::extra(st, addon).always_first,
            })
        })
        .collect();
    let requirements: Vec<ModRequirements> = mods
        .iter()
        .filter(|m| m.enabled)
        .filter_map(|m| {
            let (_, list) = st.installed.gates.known.get(&m.gamebanana_id)?;
            Some(ModRequirements::new(Some(m.gamebanana_id), list.clone()))
        })
        .collect();
    let plain = order_problems(&mods, &requirements);
    let fresh = narrow(st, &mods, &requirements, plain);
    if fresh != st.installed.order_problems {
        st.installed.order_problems = fresh;
        return true;
    }
    false
}

/// Keep only the pairs where the dependent really overlays its base's textures, once both file
/// lists are read; until then (or for a file that cannot be read) the plain slot check stands.
fn narrow(st: &mut State, mods: &[OrderedMod], requirements: &[ModRequirements], plain: Vec<OrderProblem>) -> Vec<OrderProblem> {
    if plain.is_empty() {
        return plain;
    }
    if let Some((seen, generation, answer)) = &st.installed.order_files.last
        && *seen == plain
        && *generation == st.installed.order_files.generation
        && !st.installed.order_files.job.is_running()
    {
        return answer.clone();
    }
    let dir = super::addons_folder(st);
    let size_of = |slot: u16| st.addons.addons.iter().find(|a| a.slot == slot).map(|a| (a.size_bytes, a.path(&dir)));
    let mut needed: Vec<(u64, u64, PathBuf)> = Vec::new();
    for problem in &plain {
        for (id, slot) in [(problem.overlay, problem.overlay_slot), (problem.base, problem.base_slot)] {
            if let Some((size, path)) = size_of(slot) {
                needed.push((id, size, path));
            }
        }
    }
    let files = &mut st.installed.order_files;
    let missing: Vec<(u64, u64, PathBuf)> = needed
        .iter()
        .filter(|(id, size, _)| files.lists.get(id).is_none_or(|(at, _)| at != size))
        .filter(|(id, size, _)| files.tried.get(id) != Some(size))
        .cloned()
        .collect();
    if !missing.is_empty() && !files.job.is_running() && !st.demo && !rune::is_headless() {
        for (id, size, _) in &missing {
            files.tried.insert(*id, *size);
        }
        files.job.start(st.wake.clone(), move || {
            let paths: Vec<(u64, PathBuf)> = missing.iter().map(|(id, _, path)| (*id, path.clone())).collect();
            let lists = deadlock_core::requirements::file_lists(&paths);
            Ok(missing
                .into_iter()
                .filter_map(|(id, size, _)| lists.get(&id).map(|list| (id, size, list.clone())))
                .collect())
        });
    }
    let current: BTreeMap<u64, Vec<String>> = needed
        .iter()
        .filter_map(|(id, size, _)| {
            let (at, list) = files.lists.get(id)?;
            (at == size).then(|| (*id, list.clone()))
        })
        .collect();
    let answer = if current.is_empty() {
        plain.clone()
    } else {
        order_problems_with_files(mods, requirements, &current)
    };
    if !files.job.is_running() {
        files.last = Some((plain, files.generation, answer.clone()));
    }
    answer
}

/// The first problem where `slot` is the dependent.
pub fn problem_for(s: &State, slot: u16) -> Option<&OrderProblem> {
    s.installed.order_problems.iter().find(|p| p.overlay_slot == slot)
}

fn name_of(s: &State, slot: u16) -> String {
    s.addons
        .addons
        .iter()
        .find(|a| a.slot == slot)
        .map(|a| display_name(s, a))
        .unwrap_or_else(|| format!("{slot:02}"))
}

/// "X won't show: Y is hiding it."
pub fn line(s: &State, problem: &OrderProblem) -> String {
    fill(
        tr("{} won't show: {} loads first and hides it."),
        &[&name_of(s, problem.overlay_slot), &name_of(s, problem.base_slot)],
    )
}

/// The amber mark on the dependent's preview corner, or a chip in a list row.
pub fn badge(s: &State, u: &Ui, slot: u16, on_image: bool) -> Option<El> {
    let problem = problem_for(s, slot)?;
    let open = s.installed.menu == Some(Menu::Order(slot));
    let base = el()
        .key(format!("installed-order-{slot:02}"))
        .focusable(true)
        .role("button")
        .label(line(s, problem))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| super::open_menu(st, Menu::Order(slot)));
    Some(if on_image {
        base.size(28.0, 28.0)
            .center()
            .radius(crate::ui::r(8.0))
            .fill(Color::BLACK.alpha(if open { 0.78 } else { 0.6 }))
            .hover(Color::BLACK.alpha(0.78))
            .child(icon(&icons::ARROW_UP, 14.0).color(u.amber).a11y_hidden())
    } else {
        base.size(24.0, 24.0)
            .center()
            .radius(crate::ui::r(6.0))
            .fill(if open { u.selected } else { Color::TRANSPARENT })
            .border(1.0, u.amber.alpha(0.45))
            .hover(u.hover)
            .child(icon(&icons::ARROW_UP, 13.0).color(u.amber).a11y_hidden())
    })
}

/// The popover: what is wrong, and Fix it (or why it cannot be fixed by a move).
pub fn menu_items(s: &State, u: &Ui, slot: u16) -> Vec<El> {
    let Some(problem) = problem_for(s, slot) else {
        return Vec::new();
    };
    let mut items = vec![
        col()
            .w_full()
            .px(10.0)
            .pt(8.0)
            .pb(6.0)
            .gap(4.0)
            .child(text(line(s, problem)).font(crate::ui::BODY_MED).color(u.ink).wrap())
            .child(
                text(if problem.fixable {
                    tr("Its GameBanana page lists the other mod as a requirement. It has to load first to show.")
                } else {
                    tr("The base is set to load first, so nothing can go above it. Turn Loads first off on the base, then fix it here.")
                })
                .font(SMALL)
                .color(u.faint)
                .wrap(),
            ),
    ];
    if problem.fixable {
        let moves = problem.moves.clone();
        items.push(super::menu_rule(u));
        items.push(
            super::menu_item(
                u,
                "menu-order-fix",
                Some(&icons::ARROW_UP),
                tr("Fix it: load it before the base"),
                "",
                true,
            )
            .on_press(move |st: &mut State| {
                st.installed.menu = None;
                super::apply_moves(st, moves.clone());
            }),
        );
    }
    items.push(row().h(4.0));
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use deadlock_core::requirements::Requirement;

    #[test]
    fn a_recolor_after_its_base_is_found_and_moved_before_it() {
        let mods = vec![
            OrderedMod { gamebanana_id: 10, slot: 2, enabled: true, always_first: false },
            OrderedMod { gamebanana_id: 20, slot: 7, enabled: true, always_first: false },
        ];
        let needs = vec![ModRequirements::new(
            Some(20),
            vec![Requirement {
                name: "Base".into(),
                url: "https://gamebanana.com/mods/10".into(),
                mod_id: Some(10),
            }],
        )];
        let found = order_problems(&mods, &needs);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].overlay_slot, found[0].base_slot), (7, 2));
        assert!(found[0].fixable && !found[0].moves.is_empty());
    }
}
