//! Game index: search the exact paths the game ships, filter by hero, type and VPK.

use rune::{Color, El, Key, KeyEvent, TextEdit, col, el, row, text};

use crate::app::State;
use crate::icons;
use crate::model::{IndexRow, Screen};
use crate::screens::subpage;
use crate::ui::{ActionKind, BODY, MONO, SMALL, Ui};

const HEROES: [&str; 3] = ["Infernus", "Lash", "Vindicta"];
const TYPES: [&str; 5] = ["material", "model", "particle", "sound", "layout"];

const VPK_W: f32 = 160.0;
const SIZE_W: f32 = 72.0;
const TYPE_W: f32 = 72.0;
const ACTION_W: f32 = 72.0;

pub fn view(s: &State, u: &Ui) -> El {
    let actions = row().gap(8.0).center_y().child(
        u.icon_action(&icons::REFRESH_CW, "Rebuild", ActionKind::Secondary)
            .key("index-rebuild")
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| {
                st.status = "Rebuilding the index is not available yet.".to_string();
            }),
    );

    let detail = match &s.catalog {
        Some(c) => format!(
            "{} citadel files, {} core files, {} map VPKs. Indexed {}.",
            format_count(c.citadel),
            format_count(c.core),
            c.maps,
            c.indexed_at
        ),
        None => String::new(),
    };

    let tabs = crate::screens::explorer::tabs(s, u);
    let body = if s.explorer.tab == crate::screens::explorer::Tab::Files {
        col().w_full().gap(16.0).child(tabs).child(crate::screens::explorer::view(s, u))
    } else if s.catalog.is_none() {
        col().w_full().gap(16.0).child(tabs).child(u.empty_state(
            "",
            "No game index yet",
            "Set the game folder in Settings, then build the index.",
            Some(
                u.action("Open Settings", ActionKind::Secondary)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| st.go(Screen::Settings)),
            ),
        ))
    } else {
        col()
            .w_full()
            .gap(16.0)
            .child(tabs)
            .child(toolbar(s, u))
            .child(table(s, u))
            .child(source(s, u))
    };

    subpage::page(
        "index",
        subpage::head(u, "Game index", &detail, Some(actions)),
        body,
    )
}

fn toolbar(s: &State, u: &Ui) -> El {
    let field = rune::text_input(&s.index_query, BODY)
        .key("index-search")
        .label("Search game paths")
        .single_line()
        .color(u.ink)
        .placeholder("Search paths", u.faint)
        .caret(u.accent, u.accent.alpha(0.22))
        .on_edit(|st: &mut State| &mut st.index_query)
        .on_input_key(|st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                st.index_query = TextEdit::new("");
                true
            } else {
                false
            }
        });

    let mut heroes = vec![
        crate::ui::SegPart::new(format!("index-heroes-{}", "All heroes"), "All heroes").on(s.hero_filter.is_none()).with(move |e| e.on_press(|st: &mut State| st.hero_filter = None)),
    ];
    for hero in HEROES {
        heroes.push(
            crate::ui::SegPart::new(format!("index-heroes-{}", hero), hero).on(s.hero_filter.as_deref() == Some(hero)).with(move |e| e.on_press(move |st: &mut State| st.hero_filter = Some(hero.to_string()))),
        );
    }

    let mut types = vec![
        crate::ui::SegPart::new(format!("index-types-{}", "All types"), "All types").on(s.type_filter.is_none()).with(move |e| e.on_press(|st: &mut State| st.type_filter = None)),
    ];
    for kind in TYPES {
        types.push(
            crate::ui::SegPart::new(format!("index-types-{}", &capital(kind)), &capital(kind)).on(s.type_filter.as_deref() == Some(kind)).with(move |e| e.on_press(move |st: &mut State| st.type_filter = Some(kind.to_string()))),
        );
    }

    let vpks = vec![
        crate::ui::SegPart::new(format!("index-vpks-{}", "All VPKs"), "All VPKs").on(s.vpk_filter.is_none()).with(move |e| e.on_press(|st: &mut State| st.vpk_filter = None)),
        crate::ui::SegPart::new(format!("index-vpks-{}", "pak01"), "pak01").on(s.vpk_filter.as_deref() == Some("pak01")).with(move |e| e.on_press(|st: &mut State| st.vpk_filter = Some("pak01".to_string()))),
        crate::ui::SegPart::new(format!("index-vpks-{}", "Maps"), "Maps").on(s.vpk_filter.as_deref() == Some("maps")).with(move |e| e.on_press(|st: &mut State| st.vpk_filter = Some("maps".to_string()))),
    ];

    col()
        .key("index-toolbar")
        .w_full()
        .gap(8.0)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(u.search_shell(field))
                .child(crate::ui::Segmented::new("index-heroes", "Heroes").parts(heroes).build(u)),
        )
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(crate::ui::Segmented::new("index-types", "Types").parts(types).build(u))
                .child(crate::ui::Segmented::new("index-vpks", "VPKs").parts(vpks).build(u)),
        )
}

