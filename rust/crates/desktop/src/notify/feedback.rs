//! Send feedback from any error (issue #5).
//!
//! Every error toast, inline error box and failed-step row offers Send feedback. It opens one
//! dialog, prefilled with what failed (the error, the page, Sigil's version and the system),
//! shown in full and scrubbed (`deadlock_core::telemetry::scrub`) before anything goes, with an
//! optional note. Send uses the player-pressed diagnostics path (`telemetry::send_diagnostics`,
//! no usage consent needed because the player presses Send) and shows the `SGL-XXXX-XXXX`
//! reference code with Copy. Copy details and Open a GitHub issue (prefilled title and body, in
//! the browser) are there for players who prefer those.
//!
//! - Deadlock Mod Manager: an error boundary with Copy and Retry, and a titlebar link to its
//!   GitHub issue chooser. Toasts carry no report action.
//! - Grimoire: Settings > Support builds a sanitized bug report to copy and a prefilled GitHub
//!   issue link that asks the player to paste it. Toasts carry no report action.

use deadlock_core::telemetry;
use rune::{El, TextEdit, Tx, col, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::jobs::Job;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{ActionKind, BODY, BODY_MED, H1, MONO, SMALL, Ui};

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Where Open a GitHub issue goes; the browser opens it, Sigil sends nothing.
pub const ISSUES: &str = "https://github.com/superuserkalo/sigil/issues/new";
/// Log lines sent with the report (shown first, like everything else).
const LOG_LINES: usize = 200;
/// GitHub refuses very long issue links; the body is cut to stay under this.
const URL_BODY_MAX: usize = 5000;

pub struct Feedback {
    pub open: bool,
    /// What failed: the toast's title and detail, or an inline box's text.
    pub title: String,
    pub detail: String,
    /// The page it happened on.
    pub page: String,
    /// The player's optional "what were you doing?".
    pub note: TextEdit,
    /// The scrubbed log tail, read when the dialog opens.
    pub log: String,
    pub sending: Job<String>,
    pub code: Option<Result<String, String>>,
}

impl Default for Feedback {
    fn default() -> Self {
        Feedback {
            open: false,
            title: String::new(),
            detail: String::new(),
            page: String::new(),
            note: TextEdit::new(""),
            log: String::new(),
            sending: Job::default(),
            code: None,
        }
    }
}

impl Feedback {
    /// Take a finished send.
    pub fn poll(&mut self) -> bool {
        match self.sending.try_take() {
            Some(result) => {
                self.code = Some(result);
                true
            }
            None => false,
        }
    }

    /// What failed, as the dialog shows it and every channel sends it: scrubbed.
    pub fn details(&self, game_found: bool) -> String {
        let mut lines = vec![format!("Error: {}", self.title)];
        if !self.detail.trim().is_empty() {
            lines.push(self.detail.trim().to_string());
        }
        lines.push(format!("Page: {}", self.page));
        lines.push(format!("Sigil {VERSION}"));
        lines.push(format!("System: {} {}", std::env::consts::OS, std::env::consts::ARCH));
        lines.push(format!("Game found: {game_found}"));
        telemetry::scrub(&lines.join("\n"))
    }

    /// The summary the Worker gets: the player's note first, then what failed.
    pub fn summary(&self, game_found: bool) -> String {
        let note = self.note.text.trim();
        let details = self.details(game_found);
        if note.is_empty() {
            details
        } else {
            telemetry::scrub(&format!("Note: {note}\n\n{details}"))
        }
    }

    /// A GitHub new-issue link with the title and the scrubbed details filled in.
    pub fn issue_url(&self, game_found: bool) -> String {
        let note = self.note.text.trim();
        let mut body = String::new();
        if !note.is_empty() {
            body.push_str(&telemetry::scrub(note));
            body.push_str("\n\n");
        }
        body.push_str("```\n");
        body.push_str(&self.details(game_found));
        body.push_str("\n```\n");
        if body.len() > URL_BODY_MAX {
            let mut cut = URL_BODY_MAX;
            while !body.is_char_boundary(cut) {
                cut -= 1;
            }
            body.truncate(cut);
        }
        let title = telemetry::scrub(&format!("Error: {}", self.title));
        format!("{ISSUES}?title={}&body={}", encode(&title), encode(&body))
    }
}

/// Percent-encode a query value.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len() * 3);
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Open the dialog for one error.
pub fn open(st: &mut State, title: &str, detail: &str) {
    let f = &mut st.notices.feedback;
    f.open = true;
    f.title = title.trim().to_string();
    f.detail = detail.trim().to_string();
    f.page = tr(st.screen.title()).to_string();
    f.note = TextEdit::new("");
    f.code = None;
    f.log = if rune::is_headless() || cfg!(test) {
        String::new()
    } else {
        telemetry::scrub(&crate::screens::settings::log::tail(LOG_LINES))
    };
}

fn close(st: &mut State) {
    st.notices.feedback.open = false;
}

fn send(st: &mut State) {
    let f = &st.notices.feedback;
    if f.sending.is_running() || st.demo || rune::is_headless() || st.prefs.offline {
        return;
    }
    let summary = f.summary(st.game_detected);
    let log = f.log.clone();
    crate::screens::settings::telemetry::event(st, "diagnostics:send", None);
    let wake = st.wake.clone();
    st.notices.feedback.code = None;
    st.notices.feedback.sending.start(wake, move || {
        telemetry::send_diagnostics(&crate::data::data_dir(), VERSION, &summary, &log)
            .map_err(|e| e.to_string())
    });
}

/// A small Send feedback button for a toast, an inline error box or a failed step.
pub fn button(u: &Ui, key: &str, title: &str, detail: &str) -> El {
    let (title, detail) = (title.to_string(), detail.to_string());
    button_look(u, key).on_press(move |st: &mut State| open(st, &title, &detail))
}

/// The button without a handler, for a caller that does more on the press.
pub fn button_look(u: &Ui, key: &str) -> El {
    let label = tr("Send feedback");
    row()
        .key(key.to_string())
        .h(24.0)
        .px(8.0)
        .gap(6.0)
        .center_y()
        .radius(crate::ui::r(6.0))
        .hover(u.hover)
        .press(u.over.alpha(0.08))
        .focusable(true)
        .role("button")
        .label(label)
        .cursor(rune::Cursor::Pointer)
        .child(rune::icon(&icons::MESSAGE_SQUARE, 13.0).color(u.muted).a11y_hidden())
        .child(text(label.to_string()).font(BODY_MED).color(u.ink).a11y_hidden())
}

/// The shared inline error box: the message and Send feedback. Use it wherever a page shows an
/// error in place, so every error gets the same way to report it.
pub fn error_box(u: &Ui, key: &str, message: &str) -> El {
    row()
        .key(key.to_string())
        .w_full()
        .gap(10.0)
        .center_y()
        .pl(12.0)
        .pr(6.0)
        .py(8.0)
        .radius(crate::ui::r(8.0))
        .fill(u.rose.alpha(0.08))
        .border(1.0, u.rose.alpha(0.35))
        .role("alert")
        .label(message.to_string())
        .child(
            col()
                .size(16.0, 16.0)
                .center()
                .child(rune::icon(&icons::CIRCLE_X, 15.0).color(u.rose).a11y_hidden()),
        )
        .child(
            text(message.to_string())
                .font(BODY)
                .color(u.ink)
                .wrap()
                .grow(1.0)
                .shrink()
                .min_w(0.0)
                .a11y_hidden(),
        )
        .child(button(u, &format!("{key}-feedback"), message, ""))
}

/// The dialog, drawn over everything with the toasts (`notify::region`).
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    let f = &s.notices.feedback;
    if !f.open {
        return None;
    }
    let sending = f.sending.is_running();
    let details = f.details(s.game_detected);
    let shown = if f.log.trim().is_empty() {
        details.clone()
    } else {
        format!("{details}\n\n{}\n{}", tr("Log, last lines:"), f.log)
    };
    let status = match &f.code {
        Some(Ok(code)) => Some((fill(tr("Sent. Your reference code is {}."), &[code]), u.teal)),
        Some(Err(error)) => Some((fill(tr("Not sent: {}"), &[error]), u.rose)),
        None if s.prefs.offline => Some((tr("Offline mode is on, so Send is off. Copy the details or open a GitHub issue instead.").to_string(), u.muted)),
        None => None,
    };
    let field = rune::text_input(&f.note, BODY)
        .key("feedback-note")
        .label(tr("What were you doing?"))
        .w_full()
        .min_h(64.0)
        .padding(10.0)
        .radius(crate::ui::r(8.0))
        .fill(u.well)
        .border(1.0, u.hair)
        .color(u.ink)
        .placeholder(tr("Optional: what were you doing when it failed?"), u.faint)
        .caret(u.accent, u.accent.alpha(0.20))
        .on_edit(rune::edit_access(|st: &mut State| &mut st.notices.feedback.note));
    let copy_text = {
        let note = f.note.text.trim();
        if note.is_empty() {
            details.clone()
        } else {
            format!("{}\n\n{details}", telemetry::scrub(note))
        }
    };
    let url = f.issue_url(s.game_detected);
    let mut actions = row().w_full().gap(8.0).center_y();
    actions = actions
        .child(
            u.action(tr("Copy details"), ActionKind::Secondary)
                .key("feedback-copy")
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |_st: &mut State, tx: &mut Tx| tx.copy(copy_text.clone())),
        )
        .child(
            u.action(tr("Open a GitHub issue"), ActionKind::Ghost)
                .key("feedback-github")
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                    crate::screens::settings::tabs::open_url(st, tx, &url)
                }),
        )
        .child(spacer());
    if let Some(Ok(code)) = &f.code {
        let code = code.clone();
        actions = actions
            .child(
                u.action(tr("Copy code"), ActionKind::Secondary)
                    .key("feedback-copy-code")
                    .cursor(rune::Cursor::Pointer)
                    .on_press_tx(move |_st: &mut State, tx: &mut Tx| tx.copy(code.clone())),
            )
            .child(
                u.action(tr("Close"), ActionKind::Primary)
                    .key("feedback-close")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(close),
            );
    } else {
        actions = actions
            .child(
                u.action(tr("Cancel"), ActionKind::Ghost)
                    .key("feedback-cancel")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(close),
            )
            .child(
                u.action(if sending { tr("Sending") } else { tr("Send") }, ActionKind::Primary)
                    .key("feedback-send")
                    .disabled(sending || s.prefs.offline || s.demo)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(send),
            );
    }
    let mut body = u
        .dialog_shell()
        .key("feedback")
        .role("dialog")
        .label(tr("Send feedback"))
        .w(620.0)
        .padding(24.0)
        .gap(14.0)
        .modal(true, close)
        .child(text(tr("Send feedback")).font(H1).color(u.ink))
        .child(
            text(tr("This is everything that goes to Sigil's server, after paths, names, keys and emails are scrubbed. You get a reference code to give to whoever helps you."))
                .font(BODY)
                .color(u.muted)
                .wrap(),
        )
        .child(
            col()
                .key("feedback-text")
                .w_full()
                .h(200.0)
                .padding(12.0)
                .radius(crate::ui::r(8.0))
                .fill(u.well)
                .border(1.0, u.hair)
                .scroll(rune::ScrollSpec { stick_bottom: false })
                .child(text(shown).font(MONO).color(u.prose).wrap_anywhere().selectable(true).w_full()),
        )
        .child(text(tr("What were you doing?")).font(BODY_MED).color(u.ink))
        .child(field);
    if let Some((line, color)) = status {
        body = body.child(text(line).font(SMALL).color(color).wrap().selectable(true));
    }
    Some(body.child(actions))
}

