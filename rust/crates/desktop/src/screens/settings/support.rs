//! Logs and support: every crash dump as a row (50 at a time, Show more), a details dialog
//! with the dump's exception and the game's own error text, crashes picked for the bug
//! report, and a description of what went wrong. Grimoire's Support section (MIT) is the
//! model: only the minidump's header and comment streams are read, never its memory.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use deadlock_core::game_session::{self, Dump, DumpInfo};
use rune::{El, TextEdit, Tx, col, row, text};

use super::i18n::{fill, tr};
use super::options;
use super::{button, detail, disabled, setting_row, stacked_row};
use crate::app::State;
use crate::icons;
use crate::ui::{ActionKind, BODY, BODY_MED, MONO, SMALL, Ui};

pub const PAGE: usize = 50;
pub const MAX_PICKED: usize = 10;

#[derive(Default)]
pub struct Support {
    pub dumps: Option<Vec<Dump>>,
    pub shown: usize,
    pub picked: BTreeSet<PathBuf>,
    pub bug: TextEdit,
}

pub fn read_crashes(st: &mut State) {
    let dumps = if st.demo {
        demo_dumps()
    } else {
        let mut roots = deadlock_core::discovery::default_steam_roots();
        if let Some(custom) = options::get().steam_root.clone() {
            roots.insert(0, PathBuf::from(custom));
        }
        game_session::list_dumps(&game_session::dump_folders(st.game_root.as_deref(), &roots))
    };
    let s = &mut st.settings.extra.support;
    s.picked.retain(|p| dumps.iter().any(|d| d.path == *p));
    s.shown = s.shown.clamp(PAGE, PAGE.max(dumps.len()));
    s.dumps = Some(dumps);
}

fn when(s: &State, d: &Dump) -> String {
    d.modified
        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|t| super::tabs::date_label(s, t.as_secs()))
        .unwrap_or_default()
}

fn name(d: &Dump) -> String {
    d.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The crash list: one row per dump, a box to include it in the report, details.
pub fn crash_rows(s: &State, u: &Ui) -> Vec<El> {
    let sup = &s.settings.extra.support;
    let Some(dumps) = &sup.dumps else { return Vec::new() };
    let mut rows = Vec::new();
    for (index, d) in dumps.iter().take(sup.shown.max(PAGE)).enumerate() {
        let picked = sup.picked.contains(&d.path);
        let path = d.path.clone();
        let open = d.path.clone();
        let full = sup.picked.len() >= MAX_PICKED && !picked;
        let source = if d.source == "steam" { tr("Steam") } else { tr("Deadlock") };
        rows.push(
            row()
                .key(format!("settings-crash-{index}"))
                .w_full()
                .h(48.0)
                .gap(12.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(disabled(
                    checkbox(u, picked)
                        .key(format!("settings-crash-pick-{index}"))
                        .label(fill(tr("Include the crash from {} in the report"), &[&when(s, d)]))
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| {
                            let p = &mut st.settings.extra.support.picked;
                            if !p.remove(&path) && p.len() < MAX_PICKED {
                                p.insert(path.clone());
                            }
                        }),
                    full,
                ))
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(text(when(s, d)).font(BODY_MED).color(u.ink))
                        .child(
                            text(format!("{source} · {} · {}", super::size_label(d.bytes), name(d)))
                                .font(SMALL)
                                .color(u.muted)
                                .truncate(),
                        ),
                )
                .child(
                    button(u, &format!("settings-crash-details-{index}"), "Details", ActionKind::Ghost)
                        .on_press(move |st: &mut State| {
                            st.settings.extra.dialog = Some(super::extra::Dialog::Crash(open.clone()))
                        }),
                ),
        );
    }
    if dumps.len() > sup.shown.max(PAGE) {
        let more = dumps.len() - sup.shown.max(PAGE);
        rows.push(
            row().w_full().h(44.0).center_y().child(
                button(u, "settings-crash-more", &fill(tr("Show {} more"), &[&more.min(PAGE).to_string()]), ActionKind::Ghost)
                    .on_press(|st: &mut State| st.settings.extra.support.shown = st.settings.extra.support.shown.max(PAGE) + PAGE),
            ),
        );
    }
    rows
}

