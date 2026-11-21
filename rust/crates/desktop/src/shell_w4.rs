//! Shell additions from parity wave 4 (UI B): the collapsible sidebar (Grimoire and DMM), "New"
//! marks on pages until they are opened (DMM), the app update banner with "Hide until next
//! launch" (Grimoire), and the session-only hiding of the gameinfo banner (Grimoire).
//!
//! Contextual tips, the experimental tour and the easter eggs live in `shell_tips.rs`.

use rune::{Color, El, col, el, row, text};

use crate::app::State;
use crate::icons;
use crate::model::Screen;
use crate::screens::settings::i18n::{fill, tr};
use crate::screens::settings::options;
use crate::ui::{ActionKind, BODY_MED, MICRO, Ui};

/// The sidebar's width with icons only.
pub const COLLAPSED_W: f32 = 64.0;

/// Pages marked "New" in the sidebar until they are opened once. DMM marks the pages it added
/// last; these are Sigil's newest places.
pub const NEW_PAGES: [Screen; 3] = [Screen::Stats, Screen::Servers, Screen::Performance];

/// Shell state for this wave. The saved parts mirror `options.json` so a frame reads fields,
/// not the options file.
#[derive(Debug)]
pub struct W4 {
    pub collapsed: bool,
    pub seen_pages: Vec<String>,
    /// The update banner was hidden this run (Grimoire: until the next launch).
    pub update_hidden: bool,
    /// The gameinfo banner text hidden this run; a different problem still shows.
    pub gameinfo_hidden: Option<String>,
    /// Mirror of `options.update_badges`, read by every card.
    pub update_badges: bool,
    /// Mirrors of `options.easter_eggs` and `options.themed_art`, read every frame.
    pub eggs: bool,
    pub grid: bool,
    pub crest: bool,
    pub mono: bool,
    /// The type families last handed to Rune (Terminal's monospace), so a change applies once.
    pub mono_applied: Option<bool>,
    /// Mirrors of `options.seasonal` and `options.seasonal_effects`.
    pub seasonal: bool,
    pub effects: bool,
}

impl Default for W4 {
    fn default() -> Self {
        let o = options::get();
        W4 {
            collapsed: o.sidebar_collapsed,
            seen_pages: o.seen_pages,
            update_hidden: false,
            gameinfo_hidden: None,
            update_badges: o.update_badges,
            eggs: o.easter_eggs,
            grid: matches!(o.themed_art.as_str(), "grid" | "terminal"),
            crest: o.themed_art == "crest",
            mono: o.themed_art == "terminal",
            mono_applied: None,
            seasonal: o.seasonal,
            effects: o.seasonal_effects,
        }
    }
}

/// The sidebar's width: icons only when collapsed, wide enough for the macOS traffic lights.
pub fn sidebar_width(s: &State, leading: f32) -> f32 {
    if s.shell.w4.collapsed {
        COLLAPSED_W.max(leading + 4.0)
    } else {
        crate::ui::SIDEBAR_W
    }
}

/// Collapse or expand the sidebar, kept across launches (Grimoire; DMM's Cmd+B).
pub fn toggle_sidebar(st: &mut State) {
    let collapsed = !st.shell.w4.collapsed;
    st.shell.w4.collapsed = collapsed;
    options::edit(st, |o| o.sidebar_collapsed = collapsed);
}

/// The collapse control: a square icon button, on the wordmark's row while the sidebar is
/// open, its name and shortcut in the tooltip.
pub fn collapse_button(s: &State, u: &Ui) -> El {
    let collapsed = s.shell.w4.collapsed;
    let label = tr(if collapsed { "Expand sidebar" } else { "Collapse sidebar" });
    row()
        .key("sidebar-collapse")
        .size(28.0, 28.0)
        .center()
        .radius(crate::ui::r(6.0))
        .hover(u.hover)
        .press(Color::WHITE.alpha(0.06))
        .focusable(true)
        .role("button")
        .label(label)
        .help(fill(tr("{} ({})"), &[label, shortcut_label()]))
        .cursor(rune::Cursor::Pointer)
        .on_press(toggle_sidebar)
        .child(
            rune::icon(if collapsed { &icons::CHEVRON_RIGHT } else { &icons::PANEL_LEFT }, 15.0)
                .color(u.faint)
                .a11y_hidden(),
        )
}

fn shortcut_label() -> &'static str {
    if cfg!(target_os = "macos") { "Cmd+B" } else { "Ctrl+B" }
}

// --- New marks ------------------------------------------------------------------------------

