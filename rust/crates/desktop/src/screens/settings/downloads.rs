//! Settings > Mods > Downloads: which GameBanana file server mod downloads use, the server
//! status and a speed test (Grimoire's Download servers, MIT), trying the fastest server
//! first (DMM's routing), faster downloads in parts, and where the download cache lives.

use std::path::PathBuf;
use std::time::Instant;

use deadlock_core::file_servers::{self, Directory, Probe};
use rune::{El, FileDialogOptions, Tx, col, icon, row, text};

use super::i18n::{fill, tr};
use super::options;
use super::{button, detail, disabled, path_detail, say, section, segmented, setting_row, toggle_row, trailing};
use crate::app::State;
use crate::icons;
use crate::ui::{ActionKind, BODY_MED, MONO, SMALL, Ui};

/// The file server downloads go to: the user's pick, else the fastest tested one when
/// "Fastest server first" is on, else None (GameBanana decides).
pub fn effective_host(o: &options::Options) -> Option<String> {
    if o.download_server != "auto" {
        return Some(format!("{}.gamebanana.com", o.download_server));
    }
    if o.mirror_first {
        return o.fastest_server.as_ref().map(|s| format!("{s}.gamebanana.com"));
    }
    None
}

/// Push the download choices into the core. At startup and after every change.
pub fn apply() {
    let o = options::get();
    deadlock_core::net::set_download_host(effective_host(&o));
    deadlock_core::net::set_download_parts(o.parts);
}

fn download_cache_dir() -> PathBuf {
    deadlock_core::install::downloads_dir(&crate::data::workspace_dir())
}

pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    let x = &mut st.settings.extra;
    if let Some(result) = x.dir_job.try_take() {
        x.directory = Some(result);
        x.dir_at = Some(Instant::now());
        changed = true;
    }
    if let Some(result) = x.test_job.try_take() {
        if let Ok(probes) = &result
            && let Some(best) = file_servers::fastest(probes)
        {
            let name = best.name.clone();
            options::edit_quiet(|o| o.fastest_server = Some(name));
            apply();
        }
        st.settings.extra.probes = Some(result);
        changed = true;
    }
    if let Some(result) = st.settings.extra.cache_job.try_take() {
        match result {
            Ok((dir, bytes)) => {
                options::edit_quiet(|o| o.cache_dir = dir.clone());
                let line = match dir {
                    Some(d) => fill(tr("Moved {} to {}."), &[&super::size_label(bytes), &d]),
                    None => fill(tr("Moved {} back."), &[&super::size_label(bytes)]),
                };
                say(st, line, false);
            }
            Err(error) => say(st, error, true),
        }
        changed = true;
    }
    changed
}

pub fn refresh_directory(st: &mut State) {
    if st.demo || rune::is_headless() {
        st.settings.extra.directory = Some(Ok(demo_directory()));
        st.settings.extra.dir_at = Some(Instant::now());
        return;
    }
    let wake = st.wake.clone();
    st.settings.extra.dir_job.start(wake, || file_servers::fetch_directory().map_err(|e| e.to_string()));
}

fn run_test(st: &mut State) {
    let Some(Ok(dir)) = st.settings.extra.directory.clone() else {
        return refresh_directory(st);
    };
    if st.demo || rune::is_headless() {
        st.settings.extra.probes = Some(Ok(demo_probes()));
        return;
    }
    let wake = st.wake.clone();
    st.settings.extra.test_job.start(wake, move || file_servers::test_servers(&dir).map_err(|e| e.to_string()));
}

fn ago(at: Option<Instant>) -> String {
    match at.map(|t| t.elapsed().as_secs()) {
        None => String::new(),
        Some(s) if s < 60 => tr("just now").to_string(),
        Some(s) => fill(tr("{} min ago"), &[&(s / 60).to_string()]),
    }
}

