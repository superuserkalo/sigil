//! Profiles: named sets of enabled addon slots.
//!
//! Every row comes from `deadlock_core::profiles::list_profiles`, which reads
//! `<workspace>/.dah/profiles`. Capture writes the workspace only. Apply renames
//! slot files inside `game/citadel/addons`, so it sits behind a confirm dialog
//! and an explicit approve flag.

use rune::{Color, El, Key, KeyEvent, ScrollSpec, TextEdit, col, el, row, text};

use crate::app::State;
use crate::model::Screen;
use crate::ui::{ActionKind, BODY, BODY_MED, H2, LABEL, MONO, SMALL, Tone, Ui};

#[derive(Clone)]
pub struct Row {
    pub name: String,
    pub slug: String,
    pub updated: String,
    pub enabled: usize,
    pub total: usize,
    /// Slot labels, for example `01 on`.
    pub slots: Vec<String>,
}

pub struct Load {
    pub rows: Vec<Row>,
    pub directory: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Dialog {
    None,
    Capture,
    Apply(usize),
}

pub struct View {
    pub loaded: bool,
    pub error: Option<String>,
    pub directory: String,
    pub rows: Vec<Row>,
    pub name: TextEdit,
    pub dialog: Dialog,
    pub busy: bool,
    pub last: Option<String>,
}

impl Default for View {
    fn default() -> Self {
        View {
            loaded: false,
            error: None,
            directory: String::new(),
            rows: Vec::new(),
            name: TextEdit::new(""),
            dialog: Dialog::None,
            busy: false,
            last: None,
        }
    }
}

pub fn view(s: &State, u: &Ui) -> El {
    let v = &s.profiles;
    let total_slots: usize = v.rows.iter().map(|row| row.total).sum();
    let trailing = row()
        .gap(8.0)
        .center_y()
        .child(if s.profiles_job.is_running() || v.busy {
            u.status_chip("working", Tone::Warn)
        } else if v.error.is_some() {
            u.status_chip("read failed", Tone::Bad)
        } else if v.loaded {
            u.status_chip(
                &format!("{} profiles", v.rows.len()),
                if v.rows.is_empty() {
                    Tone::Quiet
                } else {
                    Tone::Ok
                },
            )
        } else {
            u.status_chip("not read", Tone::Quiet)
        })
        .child(
            u.action("Capture current", ActionKind::Primary)
                .key("profiles-capture")
                .on_press(|st: &mut State| {
                    st.profiles.name = TextEdit::new("");
                    st.profiles.dialog = Dialog::Capture;
                }),
        );

    let detail = if let Some(error) = &v.error {
        error.clone()
    } else if !v.loaded {
        "Reading the profile folder in the workspace.".to_string()
    } else if v.rows.is_empty() {
        "No profiles yet. Capture the current enabled slots as the first set.".to_string()
    } else {
        format!(
            "{} profile{} · {} recorded slot{} · slot files are renamed, never deleted",
            v.rows.len(),
            if v.rows.len() == 1 { "" } else { "s" },
            total_slots,
            if total_slots == 1 { "" } else { "s" }
        )
    };

    let mut page = col()
        .key("profiles-page")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .px(24.0)
        .pt(18.0)
        .pb(16.0)
        .gap(13.0)
        .child(u.glow(
            "profiles-glow",
            [0.08, 0.0],
            (700.0, 400.0),
            u.glow_cool,
        ))
        .child(u.page_head(
            Screen::Profiles.kicker(),
            "Profiles",
            &detail,
            Some(trailing),
        ))
        .child(folder_strip(s, u));

    if let Some(last) = &v.last {
        page = page.child(result_strip(u, last));
    }

    let body = if !v.loaded && s.profiles_job.is_running() {
        loading(u)
    } else if v.rows.is_empty() {
        u.empty_state(
            "☰",
            "No profiles saved",
            "A profile records which addon slots were on. Capture one before switching between loadouts, or before a game update.",
            Some(
                u.action("Capture current slots", ActionKind::Secondary)
                    .on_press(|st: &mut State| {
                        st.profiles.name = TextEdit::new("");
                        st.profiles.dialog = Dialog::Capture;
                    }),
            ),
        )
    } else {
        let mut grid = col().w_full().gap(10.0);
        for start in (0..v.rows.len()).step_by(2) {
            let mut r = row().w_full().gap(10.0).items_start();
            for index in start..(start + 2).min(v.rows.len()) {
                r = r.child(card(s, u, &v.rows[index], index));
            }
            if v.rows.len() - start == 1 {
                r = r.child(el().grow(1.0).min_w(0.0).h(1.0));
            }
            grid = grid.child(r);
        }
        col()
            .key("profiles-scroll")
            .grow(1.0)
            .min_h(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(grid)
    };

    page.child(body)
}

/// Mounted at the root so the capture and apply dialogs overlay the shell.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    match s.profiles.dialog {
        Dialog::Capture => Some(capture_layer(s, u)),
        Dialog::Apply(index) => s
            .profiles
            .rows
            .get(index)
            .map(|profile| apply_layer(s, u, index, profile)),
        Dialog::None => None,
    }
}

