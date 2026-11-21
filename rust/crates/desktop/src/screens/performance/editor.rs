//! The convar editor: search what a config writes, change a value, read the
//! history of changes, and exchange the settings as a file.
//!
//! The editor only changes Sigil's saved edits. Writing stays the existing Apply
//! step: `performance::apply_to_install` merges the edits into the config's
//! `gameinfo.gi` block, and turning the config off restores every stock value.
//! Edits are saved on a worker; file reads, writes and the export text are too.

use std::collections::BTreeMap;

use deadlock_core::performance::{
    ConvarState, Config, Options,
    classify,
    edits::{self, ConfigEdits},
    exchange::{self, Exchange},
};
use rune::{
    El, FileDialogOptions, Key, KeyEvent, TextEdit, Tx, col, el, icon, row, spacer, text,
};

use crate::app::State;
use crate::icons;
use crate::screens::settings::i18n::fill;
use crate::ui::{ActionKind, BODY, BODY_MED, DISPLAY, H1, MONO, SMALL, Ui};

use super::{
    applied_id, author_value, config, edits_for, options_for, page, pending, tr,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Settings,
    History,
}

#[derive(Clone)]
pub enum Dialog {
    /// Change one value. `field` holds what the player is typing.
    Value {
        key: String,
        field: TextEdit,
        error: Option<String>,
    },
    /// Put the config's own value back.
    Reset { key: String },
    /// Drop every change for this config.
    Clear,
    /// Paste or choose a settings file.
    Import { field: TextEdit, error: Option<String> },
    /// What the file held, before it becomes edits.
    Review { path: String, read: Exchange },
}

pub struct Editor {
    pub config: String,
    pub search: TextEdit,
    pub tab: Tab,
    pub only_changed: bool,
    pub dialog: Option<Dialog>,
    pub message: Option<(String, bool)>,
}

impl Default for Editor {
    fn default() -> Self {
        Editor {
            config: String::new(),
            search: TextEdit::new(""),
            tab: Tab::default(),
            only_changed: false,
            dialog: None,
            message: None,
        }
    }
}

/// Open the editor for a config.
pub fn open(st: &mut State, id: &str) {
    st.performance.editor = Some(Editor {
        config: id.to_string(),
        ..Editor::default()
    });
}

fn close(st: &mut State) {
    st.performance.editor = None;
}

/// Take the file and export jobs. Returns true when the screen changed.
pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.performance.export_job.try_take() {
        if let Some(editor) = st.performance.editor.as_mut() {
            editor.message = Some(match result {
                Ok(line) => (line, false),
                Err(error) => (error, true),
            });
        }
        changed = true;
    }
    if let Some(result) = st.performance.import_job.try_take() {
        if let Some(editor) = st.performance.editor.as_mut() {
            match result {
                Ok((path, read)) => {
                    editor.dialog = Some(Dialog::Review { path, read });
                }
                Err(error) => {
                    editor.message = Some((error, true));
                }
            }
        }
        changed = true;
    }
    changed
}

/// Persist the edits on a worker. The in-memory copy is already current, so a
/// save that is still running just marks the file due again.
fn persist(st: &mut State) {
    st.performance.edits_dirty = true;
    start_save(st);
}

pub(super) fn start_save(st: &mut State) {
    if st.performance.save_job.is_running() || !st.performance.edits_dirty {
        return;
    }
    st.performance.edits_dirty = false;
    let edits = st.performance.edits.clone();
    let wake = st.wake.clone();
    st.performance.save_job.start(wake, move || {
        edits::save(&crate::data::data_dir(), &edits).map_err(|error| error.to_string())
    });
}

// --- view ---------------------------------------------------------------------

