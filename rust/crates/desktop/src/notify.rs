//! Notifications: short toasts in the bottom-right corner, with an optional action button and
//! an optional Undo. They replace the old one-line status in the titlebar.
//!
//! ```ignore
//! notify::push(st, Notice::success("Turned off slot 03").undo(move |st| { /* put it back */ }));
//! ```
//!
//! `State::status` still works: whatever a handler writes there becomes a toast on the next
//! frame (`poll` reads it, classifies it and clears it), so the old call sites need no edits.
//!
//! Each toast is an explicit state machine (`Phase`): queued behind the cap, shown with an
//! auto-dismiss timer, held while the pointer is over it, then leaving while its exit plays.
//! Timers are events: `step` returns `Effect::WakeAt` instead of sleeping, the shell hands it to
//! `Tx::wake_at`, and `poll` feeds the due timer back as `Event::Due`. A timer whose attempt the
//! toast no longer holds is stale and changes nothing.

use rune::machine::{Attempt, Effects, Seq};
use rune::{Color, El, Key, KeyEvent, Tx, col, el, row, text};

use crate::app::State;
use crate::icons;
use crate::ui::{BODY, BODY_MED, PAGE_X, Ui};

/// Toasts on screen at once. More wait in the queue.
pub const CAP: usize = 3;
/// Queued toasts kept; the oldest queued one is dropped past this.
pub const QUEUE_MAX: usize = 12;
/// How long a dismissed toast keeps its place while its exit plays.
pub const LEAVE: f64 = 0.2;
/// After the pointer leaves a held toast, at least this long remains on its timer.
pub const RESUME_MIN: f64 = 2.0;
/// A toast with a button stays at least this long, so there is time to press it.
pub const ACTION_MIN: f64 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Info,
    Success,
    Warning,
    Error,
}

impl Kind {
    /// Seconds on screen before the toast leaves by itself. Errors stay longest.
    pub fn duration(self) -> f64 {
        match self {
            Kind::Info | Kind::Success => 4.0,
            Kind::Warning => 6.0,
            Kind::Error => 10.0,
        }
    }

    /// Warnings and errors interrupt a screen reader; the rest wait their turn.
    pub fn role(self) -> &'static str {
        match self {
            Kind::Info | Kind::Success => "status",
            Kind::Warning | Kind::Error => "alert",
        }
    }
}

/// A one-shot callback a toast button runs against the app state.
pub type Run<C> = Box<dyn FnOnce(&mut C)>;

/// What to say. Build with `Notice::info` and friends, then `.detail`, `.action`, `.undo`.
pub struct Notice<C> {
    pub kind: Kind,
    pub title: String,
    pub detail: Option<String>,
    action: Option<(String, Run<C>)>,
    undo: Option<Run<C>>,
}

impl<C> Notice<C> {
    pub fn new(kind: Kind, title: impl Into<String>) -> Self {
        Notice {
            kind,
            title: title.into(),
            detail: None,
            action: None,
            undo: None,
        }
    }

    pub fn info(title: impl Into<String>) -> Self {
        Self::new(Kind::Info, title)
    }

    pub fn success(title: impl Into<String>) -> Self {
        Self::new(Kind::Success, title)
    }

    pub fn warning(title: impl Into<String>) -> Self {
        Self::new(Kind::Warning, title)
    }

    pub fn error(title: impl Into<String>) -> Self {
        Self::new(Kind::Error, title)
    }

    /// A second, quieter line.
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        let detail = detail.into();
        self.detail = (!detail.trim().is_empty()).then_some(detail);
        self
    }

    /// A button that runs `f` once and dismisses the toast.
    pub fn action(mut self, label: impl Into<String>, f: impl FnOnce(&mut C) + 'static) -> Self {
        self.action = Some((label.into(), Box::new(f)));
        self
    }

    /// An Undo button that runs `f` once and dismisses the toast.
    pub fn undo(mut self, f: impl FnOnce(&mut C) + 'static) -> Self {
        self.undo = Some(Box::new(f));
        self
    }

    pub fn has_undo(&self) -> bool {
        self.undo.is_some()
    }

    pub fn action_label(&self) -> Option<&str> {
        self.action.as_ref().map(|(label, _)| label.as_str())
    }

    fn duration(&self) -> f64 {
        let base = self.kind.duration();
        if self.undo.is_some() || self.action.is_some() {
            base.max(ACTION_MIN)
        } else {
            base
        }
    }

    /// The same words of the same kind: shown once, with its timer restarted.
    fn same(&self, other: &Notice<C>) -> bool {
        self.kind == other.kind && self.title == other.title && self.detail == other.detail
    }
}

