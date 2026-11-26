//! Small pieces every Stats section draws with: table cells, numbers, avatars
//! and the two chart primitives. Nothing here reads state or the network.

use deadlock_core::stats::{HeroPerformance, StatsSnapshot};
use rune::{Color, El, Shape, SurfaceFit, TextStyle, Tx, canvas, col, el, icon, row, text};

use crate::app::State;
use crate::icons;
use crate::ui::{BODY, H1, H2, LABEL, SMALL, Ui};

use super::german::tr;
use super::icon_key;

/// Tabular figures for every number in a column.
pub(super) const TNUM: TextStyle = TextStyle::new(13.0, 420.0).lh(20.0).features("'tnum' 1");
pub(super) const TNUM_MED: TextStyle = TextStyle::new(13.0, 540.0).lh(20.0).features("'tnum' 1");
pub(super) const TNUM_SMALL: TextStyle = TextStyle::new(12.0, 430.0).lh(16.0).features("'tnum' 1");
/// The big number in a stat tile.
pub(super) const TNUM_BIG: TextStyle =
    TextStyle::new(22.0, 600.0).lh(28.0).tracking(-0.2).features("'tnum' 1");

/// The Stats tab row: the shared segmented selector as tabs, eight parts. Live carries a green
/// mark only while a match runs. Every press goes through `nav::set_tab`, so the top place
/// remembers its tab.
pub(super) fn tabs(s: &State, u: &Ui, open: super::Tab, enabled: bool) -> El {
    use super::Tab;
    let running = super::live::running(s) && super::live::in_match(s);
    let mut bar = crate::ui::Segmented::new("stats-tabs", tr("Stats sections")).tabs();
    for tab in Tab::ROW {
        let usable = enabled;
        let mut part = crate::ui::SegPart::new(format!("stats-tab-{tab:?}").to_lowercase(), tr(tab.label())).on(tab == open);
        if tab == Tab::Live && running {
            part = part.mark(crate::ui::switch_green());
        }
        if usable {
            part = part.with(move |e| e.on_press(move |st: &mut State| super::nav::set_tab(st, tab)));
        } else {
            part = part.with(|e| e.opacity(0.5).disabled(true));
        }
        bar = bar.part(part);
    }
    bar.build(u)
}

/// Tabular figures at weight 600: the lobby's best value in a column.
pub(super) const TNUM_BOLD: TextStyle = TextStyle::new(13.0, 600.0).lh(20.0).features("'tnum' 1");

/// A player's picture: their avatar when it is loaded, else the initial in a square.
pub(super) fn player_avatar(s: &State, u: &Ui, account_id: u64, name: &str, size: f32) -> El {
    match s.thumbnails.get(&super::pictures::avatar_key(account_id)) {
        Some(pixels) => el()
            .size(size, size)
            .radius(crate::ui::r(4.0))
            .fill(Color::WHITE.alpha(0.04))
            .pixels_fit(pixels, SurfaceFit::Fill)
            .a11y_hidden(),
        None => avatar(u, name, size),
    }
}

/// A rank badge: the badge art when it is loaded, else a filled tile that still reads as a
/// rank: the tier's initial and the subrank ("E5" for Eternus 5), sized to the tile, so an
/// offline page looks finished. Nothing for an unranked badge.
pub(super) fn rank_badge(s: &State, u: &Ui, badge: u32, size: f32) -> El {
    let Some(key) = super::pictures::rank_key(badge) else {
        return el().size(size, size).a11y_hidden();
    };
    if let Some(pixels) = s.thumbnails.get(&key) {
        return el().size(size, size).pixels_fit(pixels, SurfaceFit::Fit).a11y_hidden();
    }
    let initial = s
        .stats
        .snapshot
        .as_ref()
        .and_then(|snap| snap.tier_names.get(&(badge / 10)))
        .and_then(|name| name.chars().next())
        .map(|c| c.to_uppercase().collect::<String>())
        .unwrap_or_default();
    let font = if size >= 48.0 {
        TextStyle::new(20.0, 650.0).lh(24.0).features("'tnum' 1")
    } else if size >= 28.0 {
        TextStyle::new(13.0, 600.0).lh(16.0).features("'tnum' 1")
    } else {
        TextStyle::new(10.0, 600.0).lh(12.0).features("'tnum' 1")
    };
    el()
        .size(size, size)
        .radius(crate::ui::r(2.0))
        .fill(Color::WHITE.alpha(0.08))
        .border(1.0, u.hair_strong)
        .center()
        .a11y_hidden()
        .child(text(format!("{initial}{}", badge % 10)).font(font).color(u.ink))
}