pub fn view(s: &State, u: &Ui) -> El {
    let Some(editor) = s.performance.editor.as_ref() else {
        return el();
    };
    let Some(config) = config(s, &editor.config) else {
        return page("perf-editor-page")
            .child(
                row().w_full().h(32.0).center_y().child(
                    row()
                        .key("perf-editor-back")
                        .h(28.0)
                        .gap(6.0)
                        .center_y()
                        .hover(u.hover)
                        .focusable(true)
                        .role("button")
                        .label(tr("Back to Performance"))
                        .cursor(rune::Cursor::Pointer)
                        .on_press(close)
                        .child(icon(&icons::ARROW_LEFT, 14.0).color(u.muted).a11y_hidden())
                        .child(text(tr("Performance")).font(BODY).color(u.muted).a11y_hidden()),
                ),
            )
            .child(
                text(tr("This config is no longer in the list."))
                    .font(BODY)
                    .color(u.muted),
            );
    };
    let options = options_for(s, config);
    let edits = edits_for(s, config.id());
    let dirty = pending(s, config);
    let is_on = applied_id(s).as_deref() == Some(config.id());

    let mut actions = row().gap(8.0).center_y();
    actions = actions
        .child(
            u.icon_action(&icons::COPY, tr("Copy"), ActionKind::Ghost)
                .key("perf-editor-copy")
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(|st: &mut State, tx: &mut Tx| {
                    if let Some(config) = current_config(st) {
                        let options = options_for(st, &config);
                        tx.copy(exchange::export_text(&config, &options));
                        if let Some(editor) = st.performance.editor.as_mut() {
                            editor.message = Some((tr("Copied the settings.").to_string(), false));
                        }
                    }
                }),
        )
        .child(
            u.icon_action(&icons::DOWNLOAD, tr("Export"), ActionKind::Ghost)
                .key("perf-editor-export")
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(export),
        )
        .child(
            u.icon_action(&icons::UPLOAD, tr("Import"), ActionKind::Ghost)
                .key("perf-editor-import")
                .cursor(rune::Cursor::Pointer)
                .on_press(paste),
        );
    // "Reset all" and "Apply changes" keep their room when they do not apply, so the other
    // actions never move.
    let reset_all = u
        .icon_action(&icons::ROTATE_CCW, tr("Reset all"), ActionKind::Ghost)
        .key("perf-editor-reset-all");
    actions = actions.child(if edits.is_empty() {
        reset_all.opacity(0.0).focusable(false).a11y_hidden()
    } else {
        reset_all.cursor(rune::Cursor::Pointer).on_press(|st: &mut State| {
            if let Some(editor) = st.performance.editor.as_mut() {
                editor.dialog = Some(Dialog::Clear);
            }
        })
    });
    let id = config.id().to_string();
    let apply = u
        .action(tr("Apply changes"), ActionKind::Primary)
        .key("perf-editor-apply")
        .w(132.0);
    actions = actions.child(if dirty {
        apply.cursor(rune::Cursor::Pointer).on_press(move |st: &mut State| {
            if !st.performance.write_job.is_running() {
                st.performance.dialog = Some(super::Dialog::Apply(id.clone()));
            }
        })
    } else {
        apply.opacity(0.0).focusable(false).a11y_hidden()
    });

    let head = col()
        .w_full()
        .gap(4.0)
        .child(
            row()
                .w_full()
                .h(32.0)
                .center_y()
                .child(
                    row()
                        .key("perf-editor-back")
                        .h(28.0)
                        .pl(6.0)
                        .pr(8.0)
                        .ml(-6.0)
                        .gap(6.0)
                        .center_y()
                        .radius(crate::ui::r(6.0))
                        .hover(u.hover)
                        .focusable(true)
                        .role("button")
                        .label(tr("Back to the config"))
                        .cursor(rune::Cursor::Pointer)
                        .on_press(close)
                        .child(icon(&icons::ARROW_LEFT, 14.0).color(u.muted).a11y_hidden())
                        .child(text(tr("Performance")).font(BODY).color(u.muted).a11y_hidden()),
                )
                .child(spacer()),
        )
        .child(
            row()
                .w_full()
                .gap(16.0)
                .center_y()
                .child(
                    text(fill(tr("Settings for {}"), &[config.name()]))
                        .font(DISPLAY)
                        .color(u.ink)
                        .truncate()
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(actions),
        )
        .child(
            text({
                let mut line = fill(
                    tr("{} settings · {} changed"),
                    &[
                        &exchange::count(config, &options).to_string(),
                        &edits.values.len().to_string(),
                    ],
                );
                if is_on {
                    line.push_str(" · ");
                    line.push_str(tr("on"));
                }
                if dirty {
                    line.push_str(" · ");
                    line.push_str(tr("apply to write"));
                }
                line
            })
            .font(SMALL)
            .color(u.muted),
        );

    // The message slot is always there, so a note never pushes the list.
    let message = match editor.message.as_ref().or(s.performance.message.as_ref()) {
        Some((line, error)) => row().w_full().h(20.0).center_y().child(
            text(line.clone())
                .font(SMALL)
                .color(if *error { u.rose } else { u.muted })
                .truncate(),
        ),
        // With nothing to report, the line says what the page is for.
        None => row().w_full().h(20.0).center_y().child(
            text(tr("These are the values the config writes. Change one and apply; turning the config off puts the game's own values back."))
                .font(SMALL)
                .color(u.muted)
                .truncate(),
        ),
    };
    let mut out = page("perf-editor-page").gap(16.0).child(head.child(message));

    let field = rune::text_input(&editor.search, BODY)
        .key("perf-editor-search")
        .single_line()
        .label(tr("Search settings"))
        .grow(1.0)
        .min_w(0.0)
        .color(u.ink)
        .placeholder(tr("Search by name, like shadows or fps"), u.faint)
        .caret(u.accent, u.accent.alpha(0.20))
        .on_edit(rune::edit_access(|st: &mut State| {
            &mut st.performance.editor.as_mut().expect("editor").search
        }));
    let tabs = row()
        .gap(4.0)
        .center_y()
        .child(tab(u, editor, Tab::Settings, tr("Settings")))
        .child(tab(u, editor, Tab::History, tr("History")));
    out = out.child(
        row()
            .w_full()
            .h(32.0)
            .gap(16.0)
            .center_y()
            .child(u.search_shell(field))
            .child(tabs)
            .child(spacer())
            .child(
                u.toggle(editor.only_changed)
                    .key("perf-editor-only-changed")
                    .label(tr("Show only changed settings"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| {
                        if let Some(editor) = st.performance.editor.as_mut() {
                            editor.only_changed = !editor.only_changed;
                        }
                    }),
            )
            .child(text(tr("Changed only")).font(SMALL).color(u.muted)),
    );

    match editor.tab {
        Tab::Settings => out = out.child(settings_list(s, u, config, &options, edits)),
        Tab::History => out = out.child(history(s, u, config, edits)),
    }
    out
}

fn current_config(st: &State) -> Option<Config> {
    let id = &st.performance.editor.as_ref()?.config;
    config(st, id).cloned()
}

fn tab(u: &Ui, editor: &Editor, tab: Tab, label: &str) -> El {
    let on = editor.tab == tab;
    row()
        .key(format!("perf-editor-tab-{label}"))
        .h(28.0)
        .px(10.0)
        .center()
        .radius(crate::ui::r(6.0))
        .fill(if on { u.selected } else { rune::Color::TRANSPARENT })
        .hover(u.hover)
        .focusable(true)
        .role("tab")
        .checked(on)
        .label(label.to_string())
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            if let Some(editor) = st.performance.editor.as_mut() {
                editor.tab = tab;
            }
        })
        .child(
            text(label.to_string())
                .font(BODY_MED)
                .color(if on { u.ink } else { u.muted })
                .a11y_hidden(),
        )
}

