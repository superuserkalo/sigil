//! A picture per mod in the Locker (Grimoire's Locker image per mod).
//!
//! - Grimoire: card, thumbnail and background art per mod, from the mod's gallery or an upload.
//! - Deadlock Mod Manager: the mod's GameBanana image on its skin cards.
//! - Sigil: every Locker row shows the picture Installed shows for that slot (its own image from
//!   the name and picture editor, else its GameBanana preview), and a Picture button opens that
//!   editor. One picture per mod, shared by both pages.

use rune::{El, SurfaceFit, el};

use crate::app::State;
use crate::icons;
use crate::ui::Ui;

/// Fetch the GameBanana previews of the mods the Locker shows, once each, while it is open.
/// Local pictures load with Installed's notes.
pub fn poll(st: &mut State) -> bool {
    if st.screen != crate::model::Screen::Locker || st.demo || rune::is_headless() || !st.locker.loaded {
        return false;
    }
    let wanted: Vec<String> = st
        .locker
        .mods
        .iter()
        .filter_map(|m| crate::screens::installed::preview_key_for_slot(st, m.slot))
        .filter(|key| key.starts_with("http") && !st.thumbnails.contains_key(key) && !st.locker.art_asked.contains(key))
        .collect();
    if wanted.is_empty() {
        return false;
    }
    st.locker.art_asked.extend(wanted.iter().cloned());
    crate::data::queue_images(st, wanted, false);
    false
}

/// The row's picture: 40 points square, or a quiet box until it decodes.
pub fn thumb(s: &State, u: &Ui, slot: u16) -> El {
    let base = el().size(40.0, 40.0).radius(crate::ui::r(6.0)).a11y_hidden();
    match crate::screens::installed::preview_key_for_slot(s, slot).and_then(|key| s.thumbnails.get(&key)) {
        Some(pixels) => base.pixels_fit(pixels, SurfaceFit::Fill),
        None => base
            .fill(u.well)
            .center()
            .child(rune::icon(&icons::PACKAGE, 16.0).color(u.faint.alpha(0.6))),
    }
}

