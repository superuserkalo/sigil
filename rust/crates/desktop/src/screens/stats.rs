//! Stats: the player surface fed by `deadlock-core::stats`.
//!
//! Every number on this screen comes from a `StatsSnapshot` fetched from
//! `api.deadlock-api.com`. The heatmap is the aggregate kill and death raster
//! bucketed into a fixed grid, with the window the API actually returned. No
//! aim, impact or positioning score is shown, because the API does not carry
//! one and the demo backend is what would supply it.

use rune::{Color, El, ScrollSpec, TextEdit, col, el, row, spacer, text};

use crate::app::State;
use crate::ui::{ActionKind, BODY, H2, LABEL, MONO, NUM, SMALL, Tone, Ui};

pub struct HeatCell {
    pub cx: usize,
    pub cy: usize,
    pub kills: u64,
    pub deaths: u64,
}

pub struct HeatGrid {
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<HeatCell>,
    pub max: u64,
    pub kills: u64,
    pub deaths: u64,
    pub window: String,
}

#[derive(Clone)]
pub struct DemoRow {
    pub match_id: String,
    pub source: String,
    pub map: Option<String>,
    pub duration_seconds: Option<u64>,
    pub players: usize,
    pub events: usize,
    pub heat_cells: usize,
    pub has_api: bool,
    pub updated: String,
}

pub struct DemoLoad {
    pub rows: Vec<DemoRow>,
    pub error: Option<String>,
}

pub struct Load {
    pub account_id: u64,
    pub snapshot: Result<deadlock_core::stats::StatsSnapshot, String>,
    pub heatmap: Result<HeatGrid, String>,
}

pub struct View {
    pub loaded: bool,
    pub account: TextEdit,
    pub error: Option<String>,
    pub snapshot: Option<deadlock_core::stats::StatsSnapshot>,
    pub heat: Option<HeatGrid>,
    pub heat_error: Option<String>,
    pub selected_match: Option<usize>,
    pub demo_path: TextEdit,
    pub demos: Vec<DemoRow>,
    pub demo_error: Option<String>,
    pub demo_busy: bool,
    pub review: Option<ReviewPanel>,
}

pub struct ReviewPanel {
    pub match_id: String,
    pub facts: deadlock_core::review::ReviewFacts,
    pub body: TextEdit,
    pub log: Option<String>,
    pub tone: Tone,
}

impl Default for View {
    fn default() -> Self {
        View {
            loaded: false,
            account: TextEdit::new(
                std::env::var("DAH_STEAM_ID")
                    .unwrap_or_default(),
            ),
            error: None,
            snapshot: None,
            heat: None,
            heat_error: None,
            selected_match: None,
            demo_path: TextEdit::new(""),
            demos: Vec::new(),
            demo_error: None,
            demo_busy: false,
            review: None,
        }
    }
}

/// Bucket raw kill and death positions into a fixed grid. The map asset reports
/// a radius of 10752 world units, so that is the normalisation bound.
pub fn bucket(
    heatmap: &deadlock_core::stats::KillHeatmap,
    cols: usize,
    rows: usize,
) -> HeatGrid {
    let radius = 10752.0_f32;
    let mut counts = vec![(0_u64, 0_u64); cols * rows];
    for point in &heatmap.points {
        let nx = (point.position_x as f32 + radius) / (2.0 * radius);
        let ny = (point.position_y as f32 + radius) / (2.0 * radius);
        if !(0.0..=1.0).contains(&nx) || !(0.0..=1.0).contains(&ny) {
            continue;
        }
        let cx = ((nx * cols as f32) as usize).min(cols - 1);
        let cy = ((ny * rows as f32) as usize).min(rows - 1);
        let slot = &mut counts[cy * cols + cx];
        slot.0 += point.kills;
        slot.1 += point.deaths;
    }
    let mut cells = Vec::new();
    let mut max = 1;
    for (index, (kills, deaths)) in counts.iter().enumerate() {
        if *kills == 0 && *deaths == 0 {
            continue;
        }
        max = max.max(*kills).max(*deaths);
        cells.push(HeatCell {
            cx: index % cols,
            cy: index / cols,
            kills: *kills,
            deaths: *deaths,
        });
    }
    HeatGrid {
        cols,
        rows,
        cells,
        max,
        kills: heatmap.kills,
        deaths: heatmap.deaths,
        window: heatmap.window.clone(),
    }
}

