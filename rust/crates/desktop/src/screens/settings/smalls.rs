//! Small parity pieces for Settings (the "smalls" round):
//!
//! - Background > "Behind the status bar" (Deadlock Mod Manager's "Include bottom bar").
//! - Art > Volume bar: the preview of the status bar's volume control (Grimoire's volume bar
//!   art). The art itself paints in `personalize::volume_art`.
//! - Update download progress as one line with bytes and a percent, for Settings > Updates
//!   and the update banner (Grimoire's Updates progress).
//! - The package manager note: a copy installed by winget, Scoop, a Linux package and so on
//!   says updates come from that manager and hides the in-app installer (Grimoire's
//!   "managed" install). See `crate::package_manager`.
//!
//! Every string here has German in `locales_smalls.rs`.

use rune::{El, col, el, icon, row, text};

use super::i18n::{fill, tr};
use crate::app::State;
use crate::icons;
use crate::package_manager::Manager;
use crate::ui::personalize::Surface;
use crate::ui::{MICRO, SMALL, Ui};

// --- background ---------------------------------------------------------------------------

/// The Background section's status bar switch. Off by default.
pub fn statusbar_row(s: &State, u: &Ui, press: fn(&mut State)) -> El {
    let line = if s.prefs.status_bar {
        "The image runs under the status bar at the bottom of the window."
    } else {
        "The status bar is hidden in Settings, so this has no effect until it is back."
    };
    super::toggle_row(
        u,
        "settings-bg-statusbar",
        "Behind the status bar",
        line,
        s.prefs.personal.background.statusbar,
        press,
    )
}

// --- art: volume bar ----------------------------------------------------------------------

/// The status bar's volume control as Personalize's art preview shows it: the speaker and the
/// level, at the control's real height.
pub fn volume_preview(s: &State, u: &Ui) -> El {
    let level = s.browse_view.audio.volume.clamp(0.0, 1.0);
    row()
        .h(20.0)
        .px(6.0)
        .gap(5.0)
        .center_y()
        .radius(crate::ui::r(5.0))
        .child(icon(&icons::VOLUME_2, 12.0).color(u.ink).a11y_hidden())
        .child(text(format!("{}%", (level * 100.0).round() as u32)).font(MICRO).color(u.prose).a11y_hidden())
}

/// A line under the art picker for the surfaces that do not live in the sidebar.
pub fn art_note(u: &Ui, surface: Surface) -> Option<El> {
    (surface == Surface::Volume).then(|| {
        text(tr("The volume bar is the speaker at the right end of the status bar."))
            .font(SMALL)
            .color(u.muted)
            .wrap()
    })
}

// --- update progress ----------------------------------------------------------------------

/// Whole percent of a download, 0 to 100. Zero while the size is unknown.
pub fn percent(done: u64, total: u64) -> u32 {
    if total == 0 {
        return 0;
    }
    ((done.min(total) as f64 / total as f64) * 100.0).floor() as u32
}

/// "18.4 MB of 41.2 MB (44%)".
pub fn progress_line(done: u64, total: u64) -> String {
    fill(
        tr("{} of {} ({}%)"),
        &[
            &crate::updater::megabytes(done),
            &crate::updater::megabytes(total),
            &percent(done, total).to_string(),
        ],
    )
}

/// The update download in flight: the version, the progress line and the fraction done.
pub fn download(s: &State) -> Option<(String, String, f32)> {
    let up = &s.settings.updater;
    let (done, total) = up.progress()?;
    let version = up.downloading.clone()?;
    let frac = if total > 0 { (done as f32 / total as f32).clamp(0.0, 1.0) } else { 0.0 };
    Some((version, progress_line(done, total), frac))
}

/// A thin progress bar, as Settings' download row draws it.
pub fn bar(u: &Ui, frac: f32) -> El {
    row()
        .w_full()
        .h(4.0)
        .radius(2.0)
        .fill(u.well)
        .child(el().w_frac(frac.clamp(0.0, 1.0)).h_full().radius(2.0).fill(u.accent))
}