fn folder_strip(s: &State, u: &Ui) -> El {
    let directory = if s.profiles.directory.is_empty() {
        ".dah/profiles".to_string()
    } else {
        s.profiles.directory.clone()
    };
    let configured = if s.addons.scanned {
        format!(
            "{} enabled across {} slots",
            s.addons.enabled_count(),
            s.addons.addons.len()
        )
    } else {
        "addon folder not read yet".to_string()
    };
    row()
        .key("profiles-folder")
        .w_full()
        .padding(11.0)
        .gap(12.0)
        .center_y()
        .radius(10.0)
        .fill(Color::WHITE.alpha(0.022))
        .border(1.0, u.hair)
        .child(text("STORE").font(LABEL).color(u.faint))
        .child(
            text(directory)
                .font(MONO)
                .color(u.muted)
                .grow(1.0)
                .min_w(0.0)
                .truncate()
                .selectable(true),
        )
        .child(u.chip(&configured, u.violet, u.violet.alpha(0.10)))
        .child(
            u.pill("Reload", "↻", ActionKind::Ghost)
                .key("profiles-reload")
                .on_press(|st: &mut State| crate::data::start_profile_list(st)),
        )
}

fn result_strip(u: &Ui, message: &str) -> El {
    let bad = message.contains("not ")
        || message.contains("could not")
        || message.contains("failed")
        || message.contains("Nothing");
    row()
        .key("profiles-result")
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

fn card(_s: &State, u: &Ui, profile: &Row, index: usize) -> El {
    let detail = format!(
        "{} of {} recorded slots enabled",
        profile.enabled, profile.total
    );
    let preview: Vec<String> = profile.slots.iter().take(8).cloned().collect();
    let more = profile.slots.len().saturating_sub(preview.len());

    let mut body = col()
        .w_full()
        .px(14.0)
        .pt(10.0)
        .gap(5.0)
        .child(text(detail).font(SMALL).color(u.muted))
        .child(
            text(preview.join("   "))
                .font(MONO)
                .color(u.faint)
                .wrap_anywhere(),
        );
    if more > 0 {
        body = body.child(
            text(format!("+ {more} more slot(s)"))
                .font(LABEL)
                .color(u.faint),
        );
    }

    col()
        .key(format!("profile-{}", profile.slug))
        .grow(1.0)
        .min_w(0.0)
        .radius(12.0)
        .fill(Color::WHITE.alpha(0.026))
        .border(1.0, u.hair)
        .hover(Color::WHITE.alpha(0.04))
        .child(
            row()
                .w_full()
                .px(14.0)
                .pt(12.0)
                .gap(10.0)
                .items_start()
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(3.0)
                        .child(
                            text(profile.name.clone())
                                .font(BODY_MED)
                                .color(u.ink)
                                .truncate(),
                        )
                        .child(
                            text(format!("updated {}", profile.updated))
                                .font(LABEL)
                                .color(u.faint),
                        ),
                )
                .child(u.chip(
                    &format!("{} on", profile.enabled),
                    u.teal,
                    u.teal.alpha(0.10),
                )),
        )
        .child(body)
        .child(el().h(10.0))
        .child(
            row()
                .w_full()
                .px(14.0)
                .py(9.0)
                .gap(8.0)
                .center_y()
                .border_sides([1.0, 0.0, 0.0, 0.0], u.hair)
                .child(
                    text(profile.slug.clone())
                        .font(LABEL)
                        .color(u.faint)
                        .grow(1.0)
                        .truncate(),
                )
                .child(
                    u.action("Apply", ActionKind::Secondary)
                        .key(format!("profile-apply-{}", profile.slug))
                        .on_press(move |st: &mut State| {
                            st.profiles.dialog = Dialog::Apply(index);
                        }),
                ),
        )
}

fn loading(u: &Ui) -> El {
    let mut rows = col().w_full().gap(10.0);
    for _ in 0..3 {
        rows = rows.child(
            col()
                .w_full()
                .h(96.0)
                .radius(12.0)
                .fill(Color::WHITE.alpha(0.022))
                .border(1.0, u.hair)
                .center()
                .child(text("reading profile").font(LABEL).color(u.faint)),
        );
    }
    rows
}

