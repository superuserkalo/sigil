//! Headless scenes for parity wave 3: the Pages switches, the status bar and its volume, the
//! Statlocker links, Browse's quick stats strip and the review's outdated-data switch.

use crate::app::State;
use crate::model::Screen;
use crate::screens::settings::Tab;

/// Settings > General > Pages with Stats and Servers off: the sidebar loses both.
pub fn pages(s: &mut State) {
    crate::screens::settings::scene_tab(s, Tab::General);
    s.settings.extra.scene_only = Some(Tab::General);
    s.settings.extra.scene_page = 3;
    s.prefs.toggle_flag("page_stats");
    s.prefs.toggle_flag("servers");
    crate::statusbar::refresh(s);
}

/// "Mods only" on: every optional page is paused and the sidebar keeps mod management.
pub fn mods_only(s: &mut State) {
    pages(s);
    s.prefs.toggle_flag(crate::prefs::MODS_ONLY);
}

/// The status bar over Installed with downloads moving and a Sigil update out.
pub fn status_bar(s: &mut State) {
    crate::app::scene_demo_installed(s);
    crate::screens::launch::stage_queue(s);
    s.settings.update = Some(Ok(crate::screens::settings::UpdateCheck {
        latest: Some(crate::screens::settings::Release {
            version: "0.2.0".to_string(),
            url: "https://github.com/".to_string(),
            notes: String::new(),
            install: Err(String::new()),
        }),
        newer: true,
    }));
    crate::statusbar::refresh(s);
}

/// The status bar's volume popover open, at 40 percent.
pub fn status_volume(s: &mut State) {
    status_bar(s);
    s.browse_view.audio.volume = 0.4;
    s.bar.volume_open = true;
}

/// Stats with the newest match open: the header and the match link to Statlocker.
pub fn statlocker(s: &mut State) {
    crate::screens::stats::scene_drill(s);
    s.stats.account_open = false;
}

/// The scoreboard dialog with its Statlocker link.
pub fn statlocker_board(s: &mut State) {
    crate::screens::stats::scene_board(s);
}

/// Teammates and opponents, each with a Statlocker link.
pub fn statlocker_people(s: &mut State) {
    crate::screens::stats::scene_people(s);
}

/// Browse's landing with the quick stats strip: two updates waiting and a queue.
pub fn browse_strip(s: &mut State) {
    crate::app::scene_demo_installed(s);
    crate::screens::installed::stage_updates_for_scene(s);
    crate::screens::launch::stage_queue(s);
    crate::screens::browse::scene_dashboard(s);
    s.screen = Screen::Browse;
    crate::statusbar::refresh(s);
}

/// The compatibility review with one mod whose game data is outdated.
pub fn health_outdated(s: &mut State) {
    crate::screens::installed::scene_health(s);
    crate::statusbar::refresh(s);
}

/// The same, with the bulk switch asking first.
pub fn health_outdated_confirm(s: &mut State) {
    health_outdated(s);
    if let Some(ui) = s.installed.tools.health.as_mut() {
        ui.confirm_off = true;
    }
}

/// Mod lists with "One at random each launch" on for the demo list.
pub fn lists_random(s: &mut State) {
    crate::screens::installed::scene_lists(s);
    for list in &mut s.installed.notes.extras.lists {
        list.random_each_launch = true;
    }
}

/// A modded launch with two map mods on: the question.
pub fn map_mods(s: &mut State) {
    crate::app::scene_demo_installed(s);
    s.launch_extras.ask = Some(crate::screens::launch_extras::MapAsk {
        slots: vec![3, 7],
        page: false,
        dont_ask: false,
    });
    crate::statusbar::refresh(s);
}

