//! The status bar (Deadlock Mod Manager's bottom bar): one quiet line under every page with the
//! game, the saved catalog, downloads, the active profile, the preview volume and an app update.
//!
//! Every part reads state other modules already keep. The few counts that cost a walk (pending
//! mod updates, the download queue) are taken in [`poll`] at most once a second and kept in
//! [`Bar`], so a frame only reads numbers. Nothing here runs a job, reads a file or animates.
//! Settings > Appearance > Status bar hides it.
//!
//! The volume is Grimoire's global preview volume: the one value every audio preview plays at,
//! the same one Browse's View menu shows. A press on the speaker mutes and brings the level
//! back, as Grimoire's sidebar control does.

use std::time::{Duration, Instant};

use rune::{Align, El, Key, KeyEvent, Side, col, el, icon, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::model::Screen;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{SMALL, SMALL_NUM, Ui};

/// The bar's height, in points.
pub const BAR_H: f32 = 24.0;
/// How often the cached counts are taken again.
const REFRESH: Duration = Duration::from_secs(1);
/// The level a muted preview comes back to when nothing was remembered.
const UNMUTE: f32 = 0.7;

/// Counts the bar and Browse's quick stats read, taken in [`poll`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub downloading: usize,
    pub waiting: usize,
    pub paused: usize,
    /// Installed mods with an update on GameBanana.
    pub updates: usize,
}

impl Counts {
    /// Everything in the download queue that has not finished.
    pub fn queued(&self) -> usize {
        self.downloading + self.waiting + self.paused
    }
}

#[derive(Default)]
pub struct Bar {
    pub counts: Counts,
    counted_at: Option<Instant>,
    /// The volume popover is open.
    pub volume_open: bool,
    /// The level before a mute, so the next press brings it back.
    pub unmuted: Option<f32>,
}

/// The counts, taken now. Scenes call it after they stage their state.
pub fn refresh(st: &mut State) {
    st.bar.counts = count(st);
    st.bar.counted_at = Some(Instant::now());
}

/// Take the counts again once a second. True when one changed.
pub fn poll(st: &mut State) -> bool {
    if st.bar.counted_at.is_some_and(|at| at.elapsed() < REFRESH) {
        return false;
    }
    let before = st.bar.counts;
    refresh(st);
    st.bar.counts != before
}

fn count(s: &State) -> Counts {
    use deadlock_core::download_queue::State as Q;
    let queue = s.launch.downloads.snapshot();
    let of = |state: Q| queue.items.iter().filter(|item| item.state == state).count();
    Counts {
        downloading: of(Q::Running),
        waiting: of(Q::Queued),
        paused: of(Q::Paused),
        updates: if s.addons.scanned {
            crate::screens::installed::pending_update_count(s)
        } else {
            0
        },
    }
}

/// Mute, or bring the level from before the mute back.
pub fn toggle_mute(st: &mut State) {
    let now = st.browse_view.audio.volume;
    let next = if now > 0.0 {
        st.bar.unmuted = Some(now);
        0.0
    } else {
        st.bar.unmuted.take().filter(|v| *v > 0.0).unwrap_or(UNMUTE)
    };
    st.browse_view.audio.set_volume(next);
    crate::screens::browse::store::save_browse_state(st);
}

fn speaker(volume: f32) -> &'static rune::Icon {
    if volume <= 0.0 {
        &icons::VOLUME_X
    } else if volume < 0.5 {
        &icons::VOLUME_1
    } else {
        &icons::VOLUME_2
    }
}

/// A quiet pressable part: an icon and a few words.
pub(crate) fn part(u: &Ui, key: &str, glyph: &'static rune::Icon, words: String, tint: rune::Color, label: String) -> El {
    row()
        .key(key.to_string())
        .h(BAR_H)
        .px(8.0)
        .gap(6.0)
        .center_y()
        .hover(u.hover)
        .press(u.over.alpha(0.08))
        .focusable(true)
        .role("button")
        .label(label)
        .cursor(rune::Cursor::Pointer)
        .help_above()
        .child(icon(glyph, 14.0).color(tint).a11y_hidden())
        .child(text(words).font(SMALL).color(u.muted).a11y_hidden())
}

fn divider(u: &Ui) -> El {
    el().w(1.0).h(12.0).mx(4.0).fill(u.hair).a11y_hidden()
}

/// "4 minutes ago" and the like, for the catalog's last sync.
pub fn ago(seconds: i64) -> String {
    match seconds.max(0) {
        s if s < 60 => tr("just now").to_string(),
        s if s < 3600 => fill(tr("{} min ago"), &[&(s / 60).to_string()]),
        s if s < 86_400 => fill(tr("{} h ago"), &[&(s / 3600).to_string()]),
        s => fill(tr("{} d ago"), &[&(s / 86_400).to_string()]),
    }
}