/// One row of the settings table.
struct Entry {
    key: String,
    author: Option<String>,
    /// The value that will be written.
    value: String,
    changed: bool,
    unknown: bool,
}

fn entries(config: &Config, options: &Options, edits: &ConfigEdits) -> Vec<Entry> {
    let mut map: BTreeMap<String, (Option<String>, bool)> = BTreeMap::new();
    for (key, value) in config.body_convars() {
        map.insert(key.clone(), (Some(value.clone()), false));
    }
    for opt in config.switchable() {
        if options.opt_ins.contains(&opt.key) {
            map.insert(opt.key.clone(), (Some(opt.value.clone()), false));
        }
    }
    for (key, value) in &options.edits {
        if config.convar_state(key) == ConvarState::Ignored {
            continue;
        }
        let entry = map.entry(key.clone()).or_insert((None, true));
        entry.1 = entry.0.is_none();
        let _ = value;
    }
    map.into_iter()
        .map(|(key, (author, _))| {
            let value = edits
                .get(&key)
                .map(str::to_string)
                .or_else(|| author.clone())
                .unwrap_or_default();
            Entry {
                changed: edits.get(&key).is_some(),
                unknown: config.convar_state(&key) == ConvarState::Unknown,
                key,
                author,
                value,
            }
        })
        .collect()
}

