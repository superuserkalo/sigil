//! Game Index: search, filters, result rows and the source summary.

use rune::tok::motion;
use rune::{Color, El, Key, KeyEvent, ScrollSpec, TextEdit, col, el, row, text};

use crate::app::{State, filter_chip};
use crate::model::{IndexRow, Screen};
use crate::ui::{ActionKind, BODY, BODY_MED, LABEL, MONO, MONO_MED, SMALL, Tone, Ui};

const HEROES: [&str; 3] = ["Infernus", "Lash", "Vindicta"];
const TYPES: [&str; 5] = ["material", "model", "particle", "sound", "layout"];

pub fn view(s: &State, u: &Ui) -> El {
    let trailing = row()
        .gap(8.0)
        .center_y()
        .child(u.status_chip(
            if s.catalog.is_some() { "index ready" } else { "not indexed" },
            if s.catalog.is_some() { Tone::Ok } else { Tone::Warn },
        ))
        .child(u.action("Rebuild index", ActionKind::Secondary).on_press(|st: &mut State| {
            st.status =
                "Index rebuild is not wired in this build. No game install was read and no cache was written."
                    .to_string();
        }));

    col()
        .key("index-page")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .px(24.0)
        .pt(20.0)
        .pb(18.0)
        .gap(13.0)
        .child(u.page_head(
            Screen::Index.kicker(),
            "Game Index",
            "Exact paths from citadel, core and the map VPKs. Read only, one wrong letter and the game ignores the file.",
            Some(trailing),
        ))
        .child(search(s, u))
        .child(filters(s, u))
        .child(summary(s, u))
        .child(table(s, u))
        .child(u.note(
            "The index is read only. Extraction writes to the staging folder, never to the game install.",
        ))
}

fn search(s: &State, u: &Ui) -> El {
    let field = rune::text_input(&s.index_query, BODY)
        .key("index-search")
        .label("Search game paths")
        .single_line()
        .grow(1.0)
        .color(u.ink)
        .placeholder("Search exact paths, heroes or VPKs", u.faint)
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

    row()
        .key("index-search-row")
        .w_full()
        .h(38.0)
        .px(12.0)
        .gap(9.0)
        .center_y()
        .radius(8.0)
        .fill(u.well)
        .border(1.0, u.hair_strong)
        .child(text("⌕").font(BODY).color(u.faint).a11y_hidden())
        .child(field)
        .child(
            text(format!("{} paths", s.index.len()))
                .font(SMALL)
                .color(u.faint),
        )
}

fn filters(s: &State, u: &Ui) -> El {
    let mut r = row()
        .key("index-filters")
        .w_full()
        .gap(6.0)
        .flex_wrap()
        .items_center();
    r = r
        .child(text("HERO").font(LABEL).color(u.faint).mr(3.0))
        .child(
            filter_chip(u, "All", s.hero_filter.is_none())
                .on_press(|st: &mut State| st.hero_filter = None),
        );
    for hero in HEROES {
        r = r.child(
            filter_chip(u, hero, s.hero_filter.as_deref() == Some(hero)).on_press(
                move |st: &mut State| {
                    st.hero_filter = Some(hero.to_string());
                },
            ),
        );
    }
    r = r
        .child(el().w(10.0))
        .child(text("TYPE").font(LABEL).color(u.faint).mr(3.0))
        .child(
            filter_chip(u, "All", s.type_filter.is_none())
                .on_press(|st: &mut State| st.type_filter = None),
        );
    for kind in TYPES {
        r = r.child(
            filter_chip(u, kind, s.type_filter.as_deref() == Some(kind)).on_press(
                move |st: &mut State| {
                    st.type_filter = Some(kind.to_string());
                },
            ),
        );
    }
    r = r
        .child(el().w(10.0))
        .child(text("VPK").font(LABEL).color(u.faint).mr(3.0))
        .child(
            filter_chip(u, "All", s.vpk_filter.is_none())
                .on_press(|st: &mut State| st.vpk_filter = None),
        )
        .child(
            filter_chip(u, "pak01", s.vpk_filter.as_deref() == Some("pak01"))
                .on_press(|st: &mut State| st.vpk_filter = Some("pak01".to_string())),
        )
        .child(
            filter_chip(u, "maps", s.vpk_filter.as_deref() == Some("maps"))
                .on_press(|st: &mut State| st.vpk_filter = Some("maps".to_string())),
        );
    r
}

