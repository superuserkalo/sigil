//! Runtime state and dialogs for the settings added in the parity round, plus the small rows
//! that do not need a module of their own (custom Steam folder, text editor, tracker sites,
//! contributors). `settings.rs` holds one `Extra` in its `View` and calls `poll`, `dialog`
//! and `overlay` from here.

use std::collections::BTreeSet;
use std::path::PathBuf;

use rune::{El, FileDialogOptions, Key, KeyEvent, TextEdit, Tx, col, row, spacer, text};

use super::i18n::{fill, tr};
use super::options;
use super::{Tab, button, detail, disabled, path_detail, say, section, setting_row, trailing};
use crate::app::State;
use crate::icons;
use crate::jobs::Job;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, SMALL, Ui};

pub enum Dialog {
    Editor,
    Crash(PathBuf),
    RestoreSnapshot(PathBuf),
    DeleteSnapshot(PathBuf),
    ClearAllMods,
    ResetRecords,
    WipeData,
    Recovery,
    CopyDev(PathBuf),
    SourceAdd(Box<super::sources::Synced>),
    SourceBrowse(String),
    SourceRemove(String),
    /// A `deadlock-mod-manager://sources/...` link, before the player has confirmed anything.
    SourceLink(Box<deadlock_core::mod_sources::SourceLink>),
    Templates,
    AllowScripts,
    Contributors,
}

#[derive(Default)]
pub struct Extra {
    pub dialog: Option<Dialog>,
    pub watch: super::watch::Watch,
    pub dir_job: Job<deadlock_core::file_servers::Directory>,
    pub directory: Option<Result<deadlock_core::file_servers::Directory, String>>,
    pub dir_at: Option<std::time::Instant>,
    pub test_job: Job<Vec<deadlock_core::file_servers::Probe>>,
    pub probes: Option<Result<Vec<deadlock_core::file_servers::Probe>, String>>,
    pub cache_job: Job<(Option<String>, u64)>,
    pub sources: super::sources::Sources,
    pub net: super::netx::Net,
    pub support: super::support::Support,
    pub tools: super::tools::Tools,
    pub draft: Option<super::templates::Draft>,
    pub forge: super::forge::Bridge,
    pub onboarding: Option<super::onboarding::Step>,
    /// Wave 4: the page the setup opens when it finishes (Installed after the mods step).
    pub after_onboarding: Option<crate::model::Screen>,
    pub onboarding_error: Option<String>,
    pub last_screen: Option<String>,
    pub editors: Option<Vec<super::editor::Found>>,
    pub contributors_job: Job<Vec<(String, String)>>,
    pub contributors: Option<Result<Vec<(String, String)>, String>>,
    pub requirements_job: Job<(u64, Vec<deadlock_core::requirements::Requirement>)>,
    pub requirements: Option<(u64, Vec<deadlock_core::requirements::Requirement>)>,
    pub started: bool,
    /// Mods whose "needs" were already looked up this run.
    pub asked: BTreeSet<u64>,
    /// Scenes only: show just this round's sections of a tab.
    pub scene_only: Option<Tab>,
    pub scene_page: u8,
}

/// Open the settings sheet on a tab.
pub fn open_on(st: &mut State, tab: Tab) {
    st.settings.tab = tab;
    st.settings.search = TextEdit::new("");
    super::open(st);
}

pub fn close(st: &mut State) {
    st.settings.extra.dialog = None;
}

/// Startup for everything in this round: options reach the core, the watcher, the tray,
/// the forge listener and the first-run setup.
pub fn on_startup(st: &mut State) {
    super::downloads::apply();
    super::netx::on_startup();
    super::pages::apply();
    super::onboarding::on_startup(st);
}

/// What a tab reads when it opens.
pub fn on_select(st: &mut State, tab: Tab) {
    match tab {
        Tab::Network => super::netx::on_open(st),
        Tab::Logs => super::support::read_crashes(st),
        Tab::Backups | Tab::Storage | Tab::Experimental => super::tools::read(st),
        Tab::Mods => {
            if st.settings.extra.directory.is_none() && !st.prefs.offline {
                super::downloads::refresh_directory(st);
            }
        }
        _ => {}
    }
}

