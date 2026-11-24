//! Sigil's motion tokens: the only place a spring, a tween or a transition is written.
//!
//! Motion is short, paint only where it can be, and never runs at rest. Layout of anything in
//! page flow lands at once: the root of the view sets `Spring::NONE`, and a control opts in on
//! the smallest element that moves (a knob, a thumb) or with `colors`, which eases its fill
//! and text while its box still lands at once. Every token is a function so Settings >
//! Appearance > Reduce motion (or the system setting) can make all of them instant: durations
//! become zero, not shorter.
//!
//! Measured in Rune (`motion_tests`): a spring on an element eases that element's own place in
//! its parent, so `.motion(fast())` on a row in a list would slide it when a row above it
//! grows. `colors` adds `instant_changes`, which lands the box and keeps the colour spring.

use std::cell::Cell;

use rune::{Anim, Curve, Easing, El, Spring, SpringPair, Transition};

thread_local! {
    // Thread local, not process wide: the view and every element are built on the UI thread,
    // and tests that run views in parallel must not switch each other's motion.
    static REDUCED: Cell<bool> = const { Cell::new(false) };
}

/// Set once per view run from the system preference and Settings > Appearance > Reduce motion.
pub fn set_reduced(on: bool) {
    REDUCED.with(|r| r.set(on));
}

pub fn reduced() -> bool {
    REDUCED.with(|r| r.get())
}

fn spring(visual: f32) -> Spring {
    if reduced() { Spring::NONE } else { Spring::from_motion(visual, 0.0) }
}

fn tween(seconds: f32) -> Anim {
    if reduced() { Anim::INSTANT } else { Curve::tween(seconds, Easing::EASE_OUT).into() }
}

/// The pointer arrives: hover and press fills light at once. Measured: a 70 ms ease in with
/// a 140 ms fade out (the spec's first values) drew 20 to 45 percent more frames in a pointer
/// sweep than Rune's own default (at once in, 100 ms out), with no commit either way, so the
/// tokens keep that cost: the hover answers in the frame the pointer arrives.
pub fn hover_in() -> Anim {
    Anim::INSTANT
}

/// The pointer leaves: hover fills fade over 100 ms.
pub fn hover_out() -> Anim {
    tween(0.10)
}

/// The Browse billboard's pager bar: it glides from one half-second step to the next, so the
/// fill reads as one steady movement.
pub fn countdown() -> Spring {
    spring(0.5)
}

/// 90 ms: fills, colours, the switch knob, a slider settling, a drag dim.
pub fn fast() -> Spring {
    spring(0.09)
}

/// 140 ms: a sliding selection, a tab underline, toasts closing a gap.
pub fn base() -> Spring {
    spring(0.14)
}

/// 180 ms: overlays arriving, a shelf page coming in. The longest token.
pub fn enter() -> Spring {
    spring(0.18)
}

/// 110 ms: overlays leaving, a shelf page going.
pub fn exit() -> Spring {
    spring(0.11)
}

/// 80 ms: menus and popovers leaving.
pub fn exit_fast() -> Spring {
    spring(0.08)
}

/// 120 ms: a new page fading in after a sidebar press.
pub fn page() -> Spring {
    spring(0.12)
}

/// Menu and popover travel toward their anchor.
pub const RISE: f32 = 4.0;
/// Dialog and selection bar travel.
pub const LIFT: f32 = 8.0;
/// Drawer and toast travel.
pub const SLIDE: f32 = 24.0;
/// Shelf and spotlight paging travel.
pub const PAGE: f32 = 32.0;

/// Tooltip fade in and out, seconds.
pub fn tip_fade() -> (f64, f64) {
    if reduced() { (0.0, 0.0) } else { (0.09, 0.06) }
}

fn travel(x: f32, y: f32, springs: SpringPair) -> Transition {
    if reduced() {
        return Transition::none();
    }
    // Exits never travel: a leaving surface fades in place, so it cannot cross an arriving one.
    Transition::asymmetric(
        Transition::offset(x, y).combined(Transition::opacity()),
        Transition::opacity(),
    )
    .springs(springs)
}

/// A menu or popover: rises 4 points toward its anchor and fades in, leaves in 80 ms.
/// `above` for a menu that opens above its anchor (it comes down instead).
pub fn pop(above: bool) -> Transition {
    travel(0.0, if above { -RISE } else { RISE }, SpringPair::new(enter(), exit_fast()))
}

/// A dialog or the selection bar: 8 points up and a fade in, a fade out.
pub fn lift() -> Transition {
    travel(0.0, LIFT, SpringPair::new(enter(), exit()))
}