fn settings_list(s: &State, u: &Ui, config: &Config, options: &Options, edits: &ConfigEdits) -> El {
    let editor = s.performance.editor.as_ref().expect("editor");
    let query = editor.search.text.trim().to_ascii_lowercase();
    let mut rows: Vec<Entry> = entries(config, options, edits)
        .into_iter()
        .filter(|entry| {
            if editor.only_changed && !entry.changed {
                return false;
            }
            if query.is_empty() {
                return true;
            }
            entry.key.to_ascii_lowercase().contains(&query)
                || classify::opt_in_label(&entry.key)
                    .is_some_and(|label| label.to_ascii_lowercase().contains(&query))
        })
        .collect();
    rows.sort_by(|a, b| {
        b.changed
            .cmp(&a.changed)
            .then_with(|| a.key.cmp(&b.key))
    });
    let total = rows.len();
    let shown = total.min(60);
    let mut list = col().w_full().border_sides([1.0, 0.0, 0.0, 0.0], u.hair);
    if rows.is_empty() {
        list = list.child(
            row()
                .w_full()
                .h(52.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(
                    text(if editor.only_changed {
                        tr("No changed settings yet.")
                    } else if query.is_empty() {
                        tr("This config has no editable settings.")
                    } else {
                        tr("No settings match that search.")
                    })
                    .font(BODY)
                    .color(u.muted),
                ),
        );
    }
    for entry in rows.into_iter().take(shown) {
        list = list.child(entry_row(u, config, entry));
    }
    if total > shown {
        list = list.child(
            row()
                .w_full()
                .h(44.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(
                    text(fill(
                        tr("{} more. Type in the search to narrow them."),
                        &[&(total - shown).to_string()],
                    ))
                    .font(SMALL)
                    .color(u.muted),
                ),
        );
    }
    list
}

fn entry_row(u: &Ui, config: &Config, entry: Entry) -> El {
    let key = entry.key.clone();
    let reset_key = entry.key.clone();
    let id = config.id().to_string();
    let press_id = id.clone();
    let label = classify::opt_in_label(&entry.key)
        .map(str::to_string)
        .unwrap_or_else(|| {
            if entry.unknown {
                tr("Not in the game's setting list").to_string()
            } else {
                String::new()
            }
        });
    // Fixed cells: value 140, "was" 104 and two button slots of 28, reserved when empty, so
    // every row's columns line up whatever it holds.
    let value = text(if entry.value.is_empty() {
        "-".to_string()
    } else {
        entry.value.clone()
    })
    .font(crate::ui::MONO_MED)
    .color(u.ink)
    .w(140.0)
    .align(rune::Align::End)
    .truncate();
    let was = match &entry.author {
        Some(author) if entry.changed && author != &entry.value => fill(tr("was {}"), &[author]),
        _ => String::new(),
    };
    let was = text(was).font(SMALL).color(u.muted).w(104.0).truncate();
    let reset: El = if entry.changed {
        small_button(u, &icons::ROTATE_CCW, tr("Use the config's value"))
            .key(format!("perf-editor-reset-{key}"))
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| {
                if let Some(editor) = st.performance.editor.as_mut() {
                    editor.dialog = Some(Dialog::Reset {
                        key: reset_key.clone(),
                    });
                }
            })
    } else {
        el().size(28.0, 28.0)
    };
    let open_key = entry.key.clone();
    let change = small_button(u, &icons::SQUARE_PEN, tr("Change this setting"))
        .key(format!("perf-editor-edit-{key}"))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            open_value(st, &id, &open_key);
        });
    row()
        .key(format!("perf-editor-row-{key}"))
        .w_full()
        .h(44.0)
        .gap(12.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .hover(u.hover)
        .focusable(true)
        .role("button")
        .label(format!("{}, {}", entry.key, entry.value))
        .cursor(rune::Cursor::Pointer)
        .on_press({
            let key = entry.key.clone();
            move |st: &mut State| open_value(st, &press_id, &key)
        })
        // An edited row is marked by an amber edge, not a tint.
        // In the gutter, so names keep the page's left edge.
        .child(
            el().absolute(-8.0, 0.0)
                .w(3.0)
                .h(43.0)
                .fill(if entry.changed { u.amber } else { rune::Color::TRANSPARENT })
                .a11y_hidden(),
        )
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(0.0)
                .child(text(entry.key).font(MONO).color(u.ink).truncate())
                .child(text(label).font(SMALL).color(u.muted).truncate()),
        )
        .child(value)
        .child(was)
        .child(row().gap(0.0).center_y().child(reset).child(change))
}