pub type Id = u64;

/// One toast's life. Each variant carries only what is valid in it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    /// Waiting for a place under the cap.
    Queued,
    /// On screen; leaves at `due` unless its attempt is replaced first.
    Shown { attempt: Attempt, due: f64 },
    /// The pointer is over it: no timer runs. `left` seconds remained when it was held.
    Held { left: f64 },
    /// Dismissed; not drawn (Rune plays the exit) but keeps its place until `due`.
    Leaving { attempt: Attempt, due: f64 },
}

pub struct Toast<C> {
    pub id: Id,
    pub notice: Notice<C>,
    pub phase: Phase,
}

impl<C> Toast<C> {
    pub fn visible(&self) -> bool {
        matches!(self.phase, Phase::Shown { .. } | Phase::Held { .. })
    }

    fn occupies(&self) -> bool {
        !matches!(self.phase, Phase::Queued)
    }
}

pub enum Event<C> {
    Push { notice: Notice<C>, now: f64 },
    Due { id: Id, attempt: Attempt, now: f64 },
    Hover { id: Id, inside: bool, now: f64 },
    Dismiss { id: Id, now: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    /// Wake the poll at this clock time (seconds on `Tx::clock`).
    WakeAt(f64),
}

/// The toast list and its timers. Generic over the state the buttons act on, so the machine is
/// tested without the app.
pub struct Notices<C> {
    toasts: Vec<Toast<C>>,
    seq: Seq,
    next_id: Id,
    /// The clock at the last poll, for pushes from handlers that have no `Tx`.
    pub now: f64,
    /// Wakes asked for by pushes outside a `Tx`; the next poll hands them over.
    pending: Vec<f64>,
}

impl<C> Default for Notices<C> {
    fn default() -> Self {
        Notices {
            toasts: Vec::new(),
            seq: Seq::default(),
            next_id: 1,
            now: 0.0,
            pending: Vec::new(),
        }
    }
}

impl<C> Notices<C> {
    /// The toasts to draw, oldest first.
    pub fn visible(&self) -> impl Iterator<Item = &Toast<C>> {
        self.toasts.iter().filter(|toast| toast.visible())
    }

    #[cfg(test)]
    pub fn get(&self, id: Id) -> Option<&Toast<C>> {
        self.toasts.iter().find(|toast| toast.id == id)
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.toasts.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.toasts.is_empty()
    }

    /// A fingerprint of what is drawn, to tell when the stack changed.
    pub fn drawn(&self) -> Vec<Id> {
        self.visible().map(|toast| toast.id).collect()
    }

    fn index(&self, id: Id) -> Option<usize> {
        self.toasts.iter().position(|toast| toast.id == id)
    }

    fn occupied(&self) -> usize {
        self.toasts.iter().filter(|toast| toast.occupies()).count()
    }

    fn show(&mut self, index: usize, now: f64) -> Effects<Effect> {
        let due = now + self.toasts[index].notice.duration();
        let attempt = self.seq.next();
        self.toasts[index].phase = Phase::Shown { attempt, due };
        Effects::one(Effect::WakeAt(due))
    }

    fn leave(&mut self, index: usize, now: f64) -> Effects<Effect> {
        let due = now + LEAVE;
        let attempt = self.seq.next();
        self.toasts[index].phase = Phase::Leaving { attempt, due };
        Effects::one(Effect::WakeAt(due))
    }