pub fn view(s: &State, u: &Ui) -> El {
    let v = &s.stats;
    let trailing = row()
        .gap(8.0)
        .center_y()
        .child(account_field(s, u))
        .child(if s.stats_job.is_running() {
            u.status_chip("fetching", Tone::Warn)
        } else if v.snapshot.is_some() {
            u.status_chip("live API", Tone::Ok)
        } else if v.error.is_some() {
            u.status_chip("no data", Tone::Bad)
        } else {
            u.status_chip("no account", Tone::Quiet)
        })
        .child(
            u.pill("Load", "↻", ActionKind::Secondary)
                .key("stats-load")
                .on_press(|st: &mut State| crate::data::start_stats_load(st)),
        );

    let detail = match (&v.snapshot, &v.error) {
        (Some(snapshot), _) => format!(
            "{} · {} matches in the window · source {} · generated {}",
            snapshot.profile.persona_name,
            snapshot.window.matches,
            snapshot.source,
            snapshot.generated_at
        ),
        (None, Some(error)) => error.clone(),
        (None, None) => {
            "Enter a Steam account id, or set DAH_STEAM_ID. The public API needs no key.".to_string()
        }
    };

    let page = col()
        .key("stats-page")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .px(24.0)
        .pt(18.0)
        .pb(16.0)
        .gap(12.0)
        .child(u.glow(
            "stats-glow",
            [0.16, 0.0],
            (900.0, 460.0),
            u.glow_cool,
        ))
        .child(u.page_head("PLAYER STATS", "Stats", &detail, Some(trailing)));

    let body = match &v.snapshot {
        Some(snapshot) => loaded(s, u, snapshot),
        None if s.stats_job.is_running() => loading(u),
        None => u.empty_state(
            "◉",
            "No player loaded",
            "Stats read the public Deadlock API and need a Steam account id. Nothing is uploaded and no key is required.",
            Some(
                u.action("Use the field above", ActionKind::Secondary)
                    .on_press(|st: &mut State| crate::data::start_stats_load(st)),
            ),
        ),
    };

    let mut page = page.child(body);
    page = page.child(demo_panel(s, u));
    page
}

