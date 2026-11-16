//! Character: reference-to-mesh tasks, stopped at a verified artifact.
//!
//! `deadlock_core::character` owns the task pipeline. This screen creates a
//! task, polls it, imports the GLB and shows the compile requirements. It never
//! claims the mesh is in game: the record carries `source2Compile: required`
//! and `inGameReady: false` until a real compile and an in-game check happen.

use rune::{Color, El, ScrollSpec, TextEdit, col, el, row, spacer, text};

use crate::app::State;
use crate::model::Screen;
use crate::ui::{ActionKind, BODY, BODY_MED, CHAT, H2, LABEL, MONO, SMALL, Tone, Ui};

#[derive(Clone)]
pub struct StageRow {
    pub id: String,
    pub title: String,
    pub state: String,
    pub backend: String,
    pub generative: bool,
    pub paid: bool,
    pub lossy: bool,
    pub note: Option<String>,
}

#[derive(Clone)]
pub struct Row {
    pub slug: String,
    pub prompt: String,
    pub provider: String,
    pub stage: String,
    pub status: String,
    pub progress: u32,
    pub created: String,
    pub error: Option<String>,
    pub model_bytes: u64,
    pub imported: bool,
    pub in_game_ready: bool,
    pub reference: Option<String>,
    pub hero: Option<String>,
    pub backend: String,
    pub stages: Vec<StageRow>,
}

pub struct Load {
    pub rows: Vec<Row>,
    pub workspace: String,
}

pub fn row_from_record(record: &deadlock_core::character::CharacterTaskRecord) -> Row {
    Row {
        slug: record.slug.clone(),
        prompt: record.request.prompt.clone(),
        provider: record.request.provider.as_str().to_string(),
        stage: record.task.stage.clone(),
        status: record.task.status.as_str().to_string(),
        progress: record.task.progress,
        created: record
            .created_at
            .chars()
            .take(16)
            .collect::<String>()
            .replace('T', " "),
        error: record.task.error.clone(),
        model_bytes: record.asset.as_ref().map(|asset| asset.bytes).unwrap_or(0),
        imported: record.asset.is_some(),
        in_game_ready: false,
        reference: record
            .request
            .reference_image
            .as_ref()
            .map(|path| path.display().to_string()),
        hero: record.request.target_hero.clone(),
        backend: if record.backend.is_empty() {
            "none".to_string()
        } else {
            record.backend.clone()
        },
        stages: record
            .stages
            .iter()
            .map(|stage| StageRow {
                id: stage.id.id().to_string(),
                title: stage.id.title().to_string(),
                state: stage.state.as_str().to_string(),
                backend: stage.backend.clone(),
                generative: stage.id.is_generative(),
                paid: stage.id.is_paid(),
                lossy: stage.id.is_lossy(),
                note: stage.notes.first().cloned(),
            })
            .collect(),
    }
}

pub struct View {
    pub loaded: bool,
    pub rows: Vec<Row>,
    pub workspace: String,
    pub error: Option<String>,
    pub create_open: bool,
    pub prompt: TextEdit,
    pub reference: TextEdit,
    pub hero: TextEdit,
    pub style: TextEdit,
    pub consent: bool,
    pub busy: bool,
    pub last: Option<String>,
}

impl Default for View {
    fn default() -> Self {
        View {
            loaded: false,
            rows: Vec::new(),
            workspace: String::new(),
            error: None,
            create_open: false,
            prompt: TextEdit::new(""),
            reference: TextEdit::new(""),
            hero: TextEdit::new(""),
            style: TextEdit::new("stylized game character, A-pose"),
            consent: false,
            busy: false,
            last: None,
        }
    }
}