    /// The one transition. Returns what should happen instead of doing it.
    pub fn step(&mut self, event: Event<C>) -> Effects<Effect> {
        match event {
            Event::Push { notice, now } => {
                if let Some(index) = self.toasts.iter().position(|toast| {
                    !matches!(toast.phase, Phase::Leaving { .. }) && toast.notice.same(&notice)
                }) {
                    // Dedup: keep one toast, take the newest buttons, restart its timer.
                    self.toasts[index].notice = notice;
                    return match self.toasts[index].phase {
                        Phase::Shown { .. } => self.show(index, now),
                        Phase::Held { .. } => {
                            let left = self.toasts[index].notice.duration();
                            self.toasts[index].phase = Phase::Held { left };
                            Effects::none()
                        }
                        Phase::Queued | Phase::Leaving { .. } => Effects::none(),
                    };
                }
                let id = self.next_id;
                self.next_id += 1;
                let room = self.occupied() < CAP;
                self.toasts.push(Toast {
                    id,
                    notice,
                    phase: Phase::Queued,
                });
                let queued = self
                    .toasts
                    .iter()
                    .filter(|toast| toast.phase == Phase::Queued)
                    .count();
                if queued > QUEUE_MAX
                    && let Some(oldest) = self
                        .toasts
                        .iter()
                        .position(|toast| toast.phase == Phase::Queued)
                {
                    self.toasts.remove(oldest);
                }
                if room {
                    let index = self.toasts.len() - 1;
                    self.show(index, now)
                } else {
                    Effects::none()
                }
            }
            Event::Due { id, attempt, now } => {
                let Some(index) = self.index(id) else {
                    return Effects::none();
                };
                match self.toasts[index].phase {
                    Phase::Shown { attempt: a, due } if a == attempt && now >= due => {
                        self.leave(index, now)
                    }
                    Phase::Leaving { attempt: a, due } if a == attempt && now >= due => {
                        self.toasts.remove(index);
                        match self
                            .toasts
                            .iter()
                            .position(|toast| toast.phase == Phase::Queued)
                        {
                            Some(next) if self.occupied() < CAP => self.show(next, now),
                            _ => Effects::none(),
                        }
                    }
                    _ => Effects::none(),
                }
            }
            Event::Hover { id, inside, now } => {
                let Some(index) = self.index(id) else {
                    return Effects::none();
                };
                match (self.toasts[index].phase, inside) {
                    (Phase::Shown { due, .. }, true) => {
                        self.toasts[index].phase = Phase::Held {
                            left: (due - now).max(0.0),
                        };
                        Effects::none()
                    }
                    (Phase::Held { left }, false) => {
                        let due = now + left.max(RESUME_MIN);
                        let attempt = self.seq.next();
                        self.toasts[index].phase = Phase::Shown { attempt, due };
                        Effects::one(Effect::WakeAt(due))
                    }
                    _ => Effects::none(),
                }
            }
            Event::Dismiss { id, now } => {
                let Some(index) = self.index(id) else {
                    return Effects::none();
                };
                match self.toasts[index].phase {
                    Phase::Queued => {
                        self.toasts.remove(index);
                        Effects::none()
                    }
                    Phase::Shown { .. } | Phase::Held { .. } => self.leave(index, now),
                    Phase::Leaving { .. } => Effects::none(),
                }
            }
        }
    }

    /// Feed every timer that has come due back into `step`.
    pub fn tick(&mut self, now: f64) -> Effects<Effect> {
        let due: Vec<(Id, Attempt)> = self
            .toasts
            .iter()
            .filter_map(|toast| match toast.phase {
                Phase::Shown { attempt, due } | Phase::Leaving { attempt, due } if now >= due => {
                    Some((toast.id, attempt))
                }
                _ => None,
            })
            .collect();
        let mut effects = Effects::none();
        for (id, attempt) in due {
            effects.extend(self.step(Event::Due { id, attempt, now }));
        }
        effects
    }

    /// Take a toast's Undo, if it still has one. The caller runs it and dismisses the toast.
    pub fn take_undo(&mut self, id: Id) -> Option<Run<C>> {
        let index = self.index(id)?;
        self.toasts[index].notice.undo.take()
    }

    /// Take a toast's action button callback.
    pub fn take_action(&mut self, id: Id) -> Option<Run<C>> {
        let index = self.index(id)?;
        self.toasts[index].notice.action.take().map(|(_, run)| run)
    }

    /// Keep wakes asked for outside a `Tx` until the next poll.
    pub fn defer(&mut self, effects: Effects<Effect>) {
        for Effect::WakeAt(due) in effects {
            self.pending.push(due);
        }
    }