pub fn section_el(s: &State, u: &Ui) -> El {
    let o = options::get();
    let x = &s.settings.extra;
    // Status of the servers.
    let status = match &x.directory {
        None if x.dir_job.is_running() => tr("Checking GameBanana").to_string(),
        None => tr("Not checked yet.").to_string(),
        Some(Err(error)) => fill(tr("Could not reach GameBanana: {}"), &[error]),
        Some(Ok(d)) => fill(
            tr("{} of {} online, checked {}."),
            &[&d.available.to_string(), &d.total.to_string(), &ago(x.dir_at)],
        ),
    };
    let degraded = matches!(&x.directory, Some(Ok(d)) if !d.healthy()) || matches!(&x.directory, Some(Err(_)));
    let mut rows = vec![setting_row(
        u,
        "settings-servers-status",
        "GameBanana file servers",
        text(status).font(SMALL).color(if degraded { u.amber } else { u.muted }).truncate(),
        trailing(vec![
            u.icon_button(&icons::EXTERNAL_LINK, tr("Open GameBanana's status page"))
                .key("settings-servers-page")
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(|st: &mut State, tx: &mut Tx| super::tabs::open_url(st, tx, file_servers::STATUS_PAGE)),
            disabled(
                u.icon_button(&icons::REFRESH_CW, tr("Refresh status"))
                    .key("settings-servers-refresh")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(refresh_directory),
                x.dir_job.is_running() || s.prefs.offline,
            ),
        ]),
    )];

    // The choice: Automatic, then each online server.
    let auto_line = match (&o.fastest_server, o.mirror_first) {
        (Some(f), true) => fill(tr("Fastest tested first: {}."), &[f]),
        _ => tr("GameBanana picks a server for each download.").to_string(),
    };
    let mut choices = vec![("auto".to_string(), tr("Automatic").to_string(), auto_line)];
    if let Some(Ok(d)) = &x.directory {
        for server in d.servers.iter().filter(|s| s.up).take(6) {
            let tested = match &x.probes {
                Some(Ok(p)) => p
                    .iter()
                    .find(|p| p.name == server.name)
                    .map(|p| match p.bytes_per_second {
                        Some(b) => fill(tr("Tested here: {}"), &[&file_servers::speed_label(b)]),
                        None => tr("Tested here: unavailable").to_string(),
                    }),
                _ => None,
            };
            let line = tested.unwrap_or_else(|| {
                fill(tr("{} on GameBanana in the last 10 minutes"), &[&file_servers::speed_label(server.rate)])
            });
            choices.push((server.name.clone(), server.name.clone(), line));
        }
    }
    if o.download_server != "auto" && !choices.iter().any(|(id, _, _)| *id == o.download_server) {
        choices.push((o.download_server.clone(), o.download_server.clone(), tr("Chosen earlier").to_string()));
    }
    let mut picker = col().w_full().key("settings-server-pick").role("radiogroup").label(tr("Download server"));
    for (index, (id, title, line)) in choices.into_iter().enumerate() {
        let on = o.download_server == id;
        let pick = id.clone();
        picker = picker.child(
            row()
                .key(format!("settings-server-{index}"))
                .w_full()
                .h(40.0)
                .gap(12.0)
                .center_y()
                .focusable(true)
                .role("radio")
                .checked(on)
                .label(title.clone())
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    let id = pick.clone();
                    options::edit(st, |o| o.download_server = id);
                    apply();
                })
                .child(
                    rune::el()
                        .size(16.0, 16.0)
                        .radius(8.0)
                        .border(if on { 5.0 } else { 1.5 }, if on { u.accent } else { u.hair_strong })
                        .a11y_hidden(),
                )
                .child(text(title).font(BODY_MED).color(u.ink).w(140.0))
                .child(text(line).font(SMALL).color(u.muted).grow(1.0).min_w(0.0).truncate()),
        );
    }
    rows.push(
        col()
            .key("settings-server-choice")
            .w_full()
            .py(10.0)
            .gap(6.0)
            .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
            .child(text(tr("Download server")).font(BODY_MED).color(u.ink))
            .child(picker),
    );

    let testing = x.test_job.is_running();
    let test_line = match &x.probes {
        _ if testing => tr("Testing 3 servers").to_string(),
        Some(Err(error)) => error.clone(),
        Some(Ok(probes)) => match file_servers::fastest(probes) {
            Some(best) => fill(
                tr("Fastest here: {} at {}."),
                &[&best.name, &file_servers::speed_label(best.bytes_per_second.unwrap_or(0.0))],
            ),
            None => tr("No server answered.").to_string(),
        },
        None => tr("Reads up to 256 KB from each of 3 servers.").to_string(),
    };
    rows.push(setting_row(
        u,
        "settings-servers-test",
        "Speed test",
        detail(u, &test_line),
        disabled(
            button(u, "settings-servers-test-run", if testing { "Testing" } else { "Test now" }, ActionKind::Secondary)
                .on_press(run_test),
            testing || s.prefs.offline || crate::screens::browse::quick::installs_busy(s),
        ),
    ));
    rows.push(toggle_row(
        u,
        "settings-mirror-first",
        "Fastest server first",
        "Downloads try the fastest tested server, then the one GameBanana picked.",
        o.mirror_first,
        |st| {
            options::edit(st, |o| o.mirror_first = !o.mirror_first);
            apply();
        },
    ));
    let parts: Vec<(u32, &str)> = vec![(1, "Off"), (4, "4 parts"), (8, "8 parts")];
    rows.push(setting_row(
        u,
        "settings-parts",
        "Faster downloads",
        detail(u, "Files of 8 MB or more come in parts at once, when the server allows it."),
        segmented(u, "settings-parts-pick", &parts, o.parts, |st, n| {
            options::edit(st, |o| o.parts = n);
            apply();
        }),
    ));
    let cache = download_cache_dir();
    let moved = o.cache_dir.clone();
    let shown = moved.clone().unwrap_or_else(|| super::tidy(&cache.display().to_string()));
    let moving = x.cache_job.is_running();
    rows.push(setting_row(
        u,
        "settings-cache-move",
        "Download cache",
        path_detail(u, &shown),
        trailing(vec![
            disabled(
                button(u, "settings-cache-move-run", if moving { "Moving" } else { "Move" }, ActionKind::Secondary)
                    .on_press_tx(choose_cache_dir),
                moving || moved.is_some(),
            ),
            disabled(
                button(u, "settings-cache-back", "Move back", ActionKind::Ghost).on_press(move_back),
                moving || moved.is_none(),
            ),
        ]),
    ));
    section(u, "Downloads", rows)
}

