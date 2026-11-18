//! Downloads: the queue behind Browse's install dialog.
//!
//! The queue itself lives in `deadlock_core::download_queue`, on its own worker thread. This
//! module draws it on the Launch page: one row per archive with its progress, the controls that
//! fit its state, and the look inside the archive before it reaches the install dialog.
//!
//! Running rows show their smoothed speed and the time left (`Item::speed_bps`,
//! `Item::eta_secs`), and a crawl under 150 KB/s for 8 seconds says so (`Item::is_slow`,
//! Deadlock Mod Manager's threshold with Grimoire's gap rule). Rows still waiting can be moved up and down or dragged; the queue's
//! `move_to` refuses a row that started. The titlebar shows [`indicator`] on every page while
//! anything downloads, and its press opens this list in a panel ([`panel`]).
//!
//! Nothing here writes to the game folder. A finished row hands its path to Browse's install
//! dialog, which still plans, validates and asks before anything lands in `citadel/addons`.

use std::path::PathBuf;

use deadlock_core::download_queue::{self as queue, Item, State as ItemState};
use rune::{Align, Color, El, Side, TextEdit, col, el, icon, row, spacer, text};

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
    /// The titlebar panel with the list is open.
    pub panel: bool,
    /// A waiting row being dragged: its id, where it started and where it would land, as
    /// positions in the queue's list.
    pub drag: Option<(String, usize, usize)>,
    /// Fixture speeds for the scenes, by row id: bytes a second and whether it crawls. The
    /// worker's own rows read their speed from the queue.
    pub demo_rates: std::collections::HashMap<String, (u64, bool)>,
    /// Rows the slow-download toast already spoke for. Each download gets one, ever.
    pub slow_told: std::collections::HashSet<String>,
}

impl Default for Downloads {
    fn default() -> Self {
        Downloads {
            handle: None,
            input: TextEdit::new(""),
            dialog: Dialog::None,
            demo: None,
            panel: false,
            drag: None,
            demo_rates: std::collections::HashMap::new(),
            slow_told: std::collections::HashSet::new(),
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
        add("d7", "kelvin ice pack", Q::Queued, 0, 0, false);
        add("d4", "old crosshair", Q::Failed, 900_000, 2_400_000, false);
        add("d5", "masked skin pack", Q::Done, 8_100_000, 8_100_000, true);
        add("d6", "extra hud", Q::Done, 2_200_000, 2_200_000, false);
        if let Some(item) = stage.get_mut("d4") {
            item.error = Some("The server closed the connection.".to_string());
        }
        self.demo = Some(stage);
        self.demo_rates.insert("d1".to_string(), (2_300_000, false));
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
        queue::Command::MoveTo { id, index } => {
            let _ = queue.move_to(&id, index);
        }
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
    let drag = downloads.drag.as_ref();
    for (index, item) in queue.items.iter().enumerate() {
        let target = drag.filter(|(id, from, to)| *id != item.id && *to == index && from != to);
        let speed = rate(&downloads.demo_rates, item);
        body = body.child(item_row(u, &queue, index, item, speed, drag.is_some_and(|(id, _, _)| *id == item.id), target.map(|(_, from, to)| from < to)));
    }
    body
}

// --- reorder ------------------------------------------------------------------------------------

/// The row height the drag math steps by.
const ROW_H: f32 = 56.0;

/// Where a waiting row lands: the position `steps` rows from `from` in the list, clamped to the
/// rows still waiting (a waiting row never jumps above the running one or below a finished one).
pub fn landing(items: &[Item], from: usize, steps: isize) -> usize {
    let waiting: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.state == ItemState::Queued)
        .map(|(at, _)| at)
        .collect();
    let Some(me) = waiting.iter().position(|at| *at == from) else {
        return from;
    };
    let wanted = (me as isize + steps).clamp(0, waiting.len() as isize - 1) as usize;
    waiting[wanted]
}

fn drag_row(st: &mut State, id: &str, from: usize, d: &mut rune::Drag) {
    match d.phase {
        rune::Phase::Down => st.launch.downloads.drag = Some((id.to_string(), from, from)),
        rune::Phase::Move => {
            let items = st.launch.downloads.snapshot().items;
            let steps = (d.dy / ROW_H).round() as isize;
            st.launch.downloads.drag = Some((id.to_string(), from, landing(&items, from, steps)));
        }
        rune::Phase::Up => {
            if let Some((id, from, to)) = st.launch.downloads.drag.take()
                && !d.cancelled()
                && from != to
            {
                st.launch.downloads.send(queue::Command::MoveTo { id, index: to });
            }
        }
    }
}

// --- speed ----------------------------------------------------------------------------------------

/// Seconds left as words: "40 s left", "3 min left", "1 h 5 min left".
pub fn eta_label(secs: u64) -> String {
    if secs < 60 {
        fill(tr("{} s left"), &[&secs.max(1).to_string()])
    } else if secs < 3600 {
        fill(tr("{} min left"), &[&secs.div_ceil(60).to_string()])
    } else {
        let minutes = secs.div_ceil(60);
        fill(tr("{} h {} min left"), &[&(minutes / 60).to_string(), &(minutes % 60).to_string()])
    }
}

/// A row's speed, time left and crawl flag: the queue's numbers, or a scene's fixture.
pub fn rate(rates: &std::collections::HashMap<String, (u64, bool)>, item: &Item) -> (Option<u64>, Option<u64>, bool) {
    if item.state == ItemState::Running
        && let Some((bps, slow)) = rates.get(&item.id)
    {
        let left = item.bytes_total.saturating_sub(item.bytes_done);
        let eta = (item.bytes_total > 0 && left > 0 && *bps > 0).then(|| left.div_ceil(*bps));
        return (Some(*bps), eta, *slow);
    }
    (item.speed_bps(), item.eta_secs(), item.is_slow())
}

// --- the slow-download toast ------------------------------------------------------------------

/// Rows that crawl now and have not had their toast: Grimoire's notice, once per download.
pub fn newly_slow<'a>(
    items: &'a [Item],
    rates: &std::collections::HashMap<String, (u64, bool)>,
    told: &std::collections::HashSet<String>,
) -> Vec<&'a Item> {
    items
        .iter()
        .filter(|item| item.state == ItemState::Running && !told.contains(&item.id))
        .filter(|item| rate(rates, item).2)
        .collect()
}