fn demo_panel(s: &State, u: &Ui) -> El {
    let v = &s.stats;
    let mut panel = col()
        .key("stats-demos")
        .w_full()
        .gap(9.0)
        .padding(14.0)
        .radius(14.0)
        .fill(Color::WHITE.alpha(0.022))
        .border(1.0, u.hair)
        .child(
            row()
                .w_full()
                .gap(9.0)
                .center_y()
                .child(text("LOCAL DEMOS").font(LABEL).color(u.faint))
                .child(u.chip(
                    &format!("{} imported", v.demos.len()),
                    u.violet,
                    u.violet.alpha(0.10),
                ))
                .child(spacer())
                .child(
                    text("The demo is the server's view: events, players and, with a serializer-aware extractor, positions.")
                        .font(LABEL)
                        .color(u.faint)
                        .max_w(520.0)
                        .truncate(),
                ),
        );

    panel = panel.child(
        row()
            .w_full()
            .gap(9.0)
            .items_end()
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(4.0)
                    .child(u.field_label(".DEM FILE"))
                    .child(
                        rune::text_input(&v.demo_path, BODY)
                            .key("demo-path")
                            .label("Demo file path")
                            .single_line()
                            .color(u.ink)
                            .placeholder("/path/to/match.dem", u.faint)
                            .caret(u.accent, u.accent.alpha(0.20))
                            .on_edit(|st: &mut State| &mut st.stats.demo_path),
                    ),
            )
            .child(
                u.action("Import demo", ActionKind::Secondary)
                    .key("demo-import")
                    .disabled(v.demo_busy || v.demo_path.text.trim().is_empty())
                    .opacity(if v.demo_path.text.trim().is_empty() { 0.45 } else { 1.0 })
                    .on_press(|st: &mut State| crate::data::import_demo_file(st)),
            )
            .child(
                u.pill("Reload", "↻", ActionKind::Ghost)
                    .key("demo-reload")
                    .on_press(|st: &mut State| crate::data::start_demo_list(st)),
            )
            .child(
                text("review: dah demo review <id> --file reply.md --write")
                    .font(LABEL)
                    .color(u.faint)
                    .selectable(true),
            ),
    );

    if let Some(error) = &v.demo_error {
        panel = panel.child(
            text(error.clone())
                .font(SMALL)
                .color(u.amber)
                .wrap(),
        );
    }

    if v.demos.is_empty() {
        panel = panel.child(
            text("No demos imported. A .dem file is copied into the workspace, parsed for its container header and provenance, and listed here. Positions and Citadel events need a serializer-aware backend; the container parser never invents them.")
                .font(SMALL)
                .color(u.muted)
                .wrap(),
        );
        return panel;
    }

    panel = panel.child(u.table_head(&[
        ("MATCH", 150.0),
        ("SOURCE", 96.0),
        ("MAP", 110.0),
        ("DURATION", 74.0),
        ("PLAYERS", 64.0),
        ("EVENTS", 60.0),
        ("CELLS", 56.0),
        ("API", 52.0),
    ]));
    for item in v.demos.iter().take(8) {
        panel = panel.child(
            row()
                .key(format!("demo-{}", item.match_id))
                .w_full()
                .py(5.0)
                .gap(8.0)
                .center_y()
                .child(
                    text(item.match_id.clone())
                        .font(MONO)
                        .color(u.ink)
                        .w(150.0)
                        .truncate(),
                )
                .child(
                    col().w(96.0).child(u.chip(
                        &item.source,
                        u.violet,
                        u.violet.alpha(0.10),
                    )),
                )
                .child(
                    text(item.map.clone().unwrap_or_else(|| "unknown".to_string()))
                        .font(SMALL)
                        .color(u.muted)
                        .w(110.0)
                        .truncate(),
                )
                .child(
                    text(
                        item.duration_seconds
                            .map(|seconds| format!("{}m", seconds / 60))
                            .unwrap_or_else(|| "n/a".to_string()),
                    )
                    .font(MONO)
                    .color(u.muted)
                    .w(74.0),
                )
                .child(
                    text(item.players.to_string())
                        .font(MONO)
                        .color(if item.players == 0 { u.faint } else { u.muted })
                        .w(64.0),
                )
                .child(
                    text(item.events.to_string())
                        .font(MONO)
                        .color(if item.events == 0 { u.faint } else { u.muted })
                        .w(60.0),
                )
                .child(
                    text(item.heat_cells.to_string())
                        .font(MONO)
                        .color(if item.heat_cells == 0 { u.faint } else { u.teal })
                        .w(56.0),
                )
                .child(
                    text(if item.has_api { "merged" } else { "none" })
                        .font(LABEL)
                        .color(if item.has_api { u.teal } else { u.faint })
                        .w(52.0),
                )
                .child(
                    text(item.updated.clone())
                        .font(LABEL)
                        .color(u.faint)
                        .grow(1.0)
                        .truncate(),
                )
                .child(
                    u.action("Review", ActionKind::Secondary)
                        .key(format!("demo-review-{}", item.match_id))
                        .on_press({
                            let id = item.match_id.clone();
                            move |st: &mut State| crate::data::open_review(st, &id)
                        }),
                ),
        );
    }
    panel
}

/// Mounted at the root so the review panel overlays the shell.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    s.stats
        .review
        .as_ref()
        .map(|review| review_dialog(s, u, review))
}