fn table(s: &State, u: &Ui) -> El {
    let rows: Vec<(usize, &IndexRow)> = s
        .index
        .iter()
        .enumerate()
        .filter(|(_, r)| matches(s, r))
        .collect();

    if rows.is_empty() {
        return u.empty_state(
            "",
            "Nothing matches",
            "Clear the search or filters to see every path again.",
            Some(
                u.action("Clear filters", ActionKind::Secondary)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| {
                        st.index_query = TextEdit::new("");
                        st.hero_filter = None;
                        st.type_filter = None;
                        st.vpk_filter = None;
                    }),
            ),
        );
    }

    let head_cell = |label: &str| text(label.to_string()).font(SMALL).color(u.faint);
    let head = row()
        .w_full()
        .h(32.0)
        .px(16.0)
        .gap(12.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(head_cell("Path").grow(1.0).min_w(0.0))
        .child(head_cell("Archive").w(VPK_W))
        .child(head_cell("Size").w(SIZE_W).align(rune::Align::End))
        .child(el().w(4.0))
        .child(head_cell("Type").w(TYPE_W))
        .child(el().w(ACTION_W));
    let mut table = u.panel().key("index-table").w_full().child(head);
    let last = rows.len() - 1;
    for (i, (original, r)) in rows.iter().enumerate() {
        table = table.child(result_row(s, u, *original, r, i == last));
    }
    table
}

fn result_row(s: &State, u: &Ui, original: usize, r: &IndexRow, last: bool) -> El {
    let on = s.index_selected == Some(original);
    let path = r.path.clone();
    let path_for_extract = path.clone();
    let mut line = row()
        .key(format!("index-row-{original}"))
        .w_full()
        .h(44.0)
        .px(16.0)
        .gap(12.0)
        .center_y()
        .fill(if on { u.selected } else { Color::TRANSPARENT })
        .hover(u.hover)
        .focusable(true)
        .role("row")
        .label(path.clone())
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            st.index_selected = Some(original);
        })
        .child(
            text(path)
                .font(MONO)
                .color(u.ink)
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        )
        .child(
            text(r.vpk.clone())
                .font(MONO)
                .color(u.muted)
                .w(VPK_W)
                .truncate(),
        )
        .child(
            text(r.size_label())
                .font(MONO)
                .color(u.muted)
                .w(SIZE_W)
                .align(rune::Align::End),
        )
        .child(el().w(4.0))
        .child(text(capital(&r.kind)).font(BODY).color(u.muted).w(TYPE_W))
        .child(
            el().w(ACTION_W).child(
                u.action("Extract", ActionKind::Ghost)
                    .key(format!("index-extract-{original}"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        st.status = format!("Extracting {path_for_extract} is not available yet.");
                    }),
            ),
        );
    if !last {
        line = line.border_sides([0.0, 0.0, 1.0, 0.0], u.hair);
    }
    line
}

fn source(s: &State, u: &Ui) -> El {
    let Some(c) = &s.catalog else {
        return el();
    };
    let shown = s.index.iter().filter(|r| matches(s, r)).count();
    row()
        .w_full()
        .gap(8.0)
        .center_y()
        .child(
            text(format!("{shown} of {} paths", s.index.len()))
                .font(SMALL)
                .color(u.faint),
        )
        .child(text("·").font(SMALL).color(u.faint))
        .child(
            text(c.source.clone())
                .font(SMALL)
                .color(u.faint)
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        )
}

fn matches(s: &State, r: &IndexRow) -> bool {
    let q = s.index_query.text.to_lowercase();
    let text_match = q.is_empty()
        || r.path.to_lowercase().contains(&q)
        || r.vpk.to_lowercase().contains(&q)
        || r.hero
            .as_deref()
            .is_some_and(|h| h.to_lowercase().contains(&q));
    let hero_match = s
        .hero_filter
        .as_deref()
        .is_none_or(|h| r.hero.as_deref() == Some(h));
    let type_match = s.type_filter.as_deref().is_none_or(|t| r.kind == t);
    let vpk_match = match s.vpk_filter.as_deref() {
        Some("pak01") => r.vpk == "pak01_dir.vpk",
        Some("maps") => r.vpk.contains("/maps/"),
        _ => true,
    };
    text_match && hero_match && type_match && vpk_match
}

fn capital(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn format_count(n: u32) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out.chars().rev().collect()
}
