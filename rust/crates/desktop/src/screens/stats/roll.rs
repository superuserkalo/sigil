//! Fun run: a random hero, lane, twelve item build, ability order and three house rules,
//! from one seed (`deadlock_core::stats::roll`). The code (`DLR-...`) brings the same roll
//! back on any Sigil, and Copy build puts a plain text version on the clipboard for a chat.
//!
//! Behaviour after Deadlock Mod Manager's hero build randomizer (GPL-3.0, behaviour only);
//! Grimoire has none. The asset lists come from `api.deadlock-api.com` on the first Roll and
//! are cached for a week; nothing about the player is sent.

use std::collections::HashMap;

use deadlock_core::stats::roll::{self as core, Category, Code, Roll, RollData};
use rune::{Color, El, Key, KeyEvent, SurfaceFit, TextEdit, Tx, col, el, row, spacer, text};

use crate::app::State;
use crate::jobs::Job;
use crate::screens::settings::i18n::fill;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, SMALL, Ui};

use super::german::tr;
use super::shared::*;
use super::Fetched;

/// The roll section's state, kept in Stats' view.
pub struct RollView {
    pub data: Fetched<RollData>,
    pub job: Job<(RollData, Vec<(String, Vec<u8>)>)>,
    pub roll: Option<Roll>,
    /// What to roll once the lists arrive.
    pub pending: Option<Code>,
    pub lock_hero: bool,
    pub show_build: bool,
    pub show_abilities: bool,
    pub show_rules: bool,
    pub code: TextEdit,
    pub note: Option<String>,
}

impl Default for RollView {
    fn default() -> Self {
        RollView {
            data: Fetched::Idle,
            job: Job::default(),
            roll: None,
            pending: None,
            lock_hero: false,
            show_build: true,
            show_abilities: true,
            show_rules: true,
            code: TextEdit::new(""),
            note: None,
        }
    }
}

/// The house rule a roll drew, in the player's language. Sigil's own list.
fn rule_text(index: usize) -> &'static str {
    match index {
        0 => tr("Buy the build in the order shown. No swaps."),
        1 => tr("No trips to the base shop after the 15 minute mark."),
        2 => tr("Parry every melee you see coming."),
        3 => tr("No ziplines for the first 10 minutes."),
        4 => tr("Melee only until your first tier 2 item."),
        5 => tr("Cast your ultimate the moment it is ready."),
        6 => tr("Never back out of a fight you started."),
        7 => tr("Never sit on more than 1,000 unspent souls."),
        8 => tr("Stay in your lane until your Guardian falls."),
        9 => tr("Ping before every fight. No voice chat."),
        10 => tr("Post this roll's code in chat when the match starts."),
        _ => tr("No healing at base: urns, items and teammates only."),
    }
}

fn focus_label(category: Category) -> &'static str {
    match category {
        Category::Weapon => tr("Gun build"),
        Category::Vitality => tr("Tank build"),
        Category::Spirit => tr("Spirit build"),
    }
}

fn category_label(category: Category) -> &'static str {
    match category {
        Category::Weapon => tr("Weapon"),
        Category::Vitality => tr("Vitality"),
        Category::Spirit => tr("Spirit"),
    }
}

fn lane_label(lane: core::Lane) -> &'static str {
    match lane {
        core::Lane::Solo => tr("Solo lane"),
        core::Lane::Duo => tr("Duo lane"),
    }
}

fn category_color(category: Category) -> Color {
    match category {
        Category::Weapon => rune::oklch(0.76, 0.14, 70.0),
        Category::Vitality => rune::oklch(0.74, 0.14, 150.0),
        Category::Spirit => rune::oklch(0.70, 0.15, 305.0),
    }
}

fn rgb(c: [u8; 3]) -> Color {
    Color {
        r: f32::from(c[0]) / 255.0,
        g: f32::from(c[1]) / 255.0,
        b: f32::from(c[2]) / 255.0,
        a: 1.0,
    }
}