fn review_dialog(s: &State, u: &Ui, review: &ReviewPanel) -> El {
    let endpoint = s.local_endpoint.text.trim().to_string();
    let model = s
        .agent
        .model
        .clone()
        .or_else(|| {
            if s.model_label.is_empty() || s.model_label == "No model" {
                None
            } else {
                Some(s.model_label.clone())
            }
        });
    let can_generate = !endpoint.is_empty() && model.is_some() && !s.model_review_job.is_running();
    let mut facts = col().w_full().gap(3.0);
    for fact in review.facts.facts.iter().take(7) {
        facts = facts.child(
            text(fact.text.clone())
                .font(MONO)
                .color(u.muted)
                .wrap_anywhere(),
        );
    }
    let more = review.facts.facts.len().saturating_sub(7);
    if more > 0 {
        facts = facts.child(
            text(format!("+ {more} more verified fact(s)"))
                .font(LABEL)
                .color(u.faint),
        );
    }

    let body = col()
        .w_full()
        .gap(13.0)
        .child(
            row()
                .w_full()
                .gap(12.0)
                .items_start()
                .child(
                    col()
                        .grow(1.0)
                        .gap(5.0)
                        .child(u.eyebrow("VERIFIED FACTS · LINTED REVIEW"))
                        .child(
                            text(format!("Review match {}", review.match_id))
                                .font(H2)
                                .color(u.ink),
                        )
                        .child(
                            text("Send the prompt to your model, paste the reply, and the linter refuses any number the facts do not support. review.md is written only when it passes.")
                                .font(SMALL)
                                .color(u.muted)
                                .wrap(),
                        ),
                )
                .child(u.glyph_box("◇", Tone::Info, 36.0)),
        )
        .child(u.divider())
        .child(
            row()
                .w_full()
                .gap(12.0)
                .items_start()
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(5.0)
                        .child(u.field_label("VERIFIED FACTS"))
                        .child(facts),
                )
                .child(
                    col()
                        .w(300.0)
                        .gap(4.0)
                        .child(u.field_label("ALLOWED NUMBERS"))
                        .child(
                            text(
                                review
                                    .facts
                                    .allowed_numbers
                                    .iter()
                                    .cloned()
                                    .collect::<Vec<_>>()
                                    .join("  "),
                            )
                            .font(MONO)
                            .color(u.faint)
                            .wrap_anywhere(),
                        )
                        .child(u.field_label("ABSENT"))
                        .child(
                            text(if review.facts.absent.is_empty() {
                                "none".to_string()
                            } else {
                                review.facts.absent.join(", ")
                            })
                            .font(MONO)
                            .color(u.amber)
                            .wrap_anywhere(),
                        ),
                ),
        )
        .child(u.field_label("PROMPT"))
        .child(
            col()
                .key("review-prompt")
                .w_full()
                .h(118.0)
                .padding(10.0)
                .radius(9.0)
                .fill(u.field)
                .border(1.0, u.hair)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(
                    text(review.facts.prompt.clone())
                        .font(MONO)
                        .color(u.prose)
                        .wrap_anywhere()
                        .selectable(true),
                ),
        )
        .child(
            row()
                .w_full()
                .gap(9.0)
                .center_y()
                .child(u.field_label("MODEL"))
                .child(
                    text(format!(
                        "{} · {}",
                        model.clone().unwrap_or_else(|| "no model".to_string()),
                        if endpoint.is_empty() {
                            "no local endpoint".to_string()
                        } else {
                            endpoint.clone()
                        }
                    ))
                    .font(MONO)
                    .color(if can_generate { u.muted } else { u.faint })
                    .grow(1.0)
                    .truncate(),
                )
                .child(if s.model_review_job.is_running() {
                    u.status_chip("generating", Tone::Warn)
                } else {
                    u.action("Generate with model", ActionKind::Secondary)
                        .disabled(!can_generate)
                        .opacity(if can_generate { 1.0 } else { 0.45 })
                        .on_press(|st: &mut State| crate::data::start_review_generation(st))
                }),
        )
        .child(u.field_label("REVIEW REPLY"))
        .child(
            rune::text_input(&review.body, BODY)
                .key("review-body")
                .label("Review reply")
                .h(92.0)
                .color(u.ink)
                .placeholder("Paste the model reply here", u.faint)
                .caret(u.accent, u.accent.alpha(0.20))
                .on_edit(|st: &mut State| {
                    st.stats
                        .review
                        .as_mut()
                        .map(|review| &mut review.body)
                        .unwrap_or(&mut st.stats.demo_path)
                }),
        );

    let mut body = body;
    if let Some(log) = &review.log {
        body = body.child(
            text(log.clone())
                .font(SMALL)
                .color(review.tone.fg(u))
                .wrap(),
        );
    }

    body = body.child(
        row()
            .w_full()
            .gap(9.0)
            .child(
                u.action("Close", ActionKind::Ghost)
                    .grow(1.0)
                    .on_press(|st: &mut State| st.stats.review = None),
            )
            .child(
                u.action("Lint and save review", ActionKind::Primary)
                    .grow(1.0)
                    .on_press(|st: &mut State| crate::data::save_review(st)),
            ),
    );

    let close = |st: &mut State| st.stats.review = None;
    u.dialog_shell()
        .key("review-dialog")
        .role("dialog")
        .label("Review a match")
        .w(780.0)
        .max_w(780.0)
        .padding(1.0)
        .modal(true, close)
        .on_key(move |st: &mut State, ev: &rune::KeyEvent, _| {
            if ev.key == rune::Key::Escape {
                close(st);
                true
            } else {
                false
            }
        })
        .child(u.dialog_core().padding(20.0).child(body))
}

