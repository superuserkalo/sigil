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

/// The Stats tab row: 32 high over a hairline, the open tab underlined in ink. Live keeps
/// room for its "running" mark so the tabs after it never move.
pub(super) fn tabs(s: &State, u: &Ui, open: super::Tab, enabled: bool) -> El {
    use super::Tab;
    let running = super::live::running(s);
    let mut r = row()
        .key("stats-tabs")
        .w_full()
        .h(32.0)
        .role("tablist")
        .label(tr("Stats sections"))
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair);
    for tab in Tab::ALL {
        let on = tab == open;
        let usable = enabled || tab == Tab::FunRun;
        let mut t = row()
            .key(format!("stats-tab-{tab:?}").to_lowercase())
            .h(32.0)
            .px(12.0)
            .gap(6.0)
            .center_y()
            .role("tab")
            .checked(on)
            .label(tr(tab.label()).to_string())
            .opacity(if usable { 1.0 } else { 0.5 })
            .child(
                text(tr(tab.label()).to_string())
                    .font(crate::ui::BODY_MED)
                    .color(if on { u.ink } else { u.muted })
                    .a11y_hidden(),
            );
        if tab == Tab::Live {
            // Drawn in the tab's own padding, so a running match never widens the tab.
            t = t.child(
                el().absolute(2.0, 13.0)
                    .size(6.0, 6.0)
                    .radius(1.0)
                    .fill(if running { crate::ui::switch_green() } else { Color::TRANSPARENT })
                    .a11y_hidden(),
            );
        }
        t = t.child(
            el().absolute(0.0, 30.0)
                .w_full()
                .h(2.0)
                .fill(if on { u.ink } else { Color::TRANSPARENT })
                .a11y_hidden(),
        );
        if usable {
            t = t
                .focusable(true)
                .hover(u.hover)
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| st.stats.tab = tab)
                .on_key(move |st: &mut State, ev: &rune::KeyEvent, _| {
                    let i = Tab::ALL.iter().position(|x| *x == st.stats.tab).unwrap_or(0);
                    let next = match ev.key {
                        rune::Key::Left => i.saturating_sub(1),
                        rune::Key::Right => (i + 1).min(Tab::ALL.len() - 1),
                        _ => return false,
                    };
                    st.stats.tab = Tab::ALL[next];
                    true
                });
        }
        r = r.child(t);
    }
    r
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

pub(super) fn fact(u: &Ui, label: &str, value: String) -> El {
    col()
        .gap(2.0)
        .child(text(tr(label)).font(SMALL).color(u.muted))
        .child(text(value).font(TNUM).color(u.prose))
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