/// Stats with the filter on Ranked, last 30 days, and Items and Party read.
pub fn stats_more(s: &mut State) {
    use deadlock_core::stats::items::ItemStat;
    use deadlock_core::stats::party::LineupRating;
    crate::screens::stats::scene_people(s);
    s.stats.remembered.push(crate::screens::stats::Remembered {
        account_id: 1_382_044,
        name: "Juniper".to_string(),
        primary: false,
    });
    let more = &mut s.stats.more;
    more.ranked = Some(true);
    more.days = 30;
    let item = |id: u64, matches: u64, rate: f64, global: f64, buy: f64, global_buy: f64| ItemStat {
        item_id: id,
        matches,
        wins: (matches as f64 * rate / 100.0) as u64,
        losses: matches - (matches as f64 * rate / 100.0) as u64,
        players: 1,
        win_rate_percent: rate,
        global_win_rate_percent: global,
        win_rate_delta_percent: rate - global,
        avg_buy_time_s: buy,
        global_avg_buy_time_s: global_buy,
        buy_time_delta_s: buy - global_buy,
    };
    let rows = vec![
        item(11, 64, 58.0, 51.2, 312.0, 365.0),
        item(12, 41, 46.0, 50.4, 620.0, 540.0),
        item(13, 33, 61.0, 52.0, 905.0, 990.0),
        item(14, 18, 50.0, 49.8, 1210.0, 1180.0),
    ];
    let names = [(11, "Rapid Recharge"), (12, "Extra Health"), (13, "Mystic Burst"), (14, "Boundless Spirit")]
        .into_iter()
        .map(|(id, name)| (id, name.to_string()))
        .collect();
    more.items = crate::screens::stats::Fetched::Ready(crate::screens::stats::more_items(rows, names));
    let first = s.stats.snapshot.as_ref().and_then(|snap| snap.heroes.first().map(|h| h.hero_id)).unwrap_or(1);
    let second = s.stats.snapshot.as_ref().and_then(|snap| snap.heroes.get(1).map(|h| h.hero_id)).unwrap_or(2);
    let lineup = |a: u32, b: u32, rate: Option<f64>, matches: Option<u64>, sample: u64| LineupRating {
        account_ids: vec![1, 2],
        hero_ids: vec![a, b],
        win_rate_percent: rate,
        matches,
        sample_matches: sample,
        estimated: false,
    };
    let more = &mut s.stats.more;
    if let Some(mate) = s.stats.remembered.first().map(|r| r.account_id) {
        more.members.insert(mate);
    }
    more.party = crate::screens::stats::Fetched::Ready(vec![
        lineup(first, second, Some(54.2), Some(4_812), 4_812),
        lineup(second, first, Some(51.0), Some(1_920), 1_920),
        lineup(first, first + 1, Some(47.5), Some(120), 120),
    ]);
}

/// The Locker's General page on Cursors: Deadlock's own set and two packs, the second on.
pub fn locker_cursors(s: &mut State) {
    crate::app::scene_locker(s);
    s.locker.page = crate::screens::locker::Page::General;
    s.locker.general_tab = Some(crate::screens::locker::GeneralTab::Cursors);
    crate::screens::locker::cursors::stage_scene(s);
}

/// The review with its repairs: two fixable mods, a split archive, one repaired mod.
pub fn repairs(s: &mut State) {
    health_outdated(s);
    crate::screens::installed::repair::stage_scene(s, false);
}

/// The same, with Fix all asking about another mod's animations.
pub fn repairs_ask(s: &mut State) {
    health_outdated(s);
    crate::screens::installed::repair::stage_scene(s, true);
}

/// The install dialog while its download runs: Cancel stops it.
pub fn install_cancel(s: &mut State) {
    crate::screens::browse::scene_dashboard(s);
    let mut view = crate::screens::browse::InstallView::for_mod(600_009, "Taokaka over Drifter");
    view.busy = true;
    view.log = Some("Downloading, extracting and validating on a worker.".to_string());
    view.cancel = Some(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)));
    s.install = view;
}

/// The first-start telemetry question over Browse.
pub fn telemetry_consent(s: &mut State) {
    crate::screens::browse::scene_dashboard(s);
    crate::screens::settings::telemetry::stage_scene(s, false);
}

/// Settings > Privacy with Help improve Sigil, usage and crashes on.
pub fn telemetry_privacy(s: &mut State) {
    crate::screens::settings::scene_tab(s, Tab::Privacy);
    s.settings.telemetry.loaded = true;
    s.settings.telemetry.consent =
        deadlock_core::telemetry::Consent { asked: true, usage: true, crashes: true, hash_lookup: false };
}

/// Send diagnostics: the scrubbed preview over Settings > Logs.
pub fn telemetry_diagnostics(s: &mut State) {
    crate::screens::settings::scene_tab(s, Tab::Logs);
    crate::screens::settings::telemetry::stage_scene(s, true);
}
