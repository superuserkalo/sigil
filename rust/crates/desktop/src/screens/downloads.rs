//! Downloads: the queue behind Browse's install dialog.
//!
//! The queue itself lives in `deadlock_core::download_queue`, on its own worker thread. This
//! module draws it on the Launch page: one row per archive with its progress, the controls that
//! fit its state, and the look inside the archive before it reaches the install dialog.
//!
//! Nothing here writes to the game folder. A finished row hands its path to Browse's install
//! dialog, which still plans, validates and asks before anything lands in `citadel/addons`.

use std::path::PathBuf;

use deadlock_core::download_queue::{self as queue, Item, State as ItemState};
use rune::{El, TextEdit, col, el, icon, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, MONO, SMALL, Ui};
use crate::screens::settings::i18n::{fill, tr};

/// The dialog this module can open. Mounted through `launch::dialog`.
#[derive(Clone, PartialEq, Eq)]
pub enum Dialog {
    None,
    /// The look inside one finished archive.
    Review(String),
}

/// The queue surface, kept in `State` with the Launch screen.
pub struct Downloads {
    pub handle: Option<queue::Handle>,
    pub input: TextEdit,
    pub dialog: Dialog,
    /// Fixture rows for the scenes and the pictures. A real queue never uses this.
    pub demo: Option<queue::Queue>,
}

impl Default for Downloads {
    fn default() -> Self {
        Downloads {
            handle: None,
            input: TextEdit::new(""),
            dialog: Dialog::None,
            demo: None,
        }
    }
}

impl Downloads {
    /// Start the worker once. The queue file is read on the worker thread.
    pub fn ensure(&mut self, data_dir: PathBuf, workspace: PathBuf, wake: Option<rune::Waker>) {
        if self.handle.is_some() {
            return;
        }
        let wake: Option<queue::Wake> = wake.map(|wake| {
            let wake = wake.clone();
            std::sync::Arc::new(move || wake()) as queue::Wake
        });
        self.handle = Some(queue::Handle::start(data_dir, workspace, wake));
    }

    pub fn snapshot(&self) -> queue::Queue {
        if let Some(demo) = &self.demo {
            return demo.clone();
        }
        self.handle
            .as_ref()
            .map(queue::Handle::snapshot)
            .unwrap_or_default()
    }

    /// Take what is in the field and clear it. None when there is nothing to queue.
    pub fn take_input(&mut self) -> Option<String> {
        let target = self.input.text.trim().to_string();
        if target.is_empty() {
            return None;
        }
        self.input = TextEdit::new("");
        Some(target)
    }

    pub fn send(&mut self, command: queue::Command) {
        if let Some(demo) = &mut self.demo {
            demo_apply(demo, command);
            return;
        }
        match &self.handle {
            Some(handle) => handle.send(command),
            None => {
                // The worker has not started yet: start it, then send.
                let data = crate::data::data_dir();
                let workspace = crate::data::workspace_dir();
                self.ensure(data, workspace, None);
                if let Some(handle) = &self.handle {
                    handle.send(command);
                }
            }
        }
    }

    /// A fixture queue with one row of every state.
    pub fn stage_demo(&mut self) {
        use queue::State as Q;
        let mut stage = queue::Queue::default();
        let mut add = |id: &str, name: &str, state: Q, done: u64, total: u64, warnings: bool| {
            let mut item = queue::Item::new(id.to_string(), format!("https://gamebanana.com/mods/{id}"));
            item.name = name.to_string();
            item.state = state;
            item.bytes_done = done;
            item.bytes_total = total;
            item.file_name = "archive.zip".to_string();
            item.server = Some("filecache9".to_string());
            if warnings {
                item.scan = queue::scan_entries(&["citadel/installer.exe".to_string()], 4096);
            }
            stage.items.push(item);
        };
        add("d1", "billy recolor", Q::Running, 4_400_000, 11_000_000, false);
        add("d2", "hud pack", Q::Paused, 1_200_000, 3_000_000, false);
        add("d3", "yoshi sounds", Q::Queued, 0, 0, false);
        add("d4", "old crosshair", Q::Failed, 900_000, 2_400_000, false);
        add("d5", "masked skin pack", Q::Done, 8_100_000, 8_100_000, true);
        add("d6", "extra hud", Q::Done, 2_200_000, 2_200_000, false);
        if let Some(item) = stage.get_mut("d4") {
            item.error = Some("The server closed the connection.".to_string());
        }
        self.demo = Some(stage);
    }
}