fn account_field(s: &State, u: &Ui) -> El {
    rune::text_input(&s.stats.account, SMALL)
        .key("stats-account")
        .label("Steam account id")
        .single_line()
        .w(150.0)
        .color(u.ink)
        .placeholder("account id", u.faint)
        .caret(u.accent, u.accent.alpha(0.20))
        .on_edit(|st: &mut State| &mut st.stats.account)
}

fn loaded(s: &State, u: &Ui, snap: &deadlock_core::stats::StatsSnapshot) -> El {
    let mut page = col().w_full().gap(12.0);

    page = page.child(identity(s, u, snap));
    page = page.child(kpis(s, u, snap));

    let middle = row()
        .w_full()
        .gap(12.0)
        .items_start()
        .child(heat_panel(s, u))
        .child(hero_panel(u, snap));

    page = page.child(middle);

    let lower = row()
        .w_full()
        .gap(12.0)
        .items_start()
        .child(match_panel(s, u, snap))
        .child(form_panel(u, snap));
    page = page.child(lower);

    col()
        .key("stats-scroll")
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(page)
}

fn identity(_s: &State, u: &Ui, snap: &deadlock_core::stats::StatsSnapshot) -> El {
    let initial = snap
        .profile
        .persona_name
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "?".to_string());
    let rank = if snap.rank.badge == 0 {
        "Unranked".to_string()
    } else {
        format!(
            "{} {}",
            snap.rank.tier_name.clone().unwrap_or_else(|| "Rank".to_string()),
            snap.rank.subrank
        )
    };
    let percentile = snap
        .rank
        .benchmark
        .as_ref()
        .map(|bench| format!("top {:.2}% of {} ranked", 100.0 - bench.percentile, bench.ranked_players))
        .unwrap_or_else(|| "no population benchmark".to_string());

    row()
        .key("stats-identity")
        .w_full()
        .gap(16.0)
        .padding(16.0)
        .radius(14.0)
        .fill(Color::WHITE.alpha(0.03))
        .border(1.0, u.hair)
        .items_center()
        .child(
            el().size(64.0, 64.0)
                .radius(16.0)
                .fill(u.accent.alpha(0.14))
                .border(1.0, u.accent.alpha(0.28))
                .center()
                .child(text(initial).font(NUM).color(u.accent)),
        )
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(4.0)
                .child(
                    row()
                        .gap(9.0)
                        .center_y()
                        .child(
                            text(snap.profile.persona_name.clone())
                                .font(H2)
                                .color(u.ink),
                        )
                        .child(
                            text(format!("#{}", snap.profile.account_id))
                                .font(MONO)
                                .color(u.faint),
                        ),
                )
                .child(
                    row()
                        .gap(8.0)
                        .center_y()
                        .child(u.chip(&rank, u.accent, u.accent.alpha(0.12)))
                        .child(text(percentile).font(SMALL).color(u.muted)),
                )
                .child(
                    text(format!(
                        "{} matches in the last 30 days · {} friends · profile refreshed {}",
                        snap.profile.matches_played_last_30d,
                        snap.profile.friend_count,
                        snap.profile.last_updated
                    ))
                    .font(LABEL)
                    .color(u.faint)
                    .truncate(),
                ),
        )
        .child(
            col()
                .gap(3.0)
                .items_end()
                .child(u.eyebrow("CAREER"))
                .child(
                    text(format!("{} matches", snap.career.matches_played))
                        .font(NUM)
                        .color(u.ink),
                )
                .child(
                    text(format!(
                        "{:.1}% win · {:.2} KDA",
                        snap.career.win_rate_percent, snap.career.kda
                    ))
                    .font(SMALL)
                    .color(u.muted),
                ),
        )
}

