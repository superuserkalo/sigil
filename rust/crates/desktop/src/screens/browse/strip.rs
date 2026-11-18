//! The quick stats strip over Browse's landing (Deadlock Mod Manager's dashboard strip): mods
//! installed, updates waiting, downloads queued and the active profile. Each one is a shortcut
//! to the place that owns it. The numbers come from the status bar's cached counts
//! (`crate::statusbar::Counts`), so the strip walks nothing per frame.

use rune::{El, col, icon, row, text};

use crate::app::State;
use crate::icons;
use crate::model::Screen;
use crate::screens::settings::i18n::tr;
use crate::ui::{NUM, SMALL, Ui};

/// One chip: an icon, a label over a value, and the press that opens its place.
fn chip(
    u: &Ui,
    key: &str,
    glyph: &'static rune::Icon,
    label: &str,
    value: String,
    accent: bool,
    first: bool,
) -> El {
    let tint = if accent { u.accent } else { u.muted };
    let mut c = row()
        .key(key.to_string())
        .grow(1.0)
        .min_w(0.0)
        .h(56.0)
        .px(16.0)
        .gap(12.0)
        .center_y()
        .hover(u.hover)
        .press(u.over.alpha(0.08))
        .focusable(true)
        .role("button")
        .label(format!("{}: {value}", tr(label)))
        .cursor(rune::Cursor::Pointer)
        .child(icon(glyph, 18.0).color(tint).a11y_hidden())
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(0.0)
                .child(text(tr(label).to_string()).font(SMALL).color(if accent { u.accent } else { u.faint }).truncate())
                .child(text(value).font(NUM).color(if accent { u.accent } else { u.ink }).truncate()),
        );
    if !first {
        c = c.border_sides([0.0, 0.0, 0.0, 1.0], u.hair);
    }
    c
}

/// The strip. Installed reads a dash until the first library scan.
pub fn view(s: &State, u: &Ui) -> El {
    let counts = s.bar.counts;
    let installed = if s.addons.scanned { s.addons.addons.len().to_string() } else { "-".to_string() };
    let profile = s
        .profiles
        .active
        .as_ref()
        .and_then(|slug| s.profiles.rows.iter().find(|row| &row.slug == slug))
        .map(|row| row.name.clone())
        .unwrap_or_else(|| tr("None").to_string());
    row()
        .key("browse-quick-stats")
        .w_full()
        .radius(crate::ui::r(10.0))
        .border(1.0, u.hair)
        .clip_rounded(crate::ui::r(10.0))
        .child(
            chip(u, "browse-quick-installed", &icons::PACKAGE, "Installed mods", installed, false, true)
                .on_press(|st: &mut State| st.go(Screen::Installed)),
        )
        .child(
            chip(
                u,
                "browse-quick-updates",
                &icons::REFRESH_CW,
                "Updates waiting",
                counts.updates.to_string(),
                counts.updates > 0,
                false,
            )
            .on_press(|st: &mut State| st.go(Screen::Installed)),
        )
        .child(
            chip(u, "browse-quick-queue", &icons::DOWNLOAD, "Downloads queued", counts.queued().to_string(), false, false)
                .on_press(|st: &mut State| st.launch.downloads.panel = true),
        )
        .child(
            chip(u, "browse-quick-profile", &icons::LAYERS, "Active profile", profile, false, false)
                .on_press(|st: &mut State| st.go(Screen::Profiles)),
        )
}
