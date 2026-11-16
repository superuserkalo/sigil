//! Autoexec: the game config file, read and edited behind approval.
//!
//! The path comes from `deadlock_core::autoexec::autoexec_path`, derived from the
//! discovered game root. Reading is free. A save copies the old file beside
//! itself and refuses a NUL byte, and the screen still asks before it calls
//! `write_autoexec` with the approval flag.

use rune::{Color, El, Key, KeyEvent, TextEdit, col, el, row, text};

use crate::app::State;
use crate::model::Screen;
use crate::ui::{ActionKind, BODY_MED, CHAT, H2, LABEL, MONO, SMALL, Tone, Ui};

pub struct Read {
    pub path: String,
    pub exists: bool,
    pub text: String,
}

pub struct View {
    pub loaded: bool,
    pub error: Option<String>,
    pub path: String,
    pub exists: bool,
    pub editor: TextEdit,
    pub confirm_save: bool,
    pub busy: bool,
    pub last: Option<String>,
}

impl Default for View {
    fn default() -> Self {
        View {
            loaded: false,
            error: None,
            path: String::new(),
            exists: false,
            editor: TextEdit::new(""),
            confirm_save: false,
            busy: false,
            last: None,
        }
    }
}

impl View {
    /// The text as it stands, with the same trailing-newline rule the writer
    /// applies, so the dirty check matches what would land on disk.
    pub fn dirty(&self, original: &str) -> bool {
        deadlock_core::autoexec::normalize_trailing_newline(&self.editor.text)
            != deadlock_core::autoexec::normalize_trailing_newline(original)
    }

    pub fn line_count(&self) -> usize {
        deadlock_core::autoexec::list_lines(&self.editor.text).len()
    }
}

pub fn view(s: &State, u: &Ui) -> El {
    let v = &s.autoexec;
    let original = s.autoexec_original.as_str();
    let dirty = v.loaded && v.dirty(original);
    let trailing = row()
        .gap(8.0)
        .center_y()
        .child(if s.autoexec_job.is_running() || v.busy {
            u.status_chip("working", Tone::Warn)
        } else if v.error.is_some() {
            u.status_chip("read failed", Tone::Bad)
        } else if dirty {
            u.status_chip("unsaved changes", Tone::Warn)
        } else if v.exists {
            u.status_chip("on disk", Tone::Ok)
        } else if v.loaded {
            u.status_chip("no file", Tone::Quiet)
        } else {
            u.status_chip("not read", Tone::Quiet)
        })
        .child(
            u.action("Reload", ActionKind::Secondary)
                .key("autoexec-reload")
                .on_press(|st: &mut State| crate::data::start_autoexec_read(st)),
        )
        .child(
            u.action("Save", ActionKind::Primary)
                .key("autoexec-save")
                .disabled(!dirty)
                .opacity(if dirty { 1.0 } else { 0.45 })
                .on_press(|st: &mut State| {
                    if st.autoexec.loaded {
                        st.autoexec.confirm_save = true;
                    }
                }),
        );

    let detail = if let Some(error) = &v.error {
        error.clone()
    } else if !v.loaded {
        "Reading game/citadel/cfg/autoexec.cfg.".to_string()
    } else if !v.exists {
        "The file does not exist yet. A save creates it and copies nothing, because there is nothing to back up."
            .to_string()
    } else {
        format!(
            "{} lines · {} bytes · the previous file is copied to autoexec.cfg.dah-backup-<timestamp> on save",
            v.line_count(),
            v.editor.text.len()
        )
    };

    let mut page = col()
        .key("autoexec-page")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .px(24.0)
        .pt(18.0)
        .pb(16.0)
        .gap(13.0)
        .child(u.glow(
            "autoexec-glow",
            [0.9, 0.0],
            (620.0, 380.0),
            u.glow_warm,
        ))
        .child(u.page_head(
            Screen::Autoexec.kicker(),
            "Autoexec",
            &detail,
            Some(trailing),
        ))
        .child(path_strip(s, u));

    if let Some(last) = &v.last {
        page = page.child(result_strip(u, last));
    }

    let body = if !s.game_detected {
        u.empty_state(
            "≡",
            "No Deadlock install",
            "The autoexec path is derived from the game root. Settings lists where the manager looked for Deadlock.",
            Some(
                u.action("Open Settings", ActionKind::Secondary)
                    .on_press(|st: &mut State| st.go(Screen::Settings)),
            ),
        )
    } else if !v.loaded {
        u.empty_state(
            "≡",
            "Reading the config",
            "The file is read on a worker. This view keeps painting.",
            None,
        )
    } else {
        editor(s, u, dirty)
    };

    page.child(body)
}