fn souls(value: u32) -> String {
    let text = value.to_string();
    let mut out = String::new();
    for (index, digit) in text.chars().enumerate() {
        if index > 0 && (text.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

pub fn icon_key(hero_id: u32) -> String {
    format!("stats-roll-hero:{hero_id}")
}

// --- actions -----------------------------------------------------------------------------------

/// Roll with a fresh seed, keeping the hero when it is locked.
pub fn reroll(st: &mut State) {
    let hero = if st.stats.roll.lock_hero {
        st.stats.roll.roll.as_ref().map(|roll| roll.hero_id)
    } else {
        None
    };
    roll_code(st, Code { seed: core::fresh_seed(), hero });
}

/// Roll one code: now when the lists are here, after the load otherwise.
pub fn roll_code(st: &mut State, code: Code) {
    let r = &mut st.stats.roll;
    if let Fetched::Ready(data) = &r.data {
        match core::roll(data, code) {
            Ok(roll) => {
                r.roll = Some(roll);
                r.note = None;
            }
            Err(error) => r.note = Some(error.to_string()),
        }
        return;
    }
    r.pending = Some(code);
    start_load(st);
}

fn start_load(st: &mut State) {
    if st.stats.roll.job.is_running() {
        return;
    }
    if st.prefs.offline {
        st.stats.roll.data = Fetched::Failed(tr("Sigil is offline. The hero and item lists need one download first.").to_string());
        return;
    }
    st.stats.roll.data = Fetched::Loading;
    let wake = st.wake.clone();
    st.stats.roll.job.start(wake, move || {
        let data = crate::data::stats_client().fetch_roll_data().map_err(|error| error.to_string())?;
        let icons = hero_icons(&data);
        Ok((data, icons))
    });
}

/// Hero portraits from the local art (`hero_portraits/chip-icons`), like Stats' own chips.
fn hero_icons(data: &RollData) -> Vec<(String, Vec<u8>)> {
    let assets = deadlock_core::locker::LockerAssetPaths::discover();
    let Some(dir) = assets.hero_portraits_dir.map(|dir| dir.join("chip-icons")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for hero in &data.heroes {
        let key = deadlock_core::locker::hero_asset_key(&hero.name);
        for file in [format!("{key}.png"), format!("{key}.webp"), format!("the_{key}.png"), format!("the_{key}.webp")] {
            if let Ok(bytes) = std::fs::read(dir.join(&file)) {
                out.push((icon_key(hero.id), bytes));
                break;
            }
        }
    }
    out
}

/// Bring back the roll a pasted code names.
fn load_code(st: &mut State) {
    let pasted = st.stats.roll.code.text.clone();
    match Code::find(&pasted) {
        Some(code) => {
            st.stats.roll.lock_hero = code.hero.is_some();
            roll_code(st, code);
        }
        None => st.stats.roll.note = Some(tr("That is not a roll code. Codes look like DLR-1A2B3C.").to_string()),
    }
}

/// Take the finished list load. Called from Stats' poll.
pub fn poll(st: &mut State) -> bool {
    let Some(result) = st.stats.roll.job.try_take() else {
        return false;
    };
    match result {
        Ok((data, icons)) => {
            st.pending_thumbs.extend(icons);
            st.stats.roll.data = Fetched::Ready(data);
            if let Some(code) = st.stats.roll.pending.take() {
                roll_code(st, code);
            }
        }
        Err(error) => {
            st.stats.roll.pending = None;
            st.stats.roll.data = Fetched::Failed(error);
        }
    }
    true
}

// --- view --------------------------------------------------------------------------------------

fn pill(u: &Ui, key: &str, label: &str, on: bool) -> El {
    row()
        .key(key.to_string())
        .h(28.0)
        .px(10.0)
        .center_y()
        .radius(crate::ui::r(6.0))
        .fill(if on { u.selected } else { Color::TRANSPARENT })
        .hover(u.hover)
        .focusable(true)
        .role("checkbox")
        .checked(on)
        .label(label.to_string())
        .cursor(rune::Cursor::Pointer)
        .child(text(label.to_string()).font(SMALL).color(if on { u.ink } else { u.muted }).a11y_hidden())
}

fn hero_chip(s: &State, u: &Ui, hero_id: u32, name: &str, size: f32) -> El {
    match s.thumbnails.get(&icon_key(hero_id)) {
        Some(pixels) => el()
            .size(size, size)
            .radius(crate::ui::r(8.0))
            .fill(Color::WHITE.alpha(0.04))
            .pixels_fit(pixels, SurfaceFit::Fit)
            .a11y_hidden(),
        None => el()
            .size(size, size)
            .radius(crate::ui::r(8.0))
            .fill(Color::WHITE.alpha(0.06))
            .center()
            .a11y_hidden()
            .child(text(name.chars().next().map(String::from).unwrap_or_default()).font(H1).color(u.muted)),
    }
}

fn controls(s: &State, u: &Ui) -> El {
    let r = &s.stats.roll;
    let busy = r.job.is_running();
    let mut bar = row().key("stats-roll-controls").w_full().gap(8.0).center_y().flex_wrap();
    bar = bar.child(
        u.action(if r.roll.is_some() { tr("Reroll") } else { tr("Roll a fun run") }, ActionKind::Secondary)
            .key("stats-roll-go")
            .opacity(if busy { 0.5 } else { 1.0 })
            .cursor(rune::Cursor::Pointer)
            .on_press(reroll),
    );
    if r.roll.is_some() {
        bar = bar.child(
            row()
                .key("stats-roll-lock")
                .gap(8.0)
                .center_y()
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| st.stats.roll.lock_hero = !st.stats.roll.lock_hero)
                .child(u.toggle(r.lock_hero).label(tr("Keep the hero")))
                .child(text(tr("Keep the hero")).font(SMALL).color(u.muted).a11y_hidden()),
        );
    }
    bar = bar.child(spacer()).child(
        row()
            .w(220.0)
            .h(32.0)
            .px(10.0)
            .center_y()
            .radius(crate::ui::r(8.0))
            .fill(u.well)
            .border(1.0, u.hair)
            .child(
                rune::text_input(&r.code, BODY)
                    .key("stats-roll-code")
                    .label(tr("Paste a roll code"))
                    .single_line()
                    .grow(1.0)
                    .min_w(0.0)
                    .color(u.ink)
                    .placeholder(tr("Paste a roll code"), u.faint)
                    .caret(u.accent, u.accent.alpha(0.20))
                    .on_edit(|st: &mut State| &mut st.stats.roll.code)
                    .on_key(|st: &mut State, ev: &KeyEvent, _| {
                        if ev.key == Key::Enter {
                            load_code(st);
                            true
                        } else {
                            false
                        }
                    }),
            ),
    );
    bar.child(
        u.action(tr("Open code"), ActionKind::Ghost)
            .key("stats-roll-open")
            .cursor(rune::Cursor::Pointer)
            .on_press(load_code),
    )
}

fn header(s: &State, u: &Ui, roll: &Roll, data: &RollData) -> El {
    let hero = data.hero(roll.hero_id);
    let accent = hero.map(|hero| rgb(hero.accent)).unwrap_or(u.ink);
    let mut facts = vec![lane_label(roll.lane).to_string()];
    if let Some(hero) = hero {
        facts.push(fill(tr("complexity {} of 3"), &[&hero.complexity.to_string()]));
        if !hero.tags.is_empty() {
            facts.push(hero.tags.join(", "));
        }
    }
    let code = roll.code.text();
    let share = roll.share_text(data);
    let code_copy = code.clone();
    row()
        .key("stats-roll-head")
        .w_full()
        .gap(16.0)
        .center_y()
        .child(hero_chip(s, u, roll.hero_id, &roll.hero_name, 64.0))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(text(tr("You are playing")).font(SMALL).color(u.faint))
                .child(text(roll.hero_name.clone()).font(H1).color(accent))
                .children(
                    hero.filter(|hero| !hero.role.is_empty())
                        .map(|hero| text(hero.role.clone()).font(BODY).color(u.prose).truncate()),
                )
                .child(text(facts.join(" · ")).font(SMALL).color(u.muted).truncate()),
        )
        .child(
            col()
                .gap(6.0)
                .items_end()
                .child(text(code.clone()).font(TNUM_MED).color(u.ink))
                .child(
                    row()
                        .gap(6.0)
                        .child(
                            u.action(tr("Copy code"), ActionKind::Ghost)
                                .key("stats-roll-copy-code")
                                .cursor(rune::Cursor::Pointer)
                                .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                                    tx.copy(code_copy.clone());
                                    crate::notify::push(st, crate::notify::Notice::info(fill(tr("Copied {} to the clipboard"), &[&code_copy])));
                                }),
                        )
                        .child(
                            u.action(tr("Copy build"), ActionKind::Secondary)
                                .key("stats-roll-copy-build")
                                .cursor(rune::Cursor::Pointer)
                                .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                                    tx.copy(share.clone());
                                    crate::notify::push(st, crate::notify::Notice::info(tr("Copied the build to the clipboard")));
                                }),
                        ),
                ),
        )
}