pub fn poll(st: &mut State, tx: &mut Tx) -> bool {
    let mut changed = false;
    if !st.settings.extra.started {
        st.settings.extra.started = true;
        on_startup(st);
        super::tools::warn_unknown(st);
    }
    changed |= super::watch::poll(st, tx);
    changed |= super::downloads::poll(st);
    changed |= super::sources::poll(st);
    changed |= super::netx::poll(st);
    changed |= super::tools::poll(st);
    changed |= super::onboarding::poll_tips(st);
    if let Some(result) = st.settings.extra.contributors_job.try_take() {
        st.settings.extra.contributors = Some(result);
        changed = true;
    }
    if let Some(Ok(found)) = st.settings.extra.requirements_job.try_take() {
        st.settings.extra.requirements = Some(found);
        changed = true;
    }
    changed |= poll_requirements(st);
    let _ = tx;
    changed
}

/// Look up what the mod in the install panel needs, once per mod.
fn poll_requirements(st: &mut State) -> bool {
    if !options::get().prompt_requirements || !st.install.open || st.prefs.offline || st.demo || rune::is_headless() {
        return false;
    }
    let Ok(id) = st.install.target.trim().parse::<u64>() else { return false };
    if st.settings.extra.asked.contains(&id) || st.settings.extra.requirements_job.is_running() {
        return false;
    }
    st.settings.extra.asked.insert(id);
    let wake = st.wake.clone();
    st.settings.extra.requirements_job.start(wake, move || {
        deadlock_core::requirements::fetch_requirements(id).map(|r| (id, r)).map_err(|e| e.to_string())
    });
    false
}

/// Rows for Browse's install panel: what the mod's page says it needs. Empty when nothing.
pub fn requirements_panel(s: &State, u: &Ui, mod_id: u64) -> Option<El> {
    if !options::get().prompt_requirements {
        return None;
    }
    let (id, list) = s.settings.extra.requirements.as_ref()?;
    if *id != mod_id || list.is_empty() {
        return None;
    }
    let mut c = col().w_full().gap(6.0).mt(6.0).child(text(tr("This mod needs")).font(BODY_MED).color(u.amber));
    for (index, r) in list.iter().enumerate() {
        let url = r.url.clone();
        let installed = r.mod_id.is_some_and(|m| s.installed.library.mods.iter().any(|a| a.gamebanana_id == Some(m)));
        c = c.child(
            row()
                .key(format!("install-needs-{index}"))
                .w_full()
                .gap(8.0)
                .center_y()
                .child(text(r.name.clone()).font(SMALL).color(u.prose).grow(1.0).min_w(0.0).truncate())
                .child(if installed {
                    text(tr("Installed")).font(SMALL).color(u.muted)
                } else if url.is_empty() {
                    rune::el()
                } else {
                    u.icon_button(&icons::EXTERNAL_LINK, tr("Open"))
                        .key(format!("install-needs-open-{index}"))
                        .cursor(rune::Cursor::Pointer)
                        .on_press_tx(move |st: &mut State, tx: &mut Tx| super::tabs::open_url(st, tx, &url))
                }),
        );
    }
    Some(c)
}

// --- dialog helpers ------------------------------------------------------------------------

/// A dialog card: title, then the caller's children.
pub fn shell(u: &Ui, key: &str, title: &str) -> El {
    u.dialog_shell()
        .key(key.to_string())
        .role("dialog")
        .label(title.to_string())
        .w(480.0)
        .padding(24.0)
        .gap(16.0)
        .modal(true, close)
        .on_key(|st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                close(st);
                true
            } else {
                false
            }
        })
        .child(text(title.to_string()).font(H1).color(u.ink))
}

/// Cancel and a primary (or danger) button, right-aligned.
pub fn buttons(u: &Ui, go: &str, danger: bool, run: impl Fn(&mut State) + 'static) -> El {
    row()
        .w_full()
        .gap(8.0)
        .pt(8.0)
        .child(spacer())
        .child(button(u, "settings-dialog-cancel", "Cancel", ActionKind::Ghost).on_press(close))
        .child(
            button(u, "settings-dialog-go", go, if danger { ActionKind::Danger } else { ActionKind::Primary })
                .on_press(run),
        )
}

fn confirm(u: &Ui, key: &str, title: &str, body: &str, go: &str, danger: bool, run: impl Fn(&mut State) + 'static) -> El {
    shell(u, key, tr(title))
        .child(text(tr(body).to_string()).font(BODY).color(u.muted).wrap().w_full())
        .child(buttons(u, go, danger, run))
}