fn kpis(_s: &State, u: &Ui, snap: &deadlock_core::stats::StatsSnapshot) -> El {
    let career = &snap.career;
    let window = &snap.window;
    let tiles = [
        (
            format!("{:.1}%", career.win_rate_percent),
            format!(
                "career win · {:.1}-{:.1} CI",
                career.win_rate_low_percent, career.win_rate_high_percent
            ),
            Tone::Ok,
        ),
        (
            format!("{:.2}", career.kda),
            format!("{} / {} / {}", career.kills, career.deaths, career.assists),
            Tone::Accent,
        ),
        (
            format!("{:.0}", career.damage_per_minute),
            "damage per minute".to_string(),
            Tone::Info,
        ),
        (
            format!("{:.1}M", career.objective_damage as f64 / 1_000_000.0),
            "objective damage, estimated".to_string(),
            Tone::Quiet,
        ),
        (
            format!("{:.0}%", window.recent_win_rate_percent),
            format!(
                "last {} · {:.1} KDA",
                window.matches, window.kda
            ),
            if window.recent_win_rate_percent >= 50.0 {
                Tone::Ok
            } else {
                Tone::Warn
            },
        ),
        (
            format!("{:.0}m", career.average_match_seconds / 60.0),
            "average match".to_string(),
            Tone::Quiet,
        ),
    ];
    let mut r = row().w_full().gap(10.0).items_start();
    for (value, label, tone) in tiles {
        r = r.child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(3.0)
                .padding(12.0)
                .radius(11.0)
                .fill(Color::WHITE.alpha(0.022))
                .border(1.0, u.hair)
                .child(text(value).font(NUM).color(tone.fg(u)))
                .child(text(label).font(LABEL).color(u.faint).truncate()),
        );
    }
    r
}

fn heat_panel(s: &State, u: &Ui) -> El {
    let mut panel = col()
        .key("stats-heat")
        .grow(1.35)
        .min_w(0.0)
        .gap(10.0)
        .padding(14.0)
        .radius(14.0)
        .fill(Color::WHITE.alpha(0.024))
        .border(1.0, u.hair);

    let header = row()
        .w_full()
        .gap(9.0)
        .center_y()
        .child(text("KILL AND DEATH HEATMAP").font(LABEL).color(u.faint))
        .child(spacer())
        .child(match &s.stats.heat {
            Some(heat) => u.chip(
                &format!("{} cells", heat.cells.len()),
                u.violet,
                u.violet.alpha(0.10),
            ),
            None => u.chip("no raster", u.faint, Color::WHITE.alpha(0.04)),
        });
    panel = panel.child(header);

    match &s.stats.heat {
        Some(heat) => {
            let meta = row()
                .w_full()
                .gap(14.0)
                .center_y()
                .child(u.stat(heat.kills.to_string(), "kills", Tone::Accent))
                .child(u.stat(heat.deaths.to_string(), "deaths", Tone::Bad))
                .child(
                    text(if heat.window.is_empty() {
                        "window not reported by the API".to_string()
                    } else {
                        format!("window {}", heat.window)
                    })
                    .font(LABEL)
                    .color(u.faint)
                    .grow(1.0)
                    .truncate(),
                );
            panel = panel.child(meta);
            panel = panel.child(raster(u, heat));
            panel = panel.child(
                text("World units mapped from the map radius of 10752. Each cell is a bucket, not a single event.")
                    .font(LABEL)
                    .color(u.faint)
                    .wrap(),
            );
        }
        None => {
            panel = panel.child(u.note(
                s.stats
                    .heat_error
                    .clone()
                    .unwrap_or_else(|| "The heatmap request did not return.".to_string()),
            ));
        }
    }
    panel
}

fn raster(u: &Ui, heat: &HeatGrid) -> El {
    let size = 7.0;
    let gap = 1.0;
    let width = (heat.cols as f32) * (size + gap);
    let height = (heat.rows as f32) * (size + gap);
    let mut field = el()
        .key("heat-raster")
        .w(width)
        .h(height)
        .radius(8.0)
        .fill(Color::BLACK.alpha(0.28))
        .border(1.0, u.hair);
    for cell in &heat.cells {
        let intensity = (cell.kills.max(cell.deaths) as f32 / heat.max as f32).clamp(0.0, 1.0);
        let color = if cell.deaths > cell.kills {
            u.rose
        } else if cell.kills > 0 {
            u.accent
        } else {
            u.violet
        };
        field = field.child(
            el().key(format!("heat-{}-{}", cell.cx, cell.cy))
                .absolute(
                    cell.cx as f32 * (size + gap),
                    cell.cy as f32 * (size + gap),
                )
                .size(size, size)
                .radius(1.5)
                .fill(color.alpha(0.14 + 0.80 * intensity))
                .a11y_hidden(),
        );
    }
    row()
        .w_full()
        .items_start()
        .gap(16.0)
        .child(field)
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(8.0)
                .child(
                    text("Density")
                        .font(LABEL)
                        .color(u.faint),
                )
                .child(legend(u, u.accent, "kills outnumber deaths"))
                .child(legend(u, u.rose, "deaths outnumber kills"))
                .child(u.divider())
                .child(
                    text("Above average cells are brighter. The raster is a 30 day aggregate because an older start time times out for an active account.")
                        .font(LABEL)
                        .color(u.faint)
                        .wrap(),
                ),
        )
}