/// The one clickable player cell every table uses: picture and name. A press opens the
/// player's page; a right press opens Track, Statlocker and Copy id. An account id of 0 (a
/// bot, a hidden profile) is plain text.
pub(super) fn player_link(s: &State, u: &Ui, account_id: u64, name: &str, size: f32, mine: bool) -> El {
    let label = text(name.to_string())
        .font(if mine { crate::ui::BODY_MED } else { BODY })
        .color(u.ink)
        .truncate()
        .min_w(0.0)
        .shrink();
    let mut r = row()
        .key(format!("stats-player-{account_id}-{}", name.len()))
        .min_w(0.0)
        .h(size + 8.0)
        .pr(6.0)
        .gap(8.0)
        .center_y()
        .radius(crate::ui::r(2.0))
        .child(player_avatar(s, u, account_id, name, size))
        .child(label);
    if account_id == 0 {
        return r;
    }
    r = r
        .hover(u.hover)
        .focusable(true)
        .role("link")
        .label(crate::screens::settings::i18n::fill(tr("Open the stats for {}"), &[name]))
        .cursor(rune::Cursor::Pointer)
        .on_press_tx(move |st: &mut State, tx: &mut Tx| super::nav::open_player(st, tx, account_id));
    let anchor = format!("stats-player-{account_id}-{}", name.len());
    r.on_secondary_press(move |st: &mut State, _p, _tx: &mut Tx| {
        st.stats.menu = Some(super::Menu::Person(account_id, anchor.clone()));
    })
}

/// Track, Statlocker and Copy id for one player, under the row that opened it.
pub(super) fn person_menu(s: &State, u: &Ui, account_id: u64, anchor: &str) -> El {
    let tracked = s.stats.remembered.iter().any(|r| r.account_id == account_id);
    let mut menu = u.menu("stats-person-menu", anchor, 220.0);
    if !tracked {
        menu = menu.child(u.menu_item("stats-person-track", &icons::PIN, tr("Track"), false).on_press(move |st: &mut State| {
            st.stats.menu = None;
            let name = st
                .stats
                .snapshot
                .as_ref()
                .filter(|snap| snap.account_id == account_id)
                .map(|snap| snap.profile.persona_name.clone())
                .unwrap_or_else(|| account_id.to_string());
            crate::data::remember_stats_account(st, account_id, &name);
        }));
    }
    menu.child(
        u.menu_item("stats-person-statlocker", &icons::EXTERNAL_LINK, tr("Open on Statlocker"), false).on_press_tx(
            move |st: &mut State, tx: &mut Tx| {
                st.stats.menu = None;
                open_statlocker(st, tx, &statlocker_profile_url(account_id));
            },
        ),
    )
    .child(u.menu_item("stats-person-copy", &icons::COPY, tr("Copy account id"), false).on_press_tx(
        move |st: &mut State, tx: &mut Tx| {
            st.stats.menu = None;
            tx.copy(account_id.to_string());
        },
    ))
}