    pub fn take_pending(&mut self) -> Vec<f64> {
        std::mem::take(&mut self.pending)
    }
}

// --- status lines ---------------------------------------------------------------------------

/// Turn an old status line into a toast: pick its kind from its words and split a long line
/// into a title (its first sentence) and a detail (the rest).
pub fn from_status<C>(line: &str) -> Notice<C> {
    let line = line.trim();
    let (title, detail) = split_sentence(line);
    let notice = Notice::new(classify(line), title);
    match detail {
        Some(detail) => notice.detail(detail),
        None => notice,
    }
}

pub fn classify(line: &str) -> Kind {
    let lower = line.to_lowercase();
    const ERROR: [&str; 12] = [
        "failed",
        "error",
        "could not",
        "couldn't",
        "cannot",
        "can't",
        "not found",
        "blocked",
        "no deadlock install",
        "refus",
        "stopped before",
        "was not found",
    ];
    const WARNING: [&str; 9] = [
        "nothing was",
        "not approved",
        "missing",
        "not installed",
        "not running",
        "not detected",
        "first",
        "already active",
        "warning",
    ];
    const SUCCESS: [&str; 22] = [
        "saved",
        "installed",
        "applied",
        "restored",
        "repaired",
        "deleted",
        "removed",
        "enabled",
        "disabled",
        "imported",
        "exported",
        "copied",
        "updated",
        "created",
        "renamed",
        "cleared",
        "forgotten",
        "approved",
        "added",
        "moved",
        "wrote",
        "turned",
    ];
    if ERROR.iter().any(|word| lower.contains(word)) {
        return Kind::Error;
    }
    if WARNING.iter().any(|word| lower.contains(word)) {
        return Kind::Warning;
    }
    let first = lower
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .find(|word| !word.is_empty())
        .unwrap_or_default();
    if SUCCESS.contains(&first) {
        return Kind::Success;
    }
    Kind::Info
}

/// A line longer than a toast title splits after its first sentence: a period, then a space
/// and a capital letter, so `1.5 s` and `e.g. this` stay whole.
fn split_sentence(line: &str) -> (String, Option<String>) {
    const TITLE_MAX: usize = 64;
    if line.chars().count() <= TITLE_MAX {
        return (line.to_string(), None);
    }
    let bytes = line.as_bytes();
    for (index, _) in line.match_indices(". ") {
        let next = bytes.get(index + 2).copied().unwrap_or(b' ');
        if next.is_ascii_uppercase() {
            let title = line[..=index].trim().to_string();
            let rest = line[index + 2..].trim().to_string();
            return (title, (!rest.is_empty()).then_some(rest));
        }
    }
    (line.to_string(), None)
}

// --- the app's side -------------------------------------------------------------------------

/// Show a toast. Callable from any handler: its timer starts on the next poll, which this asks
/// for.
pub fn push(st: &mut State, notice: Notice<State>) {
    let now = st.notices.now;
    let effects = st.notices.step(Event::Push { notice, now });
    st.notices.defer(effects);
    kick(st);
}

/// Ask for a frame, so the next poll starts the timers a push from a plain handler deferred
/// and turns a fresh status line into a toast.
fn kick(st: &State) {
    if let Some(wake) = &st.wake {
        wake();
    }
}

/// Called from the view: a status line written by a handler since the last poll needs a poll.
pub fn needs_poll(s: &State) -> bool {
    let fresh = !s.status.trim().is_empty();
    if fresh {
        kick(s);
    }
    fresh
}

fn carry(tx: &mut Tx, effects: Effects<Effect>) {
    for Effect::WakeAt(due) in effects {
        tx.wake_at(due);
    }
}

/// The shell's poll: adopt the status line, fire due timers, start deferred ones. True when
/// the stack changed.
pub fn poll(st: &mut State, now: f64, tx: &mut Tx) -> bool {
    st.notices.now = now;
    let before = st.notices.drawn();
    let line = std::mem::take(&mut st.status);
    if !line.trim().is_empty() {
        let effects = st.notices.step(Event::Push {
            notice: from_status(&line),
            now,
        });
        carry(tx, effects);
    }
    let effects = st.notices.tick(now);
    carry(tx, effects);
    for due in st.notices.take_pending() {
        tx.wake_at(due);
    }
    let changed = st.notices.drawn() != before;
    if changed {
        // A poll's commit would otherwise mount and unmount at once; the toast slides.
        tx.animated();
    }
    changed
}

fn dismiss(st: &mut State, id: Id, tx: &mut Tx) {
    let effects = st.notices.step(Event::Dismiss {
        id,
        now: tx.clock(),
    });
    carry(tx, effects);
}

fn run_undo(st: &mut State, id: Id, tx: &mut Tx) {
    dismiss(st, id, tx);
    if let Some(undo) = st.notices.take_undo(id) {
        undo(st);
    }
}

fn run_action(st: &mut State, id: Id, tx: &mut Tx) {
    dismiss(st, id, tx);
    if let Some(action) = st.notices.take_action(id) {
        action(st);
    }
}

fn hover(st: &mut State, id: Id, inside: bool, tx: &mut Tx) {
    let effects = st.notices.step(Event::Hover {
        id,
        inside,
        now: tx.clock(),
    });
    carry(tx, effects);
}

// --- view -----------------------------------------------------------------------------------

pub const TOAST_W: f32 = 400.0;

/// The stack, pinned to the window's bottom-right corner. None when nothing is shown.
pub fn region(s: &State, u: &Ui) -> Option<El> {
    let toasts: Vec<El> = s.notices.visible().map(|toast| card(toast, u)).collect();
    if toasts.is_empty() {
        return None;
    }
    Some(
        col()
            .key("toasts")
            .role("region")
            .label("Notifications")
            .pin(None, None, Some(PAGE_X), Some(24.0))
            .w(TOAST_W)
            .gap(8.0)
            .children(toasts),
    )
}

fn kind_icon(kind: Kind, u: &Ui) -> El {
    let (glyph, color) = match kind {
        Kind::Info => (&icons::INFO, u.muted),
        Kind::Success => (&icons::CIRCLE_CHECK, u.teal),
        Kind::Warning => (&icons::TRIANGLE_ALERT, u.amber),
        Kind::Error => (&icons::CIRCLE_X, u.rose),
    };
    rune::icon(glyph, 16.0).color(color).a11y_hidden()
}

fn card(toast: &Toast<State>, u: &Ui) -> El {
    let id = toast.id;
    let notice = &toast.notice;
    let mut words = col()
        .grow(1.0)
        .shrink()
        .min_w(0.0)
        .gap(2.0)
        .a11y_hidden()
        .child(
            text(notice.title.clone())
                .font(BODY_MED)
                .color(u.ink)
                .wrap()
                .w_full(),
        );
    if let Some(detail) = &notice.detail {
        words = words.child(
            text(detail.clone())
                .font(BODY)
                .color(u.muted)
                .wrap()
                .w_full(),
        );
    }
    let mut body = row()
        .w_full()
        .gap(10.0)
        .items_start()
        .child(el().size(16.0, 20.0).center().child(kind_icon(notice.kind, u)))
        .child(words);
    if let Some(label) = notice.action_label() {
        body = body.child(
            text_button(u, &format!("toast-{id}-action"), label)
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |st: &mut State, tx: &mut Tx| run_action(st, id, tx)),
        );
    }
    if notice.has_undo() {
        body = body.child(
            text_button(u, &format!("toast-{id}-undo"), "Undo")
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |st: &mut State, tx: &mut Tx| run_undo(st, id, tx)),
        );
    }
    body = body.child(
        el().key(format!("toast-{id}-close"))
            .size(24.0, 24.0)
            .mt(-2.0)
            .center()
            .radius(crate::ui::r(6.0))
            .hover(u.hover)
            .press(u.over.alpha(0.08))
            .focusable(true)
            .role("button")
            .label("Dismiss")
            .cursor(rune::Cursor::Pointer)
            .on_press_tx(move |st: &mut State, tx: &mut Tx| dismiss(st, id, tx))
            .child(rune::icon(&icons::X, 14.0).color(u.faint).a11y_hidden()),
    );
    let border = match notice.kind {
        Kind::Error => u.rose.alpha(0.35),
        Kind::Warning => u.amber.alpha(0.35),
        Kind::Info | Kind::Success => u.hair_strong,
    };
    let mut card = col()
        .key(format!("toast-{id}"))
        .role(notice.kind.role())
        .label(notice.title.clone())
        .w_full()
        .pl(12.0)
        .pr(8.0)
        .py(10.0)
        .radius(crate::ui::r(10.0))
        .fill(u.raised)
        .border(1.0, border)
        .shadow(
            12.0,
            32.0,
            Color::BLACK.alpha(if u.light { 0.14 } else { 0.45 }),
        )
        .on_hover_change(move |st: &mut State, inside: bool, tx: &mut Tx| {
            hover(st, id, inside, tx)
        })
        .on_key(move |st: &mut State, ev: &KeyEvent, tx: &mut Tx| {
            if ev.key == Key::Escape {
                dismiss(st, id, tx);
                true
            } else {
                false
            }
        })
        .transition(
            rune::Transition::offset(24.0, 0.0).combined(rune::Transition::opacity()),
        )
        .motion(rune::tok::motion::SNAPPY)
        .child(body);
    if let Some(detail) = &notice.detail {
        card = card.description(detail.clone());
    }
    card
}

