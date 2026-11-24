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
//! Every error and warning toast carries Send feedback (`feedback.rs`), prefilled with its title,
//! detail and page, whoever pushed it: a `Notice::error`, a `Notice::warning` or a status line
//! classified as one. Errors stay until closed; warnings stay at least [`WARNING_MIN`] seconds.
//! The same title pushed again folds into one toast with a repeat count instead of stacking.
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
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{BODY, BODY_MED, PAGE_X, SMALL, Ui};

pub mod feedback;

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
/// A warning stays at least this long: time to read it and press Send feedback.
pub const WARNING_MIN: f64 = 20.0;
/// Reading time per word: about 200 words a minute (issue #5).
pub const PER_WORD: f64 = 0.3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Info,
    Success,
    Warning,
    Error,
}

impl Kind {
    /// The shortest time on screen before the toast leaves by itself. `None`: an error stays
    /// until the player closes it (issue #5).
    pub fn duration(self) -> Option<f64> {
        match self {
            Kind::Info | Kind::Success => Some(4.0),
            Kind::Warning => Some(WARNING_MIN),
            Kind::Error => None,
        }
    }

    /// Louder kinds win when two toasts say the same thing.
    fn rank(self) -> u8 {
        match self {
            Kind::Info => 0,
            Kind::Success => 1,
            Kind::Warning => 2,
            Kind::Error => 3,
        }
    }

    /// Errors and warnings offer Send feedback.
    pub fn loud(self) -> bool {
        matches!(self, Kind::Warning | Kind::Error)
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
    /// How many times this was pushed while on screen or waiting (1 the first time).
    pub repeats: u32,
}

impl<C> Notice<C> {
    pub fn new(kind: Kind, title: impl Into<String>) -> Self {
        Notice {
            kind,
            title: tidy_title(&title.into()),
            detail: None,
            action: None,
            undo: None,
            repeats: 1,
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

    /// A second, quieter line. A detail that only repeats the title is dropped (issue #4).
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        let detail = detail.into();
        let detail = detail.trim();
        let repeats = detail
            .trim_end_matches('.')
            .eq_ignore_ascii_case(self.title.trim_end_matches('.'));
        self.detail = if detail.is_empty() || repeats {
            None
        } else if let Some(rest) = repeat_prefix(detail, &self.title) {
            Some(rest.to_string())
        } else {
            Some(detail.to_string())
        };
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

    /// Seconds on screen: the kind's minimum, the reading time of its words, and the button
    /// minimum. `None` for an error: it stays until closed.
    pub fn duration(&self) -> Option<f64> {
        let base = self.kind.duration()?;
        let words = self.title.split_whitespace().count()
            + self.detail.as_deref().map_or(0, |d| d.split_whitespace().count());
        let mut seconds = base.max(words as f64 * PER_WORD);
        if self.undo.is_some() || self.action.is_some() {
            seconds = seconds.max(ACTION_MIN);
        }
        Some(seconds)
    }

    /// The same title: one action, one toast (issue #4), however the two callers worded the
    /// rest.
    fn same(&self, other: &Notice<C>) -> bool {
        self.title.trim().eq_ignore_ascii_case(other.title.trim())
    }

    /// Fold a repeat into this toast: the louder kind, the newer detail and buttons, keeping
    /// any the repeat does not bring.
    fn absorb(&mut self, newer: Notice<C>) {
        self.repeats = self.repeats.saturating_add(newer.repeats);
        if newer.kind.rank() >= self.kind.rank() {
            self.kind = newer.kind;
        }
        if newer.detail.is_some() {
            self.detail = newer.detail;
        }
        if newer.action.is_some() {
            self.action = newer.action;
        }
        if newer.undo.is_some() {
            self.undo = newer.undo;
        }
    }
}

/// One format for every title: no trailing period (an ellipsis stays), no doubled spaces, and
/// a first word in plain lowercase letters gets its capital.
pub fn tidy_title(title: &str) -> String {
    let mut out = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.ends_with('.') && !out.ends_with("..") {
        out.pop();
    }
    let first = out.split(' ').next().unwrap_or_default();
    if !first.is_empty() && first.chars().all(|c| c.is_ascii_lowercase()) {
        let mut chars = out.chars();
        if let Some(c) = chars.next() {
            out = c.to_ascii_uppercase().to_string() + chars.as_str();
        }
    }
    out
}

/// A detail that opens with the title and a separator: the part after it.
fn repeat_prefix<'a>(detail: &'a str, title: &str) -> Option<&'a str> {
    let title = title.trim_end_matches('.');
    let head = detail.get(..title.len())?;
    if !head.eq_ignore_ascii_case(title) {
        return None;
    }
    let rest = detail[title.len()..].trim_start_matches(['.', ':', ',', ' ']);
    (!rest.is_empty() && rest.len() < detail.len()).then_some(rest)
}

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.len() >= 3)
        .map(str::to_lowercase)
        .collect()
}

