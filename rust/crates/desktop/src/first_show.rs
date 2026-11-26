//! A start the player never sees: Windows starting Sigil at sign-in with "Start minimized" on.
//! Such a launch makes no GPU device and decodes no pictures until the window is first shown.
//!
//! - With the tray up (Minimize to tray or Close to tray on), Sigil starts with no window at all
//!   (`App::windowless`): the tray's Open makes it, CPU first like any launch.
//! - Without the tray, on Windows, the window opens minimized on the taskbar
//!   (`App::open_minimized`): Rune paints its first frame on the CPU while it is hidden and makes
//!   the GPU only once the window is in sight.
//! - Elsewhere, and with the transparent sidebar on (a see-through window needs the GPU from its
//!   first frame), the window opens as before and minimizes on the first poll.
//!
//! Pictures wait behind [`Gate`]: until it opens, `data::drain_thumbnails` hands nothing to the
//! image cache, so nothing is decoded and nothing reaches an atlas. The updater counts a launch
//! as started only once a frame was drawn (`drew_frame`).

use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

/// How this launch opens its window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    /// A normal launch: the window opens and shows.
    Shown,
    /// Started minimized with the tray up: no window until the tray opens it.
    Windowless,
    /// Started minimized without the tray, on Windows: on the taskbar, minimized from the start.
    Minimized,
    /// Started minimized where neither applies: the window shows, then minimizes on the first
    /// poll, as before.
    MinimizeAfterShow,
}

/// What the launch knows before the window exists.
#[derive(Clone, Copy, Debug, Default)]
pub struct Facts {
    /// `--minimized` from the sign-in entry, with "Start minimized" still on.
    pub minimized: bool,
    /// The tray icon is up.
    pub tray: bool,
    /// The window is see-through (Transparent sidebar), so Rune makes its GPU at once.
    pub glass: bool,
    /// An update waits for its first drawn frame before it counts as good: this launch shows a
    /// window so it can draw one (a launch that never draws would be rolled back next time).
    pub update_unconfirmed: bool,
    /// Windows, where Rune can open a window minimized.
    pub windows: bool,
}

/// How a launch with these facts opens.
pub fn decide(f: Facts) -> Start {
    if !f.minimized {
        return Start::Shown;
    }
    if f.tray && !f.update_unconfirmed {
        return Start::Windowless;
    }
    if f.windows && !f.glass {
        return Start::Minimized;
    }
    Start::MinimizeAfterShow
}

/// Whether pictures may go to the image cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    Open,
    /// Held until `frames` more frames are drawn, or the window is reported visible.
    Held { frames: u8 },
}

/// What moves the gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Rune drew a frame (`App::on_frame_drawn`).
    FrameDrawn,
    /// The window went off screen or came back (`App::on_visibility`).
    Visible(bool),
}

impl Gate {
    /// The gate a launch starts with.
    pub fn at_start(start: Start) -> Gate {
        match start {
            Start::Shown | Start::MinimizeAfterShow => Gate::Open,
            // No window until the player opens one: its first frame is in sight.
            Start::Windowless => Gate::Held { frames: 1 },
            // The first frame is painted while the window is hidden; the next one is drawn
            // only once it is restored (a minimized window draws nothing).
            Start::Minimized => Gate::Held { frames: 2 },
        }
    }

    pub fn step(self, event: Event) -> Gate {
        match (self, event) {
            (Gate::Open, _) => Gate::Open,
            (Gate::Held { .. }, Event::Visible(true)) => Gate::Open,
            (Gate::Held { .. }, Event::Visible(false)) => self,
            (Gate::Held { frames }, Event::FrameDrawn) if frames <= 1 => Gate::Open,
            (Gate::Held { frames }, Event::FrameDrawn) => Gate::Held { frames: frames - 1 },
        }
    }

    pub fn open(self) -> bool {
        self == Gate::Open
    }
}

const OPEN: u8 = 0;
/// `Gate::Held { frames }` is stored as `frames`; `OPEN` is 0, so a held gate is never 0.
static GATE: AtomicU8 = AtomicU8::new(OPEN);
static START: OnceLock<Start> = OnceLock::new();
static FRAMES: AtomicU32 = AtomicU32::new(0);
static WAKE: Mutex<Option<Box<dyn Fn() + Send>>> = Mutex::new(None);

fn load() -> Gate {
    match GATE.load(Ordering::Acquire) {
        OPEN => Gate::Open,
        frames => Gate::Held { frames },
    }
}