/// A compact text button for a toast: 24 points tall, ink text, a hover fill.
fn text_button(u: &Ui, key: &str, label: &str) -> El {
    row()
        .key(key.to_string())
        .h(24.0)
        .mt(-2.0)
        .px(8.0)
        .center()
        .radius(crate::ui::r(6.0))
        .hover(u.hover)
        .press(u.over.alpha(0.08))
        .focusable(true)
        .role("button")
        .label(label)
        .child(text(label.to_string()).font(BODY_MED).color(u.ink).a11y_hidden())
}

// --- scenes ---------------------------------------------------------------------------------

/// `notify-stack`: one toast of each loud kind over Installed, the newest at the bottom.
pub fn scene_stack(s: &mut State) {
    crate::app::scene_demo_installed(s);
    push(s, Notice::success("Installed Ashfall in slot 03"));
    push(
        s,
        Notice::warning("2 mods were not installed").detail("Install them from Browse, then apply the profile again."),
    );
    push(
        s,
        Notice::error("Could not write autoexec.cfg").detail("Permission denied. Close the game and try again."),
    );
}

/// `notify-undo`: an info line and a toast with Undo.
pub fn scene_undo(s: &mut State) {
    crate::app::scene_demo_installed(s);
    s.status = "Starting Deadlock with 4 mods on.".to_string();
    let line = std::mem::take(&mut s.status);
    push(s, from_status(&line));
    push(
        s,
        Notice::success("Turned off slot 03").undo(|st: &mut State| {
            push(st, Notice::info("Turned slot 03 back on"));
        }),
    );
}

