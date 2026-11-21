//! Mature content extras: blur strength, reveal on hover, an always-visible 18+ tag, showing
//! unclassified items while hiding mature ones, and remembering what you revealed. Browse
//! reads them through the small functions here.

use rune::El;

use super::i18n::{fill, tr};
use super::options;
use super::{button, detail, disabled, segmented, setting_row, toggle_row, trailing};
use crate::app::State;
use crate::prefs::Mature;
use crate::ui::{ActionKind, Ui};

/// Blur radius for a card image.
pub fn card_blur() -> f32 {
    options::get().blur as f32
}

/// Blur radius for the large preview, a little stronger than the card.
pub fn lightbox_blur() -> f32 {
    options::get().blur as f32 * 1.55
}

/// True when the user revealed this mod before and Sigil is asked to remember it.
pub fn remembered(id: u64) -> bool {
    let o = options::get();
    o.remember_reveals && o.revealed.contains(&id)
}

/// True when hovering a card shows its image.
pub fn reveal_on_hover() -> bool {
    options::get().reveal_on_hover
}

/// True when mature cards always show their 18+ tag.
pub fn always_tag() -> bool {
    options::get().always_tag
}

/// Record a reveal from a card or the details.
pub fn revealed(st: &mut State, id: u64) {
    st.browse_view.revealed.insert(id);
    if options::get().remember_reveals {
        options::edit(st, |o| o.remember_reveal(id));
    }
}

/// An item from a source with no mature classification: hidden with mature content unless
/// "Show unclassified items" is on (DMM's Show likely NSFW).
pub fn hide_unclassified(mature: Mature) -> bool {
    mature == Mature::Hide && !options::get().likely_nsfw
}

pub fn rows(s: &State, u: &Ui) -> Vec<El> {
    let o = options::get();
    let blurring = s.prefs.mature == Mature::Blur;
    let blurs: Vec<(u32, &str)> = vec![(8, "Light"), (18, "Medium"), (28, "Strong"), (40, "Full")];
    let saved = o.revealed.len();
    let mature: Vec<(Mature, &str)> = Mature::ALL.iter().map(|m| (*m, m.label())).collect();
    vec![
        // The section's own switch first: the heading names it, so the row has no line.
        setting_row(
            u,
            "settings-mature",
            "Mature content",
            detail(u, ""),
            segmented(u, "settings-mature-pick", &mature, s.prefs.mature, |st, m| {
                crate::prefs::edit(st, |p| p.mature = m)
            }),
        ),
        disabled(
            setting_row(
                u,
                "settings-blur-strength",
                "Blur strength",
                detail(u, ""),
                segmented(u, "settings-blur-pick", &blurs, o.blur, |st, b| options::edit(st, |o| o.blur = b)),
            ),
            !blurring,
        ),
        disabled(
            toggle_row(
                u,
                "settings-reveal-hover",
                "Reveal on hover",
                "Pointing at a blurred card shows its image.",
                o.reveal_on_hover,
                |st| options::edit(st, |o| o.reveal_on_hover = !o.reveal_on_hover),
            ),
            !blurring,
        ),
        toggle_row(
            u,
            "settings-always-tag",
            "Always show the 18+ tag",
            "Mature cards keep their tag, also when they are not blurred.",
            o.always_tag,
            |st| options::edit(st, |o| o.always_tag = !o.always_tag),
        ),
        disabled(
            toggle_row(
                u,
                "settings-likely-nsfw",
                "Show unclassified items",
                "While mature content is hidden, still show source mods that do not say whether they are mature.",
                o.likely_nsfw,
                |st| options::edit(st, |o| o.likely_nsfw = !o.likely_nsfw),
            ),
            s.prefs.mature != Mature::Hide,
        ),
        setting_row(
            u,
            "settings-remember-reveals",
            "Remember what you reveal",
            detail(
                u,
                &if saved == 0 {
                    tr("A revealed mod stays revealed after Sigil restarts.").to_string()
                } else {
                    fill(tr("{} mods remembered."), &[&saved.to_string()])
                },
            ),
            trailing(vec![
                disabled(
                    button(u, "settings-reveals-clear", "Clear", ActionKind::Ghost).on_press(|st: &mut State| {
                        options::edit(st, |o| o.revealed.clear());
                        st.browse_view.revealed.clear();
                        super::say(st, tr("Cleared."), false);
                    }),
                    saved == 0,
                ),
                u.toggle(o.remember_reveals)
                    .key("settings-remember-reveals-switch")
                    .label(tr("Remember what you reveal"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| options::edit(st, |o| o.remember_reveals = !o.remember_reveals)),
            ]),
        ),
    ]
}
