//! Character: prompt or reference image to a mesh, one task per character.
//!
//! `deadlock_core::character` owns the task pipeline. This screen creates a task, advances it,
//! imports the GLB and lists each stage and whether this machine can run it. A mesh is never
//! marked ready for the game: a Source 2 compile and an in-game check always remain.

use rune::{Color, El, ScrollSpec, TextEdit, col, el, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::screens::subpage;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, MONO, SMALL, Ui};

#[derive(Clone)]
pub struct StageRow {
    pub id: String,
    pub title: String,
    pub state: String,
    pub note: Option<String>,
}

#[derive(Clone)]
pub struct Row {
    pub slug: String,
    pub prompt: String,
    pub provider: String,
    pub stage: String,
    pub status: String,
    pub created: String,
    pub error: Option<String>,
    pub model_bytes: u64,
    pub imported: bool,
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
        created: record
            .created_at
            .chars()
            .take(16)
            .collect::<String>()
            .replace('T', " "),
        error: record.task.error.clone(),
        model_bytes: record.asset.as_ref().map(|asset| asset.bytes).unwrap_or(0),
        imported: record.asset.is_some(),
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
    /// The task shown in the detail pane.
    pub selected: usize,
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
            selected: 0,
        }
    }
}

pub fn view(s: &State, u: &Ui) -> El {
    let v = &s.character;
    let working = s.character_job.is_running() || v.busy;

    let mut actions = row().gap(8.0).center_y().child(
        u.icon_button(&icons::REFRESH_CW, "Reload")
            .key("character-reload")
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| crate::data::start_character_read(st)),
    );
    if !v.create_open {
        actions = actions.child(
            u.icon_action(&icons::PLUS, "New character", ActionKind::Primary)
                .key("character-new")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| st.character.create_open = true),
        );
    }

    let detail = if let Some(error) = &v.error {
        error.clone()
    } else if working {
        "Working".to_string()
    } else if v.loaded && !v.rows.is_empty() {
        format!(
            "{} task{}",
            v.rows.len(),
            if v.rows.len() == 1 { "" } else { "s" }
        )
    } else {
        String::new()
    };
    let mut head =
        col()
            .w_full()
            .gap(16.0)
            .child(subpage::head(u, "Character", &detail, Some(actions)));
    if let Some(last) = &v.last {
        head = head.child(result_line(u, last));
    }

    if v.create_open {
        return subpage::page("character", head, create_form(s, u));
    }

    if v.rows.is_empty() {
        let body = if !v.loaded && s.character_job.is_running() {
            el()
        } else {
            u.empty_state(
                "",
                "No characters yet",
                "Describe a character or pick a reference image, and Sigil turns it into a mesh.",
                Some(
                    u.action("New character", ActionKind::Secondary)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| st.character.create_open = true),
                ),
            )
        };
        return subpage::page("character", head, body);
    }

    let selected = v.selected.min(v.rows.len() - 1);
    subpage::fixed_page(
        "character",
        head,
        row()
            .key("character-body")
            .w_full()
            .min_h(0.0)
            .child(task_list(u, &v.rows, selected))
            .child(task_detail(s, u, selected, &v.rows[selected])),
    )
}

fn result_line(u: &Ui, message: &str) -> El {
    let bad = message.contains("not ")
        || message.contains("could not")
        || message.contains("failed")
        || message.contains("refus");
    text(message.to_string())
        .key("character-result")
        .font(BODY)
        .color(if bad { u.amber } else { u.muted })
        .wrap()
}