/// `notify-banner`: toasts and the gameinfo.gi banner together.
pub fn scene_with_banner(s: &mut State) {
    crate::banner::scene_banner(s);
    push(s, Notice::info("Checked 12 mods. All up to date"));
}

#[cfg(test)]
mod tests {
    use super::*;

    type Log = Vec<&'static str>;
    type N = Notices<Log>;

    fn wakes(effects: Effects<Effect>) -> Vec<f64> {
        effects.into_iter().map(|Effect::WakeAt(due)| due).collect()
    }

    fn push(n: &mut N, notice: Notice<Log>, now: f64) -> Vec<f64> {
        wakes(n.step(Event::Push { notice, now }))
    }

    fn phase(n: &N, id: Id) -> Phase {
        n.get(id).expect("toast exists").phase
    }

    #[test]
    fn a_push_shows_and_asks_for_its_timer() {
        let mut n = N::default();
        assert_eq!(push(&mut n, Notice::info("Hello"), 1.0), vec![5.0]);
        assert!(matches!(phase(&n, 1), Phase::Shown { due, .. } if due == 5.0));
        assert_eq!(n.drawn(), vec![1]);
    }

    #[test]
    fn errors_stay_longer_and_buttons_get_time() {
        let mut n = N::default();
        assert_eq!(push(&mut n, Notice::error("Broke"), 0.0), vec![10.0]);
        assert_eq!(push(&mut n, Notice::warning("Careful"), 0.0), vec![6.0]);
        assert_eq!(
            push(&mut n, Notice::success("Done").undo(|log: &mut Log| log.push("undo")), 0.0),
            vec![ACTION_MIN]
        );
    }

    #[test]
    fn the_timer_moves_it_to_leaving_then_removes_it() {
        let mut n = N::default();
        push(&mut n, Notice::info("Hi"), 0.0);
        // Early: nothing happens.
        assert!(wakes(n.tick(3.9)).is_empty());
        assert!(n.get(1).unwrap().visible());
        // Due: it leaves, keeps its place, asks for the exit's end.
        let leave = wakes(n.tick(4.0));
        assert_eq!(leave.len(), 1);
        assert!((leave[0] - (4.0 + LEAVE)).abs() < 1e-9);
        assert!(matches!(phase(&n, 1), Phase::Leaving { .. }));
        assert!(n.drawn().is_empty());
        n.tick(4.0 + LEAVE);
        assert!(n.is_empty());
    }