fn checkbox(u: &Ui, on: bool) -> El {
    rune::el()
        .size(18.0, 18.0)
        .center()
        .radius(crate::ui::r(5.0))
        .fill(if on { u.accent } else { rune::Color::TRANSPARENT })
        .border(1.5, if on { u.accent } else { u.hair_strong })
        .role("checkbox")
        .checked(on)
        .focusable(true)
        .child(if on {
            rune::icon(&icons::CHECK, 12.0).color(u.accent_ink).a11y_hidden()
        } else {
            rune::el()
        })
}

/// The bug report rows: what happened, then the report buttons.
pub fn report_rows(s: &State, u: &Ui) -> Vec<El> {
    let sup = &s.settings.extra.support;
    let picked = sup.picked.len();
    vec![
        stacked_row(
            u,
            "settings-bug-description",
            "What went wrong",
            "Goes at the top of the report.",
            row()
                .w_full()
                .min_h(72.0)
                .px(10.0)
                .py(8.0)
                .radius(crate::ui::r(8.0))
                .fill(u.well)
                .border(1.0, u.hair)
                .child(
                    rune::text_input(&sup.bug, BODY)
                        .key("settings-bug-input")
                        .label(tr("What went wrong"))
                        .grow(1.0)
                        .color(u.ink)
                        .placeholder(tr("Describe what happened and what you expected."), u.faint)
                        .caret(u.accent, u.accent.alpha(0.22))
                        .on_edit(|st: &mut State| &mut st.settings.extra.support.bug),
                ),
        ),
        setting_row(
            u,
            "settings-report-copy",
            "Diagnostic report",
            detail(
                u,
                &if picked == 0 {
                    tr("Version, system, game status and recent log lines, with paths and keys masked.").to_string()
                } else {
                    fill(tr("Also the {} crashes you ticked above."), &[&picked.to_string()])
                },
            ),
            button(u, "settings-report-copy-run", "Copy report", ActionKind::Secondary).on_press_tx(
                |st: &mut State, tx: &mut Tx| {
                    tx.copy(report(st));
                    super::say(st, tr("Copied. Paste it into your report."), false);
                },
            ),
        ),
        setting_row(
            u,
            "settings-report-issue",
            "Open an issue",
            detail(u, "Opens Sigil's issue tracker on GitHub with your description filled in."),
            u.icon_action(&icons::EXTERNAL_LINK, tr("New issue"), ActionKind::Secondary)
                .key("settings-report-issue-open")
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(|st: &mut State, tx: &mut Tx| {
                    let url = issue_url(&st.settings.extra.support.bug.text);
                    super::tabs::open_url(st, tx, &url)
                }),
        ),
    ]
}

/// GitHub's new-issue page with a title and body from the description.
pub fn issue_url(description: &str) -> String {
    let first: String = description.lines().next().unwrap_or("").chars().take(100).collect();
    let body = format!(
        "{}\n\n<!-- Paste the diagnostic report from Settings > Logs and support here. -->",
        description.trim()
    );
    format!(
        "https://github.com/{}/issues/new?title={}&body={}",
        super::UPDATE_REPO,
        encode(&first),
        encode(&body)
    )
}

fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The report: the description, the base report, then the picked crashes. Redacted.
pub fn report(st: &State) -> String {
    let mut parts = Vec::new();
    let bug = st.settings.extra.support.bug.text.trim();
    if !bug.is_empty() {
        parts.push(format!("--- what happened ---\n{bug}\n"));
    }
    parts.push(super::tabs::diagnostic_report(st));
    // Grimoire's "Include full log": the whole log after the summary, capped at 5 MB.
    if options::get().full_log {
        parts.push(format!("\n--- full log ---\n{}", super::logs_w4::full_log()));
    }
    if !st.settings.extra.support.picked.is_empty() {
        let mut lines = vec![String::new(), "--- selected Deadlock crashes ---".to_string()];
        for path in &st.settings.extra.support.picked {
            let info = game_session::read_dump_info(path);
            lines.push(crash_text(path, &info));
        }
        parts.push(lines.join("\n"));
    }
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default();
    let joined = parts.join("\n");
    let masked = if home.len() > 1 { joined.replace(&home, "~") } else { joined };
    super::log::redact(&masked)
}

