//! Settings rows from parity wave 4 that need no new backend:
//!
//! - Release channel notes and the way back to an earlier version (DMM's notes and guide link,
//!   Grimoire's per-channel lines and its "running a preview build" note).
//! - Common launch options (DMM serves a curated list from its own API; Sigil ships its list).
//! - Vanilla launches without launch options (DMM's Vanilla Mode).
//! - Mod update badges on cards and details (DMM, on by default).

use rune::{El, Tx, col, row, text};

use super::i18n::{fill, tr};
use super::options;
use super::{setting_row, stacked_row, toggle_row};
use crate::app::State;
use crate::icons;
use crate::prefs::Channel;
use crate::ui::{ActionKind, LABEL, SMALL, Ui};

/// Where an older Sigil is downloaded from.
pub fn releases_url() -> String {
    format!("https://github.com/{}/releases", super::UPDATE_REPO)
}

/// This copy is a preview build: its version carries a pre-release part (`0.2.0-preview.1`).
pub fn running_preview() -> bool {
    env!("CARGO_PKG_VERSION").contains('-')
}

/// Under the channel picker: what the channel means, and how to go back.
pub fn channel_notes(s: &State, u: &Ui) -> El {
    let mut line = tr(match s.prefs.channel {
        Channel::Stable => "Recommended for everyday use: official releases only.",
        Channel::Preview => {
            "Test builds as soon as they are made. They can have bugs; you can switch back to Stable at any time."
        }
    })
    .to_string();
    if s.prefs.channel == Channel::Stable && running_preview() {
        line.push(' ');
        line.push_str(tr(
            "You are running a preview build. Sigil never installs an older version by itself, so download the stable release from GitHub to go back.",
        ));
    }
    let url = releases_url();
    setting_row(
        u,
        "settings-channel-notes",
        "Going back to an earlier version",
        text(line).font(SMALL).color(u.muted).wrap(),
        u.icon_action(&icons::EXTERNAL_LINK, tr("Releases"), ActionKind::Ghost)
            .key("settings-channel-releases")
            .help(tr("Every release is on GitHub. Install an older one over this copy; your mods and settings stay."))
            .cursor(rune::Cursor::Pointer)
            .on_press_tx(move |st: &mut State, tx: &mut Tx| super::tabs::open_url(st, tx, &url)),
    )
}

// --- launch options ---------------------------------------------------------------------

/// One known launch option and what it does. Static: DMM fetches its list from its own server,
/// which Sigil does not have.
pub struct Known {
    pub option: &'static str,
    pub line: &'static str,
}

pub const KNOWN: [Known; 10] = [
    Known { option: "-novid", line: "Skips the intro video." },
    Known { option: "-high", line: "Runs the game at high CPU priority." },
    Known { option: "-fullscreen", line: "Starts in exclusive full screen." },
    Known { option: "-windowed", line: "Starts in a window." },
    Known { option: "-noborder", line: "A window without a border, with -windowed." },
    Known { option: "-dx11", line: "Draws with DirectX 11." },
    Known { option: "-vulkan", line: "Draws with Vulkan." },
    Known { option: "-console", line: "Opens the console at start." },
    Known { option: "-condebug", line: "Writes the console to console.log (Discord status reads it)." },
    Known { option: "-nojoy", line: "Turns controller support off." },
];

/// The options in a launch line, split on spaces.
fn words(line: &str) -> Vec<&str> {
    line.split_whitespace().collect()
}

/// Add the option when it is missing, take it out when it is there.
pub fn toggle_option(line: &str, option: &str) -> String {
    let mut parts = words(line);
    if parts.contains(&option) {
        parts.retain(|p| *p != option);
    } else {
        parts.push(option);
    }
    parts.join(" ")
}