/// The update banner's body while Sigil downloads its update: the progress line over a bar.
pub fn banner_body(s: &State, u: &Ui) -> Option<(String, El)> {
    let (version, line, frac) = download(s)?;
    let title = fill(tr("Downloading Sigil {}"), &[&version]);
    let body = col()
        .key("update-banner-progress")
        .w_full()
        .gap(4.0)
        .child(text(line).font(SMALL).color(u.muted).truncate())
        .child(bar(u, frac));
    Some((title, body))
}

// --- package manager ----------------------------------------------------------------------

/// Why this copy does not install updates itself, when a package manager owns it.
pub fn managed_reason(manager: Manager) -> String {
    if manager == Manager::SigilDeb {
        return tr("This copy came from Sigil's .deb package. Install the new .deb from the release page, or switch to the AppImage, which updates itself.").to_string();
    }
    match manager.name() {
        Some(name) => fill(tr("Updates for this copy come from {}. Update Sigil there."), &[name]),
        None => tr("Updates for this copy come from your system's package manager. Update Sigil there.").to_string(),
    }
}

/// Settings > Updates: the note that replaces the installer for a package-managed copy.
pub fn package_manager_row(u: &Ui) -> Option<El> {
    package_manager_row_for(u, crate::package_manager::current()?)
}

fn package_manager_row_for(u: &Ui, manager: Manager) -> Option<El> {
    Some(super::setting_row(
        u,
        "settings-update-managed",
        "Updates are managed by your package manager",
        text(managed_reason(manager)).font(SMALL).color(u.muted).wrap(),
        el(),
    ))
}

// --- scenes -------------------------------------------------------------------------------

/// A 0.2.0 release on offer with the download 44% in, for the update scenes.
fn stage_download(st: &mut State) {
    use std::sync::atomic::Ordering;
    let offer = crate::updater::Offer {
        version: "0.2.0".into(),
        url: "https://github.com/superuserkalo/sigil/releases/tag/v0.2.0".into(),
        notes: "## What's new\n\n- Faster start on large mod folders.".into(),
        install: Err(String::new()),
    };
    st.settings.update = Some(Ok(super::UpdateCheck { latest: Some(offer), newer: true }));
    let progress = std::sync::Arc::new(crate::updater::Progress::default());
    progress.done.store(18_400_000, Ordering::Relaxed);
    progress.total.store(41_200_000, Ordering::Relaxed);
    st.settings.updater.progress = Some(progress);
    st.settings.updater.downloading = Some("0.2.0".into());
}

/// The update banner over Installed while the update downloads.
pub fn scene_update_banner(st: &mut State) {
    st.load_demo();
    st.screen = crate::model::Screen::Installed;
    stage_download(st);
}

/// Settings > About > Updates while the update downloads.
pub fn scene_update_settings(st: &mut State) {
    super::scene_tab(st, super::Tab::About);
    stage_download(st);
}

/// Settings > About > Updates for a package-managed copy. Snap it with
/// `SIGIL_PACKAGE_MANAGER_SCENE=scoop` (debug builds only).
pub fn scene_update_managed(st: &mut State) {
    super::scene_tab(st, super::Tab::About);
    let offer = crate::updater::Offer {
        version: "0.2.0".into(),
        url: "https://github.com/superuserkalo/sigil/releases/tag/v0.2.0".into(),
        notes: "## What's new\n\n- Faster start on large mod folders.".into(),
        install: Err(managed_reason(crate::package_manager::current().unwrap_or(Manager::Scoop))),
    };
    st.settings.update = Some(Ok(super::UpdateCheck { latest: Some(offer), newer: true }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_is_whole_and_bounded() {
        assert_eq!(percent(0, 0), 0);
        assert_eq!(percent(18_400_000, 41_200_000), 44);
        assert_eq!(percent(41_200_000, 41_200_000), 100);
        assert_eq!(percent(50, 40), 100, "a server that sends more never reads past 100");
    }

    #[test]
    fn the_progress_line_has_bytes_and_percent() {
        assert_eq!(progress_line(18_400_000, 41_200_000), "18.4 MB of 41.2 MB (44%)");
    }

    #[test]
    fn the_managed_reason_names_the_manager() {
        assert!(managed_reason(Manager::Scoop).contains("Scoop"));
        assert!(managed_reason(Manager::System).contains("system's package manager"));
        assert!(managed_reason(Manager::SigilDeb).contains(".deb"));
    }
}