/// Above a visited player's page: Back, who this is, their badge, Track and Statlocker. Not
/// drawn on your own page.
pub(super) fn context_bar(s: &State, u: &Ui) -> Option<El> {
    if s.stats.nav.stack.len() < 2 || s.stats.nav.match_open().is_some() || s.stats.nav.hero_open().is_some() {
        return None;
    }
    let snap = s.stats.snapshot.as_ref();
    let pending = s.stats.nav.pending;
    let account_id = snap.map(|snap| snap.account_id).or(pending).unwrap_or(0);
    let name = snap.map(|snap| snap.profile.persona_name.clone()).unwrap_or_else(|| account_id.to_string());
    let tracked = s.stats.remembered.iter().any(|r| r.account_id == account_id);
    let home = Some(account_id) == s.stats.home;
    let mut bar = row()
        .key("stats-context")
        .w_full()
        .h(36.0)
        .gap(12.0)
        .center_y()
        .child(back_button(u))
        .child(text(if home { tr("Your page").to_string() } else { crate::screens::settings::i18n::fill(tr("Viewing {}"), &[&name]) }).font(crate::ui::BODY_MED).color(u.ink).truncate().min_w(0.0).shrink());
    if let Some(snap) = snap
        && snap.rank.reported
    {
        bar = bar
            .child(rank_badge(s, u, snap.rank.badge, 20.0))
            .child(text(deadlock_core::stats::insight::badge_label(snap.rank.badge, &snap.tier_names)).font(SMALL).color(u.prose));
    }
    bar = bar.child(el().w(0.0).grow(1.0));
    if !home && snap.is_some() {
        bar = bar.child(if tracked {
            text(tr("Tracked")).font(SMALL).color(u.muted)
        } else {
            u.action(tr("Track"), crate::ui::ActionKind::Secondary)
                .key("stats-context-track")
                .cursor(rune::Cursor::Pointer)
                .on_press(super::nav::track)
        });
    }
    Some(bar.child(statlocker_link(u, "stats-context-statlocker".to_string(), statlocker_profile_url(account_id), tr("Open on Statlocker"))))
}

/// The Back button of the context bar and the match page.
pub(super) fn back_button(u: &Ui) -> El {
    u.icon_action(&icons::ARROW_LEFT, tr("Back"), crate::ui::ActionKind::Ghost)
        .key("stats-back")
        .cursor(rune::Cursor::Pointer)
        .on_press_tx(|st: &mut State, tx: &mut Tx| {
            super::nav::back(st, tx);
        })
}

/// Results as beads, oldest left: a win stands up, a loss hangs down.
pub(super) fn form_beads(u: &Ui, results: &[bool]) -> El {
    let (bead, gap, h) = (6.0_f32, 3.0_f32, 24.0_f32);
    let base = 14.0;
    let mut shape = Shape::new();
    for (index, win) in results.iter().enumerate() {
        let x0 = index as f32 * (bead + gap);
        shape = if *win {
            shape.fill(rect(x0, 2.0, x0 + bead, base - 1.0), u.teal.alpha(0.85))
        } else {
            shape.fill(rect(x0, base + 1.0, x0 + bead, h - 2.0), u.rose.alpha(0.75))
        };
    }
    let width = results.len() as f32 * (bead + gap);
    canvas(shape).size(width.max(1.0), h)
}

/// An item's slot colour: weapon amber, vitality teal, spirit violet.
pub(super) fn slot_color(u: &Ui, slot: &str) -> Color {
    match slot {
        "weapon" => u.amber,
        "vitality" => u.teal,
        "spirit" => u.violet,
        _ => u.hair_strong,
    }
}

/// One item of a final build: its tile, with the buy time on hover.
pub(super) fn item_icon(s: &State, u: &Ui, item: &deadlock_core::stats::board::BuildItem, size: f32) -> El {
    let bought = (item.bought_at > 0).then_some(item.bought_at);
    item_tile(s, u, item.item_id, &item.name, &item.slot, size, bought)
}