pub fn view(s: &State, u: &Ui) -> El {
    let v = &s.character;
    let trailing = row()
        .gap(8.0)
        .center_y()
        .child(if s.character_job.is_running() || v.busy {
            u.status_chip("working", Tone::Warn)
        } else if v.error.is_some() {
            u.status_chip("read failed", Tone::Bad)
        } else if v.loaded {
            u.status_chip(
                &format!("{} task(s)", v.rows.len()),
                if v.rows.is_empty() { Tone::Quiet } else { Tone::Accent },
            )
        } else {
            u.status_chip("not read", Tone::Quiet)
        })
        .child(
            u.pill("Reload", "↻", ActionKind::Ghost)
                .key("character-reload")
                .on_press(|st: &mut State| crate::data::start_character_read(st)),
        )
        .child(
            u.action(
                if v.create_open { "Close form" } else { "New character" },
                ActionKind::Primary,
            )
            .key("character-new")
            .on_press(|st: &mut State| st.character.create_open = !st.character.create_open),
        );

    let detail = if let Some(error) = &v.error {
        error.clone()
    } else if !v.loaded {
        "Reading generated character tasks from the workspace.".to_string()
    } else {
        format!(
            "{} · {} stage graph · stopped at a GLB asset · a Source 2 compile and an in-game check are always required",
            v.workspace,
            deadlock_core::character::stage_graph().len()
        )
    };

    let mut page = col()
        .key("character-page")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .px(24.0)
        .pt(18.0)
        .pb(16.0)
        .gap(13.0)
        .child(u.glow(
            "character-glow",
            [0.2, 0.02],
            (700.0, 420.0),
            u.glow_cool,
        ))
        .child(u.page_head(
            Screen::Character.kicker(),
            "Character",
            &detail,
            Some(trailing),
        ));

    if let Some(last) = &v.last {
        page = page.child(result_strip(u, last));
    }
    if v.create_open {
        page = page.child(create_panel(s, u));
    }

    let body = if !v.loaded && s.character_job.is_running() {
        u.note("Reading generated/characters.")
    } else if v.rows.is_empty() {
        u.empty_state(
            "◭",
            "No character tasks",
            "A task takes a prompt or a reference image to a mesh provider, downloads the GLB and records the compile work that remains.",
            Some(
                u.action("Start a character", ActionKind::Secondary)
                    .on_press(|st: &mut State| st.character.create_open = true),
            ),
        )
    } else {
        let mut rows = col().w_full().gap(10.0);
        for (index, row) in v.rows.iter().enumerate() {
            rows = rows.child(card(s, u, index, row));
        }
        col()
            .key("character-scroll")
            .grow(1.0)
            .min_h(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(rows)
    };

    page.child(body)
}

fn result_strip(u: &Ui, message: &str) -> El {
    let bad = message.contains("not ")
        || message.contains("could not")
        || message.contains("failed")
        || message.contains("refus")
        || message.contains("required");
    row()
        .key("character-result")
        .w_full()
        .padding(10.0)
        .gap(10.0)
        .items_start()
        .radius(9.0)
        .fill(if bad {
            u.amber.alpha(0.06)
        } else {
            u.teal.alpha(0.06)
        })
        .border(
            1.0,
            if bad {
                u.amber.alpha(0.20)
            } else {
                u.teal.alpha(0.18)
            },
        )
        .child(u.dot(if bad { Tone::Warn } else { Tone::Ok }))
        .child(text(message.to_string()).font(SMALL).color(u.muted).wrap())
}

fn create_panel(s: &State, u: &Ui) -> El {
    let v = &s.character;
    let has_prompt = !v.prompt.text.trim().is_empty();
    let needs_consent = !v.reference.text.trim().is_empty() && !v.consent;
    let ready = has_prompt && !needs_consent && !v.busy;

    row()
        .key("character-create")
        .w_full()
        .gap(16.0)
        .items_start()
        .padding(16.0)
        .radius(14.0)
        .fill(Color::WHITE.alpha(0.03))
        .border(1.0, u.violet.alpha(0.16))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(10.0)
                .child(
                    row()
                        .w_full()
                        .gap(9.0)
                        .center_y()
                        .child(u.eyebrow("NEW TASK"))
                        .child(spacer())
                        .child(u.chip(
                            "inGameReady false by design",
                            u.amber,
                            u.amber.alpha(0.10),
                        )),
                )
                .child(u.field_label("PROMPT"))
                .child(
                    rune::text_input(&v.prompt, CHAT)
                        .key("character-prompt")
                        .label("Character prompt")
                        .grow(1.0)
                        .min_h(64.0)
                        .color(u.ink)
                        .placeholder(
                            "a slim pilot in a green coat, worn leather, stylized proportions",
                            u.faint,
                        )
                        .caret(u.accent, u.accent.alpha(0.20))
                        .on_edit(|st: &mut State| &mut st.character.prompt),
                )
                .child(
                    row()
                        .w_full()
                        .gap(10.0)
                        .items_start()
                        .child(
                            col()
                                .grow(1.0)
                                .gap(4.0)
                                .child(u.field_label("REFERENCE IMAGE (OPTIONAL)"))
                                .child(
                                    rune::text_input(&v.reference, BODY)
                                        .key("character-reference")
                                        .label("Reference image path")
                                        .single_line()
                                        .color(u.ink)
                                        .placeholder("/path/to/reference.png", u.faint)
                                        .caret(u.accent, u.accent.alpha(0.20))
                                        .on_edit(|st: &mut State| &mut st.character.reference),
                                ),
                        )
                        .child(
                            col()
                                .w(150.0)
                                .gap(4.0)
                                .child(u.field_label("HERO"))
                                .child(
                                    rune::text_input(&v.hero, BODY)
                                        .key("character-hero")
                                        .label("Target hero")
                                        .single_line()
                                        .color(u.ink)
                                        .placeholder("billy", u.faint)
                                        .caret(u.accent, u.accent.alpha(0.20))
                                        .on_edit(|st: &mut State| &mut st.character.hero),
                                ),
                        ),
                )
                .child(
                    row()
                        .w_full()
                        .gap(10.0)
                        .items_start()
                        .child(
                            col()
                                .grow(1.0)
                                .gap(4.0)
                                .child(u.field_label("STYLE"))
                                .child(
                                    rune::text_input(&v.style, BODY)
                                        .key("character-style")
                                        .label("Style")
                                        .single_line()
                                        .color(u.ink)
                                        .placeholder("stylized game character", u.faint)
                                        .caret(u.accent, u.accent.alpha(0.20))
                                        .on_edit(|st: &mut State| &mut st.character.style),
                                ),
                        )
                        .child(
                            col()
                                .w(190.0)
                                .gap(4.0)
                                .child(u.field_label("REFERENCE UPLOAD"))
                                .child(
                                    row()
                                        .h(30.0)
                                        .gap(8.0)
                                        .center_y()
                                        .child(u.toggle(v.consent))
                                        .child(
                                            text(if v.consent {
                                                "consent recorded"
                                            } else {
                                                "not consented"
                                            })
                                            .font(SMALL)
                                            .color(if v.consent { u.teal } else { u.muted }),
                                        ),
                                ),
                        ),
                )
                .child(
                    text("The reference image leaves this machine only with the consent toggle on. A task without a reference is text to 3D.")
                        .font(LABEL)
                        .color(u.faint)
                        .wrap(),
                ),
        )
        .child(
            col()
                .w(240.0)
                .gap(9.0)
                .padding(13.0)
                .radius(12.0)
                .fill(u.field)
                .border(1.0, u.hair)
                .child(text("What comes back").font(BODY_MED).color(u.ink))
                .child(u.kv("Result", "GLB mesh", u.muted))
                .child(u.kv("Record", "character.json", u.muted))
                .child(u.kv("Compile", "required", u.amber))
                .child(u.divider())
                .child(
                    text("No rig, no retarget, no vmdl or material compile, and no in-game check happens in this pipeline. The import records the work that remains.")
                        .font(LABEL)
                        .color(u.faint)
                        .wrap(),
                )
                .child(
                    u.action("Create task", ActionKind::Primary)
                        .w_full()
                        .disabled(!ready)
                        .opacity(if ready { 1.0 } else { 0.45 })
                        .on_press(|st: &mut State| crate::data::create_character_task(st)),
                )
                .child(if needs_consent {
                    text("Turn on reference upload consent first.")
                        .font(LABEL)
                        .color(u.amber)
                        .wrap()
                } else {
                    el()
                }),
        )
}

