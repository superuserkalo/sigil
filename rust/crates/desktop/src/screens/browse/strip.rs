//! The quick numbers beside the landing's spotlight (Deadlock Mod Manager's dashboard strip):
//! mods installed, updates waiting, downloads queued and the active profile. Each one is a
//! shortcut to the place that owns it. The numbers come from the status bar's cached counts
//! (`crate::statusbar::Counts`), so nothing is walked per frame.

use rune::{El, col, row, text};

use crate::app::State;
use crate::model::Screen;
use crate::screens::settings::i18n::tr;
use crate::ui::{NUM, SMALL, SMALL_NUM, Ui};

/// One number: label over value in the column (`stacked`), or label and value on one line.
fn cell(u: &Ui, key: &str, label: &str, value: String, warn: bool, h: f32, stacked: bool, first: bool) -> El {
    let ink = if warn { u.amber } else { u.ink };
    let base = if stacked { col().justify_center() } else { row().gap(6.0).center_y() };
    let mut c = base
        .key(key.to_string())
        .h(h)
        .px(if stacked { 16.0 } else { 8.0 })
        .min_w(0.0)
        .hover(u.hover)
        .press(u.over.alpha(0.08))
        .focusable(true)
        .role("button")
        .label(format!("{}: {value}", tr(label)))
        .cursor(rune::Cursor::Pointer)
        .child(text(tr(label).to_string()).font(SMALL).color(u.muted).truncate().min_w(0.0).a11y_hidden())
        .child(text(value).font(if stacked { NUM } else { SMALL_NUM }).color(ink).truncate().a11y_hidden());
    if stacked {
        c = c.w_full();
        if !first {
            c = c.border_sides([1.0, 0.0, 0.0, 0.0], u.hair);
        }
    } else {
        c = c.grow(1.0).w(0.0).radius(crate::ui::r(4.0));
    }
    c
}

/// The four numbers, `h` high each. Installed reads a dash until the first library scan.
pub fn cells(s: &State, u: &Ui, h: f32, stacked: bool) -> Vec<El> {
    let counts = s.bar.counts;
    let installed = if s.addons.scanned { s.addons.addons.len().to_string() } else { "-".to_string() };
    let profile = s
        .profiles
        .active
        .as_ref()
        .and_then(|slug| s.profiles.rows.iter().find(|row| &row.slug == slug))
        .map(|row| row.name.clone())
        .unwrap_or_else(|| tr("None").to_string());
    vec![
        cell(u, "browse-quick-installed", "Installed mods", installed, false, h, stacked, true)
            .on_press(|st: &mut State| st.go(Screen::Installed)),
        cell(u, "browse-quick-updates", "Updates waiting", counts.updates.to_string(), counts.updates > 0, h, stacked, false)
            .on_press(|st: &mut State| st.go(Screen::Installed)),
        cell(u, "browse-quick-queue", "Downloads queued", counts.queued().to_string(), false, h, stacked, false)
            .on_press(|st: &mut State| st.launch.downloads.panel = true),
        cell(u, "browse-quick-profile", "Active profile", profile, false, h, stacked, false)
            .on_press(|st: &mut State| st.go(Screen::Profiles)),
    ]
}