fn create_form(s: &State, u: &Ui) -> El {
    let v = &s.character;
    let has_prompt = !v.prompt.text.trim().is_empty();
    let needs_consent = !v.reference.text.trim().is_empty() && !v.consent;
    let ready = has_prompt && !needs_consent && !v.busy;

    let input = |value: &TextEdit,
                 key: &str,
                 label: &str,
                 placeholder: &str,
                 edit: fn(&mut State) -> &mut TextEdit| {
        row()
            .grow(1.0)
            .min_w(0.0)
            .h(32.0)
            .px(10.0)
            .center_y()
            .radius(crate::ui::r(8.0))
            .fill(u.well)
            .border(1.0, u.hair)
            .child(
                rune::text_input(value, BODY)
                    .key(key.to_string())
                    .label(label.to_string())
                    .single_line()
                    .grow(1.0)
                    .min_w(0.0)
                    .color(u.ink)
                    .placeholder(placeholder.to_string(), u.faint)
                    .caret(u.accent, u.accent.alpha(0.20))
                    .on_edit(edit),
            )
    };

    let prompt = row()
        .grow(1.0)
        .min_w(0.0)
        .min_h(80.0)
        .padding(10.0)
        .radius(crate::ui::r(8.0))
        .fill(u.well)
        .border(1.0, u.hair)
        .child(
            rune::text_input(&v.prompt, BODY)
                .key("character-prompt")
                .label("Character prompt")
                .grow(1.0)
                .min_w(0.0)
                .min_h(60.0)
                .color(u.ink)
                .placeholder(
                    "A slim pilot in a green coat, worn leather, stylized proportions",
                    u.faint,
                )
                .caret(u.accent, u.accent.alpha(0.20))
                .on_edit(|st: &mut State| &mut st.character.prompt),
        );

    let consent = row()
        .grow(1.0)
        .gap(12.0)
        .center_y()
        .child(
            u.toggle(v.consent)
                .key("character-consent")
                .label("Allow uploading the reference image")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| st.character.consent = !st.character.consent),
        )
        .child(
            text("Send the reference image to the mesh provider")
                .font(BODY)
                .color(u.prose),
        );

    let mut c = col()
        .key("character-create")
        .w_full()
        .max_w(760.0)
        .gap(8.0)
        .child(u.section("New character"))
        .child(
            subpage::rows(u)
                .child(form_row(u, "Prompt", prompt, true))
                .child(form_row(
                    u,
                    "Reference image",
                    input(
                        &v.reference,
                        "character-reference",
                        "Reference image path",
                        "Optional, full path to a PNG or JPG",
                        |st| &mut st.character.reference,
                    ),
                    false,
                ))
                .child(form_row(
                    u,
                    "Hero",
                    input(
                        &v.hero,
                        "character-hero",
                        "Target hero",
                        "Optional, for example billy",
                        |st| &mut st.character.hero,
                    ),
                    false,
                ))
                .child(form_row(
                    u,
                    "Style",
                    input(
                        &v.style,
                        "character-style",
                        "Style",
                        "Stylized game character",
                        |st| &mut st.character.style,
                    ),
                    false,
                ))
                .child(form_row(u, "Upload", consent, false)),
        );
    if needs_consent {
        c = c.child(
            text("Allow the upload to use a reference image, or clear the path.")
                .font(BODY)
                .color(u.amber)
                .pt(4.0),
        );
    }
    c.child(
        row()
            .w_full()
            .pt(8.0)
            .gap(8.0)
            .child(spacer())
            .child(
                u.action("Cancel", ActionKind::Ghost)
                    .key("character-cancel")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| st.character.create_open = false),
            )
            .child(
                u.action("Create", ActionKind::Primary)
                    .key("character-create-task")
                    .disabled(!ready)
                    .opacity(if ready { 1.0 } else { 0.45 })
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| crate::data::create_character_task(st)),
            ),
    )
}

fn form_row(u: &Ui, label: &str, control: El, top: bool) -> El {
    let r = row()
        .w_full()
        .min_h(48.0)
        .py(8.0)
        .gap(16.0)
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair);
    let r = if top { r.items_start() } else { r.center_y() };
    r.child(
        text(label.to_string())
            .font(BODY)
            .color(u.muted)
            .w(140.0)
            .mt(if top { 6.0 } else { 0.0 }),
    )
    .child(control)
}

fn status_text(u: &Ui, status: &str) -> (&'static str, Color) {
    match status {
        "SUCCEEDED" => ("Mesh ready", u.muted),
        "FAILED" => ("Failed", u.rose),
        "CANCELED" => ("Canceled", u.muted),
        "IN_PROGRESS" => ("Generating", u.ink),
        "PENDING" => ("Queued", u.muted),
        _ => ("Not started", u.muted),
    }
}

fn task_list(u: &Ui, rows: &[Row], selected: usize) -> El {
    let mut list = col()
        .key("character-list-inner")
        .w_full()
        .pr(12.0)
        .pb(24.0)
        .gap(2.0);
    for (index, item) in rows.iter().enumerate() {
        let on = index == selected;
        let (status, color) = status_text(u, &item.status);
        list = list.child(
            col()
                .key(format!("character-{}", item.slug))
                .w_full()
                .h(56.0)
                .px(12.0)
                .gap(2.0)
                .justify_center()
                .radius(crate::ui::r(8.0))
                .fill(if on { u.selected } else { Color::TRANSPARENT })
                .hover(u.hover)
                .focusable(true)
                .role("button")
                .label(item.prompt.clone())
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| st.character.selected = index)
                .child(
                    text(item.prompt.clone())
                        .font(BODY_MED)
                        .color(if on { u.ink } else { u.prose })
                        .truncate(),
                )
                .child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .child(text(item.created.clone()).font(SMALL).color(u.faint))
                        .child(spacer())
                        .child(text(status).font(SMALL).color(color)),
                ),
        );
    }
    col()
        .key("character-list")
        .w(320.0)
        .h_full()
        .min_h(0.0)
        .border_sides([0.0, 1.0, 0.0, 0.0], u.hair)
        .child(
            col()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(list),
        )
}