/// The same state changes the worker makes, for fixture rows.
fn demo_apply(queue: &mut queue::Queue, command: queue::Command) {
    match command {
        queue::Command::Add(target) => {
            let _ = queue.add(&target);
        }
        queue::Command::Retry(id) => {
            queue.retry(&id);
        }
        queue::Command::Pause(id) => {
            queue.pause(&id);
        }
        queue::Command::Resume(id) => {
            queue.resume(&id);
        }
        queue::Command::Cancel(id) => {
            queue.cancel(&id);
        }
        queue::Command::Remove(id) => {
            queue.remove(&id);
        }
        queue::Command::ClearFinished => {
            queue.clear_finished();
        }
        queue::Command::PauseAll(paused) => queue.paused = paused,
        queue::Command::Shutdown => {}
    }
}

/// The section on the Launch page: heading, the add row, then one row per archive.
pub fn section(s: &State, u: &Ui) -> El {
    let downloads = &s.launch.downloads;
    let queue = downloads.snapshot();
    let active = queue.active();
    let done = queue.done();
    let finished = queue.items.iter().filter(|item| item.state.finished()).count();

    let meta = if queue.items.is_empty() {
        String::new()
    } else if active == 0 {
        fill(
            tr("{} downloaded"),
            &[&fill(tr("{} of {}"), &[&done.to_string(), &queue.items.len().to_string()])],
        )
    } else if queue.total_bytes() > 0 {
        format!(
            "{} · {} of {}",
            fill(tr("{} in the queue"), &[&active.to_string()]),
            queue::size(queue.done_bytes()),
            queue::size(queue.total_bytes())
        )
    } else {
        fill(tr("{} in the queue"), &[&active.to_string()])
    };

    let mut head = row()
        .w_full()
        .h(32.0)
        .gap(8.0)
        .center_y()
        .child(text(tr("Downloads")).font(BODY_MED).color(u.muted));
    if !meta.is_empty() {
        head = head.child(text(meta).font(SMALL).color(u.faint).truncate());
    }
    head = head.child(spacer());
    if !queue.items.is_empty() {
        let (label, command) = if queue.paused {
            (tr("Resume all"), queue::Command::PauseAll(false))
        } else {
            (tr("Pause all"), queue::Command::PauseAll(true))
        };
        head = head.child(
            u.action(label, ActionKind::Ghost)
                .key("downloads-pause-all")
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| st.launch.downloads.send(command.clone())),
        );
    }
    if finished > 0 {
        head = head.child(
            u.action(tr("Clear finished"), ActionKind::Ghost)
                .key("downloads-clear")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| {
                    st.launch.downloads.send(queue::Command::ClearFinished)
                }),
        );
    }

    let mut body = col().w_full().gap(0.0).child(head).child(add_row(s, u));
    if queue.items.is_empty() {
        body = body.child(
            text(tr(
                "Nothing queued. Paste a GameBanana link and the archive downloads in the background.",
            ))
            .font(SMALL)
            .color(u.faint)
            .w_full()
            .pt(12.0)
            .wrap(),
        );
        return body;
    }
    for item in &queue.items {
        body = body.child(item_row(u, item));
    }
    body
}

/// The field and the Add button: a queued download starts without leaving the page.
fn add_row(s: &State, u: &Ui) -> El {
    let field = rune::text_input(&s.launch.downloads.input, SMALL)
        .key("downloads-input")
        .label(tr("A mod link, a mod id or an archive path"))
        .single_line()
        .grow(1.0)
        .color(u.ink)
        .placeholder(tr("Paste a GameBanana link or mod id"), u.faint)
        .caret(u.accent, u.accent.alpha(0.20))
        .on_edit(|st: &mut State| &mut st.launch.downloads.input)
        .on_submit(|st: &mut State, _tx: &mut rune::Tx| queue_target(st));
    row()
        .key("downloads-add")
        .w_full()
        .min_h(44.0)
        .py(8.0)
        .gap(8.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(
            el()
                .grow(1.0)
                .min_w(0.0)
                .h(32.0)
                .px(10.0)
                .center_y()
                .radius(crate::ui::r(8.0))
                .fill(u.well)
                .border(1.0, u.hair)
                .child(field),
        )
        .child(
            u.icon_action(&icons::PLUS, tr("Add"), ActionKind::Secondary)
                .key("downloads-add-run")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| queue_target(st)),
        )
}

/// Queue the field. The worker decides whether the target is usable; a bad one comes back as a
/// failed row that says why.
fn queue_target(st: &mut State) {
    let Some(target) = st.launch.downloads.take_input() else {
        crate::notify::push(
            st,
            crate::notify::Notice::warning(tr("Paste a mod link or a mod id first.")),
        );
        return;
    };
    crate::data::queue_install(st, &target);
}

