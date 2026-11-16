//! Installed: the addon slots on disk. One card per slot, grouped by state.
//!
//! Every value comes from `deadlock_core::addons::read_conflict_report`, which
//! is read only. Enable, disable and install are app actions that write to the
//! game install; they are not wired here yet, so a card shows state instead of
//! a toggle that would do nothing.

use std::path::PathBuf;

use rune::{Color, El, ScrollSpec, col, el, row, spacer, text};

use crate::app::State;
use crate::model::{AddonRow, Screen, format_size};
use crate::ui::{ActionKind, BODY_MED, H2, LABEL, MONO, NUM, SMALL, Tone, Ui};

pub fn view(s: &State, u: &Ui) -> El {
    let query = s.installed_query.text.trim().to_lowercase();
    let all: Vec<&AddonRow> = s.addons.addons.iter().collect();
    let shown: Vec<&AddonRow> = all
        .iter()
        .copied()
        .filter(|addon| matches_query(addon, &query))
        .collect();
    let enabled: Vec<&AddonRow> = shown.iter().copied().filter(|a| a.enabled).collect();
    let disabled: Vec<&AddonRow> = shown.iter().copied().filter(|a| !a.enabled).collect();

    let trailing = row()
        .gap(8.0)
        .center_y()
        .child(if s.addon_job.is_running() {
            u.status_chip("scanning", Tone::Warn)
        } else if s.addons.error.is_some() {
            u.status_chip("scan failed", Tone::Bad)
        } else if s.addons.scanned {
            u.status_chip(
                &format!("{} conflicts", s.addons.conflicts.len()),
                if s.addons.conflicts.is_empty() {
                    Tone::Ok
                } else {
                    Tone::Warn
                },
            )
        } else {
            u.status_chip("not scanned", Tone::Quiet)
        })
        .child(
            u.action("Open folder", ActionKind::Secondary)
                .key("installed-open-folder")
                .on_press(|st: &mut State| {
                    let folder = addons_folder(st);
                    reveal(&mut st.status, folder);
                }),
        )
        .child(
            u.pill("Rescan", "↻", ActionKind::Ghost)
                .key("installed-rescan")
                .on_press(|st: &mut State| {
                    if st.demo {
                        st.status =
                            "Demo fixture: the addon scan stays on the fixture. Nothing was read."
                                .to_string();
                    } else {
                        crate::data::start_addon_scan(st);
                    }
                }),
        );

    let detail = if !s.game_detected {
        "No Deadlock install was found, so there is no addon folder to read.".to_string()
    } else if !s.addons.scanned {
        "Reading game/citadel/addons in the background.".to_string()
    } else {
        format!(
            "{} · {}",
            s.addons.summary(),
            if query.is_empty() {
                "lower slot wins a shared path".to_string()
            } else {
                format!("{} shown for \"{}\"", shown.len(), s.installed_query.text.trim())
            }
        )
    };

    let mut page = col()
        .key("installed-page")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .px(24.0)
        .pt(18.0)
        .pb(16.0)
        .gap(13.0)
        .child(u.glow(
            "installed-glow",
            [0.10, 0.0],
            (720.0, 420.0),
            u.glow_warm,
        ))
        .child(u.page_head(
            Screen::Installed.kicker(),
            "Installed",
            &detail,
            Some(trailing),
        ))
        .child(folder_strip(s, u));

    if let Some(error) = &s.addons.error
        && s.game_detected
    {
        page = page.child(error_card(s, u, error));
    }

    let body = if !s.game_detected {
        u.empty_state(
            "▦",
            "No Deadlock install",
            "The manager reads game/citadel/addons from the installed game. Point Settings at a Deadlock folder, then rescan.",
            Some(
                u.action("Open Settings", ActionKind::Secondary)
                    .on_press(|st: &mut State| st.go(Screen::Settings)),
            ),
        )
    } else if !s.addons.scanned && s.addon_job.is_running() {
        loading_grid(u)
    } else if s.addons.addons.is_empty() {
        u.empty_state(
            "▦",
            "No addons installed",
            "The addon folder is empty. A mod is pakNN_dir.vpk, NN from 01 to 99, placed directly in game/citadel/addons.",
            None,
        )
    } else if shown.is_empty() {
        u.empty_state(
            "⌕",
            "No addon matches",
            "The search covers the file name, slot number and chunk files. Clear it to see every slot.",
            None,
        )
    } else {
        let mut sections = col().w_full().gap(9.0);
        if !enabled.is_empty() {
            sections = sections
                .child(u.grid_head("ENABLED", &format!("{} slots", enabled.len())))
                .child(grid(s, u, &enabled));
        }
        if !disabled.is_empty() {
            sections = sections
                .child(el().h(8.0))
                .child(u.grid_head("DISABLED", &format!("{} slots", disabled.len())))
                .child(grid(s, u, &disabled));
        }
        col()
            .key("installed-scroll")
            .grow(1.0)
            .min_h(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(sections)
    };

    page.child(body)
}

fn matches_query(addon: &AddonRow, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    if addon.file.to_lowercase().contains(query) {
        return true;
    }
    if addon.slot_label().contains(query) {
        return true;
    }
    addon
        .chunk_files
        .iter()
        .any(|chunk| chunk.to_lowercase().contains(query))
}

fn folder_strip(s: &State, u: &Ui) -> El {
    let folder = s
        .addons_dir
        .clone()
        .unwrap_or_else(|| "game/citadel/addons not found".to_string());
    let mut meta = row().gap(6.0).center_y();
    if s.addons.scanned {
        meta = meta
            .child(u.chip(
                &format!("{} enabled", s.addons.enabled_count()),
                u.teal,
                u.teal.alpha(0.10),
            ))
            .child(u.chip(
                &format!("{} disabled", s.addons.disabled_count()),
                u.faint,
                Color::WHITE.alpha(0.04),
            ))
            .child(u.chip(
                &format!("{} chunks", s.addons.chunk_count()),
                u.violet,
                u.violet.alpha(0.10),
            ))
            .child(u.chip(
                &format_size(s.addons.total_bytes()),
                u.muted,
                Color::WHITE.alpha(0.04),
            ));
    } else {
        meta = meta.child(u.status_chip("folder not read yet", Tone::Quiet));
    }

    row()
        .key("installed-folder")
        .w_full()
        .padding(11.0)
        .gap(12.0)
        .center_y()
        .radius(10.0)
        .fill(Color::WHITE.alpha(0.022))
        .border(1.0, u.hair)
        .child(text("ADDONS").font(LABEL).color(u.faint))
        .child(
            text(folder)
                .font(MONO)
                .color(u.muted)
                .grow(1.0)
                .min_w(0.0)
                .truncate()
                .selectable(true),
        )
        .child(meta)
}

fn grid<'a>(s: &State, u: &Ui, addons: &[&'a AddonRow]) -> El {
    let mut rows = col().w_full().gap(10.0);
    for chunk in addons.chunks(4) {
        let mut r = row().w_full().gap(10.0).items_start();
        for addon in chunk {
            r = r.child(card(s, u, addon));
        }
        for _ in chunk.len()..4 {
            r = r.child(el().grow(1.0).min_w(0.0).h(1.0));
        }
        rows = rows.child(r);
    }
    rows
}

