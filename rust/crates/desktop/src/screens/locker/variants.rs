//! Skins and ability sounds: the rows on a hero page and under an expanded list row.
//!
//! A hero's skins load in slot order and the lowest enabled slot wins a shared path, so
//! "Equip" turns one on and the rest of that hero's mods off. The arrows move a skin up or
//! down that hero's own order and leave every other mod's slot alone. Both are game writes
//! through `deadlock_core::locker`, never here.

use std::collections::BTreeMap;

use rune::{El, col, el, row, text};

use crate::app::State;
use crate::icons;
use crate::ui::{ActionKind, SMALL, Ui};

use super::{hero_sounds, mod_row, HeroCard, ModRow, View};

fn enabled(s: &State, m: &ModRow) -> bool {
    s.addons
        .addons
        .iter()
        .find(|addon| addon.slot == m.slot)
        .map(|addon| addon.enabled)
        .unwrap_or(m.enabled)
}

fn hero_mods<'a>(v: &'a View, hero: &HeroCard) -> Vec<&'a ModRow> {
    let mut rows: Vec<&ModRow> = v
        .mods
        .iter()
        .filter(|m| m.heroes.iter().any(|name| name == &hero.display_name))
        .collect();
    rows.sort_by_key(|m| m.slot);
    rows
}

fn step_button(
    u: &Ui,
    glyph: &'static rune::Icon,
    label: String,
    key: String,
    active: bool,
    press: impl Fn(&mut State) + 'static,
) -> El {
    let button = u
        .icon_button(glyph, &label)
        .key(key)
        .cursor(rune::Cursor::Pointer)
        .on_press(press);

    if active {
        button
    } else {
        button.disabled(true).opacity(0.30)
    }
}

/// The skins of one hero, in load order. `full` adds the order arrows; the list view's
/// expanded body only needs the switches. `preview` adds Preview in 3D (the hero page; with
/// its 3D view on the arrows give way to keep the names readable).
pub(super) fn skins_list(s: &State, u: &Ui, v: &View, hero: &HeroCard, full: bool, preview: bool) -> El {
    let mods = hero_mods(v, hero);
    let equipped = mods
        .iter()
        .filter(|m| enabled(s, m))
        .map(|m| m.slot)
        .min();
    let last = mods.len().saturating_sub(1);
    let mut rows = col().w_full();

    for (index, m) in mods.iter().enumerate() {
        let slot = m.slot;
        let name = hero.display_name.clone();
        let mut trailing = row().gap(2.0).center_y();

        if full {
            let up_name = name.clone();
            let up = step_button(
                u,
                &icons::ARROW_UP,
                format!("Move {} earlier", m.label),
                format!("locker-up-{}", m.slot),
                index > 0,
                move |st: &mut State| {
                    crate::data::move_locker_skin(st, &up_name, slot, index.saturating_sub(1))
                },
            );
            let down_name = name.clone();
            let down = step_button(
                u,
                &icons::CHEVRON_DOWN,
                format!("Move {} later", m.label),
                format!("locker-down-{}", m.slot),
                index < last,
                move |st: &mut State| {
                    crate::data::move_locker_skin(st, &down_name, slot, index + 1)
                },
            );
            trailing = trailing.child(up).child(down);
        }

        if preview {
            // Preview this skin on the hero in 3D, on or off, against the hero without it.
            let (label, hero_name) = (m.label.clone(), hero.display_name.clone());
            trailing = trailing.child(
                u.icon_button(&icons::BOX, &crate::screens::settings::i18n::fill(crate::screens::settings::i18n::tr("Preview {} in 3D"), &[&m.label]))
                    .key(format!("locker-3d-{}", m.slot))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| crate::viewer::dialog::open_on(st, slot, &label, &hero_name)),
            );
        }

        let equip_name = name.clone();
        trailing = trailing.child(state_slot(
            u,
            Some(slot) == equipped,
            "Equipped",
            u.action_m("Equip", ActionKind::Ghost)
                .key(format!("locker-equip-{}", m.slot))
                .label(format!("Equip {} for {}", m.label, hero.display_name))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    crate::data::equip_locker_skin(st, &equip_name, slot)
                }),
        ));

        rows = rows.child(mod_row(s, u, m, Some(trailing)));
    }

    rows
}

/// The fixed 72 point state slot: "Equip", or the state word with a teal mark, so the
/// switch and the buttons beside it never move between rows.
fn state_slot(u: &Ui, on: bool, word: &str, equip: El) -> El {
    row().w(72.0).h(28.0).center_y().justify_end().child(if on {
        row()
            .gap(6.0)
            .center_y()
            .pr(4.0)
            .child(u.mark(u.teal))
            .child(text(word.to_string()).font(SMALL).color(u.muted))
    } else {
        equip
    })
}

/// The ability slot a sound label names: 1 to 3, 4 for the ultimate, 0 for anything else.
fn ability_label(ability: u8) -> String {
    match ability {
        1 => "Ability 1".to_string(),
        2 => "Ability 2".to_string(),
        3 => "Ability 3".to_string(),
        4 => "Ultimate".to_string(),
        _ => "Other sounds".to_string(),
    }
}

/// One hero's ability sound mods, grouped by the ability they replace. One sound plays per
/// ability, so each group offers Equip beside the switch.
pub(super) fn sounds_list(s: &State, u: &Ui, v: &View, hero: &HeroCard) -> El {
    let rows = hero_sounds(v, hero);
    let mut groups: BTreeMap<u8, Vec<&ModRow>> = BTreeMap::new();

    for m in rows {
        let ability = m
            .sounds
            .iter()
            .find(|sound| sound.hero == hero.display_name)
            .map(|sound| sound.ability)
            .unwrap_or(0);
        groups.entry(ability).or_default().push(m);
    }

    let mut out = col().w_full().gap(16.0);

    for (ability, list) in groups {
        let equipped = list
            .iter()
            .filter(|m| enabled(s, m))
            .map(|m| m.slot)
            .min();
        let mut group = col()
            .w_full()
            .child(el().pb(8.0).child(u.section_head(&ability_label(ability), Some(list.len()), None)));

        for m in list {
            let slot = m.slot;
            let name = hero.display_name.clone();
            let trailing = row().gap(2.0).child(state_slot(
                u,
                Some(slot) == equipped,
                "Playing",
                u.action_m("Equip", ActionKind::Ghost)
                    .key(format!("locker-sound-equip-{}", m.slot))
                    .label(format!("Equip {} for {}", m.label, ability_label(ability)))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        crate::data::equip_locker_sound(st, &name, ability, slot)
                    }),
            ));
            group = group.child(mod_row(s, u, m, Some(trailing)));
        }

        out = out.child(group);
    }

    out
}

/// A muted line for a hero with no ability sound mods, used by the expanded list row.
#[allow(dead_code)]
pub(super) fn no_sounds(u: &Ui) -> El {
    row()
        .w_full()
        .h(44.0)
        .px(12.0)
        .center_y()
        .child(
            text("No ability sounds installed")
                .font(SMALL)
                .color(u.faint),
        )
}