/// Chips under the Steam launch options field: a press adds or removes the option in the field;
/// Save to Steam still writes it.
pub fn known_options(s: &State, u: &Ui) -> El {
    let typed = s.settings.launch_options.text.clone();
    let present = words(&typed).into_iter().map(str::to_string).collect::<Vec<_>>();
    let mut chips = row().w_full().gap(6.0).flex_wrap();
    for (index, known) in KNOWN.iter().enumerate() {
        let on = present.iter().any(|p| p == known.option);
        let option = known.option;
        chips = chips.child(
            row()
                .key(format!("settings-known-option-{index}"))
                .h(26.0)
                .px(10.0)
                .gap(6.0)
                .center_y()
                .radius(crate::ui::r(13.0))
                .fill(if on { u.accent.alpha(0.14) } else { u.well })
                .border(1.0, if on { u.accent.alpha(0.5) } else { u.hair })
                .hover(if on { u.accent.alpha(0.2) } else { u.hover })
                .focusable(true)
                .role("checkbox")
                .checked(on)
                .label(fill(tr("{}: {}"), &[option, tr(known.line)]))
                .help(tr(known.line))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    let next = toggle_option(&st.settings.launch_options.text, option);
                    st.settings.launch_options = rune::TextEdit::new(next);
                })
                .children(on.then(|| rune::icon(&icons::CHECK, 12.0).color(u.accent).a11y_hidden()))
                .child(text(option).font(LABEL).color(if on { u.ink } else { u.prose })),
        );
    }
    stacked_row(
        u,
        "settings-known-options",
        "Common launch options",
        "Press one to add it to the field above or take it out. Hover for what it does.",
        col().w_full().child(chips),
    )
}

/// DMM's Vanilla Mode: a vanilla launch runs with no launch options.
pub fn vanilla_row(u: &Ui) -> El {
    toggle_row(
        u,
        "settings-vanilla-no-args",
        "Vanilla launches without launch options",
        "A vanilla launch clears Deadlock's launch options in Steam. Sigil keeps yours and writes them back at the next modded launch.",
        options::get().vanilla_no_args,
        |st| options::edit(st, |o| o.vanilla_no_args = !o.vanilla_no_args),
    )
}

/// GAME-ADJACENT WRITE through the existing launch options sync: for a vanilla launch with the
/// switch on, Steam gets empty launch options. When Sigil never saved the player's own options,
/// Steam's current value is saved first so the next modded launch writes it back. True when it
/// handled the step (the caller then skips the normal sync).
pub fn vanilla_without_options(st: &mut State) -> bool {
    let vanilla = st.launch.progress.as_ref().is_some_and(|p| p.vanilla);
    if !vanilla || !options::get().vanilla_no_args {
        return false;
    }
    use deadlock_core::autoexec::launch_options::{self, LaunchSync, Saved};
    let data = crate::data::data_dir();
    let steam = launch_options::steam_state(st.game_root.as_deref());
    let mut saved = launch_options::load(&data);
    if !saved.managed
        && let Some(current) = steam.current.as_deref().filter(|c| !c.trim().is_empty())
    {
        match launch_options::save(&data, current) {
            Ok(kept) => saved = kept,
            Err(error) => {
                crate::notify::push(
                    st,
                    crate::notify::Notice::warning(tr("Launch options not written")).detail(error.to_string()),
                );
                return true;
            }
        }
    }
    let _ = saved;
    let empty = Saved { value: String::new(), managed: true };
    match launch_options::sync_with(&empty, steam.path.as_deref(), steam.steam_running) {
        Ok(LaunchSync::Written { .. }) => {
            st.autoexec.launch.loaded = false;
            crate::notify::push(st, crate::notify::Notice::info(tr("Vanilla launch: launch options cleared in Steam")));
        }
        Ok(LaunchSync::SteamRunning) => crate::notify::push(
            st,
            crate::notify::Notice::warning(tr("Launch options not written"))
                .detail(tr("Steam was already running. Quit Steam before launching from Sigil so they apply.")),
        ),
        Ok(_) => {}
        Err(error) => crate::notify::push(
            st,
            crate::notify::Notice::warning(tr("Launch options not written")).detail(error.to_string()),
        ),
    }
    true
}

// --- update badges ----------------------------------------------------------------------

/// DMM's "Mod update badges": marks on cards and details. Checks keep running.
pub fn update_badges_row(s: &State, u: &Ui) -> El {
    toggle_row(
        u,
        "settings-update-badges",
        "Show mod update marks",
        "Marks mods with a newer file on GameBanana on cards and in details. Update checks keep running either way.",
        s.shell.w4.update_badges,
        |st| {
            let on = !st.shell.w4.update_badges;
            st.shell.w4.update_badges = on;
            options::edit(st, |o| o.update_badges = on);
        },
    )
}

