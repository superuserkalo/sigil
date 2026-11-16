//! Launch: modded, vanilla and the restore path.
//!
//! `deadlock_core::launch` owns the stash and restore. This screen is the app
//! action surface: every launch and every stash sits behind a confirm dialog,
//! and a vanilla launch is explicit about moving addon files out of the addon
//! folder before Steam starts.

use rune::{Color, El, Key, KeyEvent, col, el, row, text};

use crate::app::State;
use crate::model::Screen;
use crate::ui::{ActionKind, BODY_MED, H2, LABEL, MONO, SMALL, Tone, Ui};

pub struct Read {
    pub stash: Option<deadlock_core::launch::StashStatus>,
    pub gameinfo_present: bool,
    pub gameinfo_addons: bool,
    pub addons_dir: Option<String>,
    pub addon_files: usize,
    pub enabled: usize,
    pub data_dir: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Dialog {
    None,
    Modded,
    Vanilla,
    Restore,
}

pub struct View {
    pub loaded: bool,
    pub error: Option<String>,
    pub read: Option<Read>,
    pub dialog: Dialog,
    pub busy: bool,
    pub last: Option<String>,
}

impl Default for View {
    fn default() -> Self {
        View {
            loaded: false,
            error: None,
            read: None,
            dialog: Dialog::None,
            busy: false,
            last: None,
        }
    }
}

pub fn view(s: &State, u: &Ui) -> El {
    let v = &s.launch;
    let pending = v.read.as_ref().and_then(|read| read.stash.as_ref());

    let trailing = row()
        .gap(8.0)
        .center_y()
        .child(if v.busy || s.launch_job.is_running() {
            u.status_chip("working", Tone::Warn)
        } else if let Some(stash) = pending {
            u.status_chip(
                &format!("{} files stashed", stash.present),
                Tone::Warn,
            )
        } else if v.loaded {
            u.status_chip("no stash", Tone::Ok)
        } else {
            u.status_chip("not read", Tone::Quiet)
        })
        .child(
            u.pill("Recheck", "↻", ActionKind::Ghost)
                .key("launch-recheck")
                .on_press(|st: &mut State| crate::data::start_launch_read(st)),
        );

    let detail = if let Some(error) = &v.error {
        error.clone()
    } else if let Some(stash) = pending {
        format!(
            "A vanilla stash was created {} and still holds {} file(s). Restore before editing addons.",
            stash.created_at, stash.present
        )
    } else if v.loaded {
        "Steam starts the game with the addon folder exactly as it sits on disk. Nothing is copied or repaired here."
            .to_string()
    } else {
        "Reading the game status and the vanilla stash.".to_string()
    };

    let mut page = col()
        .key("launch-page")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .px(24.0)
        .pt(18.0)
        .pb(16.0)
        .gap(13.0)
        .child(u.glow(
            "launch-glow",
            [0.5, 0.0],
            (900.0, 420.0),
            u.glow_warm,
        ))
        .child(u.page_head(
            Screen::Launch.kicker(),
            "Launch",
            &detail,
            Some(trailing),
        ));

    if let Some(last) = &v.last {
        page = page.child(result_strip(u, last));
    }

    let body = if !v.loaded && s.launch_job.is_running() {
        u.note("Reading game status and the stash manifest.")
    } else if !s.game_detected {
        u.empty_state(
            "▶",
            "No Deadlock install",
            "Launch hands a Steam URL to the platform opener and needs a game install for the paths. Settings shows what discovery checked.",
            Some(
                u.action("Open Settings", ActionKind::Secondary)
                    .on_press(|st: &mut State| st.go(Screen::Settings)),
            ),
        )
    } else {
        let read = v.read.as_ref();
        col()
            .w_full()
            .gap(12.0)
            .child(
                row()
                    .w_full()
                    .gap(12.0)
                    .items_start()
                    .child(modded_card(s, u, read, pending.is_some()))
                    .child(vanilla_card(s, u, read, pending.is_some())),
            )
            .child(stash_card(s, u, pending))
            .child(state_row(s, u, read))
    };

    page.child(body)
}

/// Mounted at the root so the launch confirmation overlays the shell.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    match s.launch.dialog {
        Dialog::None => None,
        dialog => Some(confirm(u, s, dialog)),
    }
}