fn legend(u: &Ui, color: Color, label: &str) -> El {
    row()
        .gap(7.0)
        .center_y()
        .child(el().size(9.0, 9.0).radius(2.0).fill(color.alpha(0.6)))
        .child(text(label).font(LABEL).color(u.muted).wrap())
}

fn hero_panel(u: &Ui, snap: &deadlock_core::stats::StatsSnapshot) -> El {
    let mut panel = col()
        .key("stats-heroes")
        .grow(1.0)
        .min_w(0.0)
        .gap(8.0)
        .padding(14.0)
        .radius(14.0)
        .fill(Color::WHITE.alpha(0.024))
        .border(1.0, u.hair)
        .child(
            row()
                .w_full()
                .child(text("HEROES BY SAMPLE").font(LABEL).color(u.faint))
                .child(spacer())
                .child(text(format!("{} played", snap.heroes.len())).font(LABEL).color(u.faint)),
        );
    if snap.heroes.is_empty() {
        panel = panel.child(u.note("The API returned no hero rows for this account."));
        return panel;
    }
    let max = snap
        .heroes
        .iter()
        .map(|hero| hero.matches_played)
        .max()
        .unwrap_or(1)
        .max(1);
    for hero in snap.heroes.iter().take(7) {
        let name = hero
            .hero_name
            .clone()
            .unwrap_or_else(|| format!("Hero {}", hero.hero_id));
        let share = hero.matches_played as f32 / max as f32;
        panel = panel.child(
            col()
                .w_full()
                .gap(4.0)
                .child(
                    row()
                        .w_full()
                        .gap(6.0)
                        .center_y()
                        .child(
                            text(name)
                                .font(SMALL)
                                .color(u.ink)
                                .grow(1.0)
                                .truncate(),
                        )
                        .child(
                            text(format!("{:.1}%", hero.win_rate_percent))
                                .font(MONO)
                                .color(if hero.win_rate_percent >= 50.0 {
                                    u.teal
                                } else {
                                    u.rose
                                }),
                        ),
                )
                .child(
                    el()
                        .w_full()
                        .h(4.0)
                        .radius(2.0)
                        .fill(Color::WHITE.alpha(0.05))
                        .child(
                            el()
                                .w_full()
                                .max_w(share * 100.0)
                                .h(4.0)
                                .radius(2.0)
                                .fill(u.accent.alpha(0.55)),
                        ),
                )
                .child(
                    text(format!(
                        "{} matches · {:.2} KDA · {:.0} dmg/min",
                        hero.matches_played, hero.kda, hero.damage_per_minute
                    ))
                    .font(LABEL)
                    .color(u.faint)
                    .truncate(),
                ),
        );
    }
    let extra = snap.heroes.len().saturating_sub(7);
    if extra > 0 {
        panel = panel.child(
            text(format!("+ {extra} more hero(es) with smaller samples"))
                .font(LABEL)
                .color(u.faint),
        );
    }
    panel
}

fn form_panel(u: &Ui, snap: &deadlock_core::stats::StatsSnapshot) -> El {
    let mut strip = row().gap(4.0);
    for entry in &snap.window.recent_form {
        strip = strip.child(
            el()
                .size(26.0, 26.0)
                .radius(5.0)
                .fill(if entry.win {
                    u.teal.alpha(0.20)
                } else {
                    u.rose.alpha(0.18)
                })
                .border(
                    1.0,
                    if entry.win {
                        u.teal.alpha(0.35)
                    } else {
                        u.rose.alpha(0.30)
                    },
                )
                .center()
                .child(
                    text(if entry.win { "W" } else { "L" })
                        .font(LABEL)
                        .color(if entry.win { u.teal } else { u.rose }),
                ),
        );
    }
    col()
        .key("stats-form")
        .w(300.0)
        .gap(9.0)
        .padding(14.0)
        .radius(14.0)
        .fill(Color::WHITE.alpha(0.024))
        .border(1.0, u.hair)
        .child(text("RECENT FORM").font(LABEL).color(u.faint))
        .child(strip)
        .child(
            text(format!(
                "{} W / {} L in the last 10 decisive matches",
                snap.window
                    .recent_form
                    .iter()
                    .filter(|entry| entry.win)
                    .count(),
                snap.window
                    .recent_form
                    .iter()
                    .filter(|entry| !entry.win)
                    .count()
            ))
            .font(SMALL)
            .color(u.muted),
        )
        .child(u.divider())
        .child(u.kv(
            "Window",
            &format!(
                "{} matches · {:.1}% wins · {:.1} KDA",
                snap.window.matches, snap.window.win_rate_percent, snap.window.kda
            ),
            u.ink,
        ))
        .child(u.kv(
            "Avg net worth",
            &format!("{:.0}", snap.window.average_net_worth),
            u.muted,
        ))
        .child(u.kv(
            "Unscored",
            &snap.window.unscored.to_string(),
            u.faint,
        ))
        .child(
            text("Penalized and unscored matches are excluded from the win rate.")
                .font(LABEL)
                .color(u.faint)
                .wrap(),
        )
}