/// Mounted at the root so the save approval overlays the shell.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    if s.autoexec.confirm_save {
        Some(save_layer(s, u))
    } else {
        None
    }
}

fn path_strip(s: &State, u: &Ui) -> El {
    let path = if s.autoexec.path.is_empty() {
        format!("{}/game/citadel/cfg/autoexec.cfg", s.paths.game)
    } else {
        s.autoexec.path.clone()
    };
    let mut meta = row().gap(6.0).center_y();
    if s.autoexec.loaded {
        meta = meta
            .child(u.chip(
                if s.autoexec.exists {
                    "present"
                } else {
                    "absent"
                },
                if s.autoexec.exists { u.teal } else { u.faint },
                Color::WHITE.alpha(0.04),
            ))
            .child(u.chip(
                &format!("{} lines", s.autoexec.line_count()),
                u.violet,
                u.violet.alpha(0.10),
            ));
    }
    row()
        .key("autoexec-path")
        .w_full()
        .padding(11.0)
        .gap(12.0)
        .center_y()
        .radius(10.0)
        .fill(Color::WHITE.alpha(0.022))
        .border(1.0, u.hair)
        .child(text("FILE").font(LABEL).color(u.faint))
        .child(
            text(path)
                .font(MONO)
                .color(u.muted)
                .grow(1.0)
                .min_w(0.0)
                .truncate()
                .selectable(true),
        )
        .child(meta)
}