fn card(s: &State, u: &Ui, index: usize, item: &Row) -> El {
    let tone = match item.status.as_str() {
        "SUCCEEDED" => Tone::Ok,
        "FAILED" | "CANCELED" => Tone::Bad,
        "IN_PROGRESS" => Tone::Warn,
        _ => Tone::Info,
    };
    let mut card = col()
        .key(format!("character-{}", item.slug))
        .w_full()
        .gap(8.0)
        .padding(14.0)
        .radius(13.0)
        .fill(Color::WHITE.alpha(0.024))
        .border(
            1.0,
            if item.status == "SUCCEEDED" {
                u.teal.alpha(0.16)
            } else {
                u.hair
            },
        )
        .child(
            row()
                .w_full()
                .gap(10.0)
                .items_start()
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(3.0)
                        .child(
                            text(item.prompt.clone())
                                .font(BODY_MED)
                                .color(u.ink)
                                .wrap(),
                        )
                        .child(
                            text(format!(
                                "{} · {} · created {}",
                                item.slug, item.provider, item.created
                            ))
                            .font(LABEL)
                            .color(u.faint)
                            .truncate(),
                        ),
                )
                .child(u.status_chip(&item.status.to_lowercase(), tone))
                .child(u.chip(
                    if item.stage.is_empty() {
                        "stage unknown"
                    } else {
                        &item.stage
                    },
                    u.violet,
                    u.violet.alpha(0.10),
                )),
        );

    card = card.child(stage_rail(u, item));

    let unsupported: Vec<&StageRow> = item
        .stages
        .iter()
        .filter(|stage| stage.state == "unsupported")
        .collect();
    if let Some(first) = unsupported.first() {
        card = card.child(
            row()
                .w_full()
                .gap(10.0)
                .items_start()
                .padding(9.0)
                .radius(8.0)
                .fill(u.amber.alpha(0.05))
                .border(1.0, u.amber.alpha(0.16))
                .child(u.dot(Tone::Warn))
                .child(
                    col()
                        .grow(1.0)
                        .gap(2.0)
                        .child(
                            text(format!("{} is unsupported on this machine", first.title))
                                .font(SMALL)
                                .color(u.muted),
                        )
                        .child(
                            text(
                                first
                                    .note
                                    .clone()
                                    .unwrap_or_else(|| "No backend is configured for this stage.".to_string()),
                            )
                            .font(LABEL)
                            .color(u.faint)
                            .wrap(),
                        ),
                ),
        );
    }

    let mut meta = row().w_full().gap(8.0).center_y();
    if item.imported {
        meta = meta.child(u.status_chip("GLB imported", Tone::Ok));
        if item.model_bytes > 0 {
            meta = meta.child(u.chip(
                &crate::model::format_size(item.model_bytes),
                u.muted,
                Color::WHITE.alpha(0.04),
            ));
        }
    } else {
        meta = meta.child(u.status_chip("no asset yet", Tone::Quiet));
    }
    meta = meta.child(u.status_chip("compile required", Tone::Warn));
    meta = meta.child(u.status_chip("not in game", Tone::Quiet));
    if let Some(hero) = &item.hero {
        meta = meta.child(u.chip(hero, u.faint, Color::WHITE.alpha(0.04)));
    }
    if let Some(reference) = &item.reference {
        meta = meta.child(
            text(reference.clone())
                .font(MONO)
                .color(u.faint)
                .grow(1.0)
                .truncate(),
        );
    } else {
        meta = meta.child(spacer());
    }
    card = card.child(meta);

    if let Some(error) = &item.error {
        card = card.child(
            text(error.clone())
                .font(SMALL)
                .color(u.rose)
                .wrap(),
        );
    }

    card.child(
        row()
            .w_full()
            .pt(8.0)
            .gap(8.0)
            .center_y()
            .border_sides([1.0, 0.0, 0.0, 0.0], u.hair)
            .child(
                text("source2Compile: required · inGameReady: false")
                    .font(MONO)
                    .color(u.amber.alpha(0.85))
                    .grow(1.0)
                    .truncate(),
            )
            .child(
                u.action("Advance", ActionKind::Secondary)
                    .key(format!("character-advance-{}", item.slug))
                    .disabled(s.character.busy)
                    .on_press(move |st: &mut State| crate::data::advance_character(st, index)),
            )
            .child(
                u.action("Import", ActionKind::Primary)
                    .key(format!("character-import-{}", item.slug))
                    .disabled(s.character.busy || item.status != "SUCCEEDED")
                    .opacity(if item.status == "SUCCEEDED" { 1.0 } else { 0.45 })
                    .on_press(move |st: &mut State| crate::data::import_character_asset(st, index)),
            ),
    )
}