fn game(s: &State, u: &Ui) -> El {
    // Normal is quiet (muted icon); not found is amber. Both say what they mean.
    let (words, tint) = if s.game_detected {
        (tr("Deadlock found"), u.muted)
    } else {
        (tr("Game not found"), u.amber)
    };
    part(u, "statusbar-game", &icons::FOLDER_OPEN, words.to_string(), tint, words.to_string()).on_press(
        |st: &mut State| {
            st.settings.tab = crate::screens::settings::Tab::Game;
            st.go(Screen::Settings);
        },
    )
}

/// The saved catalog: only while it syncs, when Sigil is offline, or on Browse, where it is the
/// page's data. When it is fine elsewhere it is noise.
fn catalog(s: &State, u: &Ui) -> Option<El> {
    let v = &s.browse_view;
    let (words, tint) = if s.prefs.offline {
        (tr("Offline").to_string(), u.amber)
    } else if v.mirror_sync_job.is_running() {
        (tr("Catalog syncing").to_string(), u.accent)
    } else if s.screen != Screen::Browse {
        return None;
    } else {
        match v.mirror.as_ref().filter(|m| !m.entries.is_empty()) {
            Some(mirror) if mirror.meta.synced_at > 0 => {
                let age = deadlock_core::gamebanana::now_unix() - mirror.meta.synced_at;
                (fill(tr("Catalog {}"), &[&ago(age)]), u.muted)
            }
            Some(_) => (tr("Catalog saved").to_string(), u.muted),
            None => (tr("Catalog not saved yet").to_string(), u.muted),
        }
    };
    let label = if v.mirror_summary.is_empty() {
        words.clone()
    } else {
        format!("{words}. {}", v.mirror_summary)
    };
    Some(part(u, "statusbar-catalog", &icons::DATABASE, words, tint, label).on_press(|st: &mut State| {
        st.settings.tab = crate::screens::settings::Tab::Network;
        st.go(Screen::Settings);
    }))
}

fn downloads(s: &State, u: &Ui) -> Option<El> {
    let c = s.bar.counts;
    if c.queued() == 0 {
        return None;
    }
    let mut words = Vec::new();
    if c.downloading > 0 {
        words.push(fill(tr("{} downloading"), &[&c.downloading.to_string()]));
    }
    if c.waiting > 0 {
        words.push(fill(tr("{} waiting"), &[&c.waiting.to_string()]));
    }
    if c.paused > 0 {
        words.push(fill(tr("{} paused"), &[&c.paused.to_string()]));
    }
    let words = words.join(", ");
    let tint = if c.downloading > 0 { u.accent } else { u.faint };
    Some(
        part(u, "statusbar-downloads", &icons::DOWNLOAD, words.clone(), tint, words)
            .on_press(|st: &mut State| st.launch.downloads.panel = !st.launch.downloads.panel),
    )
}

fn volume(s: &State, u: &Ui) -> El {
    let level = s.browse_view.audio.volume.clamp(0.0, 1.0);
    let percent = format!("{}%", (level * 100.0).round() as u32);
    let label = fill(tr("Preview volume {}"), &[&percent]);
    let mut chip = part(u, "statusbar-volume", speaker(level), percent, u.faint, label)
        .on_press(|st: &mut State| st.bar.volume_open = !st.bar.volume_open);
    if s.bar.volume_open {
        chip = chip.child(volume_popover(s, u, level));
    }
    // Personalize > Art > Volume bar (off by default) paints under the control.
    crate::screens::settings::personalize::volume_art(s, u, chip)
}

fn volume_popover(s: &State, u: &Ui, level: f32) -> El {
    let muted = level <= 0.0;
    col()
        .key("statusbar-volume-popover")
        .role("dialog")
        .label(tr("Preview volume"))
        .w(200.0)
        .padding(12.0)
        .gap(8.0)
        .fill(u.raised)
        .radius(crate::ui::r(10.0))
        .border(1.0, u.hair_strong)
        .shadow(12.0, 32.0, crate::ui::soft(rune::Color::BLACK.alpha(0.5)))
        .anchored("statusbar-volume", Side::Top, Align::End, 8.0)
        .transition(crate::motion::pop(true))
        .modal(true, |st: &mut State| st.bar.volume_open = false)
        .on_key(|st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                st.bar.volume_open = false;
                true
            } else {
                false
            }
        })
        .child(
            row()
                .w_full()
                .center_y()
                .child(text(tr("Preview volume")).font(SMALL).color(u.muted))
                .child(spacer())
                .child(text(format!("{}%", (level * 100.0).round() as u32)).font(SMALL_NUM).color(u.muted)),
        )
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(
                    u.icon_button(speaker(level), if muted { tr("Unmute") } else { tr("Mute") })
                        .key("statusbar-volume-mute")
                        .size(24.0, 24.0)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(toggle_mute),
                )
                .child(crate::screens::browse::grid::menus::audio_slider(s, u, "statusbar-volume-slider")),
        )
        .child(
            text(tr("Every sound preview in Sigil plays at this level."))
                .font(SMALL)
                .color(u.faint)
                .wrap()
                .w_full(),
        )
}

