//! Imported demos and the review step: the moment end of the drill-down.

use rune::{El, Key, KeyEvent, ScrollSpec, col, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, MONO, SMALL, Ui};

use super::german::tr;
use super::shared::*;
use super::ReviewPanel;

// --- demos and review ------------------------------------------------------

/// Imported demos and the review step: the "moment" end of the drill-down.
pub(super) fn demo_section(s: &State, u: &Ui) -> El {
    let v = &s.stats;
    let can_import = !v.demo_busy && !v.demo_path.text.trim().is_empty();
    let mut section = col()
        .key("stats-demos")
        .w_full()
        .gap(12.0)
        .child(section_head(u, "Match reviews", "From a demo file you import"))
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(
                    row()
                        .h(32.0)
                        .px(10.0)
                        .radius(crate::ui::r(8.0))
                        .fill(u.well)
                        .border(1.0, u.hair)
                        .grow(1.0)
                        .min_w(0.0)
                        .center_y()
                        .child(
                            rune::text_input(&v.demo_path, BODY)
                                .key("demo-path")
                                .label(tr("Demo file path"))
                                .single_line()
                                .grow(1.0)
                                .min_w(0.0)
                                .color(u.ink)
                                .placeholder(tr("Path to a .dem file"), u.faint)
                                .caret(u.accent, u.accent.alpha(0.20))
                                .on_edit(|st: &mut State| &mut st.stats.demo_path),
                        ),
                )
                .child(
                    u.action(tr("Import"), ActionKind::Secondary)
                        .key("demo-import")
                        .disabled(!can_import)
                        .opacity(if can_import { 1.0 } else { 0.5 })
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| crate::data::import_demo_file(st)),
                )
                .child(
                    u.icon_button(&icons::REFRESH_CW, tr("Reload demos"))
                        .key("demo-reload")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| crate::data::start_demo_list(st)),
                ),
        );

    if let Some(error) = &v.demo_error {
        section = section.child(problem(u, error));
    }

    if v.demos.is_empty() {
        return section.child(
            text(tr("Import a demo to go from a match to its moments, with your model's review."))
                .font(SMALL)
                .color(u.muted),
        );
    }

    let mut list = col().w_full().border_sides([1.0, 0.0, 0.0, 0.0], u.hair);
    for item in v.demos.iter().take(8) {
        let mut meta = vec![
            item.map
                .clone()
                .unwrap_or_else(|| tr("Unknown map").to_string()),
        ];
        if let Some(seconds) = item.duration_seconds {
            meta.push(format!("{}m", seconds / 60));
        }
        if item.players > 0 {
            meta.push(format!(
                "{} player{}",
                item.players,
                if item.players == 1 { "" } else { "s" }
            ));
        }
        meta.push(item.updated.clone());
        list = list.child(
            row()
                .key(format!("demo-{}", item.match_id))
                .w_full()
                .h(52.0)
                .gap(16.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(
                            text(format!("Match {}", item.match_id))
                                .font(BODY_MED)
                                .color(u.ink)
                                .truncate(),
                        )
                        .child(
                            text(meta.join("  ·  "))
                                .font(SMALL)
                                .color(u.muted)
                                .truncate(),
                        ),
                )
                .child(
                    u.action(tr("Review"), ActionKind::Secondary)
                        .key(format!("demo-review-{}", item.match_id))
                        .cursor(rune::Cursor::Pointer)
                        .on_press({
                            let id = item.match_id.clone();
                            move |st: &mut State| crate::data::open_review(st, &id)
                        }),
                ),
        );
    }
    section.child(list)
}