fn result_strip(u: &Ui, message: &str) -> El {
    let bad = message.contains("not ")
        || message.contains("could not")
        || message.contains("failed")
        || message.contains("Nothing")
        || message.contains("refused");
    row()
        .key("autoexec-result")
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

fn editor(s: &State, u: &Ui, dirty: bool) -> El {
    let gutter: Vec<String> = (1..=s.autoexec.line_count().max(1))
        .map(|n| format!("{n:>3}"))
        .collect();
    let hint = if s.autoexec.exists {
        "Deadlock reads this at launch. A save keeps one backup per write."
    } else {
        "This file does not exist. The first save creates game/citadel/cfg/autoexec.cfg."
    };

    row()
        .key("autoexec-body")
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .gap(12.0)
        .items_start()
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .h_full()
                .min_h(0.0)
                .gap(0.0)
                .radius(12.0)
                .fill(u.field)
                .border(1.0, u.hair)
                .child(
                    row()
                        .w_full()
                        .px(14.0)
                        .py(9.0)
                        .gap(8.0)
                        .center_y()
                        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                        .child(text("CONFIG").font(LABEL).color(u.faint))
                        .child(text("·").font(LABEL).color(u.faint))
                        .child(
                            text(if dirty {
                                "edited, not saved"
                            } else {
                                "matches disk"
                            })
                            .font(LABEL)
                            .color(if dirty { u.amber } else { u.faint }),
                        )
                        .child(el().grow(1.0))
                        .child(text("UTF-8 · LF").font(LABEL).color(u.faint)),
                )
                .child(
                    row()
                        .w_full()
                        .grow(1.0)
                        .min_h(0.0)
                        .px(12.0)
                        .py(10.0)
                        .gap(12.0)
                        .items_start()
                        .child(
                            text(gutter.join("\n"))
                                .font(MONO)
                                .color(u.faint.alpha(0.65))
                                .a11y_hidden(),
                        )
                        .child(
                            rune::text_input(&s.autoexec.editor, CHAT)
                                .key("autoexec-editor")
                                .label("autoexec.cfg contents")
                                .grow(1.0)
                                .min_w(0.0)
                                .min_h(0.0)
                                .color(u.ink)
                                .placeholder(
                                    "bind \"key\" \"command\"\nrate 128000",
                                    u.faint,
                                )
                                .caret(u.accent, u.accent.alpha(0.20))
                                .on_edit(|st: &mut State| &mut st.autoexec.editor),
                        ),
                ),
        )
        .child(
            col()
                .w(300.0)
                .gap(10.0)
                .radius(12.0)
                .padding(14.0)
                .fill(Color::WHITE.alpha(0.022))
                .border(1.0, u.hair)
                .child(text("Rules").font(BODY_MED).color(u.ink))
                .child(
                    text("Reading is free. Writing needs the approval step below, and the app never sends this file anywhere.")
                        .font(SMALL)
                        .color(u.muted)
                        .wrap(),
                )
                .child(u.divider())
                .child(u.kv("Backup", "autoexec.cfg.dah-backup-<time>", u.muted))
                .child(u.kv("Refuses", "a NUL byte, which is not configuration", u.faint))
                .child(u.kv("Hints", hint, u.faint))
                .child(u.divider())
                .child(
                    row()
                        .gap(8.0)
                        .center_y()
                        .child(u.dot(if dirty { Tone::Warn } else { Tone::Ok }))
                        .child(
                            text(if dirty {
                                "The editor differs from the file on disk."
                            } else {
                                "The editor matches the file on disk."
                            })
                            .font(SMALL)
                            .color(u.muted)
                            .wrap(),
                        ),
                )
                .child(
                    u.action("Reveal file", ActionKind::Ghost)
                        .key("autoexec-reveal")
                        .on_press(|st: &mut State| {
                            crate::data::reveal_path(st, st.autoexec.path.clone());
                        }),
                ),
        )
}

fn save_layer(s: &State, u: &Ui) -> El {
    let body = col()
        .w_full()
        .gap(15.0)
        .child(
            row()
                .w_full()
                .gap(12.0)
                .items_start()
                .child(
                    col()
                        .grow(1.0)
                        .gap(6.0)
                        .child(u.eyebrow("APP ACTION · GAME FILE"))
                        .child(text("Save autoexec.cfg").font(H2).color(u.ink))
                        .child(
                            text("The current file is copied beside itself first. The new text is written whole, normalized to one trailing newline.")
                                .font(SMALL)
                                .color(u.muted)
                                .wrap(),
                        ),
                )
                .child(u.glyph_box("≡", Tone::Warn, 36.0)),
        )
        .child(u.divider())
        .child(u.kv("Path", &s.autoexec.path, u.muted))
        .child(u.kv(
            "Lines",
            &s.autoexec.line_count().to_string(),
            u.ink,
        ))
        .child(u.kv(
            "Backup",
            if s.autoexec.exists {
                "one copy of the current file"
            } else {
                "none, because the file does not exist yet"
            },
            u.faint,
        ))
        .child(
            row()
                .w_full()
                .gap(9.0)
                .child(
                    u.action("Cancel", ActionKind::Ghost)
                        .grow(1.0)
                        .on_press(|st: &mut State| st.autoexec.confirm_save = false),
                )
                .child(
                    u.action("Save with backup", ActionKind::Primary)
                        .grow(1.0)
                        .on_press(|st: &mut State| crate::data::save_autoexec(st)),
                ),
        );
    let close = |st: &mut State| st.autoexec.confirm_save = false;
    u.dialog_shell()
        .key("autoexec-save-dialog")
        .role("dialog")
        .label("Save autoexec.cfg")
        .w(500.0)
        .padding(1.0)
        .modal(true, close)
        .on_key(move |st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                close(st);
                true
            } else {
                false
            }
        })
        .child(u.dialog_core().padding(20.0).child(body))
}