fn task_detail(s: &State, u: &Ui, index: usize, item: &Row) -> El {
    let (status, status_color) = status_text(u, &item.status);
    let busy = s.character.busy;
    let can_import = item.status == "SUCCEEDED";

    let mut facts = subpage::rows(u)
        .child(subpage::kv(
            u,
            "Status",
            row()
                .gap(8.0)
                .child(text(status).font(BODY).color(status_color))
                .child(text(item.stage.clone()).font(BODY).color(u.faint)),
        ))
        .child(subpage::kv(
            u,
            "Provider",
            subpage::val(u, item.provider.clone()),
        ))
        .child(subpage::kv(
            u,
            "Backend",
            subpage::val(u, item.backend.clone()),
        ))
        .child(subpage::kv(
            u,
            "Mesh",
            if item.imported {
                subpage::val(
                    u,
                    if item.model_bytes > 0 {
                        format!("GLB, {}", crate::model::format_size(item.model_bytes))
                    } else {
                        "GLB".to_string()
                    },
                )
            } else {
                text("Not imported yet").font(BODY).color(u.muted)
            },
        ));
    if let Some(hero) = &item.hero {
        facts = facts.child(subpage::kv(u, "Hero", subpage::val(u, hero.clone())));
    }
    if let Some(reference) = &item.reference {
        facts = facts.child(subpage::kv(
            u,
            "Reference",
            text(reference.clone()).font(MONO).color(u.prose).truncate(),
        ));
    }
    facts = facts.child(subpage::kv(
        u,
        "Created",
        text(item.created.clone()).font(BODY).color(u.muted),
    ));

    let mut stages = subpage::rows(u);
    for stage in &item.stages {
        stages = stages.child(stage_line(u, stage));
    }

    let mut page = col()
        .key("character-detail-body")
        .max_w(800.0)
        .pl(32.0)
        .pr(8.0)
        .pb(32.0)
        .gap(24.0)
        .child(
            row()
                .w_full()
                .gap(16.0)
                .items_start()
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(4.0)
                        .child(text(item.prompt.clone()).font(H1).color(u.ink).wrap())
                        .child(
                            text(item.slug.clone())
                                .font(SMALL)
                                .color(u.faint)
                                .truncate(),
                        ),
                )
                .child(
                    row()
                        .gap(8.0)
                        .child(
                            u.action("Advance", ActionKind::Secondary)
                                .key(format!("character-advance-{}", item.slug))
                                .disabled(busy)
                                .cursor(rune::Cursor::Pointer)
                                .on_press(move |st: &mut State| {
                                    crate::data::advance_character(st, index)
                                }),
                        )
                        .child(
                            u.action("Import mesh", ActionKind::Secondary)
                                .key(format!("character-import-{}", item.slug))
                                .disabled(busy || !can_import)
                                .opacity(if can_import { 1.0 } else { 0.45 })
                                .cursor(rune::Cursor::Pointer)
                                .on_press(move |st: &mut State| {
                                    crate::data::import_character_asset(st, index)
                                }),
                        ),
                ),
        );
    if let Some(error) = &item.error {
        page = page.child(text(error.clone()).font(BODY).color(u.rose).wrap());
    }
    page = page
        .child(facts)
        .child(
            col()
                .w_full()
                .gap(8.0)
                .child(subpage::group_head(u, "Stages", ""))
                .child(stages),
        )
        .child(
            text("The mesh still needs a Source 2 compile and a check in game.")
                .font(SMALL)
                .color(u.faint)
                .wrap(),
        );

    col()
        .key("character-detail")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(page)
}

/// One stage on one line: its name, whether this machine can run it, and a short note.
fn stage_line(u: &Ui, stage: &StageRow) -> El {
    let (state, color) = match stage.state.as_str() {
        "succeeded" => ("Done", u.muted),
        "running" => ("Running", u.ink),
        "failed" => ("Failed", u.rose),
        "unsupported" => ("Not available", u.amber),
        "pending" => ("Waiting", u.muted),
        "skipped" => ("Skipped", u.faint),
        _ => ("Waiting", u.muted),
    };
    row()
        .key(format!("stage-{}", stage.id))
        .w_full()
        .h(44.0)
        .gap(16.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(
            text(stage.title.clone())
                .font(BODY)
                .color(u.ink)
                .w(140.0)
                .truncate(),
        )
        .child(text(state).font(BODY).color(color).w(104.0))
        .child(
            text(stage.note.clone().unwrap_or_default())
                .font(SMALL)
                .color(u.faint)
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        )
}