/// The page carries a "New" mark: it is one of the newest places and was never opened.
pub fn is_new(s: &State, screen: Screen) -> bool {
    NEW_PAGES.contains(&screen) && !s.shell.w4.seen_pages.iter().any(|k| k == screen.key())
}

/// Opening a page clears its mark for good.
pub fn mark_seen(st: &mut State, screen: Screen) {
    if !is_new(st, screen) {
        return;
    }
    let key = screen.key().to_string();
    st.shell.w4.seen_pages.push(key.clone());
    options::edit_quiet(|o| {
        if !o.seen_pages.contains(&key) {
            o.seen_pages.push(key);
        }
    });
}

/// The small "New" pill beside a page's name.
pub fn new_pill(u: &Ui) -> El {
    row()
        .h(16.0)
        .px(5.0)
        .center()
        .radius(crate::ui::r(4.0))
        .fill(u.over.alpha(0.07))
        .child(text(tr("New")).font(MICRO).color(u.muted))
}

// --- update banner --------------------------------------------------------------------------

/// Grimoire's banner over every page while a Sigil update is out: Install when it is ready,
/// View update otherwise, and an X that hides it until the next launch.
pub fn update_banner(s: &State, u: &Ui) -> Option<El> {
    if s.shell.w4.update_hidden {
        return None;
    }
    let Some(Ok(check)) = &s.settings.update else {
        return None;
    };
    let latest = check.latest.as_ref().filter(|_| check.newer)?;
    let ready = crate::updater::sidebar_badge(s, u).is_some();
    let title = if ready {
        fill(tr("Sigil {} is ready to install"), &[&latest.version])
    } else {
        fill(tr("Sigil update available ({})"), &[&latest.version])
    };
    let body = tr(if ready {
        "Restart to finish updating. Your mods and settings stay untouched."
    } else {
        "A newer version is out. Read what changed and get it from About."
    });
    // While Sigil downloads the update, the banner shows how far it is.
    let progress = if ready { None } else { crate::screens::settings::smalls::banner_body(s, u) };
    let (title, body) = match progress {
        Some((title, bar)) => (title, bar),
        None => (title, text(body).font(crate::ui::SMALL).color(u.muted).truncate()),
    };
    Some(
        row()
            .key("update-banner")
            .w_full()
            .min_h(44.0)
            .px(16.0)
            .py(8.0)
            .gap(12.0)
            .center_y()
            .fill(u.accent.alpha(0.10))
            .border_sides([0.0, 0.0, 1.0, 0.0], u.accent.alpha(0.35))
            .role("status")
            .child(rune::icon(&icons::DOWNLOAD, 16.0).color(u.accent).a11y_hidden())
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(2.0)
                    .child(text(title).font(BODY_MED).color(u.ink).truncate())
                    .child(body),
            )
            .child(
                u.action(tr(if ready { "Install" } else { "View update" }), ActionKind::Secondary)
                    .key("update-banner-open")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| {
                        crate::screens::settings::open_tab(st, crate::screens::settings::Tab::About)
                    }),
            )
            .child(
                u.icon_button(&icons::X, tr("Hide until next launch"))
                    .key("update-banner-hide")
                    .help(tr("Hide until next launch"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| st.shell.w4.update_hidden = true),
            ),
    )
}

// --- gameinfo banner ------------------------------------------------------------------------

/// The gameinfo banner's own text is hidden for this run (Grimoire: a different problem shows).
pub fn gameinfo_hidden(s: &State, text: &str) -> bool {
    s.shell.w4.gameinfo_hidden.as_deref() == Some(text)
}

/// The X on the gameinfo banner: hides this message until Sigil starts again.
pub fn gameinfo_hide_button(u: &Ui, text: String) -> El {
    u.icon_button(&icons::X, tr("Hide until next launch"))
        .key("gameinfo-banner-hide")
        .help(tr("Hide until next launch"))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| st.shell.w4.gameinfo_hidden = Some(text.clone()))
}

/// A dot in place of a count while the sidebar is collapsed (Grimoire keeps warnings only).
pub fn dot(color: Color) -> El {
    el().size(6.0, 6.0).radius(3.0).fill(color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_page_loses_its_mark_once_opened() {
        let mut st = crate::data::init_state();
        st.shell.w4.seen_pages.clear();
        assert!(is_new(&st, Screen::Stats));
        assert!(!is_new(&st, Screen::Browse));
        mark_seen(&mut st, Screen::Stats);
        assert!(!is_new(&st, Screen::Stats));
        mark_seen(&mut st, Screen::Stats);
        assert_eq!(st.shell.w4.seen_pages.iter().filter(|k| *k == "stats").count(), 1);
    }
}