/// The open dialog, if any. Shown above the settings sheet, or alone when it is closed.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    let d = s.settings.extra.dialog.as_ref()?;
    Some(match d {
        Dialog::Editor => editor_dialog(s, u),
        Dialog::Crash(path) => super::support::crash_dialog(s, u, path),
        Dialog::RestoreSnapshot(dir) => super::tools::restore_dialog(u, dir),
        Dialog::DeleteSnapshot(dir) => {
            let dir = dir.clone();
            confirm(u, "settings-snapshot-delete-dialog", "Delete this backup", "The copied mods in it are removed for good.", "Delete", true, move |st| {
                super::tools::delete_snapshot(st, dir.clone())
            })
        }
        Dialog::ClearAllMods => confirm(
            u,
            "settings-clear-mods-dialog",
            "Clear all mods",
            "Every mod leaves the addons folder for Recently deleted, where you can restore it. Files the game ignores stay.",
            "Clear all mods",
            true,
            super::tools::clear_all,
        ),
        Dialog::ResetRecords => confirm(
            u,
            "settings-reset-records-dialog",
            "Reset mod records",
            "Names, previews and install history are moved aside. Mods stay in the game and show as unknown until matched again.",
            "Reset",
            false,
            super::tools::reset_records,
        ),
        Dialog::WipeData => confirm(
            u,
            "settings-wipe-data-dialog",
            "Clear all mod data",
            "Deletes the download cache, install backups and Recently deleted for good, and moves Sigil's mod records aside. Mods in the game stay.",
            "Clear",
            true,
            super::tools::wipe_data,
        ),
        Dialog::Recovery => super::tools::recovery_dialog(s, u),
        Dialog::CopyDev(target) => {
            let t = target.clone();
            shell(u, "settings-copy-dev-dialog", tr("Copy mods to this folder"))
                .child(text(target.display().to_string()).font(crate::ui::MONO).color(u.prose).truncate())
                .child(
                    text(tr("Its addons folder will match this game's. Mods already there go to Recently deleted."))
                        .font(BODY)
                        .color(u.muted)
                        .wrap()
                        .w_full(),
                )
                .child(buttons(u, "Copy", false, move |st| super::tools::copy_dev(st, t.clone())))
        }
        Dialog::SourceAdd(synced) => super::sources::add_dialog(u, synced),
        Dialog::SourceBrowse(url) => super::sources::browse_dialog(s, u, url),
        Dialog::SourceRemove(url) => super::sources::remove_dialog(u, url),
        Dialog::SourceLink(link) => super::sources::link_dialog(s, u, link),
        Dialog::Templates => super::templates::dialog(s, u),
        Dialog::AllowScripts => super::pages::scripts_dialog(u),
        Dialog::Contributors => contributors_dialog(s, u),
    })
}

/// What shows over the window while the settings sheet is closed: the first-run setup, a
/// DeadlockForge request, or a dialog a notification opened.
pub fn overlay(s: &State, u: &Ui) -> Option<El> {
    if let Some(o) = super::onboarding::overlay(s, u) {
        return Some(o);
    }
    dialog(s, u).map(|d| u.modal_layer("settings-extra", d))
}

// --- Game: custom Steam folder ---------------------------------------------------------

pub fn steam_rows(s: &State, u: &Ui) -> El {
    let o = options::get();
    let line = match &o.steam_root {
        Some(dir) => dir.clone(),
        None => tr("Found on its own. Choose one if Steam lives somewhere unusual.").to_string(),
    };
    // The one Steam section: the folder, then the accounts on this computer.
    let mut rows = vec![setting_row(
            u,
            "settings-steam-folder",
            "Steam folder",
            if o.steam_root.is_some() { path_detail(u, &line) } else { detail(u, &line) },
            trailing(vec![
                button(u, "settings-steam-folder-choose", "Choose", ActionKind::Secondary).on_press_tx(choose_steam),
                disabled(
                    button(u, "settings-steam-folder-auto", "Automatic", ActionKind::Ghost).on_press(|st: &mut State| {
                        options::edit(st, |o| o.steam_root = None);
                        say(st, tr("Sigil finds Steam on its own again."), false);
                    }),
                    o.steam_root.is_none(),
                ),
            ]),
        )];
    rows.extend(super::tabs::steam_account_rows(s, u));
    section(u, "Steam", rows)
}

