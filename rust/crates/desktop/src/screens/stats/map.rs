//! Where kills and deaths happen: the 30 day raster on the minimap frame.

use deadlock_core::stats::MapObjective;
use rune::{Color, El, Shape, canvas, col, el, row, spacer, text};

use crate::app::State;
use crate::ui::{BODY, H2, NUM, SMALL, Ui};

use super::german::tr;
use super::shared::*;
use super::{HeatCell, HeatGrid, Layer};

// --- map -------------------------------------------------------------------

const MAP_SIZE: f32 = 360.0;

struct Hotspot {
    cx: usize,
    cy: usize,
    count: u64,
    label: String,
}

fn hotspots(heat: &HeatGrid, objectives: &[MapObjective], layer: Layer) -> Vec<Hotspot> {
    let value = |cell: &HeatCell| match layer {
        Layer::Deaths => cell.deaths,
        Layer::Kills => cell.kills,
    };
    // Score each cell with its eight neighbours so one busy cell does not beat a busy area.
    let mut grid = vec![0u64; heat.cols * heat.rows];
    for cell in &heat.cells {
        grid[cell.cy * heat.cols + cell.cx] = value(cell);
    }
    let mut scored: Vec<(u64, usize, usize)> = heat
        .cells
        .iter()
        .map(|cell| {
            let mut sum = 0;
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    let (x, y) = (cell.cx as i64 + dx, cell.cy as i64 + dy);
                    if x >= 0 && y >= 0 && (x as usize) < heat.cols && (y as usize) < heat.rows {
                        sum += grid[y as usize * heat.cols + x as usize];
                    }
                }
            }
            (sum, cell.cx, cell.cy)
        })
        .collect();
    scored.sort_by_key(|row| std::cmp::Reverse(row.0));
    let mut out: Vec<Hotspot> = Vec::new();
    for (sum, cx, cy) in scored {
        if out.len() == 3 || sum == 0 {
            break;
        }
        if out
            .iter()
            .any(|spot| spot.cx.abs_diff(cx) <= 4 && spot.cy.abs_diff(cy) <= 4)
        {
            continue;
        }
        // The player's team in this area, from the cells around it.
        let mut team = [0u64; 2];
        for cell in heat
            .cells
            .iter()
            .filter(|cell| cell.cx.abs_diff(cx) <= 1 && cell.cy.abs_diff(cy) <= 1)
        {
            let split = match layer {
                Layer::Deaths => cell.death_team,
                Layer::Kills => cell.kill_team,
            };
            team[0] += split[0];
            team[1] += split[1];
        }
        let player_team = if team[1] > team[0] { 1 } else { 0 };
        let left = (cx as f64 + 0.5) / heat.cols as f64;
        let top = (cy as f64 + 0.5) / heat.rows as f64;
        out.push(Hotspot {
            cx,
            cy,
            count: sum,
            label: place_name(objectives, left, top, player_team),
        });
    }
    out
}

/// "Near the enemy Guardian, middle lane", from the closest objective.
fn place_name(objectives: &[MapObjective], left: f64, top: f64, player_team: u32) -> String {
    let nearest = objectives.iter().min_by(|a, b| {
        let da = (a.left - left).powi(2) + (a.top - top).powi(2);
        let db = (b.left - left).powi(2) + (b.top - top).powi(2);
        da.total_cmp(&db)
    });
    let Some(objective) = nearest else {
        return tr("Open map").to_string();
    };
    let distance = ((objective.left - left).powi(2) + (objective.top - top).powi(2)).sqrt();
    if distance > 0.14 {
        return if top < 0.46 && player_team == 0 || top > 0.46 && player_team == 1 {
            tr("Open map, enemy half").to_string()
        } else {
            tr("Open map, your half").to_string()
        };
    }
    let parts: Vec<&str> = objective.key.split('_').collect();
    let team: u32 = parts
        .first()
        .and_then(|part| part.strip_prefix("team"))
        .and_then(|digit| digit.parse().ok())
        .unwrap_or(0);
    let side = if team == player_team { "your" } else { "the enemy" };
    let kind = match parts.get(1).copied() {
        Some("core") => tr("base"),
        Some("titan") => tr("Patron"),
        Some("tier1") => tr("Guardian"),
        Some("tier2") => tr("Walker"),
        _ => tr("objective"),
    };
    let lane = match parts.get(2).copied() {
        Some("3") => tr(", middle lane"),
        Some(_) => tr(", side lane"),
        None => "",
    };
    crate::screens::settings::i18n::fill(tr("Near {} {}{}"), &[tr(side), kind, lane])
}