fn small_button(u: &Ui, glyph: &'static rune::Icon, label: &str) -> El {
    el().size(28.0, 28.0)
        .center()
        .radius(crate::ui::r(6.0))
        .hover(u.hover)
        .focusable(true)
        .role("button")
        .label(label)
        .cursor(rune::Cursor::Pointer)
        .child(icon(glyph, 14.0).color(u.muted).a11y_hidden())
}

fn open_value(st: &mut State, id: &str, key: &str) {
    let Some(config) = config(st, id).cloned() else {
        return;
    };
    let options = options_for(st, &config);
    let value = options
        .edits
        .get(key)
        .cloned()
        .or_else(|| author_value(&config, &options, key))
        .unwrap_or_default();
    if let Some(editor) = st.performance.editor.as_mut() {
        editor.dialog = Some(Dialog::Value {
            key: key.to_string(),
            field: TextEdit::new(value),
            error: None,
        });
    }
}

fn history(s: &State, u: &Ui, config: &Config, edits: &ConfigEdits) -> El {
    let options = options_for(s, config);
    let mut list = col().w_full().border_sides([1.0, 0.0, 0.0, 0.0], u.hair);
    if edits.history.is_empty() {
        return list.child(
            row()
                .w_full()
                .h(52.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(
                    text(tr("No changes yet. Every change you make is listed here."))
                        .font(BODY)
                        .color(u.muted),
                ),
        );
    }
    for entry in edits.history.iter().rev().take(80) {
        let key = entry.key.clone();
        let changed = edits.get(&entry.key).is_some();
        let author = author_value(config, &options, &entry.key);
        let mut actions = row().gap(2.0).center_y();
        if changed {
            actions = actions.child(
                small_button(u, &icons::ROTATE_CCW, tr("Use the config's value"))
                    .key(format!("perf-history-reset-{}", entry.key))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        if let Some(editor) = st.performance.editor.as_mut() {
                            editor.dialog = Some(Dialog::Reset { key: key.clone() });
                        }
                    }),
            );
        }
        let when = entry
            .at
            .get(0..16)
            .unwrap_or(&entry.at)
            .replace('T', " ");
        let from = entry.from.clone().unwrap_or_else(|| tr("unset").to_string());
        list = list.child(
            row()
                .key(format!("perf-history-{}", entry.key))
                .w_full()
                .h(52.0)
                .gap(16.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(text(entry.key.clone()).font(MONO).color(u.ink).w(380.0).truncate())
                .child(
                    text(format!("{from}  to  {}", entry.to))
                        .font(SMALL)
                        .color(u.muted)
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate(),
                )
                .child(
                    text(author
                        .map(|value| fill(tr("config: {}"), &[&value]))
                        .unwrap_or_default())
                    .font(SMALL)
                    .color(u.muted)
                    .w(140.0)
                    .truncate(),
                )
                .child(text(when).font(SMALL).color(u.muted).w(120.0))
                .child(actions),
        );
    }
    list
}

// --- actions ------------------------------------------------------------------

fn save_value(st: &mut State) {
    let Some(editor) = st.performance.editor.as_ref() else {
        return;
    };
    let id = editor.config.clone();
    let Some(Dialog::Value { key, field, .. }) = editor.dialog.as_ref() else {
        return;
    };
    let key = key.clone();
    let value = field.text.trim().to_string();
    if value.is_empty() || value.contains('\n') || value.contains('\r') || value.contains('"') {
        if let Some(Dialog::Value { error, .. }) = st
            .performance
            .editor
            .as_mut()
            .and_then(|editor| editor.dialog.as_mut())
        {
            *error = Some(tr("Use one value, with no quotes or line breaks.").into());
        }
        return;
    }
    let Some(config) = config(st, &id).cloned() else {
        return;
    };
    let options = options_for(st, &config);
    let author = author_value(&config, &options, &key);
    let from = options.edits.get(&key).cloned().or_else(|| author.clone());
    if author.as_deref() == Some(value.as_str()) && from.is_none() {
        if let Some(editor) = st.performance.editor.as_mut() {
            editor.dialog = None;
        }
        return;
    }
    let _ = st.performance.edits.set(&id, &key, from.as_deref(), &value);
    persist(st);
    if let Some(editor) = st.performance.editor.as_mut() {
        editor.dialog = None;
        editor.message = Some((tr("Saved. Apply the config to write it.").into(), false));
    }
}