fn choose_steam(_st: &mut State, tx: &mut Tx) {
    let options = FileDialogOptions {
        title: tr("Choose the Steam folder").into(),
        directory: super::editor::dialog_dir(),
        ..Default::default()
    };
    tx.open_folder(options, |st: &mut State, result, _tx| match result {
        Ok(Some(path)) => {
            super::editor::remember(&path);
            set_steam_root(st, path)
        }
        Ok(None) => {}
        Err(error) => say(st, error.to_string(), true),
    });
}

/// Use a Steam folder: it must hold `steamapps`. When Deadlock is in its libraries, that
/// install becomes the game folder.
pub fn set_steam_root(st: &mut State, path: PathBuf) {
    if !path.join("steamapps").is_dir() {
        return say(st, fill(tr("{} has no steamapps folder."), &[&path.display().to_string()]), true);
    }
    options::edit(st, |o| o.steam_root = Some(path.display().to_string()));
    let found = deadlock_core::discovery::discover_deadlock_install(deadlock_core::discovery::InstallDiscoveryOptions {
        steam_roots: Some(vec![path.clone()]),
        ..Default::default()
    });
    match found.ok().and_then(|f| f.install) {
        Some(install) => super::set_game_dir(st, install.game_root),
        None => say(st, tr("Saved. Deadlock is not in that Steam's libraries."), true),
    }
}

// --- General: text editor ---------------------------------------------------------------------

pub fn editor_row(_s: &State, u: &Ui) -> El {
    section(
        u,
        "Files",
        vec![setting_row(
            u,
            "settings-editor",
            "Text editor",
            detail(u, &fill(tr("{} opens gameinfo.gi, autoexec.cfg and configs."), &[&super::editor::current_label()])),
            button(u, "settings-editor-change", "Change", ActionKind::Secondary).on_press(|st: &mut State| {
                st.settings.extra.editors = Some(super::editor::detect());
                st.settings.extra.dialog = Some(Dialog::Editor);
            }),
        )],
    )
}

fn editor_dialog(s: &State, u: &Ui) -> El {
    let current = options::get().editor.clone();
    let mut list = col().w_full();
    let mut entries: Vec<(String, Option<PathBuf>)> = vec![(tr("System default").to_string(), None)];
    for f in s.settings.extra.editors.iter().flatten() {
        entries.push((f.name.clone(), Some(f.path.clone())));
    }
    if let Some(c) = &current
        && !entries.iter().any(|(_, p)| p.as_ref().map(|p| p.display().to_string()).as_deref() == Some(c.as_str()))
    {
        entries.push((super::editor::name_of(std::path::Path::new(c)), Some(PathBuf::from(c))));
    }
    for (index, (name, path)) in entries.into_iter().enumerate() {
        let on = path.as_ref().map(|p| p.display().to_string()) == current;
        let pick = path.clone();
        list = list.child(
            row()
                .key(format!("settings-editor-{index}"))
                .w_full()
                .h(44.0)
                .px(8.0)
                .gap(12.0)
                .center_y()
                .radius(crate::ui::r(8.0))
                .hover(u.hover)
                .focusable(true)
                .role("radio")
                .checked(on)
                .label(name.clone())
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| super::editor::choose(st, pick.clone()))
                .child(text(name).font(BODY_MED).color(u.ink).w(180.0))
                .child(
                    text(path.map(|p| super::tidy(&p.display().to_string())).unwrap_or_default())
                        .font(SMALL)
                        .color(u.muted)
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate(),
                )
                .child(if on { rune::icon(&icons::CHECK, 14.0).color(u.accent).a11y_hidden() } else { rune::el() }),
        );
    }
    shell(u, "settings-editor-dialog", tr("Text editor"))
        .w(520.0)
        .child(list)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .pt(8.0)
                .child(button(u, "settings-editor-browse", "Browse", ActionKind::Secondary).on_press_tx(super::editor::browse))
                .child(spacer())
                .child(button(u, "settings-editor-done", "Done", ActionKind::Ghost).on_press(close)),
        )
}

// --- Integrations: tracker sites ------------------------------------------------------------

/// Stat sites for Deadlock. Opening one is the browser's request, not Sigil's.
pub const TRACKERS: [(&str, &str); 5] = [
    ("Statlocker", "https://statlocker.gg/"),
    ("DeadChaps", "https://www.deadchaps.gg/"),
    ("Buildlock", "https://buildlock.net/"),
    ("DLTV", "https://dltv.gg/"),
    ("EDL", "https://edl.gg"),
];