fn choose_cache_dir(_st: &mut State, tx: &mut Tx) {
    let options = FileDialogOptions {
        title: tr("Choose where the download cache goes").into(),
        directory: super::editor::dialog_dir(),
        ..Default::default()
    };
    tx.open_folder(options, |st: &mut State, result, _tx| match result {
        Ok(Some(path)) => {
            super::editor::remember(&path);
            start_move(st, Some(path));
        }
        Ok(None) => {}
        Err(error) => say(st, error.to_string(), true),
    });
}

fn move_back(st: &mut State) {
    start_move(st, None);
}

/// Move the cache to `to` (a `Sigil downloads` folder inside it), or back.
fn start_move(st: &mut State, to: Option<PathBuf>) {
    if st.demo || rune::is_headless() {
        return say(st, tr("Demo data: nothing was changed."), false);
    }
    if crate::screens::browse::quick::installs_busy(st) {
        return say(st, tr("Wait for the current download to finish."), true);
    }
    let cache = download_cache_dir();
    let wake = st.wake.clone();
    match to {
        Some(picked) => {
            let target = if picked.file_name().is_some_and(|n| n == "Sigil downloads") {
                picked
            } else {
                picked.join("Sigil downloads")
            };
            st.settings.extra.cache_job.start(wake, move || {
                std::fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
                let bytes = deadlock_core::mod_data::relocate(&cache, &target).map_err(|e| e.to_string())?;
                Ok((Some(target.display().to_string()), bytes))
            });
        }
        None => {
            let Some(from) = options::get().cache_dir.clone().map(PathBuf::from) else { return };
            st.settings.extra.cache_job.start(wake, move || {
                let bytes = deadlock_core::mod_data::relocate_back(&cache, &from).map_err(|e| e.to_string())?;
                let _ = std::fs::remove_dir(&from);
                Ok((None, bytes))
            });
        }
    }
}

fn demo_directory() -> Directory {
    let server = |n: &str, rate: f64| file_servers::Server {
        name: n.into(),
        up: true,
        rate,
    };
    Directory {
        servers: vec![server("filecache9", 4_200_000.0), server("filecache4", 2_900_000.0), server("filecache12", 1_100_000.0)],
        available: 3,
        total: 3,
    }
}

fn demo_probes() -> Vec<Probe> {
    vec![
        Probe { name: "filecache9".into(), bytes_per_second: Some(6_100_000.0) },
        Probe { name: "filecache4".into(), bytes_per_second: Some(3_400_000.0) },
        Probe { name: "filecache12".into(), bytes_per_second: None },
    ]
}

/// A small check icon, for rows that mark the current choice.
#[allow(dead_code)]
fn check(u: &Ui) -> El {
    icon(&icons::CHECK, 14.0).color(u.accent).a11y_hidden()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_host_follows_choice_then_fastest() {
        let mut o = options::Options::default();
        assert_eq!(effective_host(&o), None);
        o.fastest_server = Some("filecache4".into());
        assert_eq!(effective_host(&o), None, "automatic without fastest-first leaves it to GameBanana");
        o.mirror_first = true;
        assert_eq!(effective_host(&o).as_deref(), Some("filecache4.gamebanana.com"));
        o.download_server = "filecache9".into();
        assert_eq!(effective_host(&o).as_deref(), Some("filecache9.gamebanana.com"));
    }
}