pub(super) fn review_dialog(s: &State, u: &Ui, review: &ReviewPanel) -> El {
    use crate::ui::Tone as UiTone;
    let endpoint = s.local_endpoint.text.trim().to_string();
    let model = s.agent.model.clone().or_else(|| {
        if s.model_label.is_empty() || s.model_label == "No model" {
            None
        } else {
            Some(s.model_label.clone())
        }
    });
    let can_generate = !endpoint.is_empty() && model.is_some() && !s.model_review_job.is_running();

    let mut facts = col().w_full().gap(2.0);
    for fact in review.facts.facts.iter().take(6) {
        facts = facts.child(
            text(fact.text.clone())
                .font(MONO)
                .color(u.prose)
                .wrap_anywhere(),
        );
    }
    let more = review.facts.facts.len().saturating_sub(6);
    if more > 0 {
        facts = facts.child(text(format!("{more} more")).font(SMALL).color(u.muted));
    }

    let label = |t: &str| text(tr(t)).font(SMALL).color(u.muted);

    let mut body = col()
        .w_full()
        .gap(16.0)
        .child(
            col()
                .w_full()
                .gap(6.0)
                .child(text(format!("Review match {}", review.match_id)).font(H1).color(u.ink))
                .child(
                    text(tr("Generate a review or paste one. Numbers the match does not back up are refused."))
                        .font(BODY)
                        .color(u.muted)
                        .wrap(),
                ),
        )
        .child(col().w_full().gap(6.0).child(label("Facts")).child(facts));

    if !review.facts.absent.is_empty() {
        body = body.child(u.kv(tr("Not in the demo"), &review.facts.absent.join(", "), u.muted));
    }

    body = body
        .child(
            col().w_full().gap(6.0).child(label("Prompt")).child(
                col()
                    .key("review-prompt")
                    .w_full()
                    .h(96.0)
                    .padding(12.0)
                    .radius(crate::ui::r(8.0))
                    .fill(u.well)
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
            ),
        )
        .child(
            col()
                .w_full()
                .gap(6.0)
                .child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .center_y()
                        .child(label("Review"))
                        .child(spacer())
                        .child(
                            text(match &model {
                                Some(model) if !endpoint.is_empty() => model.clone(),
                                Some(_) => tr("Set a local model endpoint to generate").to_string(),
                                None => tr("No model selected").to_string(),
                            })
                            .font(SMALL)
                            .color(u.muted)
                            .truncate(),
                        )
                        .child(
                            u.action(
                                if s.model_review_job.is_running() {
                                    tr("Generating")
                                } else {
                                    tr("Generate")
                                },
                                ActionKind::Secondary,
                            )
                            .disabled(!can_generate)
                            .opacity(if can_generate { 1.0 } else { 0.5 })
                            .cursor(rune::Cursor::Pointer)
                            .on_press(|st: &mut State| crate::data::start_review_generation(st)),
                        ),
                )
                .child(
                    rune::text_input(&review.body, BODY)
                        .key("review-body")
                        .label(tr("Review reply"))
                        .w_full()
                        .h(96.0)
                        .padding(12.0)
                        .radius(crate::ui::r(8.0))
                        .fill(u.well)
                        .border(1.0, u.hair)
                        .color(u.ink)
                        .placeholder(tr("Paste the reply here"), u.faint)
                        .caret(u.accent, u.accent.alpha(0.20))
                        .on_edit(|st: &mut State| {
                            st.stats
                                .review
                                .as_mut()
                                .map(|review| &mut review.body)
                                .unwrap_or(&mut st.stats.demo_path)
                        }),
                ),
        );

    if let Some(log) = &review.log
        && matches!(review.tone, UiTone::Warn | UiTone::Bad | UiTone::Ok)
    {
        body = body.child(
            text(log.clone())
                .font(SMALL)
                .color(review.tone.fg(u))
                .wrap(),
        );
    }

    let close = |st: &mut State| st.stats.review = None;
    body = body.child(
        row()
            .w_full()
            .gap(8.0)
            .pt(8.0)
            .child(spacer())
            .child(
                u.action(tr("Close"), ActionKind::Ghost)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(close),
            )
            .child(
                u.action(tr("Check and save"), ActionKind::Primary)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| crate::data::save_review(st)),
            ),
    );

    u.dialog_shell()
        .key("review-dialog")
        .role("dialog")
        .label(tr("Review a match"))
        .w(640.0)
        .max_w(640.0)
        .padding(24.0)
        .modal(true, close)
        .on_key(move |st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                close(st);
                true
            } else {
                false
            }
        })
        .child(body)
}