fn summary(s: &State, u: &Ui) -> El {
    match &s.catalog {
        Some(c) => col()
            .w_full()
            .gap(7.0)
            .child(
                row()
                    .w_full()
                    .gap(9.0)
                    .child(u.stat(format_count(c.citadel), "CITADEL FILES", Tone::Info))
                    .child(u.stat(format_count(c.core), "CORE FILES", Tone::Quiet))
                    .child(u.stat(format!("{}", c.maps), "MAP VPKS", Tone::Accent))
                    .child(u.stat(c.indexed_at.clone(), "LAST INDEXED", Tone::Ok)),
            )
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(text("SOURCE").font(LABEL).color(u.faint))
                    .child(text(c.source.clone()).font(MONO).color(u.muted).grow(1.0).truncate()),
            ),
        None => u.panel().padding(14.0).child(
            row()
                .w_full()
                .gap(10.0)
                .items_center()
                .child(u.glyph_box("⌕", Tone::Warn, 30.0))
                .child(
                    col()
                        .grow(1.0)
                        .gap(2.0)
                        .child(text("No game index loaded").font(BODY_MED).color(u.ink))
                        .child(
                            text("Set the game path, then build the index. Nothing is guessed from a folder name.")
                                .font(SMALL)
                                .color(u.muted)
                                .wrap(),
                        ),
                )
                .child(
                    u.action("Open Settings", ActionKind::Secondary)
                        .on_press(|st: &mut State| st.screen = Screen::Settings),
                ),
        ),
    }
}

fn table(s: &State, u: &Ui) -> El {
    let total = s.index.len();
    let rows: Vec<(usize, &IndexRow)> = s
        .index
        .iter()
        .enumerate()
        .filter(|(_, r)| matches(s, r))
        .collect();

    if s.catalog.is_none() {
        return el()
            .key("index-empty")
            .grow(1.0)
            .min_h(0.0)
            .child(u.empty_state(
                "⌕",
                "No paths to show",
                "The index is empty. The workbench only lists paths it has read from an installed game.",
                None,
            ));
    }

    if rows.is_empty() {
        return el().key("index-empty").grow(1.0).min_h(0.0).child(
            u.empty_state(
                "⌕",
                "No paths match",
                "Clear the search or filters to see the full index again.",
                Some(u.action("Clear filters", ActionKind::Secondary).on_press(
                    |st: &mut State| {
                        st.index_query = TextEdit::new("");
                        st.hero_filter = None;
                        st.type_filter = None;
                        st.vpk_filter = None;
                        st.status = "Filters cleared.".to_string();
                    },
                )),
            ),
        );
    }

    let mut body = col().key("index-rows").w_full();
    body = body.child(u.table_head(&[
        ("PATH", 0.0),
        ("VPK", 128.0),
        ("SIZE", 68.0),
        ("TYPE", 80.0),
        ("", 58.0),
    ]));
    for (i, (original, r)) in rows.iter().enumerate() {
        if i > 0 {
            body = body.child(u.divider());
        }
        body = body.child(result_row(s, u, *original, r));
    }

    u.panel()
        .key("index-table")
        .grow(1.0)
        .min_h(0.0)
        .child(u.panel_header("PATHS", &format!("{} of {} matching", rows.len(), total)))
        .child(
            col()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(body),
        )
}

fn result_row(s: &State, u: &Ui, original: usize, r: &IndexRow) -> El {
    let on = s.index_selected == Some(original);
    let path = r.path.clone();
    let path_for_press = path.clone();
    let path_for_text = path.clone();
    let kind_tone = kind_tone(&r.kind);
    row()
        .key(format!("index-row-{original}"))
        .w_full()
        .h(44.0)
        .px(14.0)
        .gap(12.0)
        .center_y()
        .fill(if on { u.accent.alpha(0.06) } else { Color::TRANSPARENT })
        .hover(Color::WHITE.alpha(0.028))
        .focusable(true)
        .role("row")
        .label(path.clone())
        .motion(motion::SNAPPY)
        .on_press(move |st: &mut State| {
            st.index_selected = Some(original);
            st.status = format!("Selected {path_for_press}.");
        })
        .child(
            el()
                .size(7.0, 7.0)
                .radius(4.0)
                .fill(kind_tone.fg(u).alpha(0.75))
                .a11y_hidden(),
        )
        .child(text(path_for_text).font(MONO_MED).color(u.ink).grow(1.0).truncate())
        .child(text(r.vpk.clone()).font(MONO).color(u.faint).w(128.0).truncate())
        .child(text(r.size_label()).font(MONO).color(u.muted).w(68.0).align(rune::Align::End))
        .child(
            el()
                .w(80.0)
                .child(u.chip(&r.kind, kind_tone.fg(u), kind_tone.bg(u))),
        )
        .child(
            u.action("Extract", ActionKind::Ghost).on_press(move |st: &mut State| {
                st.status = format!(
                    "Extract requested for {path}. Nothing was written: the core staging writer is not connected."
                );
            }),
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

fn kind_tone(kind: &str) -> Tone {
    match kind {
        "material" => Tone::Accent,
        "model" => Tone::Info,
        "particle" => Tone::Warn,
        "sound" => Tone::Ok,
        "map" => Tone::Info,
        _ => Tone::Quiet,
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