/// Two lines about the same thing: at least half the shorter one's words are shared.
fn similar(a: &str, b: &str) -> bool {
    let (a, b) = (words(a), words(b));
    let shorter = a.len().min(b.len());
    let shared = a.iter().filter(|word| b.contains(word)).count();
    shorter > 0 && shared * 2 >= shorter
}

/// The pushed detail already carries the line (a raw error under a friendlier title).
fn contains_words(detail: &str, line: &str) -> bool {
    let (detail, line) = (words(detail), words(line));
    !line.is_empty() && line.iter().all(|word| detail.contains(word))
}

fn wake(due: f64) -> Effects<Effect> {
    if due.is_finite() {
        Effects::one(Effect::WakeAt(due))
    } else {
        Effects::none()
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
    /// The toasts pushed since the last poll: kind, title and detail. A status line written in
    /// the same frame that says the same thing only shows when it is louder (issue #4).
    pushed: Vec<(Kind, String, String)>,
    /// The Send feedback dialog (`feedback.rs`, issue #5).
    pub feedback: feedback::Feedback,
    /// The Details dialog: a cut toast's title and whole message.
    pub details: Option<(String, String)>,
}

impl<C> Default for Notices<C> {
    fn default() -> Self {
        Notices {
            toasts: Vec::new(),
            seq: Seq::default(),
            next_id: 1,
            now: 0.0,
            pending: Vec::new(),
            pushed: Vec::new(),
            feedback: feedback::Feedback::default(),
            details: None,
        }
    }
}

impl<C> Notices<C> {
    /// Toasts waiting behind the cap, for the stack's "+N more" line.
    pub fn waiting(&self) -> usize {
        self.toasts.iter().filter(|toast| toast.phase == Phase::Queued).count()
    }

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
        let due = self.toasts[index]
            .notice
            .duration()
            .map_or(f64::INFINITY, |seconds| now + seconds);
        let attempt = self.seq.next();
        self.toasts[index].phase = Phase::Shown { attempt, due };
        wake(due)
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
                    // Dedup: keep one toast, take the newest words and buttons, restart its timer.
                    self.toasts[index].notice.absorb(notice);
                    return match self.toasts[index].phase {
                        Phase::Shown { .. } => self.show(index, now),
                        Phase::Held { .. } => {
                            let left = self.toasts[index].notice.duration().unwrap_or(f64::INFINITY);
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
                        wake(due)
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

    /// Put an Undo on the toast pushed this frame, when it has none. False when nothing was
    /// pushed this frame.
    pub fn undo_on_last_push(&mut self, f: impl FnOnce(&mut C) + 'static) -> bool {
        if self.pushed.is_empty() {
            return false;
        }
        let Some(index) = self
            .toasts
            .iter()
            .rposition(|toast| !matches!(toast.phase, Phase::Leaving { .. }))
        else {
            return false;
        };
        if self.toasts[index].notice.undo.is_none() {
            self.toasts[index].notice.undo = Some(Box::new(f));
            // The Undo needs its time on screen.
            if matches!(self.toasts[index].phase, Phase::Shown { .. }) {
                let now = self.now;
                let effects = self.show(index, now);
                self.defer(effects);
            }
        }
        true
    }

    /// A handler or worker result pushed a toast this frame.
    pub fn note_push(&mut self, notice: &Notice<C>) {
        self.pushed.push((
            notice.kind,
            notice.title.clone(),
            notice.detail.clone().unwrap_or_default(),
        ));
    }

    /// A status line from the same frame as a push that says the same thing is the same
    /// action told twice; it only shows when it is louder than that push.
    pub fn status_is_news(&self, line: &str, kind: Kind) -> bool {
        !self.pushed.iter().any(|(pushed, title, detail)| {
            kind.rank() <= pushed.rank() && (similar(line, title) || contains_words(detail, line))
        })
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
    st.notices.note_push(&notice);
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
    // Installed's Undo first: it turns this frame's status line into its toast.
    crate::screens::installed::offer_undo(st);
    let line = std::mem::take(&mut st.status);
    if !line.trim().is_empty() {
        let notice = from_status(&line);
        if st.notices.status_is_news(&line, notice.kind) {
            let effects = st.notices.step(Event::Push { notice, now });
            carry(tx, effects);
        }
    }
    st.notices.pushed.clear();
    st.notices.feedback.poll();
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

/// Every toast is this wide; it fits the 1040 point window beside the sidebar with margins.
pub const TOAST_W: f32 = 400.0;
/// Most text lines a toast draws, title and detail together; the rest is under Details.
pub const MAX_LINES: usize = 4;
/// Most lines the title takes when a detail follows it.
pub const TITLE_LINES: usize = 2;
/// The line height of `BODY` and `BODY_MED`.
const LINE: f32 = 20.0;
/// Card padding (left, right), the icon slot, the two gaps beside the words, the close button.
const CHROME_W: f32 = 12.0 + 8.0 + 16.0 + 10.0 + 10.0 + 24.0;
/// Widest a toast button grows; a longer label is cut, with the whole label in its tooltip.
const BUTTON_MAX_W: f32 = 160.0;

/// How a toast's words fit: the line caps for the title and the detail, and whether anything
/// is cut, which brings the Details button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fit {
    pub title_lines: usize,
    pub detail_lines: usize,
    pub clamped: bool,
}

/// Lines `text` takes at `width` points, wrapping inside long words as the toast does.
fn lines_at(text: &str, style: rune::TextStyle, width: f32) -> usize {
    let (_, h) = rune::text::measure_with(
        text,
        &[],
        style,
        Some(width.max(1.0)),
        rune::text::RichLayoutOptions::new().wrap_anywhere(),
    );
    ((h - 0.5) / LINE).ceil().max(1.0) as usize
}

/// The line caps for a title and detail laid out `width` points wide.
pub fn fit(title: &str, detail: Option<&str>, width: f32) -> Fit {
    let title_need = lines_at(title, BODY_MED, width);
    let Some(detail) = detail else {
        return Fit { title_lines: MAX_LINES, detail_lines: 0, clamped: title_need > MAX_LINES };
    };
    let title_lines = title_need.min(TITLE_LINES);
    let detail_lines = MAX_LINES - title_lines;
    let detail_need = lines_at(detail, BODY, width);
    Fit {
        title_lines: TITLE_LINES,
        detail_lines,
        clamped: title_need > TITLE_LINES || detail_need > detail_lines,
    }
}

/// The whole message, for Details and the tooltips.
fn full_text<C>(notice: &Notice<C>) -> String {
    match &notice.detail {
        Some(detail) => format!("{}\n\n{detail}", notice.title),
        None => notice.title.clone(),
    }
}

/// The stack, pinned to the window's bottom-right corner, and the Send feedback or Details
/// dialog over everything when one is open. None when nothing is shown.
pub fn region(s: &State, u: &Ui) -> Option<El> {
    let toasts: Vec<El> = s.notices.visible().map(|toast| card(toast, u)).collect();
    let waiting = s.notices.waiting();
    let stack = (!toasts.is_empty()).then(|| {
        col()
            .key("toasts")
            .role("region")
            .label(tr("Notifications"))
            .pin(None, None, Some(PAGE_X), Some(24.0))
            .w(TOAST_W)
            .gap(8.0)
            // The stack has no entrance, so the first toast slides in on its own; it fades with the last.
            .transition(crate::motion::layer())
            // Past the cap the rest wait their turn: one quiet line counts them.
            .children((waiting > 0).then(|| {
                row().key("toasts-more").w_full().h(16.0).justify_end().center_y().child(
                    text(fill(tr("+{} more"), &[&waiting.to_string()]))
                        .font(SMALL)
                        .color(u.muted),
                )
            }))
            .children(toasts)
    });
    let dialog = feedback::dialog(s, u).map(|d| ("feedback-modal", d)).or_else(|| details_dialog(s, u).map(|d| ("toast-details-modal", d)));
    let Some((modal_key, dialog)) = dialog else {
        return stack;
    };
    let layer = el()
        .key("feedback-layer")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .transition(crate::motion::layer())
        .child(u.modal_layer(modal_key, dialog));
    Some(match stack {
        Some(stack) => layer.child(stack),
        None => layer,
    })
}

fn close_details(st: &mut State) {
    st.notices.details = None;
}

/// Details: a toast's whole message, selectable, with Copy.
fn details_dialog(s: &State, u: &Ui) -> Option<El> {
    let (title, full) = s.notices.details.clone()?;
    let copy = full.clone();
    Some(
        u.dialog_shell()
            .key("toast-details")
            .role("dialog")
            .label(title.clone())
            .w(560.0)
            .max_w(560.0)
            .padding(24.0)
            .gap(14.0)
            .modal(true, close_details)
            .child(text(title).font(crate::ui::H2).color(u.ink).wrap_anywhere().w_full())
            .child(
                u.scroll_area(col()
                    .key("toast-details-text")
                    .w_full()
                    .max_h(280.0)
                    .padding(12.0)
                    .radius(crate::ui::r(8.0))
                    .fill(u.well)
                    .border(1.0, u.hair)
                    .scroll(rune::ScrollSpec { stick_bottom: false })
                    .child(text(full).font(BODY).color(u.prose).wrap_anywhere().selectable(true).w_full())),
            )
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(rune::spacer())
                    .child(
                        u.action(tr("Copy"), crate::ui::ActionKind::Secondary)
                            .key("toast-details-copy")
                            .cursor(rune::Cursor::Pointer)
                            .on_press_tx(move |_st: &mut State, tx: &mut Tx| tx.copy(copy.clone())),
                    )
                    .child(
                        u.action(tr("Close"), crate::ui::ActionKind::Primary)
                            .key("toast-details-close")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(close_details),
                    ),
            ),
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

/// The width a toast button takes beside the words.
fn button_w(label: &str) -> f32 {
    (rune::text::measure(label, &[], BODY_MED, None).0 + 16.0).min(BUTTON_MAX_W)
}

/// The width the words get on a card: what the icon, close and inline buttons leave.
pub fn words_width<C>(notice: &Notice<C>) -> f32 {
    let mut width = TOAST_W - CHROME_W;
    if !notice.kind.loud() {
        if let Some(label) = notice.action_label() {
            width -= button_w(label) + 10.0;
        }
        if notice.has_undo() {
            width -= button_w(tr("Undo")) + 10.0;
        }
    }
    if notice.repeats >= 2 {
        width -= 30.0;
    }
    width
}

fn card(toast: &Toast<State>, u: &Ui) -> El {
    let id = toast.id;
    let notice = &toast.notice;
    let full = full_text(notice);
    let fit = fit(&notice.title, notice.detail.as_deref(), words_width(notice));
    let mut words = col()
        .grow(1.0)
        .shrink()
        .min_w(0.0)
        .gap(2.0)
        .a11y_hidden()
        .child(title_line(notice, fit.title_lines, &full, u));
    if let Some(detail) = &notice.detail {
        words = words.child(
            text(detail.clone())
                .font(BODY)
                .color(u.muted)
                .wrap_anywhere()
                .max_lines(fit.detail_lines)
                .help_when_truncated(full.clone())
                .w_full(),
        );
    }
    let mut body = row()
        .w_full()
        .gap(10.0)
        .items_start()
        .child(el().size(16.0, 20.0).center().child(kind_icon(notice.kind, u)))
        .child(words);
    // One format: at most one action (Undo, Show, Retry) beside the words. An error or a
    // warning puts its action and Send feedback on a row of their own under the words, so long
    // titles keep room; so does Details, when the words are cut.
    let error = notice.kind.loud();
    let mut buttons: Vec<El> = Vec::new();
    if let Some(label) = notice.action_label() {
        buttons.push(
            text_button(u, &format!("toast-{id}-action"), label)
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |st: &mut State, tx: &mut Tx| run_action(st, id, tx)),
        );
    }
    if notice.has_undo() {
        buttons.push(
            text_button(u, &format!("toast-{id}-undo"), tr("Undo"))
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |st: &mut State, tx: &mut Tx| run_undo(st, id, tx)),
        );
    }
    if !error {
        for button in buttons.drain(..) {
            body = body.child(button.mt(-2.0));
        }
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
            .label(tr("Dismiss"))
            .cursor(rune::Cursor::Pointer)
            .on_press_tx(move |st: &mut State, tx: &mut Tx| dismiss(st, id, tx))
            .child(rune::icon(&icons::X, 14.0).color(u.faint).a11y_hidden()),
    );
    if fit.clamped {
        let title = notice.title.clone();
        let whole = full.clone();
        buttons.push(
            text_button(u, &format!("toast-{id}-details"), tr("Details"))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    st.notices.details = Some((title.clone(), whole.clone()));
                }),
        );
    }
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
            crate::ui::soft(Color::BLACK.alpha(if u.light { 0.14 } else { 0.45 })),
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
        // In from the right, out in place; survivors close the gap on `base`. The stack is an
        // overlay with no page content in it, so this is the one layout spring allowed.
        .transition(crate::motion::from_right())
        .motion(crate::motion::base())
        .child(body);
    let mut footer = row().key(format!("toast-{id}-buttons")).w_full().pl(18.0).pt(6.0).gap(4.0).center_y().children(buttons);
    if error {
        let (title, detail) = (notice.title.clone(), notice.detail.clone().unwrap_or_default());
        let warning = notice.kind == Kind::Warning;
        let feedback = feedback::button_look(u, &format!("toast-{id}-feedback"))
            .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                dismiss(st, id, tx);
                feedback::open_as(st, warning, &title, &detail);
            });
        footer = footer.child(feedback);
        card = card.child(footer);
    } else if fit.clamped {
        card = card.child(footer);
    }
    if let Some(detail) = &notice.detail {
        card = card.description(detail.clone());
    }
    card
}