fn card(s: &State, u: &Ui, addon: &AddonRow) -> El {
    let tone = if addon.enabled { Tone::Ok } else { Tone::Quiet };
    let inner = if addon.enabled { u.ink } else { u.muted };
    let folder = s.addons_dir.clone().unwrap_or_default();
    let file = addon.file.clone();
    let reveal_path = PathBuf::from(&folder).join(&file);
    let label = format!("Slot {} {}", addon.slot_label(), addon.file);
    let slot = addon.slot;
    let currently_enabled = addon.enabled;
    let chunks = if addon.chunk_files.is_empty() {
        "single file".to_string()
    } else {
        let shown: Vec<String> = addon.chunk_files.iter().take(3).cloned().collect();
        let extra = addon.chunk_files.len().saturating_sub(shown.len());
        if extra > 0 {
            format!("{} + {extra}", shown.join(" · "))
        } else {
            shown.join(" · ")
        }
    };
    let meta = format!(
        "{} · {} · {}",
        addon.size_label(),
        addon.entries_label(),
        addon.chunks_label()
    );

    col()
        .key(format!("addon-{:02}", addon.slot))
        .grow(1.0)
        .min_w(0.0)
        .radius(12.0)
        .fill(Color::WHITE.alpha(0.026))
        .border(1.0, u.hair)
        .hover(Color::WHITE.alpha(0.042))
        .motion(rune::tok::motion::SNAPPY)
        .child(
            row()
                .w_full()
                .px(12.0)
                .pt(11.0)
                .gap(9.0)
                .items_center()
                .child(
                    text(format!("#{}", addon.slot_label()))
                        .font(NUM)
                        .color(if addon.enabled { u.accent } else { u.faint }),
                )
                .child(spacer())
                .child(u.status_chip(addon.state_label(), tone)),
        )
        .child(
            col()
                .w_full()
                .min_w(0.0)
                .px(12.0)
                .pt(9.0)
                .gap(4.0)
                .child(
                    text(addon.file.clone())
                        .font(MONO)
                        .color(inner)
                        .truncate(),
                )
                .child(
                    text(meta)
                        .font(SMALL)
                        .color(u.muted)
                        .truncate(),
                )
                .child(
                    text(chunks)
                        .font(LABEL)
                        .color(u.faint)
                        .truncate(),
                ),
        )
        .child(el().h(10.0))
        .child(
            row()
                .w_full()
                .px(12.0)
                .py(8.0)
                .gap(8.0)
                .center_y()
                .border_sides([1.0, 0.0, 0.0, 0.0], u.hair)
                .child(u.toggle(addon.enabled).label(format!("Slot {} enabled", addon.slot_label())))
                .child(
                    text(if addon.enabled { "ACTIVE" } else { "INACTIVE" })
                        .font(LABEL)
                        .color(tone.fg(u))
                        .grow(1.0),
                )
                .child(
                    u.action(
                        if addon.enabled { "Disable" } else { "Enable" },
                        ActionKind::Ghost,
                    )
                    .key(format!("toggle-{:02}", addon.slot))
                    .label(format!(
                        "{} slot {}",
                        if addon.enabled { "Disable" } else { "Enable" },
                        addon.slot_label()
                    ))
                    .on_press(move |st: &mut State| {
                        st.addon_dialog = Some((slot, !currently_enabled));
                    }),
                )
                .child(
                    u.action("Reveal", ActionKind::Ghost)
                        .key(format!("reveal-{:02}", addon.slot))
                        .label(&label)
                        .on_press(move |st: &mut State| {
                            reveal(&mut st.status, reveal_path.clone());
                        }),
                ),
        )
}