    #[test]
    fn a_stale_timer_changes_nothing() {
        let mut n = N::default();
        push(&mut n, Notice::info("Hi"), 0.0);
        let Phase::Shown { attempt: old, .. } = phase(&n, 1) else {
            panic!("shown");
        };
        // The same words again restart the timer under a new attempt.
        push(&mut n, Notice::info("Hi"), 3.0);
        let effects = n.step(Event::Due {
            id: 1,
            attempt: old,
            now: 4.5,
        });
        assert!(effects.is_empty());
        assert!(matches!(phase(&n, 1), Phase::Shown { due, .. } if due == 7.0));
    }

    #[test]
    fn identical_messages_are_shown_once() {
        let mut n = N::default();
        push(&mut n, Notice::warning("Disk full").detail("Free some space"), 0.0);
        push(&mut n, Notice::warning("Disk full").detail("Free some space"), 1.0);
        assert_eq!(n.len(), 1);
        // A different kind or detail is a different message.
        push(&mut n, Notice::error("Disk full").detail("Free some space"), 1.0);
        push(&mut n, Notice::warning("Disk full"), 1.0);
        assert_eq!(n.len(), 3);
    }

    #[test]
    fn a_repeat_takes_the_newest_undo() {
        let mut n = N::default();
        push(&mut n, Notice::success("Saved").undo(|log: &mut Log| log.push("first")), 0.0);
        push(&mut n, Notice::success("Saved").undo(|log: &mut Log| log.push("second")), 1.0);
        let mut log = Vec::new();
        (n.take_undo(1).expect("undo"))(&mut log);
        assert_eq!(log, vec!["second"]);
        // One shot.
        assert!(n.take_undo(1).is_none());
    }

    #[test]
    fn the_cap_queues_the_rest_and_promotes_in_order() {
        let mut n = N::default();
        for (i, title) in ["a", "b", "c", "d", "e"].into_iter().enumerate() {
            let wake = push(&mut n, Notice::info(title), i as f64 * 0.1);
            assert_eq!(wake.is_empty(), i >= CAP, "toast {title}");
        }
        assert_eq!(n.drawn(), vec![1, 2, 3]);
        assert_eq!(phase(&n, 4), Phase::Queued);
        // Dismissing one frees its place only once its exit has played.
        n.step(Event::Dismiss { id: 2, now: 1.0 });
        assert_eq!(n.drawn(), vec![1, 3]);
        assert_eq!(phase(&n, 4), Phase::Queued);
        let wake = wakes(n.tick(1.0 + LEAVE));
        assert_eq!(n.drawn(), vec![1, 3, 4]);
        assert_eq!(wake, vec![1.0 + LEAVE + Kind::Info.duration()]);
        assert_eq!(phase(&n, 5), Phase::Queued);
    }

    #[test]
    fn the_queue_drops_its_oldest_past_the_limit() {
        let mut n = N::default();
        for i in 0..(CAP + QUEUE_MAX + 2) {
            push(&mut n, Notice::info(format!("m{i}")), 0.0);
        }
        assert_eq!(n.len(), CAP + QUEUE_MAX);
        // The first queued ones went; the shown ones stayed.
        assert_eq!(n.drawn(), vec![1, 2, 3]);
        assert!(n.get(4).is_none() && n.get(5).is_none());
        assert!(n.get(6).is_some());
    }

    #[test]
    fn hovering_holds_the_timer() {
        let mut n = N::default();
        push(&mut n, Notice::info("Hold me"), 0.0);
        assert!(wakes(n.step(Event::Hover { id: 1, inside: true, now: 1.0 })).is_empty());
        assert_eq!(phase(&n, 1), Phase::Held { left: 3.0 });
        // Far past the old due time, the held toast stays.
        assert!(wakes(n.tick(60.0)).is_empty());
        assert!(n.get(1).unwrap().visible());
        // Leaving restarts the timer with what was left.
        assert_eq!(
            wakes(n.step(Event::Hover { id: 1, inside: false, now: 60.0 })),
            vec![63.0]
        );
        n.tick(63.0);
        assert!(matches!(phase(&n, 1), Phase::Leaving { .. }));
    }