fn match_panel(s: &State, u: &Ui, snap: &deadlock_core::stats::StatsSnapshot) -> El {
    let mut panel = col()
        .key("stats-matches")
        .grow(1.0)
        .min_w(0.0)
        .gap(0.0)
        .padding(14.0)
        .radius(14.0)
        .fill(Color::WHITE.alpha(0.024))
        .border(1.0, u.hair)
        .child(
            row()
                .w_full()
                .pb(8.0)
                .child(text("RECENT MATCHES").font(LABEL).color(u.faint))
                .child(spacer())
                .child(
                    text(format!("newest {} of {}", snap.matches.len(), snap.profile.matches_played_last_30d))
                        .font(LABEL)
                        .color(u.faint),
                ),
        )
        .child(u.table_head(&[
            ("RESULT", 62.0),
            ("HERO", 110.0),
            ("K / D / A", 96.0),
            ("KDA", 52.0),
            ("NET", 76.0),
            ("DURATION", 70.0),
            ("STARTED", 120.0),
        ]));
    for (index, item) in snap.matches.iter().take(12).enumerate() {
        let selected = s.stats.selected_match == Some(index);
        panel = panel.child(
            row()
                .key(format!("match-{}", item.match_id))
                .w_full()
                .py(6.0)
                .gap(8.0)
                .center_y()
                .radius(6.0)
                .fill(if selected {
                    Color::WHITE.alpha(0.045)
                } else {
                    Color::TRANSPARENT
                })
                .hover(Color::WHITE.alpha(0.035))
                .child(
                    col()
                        .w(62.0)
                        .child(u.status_chip(
                            item.outcome.label(),
                            match item.win {
                                Some(true) => Tone::Ok,
                                Some(false) => Tone::Bad,
                                None => Tone::Quiet,
                            },
                        )),
                )
                .child(
                    text(
                        item.hero_name
                            .clone()
                            .unwrap_or_else(|| format!("Hero {}", item.hero_id)),
                    )
                    .font(SMALL)
                    .color(u.ink)
                    .w(110.0)
                    .truncate(),
                )
                .child(
                    text(format!("{} / {} / {}", item.kills, item.deaths, item.assists))
                        .font(MONO)
                        .color(u.muted)
                        .w(96.0),
                )
                .child(
                    text(format!("{:.2}", item.kda))
                        .font(MONO)
                        .color(u.muted)
                        .w(52.0),
                )
                .child(
                    text(format!("{}", item.net_worth))
                        .font(MONO)
                        .color(u.muted)
                        .w(76.0),
                )
                .child(
                    text(format!("{}m", item.duration_seconds / 60))
                        .font(MONO)
                        .color(u.faint)
                        .w(70.0),
                )
                .child(
                    text(item.started_at.chars().take(16).collect::<String>())
                        .font(MONO)
                        .color(u.faint)
                        .grow(1.0)
                        .truncate(),
                ),
        );
    }
    panel
}

fn loading(u: &Ui) -> El {
    col()
        .w_full()
        .gap(10.0)
        .child(u.note("Fetching player, career, hero and heatmap rows from api.deadlock-api.com."))
        .child(
            col()
                .w_full()
                .h(220.0)
                .radius(14.0)
                .fill(Color::WHITE.alpha(0.022))
                .border(1.0, u.hair)
                .center()
                .child(text("loading stats").font(LABEL).color(u.faint)),
        )
}