/// One item: its icon when loaded, else a tile with the name's first letters, always with a
/// 1 point slot colour edge along the bottom. Hover names it (and when it was bought).
pub(super) fn item_tile(s: &State, u: &Ui, item_id: u64, name: &str, slot: &str, size: f32, bought_at: Option<u32>) -> El {
    let help = match bought_at {
        Some(at) => format!("{name}, {} {}", tr("bought at"), super::notes::clock(at)),
        None => name.to_string(),
    };
    let tile = el()
        .size(size, size)
        .radius(crate::ui::r(2.0))
        .fill(Color::WHITE.alpha(0.05))
        .border_sides([0.0, 0.0, 1.0, 0.0], slot_color(u, slot))
        .help(help)
        .label(name.to_string());
    match s.thumbnails.get(&super::pictures::item_key(item_id)) {
        Some(pixels) => tile.pixels_fit(pixels, SurfaceFit::Fit),
        None => {
            let letters: String = name.split_whitespace().filter_map(|word| word.chars().next()).take(2).collect();
            tile.center().child(text(letters).font(crate::ui::MICRO).color(u.muted).a11y_hidden())
        }
    }
}

/// Win rate with and without a teammate: two thin bars on one 0 to 100 track, each with its
/// number, and the lift in points. A side without a rate reads "-".
pub(super) fn lift_bar(u: &Ui, with: Option<f64>, without: Option<f64>, lift: Option<f64>) -> El {
    let w = 120.0;
    let bar = |rate: Option<f64>, color: Color| {
        let fill = rate.map(|r| (r.clamp(0.0, 100.0) / 100.0) as f32 * w).unwrap_or(0.0);
        el().w(w).h(4.0).fill(Color::WHITE.alpha(0.08)).child(el().absolute(0.0, 0.0).w(fill).h(4.0).fill(color))
    };
    let pct = |rate: Option<f64>| rate.map(|r| format!("{r:.0}%")).unwrap_or_else(|| "-".to_string());
    let line = |label: &str, rate: Option<f64>, color: Color| {
        row()
            .gap(8.0)
            .center_y()
            .child(text(tr(label).to_string()).font(SMALL).color(u.muted).w(56.0))
            .child(bar(rate, color))
            .child(text(pct(rate)).font(TNUM_SMALL).color(u.prose).w(36.0).align(rune::Align::End))
    };
    let mut c = col().gap(4.0).child(line("With", with, u.teal.alpha(0.85))).child(line("Without", without, u.muted));
    if let Some(points) = lift {
        c = c.child(
            text(crate::screens::settings::i18n::fill(tr("{} points with them"), &[&format!("{points:+.0}")]))
                .font(TNUM_SMALL)
                .color(if points >= 3.0 { u.teal } else if points <= -3.0 { u.rose } else { u.muted }),
        );
    }
    c.a11y_hidden()
}

/// Souls a minute of a history row.
pub(super) fn spm(item: &deadlock_core::stats::MatchSummary) -> u64 {
    deadlock_core::stats::board::souls_per_minute(item.net_worth, item.duration_seconds)
}

/// A stat tile: label, big number with an optional sparkline, then two reserved sub lines.
pub(super) fn tile(u: &Ui, key: &str, label: &str, value: &str, spark: Option<El>, line3: El, line4: El) -> El {
    let mut value_row = row()
        .w_full()
        .h(28.0)
        .gap(8.0)
        .items_end()
        .child(text(value.to_string()).font(TNUM_BIG).color(u.ink).truncate().min_w(0.0).shrink())
        .child(el().w(0.0).grow(1.0));
    if let Some(spark) = spark {
        value_row = value_row.child(spark);
    }
    col()
        .key(key.to_string())
        .w(0.0)
        .grow(1.0)
        .min_w(0.0)
        .h(96.0)
        .padding(12.0)
        .gap(0.0)
        .radius(crate::ui::r(4.0))
        .fill(u.raised)
        .border(1.0, u.hair)
        .child(row().h(16.0).child(text(tr(label).to_string()).font(SMALL).color(u.muted).truncate()))
        .child(value_row)
        .child(row().w_full().h(16.0).gap(6.0).center_y().child(line3))
        .child(row().w_full().h(16.0).center_y().child(line4))
}