fn error_card(s: &State, u: &Ui, error: &str) -> El {
    let detail = if !s.game_detected {
        "The manager looks for Deadlock under the Steam roots and DAH_GAME_DIR. Settings lists what it checked."
    } else {
        "The addon folder exists but could not be read. The status line kept the exact reason."
    };
    row()
        .w_full()
        .gap(11.0)
        .items_start()
        .padding(12.0)
        .radius(10.0)
        .fill(u.rose.alpha(0.05))
        .border(1.0, u.rose.alpha(0.20))
        .child(u.glyph_box("!", Tone::Bad, 28.0))
        .child(
            col()
                .grow(1.0)
                .gap(3.0)
                .child(
                    text("The addon scan did not finish")
                        .font(BODY_MED)
                        .color(u.ink),
                )
                .child(text(error.to_string()).font(SMALL).color(u.muted).wrap())
                .child(text(detail).font(SMALL).color(u.faint).wrap()),
        )
}

fn loading_grid(u: &Ui) -> El {
    let mut rows = col().w_full().gap(10.0);
    for _ in 0..2 {
        let mut r = row().w_full().gap(10.0);
        for _ in 0..4 {
            r = r.child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .h(126.0)
                    .radius(12.0)
                    .fill(Color::WHITE.alpha(0.022))
                    .border(1.0, u.hair)
                    .center()
                    .child(text("reading slot").font(LABEL).color(u.faint)),
            );
        }
        rows = rows.child(r);
    }
    col()
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .gap(10.0)
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(u.dot(Tone::Warn))
                .child(
                    text("Reading game/citadel/addons on a worker. This view keeps painting.")
                        .font(SMALL)
                        .color(u.muted),
                ),
        )
        .child(rows)
}

