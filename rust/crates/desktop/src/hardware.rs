//! Settings > Appearance > Hardware acceleration (Windows), as Discord's switch of that name.
//!
//! On (the default, `DEFAULT`), Sigil draws with the graphics card: its first frame comes from
//! Rune's CPU renderer and the GPU takes over in the background, as before. Off, Rune draws every
//! frame on the CPU (`App::hardware_acceleration(false)`): no D3D12 device, swap chain or GPU
//! copy of a picture exists, so Sigil holds no graphics memory. What only the GPU draws is
//! replaced:
//!
//! - the transparent sidebar is off (a see-through window needs the GPU);
//! - a hidden mature picture is blurred once, when it is decoded (`want_blurred`), instead of by
//!   a blurred layer each frame;
//! - the Personalize background shows unblurred;
//! - the 3D viewer still makes its own GPU device, only while a view is open.
//!
//! The switch applies at the next launch (Restart in the row). Rune also stays on the CPU by
//! itself when Windows lists no hardware graphics adapter, or when the GPU failed its tries.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;

/// The switch's default: on. CPU frame times for long Browse scrolls are not measured on Windows
/// yet; flip this once they are and they stay under a frame.
pub const DEFAULT: bool = true;

static LAUNCH: OnceLock<bool> = OnceLock::new();

/// From `main`: whether this launch draws with the GPU. Only Windows has the CPU-only mode; a
/// headless run (snapshots, scripts) renders as it always does.
pub fn begin(pref: bool) -> bool {
    let on = pref || !supported() || rune::is_headless();
    let _ = LAUNCH.set(on);
    on
}

/// Whether the switch exists on this platform.
pub fn supported() -> bool {
    cfg!(target_os = "windows")
}

/// Whether this launch draws on the CPU only (the switch was off when Sigil started).
pub fn cpu_only() -> bool {
    LAUNCH.get().is_some_and(|on| !on)
}

/// Whether this launch draws with the GPU, as the row compares it with the saved switch.
pub fn launched_on() -> bool {
    !cpu_only()
}

/// The most blurred copies kept (handles only: their pixels live in Rune's image cache and
/// atlas, which bound them in bytes).
const BLURRED_KEPT: usize = 512;

thread_local! {
    /// Pictures a view drew hidden this frame with no blurred copy yet; `drain_thumbnails` asks
    /// the image cache for them.
    static WANTED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// Each hidden picture's copy decoded blurred, by its URL.
    static BLURRED: RefCell<HashMap<String, rune::Image>> = RefCell::new(HashMap::new());
}

/// A hidden picture's blurred copy, as a view finds it.
pub enum Blurred {
    /// Not asked for yet: the view asks (`want_blurred`) and shows its placeholder.
    Missing,
    /// It could not be decoded (the file left the disk cache, a format that can't be blurred):
    /// the placeholder stays, and nothing asks again, so no decode repeats every frame.
    Failed,
    /// Asked for: draw it once ready, else prefetch it and show the placeholder.
    Copy(rune::Pixels),
}

/// `url`'s blurred copy.
pub fn blurred(url: &str) -> Blurred {
    BLURRED.with_borrow(|m| match m.get(url) {
        None => Blurred::Missing,
        Some(image) if image.error().is_some() => Blurred::Failed,
        Some(image) => Blurred::Copy(image.pixels().clone()),
    })
}

/// Whether `url` has a blurred copy, asked for or failed.
pub fn has_blurred(url: &str) -> bool {
    BLURRED.with_borrow(|m| m.contains_key(url))
}

/// From the poll: `url`'s blurred copy, as the image cache gave it.
pub fn keep_blurred(url: String, image: rune::Image) {
    BLURRED.with_borrow_mut(|m| {
        if m.len() >= BLURRED_KEPT {
            m.clear();
        }
        m.insert(url, image);
    });
}

/// From a view, on the CPU only: `url` is drawn hidden (a mature picture under its veil) and has
/// no blurred copy yet. The poll decodes one.
pub fn want_blurred(url: &str) {
    WANTED.with_borrow_mut(|w| {
        if !w.iter().any(|u| u == url) && w.len() < 256 {
            w.push(url.to_string());
        }
    });
}

/// What views asked for since the last poll.
pub fn take_wanted() -> Vec<String> {
    WANTED.with_borrow_mut(std::mem::take)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wanted_pictures_are_asked_for_once_and_taken_once() {
        want_blurred("https://images.gamebanana.com/a.jpg");
        want_blurred("https://images.gamebanana.com/a.jpg");
        want_blurred("https://images.gamebanana.com/b.jpg");
        assert_eq!(take_wanted().len(), 2);
        assert!(take_wanted().is_empty());
        assert!(matches!(blurred("https://images.gamebanana.com/c.jpg"), Blurred::Missing));
    }
}