pub(super) fn avatar(u: &Ui, name: &str, size: f32) -> El {
    let initial = name
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().collect::<String>())
        .unwrap_or_else(|| "?".to_string());
    el().size(size, size)
        .radius(crate::ui::r(4.0))
        .fill(Color::WHITE.alpha(0.08))
        .center()
        .a11y_hidden()
        .child(
            text(initial)
                .font(if size >= 40.0 { H1 } else { LABEL })
                .color(u.prose),
        )
}

pub(super) fn problem(u: &Ui, message: &str) -> El {
    row()
        .w_full()
        .gap(8.0)
        .items_start()
        .child(
            el().pt(3.0).child(
                icon(&icons::TRIANGLE_ALERT, 14.0)
                    .color(u.amber)
                    .a11y_hidden(),
            ),
        )
        .child(
            text(tr(message))
                .font(BODY)
                .color(u.muted)
                .grow(1.0)
                .min_w(0.0)
                .wrap(),
        )
}

/// A failed read with a "Try again" button that runs `retry` (item stats, item meta, a
/// purchase timeline): a network blip should not need a page reload.
pub(super) fn problem_retry(
    u: &Ui,
    message: &str,
    key: &str,
    retry: impl Fn(&mut State) + 'static,
) -> El {
    col().w_full().gap(8.0).child(problem(u, message)).child(
        row().child(
            u.action(tr("Try again"), crate::ui::ActionKind::Secondary)
                .key(key.to_string())
                .cursor(rune::Cursor::Pointer)
                .on_press(retry),
        ),
    )
}

pub(super) fn tier_label(snap: &StatsSnapshot) -> Option<String> {
    snap.peers.as_ref().and_then(|peers| peers.tier_name.clone())
}

/// A 7 point triangle: up or down.
pub(super) fn arrow(color: Color, up: bool) -> El {
    let shape = if up {
        Shape::new().fill(vec![[0.0, 6.0], [7.0, 6.0], [3.5, 0.5]], color)
    } else {
        Shape::new().fill(vec![[0.0, 0.5], [7.0, 0.5], [3.5, 6.0]], color)
    };
    canvas(shape).size(7.0, 7.0).a11y_hidden()
}

pub(super) fn sparkline(u: &Ui, values: &[f64], width: f32, height: f32) -> El {
    if values.len() < 2 {
        return el().size(width, height).a11y_hidden();
    }
    let (lo, hi) = values
        .iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
    let span = (hi - lo).max(1e-6);
    let pad = 3.0;
    let points: Vec<[f32; 2]> = values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let x = index as f32 / (values.len() - 1) as f32 * width;
            let y = pad + (1.0 - ((value - lo) / span) as f32) * (height - 2.0 * pad);
            [x, y]
        })
        .collect();
    let last = *points.last().expect("two or more points");
    let shape = Shape::new()
        .stroke(points, 1.25, u.muted.alpha(0.8))
        .fill(circle(last, 2.0), u.ink);
    canvas(shape).size(width, height).a11y_hidden()
}

pub(super) fn circle(center: [f32; 2], radius: f32) -> Vec<[f32; 2]> {
    (0..12)
        .map(|index| {
            let angle = index as f32 / 12.0 * std::f32::consts::TAU;
            [
                center[0] + radius * angle.cos(),
                center[1] + radius * angle.sin(),
            ]
        })
        .collect()
}

pub(super) fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<[f32; 2]> {
    vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
}

pub(super) fn hero_chip(s: &State, u: &Ui, hero_id: u32, name: &str, size: f32) -> El {
    match s.thumbnails.get(&icon_key(hero_id)) {
        Some(pixels) => el()
            .key(format!("stats-chip-{hero_id}-{size}"))
            .size(size, size)
            .radius(crate::ui::r(6.0))
            .fill(Color::WHITE.alpha(0.04))
            .pixels_fit(pixels, SurfaceFit::Fit)
            .a11y_hidden(),
        None => el()
            .size(size, size)
            .radius(crate::ui::r(6.0))
            .fill(Color::WHITE.alpha(0.06))
            .center()
            .a11y_hidden()
            .child(
                text(name.chars().next().map(String::from).unwrap_or_default())
                    .font(SMALL)
                    .color(u.muted),
            ),
    }
}