/// `w4a-feedback`: the dialog for a failed install, with a note typed.
pub fn stage_scene(st: &mut State, sent: bool) {
    open(
        st,
        "Could not write autoexec.cfg",
        "Permission denied. Close the game and try again.",
    );
    st.notices.feedback.note = TextEdit::new("Saving a crosshair right after a game update");
    if sent {
        st.notices.feedback.code = Some(Ok("SGL-7K2Q-94XD".to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_issue_link_is_encoded_and_carries_the_details() {
        let mut f = Feedback {
            title: "Could not install X & Y".to_string(),
            detail: "Disk full".to_string(),
            page: "Browse".to_string(),
            ..Feedback::default()
        };
        f.note = TextEdit::new("clicked install");
        let url = f.issue_url(true);
        assert!(url.starts_with(ISSUES));
        assert!(url.contains("title=Error%3A%20Could%20not%20install%20X%20%26%20Y"));
        assert!(url.contains("clicked%20install"));
        assert!(url.contains("Disk%20full"));
        assert!(!url.contains(' '));
        let summary = f.summary(true);
        assert!(summary.starts_with("Note: clicked install"));
        assert!(summary.contains("Page: Browse"));
    }

    #[test]
    fn a_long_body_is_cut_for_the_link() {
        let f = Feedback {
            title: "Broke".to_string(),
            detail: "word ".repeat(4000),
            ..Feedback::default()
        };
        assert!(f.issue_url(false).len() < 3 * URL_BODY_MAX + 400);
    }
}