fn focus_strip(u: &Ui, roll: &Roll) -> El {
    let total = roll.total().max(1) as f32;
    let mut bar = row().w_full().h(6.0).radius(3.0).fill(u.hair);
    for category in Category::ALL {
        let part = roll.souls[category.index()] as f32 / total;
        if part > 0.0 {
            bar = bar.child(el().h_full().grow(part).fill(category_color(category)));
        }
    }
    let mut legend = row().w_full().gap(16.0).center_y();
    for category in Category::ALL {
        legend = legend.child(
            row()
                .gap(6.0)
                .center_y()
                .child(el().size(8.0, 8.0).radius(4.0).fill(category_color(category)).a11y_hidden())
                .child(
                    text(format!("{} {}", category_label(category), souls(roll.souls[category.index()])))
                        .font(TNUM_SMALL)
                        .color(u.muted),
                ),
        );
    }
    legend = legend.child(spacer()).child(
        text(fill(
            tr("{}: {} souls of {}"),
            &[focus_label(roll.focus()), &souls(roll.total()), &souls(core::BUDGET)],
        ))
        .font(SMALL)
        .color(u.faint),
    );
    col().w_full().gap(8.0).child(bar).child(legend)
}

fn item_tile(u: &Ui, data: &RollData, roll: &Roll, index: usize, item: &core::RolledItem) -> El {
    let color = category_color(item.category);
    let mut detail = fill(tr("Tier {}, {} souls"), &[&item.tier.to_string(), &souls(item.cost)]);
    if item.active {
        detail.push_str(&format!(" · {}", tr("active")));
    }
    let mut tile = col()
        .key(format!("stats-roll-item-{index}"))
        .w(176.0)
        .gap(2.0)
        .padding(10.0)
        .radius(crate::ui::r(8.0))
        .fill(u.raised)
        .border_sides([0.0, 0.0, 0.0, 3.0], color)
        .child(
            row()
                .w_full()
                .gap(6.0)
                .center_y()
                .child(text(format!("{}", index + 1)).font(TNUM_SMALL).color(u.faint))
                .child(text(item.name.clone()).font(BODY_MED).color(u.ink).truncate().grow(1.0).min_w(0.0)),
        )
        .child(text(detail).font(TNUM_SMALL).color(u.muted));
    if let (Some(slot), Some(hero)) = (item.imbue_slot, data.hero(roll.hero_id)) {
        let ability = data.ability_name(&hero.abilities[usize::from(slot.clamp(1, 4) - 1)]);
        tile = tile.child(text(fill(tr("Imbue on {}"), &[&ability])).font(SMALL).color(color).truncate());
    }
    if item.recommended {
        tile = tile.child(text(tr("A pick the hero's draft favours")).font(SMALL).color(u.faint).truncate());
    }
    tile
}

