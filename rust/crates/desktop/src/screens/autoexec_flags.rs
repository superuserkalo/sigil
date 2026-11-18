//! Flags for commands the game no longer has, on the Autoexec page and under the launch options
//! field (Deadlock Mod Manager's stale command check).
//!
//! The list and the parsing are `deadlock_core::obsolete_commands`: a removed command is
//! flagged with why, a renamed one with its new name and a button that rewrites the line. The
//! rewrite only edits the page's text; the file changes when the player saves, as with any edit.

use deadlock_core::obsolete_commands::{self as obsolete, CommandFinding, FindingKind};
use rune::{El, col, icon, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{ActionKind, BODY_MED, MONO, SMALL, Ui};

/// The findings for one autoexec line.
pub fn line_findings(line: &str) -> Vec<CommandFinding> {
    obsolete::check_autoexec(line)
}

/// `line` with `command` replaced by `replacement` wherever it stands as a command: at the
/// start of the line or after a `;`, matched without case. Values and comments stay as typed.
pub fn rename_line(line: &str, command: &str, replacement: &str) -> String {
    let mut out = String::with_capacity(line.len() + replacement.len());
    let mut rest = line;
    let mut at_statement = true;
    while !rest.is_empty() {
        if rest.starts_with("//") {
            out.push_str(rest);
            break;
        }
        let ch = rest.chars().next().unwrap_or(' ');
        if at_statement && !ch.is_whitespace() {
            let end = rest.find(|c: char| c.is_whitespace() || c == ';').unwrap_or(rest.len());
            let token = &rest[..end];
            if token.eq_ignore_ascii_case(command) {
                out.push_str(replacement);
            } else {
                out.push_str(token);
            }
            rest = &rest[end..];
            at_statement = false;
            continue;
        }
        if ch == '"' {
            // A quoted value runs to its closing quote.
            let close = rest[1..].find('"').map(|i| i + 2).unwrap_or(rest.len());
            out.push_str(&rest[..close]);
            rest = &rest[close..];
            continue;
        }
        if ch == ';' {
            at_statement = true;
        }
        out.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    out
}

/// What a finding says, in one line.
pub fn sentence(finding: &CommandFinding) -> String {
    match (&finding.kind, &finding.replacement) {
        (FindingKind::Renamed, Some(new)) => fill(tr("{} is now {}."), &[&finding.command, new]),
        _ => fill(tr("{} is no longer in the game."), &[&finding.command]),
    }
}

/// The quiet mark at the end of a flagged command row.
pub fn row_mark(u: &Ui, id: u64, line: &str) -> Option<El> {
    let finding = line_findings(line).into_iter().next()?;
    Some(
        rune::el()
            .key(format!("autoexec-flag-{id}"))
            .size(20.0, 20.0)
            .center()
            .label(sentence(&finding))
            .child(icon(&icons::TRIANGLE_ALERT, 14.0).color(u.amber).a11y_hidden()),
    )
}

/// The notice above the commands: every line the game no longer reads, with a fix for each
/// rename. `None` when there is nothing to say.
pub fn notice(s: &State, u: &Ui) -> Option<El> {
    let mut rows: Vec<(u64, CommandFinding)> = Vec::new();
    for block in &s.autoexec.blocks {
        if block.kind != crate::screens::autoexec::BlockKind::Command {
            continue;
        }
        for finding in line_findings(&block.edit.text) {
            rows.push((block.id, finding));
        }
    }
    if rows.is_empty() {
        return None;
    }
    let title = if rows.len() == 1 {
        tr("1 command the game no longer has").to_string()
    } else {
        fill(tr("{} commands the game no longer has"), &[&rows.len().to_string()])
    };
    let mut c = col()
        .key("autoexec-flags")
        .w_full()
        .padding(12.0)
        .gap(8.0)
        .radius(crate::ui::r(10.0))
        .fill(u.amber.alpha(0.06))
        .border(1.0, u.amber.alpha(0.28))
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(icon(&icons::TRIANGLE_ALERT, 14.0).color(u.amber).a11y_hidden())
                .child(text(title).font(BODY_MED).color(u.ink)),
        );
    for (index, (id, finding)) in rows.iter().take(8).enumerate() {
        let mut line = row()
            .key(format!("autoexec-flag-row-{index}"))
            .w_full()
            .gap(10.0)
            .center_y()
            .child(text(finding.command.clone()).font(MONO).color(u.ink).w(220.0).truncate())
            .child(
                text(if finding.note.is_empty() { sentence(finding) } else { finding.note.clone() })
                    .font(SMALL)
                    .color(u.muted)
                    .grow(1.0)
                    .min_w(0.0)
                    .wrap(),
            );
        if let Some(new) = finding.replacement.clone() {
            let id = *id;
            let old = finding.command.clone();
            line = line.child(
                u.action(&fill(tr("Use {}"), &[&new]), ActionKind::Secondary)
                    .key(format!("autoexec-flag-fix-{index}"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        let edit = st.autoexec.edit_mut(id);
                        let fixed = rename_line(&edit.text, &old, &new);
                        *edit = rune::TextEdit::new(&fixed);
                    }),
            );
        }
        c = c.child(line);
    }
    if rows.len() > 8 {
        c = c.child(text(fill(tr("{} more."), &[&(rows.len() - 8).to_string()])).font(SMALL).color(u.faint));
    }
    Some(c)
}

/// The line under the launch options field.
pub fn launch_notice(u: &Ui, value: &str) -> Option<El> {
    let findings = obsolete::check_launch_options(value);
    if findings.is_empty() {
        return None;
    }
    let mut c = col().key("autoexec-launch-flags").w_full().gap(4.0);
    for (index, finding) in findings.iter().take(4).enumerate() {
        c = c.child(
            row()
                .key(format!("autoexec-launch-flag-{index}"))
                .w_full()
                .gap(6.0)
                .items_start()
                .child(icon(&icons::TRIANGLE_ALERT, 12.0).color(u.amber).mt(2.0).a11y_hidden())
                .child(
                    text(if finding.note.is_empty() {
                        sentence(finding)
                    } else {
                        format!("{} {}", sentence(finding), finding.note)
                    })
                    .font(SMALL)
                    .color(u.amber)
                    .wrap()
                    .grow(1.0)
                    .min_w(0.0),
                )
                .child(spacer()),
        );
    }
    Some(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rename_touches_only_the_command() {
        assert_eq!(rename_line("old_cmd 1", "old_cmd", "new_cmd"), "new_cmd 1");
        assert_eq!(rename_line("OLD_CMD 1 // old_cmd", "old_cmd", "new_cmd"), "new_cmd 1 // old_cmd");
        assert_eq!(rename_line("bind x \"old_cmd\"; old_cmd 2", "old_cmd", "new_cmd"), "bind x \"old_cmd\"; new_cmd 2");
        assert_eq!(rename_line("other old_cmd", "old_cmd", "new_cmd"), "other old_cmd", "a value stays");
    }
}