fn result_strip(u: &Ui, message: &str) -> El {
    let bad = message.contains("not ")
        || message.contains("could not")
        || message.contains("failed")
        || message.contains("refus");
    row()
        .key("launch-result")
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

fn modded_card(s: &State, u: &Ui, read: Option<&Read>, stashed: bool) -> El {
    let addons = read
        .map(|read| {
            format!(
                "{} addon file(s), {} enabled",
                read.addon_files, read.enabled
            )
        })
        .unwrap_or_else(|| "addon folder not read".to_string());
    let ready = s.game_detected && !stashed && !s.launch.busy;
    col()
        .key("launch-modded-card")
        .grow(1.0)
        .min_w(0.0)
        .gap(10.0)
        .padding(18.0)
        .radius(16.0)
        .fill(u.accent.alpha(0.06))
        .border(1.0, u.accent.alpha(0.20))
        .child(
            row()
                .w_full()
                .gap(12.0)
                .items_start()
                .child(
                    col()
                        .grow(1.0)
                        .gap(5.0)
                        .child(u.eyebrow("STEAM · APP 1422450"))
                        .child(text("Launch modded").font(H2).color(u.ink))
                        .child(
                            text("Steam starts Deadlock with whatever is enabled in game/citadel/addons.")
                                .font(SMALL)
                                .color(u.muted)
                                .wrap(),
                        ),
                )
                .child(u.glyph_box("▶", Tone::Accent, 40.0)),
        )
        .child(el().grow(1.0).min_h(6.0))
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(
                    text(addons)
                        .font(MONO)
                        .color(u.muted)
                        .grow(1.0)
                        .truncate(),
                ),
        )
        .child(
            if stashed {
                u.note("Addons are stashed, so a modded launch would start an empty addon folder. Restore first.")
            } else {
                u.action("Launch modded", ActionKind::Primary)
                    .w_full()
                    .disabled(!ready)
                    .opacity(if ready { 1.0 } else { 0.5 })
                    .on_press(|st: &mut State| st.launch.dialog = Dialog::Modded)
            },
        )
}

fn vanilla_card(s: &State, u: &Ui, read: Option<&Read>, stashed: bool) -> El {
    let ready = s.game_detected && !stashed && !s.launch.busy;
    let count = read.map(|read| read.addon_files).unwrap_or(0);
    col()
        .key("launch-vanilla-card")
        .grow(1.0)
        .min_w(0.0)
        .gap(10.0)
        .padding(18.0)
        .radius(16.0)
        .fill(Color::WHITE.alpha(0.026))
        .border(1.0, u.hair)
        .child(
            row()
                .w_full()
                .gap(12.0)
                .items_start()
                .child(
                    col()
                        .grow(1.0)
                        .gap(5.0)
                        .child(u.eyebrow("STASH THEN LAUNCH"))
                        .child(text("Launch vanilla").font(H2).color(u.ink))
                        .child(
                            text("Moves every addon file into a timestamped stash under the app data folder, then starts Steam.")
                                .font(SMALL)
                                .color(u.muted)
                                .wrap(),
                        ),
                )
                .child(u.glyph_box("◇", Tone::Warn, 40.0)),
        )
        .child(el().grow(1.0).min_h(6.0))
        .child(
            text(format!(
                "{count} file(s) would move. Chunk files travel with their slot, and the manifest is written before the first move."
            ))
            .font(MONO)
            .color(u.muted)
            .wrap(),
        )
        .child(
            if stashed {
                u.note("A stash is already pending. Restore it instead of stashing twice.")
            } else {
                u.action("Launch vanilla", ActionKind::Secondary)
                    .w_full()
                    .disabled(!ready)
                    .opacity(if ready { 1.0 } else { 0.5 })
                    .on_press(|st: &mut State| st.launch.dialog = Dialog::Vanilla)
            },
        )
}

fn stash_card(s: &State, u: &Ui, stash: Option<&deadlock_core::launch::StashStatus>) -> El {
    match stash {
        Some(stash) => {
            let mut files = col().w_full().gap(3.0);
            for file in stash.files.iter().take(6) {
                files = files.child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .center_y()
                        .child(
                            text(
                                file.to
                                    .file_name()
                                    .map(|name| name.to_string_lossy().to_string())
                                    .unwrap_or_else(|| file.to.display().to_string()),
                            )
                            .font(MONO)
                            .color(u.muted)
                            .grow(1.0)
                            .truncate(),
                        )
                        .child(
                            text(format!("{} bytes", file.bytes))
                                .font(LABEL)
                                .color(u.faint),
                        ),
                );
            }
            let extra = stash.files.len().saturating_sub(6);
            row()
                .key("launch-stash")
                .w_full()
                .gap(16.0)
                .items_start()
                .padding(14.0)
                .radius(14.0)
                .fill(u.amber.alpha(0.05))
                .border(1.0, u.amber.alpha(0.18))
                .child(u.glyph_box("◇", Tone::Warn, 34.0))
                .child(
                    col()
                        .grow(1.0)
                        .gap(5.0)
                        .child(text("Vanilla stash pending").font(BODY_MED).color(u.ink))
                        .child(
                            text(format!(
                                "{} file(s) present, {} missing, {} total bytes, created {}",
                                stash.present, stash.missing, stash.total_bytes, stash.created_at
                            ))
                            .font(SMALL)
                            .color(u.muted),
                        )
                        .child(files)
                        .child(if extra > 0 {
                            text(format!("+ {extra} more")).font(LABEL).color(u.faint)
                        } else {
                            el()
                        })
                        .child(if let Some(problem) = &stash.problem {
                            text(problem.clone()).font(SMALL).color(u.rose).wrap()
                        } else {
                            el()
                        }),
                )
                .child(
                    u.action("Restore addons", ActionKind::Primary)
                        .disabled(s.launch.busy)
                        .on_press(|st: &mut State| st.launch.dialog = Dialog::Restore),
                )
        }
        None => row()
            .key("launch-no-stash")
            .w_full()
            .padding(12.0)
            .gap(10.0)
            .center_y()
            .radius(12.0)
            .fill(Color::WHITE.alpha(0.02))
            .border(1.0, u.hair)
            .child(u.dot(Tone::Ok))
            .child(
                text("No vanilla stash. The addon folder is where a modded launch expects it.")
                    .font(SMALL)
                    .color(u.muted)
                    .grow(1.0),
            ),
    }
}