// --- seasons and easter eggs ------------------------------------------------------------

/// Settings > Appearance > Seasons and extras: DMM's seasonal themes and Grimoire's easter
/// eggs, both off until turned on.
pub fn fun_section(_s: &State, u: &Ui) -> El {
    let o = options::get();
    let today = crate::shell_tips::today();
    let mut rows = vec![toggle_row(
        u,
        "settings-seasonal",
        "Seasonal themes",
        "Around Halloween, Christmas, New Year, Lunar New Year and Easter, a seasonal mark shows in the sidebar with a small surprise.",
        o.seasonal,
        |st| {
            let on = !st.shell.w4.seasonal;
            st.shell.w4.seasonal = on;
            options::edit(st, |o| o.seasonal = on);
        },
    )];
    if o.seasonal {
        let line = crate::shell_tips::season_line(today);
        let in_season = crate::shell_tips::season_on(today).is_some();
        let off = in_season && crate::shell_tips::active_season(today).is_none();
        rows.push(setting_row(
            u,
            "settings-seasonal-now",
            "This season",
            super::detail(u, &line),
            super::disabled(
                super::button(
                    u,
                    "settings-seasonal-dismiss",
                    if off { "Bring it back" } else { "Turn off until next time" },
                    crate::ui::ActionKind::Ghost,
                )
                .on_press(crate::shell_tips::toggle_dismiss),
                !in_season,
            ),
        ));
    }
    if o.seasonal {
        rows.push(toggle_row(
            u,
            "settings-seasonal-effects",
            "Seasonal effects",
            "Snow, sparks, lanterns or petals drifting over the window while the season shows. Light: 15 frames a second, nothing when it is off.",
            o.seasonal_effects,
            |st| {
                let on = !st.shell.w4.effects;
                st.shell.w4.effects = on;
                options::edit(st, |o| o.seasonal_effects = on);
            },
        ));
    }
    rows.push(toggle_row(
        u,
        "settings-easter-eggs",
        "Easter eggs",
        "Rare jokes here and there. Knock on the Sigil wordmark five times to find one.",
        o.easter_eggs,
        |st| {
            let on = !st.shell.w4.eggs;
            st.shell.w4.eggs = on;
            options::edit(st, |o| o.easter_eggs = on);
        },
    ));
    rows.push(setting_row(
        u,
        "settings-themed-art",
        "Themed art",
        super::detail(u, "Blueprint draws a fine grid like graph paper, Crest the Sigil mark behind the pages, Terminal the grid and monospace type."),
        super::segmented(
            u,
            "settings-themed-art-pick",
            &[(Art::None, "None"), (Art::Blueprint, "Blueprint"), (Art::Crest, "Crest"), (Art::Terminal, "Terminal")],
            Art::from_id(&o.themed_art),
            |st, art| {
                st.shell.w4.grid = matches!(art, Art::Blueprint | Art::Terminal);
                st.shell.w4.crest = art == Art::Crest;
                st.shell.w4.mono = art == Art::Terminal;
                options::edit(st, |o| o.themed_art = art.id().into());
            },
        ),
    ));
    super::section(u, "Seasons and extras", rows)
}

/// The themed presets (DMM's art presets, Sigil's own art), off by default.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Art {
    None,
    Blueprint,
    Crest,
    Terminal,
}

impl Art {
    pub fn id(self) -> &'static str {
        match self {
            Art::None => "none",
            Art::Blueprint => "grid",
            Art::Crest => "crest",
            Art::Terminal => "terminal",
        }
    }

    pub fn from_id(id: &str) -> Art {
        match id {
            "grid" => Art::Blueprint,
            "crest" => Art::Crest,
            "terminal" => Art::Terminal,
            _ => Art::None,
        }
    }
}