fn store(gate: Gate) {
    GATE.store(
        match gate {
            Gate::Open => OPEN,
            Gate::Held { frames } => frames.max(1),
        },
        Ordering::Release,
    );
}

/// From `main`, before the app runs: how this launch opens. Returns it.
pub fn begin(facts: Facts) -> Start {
    let start = decide(facts);
    let _ = START.set(start);
    store(Gate::at_start(start));
    start
}

/// How this launch opened (`Start::Shown` when `begin` never ran: scenes, tests).
pub fn start() -> Start {
    START.get().copied().unwrap_or(Start::Shown)
}

/// Wakes the app's poll once the gate opens, so held pictures go to the cache.
pub fn set_waker(wake: impl Fn() + Send + 'static) {
    if let Ok(mut slot) = WAKE.lock() {
        *slot = Some(Box::new(wake));
    }
}

fn feed(event: Event) {
    let before = load();
    let after = before.step(event);
    if after != before {
        store(after);
        if after.open()
            && let Ok(slot) = WAKE.lock()
            && let Some(wake) = slot.as_ref()
        {
            wake();
        }
    }
}

/// `App::on_frame_drawn`.
pub fn frame_drawn() {
    FRAMES.fetch_add(1, Ordering::Relaxed);
    feed(Event::FrameDrawn);
}

/// `App::on_visibility`.
pub fn visible(visible: bool) {
    feed(Event::Visible(visible));
}

/// Whether pictures may be decoded: the window has been shown, or this launch never held them.
pub fn pictures_allowed() -> bool {
    load().open()
}

/// Whether this launch has drawn a frame (`updater`'s "started OK").
pub fn drew_frame() -> bool {
    FRAMES.load(Ordering::Relaxed) > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> Facts {
        Facts { minimized: true, tray: false, glass: false, update_unconfirmed: false, windows: true }
    }

    #[test]
    fn a_normal_launch_shows_and_holds_nothing() {
        let start = decide(Facts { minimized: false, tray: true, ..facts() });
        assert_eq!(start, Start::Shown);
        assert!(Gate::at_start(start).open());
    }

    #[test]
    fn a_minimized_start_with_the_tray_opens_no_window() {
        assert_eq!(decide(Facts { tray: true, ..facts() }), Start::Windowless);
        // On any platform with a tray.
        assert_eq!(decide(Facts { tray: true, windows: false, ..facts() }), Start::Windowless);
        // Glass needs a GPU only once a window exists.
        assert_eq!(decide(Facts { tray: true, glass: true, ..facts() }), Start::Windowless);
    }

    #[test]
    fn an_update_that_has_not_drawn_yet_always_gets_a_window() {
        // Windowless, it would never draw, and the next launch would roll the update back.
        assert_eq!(decide(Facts { tray: true, update_unconfirmed: true, ..facts() }), Start::Minimized);
        assert_eq!(
            decide(Facts { tray: true, update_unconfirmed: true, windows: false, ..facts() }),
            Start::MinimizeAfterShow
        );
    }

    #[test]
    fn without_the_tray_windows_opens_minimized_unless_the_window_is_glass() {
        assert_eq!(decide(facts()), Start::Minimized);
        assert_eq!(decide(Facts { glass: true, ..facts() }), Start::MinimizeAfterShow);
        assert_eq!(decide(Facts { windows: false, ..facts() }), Start::MinimizeAfterShow);
        assert!(Gate::at_start(Start::MinimizeAfterShow).open(), "that window is shown once, as before");
    }

    #[test]
    fn a_windowless_start_holds_pictures_until_the_first_frame() {
        let gate = Gate::at_start(Start::Windowless);
        assert!(!gate.open());
        assert!(!gate.step(Event::Visible(false)).open());
        assert!(gate.step(Event::FrameDrawn).open());
    }

    #[test]
    fn a_minimized_start_holds_pictures_past_its_hidden_first_frame() {
        let gate = Gate::at_start(Start::Minimized);
        // The frame painted while hidden, then the window minimized out of sight.
        let gate = gate.step(Event::FrameDrawn).step(Event::Visible(false));
        assert!(!gate.open(), "a frame drawn hidden is not a frame seen");
        // Restored: either the visibility change or the next frame opens it.
        assert!(gate.step(Event::Visible(true)).open());
        assert!(gate.step(Event::FrameDrawn).open());
        // Once open it stays open.
        assert!(gate.step(Event::FrameDrawn).step(Event::Visible(false)).open());
    }
}