/// The title, and a quiet repeat count when the same thing was pushed more than once.
fn title_line<C>(notice: &Notice<C>, lines: usize, full: &str, u: &Ui) -> El {
    let title = text(notice.title.clone())
        .font(BODY_MED)
        .color(u.ink)
        .wrap_anywhere()
        .max_lines(lines)
        .help_when_truncated(full.to_string());
    if notice.repeats < 2 {
        return title.w_full();
    }
    row()
        .w_full()
        .gap(6.0)
        .items_start()
        .child(title.grow(1.0).shrink().min_w(0.0))
        .child(
            text(format!("\u{00d7}{}", notice.repeats))
                .font(SMALL)
                .color(u.faint)
                .mt(2.0),
        )
}

/// A compact text button for a toast: 24 points tall, ink text, a hover fill. A long label is
/// cut at [`BUTTON_MAX_W`] with the whole label in its tooltip.
fn text_button(u: &Ui, key: &str, label: &str) -> El {
    row()
        .key(key.to_string())
        .h(24.0)
        .px(8.0)
        .max_w(BUTTON_MAX_W)
        .center()
        .radius(crate::ui::r(6.0))
        .hover(u.hover)
        .press(u.over.alpha(0.08))
        .focusable(true)
        .role("button")
        .label(label)
        .child(
            text(label.to_string())
                .font(BODY_MED)
                .color(u.ink)
                .truncate()
                .help_when_truncated(label.to_string())
                .a11y_hidden(),
        )
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

/// `notify-feedback`: an error pushed three times (one toast, a count) and a warning, both with
/// Send feedback.
pub fn scene_feedback(s: &mut State) {
    crate::app::scene_demo_installed(s);
    for _ in 0..3 {
        push(
            s,
            Notice::error("Could not write autoexec.cfg").detail("Permission denied. Close the game and try again."),
        );
    }
    push(
        s,
        Notice::warning("2 mods were not installed").detail("Install them from Browse, then apply the profile again."),
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

/// The raw stash failure a second launch hit on Windows while Deadlock was starting (0.1.31).
pub const SCENE_LAUNCH_BUSY_RAW: &str = "Could not stash D:\\SteamLibrary\\steamapps\\common\\Deadlock\\game\\citadel\\addons\\pak04_dir.vpk into C:\\Users\\Player\\AppData\\Roaming\\Sigil\\vanilla-stash\\20261121-183344\\pak04_dir.vpk: Nothing was moved. The process cannot access the file because it is being used by another process. (os error 32)";

/// `toast-launch-busy`: Launch vanilla pressed while Deadlock was still starting. The launch
/// is still under way (overlay hidden), so the sidebar's launch buttons are off; the toast is
/// what the stash failure now says.
pub fn scene_launch_busy(s: &mut State) {
    crate::screens::launch::scene_page(s);
    crate::screens::launch_progress::stage_waiting(s, false);
    if let Some(notice) = crate::screens::launch_progress::vanilla_held_notice(SCENE_LAUNCH_BUSY_RAW) {
        push(s, notice);
    }
}

/// A 300 character sentence, for the overflow guards.
pub const SCENE_LONG_SENTENCE: &str = "Sigil could not finish applying the profile because three of the mods in it were changed on disk after the last scan, one of them now ships files for a hero folder that moved in the latest game update, and the addon folder ran out of free slots before the rest of the profile could be put in place for you";
/// A 180 character Windows path with no spaces.
pub const SCENE_LONG_PATH: &str = "D:\\SteamLibrary\\steamapps\\common\\Deadlock\\game\\citadel\\addons\\sigil-addons10\\pak990_dir_backup_before_profile_switch_2026-11-21T18-33-44Z_from_GameBanana_mod_612345_v3.vpk";
/// The trash error with a list of failures, one per line.
pub const SCENE_TRASH_LIST: &str = "Moved 0 to the trash; 1 failed (04: D:\\Downloads\\Steam\\steamapps\\common\\Deadlock\\game\\citadel\\addons\\pak04_dir.vpk could not be moved: Deadlock or another program has it open: close it and try again.)\n05: D:\\Downloads\\Steam\\steamapps\\common\\Deadlock\\game\\citadel\\addons\\pak05_dir.vpk could not be moved\n06: access denied";

/// The trash failure as Installed now words it: the count, then each slot's reason.
pub const SCENE_TRASH_SPLIT: &str = "Moved 0 to the trash; 1 failed. Slot 04: D:\\Downloads\\Steam\\steamapps\\common\\Deadlock\\game\\citadel\\addons\\pak04_dir.vpk could not be moved: Deadlock or another program has it open: close it and try again.";

/// Every synthetic worst case, as notices, for the scenes and the overflow test.
pub fn worst_cases() -> Vec<Notice<State>> {
    vec![
        Notice::error(SCENE_LONG_SENTENCE),
        Notice::error("Could not apply the profile").detail(SCENE_LONG_SENTENCE),
        Notice::warning(SCENE_LONG_PATH).detail(SCENE_LONG_PATH),
        from_status(SCENE_TRASH_LIST),
        from_status(SCENE_TRASH_SPLIT),
        from_status(SCENE_LAUNCH_BUSY_RAW),
        Notice::success(SCENE_LONG_PATH).undo(|_st: &mut State| {}),
        Notice::info("Installed Ashfall in slot 03").action(
            "Open the folder where Sigil keeps every backup it made",
            |_st: &mut State| {},
        ),
    ]
}

/// `toast-long-error`: a 300 character error and the trash failure list.
pub fn scene_long_error(s: &mut State) {
    crate::app::scene_demo_installed(s);
    push(s, Notice::error("Could not apply the profile").detail(SCENE_LONG_SENTENCE));
    push(s, from_status(SCENE_TRASH_SPLIT));
}

/// `toast-path`: a title and a detail that are one long Windows path, and an Undo.
pub fn scene_path(s: &mut State) {
    crate::app::scene_demo_installed(s);
    push(s, Notice::warning(SCENE_LONG_PATH).detail(SCENE_LONG_PATH));
    push(s, Notice::success(SCENE_LONG_PATH).undo(|_st: &mut State| {}));
}

/// `toast-stack`: five toasts: three on screen, the rest counted as "+2 more".
pub fn scene_toast_stack(s: &mut State) {
    crate::app::scene_demo_installed(s);
    push(s, from_status(SCENE_LAUNCH_BUSY_RAW));
    push(s, Notice::success("Installed Ashfall in slot 03"));
    push(s, Notice::error(SCENE_LONG_SENTENCE));
    push(s, Notice::info("Checked 12 mods. All up to date"));
    push(s, Notice::warning("2 mods were not installed"));
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
    fn errors_stay_until_closed_and_buttons_get_time() {
        let mut n = N::default();
        assert!(push(&mut n, Notice::error("Broke"), 0.0).is_empty());
        assert_eq!(push(&mut n, Notice::warning("Careful"), 0.0), vec![WARNING_MIN]);
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
    fn one_title_is_one_toast_and_the_louder_kind_wins() {
        let mut n = N::default();
        push(&mut n, Notice::warning("Disk full").detail("Free some space"), 0.0);
        push(&mut n, Notice::warning("Disk full.").detail("Free some space"), 1.0);
        assert_eq!(n.len(), 1);
        // Two callers word it differently: still one toast, the louder kind, the newer detail.
        push(&mut n, Notice::error("disk full").detail("No space left on device"), 1.0);
        push(&mut n, Notice::info("Disk full"), 1.0);
        assert_eq!(n.len(), 1);
        let toast = n.get(1).unwrap();
        assert_eq!(toast.notice.kind, Kind::Error);
        assert_eq!(toast.notice.detail.as_deref(), Some("No space left on device"));
        // Now an error, it no longer leaves by itself.
        assert!(n.tick(1_000.0).is_empty());
        assert!(n.get(1).unwrap().visible());
    }

    #[test]
    fn errors_and_warnings_offer_feedback_and_the_rest_do_not() {
        assert!(Kind::Error.loud() && Kind::Warning.loud());
        assert!(!Kind::Info.loud() && !Kind::Success.loud());
        // A status line classified as an error or a warning gets the same.
        assert!(from_status::<Log>("Repair failed: permission denied").kind.loud());
        assert!(from_status::<Log>("Find the game folder first.").kind.loud());
        // Warnings stay long enough to read and press, even with few words.
        assert!(Notice::<Log>::warning("Hm").duration().unwrap() >= WARNING_MIN);
        assert!(WARNING_MIN > Kind::Info.duration().unwrap() * 4.0);
    }

    #[test]
    fn repeats_collapse_into_one_toast_with_a_count() {
        let mut n = N::default();
        for at in 0..4 {
            push(&mut n, Notice::error("Could not write autoexec.cfg"), at as f64);
        }
        assert_eq!(n.len(), 1);
        assert_eq!(n.get(1).unwrap().notice.repeats, 4);
        // A different title is its own toast with its own count.
        push(&mut n, Notice::warning("Disk almost full"), 5.0);
        assert_eq!(n.len(), 2);
        assert_eq!(n.get(2).unwrap().notice.repeats, 1);
        // A dismissed toast starts over.
        n.step(Event::Dismiss { id: 1, now: 6.0 });
        n.tick(6.0 + LEAVE);
        push(&mut n, Notice::error("Could not write autoexec.cfg"), 7.0);
        assert_eq!(n.get(3).unwrap().notice.repeats, 1);
    }

    #[test]
    fn errors_are_sticky_through_hover_and_close_on_dismiss() {
        let mut n = N::default();
        push(&mut n, Notice::error("Could not write"), 0.0);
        assert!(wakes(n.tick(3_600.0)).is_empty());
        n.step(Event::Hover { id: 1, inside: true, now: 10.0 });
        assert!(wakes(n.step(Event::Hover { id: 1, inside: false, now: 20.0 })).is_empty());
        assert!(wakes(n.tick(7_200.0)).is_empty());
        assert!(n.get(1).unwrap().visible());
        let leave = wakes(n.step(Event::Dismiss { id: 1, now: 30.0 }));
        assert_eq!(leave, vec![30.0 + LEAVE]);
        n.tick(30.0 + LEAVE);
        assert!(n.is_empty());
    }

    #[test]
    fn long_words_get_reading_time() {
        let words = "word ".repeat(60);
        // 60 words at 200 a minute: 18 s, above the info and warning minimums.
        let info: Notice<Log> = Notice::info("Read me").detail(words.clone());
        assert!((info.duration().unwrap() - 62.0 * PER_WORD).abs() < 1e-9);
        let warning: Notice<Log> = Notice::warning("Short");
        assert_eq!(warning.duration(), Some(WARNING_MIN));
        let short: Notice<Log> = Notice::info("Short");
        assert_eq!(short.duration(), Some(4.0));
    }

    #[test]
    fn titles_share_one_format_and_details_do_not_repeat_them() {
        assert_eq!(tidy_title("Saved."), "Saved");
        assert_eq!(tidy_title("Loading..."), "Loading...");
        assert_eq!(tidy_title("turned  off slot 03."), "Turned off slot 03");
        assert_eq!(tidy_title("pak05_dir.vpk is missing"), "pak05_dir.vpk is missing");
        let same: Notice<Log> = Notice::error("Install failed").detail("Install failed.");
        assert!(same.detail.is_none());
        let prefixed: Notice<Log> = Notice::error("Install failed").detail("Install failed: disk full");
        assert_eq!(prefixed.detail.as_deref(), Some("disk full"));
        let long: Notice<Log> = Notice::error("x").detail("word ".repeat(100));
        let cut = fit(&long.title, long.detail.as_deref(), words_width(&long));
        assert!(cut.clamped && cut.title_lines + cut.detail_lines <= MAX_LINES + 1);
        let short: Notice<Log> = Notice::error("Install failed").detail("Disk full.");
        assert!(!fit(&short.title, short.detail.as_deref(), words_width(&short)).clamped);
    }

    #[test]
    fn a_status_line_in_the_same_frame_as_a_push_is_dropped_unless_louder() {
        let mut n = N::default();
        let line = "Installed Ashfall. Slot 03. It loads next start.";
        assert!(n.status_is_news(line, Kind::Success));
        n.note_push(&Notice::success("Installed Ashfall").detail("Slot 03"));
        assert!(!n.status_is_news(line, Kind::Success));
        assert!(!n.status_is_news(line, Kind::Info));
        // Louder, or about something else: still news.
        assert!(n.status_is_news(line, Kind::Warning));
        assert!(n.status_is_news("Starting Deadlock with 4 mods on.", Kind::Info));
        // A raw error under a friendlier title.
        n.note_push(&Notice::error("Ashfall was not installed").detail("Download failed: timed out"));
        assert!(!n.status_is_news("Download failed: timed out", Kind::Error));
    }

    #[test]
    fn a_late_undo_joins_the_toast_pushed_this_frame() {
        let mut n = N::default();
        // Nothing pushed this frame: no toast to join.
        assert!(!n.undo_on_last_push(|log: &mut Log| log.push("undo")));
        n.note_push(&Notice::success("Moved 1 mod to the trash"));
        n.step(Event::Push { notice: Notice::success("Moved 1 mod to the trash"), now: 0.0 });
        assert!(n.undo_on_last_push(|log: &mut Log| log.push("undo")));
        assert_eq!(n.len(), 1);
        assert!(n.get(1).unwrap().notice.has_undo());
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
        assert_eq!(wake, vec![1.0 + LEAVE + Kind::Info.duration().unwrap()]);
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
            "Game found, but gameinfo.gi does not list citadel/addons"
        );
        assert_eq!(notice.detail.as_deref(), Some("The game will ignore every addon."));
        // Short lines and lines without a sentence break stay whole.
        let short: Notice<()> = from_status("Saved. Done.");
        assert_eq!(short.title, "Saved. Done");
        assert!(short.detail.is_none());
        let decimal: Notice<()> = from_status(
            "The scan took 1.5 s and read every addon slot in the folder without any problem at all",
        );
        assert!(decimal.detail.is_none());
    }
}