fn build(u: &Ui, data: &RollData, roll: &Roll) -> El {
    let mut grid = row().key("stats-roll-build").w_full().gap(8.0).flex_wrap();
    for (index, item) in roll.items.iter().enumerate() {
        grid = grid.child(item_tile(u, data, roll, index, item));
    }
    col()
        .w_full()
        .gap(8.0)
        .child(text(tr("Build, in buying order")).font(BODY_MED).color(u.ink))
        .child(grid)
        .children((roll.items.len() < core::PER_CATEGORY * 3).then(|| {
            text(tr("The hero's draft list ran short, so some slots stayed empty."))
                .font(SMALL)
                .color(u.faint)
        }))
}

fn abilities(u: &Ui, data: &RollData, roll: &Roll) -> El {
    let Some(hero) = data.hero(roll.hero_id) else {
        return el();
    };
    let mut steps: HashMap<u8, Vec<usize>> = HashMap::new();
    for (index, step) in roll.abilities.iter().enumerate() {
        steps.entry(step.slot).or_default().push(index + 1);
    }
    let mut list = col().key("stats-roll-abilities").w_full().gap(4.0).child(
        text(tr("Ability points, in order")).font(BODY_MED).color(u.ink),
    );
    for slot in 1..=4u8 {
        let name = data.ability_name(&hero.abilities[usize::from(slot - 1)]);
        let lead = slot == roll.lead_ability;
        let mut marks = row().gap(4.0).center_y();
        for step in 1..=roll.abilities.len() {
            let on = steps.get(&slot).is_some_and(|list| list.contains(&step));
            marks = marks.child(
                el().size(22.0, 22.0)
                    .radius(crate::ui::r(4.0))
                    .fill(if on { u.accent.alpha(if lead { 0.9 } else { 0.55 }) } else { u.hair.alpha(0.5) })
                    .center()
                    .child(
                        text(if on { step.to_string() } else { String::new() })
                            .font(TNUM_SMALL)
                            .color(if on { u.accent_ink } else { u.faint })
                            .a11y_hidden(),
                    ),
            );
        }
        let mut label = row()
            .w(220.0)
            .gap(6.0)
            .center_y()
            .child(text(name).font(BODY).color(if lead { u.ink } else { u.prose }).truncate());
        if slot == 4 {
            label = label.child(text(tr("Ultimate")).font(SMALL).color(u.faint));
        }
        if lead {
            label = label.child(text(tr("max first")).font(SMALL).color(u.accent));
        }
        list = list.child(row().key(format!("stats-roll-ability-{slot}")).w_full().h(30.0).gap(12.0).center_y().child(label).child(marks));
    }
    list
}