fn update(s: &State, u: &Ui) -> Option<El> {
    let ready = s.settings.updater.ready().map(str::to_string);
    let offered = match &s.settings.update {
        Some(Ok(check)) if check.newer => check.latest.as_ref().map(|r| r.version.clone()),
        _ => None,
    };
    let (words, version) = match (ready, offered) {
        (Some(version), _) => (fill(tr("Sigil {} is ready to install"), &[&version]), version),
        (None, Some(version)) => (fill(tr("Sigil {} is out"), &[&version]), version),
        (None, None) => return None,
    };
    let _ = version;
    Some(
        part(u, "statusbar-update", &icons::DOWNLOAD, words.clone(), u.accent, words).on_press(|st: &mut State| {
            st.settings.tab = crate::screens::settings::Tab::About;
            st.go(Screen::Settings);
        }),
    )
}

/// The bar, or nothing when Settings hides it.
pub fn view(s: &State, u: &Ui) -> Option<El> {
    // Blocked relay regions show even when the bar is hidden: a forgotten block looks like
    // broken matchmaking, so the mark stays on every page while one is on.
    let regions = crate::screens::servers::regions::status_chip(s, u);
    if !s.prefs.status_bar {
        return regions.map(|chip| bar(u).child(chip).child(spacer()));
    }
    let mut left = row().gap(2.0).center_y().child(game(s, u));
    if let Some(catalog) = catalog(s, u) {
        left = left.child(divider(u)).child(catalog);
    }
    if let Some(chip) = regions {
        left = left.child(divider(u)).child(chip);
    }
    if let Some(chip) = crate::screens::servers::regions::health_chip(s, u) {
        left = left.child(divider(u)).child(chip);
    }
    if let Some(downloads) = downloads(s, u) {
        left = left.child(divider(u)).child(downloads);
    }
    let mut right = row().gap(2.0).center_y();
    if let Some(update) = update(s, u) {
        right = right.child(update).child(divider(u));
    }
    // The profile lives in the titlebar switcher, the one that acts.
    right = right.child(volume(s, u)).child(divider(u)).child(shortcuts(u));
    Some(bar(u).child(left).child(spacer()).child(right))
}

/// Keyboard shortcuts, at the bar's trailing end (`?` opens the same sheet).
fn shortcuts(u: &Ui) -> El {
    row()
        .key("statusbar-shortcuts")
        .size(BAR_H, BAR_H)
        .center()
        .hover(u.hover)
        .press(u.over.alpha(0.08))
        .focusable(true)
        .role("button")
        .label(tr("Keyboard shortcuts"))
        .help(tr("Keyboard shortcuts"))
        .help_keys(["?"])
        .help_above()
        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| st.shell.shortcuts = true)
        .child(icon(&icons::KEYBOARD, 14.0).color(u.muted).a11y_hidden())
}

fn bar(u: &Ui) -> El {
    // Parts carry 8 points of padding, so their icons start on the page's 24 point edge.
    // Parts that come and go with the page (the catalog on Browse) land at once: a status line
    // never fades.
    crate::motion::lands(row())
        .key("statusbar")
        .role("status")
        .label(tr("Status bar"))
        .w_full()
        .h(BAR_H)
        .pl(crate::ui::PAGE_X - 8.0)
        .pr(crate::ui::PAGE_X - 8.0)
        .gap(8.0)
        .center_y()
        .border_sides([1.0, 0.0, 0.0, 0.0], u.hair)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ago_reads_in_the_largest_unit() {
        assert_eq!(ago(5), "just now");
        assert_eq!(ago(-3), "just now");
        assert_eq!(ago(125), "2 min ago");
        assert_eq!(ago(7200), "2 h ago");
        assert_eq!(ago(3 * 86_400 + 5), "3 d ago");
    }

    #[test]
    fn queued_counts_every_unfinished_download() {
        let c = Counts { downloading: 1, waiting: 2, paused: 3, updates: 9 };
        assert_eq!(c.queued(), 6);
        assert_eq!(Counts::default().queued(), 0);
    }

    #[test]
    fn mute_remembers_the_level_and_brings_it_back() {
        let mut st = State::default();
        // Demo state never writes the browse file.
        st.demo = true;
        st.browse_view.audio.set_volume(0.4);
        toggle_mute(&mut st);
        assert_eq!(st.browse_view.audio.volume, 0.0);
        toggle_mute(&mut st);
        assert!((st.browse_view.audio.volume - 0.4).abs() < 1e-6);
        // Muted with nothing remembered comes back at the default level.
        st.browse_view.audio.set_volume(0.0);
        st.bar.unmuted = None;
        toggle_mute(&mut st);
        assert!((st.browse_view.audio.volume - UNMUTE).abs() < 1e-6);
    }
}