/// Terminal's type: every family monospace while it is on, Rune's own pair otherwise. Runs in
/// the poll; a change reaches Rune once.
pub fn apply_type(st: &mut State, tx: &mut Tx) -> bool {
    let mono = st.shell.w4.mono;
    if st.shell.w4.mono_applied == Some(mono) || (st.shell.w4.mono_applied.is_none() && !mono) {
        st.shell.w4.mono_applied = Some(mono);
        return false;
    }
    st.shell.w4.mono_applied = Some(mono);
    if mono {
        tx.set_font_families("IBM Plex Mono", "IBM Plex Mono");
    } else {
        tx.set_font_families("Inter", "IBM Plex Mono");
    }
    true
}

/// The art behind the pages: the blueprint grid (fine lines every 24 points, a stronger one
/// every 96), or the crest (the Sigil mark, large and faint, in the page's lower right).
pub fn themed_art(s: &State, u: &Ui, window: (f32, f32)) -> Option<El> {
    if s.shell.w4.crest {
        let px = crate::brand::mark(s, u)?;
        let left = crate::shell_w4::sidebar_width(s, 0.0);
        let size = (window.1 * 0.62).clamp(220.0, 640.0);
        return Some(
            rune::el()
                .key("themed-crest")
                .absolute((window.0 - size * 0.78).max(left), window.1 - size * 0.86)
                .size(size, size)
                .pixels_fit(px, rune::SurfaceFit::Fit)
                .opacity(0.05)
                .paint_order(-1)
                .a11y_hidden(),
        );
    }
    if !s.shell.w4.grid {
        return None;
    }
    const STEP: f32 = 24.0;
    let left = crate::shell_w4::sidebar_width(s, 0.0);
    let (w, h) = (window.0.max(0.0), window.1.max(0.0));
    let mut layer = rune::el()
        .key("themed-grid")
        .absolute(left, 0.0)
        .w(w - left)
        .h(h)
        .paint_order(-1)
        .a11y_hidden();
    let line = |major: bool| u.accent.alpha(if major { 0.10 } else { 0.045 });
    let mut x = 0.0;
    let mut n = 0;
    while x < w - left {
        layer = layer.child(rune::el().absolute(x, 0.0).w(1.0).h(h).fill(line(n % 4 == 0)));
        x += STEP;
        n += 1;
    }
    let (mut y, mut n) = (0.0, 0);
    while y < h {
        layer = layer.child(rune::el().absolute(0.0, y).w(w - left).h(1.0).fill(line(n % 4 == 0)));
        y += STEP;
        n += 1;
    }
    Some(layer)
}

/// Settings > General > Setup and tips: DMM's feature tips switch and reset.
pub fn tip_rows(u: &Ui) -> Vec<El> {
    let o = options::get();
    vec![
        toggle_row(
            u,
            "settings-feature-tips",
            "Show feature tips",
            "Points out a feature you have not tried yet, at most one each time Sigil starts.",
            o.feature_hints,
            |st| options::edit(st, |o| o.feature_hints = !o.feature_hints),
        ),
        setting_row(
            u,
            "settings-feature-tips-reset",
            "Reset feature tips",
            super::detail(u, "Shows feature tips again, including ones you dismissed."),
            super::disabled(
                super::button(u, "settings-feature-tips-reset-run", "Reset", crate::ui::ActionKind::Secondary).on_press(
                    |st: &mut State| {
                        options::edit(st, |o| {
                            o.hints_done.clear();
                            o.counters.retain(|k, _| !k.starts_with("ignored_"));
                            o.feature_hints = true;
                        });
                        super::say(st, tr("Feature tips will show again."), false);
                    },
                ),
                o.hints_done.is_empty() && o.feature_hints,
            ),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_option_goes_in_and_out_of_the_line() {
        assert_eq!(toggle_option("-novid +fps_max 240", "-high"), "-novid +fps_max 240 -high");
        assert_eq!(toggle_option("-novid -high", "-novid"), "-high");
        assert_eq!(toggle_option("", "-dx11"), "-dx11");
        assert_eq!(toggle_option("  -novid   -high ", "-high"), "-novid");
    }

    #[test]
    fn every_known_option_is_one_dash_word() {
        for known in KNOWN {
            assert!(known.option.starts_with('-') && !known.option.contains(' '));
            assert!(!known.line.is_empty());
        }
    }
}