pub(super) fn hero_name(hero: &HeroPerformance) -> String {
    hero.hero_name
        .clone()
        .unwrap_or_else(|| format!("Hero {}", hero.hero_id))
}

pub(super) fn compare_chip(u: &Ui, label: &str, better: bool) -> El {
    row()
        .gap(6.0)
        .center_y()
        .child(arrow(if better { u.teal } else { u.rose }, better))
        .child(text(label.to_string()).font(TNUM_SMALL).color(u.prose))
}

pub(super) fn capitalise(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// `+1,560` or `-700`.
pub(super) fn signed(n: i64) -> String {
    let sign = if n > 0 { "+" } else if n < 0 { "-" } else { "" };
    format!("{sign}{}", thousands(n.unsigned_abs()))
}

pub(super) fn souls(n: u64) -> String {
    if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

pub(super) fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, c) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub(super) fn section_head(u: &Ui, title: &str, meta: &str) -> El {
    row()
        .w_full()
        .h(32.0)
        .gap(12.0)
        .center_y()
        .child(text(tr(title)).font(H2).color(u.ink))
        .child(text(tr(meta)).font(SMALL).color(u.muted))
}

/// A table column: a fixed width, or 0 to take the rest.
pub(super) fn cell(e: El, w: f32) -> El {
    if w > 0.0 {
        e.w(w)
    } else {
        e.grow(1.0).min_w(0.0)
    }
}

pub(super) fn head_row(u: &Ui, cols: &[(&str, f32, bool)]) -> El {
    let mut r = row()
        .w_full()
        .h(28.0)
        .gap(16.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair);
    for (label, w, end) in cols {
        let mut t = text(tr(label)).font(SMALL).color(u.muted);
        if *end {
            t = t.align(rune::Align::End);
        }
        // Match rows start with a 3 point result edge; the head's first label clears it.
        if *label == "Result" {
            t = t.pl(11.0);
        }
        r = r.child(cell(t, *w));
    }
    r
}

pub(super) fn data_row(key: String, u: &Ui, h: f32) -> El {
    row()
        .key(key)
        .w_full()
        .h(h)
        .gap(16.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
}

pub(super) fn num(value: String, w: f32, color: Color) -> El {
    cell(text(value).font(TNUM).color(color).align(rune::Align::End), w)
}

// --- Statlocker -----------------------------------------------------------------

/// A player's page on Statlocker (Grimoire's `statlocker.ts`, MIT). A link the browser opens;
/// Sigil never calls Statlocker itself.
pub(super) fn statlocker_profile_url(account_id: u64) -> String {
    format!("https://statlocker.gg/profile/{account_id}")
}

/// A match's page on Statlocker.
pub(super) fn statlocker_match_url(match_id: u64) -> String {
    format!("https://statlocker.gg/match/{match_id}")
}

/// Hand a Statlocker page to the browser. A failure becomes a toast.
pub(super) fn open_statlocker(st: &mut State, tx: &mut Tx, url: &str) {
    if let Err(error) = tx.open_url(url) {
        st.status = crate::screens::settings::i18n::fill(tr("The page could not be opened: {}"), &[&error.to_string()]);
    }
}

/// A small link button that opens `url` on Statlocker.
pub(super) fn statlocker_link(u: &Ui, key: String, url: String, label: &str) -> El {
    u.icon_button(&icons::EXTERNAL_LINK, label)
        .key(key)
        .role("link")
        .cursor(rune::Cursor::Pointer)
        .on_press_tx(move |st: &mut State, tx: &mut Tx| open_statlocker(st, tx, &url))
}
