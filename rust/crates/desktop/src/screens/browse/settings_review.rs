//! The review of settings files that come inside a mod: a `gameinfo.gi`, `.cfg` files and a
//! `video.txt` (Deadlock Mod Manager's staged review). None is installed as it ships. The dialog
//! shows what each would change and hands it to the page that owns that file, where the usual
//! confirm writes it:
//!
//! - gameinfo.gi and video.txt: cached as Performance's "Config from a mod", opened there, and
//!   applied with Performance's Apply (a tagged merge, Turn off takes it out again);
//! - .cfg: the commands are added to the Autoexec page; its Save writes autoexec.cfg.
//!
//! Reading runs on a worker after an install (or a refused one) and never writes to the game.

use std::path::PathBuf;

use deadlock_core::autoexec::Placement;
use deadlock_core::mod_settings::{self, Review};
use rune::{El, Key, KeyEvent, col, row, spacer, text};

use crate::app::State;
use crate::jobs::Job;
use crate::model::Screen;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{ActionKind, BODY, BODY_MED, H1, MICRO, MONO, SMALL, Ui};

/// Lines shown per file before "and N more".
const SHOWN: usize = 6;

#[derive(Default)]
pub struct SettingsReview {
    pub open: Option<Review>,
    /// The mod had nothing else to install: the review is all there is.
    pub only: bool,
    read: Job<(Review, bool)>,
    stage: Job<()>,
    pub message: Option<String>,
}

/// Read the settings files of an unpacked archive on a worker. When there are any, a notice
/// offers the review (`only`: the archive had nothing else, so the review opens at once).
pub fn start(st: &mut State, dir: PathBuf, name: &str, mod_id: Option<u64>, only: bool) {
    let Some(game_root) = st.game_root.clone() else { return };
    if st.demo || rune::is_headless() || st.browse_view.installs.settings.read.is_running() {
        return;
    }
    let name = name.to_string();
    let wake = st.wake.clone();
    st.browse_view.installs.settings.read.start(wake, move || {
        let mut review = mod_settings::review_dir(&dir, &game_root, &deadlock_core::discovery::default_steam_roots());
        review.mod_name = name;
        review.mod_id = mod_id;
        Ok((review, only))
    });
}

/// The unpacked archive of an install target, where the plan left it.
pub fn extracted_dir(target: &str) -> Option<PathBuf> {
    let source = deadlock_core::install::InstallSource::parse(target).ok()?;
    Some(deadlock_core::install::download_dir(&crate::data::workspace_dir(), &source).join("extracted"))
}

pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(Ok((review, only))) = st.browse_view.installs.settings.read.try_take() {
        changed = true;
        if !review.is_empty() {
            let name = review.mod_name.clone();
            if only {
                st.browse_view.installs.settings.only = true;
                st.browse_view.installs.settings.open = Some(review);
            } else {
                crate::notify::push(
                    st,
                    crate::notify::Notice::info(fill(tr("{} also ships settings files"), &[&name]))
                        .detail(tr("A gameinfo.gi, .cfg or video.txt came with it. Sigil did not copy them; review what they would change."))
                        .action(tr("Review"), move |st: &mut State| {
                            st.browse_view.installs.settings.only = false;
                            st.browse_view.installs.settings.open = Some(review);
                        }),
                );
            }
        }
    }
    if let Some(result) = st.browse_view.installs.settings.stage.try_take() {
        changed = true;
        match result {
            Ok(()) => {
                let data = crate::data::data_dir();
                st.performance.catalog = Some(deadlock_core::performance::load(&data));
                st.performance.open = Some(mod_settings::CONFIG_ID.to_string());
                st.browse_view.installs.settings = SettingsReview::default();
                st.go(Screen::Performance);
                crate::notify::push(
                    st,
                    crate::notify::Notice::success(tr("The mod's settings are on the Performance page"))
                        .detail(tr("Press Apply there to write them into your gameinfo.gi. Turn off takes them out again.")),
                );
            }
            Err(error) => st.browse_view.installs.settings.message = Some(error),
        }
    }
    changed
}

fn close(st: &mut State) {
    st.browse_view.installs.settings.open = None;
    st.browse_view.installs.settings.message = None;
}

/// Cache the gameinfo (or the video.txt alone) for Performance, then open it there.
fn to_performance(st: &mut State, gameinfo: Option<usize>) {
    let Some(review) = st.browse_view.installs.settings.open.clone() else { return };
    if st.browse_view.installs.settings.stage.is_running() {
        return;
    }
    let data = crate::data::data_dir();
    let wake = st.wake.clone();
    st.browse_view.installs.settings.stage.start(wake, move || {
        mod_settings::stage(&review, gameinfo, &data).map_err(|e| e.to_string())
    });
}