fn stage_rail(u: &Ui, item: &Row) -> El {
    let mut rail = row().w_full().gap(3.0);
    for stage in &item.stages {
        let color = stage_color(u, &stage.state);
        rail = rail.child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(3.0)
                .child(el().w_full().h(5.0).radius(2.5).fill(color.alpha(0.85)))
                .child(
                    text(short_stage(&stage.id))
                        .font(LABEL)
                        .color(if stage.state == "unsupported" {
                            u.amber
                        } else {
                            u.faint
                        })
                        .truncate(),
                ),
        );
    }
    let succeeded = item
        .stages
        .iter()
        .filter(|stage| stage.state == "succeeded")
        .count();
    let unsupported = item
        .stages
        .iter()
        .filter(|stage| stage.state == "unsupported")
        .count();
    col()
        .w_full()
        .gap(5.0)
        .child(rail)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(u.chip(
                    &format!("{succeeded}/{} stages", item.stages.len()),
                    u.teal,
                    u.teal.alpha(0.10),
                ))
                .child(u.chip(
                    &format!("{unsupported} unsupported"),
                    if unsupported == 0 { u.faint } else { u.amber },
                    Color::WHITE.alpha(0.04),
                ))
                .child(u.chip(
                    &format!("backend {}", item.backend),
                    u.violet,
                    u.violet.alpha(0.10),
                ))
                .child(spacer())
                .child(
                    text(
                        item.stages
                            .iter()
                            .find(|stage| stage.state == "unsupported")
                            .map(|stage| format!("next: {}", stage.title))
                            .unwrap_or_else(|| "all stages resolved".to_string()),
                    )
                    .font(LABEL)
                    .color(u.faint),
                ),
        )
}

fn stage_color(u: &Ui, state: &str) -> Color {
    match state {
        "succeeded" => u.teal,
        "running" => u.amber,
        "failed" => u.rose,
        "unsupported" => u.amber,
        "pending" => u.violet,
        _ => u.faint,
    }
}

fn short_stage(id: &str) -> String {
    match id {
        "prepare_reference" => "prep".to_string(),
        "generate_views" => "views".to_string(),
        "reconstruct" => "mesh".to_string(),
        "cleanup" => "weld".to_string(),
        "uv_bake" => "uv".to_string(),
        "pbr_synthesize" => "pbr".to_string(),
        "retopo" => "retopo".to_string(),
        "rig" => "rig".to_string(),
        "animate_retarget" => "anim".to_string(),
        "source2_map" => "s2".to_string(),
        "export" => "glb".to_string(),
        other => other.chars().take(5).collect(),
    }
}

#[allow(dead_code)]
fn _tone_bg(u: &Ui, tone: Tone) -> Color {
    tone.bg(u)
}