/// The first time a download crawls, one warning toast says why, with a way to the list. It is
/// never repeated for that download; the row's own warning in the list stays while it crawls.
pub fn poll(st: &mut State) -> bool {
    let downloads = &st.launch.downloads;
    if downloads.handle.is_none() && downloads.demo.is_none() {
        return false;
    }
    let queue = downloads.snapshot();
    let fresh: Vec<(String, String)> = newly_slow(&queue.items, &downloads.demo_rates, &downloads.slow_told)
        .into_iter()
        .map(|item| (item.id.clone(), item.name.clone()))
        .collect();
    if fresh.is_empty() {
        return false;
    }
    for (id, name) in fresh {
        st.launch.downloads.slow_told.insert(id);
        let title = if name.trim().is_empty() {
            tr("A download is running slowly").to_string()
        } else {
            fill(tr("{} is downloading slowly"), &[&name])
        };
        crate::notify::push(
            st,
            crate::notify::Notice::warning(title)
                .detail(tr("This is most likely GameBanana's servers under heavy load or your internet provider. The download keeps going."))
                .action(tr("Show downloads"), |st: &mut State| st.launch.downloads.panel = true),
        );
    }
    true
}

/// The running line: bytes, speed and time left when they are known.
fn running_detail(item: &Item, speed: Option<u64>, eta: Option<u64>) -> String {
    let mut parts = vec![item.detail()];
    if let Some(bps) = speed {
        parts.push(format!("{}/s", queue::size(bps)));
    }
    if let Some(eta) = eta {
        parts.push(eta_label(eta));
    }
    parts.join(" · ")
}

// --- the titlebar indicator -----------------------------------------------------------------------

/// What the indicator says: rows downloading or waiting, the share done when sizes are known,
/// and whether any of them crawls.
pub fn summary(queue: &queue::Queue, rates: &std::collections::HashMap<String, (u64, bool)>) -> Option<(usize, Option<f32>, bool)> {
    let moving: Vec<&Item> = queue
        .items
        .iter()
        .filter(|item| matches!(item.state, ItemState::Queued | ItemState::Running | ItemState::Paused))
        .collect();
    if moving.is_empty() {
        return None;
    }
    let total: u64 = moving.iter().map(|item| item.bytes_total).sum();
    let done: u64 = moving.iter().map(|item| item.bytes_done.min(item.bytes_total)).sum();
    let share = (total > 0 && moving.iter().all(|item| item.bytes_total > 0)).then(|| done as f32 / total as f32);
    let slow = moving.iter().any(|item| rate(rates, item).2);
    Some((moving.len(), share, slow))
}