pub fn trackers(s: &State, u: &Ui) -> El {
    let account = s.prefs.steam_account;
    let mut tiles = rune::el().key("settings-trackers").w_full().equal_columns(5).gap(8.0);
    for (name, home) in TRACKERS {
        let url = match (name, account) {
            ("Statlocker", Some(a)) => format!("https://statlocker.gg/profile/{a}"),
            _ => home.to_string(),
        };
        tiles = tiles.child(
            row()
                .key(format!("settings-tracker-{name}"))
                .h(36.0)
                .px(12.0)
                .gap(8.0)
                .center_y()
                .radius(crate::ui::r(8.0))
                .border(1.0, u.hair)
                .hover(u.hover)
                .focusable(true)
                .role("link")
                .label(name.to_string())
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |st: &mut State, tx: &mut Tx| super::tabs::open_url(st, tx, &url))
                .child(text(name.to_string()).font(BODY_MED).color(u.ink).grow(1.0).truncate())
                .child(rune::icon(&icons::EXTERNAL_LINK, 12.0).color(u.faint).a11y_hidden()),
        );
    }
    col()
        .w_full()
        .gap(8.0)
        .child(u.section(tr("Stat sites")))
        .child(tiles)
}

// --- About: contributors ---------------------------------------------------------------------

pub fn contributors_row(s: &State, u: &Ui) -> El {
    setting_row(
        u,
        "settings-contributors",
        "Contributors",
        detail(u, "People who built Sigil, from its GitHub repository."),
        disabled(
            button(u, "settings-contributors-open", "Show", ActionKind::Secondary).on_press(open_contributors),
            s.prefs.offline && s.settings.extra.contributors.is_none(),
        ),
    )
}

fn open_contributors(st: &mut State) {
    st.settings.extra.dialog = Some(Dialog::Contributors);
    if st.settings.extra.contributors.is_some() || st.settings.extra.contributors_job.is_running() {
        return;
    }
    if st.demo || rune::is_headless() {
        st.settings.extra.contributors = Some(Ok(vec![("superuserkalo".into(), "https://github.com/superuserkalo".into())]));
        return;
    }
    let wake = st.wake.clone();
    st.settings.extra.contributors_job.start(wake, fetch_contributors);
}

/// GitHub's contributor list for Sigil's repository, people only.
pub fn parse_contributors(body: &str) -> Vec<(String, String)> {
    let Ok(serde_json::Value::Array(list)) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    list.iter()
        .filter(|c| c.get("type").and_then(|t| t.as_str()) == Some("User"))
        .filter_map(|c| {
            let login = c.get("login")?.as_str()?.to_string();
            (!login.ends_with("[bot]") && !login.ends_with("-bot")).then(|| (login.clone(), format!("https://github.com/{login}")))
        })
        .collect()
}

fn fetch_contributors() -> Result<Vec<(String, String)>, String> {
    let url = format!("https://api.github.com/repos/{}/contributors?per_page=100", super::UPDATE_REPO);
    let mut r = deadlock_core::net::agent(std::time::Duration::from_secs(15))
        .get(&url)
        .header("User-Agent", "Sigil")
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?;
    let body = r.body_mut().read_to_string().map_err(|e| e.to_string())?;
    Ok(parse_contributors(&body))
}