fn reset_value(st: &mut State) {
    let Some(editor) = st.performance.editor.as_ref() else {
        return;
    };
    let id = editor.config.clone();
    let Some(Dialog::Reset { key }) = editor.dialog.clone() else {
        return;
    };
    let Some(config) = config(st, &id).cloned() else {
        return;
    };
    let options = options_for(st, &config);
    let author = author_value(&config, &options, &key).unwrap_or_default();
    let from = options.edits.get(&key).cloned();
    st.performance
        .edits
        .reset(&id, &key, from.as_deref(), &author);
    persist(st);
    if let Some(editor) = st.performance.editor.as_mut() {
        editor.dialog = None;
        editor.message = Some((
            tr("Back to the config's value. Apply to write it.").into(),
            false,
        ));
    }
}

fn clear_all(st: &mut State) {
    let Some(id) = st.performance.editor.as_ref().map(|e| e.config.clone()) else {
        return;
    };
    st.performance.edits.clear(&id);
    persist(st);
    if let Some(editor) = st.performance.editor.as_mut() {
        editor.dialog = None;
        editor.message = Some((tr("Every change is gone. The config's values apply again.").into(), false));
    }
}

fn paste(st: &mut State) {
    if let Some(editor) = st.performance.editor.as_mut() {
        editor.dialog = Some(Dialog::Import {
            field: TextEdit::new(""),
            error: None,
        });
    }
}

fn choose_file(_st: &mut State, tx: &mut Tx) {
    let options = FileDialogOptions {
        title: tr("Choose a settings file").into(),
        ..Default::default()
    }
    .filter(tr("Settings"), &["cfg", "txt", "gi"]);
    tx.open_file(options, |st: &mut State, result, _tx| match result {
        Ok(Some(path)) => {
            let wake = st.wake.clone();
            st.performance.import_job.start(wake, move || {
                let text = std::fs::read_to_string(&path)
                    .map_err(|error| {
                        fill(tr("The file could not be read: {}"), &[&error.to_string()])
                    })?;
                Ok((path.display().to_string(), exchange::parse(&text)))
            });
        }
        Ok(None) => {}
        Err(error) => {
            if let Some(editor) = st.performance.editor.as_mut() {
                editor.message = Some((
                    fill(
                        tr("The file picker could not open: {}"),
                        &[&error.to_string()],
                    ),
                    true,
                ));
            }
        }
    });
}

fn review_paste(st: &mut State) {
    let Some(Dialog::Import { field, error }) = st
        .performance
        .editor
        .as_mut()
        .and_then(|editor| editor.dialog.as_mut())
    else {
        return;
    };
    let read = exchange::parse(&field.text);
    if read.is_empty() {
        *error = Some(
            tr("No settings were found. Each line should hold a name and a value, like r_shadows 0.")
                .into(),
        );
        return;
    }
    if let Some(editor) = st.performance.editor.as_mut() {
        editor.dialog = Some(Dialog::Review {
            path: "pasted text".to_string(),
            read,
        });
    }
}

fn import_all(st: &mut State) {
    let Some(editor) = st.performance.editor.as_ref() else {
        return;
    };
    let id = editor.config.clone();
    let Some(Dialog::Review { path, read }) = editor.dialog.clone() else {
        return;
    };
    let Some(config) = config(st, &id).cloned() else {
        return;
    };
    let options = options_for(st, &config);
    let mut count = 0usize;
    let mut refused = 0usize;
    for (key, value) in &read.entries {
        let from = options
            .edits
            .get(key)
            .cloned()
            .or_else(|| author_value(&config, &options, key));
        if st.performance.edits.set(&id, key, from.as_deref(), value) {
            count += 1;
        } else {
            refused += 1;
        }
    }
    persist(st);
    if let Some(editor) = st.performance.editor.as_mut() {
        editor.dialog = None;
        let mut line = fill(
            tr("Added {} settings from {}. Apply to write them."),
            &[&count.to_string(), &path],
        );
        if refused > 0 {
            line.push(' ');
            line.push_str(&fill(
                tr("Skipped {} with characters a setting can't hold."),
                &[&refused.to_string()],
            ));
        }
        editor.message = Some((line, refused > 0));
    }
}

fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "performance-settings".to_string()
    } else {
        out
    }
}