/// One archive: state, name, the line under it, a progress line while it runs, and its
/// controls on the right.
fn item_row(u: &Ui, item: &Item) -> El {
    let (glyph, tint) = match item.state {
        ItemState::Queued => (&icons::DOWNLOAD, u.faint),
        ItemState::Running => (&icons::DOWNLOAD, u.ink),
        ItemState::Paused => (&icons::DOWNLOAD, u.muted),
        ItemState::Done if item.scan.suspicious() => (&icons::TRIANGLE_ALERT, u.amber),
        ItemState::Done => (&icons::CHECK, u.muted),
        ItemState::Failed => (&icons::TRIANGLE_ALERT, u.rose),
        ItemState::Cancelled => (&icons::X, u.faint),
    };
    let id = item.id.clone();
    let name = if item.name.is_empty() {
        item.target.clone()
    } else {
        item.name.clone()
    };
    let detail = if item.state == ItemState::Done && item.scan.suspicious() {
        let count = item.scan.warnings.len();
        fill(
            if count == 1 {
                tr("{} · {} thing to look at")
            } else {
                tr("{} · {} things to look at")
            },
            &[&item.detail(), &count.to_string()],
        )
    } else {
        item.detail()
    };

    let mut middle = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(2.0)
        .child(
            text(name)
                .font(BODY_MED)
                .color(if item.state == ItemState::Cancelled {
                    u.faint
                } else {
                    u.ink
                })
                .truncate(),
        )
        .child(
            text(detail)
                .font(SMALL)
                .color(if item.state == ItemState::Failed {
                    u.rose
                } else {
                    u.muted
                })
                .truncate(),
        );
    if matches!(item.state, ItemState::Running | ItemState::Paused) {
        middle = middle.child(progress_line(u, item));
    }

    let mut controls = row().gap(4.0).center_y();
    match item.state {
        ItemState::Queued | ItemState::Running => {
            let pause = id.clone();
            controls = controls.child(
                u.icon_button(&icons::SQUARE_STOP, tr("Pause"))
                    .key(format!("downloads-pause-{}", item.id))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        st.launch
                            .downloads
                            .send(queue::Command::Pause(pause.clone()))
                    }),
            );
        }
        ItemState::Paused => {
            let resume = id.clone();
            controls = controls.child(
                u.icon_button(&icons::PLAY, tr("Resume"))
                    .key(format!("downloads-resume-{}", item.id))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        st.launch
                            .downloads
                            .send(queue::Command::Resume(resume.clone()))
                    }),
            );
        }
        ItemState::Failed | ItemState::Cancelled | ItemState::Done => {
            let retry = id.clone();
            controls = controls.child(
                u.icon_button(&icons::ROTATE_CCW, tr("Retry"))
                    .key(format!("downloads-retry-{}", item.id))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        st.launch
                            .downloads
                            .send(queue::Command::Retry(retry.clone()))
                    }),
            );
        }
    }
    if item.state == ItemState::Done {
        let path = item.path.clone();
        let review = id.clone();
        controls = controls.child(
            u.icon_action(
                &icons::DOWNLOAD,
                if item.scan.suspicious() {
                    tr("Review")
                } else {
                    tr("Install")
                },
                ActionKind::Secondary,
            )
            .key(format!("downloads-install-{}", item.id))
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| {
                if st.launch.downloads.snapshot().get(&review).is_some_and(|item| item.scan.suspicious())
                {
                    st.launch.downloads.dialog = Dialog::Review(review.clone());
                } else if let Some(path) = path.clone() {
                    install(st, &path, &review);
                }
            }),
        );
    }
    let cancel = id.clone();
    controls = controls.child(
        u.icon_button(
            if item.state.finished() {
                &icons::TRASH_2
            } else {
                &icons::X
            },
            if item.state.finished() {
                tr("Remove from the list")
            } else {
                tr("Cancel")
            },
        )
        .key(format!("downloads-remove-{}", item.id))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            let command = if st
                .launch
                .downloads
                .snapshot()
                .get(&cancel)
                .is_some_and(|item| item.state.finished())
            {
                queue::Command::Remove(cancel.clone())
            } else {
                queue::Command::Cancel(cancel.clone())
            };
            st.launch.downloads.send(command);
        }),
    );

    row()
        .key(format!("downloads-row-{}", item.id))
        .w_full()
        .min_h(52.0)
        .py(8.0)
        .gap(12.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(el().w(16.0).child(icon(glyph, 16.0).color(tint).a11y_hidden()))
        .child(middle)
        .child(controls)
}