/// A drawer or toast: 24 points in from the right and a fade, a fade out.
pub fn from_right() -> Transition {
    travel(SLIDE, 0.0, SpringPair::new(enter(), exit()))
}

/// A fade in and out, no travel (dialog scrims).
pub fn fade() -> Transition {
    if reduced() {
        return Transition::none();
    }
    Transition::opacity().springs(SpringPair::new(enter(), exit()))
}

/// An image leaf arriving over its placeholder fill: it fades in over `base` (140 ms) and is gone
/// at once when it leaves, so a picture never crossfades with another. Only in a commit the
/// landing images alone cause (`app::poll`), so it never runs at rest.
pub fn image_in() -> Transition {
    if reduced() {
        return Transition::none();
    }
    Transition::opacity().springs(SpringPair::new(base(), Spring::NONE))
}

/// The 3D viewer's capture flash: Rune's own fade (0.3 s in, 0.24 s out), as it always was.
pub fn flash() -> Transition {
    if reduced() {
        return Transition::none();
    }
    Transition::opacity()
}

/// A shelf or spotlight page: in from 32 points on the side it comes from (`dir` 1 from the
/// right, -1 from the left) while fading in; the old page fades out in place.
pub fn paged(dir: i8) -> Transition {
    travel(f32::from(dir.signum()) * PAGE, 0.0, SpringPair::new(enter(), exit()))
}

/// A page body after a sidebar press, or a tab body after a tab press: the new one fades in over
/// 120 ms with no travel, and the old one is gone the same frame (its exit spring is none), so
/// one body is ever mounted. Its children do not fade on their own under it.
pub fn page_enter() -> Transition {
    if reduced() {
        return Transition::none();
    }
    Transition::opacity().springs(SpringPair::new(page(), Spring::NONE))
}

/// A full-window layer holding a scrim and a surface (a dialog, a menu with its scrim): no
/// entrance of its own, so its scrim and surface play theirs, and on close the whole layer
/// fades out in 110 ms in place. A removed element takes its children with it, so the layer's
/// exit is the only one that shows.
pub fn layer() -> Transition {
    if reduced() {
        return Transition::none();
    }
    Transition::asymmetric(Transition::identity(), Transition::opacity())
        .springs(SpringPair::new(Spring::NONE, exit()))
}

/// No entrance or exit of its own: shows whole the frame it mounts and is gone the frame it
/// leaves. Measured in Rune: its children still fade in on their own default (0.3 s) when it
/// mounts, so a new page or tab body takes `page_enter`, whose fade covers them.
pub fn cut() -> Transition {
    Transition::none()
}

/// The root of the view: layout and colour land at once unless a control below opts in, and
/// every hover and press fill follows the hover curves, anchored overlays included. Under
/// reduced motion the whole window lands every change at once, entrances and exits included:
/// a subtree whose root has `Transition::none()` still lets its children fade in on their
/// own, so cutting each transition alone would not be enough.
pub fn root(e: El) -> El {
    let e = e.motion(Spring::NONE).hover_motion_within(hover_in(), hover_out());
    if reduced() { e.instant_changes() } else { e }
}

/// A control whose fill, border and text colour ease on `fast` (selected, on, a state) while
/// its box lands at once wherever the layout puts it. For leaf controls: it also lands every
/// change inside it, so never wrap a switch or a slider with it.
pub fn colors(e: El) -> El {
    e.motion(fast()).instant_changes()
}

/// An absolutely placed part that slides inside a fixed box (a selection thumb, a knob).
pub fn slides(e: El, spring: Spring) -> El {
    e.motion(spring)
}

/// A selection thumb drawn inside the selected segment or tab of a strip whose parts differ in
/// width: on a new pick Rune flies it from the old part to the new one on `base`, without
/// measuring anything. Under reduced motion nothing is matched, so it lands.
pub fn glides(e: El, id: &str) -> El {
    if reduced() { e } else { e.matched(id.to_string()).motion(base()) }
}

/// A control that dims when it is unavailable: its opacity eases on `fast`.
pub fn dims(e: El) -> El {
    e.opacity_motion(fast())
}

/// A strip whose parts come and go with the page (the status bar): everything in it lands at
/// once, mounts and unmounts included. Colours keep their own easing.
pub fn lands(e: El) -> El {
    e.instant_changes()
}

/// A mark that is rebuilt in a new place when it moves (a slider thumb drawn in its stop's
/// cell): Rune flies it from where it was on `fast`. Under reduced motion nothing is matched,
/// so it lands.
pub fn follows(e: El, id: &str) -> El {
    if reduced() { e } else { e.matched(id.to_string()).motion(fast()) }
}