fn export(st: &mut State, tx: &mut Tx) {
    let Some(config) = current_config(st) else {
        return;
    };
    let text = exchange::export_text(&config, &options_for(st, &config));
    let options = FileDialogOptions {
        title: tr("Export settings").into(),
        file_name: Some(format!("{}.cfg", slug(config.name()))),
        ..Default::default()
    }
    .filter(tr("Settings"), &["cfg", "txt"]);
    tx.save_file(options, move |st: &mut State, result, _tx| {
        let Ok(Some(path)) = result else {
            return;
        };
        let text = text.clone();
        let wake = st.wake.clone();
        st.performance.export_job.start(wake, move || {
            std::fs::write(&path, text)
                .map(|_| fill(tr("Saved {}."), &[&path.display().to_string()]))
                .map_err(|error| fill(tr("The file could not be saved: {}"), &[&error.to_string()]))
        });
    });
}

// --- dialogs ------------------------------------------------------------------

pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    let editor = s.performance.editor.as_ref()?;
    let dialog = editor.dialog.as_ref()?;
    let config = config(s, &editor.config);
    let (title, body, buttons): (String, Vec<String>, Vec<El>) = match dialog {
        Dialog::Value { key, error, .. } => {
            let author = config.and_then(|config| {
                let options = options_for(s, config);
                author_value(config, &options, key)
            });
            (
                fill(tr("Change {key}"), &[key]),
                vec![
                    author
                        .map(|value| {
                            fill(
                                tr("The config writes {}. Your value is written instead."),
                                &[&value],
                            )
                        })
                        .unwrap_or_else(|| tr("The config does not set this. Sigil adds it to the settings it writes.").into()),
                    error.clone().unwrap_or_default(),
                ],
                vec![
                    u.action(tr("Cancel"), ActionKind::Ghost)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| {
                            if let Some(editor) = st.performance.editor.as_mut() {
                                editor.dialog = None;
                            }
                        }),
                    u.action(tr("Save"), ActionKind::Primary)
                        .key("perf-editor-save")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(save_value),
                ],
            )
        }
        Dialog::Reset { key } => {
            let author = config
                .and_then(|config| {
                    let options = options_for(s, config);
                    author_value(config, &options, key)
                })
                .unwrap_or_default();
            (
                tr("Use the config's value?").to_string(),
                vec![fill(
                    tr("{} goes back to {} the next time you apply."),
                    &[key, &author],
                )],
                vec![
                    u.action(tr("Cancel"), ActionKind::Ghost)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| {
                            if let Some(editor) = st.performance.editor.as_mut() {
                                editor.dialog = None;
                            }
                        }),
                    u.action(tr("Use it"), ActionKind::Primary)
                        .key("perf-editor-reset-confirm")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(reset_value),
                ],
            )
        }
        Dialog::Clear => {
            let count = config
                .map(|config| edits_for(s, config.id()).values.len())
                .unwrap_or(0);
            let name = config.map(Config::name).unwrap_or("this config");
            (
                tr("Reset all your changes?").to_string(),
                vec![fill(
                    tr("Drops {} change{} for {}. The config's own values apply the next time you apply."),
                    &[&count.to_string(), if count == 1 { "" } else { "s" }, name],
                )],
                vec![
                    u.action(tr("Cancel"), ActionKind::Ghost)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| {
                            if let Some(editor) = st.performance.editor.as_mut() {
                                editor.dialog = None;
                            }
                        }),
                    u.action(tr("Reset all"), ActionKind::Danger)
                        .key("perf-editor-clear-confirm")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(clear_all),
                ],
            )
        }
        Dialog::Import { error, .. } => (
            tr("Import settings").to_string(),
            vec![
                tr("Paste ConVars or a ConVars block, or choose a file. Sigil reads the names and values it knows.").to_string(),
                error.clone().unwrap_or_default(),
            ],
            vec![
                u.action(tr("Cancel"), ActionKind::Ghost)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| {
                        if let Some(editor) = st.performance.editor.as_mut() {
                            editor.dialog = None;
                        }
                    }),
                u.action(tr("Choose a file"), ActionKind::Secondary)
                    .key("perf-editor-import-file")
                    .cursor(rune::Cursor::Pointer)
                    .on_press_tx(choose_file),
                u.action(tr("Review"), ActionKind::Primary)
                    .key("perf-editor-import-review")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(review_paste),
            ],
        ),
        Dialog::Review { path, read } => {
            let config = config?;
            let known = read
                .entries
                .iter()
                .filter(|(key, _)| {
                    config.body.convars.iter().any(|(k, _)| k == key)
                        || config.switchable().any(|opt| &opt.key == key)
                })
                .count();
            let extra = read.entries.len() - known;
            let mut lines = vec![fill(
                tr("{} settings from {}. {} are already in this config."),
                &[&read.entries.len().to_string(), path, &known.to_string()],
            )];
            if extra > 0 {
                lines.push(fill(
                    tr("{} are new. Sigil writes them too."),
                    &[&extra.to_string()],
                ));
            }
            if read.skipped > 0 {
                lines.push(fill(
                    tr("{} line{} could not be read and are left out."),
                    &[
                        &read.skipped.to_string(),
                        if read.skipped == 1 { "" } else { "s" },
                    ],
                ));
            }
            let listed: Vec<String> = read
                .entries
                .iter()
                .take(8)
                .map(|(key, value)| format!("{key} = {value}"))
                .collect();
            lines.push(listed.join("\n"));
            (
                tr("Add these settings?").to_string(),
                lines,
                vec![
                    u.action(tr("Cancel"), ActionKind::Ghost)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| {
                            if let Some(editor) = st.performance.editor.as_mut() {
                                editor.dialog = None;
                            }
                        }),
                    u.action(
                        &fill(tr("Add {}"), &[&read.entries.len().to_string()]),
                        ActionKind::Primary,
                    )
                    .key("perf-editor-import-confirm")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(import_all),
                ],
            )
        }
    };

    let mut body_col = col()
        .w_full()
        .gap(8.0)
        .child(text(title.clone()).font(H1).color(u.ink));
    for line in body.into_iter().filter(|line| !line.is_empty()) {
        body_col = body_col.child(text(line).font(BODY).color(u.muted).wrap_anywhere());
    }
    if let Dialog::Value { field, error, .. } = dialog {
        let input = rune::text_input(field, MONO)
            .key("perf-editor-value")
            .single_line()
            .grow(1.0)
            .min_w(0.0)
            .color(u.ink)
            .caret(u.accent, u.accent.alpha(0.20))
            .on_edit(rune::edit_access(|st: &mut State| {
                match st
                    .performance
                    .editor
                    .as_mut()
                    .and_then(|editor| editor.dialog.as_mut())
                {
                    Some(Dialog::Value { field, .. }) => field,
                    _ => &mut st.performance.scratch,
                }
            }))
            .on_submit(|st: &mut State, _| save_value(st));
        let mut row = row()
            .w_full()
            .h(32.0)
            .px(10.0)
            .center_y()
            .radius(crate::ui::r(8.0))
            .fill(u.well)
            .border(1.0, u.hair)
            .child(input);
        if error.is_some() {
            row = row.border(1.0, u.rose);
        }
        body_col = body_col.child(row);
    }
    if let Dialog::Import { field, .. } = dialog {
        let input = rune::text_input(field, MONO)
            .key("perf-editor-import-input")
            .grow(1.0)
            .color(u.ink)
            .placeholder("r_shadows 0", u.faint)
            .caret(u.accent, u.accent.alpha(0.20))
            .on_edit(rune::edit_access(|st: &mut State| {
                match st
                    .performance
                    .editor
                    .as_mut()
                    .and_then(|editor| editor.dialog.as_mut())
                {
                    Some(Dialog::Import { field, .. }) => field,
                    _ => &mut st.performance.scratch,
                }
            }));
        body_col = body_col.child(
            col()
                .w_full()
                .h(140.0)
                .px(10.0)
                .py(8.0)
                .radius(crate::ui::r(8.0))
                .fill(u.well)
                .border(1.0, u.hair)
                .child(input),
        );
    }

    let close = close_any;
    let mut buttons_row = row().w_full().gap(8.0).child(spacer());
    for button in buttons {
        buttons_row = buttons_row.child(button);
    }
    Some(
        u.dialog_shell()
            .key("perf-editor-dialog")
            .role("dialog")
            .label(title)
            .w(520.0)
            .padding(24.0)
            .gap(16.0)
            .modal(true, move |st: &mut State| close(st))
            .on_key(|st: &mut State, ev: &KeyEvent, _| {
                if ev.key == Key::Escape {
                    close_any(st);
                    true
                } else {
                    false
                }
            })
            .child(body_col)
            .child(buttons_row),
    )
}

fn close_any(st: &mut State) {
    if let Some(editor) = st.performance.editor.as_mut() {
        editor.dialog = None;
    }
}