/// A 3 point track: the ember part grows with the fraction and the rest stays a hairline.
fn progress_line(u: &Ui, item: &Item) -> El {
    let fraction = item.fraction().unwrap_or(0.0).clamp(0.0, 1.0);
    let done = fraction.max(0.02);
    row()
        .w_full()
        .h(7.0)
        .items_end()
        .child(
            row()
                .w_full()
                .h(3.0)
                .child(
                    el()
                        .h(3.0)
                        .grow(done)
                        .radius(1.5)
                        .fill(if item.state == ItemState::Paused {
                            u.muted
                        } else {
                            u.accent
                        }),
                )
                .child(
                    el()
                        .h(3.0)
                        .grow((1.0 - fraction).max(0.0))
                        .radius(1.5)
                        .fill(u.hair_strong),
                ),
        )
}

/// Hand a finished archive to Browse's install dialog. Nothing is written here.
fn install(st: &mut State, path: &std::path::Path, id: &str) {
    let name = st
        .launch
        .downloads
        .snapshot()
        .get(id)
        .map(|item| item.name.clone())
        .unwrap_or_default();
    let target = path.display().to_string();
    st.install = crate::screens::browse::InstallView {
        open: true,
        target,
        label: name,
        ..crate::screens::browse::InstallView::default()
    };
    st.launch.downloads.dialog = Dialog::None;
}

/// The look inside an archive, before it reaches the install dialog.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    match &s.launch.downloads.dialog {
        Dialog::None => None,
        Dialog::Review(id) => {
            let item = s.launch.downloads.snapshot().get(id).cloned()?;
            Some(review(s, u, item))
        }
    }
}

fn review(_s: &State, u: &Ui, item: Item) -> El {
    let close = |st: &mut State| st.launch.downloads.dialog = Dialog::None;
    let path = item.path.clone();
    let name = if item.name.is_empty() {
        item.target.clone()
    } else {
        item.name.clone()
    };
    let files = if item.scan.files == 1 {
        fill(
            tr("{} file · {}"),
            &[&item.scan.files.to_string(), &queue::size(item.scan.bytes)],
        )
    } else {
        fill(
            tr("{} files · {}"),
            &[&item.scan.files.to_string(), &queue::size(item.scan.bytes)],
        )
    };
    let mut warnings = col().w_full().gap(8.0);
    for warning in item.scan.warnings.iter().take(6) {
        warnings = warnings.child(
            row()
                .w_full()
                .gap(8.0)
                .items_start()
                .child(el().pt(2.0).child(
                    icon(&icons::TRIANGLE_ALERT, 14.0)
                        .color(u.amber)
                        .a11y_hidden(),
                ))
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(text(warning.path.clone()).font(MONO).color(u.ink).truncate())
                        .child(text(warning.message.clone()).font(SMALL).color(u.muted).wrap()),
                ),
        );
    }
    if item.scan.warnings.len() > 6 {
        warnings = warnings.child(
            text(fill(
                tr("{} more."),
                &[(item.scan.warnings.len() - 6).to_string().as_str()],
            ))
            .font(SMALL)
            .color(u.faint),
        );
    }
    let body = if item.scan.checked {
        fill(
            tr("{} holds files a mod should not need. Install it only if you trust where it came from."),
            &[&name],
        )
    } else {
        fill(
            tr("{} could not be checked before install. {}"),
            &[&name, item.scan.summary().as_str()],
        )
    };
    let id = item.id.clone();
    let mut buttons = row().w_full().gap(8.0).child(spacer());
    buttons = buttons.child(
        u.action(tr("Cancel"), ActionKind::Ghost)
            .key("downloads-review-cancel")
            .cursor(rune::Cursor::Pointer)
            .on_press(close),
    );
    buttons = buttons.child(
        u.icon_action(&icons::DOWNLOAD, tr("Install anyway"), ActionKind::Primary)
            .key("downloads-review-install")
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| {
                if let Some(path) = path.clone() {
                    install(st, &path, &id);
                }
            }),
    );
    u.dialog_shell()
        .key("downloads-review")
        .role("dialog")
        .label(tr("Look inside the archive"))
        .w(520.0)
        .padding(24.0)
        .gap(16.0)
        .modal(true, close)
        .on_key(move |st: &mut State, ev: &rune::KeyEvent, _| {
            if ev.key == rune::Key::Escape {
                st.launch.downloads.dialog = Dialog::None;
                true
            } else {
                false
            }
        })
        .child(text(tr("Look inside the archive")).font(H1).color(u.ink))
        .child(text(body).font(BODY).color(u.muted).wrap())
        .child(warnings)
        .child(u.kv(tr("Archive"), &files, u.muted))
        .child(buttons)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_field_is_taken_once_and_cleared() {
        let mut downloads = Downloads::default();
        assert!(downloads.take_input().is_none());
        downloads.input = TextEdit::new("  https://gamebanana.com/mods/715100 ");
        assert_eq!(
            downloads.take_input().as_deref(),
            Some("https://gamebanana.com/mods/715100")
        );
        assert!(downloads.input.text.is_empty());
    }
}