fn capture_layer(s: &State, u: &Ui) -> El {
    let enabled = if s.addons.scanned {
        format!(
            "{} enabled slot{} of {} on disk will be recorded.",
            s.addons.enabled_count(),
            if s.addons.enabled_count() == 1 { "" } else { "s" },
            s.addons.addons.len()
        )
    } else {
        "The addon folder will be read when the capture runs.".to_string()
    };
    let ready = !s.profiles.name.text.trim().is_empty() && s.game_detected;
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
                        .child(u.eyebrow("WORKSPACE WRITE · NO GAME FILES"))
                        .child(text("Capture profile").font(H2).color(u.ink))
                        .child(
                            text("The profile stores slot numbers, file names and on or off. It never copies a VPK.")
                                .font(SMALL)
                                .color(u.muted)
                                .wrap(),
                        ),
                )
                .child(u.glyph_box("☰", Tone::Accent, 36.0)),
        )
        .child(u.divider())
        .child(u.field_label("PROFILE NAME"))
        .child(
            rune::text_input(&s.profiles.name, BODY)
                .key("profile-name")
                .label("Profile name")
                .single_line()
                .color(u.ink)
                .placeholder("Ranked loadout", u.faint)
                .caret(u.accent, u.accent.alpha(0.20))
                .on_edit(|st: &mut State| &mut st.profiles.name),
        )
        .child(u.kv("Records", &enabled, u.muted))
        .child(u.kv(
            "Refuses",
            "an empty name, and a slot file that does not follow the pakNN_dir.vpk naming rule",
            u.faint,
        ))
        .child(
            row()
                .w_full()
                .gap(9.0)
                .child(
                    u.action("Cancel", ActionKind::Ghost)
                        .grow(1.0)
                        .on_press(|st: &mut State| st.profiles.dialog = Dialog::None),
                )
                .child(
                    u.action("Capture", ActionKind::Primary)
                        .grow(1.0)
                        .disabled(!ready)
                        .opacity(if ready { 1.0 } else { 0.5 })
                        .on_press(|st: &mut State| {
                            if !st.profiles.name.text.trim().is_empty() {
                                crate::data::capture_profile(st);
                            }
                        }),
                ),
        );
    dialog_layer(u, "profile-capture", "Capture profile", body, 470.0, |st| {
        st.profiles.dialog = Dialog::None
    })
}

fn apply_layer(s: &State, u: &Ui, index: usize, profile: &Row) -> El {
    let adds = profile.enabled;
    let removes = profile.total.saturating_sub(profile.enabled);
    let ready = s.game_detected && !s.profiles.busy;
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
                        .child(u.eyebrow("APP ACTION · ADDN FOLDER"))
                        .child(
                            text(format!("Apply {}", profile.name))
                                .font(H2)
                                .color(u.ink),
                        )
                        .child(
                            text("Slot files are renamed inside game/citadel/addons. Chunk files stay attached. Nothing is deleted or renumbered.")
                                .font(SMALL)
                                .color(u.muted)
                                .wrap(),
                        ),
                )
                .child(u.glyph_box("⇄", Tone::Warn, 36.0)),
        )
        .child(u.divider())
        .child(u.kv("Enables", &format!("{adds} recorded slot(s)"), u.teal))
        .child(u.kv(
            "Disables",
            &format!("{removes} recorded slot(s)"),
            if removes == 0 { u.faint } else { u.amber },
        ))
        .child(u.kv(
            "Refuses",
            "a slot with both pakNN_dir.vpk and pakNN_dir.vpk.disabled, before renaming anything",
            u.faint,
        ))
        .child(
            text(if s.game_detected {
                "The addon folder is on disk, so this can run."
            } else {
                "No Deadlock install was found. Apply will refuse."
            })
            .font(SMALL)
            .color(if s.game_detected { u.muted } else { u.rose }),
        )
        .child(
            row()
                .w_full()
                .gap(9.0)
                .child(
                    u.action("Cancel", ActionKind::Ghost)
                        .grow(1.0)
                        .on_press(|st: &mut State| st.profiles.dialog = Dialog::None),
                )
                .child(
                    u.action("Apply profile", ActionKind::Primary)
                        .grow(1.0)
                        .disabled(!ready)
                        .opacity(if ready { 1.0 } else { 0.5 })
                        .on_press(move |st: &mut State| crate::data::apply_profile(st, index)),
                ),
        );
    dialog_layer(u, "profile-apply", "Apply profile", body, 490.0, |st| {
        st.profiles.dialog = Dialog::None
    })
}

fn dialog_layer(u: &Ui, key: &str, label: &str, body: El, width: f32, close: fn(&mut State)) -> El {
    u.dialog_shell()
        .key(key)
        .role("dialog")
        .label(label)
        .w(width)
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