fn rules(u: &Ui, roll: &Roll) -> El {
    let mut list = col().key("stats-roll-rules").w_full().gap(6.0).child(text(tr("House rules")).font(BODY_MED).color(u.ink));
    for (index, rule) in roll.rules.iter().enumerate() {
        list = list.child(
            row()
                .w_full()
                .gap(10.0)
                .items_start()
                .child(text(format!("{}", index + 1)).font(TNUM_SMALL).color(u.faint).w(12.0))
                .child(text(rule_text(*rule)).font(BODY).color(u.prose).grow(1.0).min_w(0.0).wrap()),
        );
    }
    list
}

fn section_pills(s: &State, u: &Ui) -> El {
    let r = &s.stats.roll;
    row()
        .gap(2.0)
        .padding(2.0)
        .radius(crate::ui::r(8.0))
        .border(1.0, u.hair)
        .child(pill(u, "stats-roll-show-build", tr("Build"), r.show_build).on_press(|st: &mut State| st.stats.roll.show_build = !st.stats.roll.show_build))
        .child(
            pill(u, "stats-roll-show-abilities", tr("Abilities"), r.show_abilities)
                .on_press(|st: &mut State| st.stats.roll.show_abilities = !st.stats.roll.show_abilities),
        )
        .child(pill(u, "stats-roll-show-rules", tr("Rules"), r.show_rules).on_press(|st: &mut State| st.stats.roll.show_rules = !st.stats.roll.show_rules))
}

/// The Fun run section. Shown with or without a loaded player.
pub(super) fn section(s: &State, u: &Ui) -> El {
    let r = &s.stats.roll;
    let mut section = col().key("stats-roll").w_full().gap(12.0).child(
        row()
            .w_full()
            .gap(12.0)
            .center_y()
            .child(section_head(u, "Fun run", "A random hero, build and ability order. Share the code and race a friend"))
            .child(spacer())
            .children(r.roll.is_some().then(|| section_pills(s, u))),
    );
    section = section.child(controls(s, u));
    if let Some(note) = &r.note {
        section = section.child(problem(u, note));
    }
    match (&r.data, &r.roll) {
        (Fetched::Loading, _) => section.child(text(tr("Reading the hero and item lists")).font(BODY).color(u.muted)),
        (Fetched::Failed(error), _) => section.child(problem(u, error)),
        (Fetched::Ready(data), Some(roll)) => {
            let mut card = col()
                .w_full()
                .gap(16.0)
                .padding(16.0)
                .radius(crate::ui::r(12.0))
                .border(1.0, u.hair)
                .child(header(s, u, roll, data))
                .child(focus_strip(u, roll));
            if r.show_build {
                card = card.child(build(u, data, roll));
            }
            if r.show_abilities {
                card = card.child(abilities(u, data, roll));
            }
            if r.show_rules {
                card = card.child(rules(u, roll));
            }
            section.child(card).child(
                text(fill(
                    tr("{} items in this hero's draft list. Hero, item and ability data from deadlock-api.com."),
                    &[&roll.pool.to_string()],
                ))
                .font(SMALL)
                .color(u.faint),
            )
        }
        _ => section,
    }
}

/// `stats-roll`: a fixed roll from the captured lists.
pub fn stage_scene(s: &mut State) {
    let data = core::data_from_bundle(include_str!("../../../fixtures/stats-roll.json")).expect("roll fixture parses");
    s.stats.roll.data = Fetched::Ready(data);
    roll_code(s, Code { seed: 0x5161_1A, hero: None });
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_fixture_rolls_a_full_build() {
        let data = deadlock_core::stats::roll::data_from_bundle(include_str!("../../../fixtures/stats-roll.json")).unwrap();
        assert!(data.heroes.len() >= 6);
        let roll = deadlock_core::stats::roll::roll(&data, deadlock_core::stats::roll::Code { seed: 1, hero: None }).unwrap();
        assert_eq!(roll.items.len(), 12);
        assert!(roll.total() <= deadlock_core::stats::roll::BUDGET);
    }

    #[test]
    fn souls_group_thousands() {
        assert_eq!(super::souls(34_000), "34,000");
        assert_eq!(super::souls(800), "800");
        assert_eq!(super::souls(1_234_567), "1,234,567");
    }
}