fn contributors_dialog(s: &State, u: &Ui) -> El {
    let mut list = col().w_full();
    match &s.settings.extra.contributors {
        None => list = list.child(text(tr("Loading")).font(BODY).color(u.muted)),
        Some(Err(e)) => list = list.child(text(e.clone()).font(BODY).color(u.muted).wrap().w_full()),
        Some(Ok(people)) if people.is_empty() => {
            list = list.child(text(tr("No contributors are listed yet.")).font(BODY).color(u.muted))
        }
        Some(Ok(people)) => {
            for (index, (login, url)) in people.iter().enumerate() {
                let url = url.clone();
                list = list.child(
                    row()
                        .key(format!("settings-contributor-{index}"))
                        .w_full()
                        .h(40.0)
                        .gap(8.0)
                        .center_y()
                        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                        .child(text(login.clone()).font(BODY_MED).color(u.ink).grow(1.0))
                        .child(
                            u.icon_button(&icons::EXTERNAL_LINK, tr("Open on GitHub"))
                                .key(format!("settings-contributor-open-{index}"))
                                .cursor(rune::Cursor::Pointer)
                                .on_press_tx(move |st: &mut State, tx: &mut Tx| super::tabs::open_url(st, tx, &url)),
                        ),
                );
            }
        }
    }
    shell(u, "settings-contributors-dialog", tr("Contributors"))
        .child(
            text(tr("Sigil also stands on Grimoire, Rune, TinyAgent and the tools listed under Open source."))
                .font(SMALL)
                .color(u.muted)
                .wrap()
                .w_full(),
        )
        .child(col().w_full().max_h(300.0).scroll(rune::ScrollSpec { stick_bottom: false }).child(list))
        .child(
            row()
                .w_full()
                .child(spacer())
                .child(button(u, "settings-contributors-done", "Done", ActionKind::Ghost).on_press(close)),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contributors_are_people_only() {
        let body = r#"[{"login":"kalo","type":"User"},{"login":"dependabot[bot]","type":"Bot"},{"login":"x-bot","type":"User"}]"#;
        assert_eq!(parse_contributors(body), vec![("kalo".to_string(), "https://github.com/kalo".to_string())]);
        assert!(parse_contributors("nope").is_empty());
    }
}

// --- scenes ------------------------------------------------------------------------------------

/// The sections this round added to a tab, alone, so a snapshot shows them without scrolling.
/// Only scenes set `scene_only`.
pub fn scene_content(s: &State, u: &Ui, tab: Tab) -> Option<El> {
    if s.settings.extra.scene_only != Some(tab) {
        return None;
    }
    let c = col().w_full().gap(super::LIST_GAP);
    Some(match tab {
        Tab::Game => c.child(steam_rows(s, u)).child(super::watch::game_rows(s, u)),
        Tab::Backups => c.child(super::tools::backups_section(s, u)),
        Tab::Mods if s.settings.extra.scene_page == 1 => c.child(super::downloads::section_el(s, u)),
        Tab::Mods if s.settings.extra.scene_page == 2 => c
            .child(super::sources::section_el(s, u))
            .child(super::pages::section_el(s, u)),
        Tab::Mods => c.child(section(u, "NSFW content", super::nsfw::rows(s, u))),
        Tab::General if s.settings.extra.scene_page == 3 => c.child(super::tabs::pages(s, u)),
        Tab::General => c
            .child(super::tray::rows(s, u))
            .child(editor_row(s, u))
            .child(super::onboarding::rows(s, u)),
        Tab::Integrations => c.child(trackers(s, u)),
        Tab::Network => c
            .child(section(u, "Proxy", super::netx::proxy_rows(s, u)))
            .child(super::netx::diagnostics_section(s, u)),
        Tab::Logs => c
            .child(section(u, "Game crashes", super::support::crash_rows(s, u)))
            .child(section(u, "Report a problem", super::support::report_rows(s, u))),
        Tab::Storage => c.child(section(u, "Mod data", super::tools::storage_rows(s, u))),
        Tab::Experimental => c.child(super::tools::experimental_section(s, u)),
        _ => return None,
    })
}

/// Open a tab over the demo library with only this round's sections showing.
pub fn scene_parity(s: &mut State, tab: Tab) {
    super::scene_tab(s, tab);
    s.settings.extra.scene_only = Some(tab);
    if tab == Tab::Mods {
        s.settings.extra.probes = Some(Ok(vec![
            deadlock_core::file_servers::Probe { name: "filecache9".into(), bytes_per_second: Some(6_100_000.0) },
            deadlock_core::file_servers::Probe { name: "filecache4".into(), bytes_per_second: Some(3_400_000.0) },
            deadlock_core::file_servers::Probe { name: "filecache12".into(), bytes_per_second: None },
        ]));
        let mut o = options::get_owned();
        o.sources = vec![options::Source {
            url: "https://mods.example.org/dmm-source.json".into(),
            name: "Community Skins".into(),
            enabled: true,
        }];
        o.revealed = vec![1, 2, 3];
        o.remember_reveals = true;
        options::set(o);
    }
    if tab == Tab::Network {
        super::netx::on_open(s);
        st_demo_diag(s);
    }
    if tab == Tab::Storage {
        s.settings.extra.tools.unknown = Some(vec!["readme.txt".into(), "old_mod.zip".into()]);
    }
}

fn st_demo_diag(s: &mut State) {
    s.settings.extra.net.diag = None;
    s.settings.extra.net.test = Some(Ok(142));
    // The demo rows Run would show.
    let mut rows = Vec::new();
    for (i, (name, url)) in super::netx::PROBES.iter().enumerate() {
        rows.push(super::netx::DiagRow {
            name: (*name).to_string(),
            host: deadlock_core::mod_sources::origin_of(url).unwrap_or_default().trim_start_matches("https://").to_string(),
            ms: (i != 5).then_some(80 + i as u128 * 37),
            problem: (i == 5).then(|| tr("Timed out.").to_string()),
            slow: false,
        });
    }
    s.settings.extra.net.diag = Some(rows);
}

/// A dialog over the demo settings sheet.
pub fn scene_dialog(s: &mut State, which: &str) {
    super::scene_tab(s, Tab::Integrations);
    match which {
        "templates" => {
            let mut d = super::templates::Draft::new();
            d.top = TextEdit::new("Grinding as {hero}");
            s.settings.extra.draft = Some(d);
            s.settings.extra.dialog = Some(Dialog::Templates);
        }
        "editor" => {
            s.settings.extra.editors = Some(vec![
                super::editor::Found { name: "TextEdit".into(), path: PathBuf::from("/System/Applications/TextEdit.app") },
                super::editor::Found { name: "Zed".into(), path: PathBuf::from("/Applications/Zed.app") },
            ]);
            s.settings.extra.dialog = Some(Dialog::Editor);
        }
        "crash" => {
            s.settings.extra.dialog = Some(Dialog::Crash(PathBuf::from("deadlock_2026_1115_204501_1_crash.mdmp")));
        }
        "recovery" => {
            s.settings.extra.tools.recovery = Some(deadlock_core::mod_data::RecoveryReport {
                moved: vec![
                    (1, PathBuf::from("20261117-101500-123-pak01")),
                    (2, PathBuf::from("20261117-101500-130-pak02")),
                    (5, PathBuf::from("20261117-101500-141-pak05")),
                ],
                failed: vec![],
                bytes: 84_000_000,
            });
            s.settings.extra.dialog = Some(Dialog::Recovery);
        }
        "source" => {
            let d = deadlock_core::mod_sources::parse_descriptor(
                "https://mods.example.org/dmm-source.json",
                r#"{"format":"dmm-source","protocolVersion":"1.0","sourceId":"c","name":"Community Skins","gameId":"deadlock","catalog":{"kind":"static","indexUrl":"./catalog.json"},"networkOrigins":{"api":["https://mods.example.org"],"downloads":["https://cdn.example.org"],"media":[]}}"#,
            )
            .expect("demo descriptor");
            let listing = |id: &str, title: &str, author: &str, version: &str, nsfw: Option<bool>| deadlock_core::mod_sources::Listing {
                id: id.into(),
                title: title.into(),
                author: author.into(),
                version: version.into(),
                nsfw,
                artifacts: vec![deadlock_core::mod_sources::Artifact { id: "a".into(), url: Some("https://cdn.example.org/a.zip".into()), ..Default::default() }],
                ..Default::default()
            };
            let synced = super::sources::Synced {
                descriptor: d.clone(),
                listings: vec![
                    listing("1", "Midnight Haze", "Kalo", "1.2", Some(false)),
                    listing("2", "Gold Abrams", "Rook", "2.0", Some(false)),
                    listing("3", "Quiet footsteps", "Ash", "", None),
                ],
                at: std::time::Instant::now(),
            };
            s.settings.extra.sources.synced.insert(d.url.clone(), synced);
            s.settings.extra.dialog = Some(Dialog::SourceBrowse(d.url));
        }
        "source-add" => {
            scene_dialog(s, "source");
            let synced = s.settings.extra.sources.synced.values().next().cloned().expect("demo");
            s.settings.extra.dialog = Some(Dialog::SourceAdd(Box::new(synced)));
        }
        "onboarding" => {
            s.settings.open = false;
            s.settings.extra.onboarding = Some(super::onboarding::Step::Gameinfo);
        }
        "scripts" => s.settings.extra.dialog = Some(Dialog::AllowScripts),
        "mods2" | "mods3" => {
            scene_parity(s, Tab::Mods);
            s.settings.extra.scene_page = if which == "mods2" { 1 } else { 2 };
        }
        _ => {}
    }
}