    #[test]
    fn a_short_remainder_gets_a_minimum_after_hover() {
        let mut n = N::default();
        push(&mut n, Notice::info("Almost gone"), 0.0);
        n.step(Event::Hover { id: 1, inside: true, now: 3.9 });
        assert_eq!(
            wakes(n.step(Event::Hover { id: 1, inside: false, now: 10.0 })),
            vec![10.0 + RESUME_MIN]
        );
    }

    #[test]
    fn a_repeat_while_held_stays_held_with_a_full_timer() {
        let mut n = N::default();
        push(&mut n, Notice::info("Same"), 0.0);
        n.step(Event::Hover { id: 1, inside: true, now: 3.5 });
        assert!(push(&mut n, Notice::info("Same"), 3.6).is_empty());
        assert_eq!(phase(&n, 1), Phase::Held { left: 4.0 });
    }

    #[test]
    fn dismiss_removes_a_queued_toast_at_once() {
        let mut n = N::default();
        for title in ["a", "b", "c", "d"] {
            push(&mut n, Notice::info(title), 0.0);
        }
        assert!(n.step(Event::Dismiss { id: 4, now: 0.5 }).is_empty());
        assert!(n.get(4).is_none());
        // Dismissing twice is harmless.
        n.step(Event::Dismiss { id: 1, now: 0.5 });
        assert!(n.step(Event::Dismiss { id: 1, now: 0.6 }).is_empty());
    }

    #[test]
    fn a_leaving_toast_is_not_revived_by_its_words() {
        let mut n = N::default();
        push(&mut n, Notice::info("Bye"), 0.0);
        n.step(Event::Dismiss { id: 1, now: 1.0 });
        push(&mut n, Notice::info("Bye"), 1.05);
        assert_eq!(n.len(), 2);
        assert_eq!(n.drawn(), vec![2]);
    }

    #[test]
    fn actions_run_once() {
        let mut n = N::default();
        push(
            &mut n,
            Notice::info("Update ready").action("Details", |log: &mut Log| log.push("details")),
            0.0,
        );
        assert_eq!(n.get(1).unwrap().notice.action_label(), Some("Details"));
        let mut log = Vec::new();
        (n.take_action(1).expect("action"))(&mut log);
        assert_eq!(log, vec!["details"]);
        assert!(n.take_action(1).is_none());
    }

    #[test]
    fn deferred_wakes_are_handed_over_once() {
        let mut n = N::default();
        let effects = n.step(Event::Push {
            notice: Notice::info("From a handler"),
            now: 2.0,
        });
        n.defer(effects);
        assert_eq!(n.take_pending(), vec![6.0]);
        assert!(n.take_pending().is_empty());
    }

    #[test]
    fn status_lines_pick_a_kind() {
        assert_eq!(classify("Repair failed: permission denied"), Kind::Error);
        assert_eq!(classify("Steam could not be opened: x"), Kind::Error);
        assert_eq!(
            classify("No Deadlock install was found. Nothing was renamed."),
            Kind::Error
        );
        assert_eq!(classify("Find the game folder first."), Kind::Warning);
        assert_eq!(classify("Saved. The game runs it on the next start."), Kind::Success);
        assert_eq!(classify("Crosshair applied to /x/autoexec.cfg."), Kind::Info);
        assert_eq!(classify("Applied Ranked: 2 mods switched."), Kind::Success);
        assert_eq!(classify("Type a prompt before sending."), Kind::Info);
    }

    #[test]
    fn long_status_lines_split_after_the_first_sentence() {
        let notice: Notice<()> = from_status(
            "Game found, but gameinfo.gi does not list citadel/addons. The game will ignore every addon.",
        );
        assert_eq!(
            notice.title,
            "Game found, but gameinfo.gi does not list citadel/addons."
        );
        assert_eq!(notice.detail.as_deref(), Some("The game will ignore every addon."));
        // Short lines and lines without a sentence break stay whole.
        let short: Notice<()> = from_status("Saved. Done.");
        assert_eq!(short.title, "Saved. Done.");
        assert!(short.detail.is_none());
        let decimal: Notice<()> = from_status(
            "The scan took 1.5 s and read every addon slot in the folder without any problem at all",
        );
        assert!(decimal.detail.is_none());
    }
}