pub fn crash_text(path: &Path, info: &DumpInfo) -> String {
    let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut out = vec![format!("File: {file}")];
    if let Some(code) = info.exception {
        out.push(format!("Type: {}", game_session::exception_label(code)));
    }
    if let Some(comment) = &info.comment {
        out.push(format!("Error: {}", comment.chars().take(1500).collect::<String>()));
    }
    out.join("\n")
}

/// The details of one dump.
pub fn crash_dialog(s: &State, u: &Ui, path: &Path) -> El {
    let info = if s.demo {
        DumpInfo {
            exception: Some(0xC000_0005),
            comment: Some("FATAL ERROR: panorama/layout/hud.xml failed to load".into()),
        }
    } else {
        game_session::read_dump_info(path)
    };
    let kind = info
        .exception
        .map(game_session::exception_label)
        .unwrap_or_else(|| tr("No exception recorded").to_string());
    let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let reveal = path.display().to_string();
    let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    super::extra::shell(u, "settings-crash-dialog", &kind)
        .w(560.0)
        .child(text(format!("{file} · {}", super::size_label(bytes))).font(MONO).color(u.muted).truncate())
        .child(
            u.scroll_area(col()
                .w_full()
                .max_h(220.0)
                .padding(12.0)
                .radius(crate::ui::r(8.0))
                .fill(u.well)
                .border(1.0, u.hair)
                .scroll(rune::ScrollSpec { stick_bottom: false })
                .child(
                    text(info.comment.unwrap_or_else(|| tr("The game left no error text in this dump.").to_string()))
                        .font(MONO)
                        .color(u.prose)
                        .wrap()
                        .selectable(true)
                        .w_full(),
                )),
        )
        .child(
            text(tr("A dump can hold parts of memory. Share the file itself only with people you trust."))
                .font(SMALL)
                .color(u.muted)
                .wrap()
                .w_full(),
        )
        .child(
            row()
                .w_full()
                .gap(8.0)
                .child(rune::spacer())
                .child(button(u, "settings-crash-dialog-close", "Close", ActionKind::Ghost).on_press(super::extra::close))
                .child(
                    button(u, "settings-crash-dialog-show", "Show file", ActionKind::Secondary)
                        .on_press(move |st: &mut State| crate::data::reveal_path(st, reveal.clone())),
                ),
        )
}

fn demo_dumps() -> Vec<Dump> {
    let now = std::time::SystemTime::now();
    (0..3)
        .map(|i| Dump {
            path: PathBuf::from(format!("~/Steam/steamapps/common/Deadlock/game/bin/win64/deadlock_2026_111{i}_204501_1_crash.mdmp")),
            bytes: 2_400_000 + i as u64 * 10_000,
            modified: now.checked_sub(std::time::Duration::from_secs(86_400 * (i as u64 + 1))),
            source: "game",
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_links_carry_the_description() {
        let url = issue_url("Crash on launch\nmore");
        assert!(url.contains("title=Crash%20on%20launch&"));
        assert!(url.contains("body=Crash%20on%20launch%0Amore"));
    }

    #[test]
    fn crash_text_names_the_file_and_error() {
        let t = crash_text(
            Path::new("/x/deadlock_1.mdmp"),
            &DumpInfo {
                exception: Some(0xC000_0005),
                comment: Some("FATAL ERROR".into()),
            },
        );
        assert_eq!(t, "File: deadlock_1.mdmp\nType: Access violation\nError: FATAL ERROR");
    }
}