/// The small button in the titlebar while anything downloads. Shown on every page.
pub fn indicator(s: &State, u: &Ui) -> Option<El> {
    let (count, share, slow) = summary(&s.launch.downloads.snapshot(), &s.launch.downloads.demo_rates)?;
    let words = match share {
        Some(share) => format!("{}%", (share.clamp(0.0, 1.0) * 100.0).floor() as u32),
        None => count.to_string(),
    };
    let label = fill(if count == 1 { tr("{} download, open the list") } else { tr("{} downloads, open the list") }, &[&count.to_string()]);
    let open = s.launch.downloads.panel;
    let tint = if slow { u.amber } else { u.accent };
    let mut b = row()
        .key("titlebar-downloads")
        .h(28.0)
        .pl(8.0)
        .pr(10.0)
        .gap(6.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .clip_rounded(crate::ui::r(8.0))
        .fill(if open { u.selected } else { tint.alpha(0.08) })
        .border(1.0, tint.alpha(0.35))
        .hover(u.hover)
        .press(u.over.alpha(0.08))
        .focusable(true)
        .role("button")
        .label(label)
        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| st.launch.downloads.panel = !st.launch.downloads.panel);
    if let Some(share) = share {
        let f = share.clamp(0.0, 1.0);
        b = b.child(
            row()
                .absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .child(el().h_full().grow(f).fill(tint.alpha(0.16)))
                .child(el().h_full().grow(1.0 - f))
                .a11y_hidden(),
        );
    }
    b = b
        .child(icon(if slow { &icons::TRIANGLE_ALERT } else { &icons::DOWNLOAD }, 14.0).color(tint).a11y_hidden())
        .child(text(words).font(SMALL).color(u.ink).a11y_hidden());
    Some(b)
}