fn state_row(s: &State, u: &Ui, read: Option<&Read>) -> El {
    let (gameinfo, addons_dir, data_dir) = match read {
        Some(read) => (
            if !read.gameinfo_present {
                "missing".to_string()
            } else if read.gameinfo_addons {
                "lists citadel/addons".to_string()
            } else {
                "does not list citadel/addons".to_string()
            },
            read.addons_dir.clone().unwrap_or_else(|| "not found".to_string()),
            read.data_dir.clone(),
        ),
        None => (
            "not read".to_string(),
            "not read".to_string(),
            s.paths.data.clone(),
        ),
    };
    row()
        .w_full()
        .gap(12.0)
        .items_start()
        .child(
            col()
                .grow(1.0)
                .gap(6.0)
                .padding(14.0)
                .radius(12.0)
                .fill(Color::WHITE.alpha(0.022))
                .border(1.0, u.hair)
                .child(text("GAMEINFO").font(LABEL).color(u.faint))
                .child(
                    text(format!("SearchPaths {gameinfo}"))
                        .font(SMALL)
                        .color(if gameinfo.contains("lists") { u.teal } else { u.amber })
                        .wrap(),
                )
                .child(
                    text("A game update or a Steam file verification resets this file. Repair lives in Updates.")
                        .font(LABEL)
                        .color(u.faint)
                        .wrap(),
                )
                .child(
                    text(addons_dir)
                        .font(MONO)
                        .color(u.muted)
                        .wrap_anywhere(),
                ),
        )
        .child(
            col()
                .w(330.0)
                .gap(6.0)
                .padding(14.0)
                .radius(12.0)
                .fill(Color::WHITE.alpha(0.022))
                .border(1.0, u.hair)
                .child(text("STASH ROOT").font(LABEL).color(u.faint))
                .child(
                    text(data_dir)
                        .font(MONO)
                        .color(u.muted)
                        .wrap_anywhere(),
                )
                .child(
                    text("Manifests are JSON under vanilla-stash. One process owns the folder.")
                        .font(LABEL)
                        .color(u.faint)
                        .wrap(),
                ),
        )
}

fn confirm(u: &Ui, s: &State, dialog: Dialog) -> El {
    let (title, eyebrow, body_text, action) = match dialog {
        Dialog::Modded => (
            "Launch Deadlock",
            "APP ACTION · STEAM",
            "Steam starts the game with the addon folder as it sits on disk. Nothing is copied, renamed or repaired.",
            "Launch through Steam",
        ),
        Dialog::Vanilla => (
            "Launch vanilla",
            "APP ACTION · STASH AND STEAM",
            "Every addon file moves into the stash first. If the move fails part way, the manifest lets a restore finish. Steam starts only after the stash completes.",
            "Stash and launch",
        ),
        Dialog::Restore => (
            "Restore addons",
            "APP ACTION · STASH",
            "Files move back from the stash into game/citadel/addons. Files already present are reported, not overwritten.",
            "Restore",
        ),
        Dialog::None => (
            "Launch",
            "APP ACTION",
            "Nothing is pending.",
            "Close",
        ),
    };
    let addons = match s.launch.read.as_ref() {
        Some(read) => format!("{} file(s), {} enabled", read.addon_files, read.enabled),
        None => "addon folder not read".to_string(),
    };
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
                        .child(u.eyebrow(eyebrow))
                        .child(text(title).font(H2).color(u.ink))
                        .child(text(body_text).font(SMALL).color(u.muted).wrap()),
                )
                .child(u.glyph_box("▶", Tone::Accent, 36.0)),
        )
        .child(u.divider())
        .child(u.kv("Game", &s.paths.game, u.ink))
        .child(u.kv("Addons", &addons, u.muted))
        .child(u.kv(
            "Headless",
            if rune::is_headless() {
                "this process has no window; the opener is not sent"
            } else {
                "no, the platform opener receives the request"
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
                        .on_press(|st: &mut State| st.launch.dialog = Dialog::None),
                )
                .child(
                    u.action(action, ActionKind::Primary)
                        .grow(1.0)
                        .on_press(move |st: &mut State| crate::data::run_launch(st, dialog)),
                ),
        );
    let close = |st: &mut State| st.launch.dialog = Dialog::None;
    u.dialog_shell()
        .key("launch-confirm")
        .role("dialog")
        .label(title)
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