/// Add the commands to the Autoexec page; its Save writes them, with its confirm.
fn to_autoexec(st: &mut State) {
    let Some(review) = st.browse_view.installs.settings.open.as_ref() else { return };
    let commands = review.new_commands();
    if !st.autoexec.loaded {
        crate::data::load_autoexec_now(st);
    }
    for command in &commands {
        st.autoexec.add_preset(command);
    }
    close(st);
    st.go(Screen::Autoexec);
    st.status = fill(tr("{} commands added to Autoexec. Press Save to write them."), &[&commands.len().to_string()]);
}

fn change_line(u: &Ui, key: &str, from: Option<&str>, to: &str) -> El {
    let words = match from {
        Some(from) => format!("{key}  {from} \u{2192} {to}"),
        None => format!("{key}  {to}"),
    };
    text(words).font(MONO).color(u.prose).truncate().w_full()
}

fn more(u: &Ui, count: usize) -> Option<El> {
    (count > SHOWN).then(|| text(fill(tr("and {} more"), &[&(count - SHOWN).to_string()])).font(MICRO).color(u.faint))
}

fn block(u: &Ui, key: String, title: String, detail: String, lines: Vec<El>, action: El) -> El {
    let mut body = col().w_full().gap(3.0);
    for line in lines {
        body = body.child(line);
    }
    col()
        .key(key)
        .w_full()
        .gap(8.0)
        .padding(12.0)
        .radius(crate::ui::r(8.0))
        .fill(u.well)
        .border(1.0, u.hair)
        .child(
            row()
                .w_full()
                .gap(10.0)
                .items_start()
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(text(title).font(BODY_MED).color(u.ink).wrap())
                        .child(text(detail).font(SMALL).color(u.muted).wrap()),
                )
                .child(action),
        )
        .child(body)
}

/// The review dialog, when one is open.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    let review = s.browse_view.installs.settings.open.as_ref()?;
    let state = &s.browse_view.installs.settings;
    let busy = state.stage.is_running();
    let mut body = col().w_full().gap(10.0);
    let intro = if state.only {
        tr("This mod is not a VPK: it changes game settings. Sigil never copies these files into the game. Here is what they would change; each one goes through the page that owns that file.")
    } else {
        tr("These files came with the mod. Sigil did not copy them into the game. Here is what they would change; each one goes through the page that owns that file.")
    };
    body = body.child(text(intro).font(BODY).color(u.prose).wrap());
    for (index, gameinfo) in review.gameinfo.iter().enumerate() {
        let mut lines: Vec<El> = gameinfo
            .changes
            .iter()
            .take(SHOWN)
            .map(|c| change_line(u, &c.key, c.from.as_deref(), &c.to))
            .collect();
        lines.extend(more(u, gameinfo.changes.len()));
        let mut detail = fill(tr("{} settings, {} engine edits"), &[&gameinfo.changes.len().to_string(), &gameinfo.engine.to_string()]);
        if !gameinfo.videos.is_empty() {
            detail = format!("{detail}. {}", fill(tr("With {}"), &[&gameinfo.videos.join(", ")]));
        }
        let action = u
            .action(tr("Review in Performance"), ActionKind::Primary)
            .key(format!("settings-review-perf-{index}"))
            .opacity(if busy { 0.5 } else { 1.0 })
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| to_performance(st, Some(index)));
        body = body.child(block(u, format!("settings-review-gameinfo-{index}"), gameinfo.path.clone(), detail, lines, action));
    }
    if review.gameinfo.is_empty() {
        for (index, video) in review.videos.iter().enumerate() {
            let mut lines: Vec<El> =
                video.changes.iter().take(SHOWN).map(|c| change_line(u, &c.key, c.from.as_deref(), &c.to)).collect();
            lines.extend(more(u, video.changes.len()));
            let action = u
                .action(tr("Review in Performance"), ActionKind::Primary)
                .key(format!("settings-review-video-{index}"))
                .opacity(if busy { 0.5 } else { 1.0 })
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| to_performance(st, None));
            let detail = fill(tr("{} graphics settings differ from yours"), &[&video.changes.len().to_string()]);
            body = body.child(block(u, format!("settings-review-video-{index}"), video.path.clone(), detail, lines, action));
        }
    }
    if !review.configs.is_empty() {
        let mut lines = Vec::new();
        let mut total = 0;
        for config in &review.configs {
            for command in &config.commands {
                total += 1;
                if lines.len() < SHOWN {
                    let tag = match command.placement {
                        Placement::Present(_) => tr("already there"),
                        Placement::Replace(_) => tr("replaces yours"),
                        Placement::Append => tr("new"),
                    };
                    lines.push(
                        row()
                            .w_full()
                            .gap(8.0)
                            .child(text(command.line.clone()).font(MONO).color(u.prose).truncate().grow(1.0).min_w(0.0))
                            .child(text(tag).font(MICRO).color(u.faint)),
                    );
                }
            }
        }
        lines.extend(more(u, total));
        let new = review.new_commands().len();
        let names: Vec<&str> = review.configs.iter().map(|c| c.path.as_str()).collect();
        let action = u
            .action(tr("Add to Autoexec"), if review.gameinfo.is_empty() { ActionKind::Primary } else { ActionKind::Secondary })
            .key("settings-review-autoexec")
            .opacity(if new == 0 { 0.5 } else { 1.0 })
            .cursor(if new == 0 { rune::Cursor::Default } else { rune::Cursor::Pointer })
            .on_press(move |st: &mut State| {
                if new > 0 {
                    to_autoexec(st);
                }
            });
        let detail = fill(tr("{} commands, {} not in your autoexec yet"), &[&total.to_string(), &new.to_string()]);
        body = body.child(block(u, "settings-review-configs".to_string(), names.join(", "), detail, lines, action));
    }
    if let Some(message) = &state.message {
        body = body.child(text(message.clone()).font(SMALL).color(u.rose).wrap());
    }
    body = body.child(
        row().w_full().pt(4.0).child(spacer()).child(
            u.action(tr("Close"), ActionKind::Ghost)
                .key("settings-review-close")
                .cursor(rune::Cursor::Pointer)
                .on_press(close),
        ),
    );
    let title = fill(tr("Settings files in {}"), &[&review.mod_name]);
    Some(
        u.dialog_shell()
            .key("settings-review-dialog")
            .role("dialog")
            .label(title.clone())
            .w(620.0)
            .padding(24.0)
            .gap(12.0)
            .modal(true, close)
            .on_key(|st: &mut State, ev: &KeyEvent, _| {
                if ev.key == Key::Escape {
                    close(st);
                    true
                } else {
                    false
                }
            })
            .child(text(title).font(H1).color(u.ink).wrap())
            .child(body),
    )
}