pub(super) fn map_section(s: &State, u: &Ui) -> El {
    let v = &s.stats;
    let mut section = col().key("stats-map").w_full().gap(12.0);
    let Some(heat) = &v.heat else {
        return section.child(section_head(u, "Where fights happen", "")).child(
            text(
                v.heat_error
                    .clone()
                    .unwrap_or_else(|| "The kill and death map did not load.".to_string()),
            )
            .font(BODY)
            .color(u.muted)
            .wrap(),
        );
    };
    let layer = v.layer;
    let switch = crate::ui::Segmented::new("stats-layer", tr("Layer")).parts(vec![
            crate::ui::SegPart::new("stats-layer-deaths", tr("Deaths")).on(layer == Layer::Deaths).with(move |e| e.on_press(|st: &mut State| st.stats.layer = Layer::Deaths)),
            crate::ui::SegPart::new("stats-layer-kills", tr("Kills")).on(layer == Layer::Kills).with(move |e| e.on_press(|st: &mut State| st.stats.layer = Layer::Kills)),
        ]).build(u);
    section = section.child(
        row()
            .w_full()
            .h(32.0)
            .gap(12.0)
            .center_y()
            .child(text(tr("Where fights happen")).font(H2).color(u.ink))
            .child(
                text(crate::screens::settings::i18n::fill(
                    tr("{}, every match, both sides of the map"),
                    &[&capitalise(&heat.window)],
                ))
                .font(SMALL)
                .color(u.muted),
            )
            .child(spacer())
            .child(switch),
    );

    let spots = hotspots(heat, &v.objectives, layer);
    let color = match layer {
        Layer::Deaths => u.rose,
        Layer::Kills => u.teal,
    };
    let mut side = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(20.0)
        .child(
            row()
                .gap(32.0)
                .child(big_number(u, &thousands(heat.kills), "Kills"))
                .child(big_number(u, &thousands(heat.deaths), "Deaths"))
                .child(big_number(
                    u,
                    &format!("{:.2}", heat.kills as f64 / heat.deaths.max(1) as f64),
                    "Kills per death",
                )),
        );
    let mut list = col().w_full().child(
        text(match layer {
            Layer::Deaths => tr("Where you die most"),
            Layer::Kills => tr("Where you get kills"),
        })
        .font(SMALL)
        .color(u.muted)
        .pb(4.0),
    );
    for (index, spot) in spots.iter().enumerate() {
        list = list.child(
            row()
                .w_full()
                .h(40.0)
                .gap(12.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(spot_badge(u, index + 1, color))
                .child(
                    text(spot.label.clone())
                        .font(BODY)
                        .color(u.prose)
                        .truncate()
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(
                    text(crate::screens::settings::i18n::fill(
                        tr("{} {}"),
                        &[
                            &thousands(spot.count),
                            match layer {
                                Layer::Deaths => tr("deaths"),
                                Layer::Kills => tr("kills"),
                            },
                        ],
                    ))
                        .font(TNUM)
                        .color(u.muted),
                ),
        );
    }
    side = side.child(list).child(
        row()
            .gap(8.0)
            .center_y()
            .child(density_scale(color))
            .child(
                text(crate::screens::settings::i18n::fill(
                    tr("Fewer to more {}. {} map cells from the API."),
                    &[
                        match layer {
                            Layer::Deaths => tr("deaths"),
                            Layer::Kills => tr("kills"),
                        },
                        &thousands(heat.source_cells as u64),
                    ],
                ))
                .font(SMALL)
                .color(u.muted),
            ),
    );

    section.child(
        row()
            .w_full()
            .gap(32.0)
            .items_start()
            .child(map_canvas(u, heat, &v.objectives, layer, &spots))
            .child(side),
    )
}

fn big_number(u: &Ui, value: &str, label: &str) -> El {
    col()
        .gap(2.0)
        .child(text(value.to_string()).font(NUM).color(u.ink))
        .child(text(tr(label)).font(SMALL).color(u.muted))
}

fn spot_badge(u: &Ui, n: usize, color: Color) -> El {
    el().size(20.0, 20.0)
        .radius(10.0)
        .border(1.5, color)
        .center()
        .a11y_hidden()
        .child(text(n.to_string()).font(TNUM_SMALL).color(u.ink))
}

fn density_scale(color: Color) -> El {
    let mut shape = Shape::new();
    for step in 0..8 {
        let x = step as f32 * 8.0;
        shape = shape.fill(
            rect(x, 0.0, x + 7.0, 8.0),
            color.alpha(0.12 + 0.86 * (step as f32 / 7.0)),
        );
    }
    canvas(shape).size(63.0, 8.0).a11y_hidden()
}

/// The map: lanes drawn through the objectives, density cells, numbered hotspots.
fn map_canvas(
    u: &Ui,
    heat: &HeatGrid,
    objectives: &[MapObjective],
    layer: Layer,
    spots: &[Hotspot],
) -> El {
    let inset = 16.0;
    let size = MAP_SIZE - 2.0 * inset;
    let cell = size / heat.cols as f32;
    let color = match layer {
        Layer::Deaths => u.rose,
        Layer::Kills => u.teal,
    };
    let at = |left: f64, top: f64| [left as f32 * size, top as f32 * size];
    let mut shape = Shape::new();

    // Lane skeleton: each team's base, Patron, Walker and Guardian per lane.
    let find = |key: &str| objectives.iter().find(|o| o.key == key);
    for lane in ["1", "3", "4"] {
        let chain = [
            format!("team0_tier2_{lane}"),
            format!("team0_tier1_{lane}"),
            format!("team1_tier1_{lane}"),
            format!("team1_tier2_{lane}"),
        ];
        let points: Vec<[f32; 2]> = chain
            .iter()
            .filter_map(|key| find(key))
            .map(|o| at(o.left, o.top))
            .collect();
        shape = shape.stroke(points, 10.0, Color::WHITE.alpha(0.05));
        for team in ["team0", "team1"] {
            if let (Some(titan), Some(walker)) = (
                find(&format!("{team}_titan")),
                find(&format!("{team}_tier2_{lane}")),
            ) {
                shape = shape.stroke(
                    vec![at(titan.left, titan.top), at(walker.left, walker.top)],
                    10.0,
                    Color::WHITE.alpha(0.05),
                );
            }
        }
    }

    // A light blur (1 2 1 kernel) so the picture reads as areas, not single cells.
    let (cols, rows) = (heat.cols, heat.rows);
    let mut raw = vec![0f32; cols * rows];
    for c in &heat.cells {
        raw[c.cy * cols + c.cx] = match layer {
            Layer::Deaths => c.deaths,
            Layer::Kills => c.kills,
        } as f32;
    }
    let mut smooth = vec![0f32; cols * rows];
    let mut peak = 0f32;
    for y in 0..rows {
        for x in 0..cols {
            let mut sum = 0.0;
            let mut weight = 0.0;
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= cols as i32 || ny >= rows as i32 {
                        continue;
                    }
                    let k = [1.0, 2.0, 1.0][(dx + 1) as usize] * [1.0, 2.0, 1.0][(dy + 1) as usize];
                    sum += k * raw[ny as usize * cols + nx as usize];
                    weight += k;
                }
            }
            let value = sum / weight;
            smooth[y * cols + x] = value;
            peak = peak.max(value);
        }
    }
    for y in 0..rows {
        for x in 0..cols {
            let t = smooth[y * cols + x] / peak.max(1e-6);
            if t < 0.05 {
                continue;
            }
            let (x0, y0) = (x as f32 * cell, y as f32 * cell);
            shape = shape.fill(
                rect(x0, y0, x0 + cell, y0 + cell),
                color.alpha(0.08 + 0.82 * t.powf(0.8)),
            );
        }
    }

    for objective in objectives {
        let [x, y] = at(objective.left, objective.top);
        let r = if objective.key.contains("core") || objective.key.contains("titan") {
            4.0
        } else {
            3.0
        };
        shape = shape.stroke_with(
            vec![[x - r, y - r], [x + r, y - r], [x + r, y + r], [x - r, y + r]],
            true,
            1.25,
            rune::LineCap::Butt,
            u.prose.alpha(0.75),
        );
    }

    let mut field = el()
        .key("stats-map-canvas")
        .size(MAP_SIZE, MAP_SIZE)
        .radius(crate::ui::r(4.0))
        .fill(Color::BLACK.alpha(0.22))
        .border(1.0, u.hair)
        .role("img")
        .label(format!(
            "{} kills and {} deaths by map position",
            heat.kills, heat.deaths
        ))
        .child(
            canvas(shape)
                .size(size, size)
                .absolute(inset, inset)
                .a11y_hidden(),
        );
    for (index, spot) in spots.iter().enumerate() {
        let x = inset + (spot.cx as f32 + 0.5) * cell;
        let y = inset + (spot.cy as f32 + 0.5) * cell;
        field = field.child(
            spot_badge(u, index + 1, u.ink)
                .fill(u.bg.alpha(0.85))
                .absolute(x - 10.0, y - 10.0),
        );
    }
    field
}

