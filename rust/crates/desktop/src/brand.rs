//! The Sigil mark and wordmark: the sidebar's header and the app icon.
//!
//! The cut-outs in `assets/brand` come from the logo (`logo-source.png`): light ink for dark
//! themes, dark ink for light ones, 112 px tall so the header stays sharp at 2x. The app icon is
//! `sigil.ico` built into Sigil.exe (`build.rs`) and `app-icon.png` for the macOS Dock.

use rune::{El, SurfaceFit, el, row, text};

use crate::app::State;
use crate::ui::{Ui, WORDMARK};

const MARK_LIGHT: &[u8] = include_bytes!("../assets/brand/mark-light.png");
const MARK_DARK: &[u8] = include_bytes!("../assets/brand/mark-dark.png");
const WORD_LIGHT: &[u8] = include_bytes!("../assets/brand/word-light.png");
const WORD_DARK: &[u8] = include_bytes!("../assets/brand/word-dark.png");
#[cfg(target_os = "macos")]
const APP_ICON: &[u8] = include_bytes!("../assets/brand/app-icon.png");

const MARK_LIGHT_KEY: &str = "sigil:brand/mark-light";
const MARK_DARK_KEY: &str = "sigil:brand/mark-dark";
const WORD_LIGHT_KEY: &str = "sigil:brand/word-light";
const WORD_DARK_KEY: &str = "sigil:brand/word-dark";

/// The wordmark's width over its height (248 by 112).
const WORD_ASPECT: f32 = 248.0 / 112.0;
const MARK_PT: f32 = 24.0;
/// The wordmark image's height; its letters read about 16 points, beside 13 point nav labels.
const WORD_PT: f32 = 18.0;
/// Mark to wordmark: the 24 point mark is centred on the nav icon axis (x 26), so 6 points put
/// the wordmark on the nav label edge (x 44).
const GAP: f32 = 6.0;

/// The app icon for windows and the Dock. Call before `run`.
pub fn install_app_icon() {
    if rune::is_headless() {
        return;
    }
    // Windows: the icon resource `build.rs` links in (Explorer shows the same one).
    #[cfg(target_os = "windows")]
    rune::set_app_icon_resource(1);
    #[cfg(target_os = "macos")]
    let _ = rune::set_app_icon(APP_ICON);
}

/// Queue the header's images; `setup` decodes them with the other startup images. About 50 KB
/// decoded each, kept for the whole run.
pub fn load(st: &mut State) {
    for (key, bytes) in [
        (MARK_LIGHT_KEY, MARK_LIGHT),
        (MARK_DARK_KEY, MARK_DARK),
        (WORD_LIGHT_KEY, WORD_LIGHT),
        (WORD_DARK_KEY, WORD_DARK),
    ] {
        if !st.thumbnails.contains_key(key) {
            st.pending_thumbs.push((key.to_string(), bytes.to_vec()));
        }
    }
}

/// The mark alone in the theme's ink, once decoded (the Crest themed art draws it large).
pub fn mark<'a>(s: &'a State, u: &Ui) -> Option<&'a rune::Pixels> {
    let dark_theme = (u.ink.r + u.ink.g + u.ink.b) / 3.0 > 0.5;
    s.thumbnails.get(if dark_theme { MARK_LIGHT_KEY } else { MARK_DARK_KEY })
}

/// The mark and the wordmark in the theme's ink; the plain name until they decode, in the
/// lockup's exact box so nothing moves when the images arrive. The mark spans the nav icon
/// column and the wordmark starts on the nav label edge.
///
/// Only the open sidebar shows it; the collapsed rail has no brand. On collapse the lockup
/// keeps its size and place, clipped by the brand row at the sidebar's edge, and fades out on
/// `motion::fade`; on expand it fades in the same way. Nothing in it scales or wraps.
pub fn header(s: &State, u: &Ui) -> El {
    lockup(s, u).transition(crate::motion::fade())
}

fn lockup(s: &State, u: &Ui) -> El {
    let dark_theme = (u.ink.r + u.ink.g + u.ink.b) / 3.0 > 0.5;
    let (mark, word) = if dark_theme {
        (MARK_LIGHT_KEY, WORD_LIGHT_KEY)
    } else {
        (MARK_DARK_KEY, WORD_DARK_KEY)
    };
    match (s.thumbnails.get(mark), s.thumbnails.get(word)) {
        (Some(mark), Some(word)) => row()
            .key("brand-lockup")
            .gap(GAP)
            .center_y()
            .label("Sigil")
            .child(el().key("brand-mark").size(MARK_PT, MARK_PT).pixels_fit(mark, SurfaceFit::Fit))
            .child(
                el().key("brand-word")
                    .size(WORD_PT * WORD_ASPECT, WORD_PT)
                    .pixels_fit(word, SurfaceFit::Fit)
                    .a11y_hidden(),
            ),
        _ => row()
            .key("brand-lockup")
            .size(MARK_PT + GAP + WORD_PT * WORD_ASPECT, MARK_PT)
            .center_y()
            .label("Sigil")
            .child(el().w(MARK_PT + GAP))
            .child(text("Sigil").font(WORDMARK).color(u.ink).a11y_hidden()),
    }
}

/// The lockup's fixed size: the mark, the gap and the wordmark (the sidebar jank guard reads it).
#[cfg_attr(not(test), allow(dead_code))]
pub const LOCKUP: (f32, f32) = (MARK_PT + GAP + WORD_PT * WORD_ASPECT, MARK_PT);