/// A headless scene: a config mod with a gameinfo, a video.txt and an autoexec.
pub fn stage_scene(st: &mut State) {
    use deadlock_core::mod_settings::{Change, Command, Config, Gameinfo, Video};
    let change = |key: &str, from: Option<&str>, to: &str| Change { key: key.into(), from: from.map(Into::into), to: to.into() };
    st.browse_view.installs.settings.only = true;
    st.browse_view.installs.settings.open = Some(Review {
        mod_name: "Potato FPS config".into(),
        mod_id: Some(512345),
        gameinfo: vec![Gameinfo {
            path: "Potato/gameinfo.gi".into(),
            changes: vec![
                change("r_shadows", Some("1"), "0"),
                change("r_farz", Some("10000"), "6000"),
                change("r_citadel_fog", None, "0"),
                change("cl_particle_fallback_base", None, "4"),
                change("r_ssao", Some("1"), "0"),
                change("r_citadel_outlines", Some("1"), "0"),
                change("r_grass_quality", Some("2"), "0"),
                change("r_texture_stream_mip_bias", None, "1"),
            ],
            engine: 2,
            videos: vec!["video.txt".into()],
        }],
        configs: vec![Config {
            path: "cfg/autoexec.cfg".into(),
            commands: vec![
                Command { line: "fps_max 240".into(), placement: Placement::Replace(0) },
                Command { line: "cl_showfps 1".into(), placement: Placement::Append },
                Command { line: "snd_mute_losefocus 0".into(), placement: Placement::Present(3) },
            ],
        }],
        videos: vec![Video { path: "Potato/video.txt".into(), changes: vec![change("setting.r_texture_quality", Some("2"), "0")] }],
        dir: PathBuf::new(),
        stock: String::new(),
    });
}

// Headless scenes for this round's install states (listed in `servers_regions/scenes.rs`).

pub fn scene_review(st: &mut State) {
    st.load_demo();
    st.screen = Screen::Browse;
    stage_scene(st);
}

pub fn scene_landed_off(st: &mut State) {
    st.load_demo();
    st.screen = Screen::Browse;
    crate::screens::installed::safety_gate::landed_off(
        st,
        7,
        "Neon HUD",
        vec!["Script: panorama/scripts/hud.vjs_c".into(), "Reaches out: panorama/scripts/net.vjs_c ($.AsyncWebRequest)".into()],
    );
}

pub fn scene_resume(st: &mut State) {
    st.load_demo();
    st.screen = Screen::Browse;
    let mut a = deadlock_core::download_queue::Item::new("d3".into(), "715100".into());
    a.name = "Wraith Neon".into();
    a.state = deadlock_core::download_queue::State::Paused;
    a.interrupted = true;
    a.install = true;
    let mut b = deadlock_core::download_queue::Item::new("d4".into(), "715101".into());
    b.name = "Ivy Gold".into();
    crate::screens::downloads::offer_resume(st, vec![a, b]);
}

pub fn scene_file_choice(st: &mut State) {
    crate::screens::browse::scene_details(st);
    if let Some(id) = st.browse_view.details.as_ref().map(|d| d.id) {
        st.browse_view.installs.files_menu = Some(id);
    }
}

pub fn scene_live_update(st: &mut State) {
    st.load_demo();
    st.screen = Screen::Stats;
    crate::screens::stats::live::scene_update_ask(st);
}