fn addons_folder(s: &State) -> PathBuf {
    s.addons_dir
        .clone()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&s.paths.game).join("game/citadel/addons"))
}

/// Reveal a file or folder in the system file manager. This is a read-only app
/// action: no game file is renamed, copied or written.
fn reveal(status: &mut String, path: PathBuf) {
    if rune::is_headless() {
        *status = format!("Headless run: reveal was not sent for {}.", path.display());
        return;
    }
    if !path.exists() {
        *status = format!("{} does not exist on disk.", path.display());
        return;
    }
    let result = reveal_command(&path);
    *status = match result {
        Ok(()) => format!("Revealed {} in the file manager.", path.display()),
        Err(error) => format!("The file manager could not open {}: {error}", path.display()),
    };
}

#[cfg(target_os = "macos")]
fn reveal_command(path: &PathBuf) -> Result<(), String> {
    std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "windows")]
fn reveal_command(path: &PathBuf) -> Result<(), String> {
    std::process::Command::new("explorer")
        .arg(format!("/select,{}", path.display()))
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn reveal_command(path: &PathBuf) -> Result<(), String> {
    let folder = path.parent().unwrap_or(path);
    std::process::Command::new("xdg-open")
        .arg(folder)
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// The enable or disable approval. A slot toggle is one rename in the addon
/// folder, and it is an app action behind this confirm step.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    let (slot, enable) = s.addon_dialog?;
    let file = s
        .addons
        .addons
        .iter()
        .find(|addon| addon.slot == slot)
        .map(|addon| addon.file.clone())
        .unwrap_or_else(|| format!("pak{slot:02}_dir.vpk"));
    let title = if enable { "Enable slot" } else { "Disable slot" };
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
                        .child(text(format!("{title} {slot:02}")).font(H2).color(u.ink))
                        .child(
                            text(if enable {
                                "Renames the slot file back to pakNN_dir.vpk. The game loads it on the next start."
                            } else {
                                "Renames the slot file to pakNN_dir.vpk.disabled. Chunk files stay attached."
                            })
                            .font(SMALL)
                            .color(u.muted)
                            .wrap(),
                        ),
                )
                .child(u.glyph_box(
                    if enable { "✓" } else { "∅" },
                    if enable { Tone::Ok } else { Tone::Warn },
                    36.0,
                )),
        )
        .child(u.divider())
        .child(u.kv("Slot", &format!("{slot:02}"), u.ink))
        .child(u.kv("File", &file, u.muted))
        .child(u.kv(
            "Refuses",
            "a slot with both names present, or a missing addon folder",
            u.faint,
        ))
        .child(
            row()
                .w_full()
                .gap(9.0)
                .child(
                    u.action("Cancel", ActionKind::Ghost)
                        .grow(1.0)
                        .on_press(|st: &mut State| st.addon_dialog = None),
                )
                .child(
                    u.action(title, ActionKind::Primary)
                        .grow(1.0)
                        .on_press(move |st: &mut State| {
                            crate::data::set_addon_enabled(st, slot, enable);
                        }),
                ),
        );
    let close = |st: &mut State| st.addon_dialog = None;
    Some(
        u.dialog_shell()
            .key("addon-toggle-dialog")
            .role("dialog")
            .label(title)
            .w(480.0)
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
            .child(u.dialog_core().padding(20.0).child(body)),
    )
}