/// The list under the indicator, over any page.
pub fn panel(s: &State, u: &Ui) -> Option<El> {
    if !s.launch.downloads.panel {
        return None;
    }
    let close = |st: &mut State| st.launch.downloads.panel = false;
    Some(
        col()
            .key("downloads-panel")
            .role("dialog")
            .label(tr("Downloads"))
            .w(560.0)
            .shrink()
            .min_h(0.0)
            .fill(u.raised)
            .radius(crate::ui::r(12.0))
            .border(1.0, u.hair_strong)
            .shadow(16.0, 40.0, Color::BLACK.alpha(0.5))
            .anchored_fit("titlebar-downloads", Side::Bottom, Align::End, 6.0)
            .modal(true, close)
            .on_key(|st: &mut State, ev: &rune::KeyEvent, _| {
                if ev.key == rune::Key::Escape {
                    st.launch.downloads.panel = false;
                    true
                } else {
                    false
                }
            })
            .child(
                col()
                    .key("downloads-panel-scroll")
                    .w_full()
                    .shrink()
                    .min_h(0.0)
                    .max_h(520.0)
                    .px(16.0)
                    .pt(8.0)
                    .pb(12.0)
                    .scroll(rune::ScrollSpec { stick_bottom: false })
                    .child(section(s, u)),
            ),
    )
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
fn item_row(
    u: &Ui,
    queue: &queue::Queue,
    index: usize,
    item: &Item,
    (speed, eta, slow): (Option<u64>, Option<u64>, bool),
    dragging: bool,
    drop_below: Option<bool>,
) -> El {
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
    let detail = if item.state == ItemState::Running {
        running_detail(item, speed, eta)
    } else if item.state == ItemState::Done && item.scan.suspicious() {
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
    if slow {
        middle = middle.child(
            row()
                .key(format!("downloads-slow-{}", item.id))
                .w_full()
                .gap(6.0)
                .items_start()
                .child(icon(&icons::TRIANGLE_ALERT, 12.0).color(u.amber).mt(2.0).a11y_hidden())
                .child(
                    text(tr("Downloading slowly, under 150 KB/s. This is most likely GameBanana's servers under heavy load or your internet provider."))
                        .font(SMALL)
                        .color(u.amber)
                        .wrap()
                        .grow(1.0)
                        .min_w(0.0),
                ),
        );
    }

    let mut controls = row().gap(4.0).center_y();
    if item.state == ItemState::Queued {
        for (up, glyph, words) in [(true, &icons::ARROW_UP, tr("Move up")), (false, &ARROW_DOWN, tr("Move down"))] {
            let to = landing(&queue.items, index, if up { -1 } else { 1 });
            let mv = id.clone();
            let mut b = u
                .icon_button(glyph, words)
                .key(format!("downloads-{}-{}", if up { "up" } else { "down" }, item.id));
            b = if to == index {
                b.disabled(true).opacity(0.35)
            } else {
                b.cursor(rune::Cursor::Pointer).on_press(move |st: &mut State| {
                    st.launch.downloads.send(queue::Command::MoveTo { id: mv.clone(), index: to })
                })
            };
            controls = controls.child(b);
        }
    }
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

    // A waiting row's glyph is its drag handle.
    let lead = if item.state == ItemState::Queued {
        let drag_id = item.id.clone();
        el().key(format!("downloads-grip-{}", item.id))
            .w(16.0)
            .h(24.0)
            .center()
            .radius(4.0)
            .hover(u.hover)
            .label(tr("Drag to reorder"))
            .cursor(if dragging { rune::Cursor::Grabbing } else { rune::Cursor::Grab })
            .on_drag(move |st: &mut State, d: &mut rune::Drag, _tx: &mut rune::Tx| drag_row(st, &drag_id, index, d))
            .child(icon(&GRIP_VERTICAL, 14.0).color(u.faint).a11y_hidden())
    } else {
        el().w(16.0).child(icon(glyph, 16.0).color(tint).a11y_hidden())
    };
    let mut r = row()
        .key(format!("downloads-row-{}", item.id))
        .w_full()
        .min_h(52.0)
        .py(8.0)
        .gap(12.0)
        .center_y()
        .fill(if dragging { u.selected } else { Color::TRANSPARENT })
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(lead)
        .child(middle)
        .child(controls);
    if let Some(below) = drop_below {
        let line = el().w_full().h(2.0).fill(u.accent).a11y_hidden();
        r = r.child(
            col()
                .absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .a11y_hidden()
                .child(if below { spacer() } else { line.clone() })
                .child(if below { line } else { spacer() }),
        );
    }
    r
}

/// Lucide grip-vertical and arrow-down (ISC), drawn like `icons.rs`.
static GRIP_VERTICAL: rune::Icon = rune::Icon {
    name: "sigil-downloads-grip-vertical",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><circle cx="9" cy="12" r="1"/><circle cx="9" cy="5" r="1"/><circle cx="9" cy="19" r="1"/><circle cx="15" cy="12" r="1"/><circle cx="15" cy="5" r="1"/><circle cx="15" cy="19" r="1"/></g>"#,
};
static ARROW_DOWN: rune::Icon = rune::Icon {
    name: "sigil-downloads-arrow-down",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M12 5v14"/><path d="m19 12-7 7-7-7"/></g>"#,
};

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

    fn row_in(id: &str, state: ItemState) -> Item {
        let mut item = Item::new(id.to_string(), format!("https://gamebanana.com/mods/{id}"));
        item.state = state;
        item
    }

    #[test]
    fn a_waiting_row_lands_among_the_waiting_rows() {
        let items = vec![
            row_in("a", ItemState::Running),
            row_in("b", ItemState::Queued),
            row_in("c", ItemState::Queued),
            row_in("d", ItemState::Done),
            row_in("e", ItemState::Queued),
        ];
        assert_eq!(landing(&items, 1, 1), 2, "one down is the next waiting row");
        assert_eq!(landing(&items, 2, 1), 4, "the finished row in between is skipped");
        assert_eq!(landing(&items, 1, -3), 1, "never above the running row");
        assert_eq!(landing(&items, 4, 9), 4, "clamped at the last waiting row");
        assert_eq!(landing(&items, 0, 1), 0, "a running row does not move");
    }

    #[test]
    fn a_crawl_is_told_once_per_download() {
        let items = vec![row_in("a", ItemState::Running), row_in("b", ItemState::Running), row_in("c", ItemState::Queued)];
        let mut rates = std::collections::HashMap::new();
        rates.insert("a".to_string(), (90_000, true));
        rates.insert("b".to_string(), (4_000_000, false));
        rates.insert("c".to_string(), (90_000, true));
        let mut told = std::collections::HashSet::new();
        let ids = |rows: Vec<&Item>| rows.iter().map(|item| item.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(newly_slow(&items, &rates, &told)), vec!["a"], "only the running crawl; b is fast, c waits");
        told.insert("a".to_string());
        assert!(newly_slow(&items, &rates, &told).is_empty(), "never twice for one download");
        rates.insert("b".to_string(), (80_000, true));
        assert_eq!(ids(newly_slow(&items, &rates, &told)), vec!["b"], "another download gets its own");
    }

    #[test]
    fn time_left_reads_as_words() {
        assert_eq!(eta_label(0), "1 s left");
        assert_eq!(eta_label(42), "42 s left");
        assert_eq!(eta_label(61), "2 min left");
        assert_eq!(eta_label(3_900), "1 h 5 min left");
    }

    #[test]
    fn the_indicator_counts_moving_rows_only() {
        let mut q = queue::Queue::default();
        assert!(summary(&q, &std::collections::HashMap::new()).is_none());
        let mut a = row_in("a", ItemState::Running);
        a.bytes_done = 25;
        a.bytes_total = 100;
        let mut b = row_in("b", ItemState::Queued);
        b.bytes_total = 100;
        q.items = vec![a, b, row_in("c", ItemState::Done)];
        let none = std::collections::HashMap::new();
        let (count, share, slow) = summary(&q, &none).unwrap();
        assert_eq!(count, 2);
        assert_eq!(share, Some(0.125));
        assert!(!slow);
        q.items.push(row_in("d", ItemState::Queued));
        assert_eq!(summary(&q, &none).unwrap().1, None, "an unknown size hides the share");
    }

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
