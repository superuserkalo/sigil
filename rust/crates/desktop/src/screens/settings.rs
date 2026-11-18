//! Settings: a sheet over the window, the categories down the side and the chosen one beside
//! them (the ngine settings sheet, in Sigil's house style). Escape, the close button and a
//! press on the scrim close it.
//!
//! Every control here reads or writes `crate::prefs::Prefs` (saved to `settings.json`), or runs
//! an action through `deadlock-core`. Writes to the game folder or to Steam's files go through a
//! confirm dialog first. No credential passes through this view: sign in, sign out and the
//! pending browser step come from `crate::signin`.

pub mod autostart;
pub mod discord;
pub mod downloads;
pub mod editor;
pub mod extra;
pub mod forge;
pub mod i18n;
pub mod keychain;
mod locales;
mod locales_parity;
mod locales_wave;
pub mod log;
pub mod netx;
pub mod nsfw;
pub mod onboarding;
pub mod options;
pub mod pages;
pub mod personalize;
mod search;
pub mod sharing;
pub mod sources;
pub mod support;
pub mod tabs;
pub mod telemetry;
pub mod templates;
pub mod tools;
pub mod tray;
pub mod watch;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use deadlock_core::backups::{self, BackupEntry, BackupKind, BackupSources};
use deadlock_core::gamebanana::BrowseSort;
use rune::{
    Color, El, FileDialogOptions, Icon, Key, KeyEvent, ScrollSpec, TextEdit, Tx, col, el, icon,
    row, spacer, text,
};

use crate::app::State;
use crate::icons;
use crate::jobs::Job;
use crate::model::{ProviderState, Screen};
use crate::prefs::{self, ACCENTS, Accent, Mature, OnLaunch, Prefs};
use i18n::{fill, tr};
use crate::ui::{ActionKind, BODY, BODY_MED, DISPLAY, H1, LABEL, MONO, SMALL, Theme, Ui};

const LIST_W: f32 = 212.0;
const WIDEST: f32 = 980.0;
const TALLEST: f32 = 720.0;
const MARGIN: f32 = 32.0;
const RADIUS: f32 = 14.0;
const CONTENT_MAX: f32 = 680.0;
const ROW_H: f32 = 56.0;
const FIELD_W: f32 = 280.0;
/// Space between the sections of a category.
const LIST_GAP: f32 = 28.0;

/// The GitHub repository the update check reads releases from.
pub const UPDATE_REPO: &str = "superuserkalo/sigil";

/// The categories, in the order the list shows them.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Tab {
    #[default]
    Game,
    Backups,
    Mods,
    General,
    Appearance,
    Integrations,
    Privacy,
    Models,
    Network,
    Logs,
    Storage,
    Experimental,
    About,
}

/// Grouped by what the player is doing: setting up the game, shaping Sigil, fixing things.
const GROUPS: [(&str, &[Tab]); 3] = [
    ("Game and mods", &[Tab::Game, Tab::Backups, Tab::Mods]),
    (
        "Sigil",
        &[
            Tab::General,
            Tab::Appearance,
            Tab::Integrations,
            Tab::Privacy,
            Tab::Models,
        ],
    ),
    (
        "Advanced",
        &[Tab::Network, Tab::Logs, Tab::Storage, Tab::Experimental, Tab::About],
    ),
];

impl Tab {
    pub fn label(self) -> &'static str {
        match self {
            Tab::Game => "Game",
            Tab::Backups => "Backups",
            Tab::Mods => "Mods",
            Tab::General => "General",
            Tab::Appearance => "Appearance",
            Tab::Integrations => "Integrations",
            Tab::Privacy => "Privacy",
            Tab::Models => "Models and sign-in",
            Tab::Network => "Network",
            Tab::Logs => "Logs and support",
            Tab::Storage => "Storage",
            Tab::Experimental => "Experimental",
            Tab::About => "About and updates",
        }
    }

    fn icon(self) -> &'static Icon {
        match self {
            Tab::Game => &icons::FOLDER_OPEN,
            Tab::Backups => &icons::HISTORY,
            Tab::Mods => &icons::PACKAGE,
            Tab::General => &icons::SLIDERS_HORIZONTAL,
            Tab::Appearance => &icons::EYE,
            Tab::Integrations => &icons::USER,
            Tab::Privacy => &icons::SHIELD,
            Tab::Models => &icons::KEY_ROUND,
            Tab::Network => &icons::GLOBE,
            Tab::Logs => &icons::TERMINAL,
            Tab::Storage => &icons::BOX,
            Tab::Experimental => &icons::SPARKLES,
            Tab::About => &icons::INFO,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Tab::Game => "game",
            Tab::Backups => "backups",
            Tab::Mods => "mods",
            Tab::General => "general",
            Tab::Appearance => "appearance",
            Tab::Integrations => "integrations",
            Tab::Privacy => "privacy",
            Tab::Models => "models",
            Tab::Network => "network",
            Tab::Logs => "logs",
            Tab::Storage => "storage",
            Tab::Experimental => "experimental",
            Tab::About => "about",
        }
    }
}

/// A question asked before a write or a reset.
#[derive(Clone)]
pub enum Confirm {
    RepairGameinfo,
    LaunchOptions(String),
    Restore(PathBuf),
    Delete(PathBuf),
    Prune,
    Reset,
    CleanupArchives,
    ForgetSharing,
    ClearLogs,
}

/// What Steam has for Deadlock's launch options.
#[derive(Clone)]
pub struct LaunchRead {
    pub path: Option<PathBuf>,
    pub value: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone)]
pub struct UpdateCheck {
    /// None when the repository has no published release.
    pub latest: Option<Release>,
    pub newer: bool,
}

/// The newest release, its notes and whether this copy can install it (`crate::updater`).
pub use crate::updater::Offer as Release;

#[derive(Default)]
pub struct View {
    pub open: bool,
    pub tab: Tab,
    pub confirm: Option<Confirm>,
    /// The last outcome, shown at the foot of the pane. True marks an error.
    pub message: Option<(String, bool)>,
    pub accent_hex: TextEdit,
    pub launch_options: TextEdit,
    pub launch_read: Option<LaunchRead>,
    pub backups: Option<Vec<BackupEntry>>,
    pub image_cache: Option<u64>,
    pub download_cache: Option<u64>,
    pub update_job: Job<UpdateCheck>,
    pub update: Option<Result<UpdateCheck, String>>,
    /// Download, staged update and rollback notice (`crate::updater`).
    pub updater: crate::updater::Updater,
    /// Steam was asked to start the game; the next poll applies "When the game starts".
    pub launched: bool,
    // --- round 3 ---
    pub search: TextEdit,
    pub sharing: sharing::Sharing,
    pub presence: discord::Presence,
    /// Read from the system when Settings opens.
    pub autostart_on: Option<bool>,
    pub proxy_edit: TextEdit,
    pub gb_edit: TextEdit,
    pub gb_job: Job<(u64, String)>,
    pub steam_users: Option<Vec<deadlock_core::steam_users::SteamUser>>,
    pub log_size: Option<u64>,
    /// Count, newest file and the folders looked in.
    pub crash_dumps: Option<(usize, Option<PathBuf>, Vec<PathBuf>)>,
    pub leftover: Option<Vec<(PathBuf, u64)>>,
    pub last_update_check: Option<Instant>,
    pub last_catalog: Option<Instant>,
    pub started: bool,
    pub session_start: u64,
    pub game_job: Job<bool>,
    pub game_running: bool,
    pub last_game_check: Option<Instant>,
    /// The single launch button's mode.
    pub launch_vanilla_pick: bool,
    /// Settings added in the parity round (`extra.rs`).
    pub extra: extra::Extra,
    /// Appearance > Personalize: the theme builder's draft, imports, the art picker.
    pub personalize: personalize::View,
    /// Opt-in telemetry: consent, the flush worker, Send diagnostics (`telemetry.rs`).
    pub telemetry: telemetry::Telemetry,
}

// --- open, close, poll ---------------------------------------------------------

/// Open the sheet on its current category and read what that category shows.
pub fn open(st: &mut State) {
    st.settings.open = true;
    st.settings.message = None;
    crate::data::refresh_subscriptions(st);
    if !st.demo && !st.launch.loaded && !st.launch_job.is_running() {
        crate::data::start_launch_read(st);
    }
    if !st.demo && !st.autoexec.loaded && !st.autoexec_job.is_running() {
        crate::data::start_autoexec_read(st);
    }
    if st.settings.autostart_on.is_none() {
        st.settings.autostart_on = Some(!st.demo && !rune::is_headless() && autostart::is_enabled());
    }
    if st.settings.sharing.record.is_none() {
        st.settings.sharing.record = Some(sharing::load());
    }
    st.settings.proxy_edit = TextEdit::new(st.prefs.proxy.clone().unwrap_or_default());
    let tab = st.settings.tab;
    select(st, tab);
}

pub fn close(st: &mut State) {
    st.settings.open = false;
    st.settings.confirm = None;
}

fn select(st: &mut State, tab: Tab) {
    st.settings.tab = tab;
    st.settings.message = None;
    match tab {
        Tab::Game => read_launch_options(st),
        Tab::Backups => read_backups(st),
        Tab::Storage => {
            read_sizes(st);
            st.settings.leftover = st
                .addons_dir
                .as_ref()
                .map(|dir| deadlock_core::cleanup::leftover_archives(Path::new(dir)));
        }
        Tab::Integrations => tabs::read_steam_users(st),
        Tab::Logs => tabs::read_crash_dumps(st),
        Tab::Appearance => {
            if let Accent::Custom(hex) = &st.prefs.accent {
                st.settings.accent_hex = TextEdit::new(hex.clone());
            }
            personalize::on_open(st);
        }
        _ => {}
    }
    extra::on_select(st, tab);
}

/// Called from the app's poll loop.
pub fn poll(st: &mut State, tx: &mut Tx) -> bool {
    let mut changed = false;
    if let Some(result) = st.settings.update_job.try_take() {
        st.settings.update = Some(result);
        changed = true;
    }
    changed |= crate::updater::poll(st, tx);
    if std::mem::take(&mut st.settings.launched) {
        after_launch(st, tx);
        changed = true;
    }
    changed |= tabs::background(st, tx);
    changed |= extra::poll(st, tx);
    changed |= personalize::poll(st);
    changed |= telemetry::poll(st);
    changed
}

/// What Sigil does once Steam has the launch request.
pub fn after_launch(st: &mut State, tx: &mut Tx) {
    st.settings.launched = false;
    if rune::is_headless() {
        return;
    }
    watch::on_launch(st);
    match st.prefs.on_launch {
        OnLaunch::Stay => {}
        OnLaunch::Minimize => tray::minimize(tx),
        OnLaunch::Close => {
            st.agent.shutdown();
            tx.quit();
        }
    }
}

/// Startup: language, log, network switches, flags, then the update check when it is on.
pub fn on_startup(st: &mut State) {
    i18n::set(st.prefs.language);
    log::set_level(st.prefs.log_level);
    apply_offline(st.prefs.offline);
    let _ = deadlock_core::net::set_proxy(st.prefs.proxy.clone());
    // At startup a flag only turns things on, so a scene that docks the details keeps them.
    if st.prefs.flag("docked_details") {
        st.browse_view.docked = true;
    }
    log::info(format!(
        "Sigil {} started on {} {}",
        VERSION,
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    install_panic_log();
    telemetry::on_startup(st);
    if st.prefs.check_updates && !st.prefs.offline && !st.demo && !rune::is_headless() {
        st.settings.last_update_check = Some(Instant::now());
        start_update_check(st);
    }
}

/// Offline mode for this process and for the helpers it starts (`DAH_OFFLINE`).
pub fn apply_offline(offline: bool) {
    deadlock_core::net::set_offline(offline);
    // SAFETY: as in `prefs::apply_game_dir`, Rust's env access is locked, and children read
    // the variable when they start.
    unsafe {
        if offline {
            std::env::set_var(deadlock_core::net::OFFLINE_ENV, "1");
        } else {
            std::env::remove_var(deadlock_core::net::OFFLINE_ENV);
        }
    }
}

/// Flags that change state another screen owns.
pub fn apply_flags(st: &mut State) {
    st.browse_view.docked = st.prefs.flag("docked_details");
    if !st.prefs.page_on(st.screen) {
        st.screen = Screen::Browse;
    }
    if !st.prefs.flag("agent") && crate::app::under_agent(st.screen) {
        st.screen = Screen::Browse;
    }
    // Turning the Agent off stops its host process; turning it on starts it when Agent opens.
    if !st.prefs.flag("agent") && st.host_started {
        st.agent.shutdown();
        st.agent = crate::agent::AgentLink::disconnected();
        st.host_started = false;
    }
}

/// A panic is written to the log (redacted) before the default hook prints it.
fn install_panic_log() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            log::error(format!("panic: {info}"));
            previous(info);
        }));
    });
}

// --- the sheet ------------------------------------------------------------------

/// The sheet and, above it, a confirm when one is asked. None while closed.
pub fn modal(s: &State, u: &Ui, window: (f32, f32)) -> Option<El> {
    if !s.settings.open {
        return extra::overlay(s, u);
    }
    let width = (window.0 - 2.0 * MARGIN).clamp(320.0, WIDEST);
    let height = (window.1 - 2.0 * MARGIN).clamp(320.0, TALLEST);
    let searching = !s.settings.search.text.trim().is_empty();
    let content = if searching {
        search::results(s, u)
    } else if let Some(only) = extra::scene_content(s, u, s.settings.tab) {
        only
    } else {
        match s.settings.tab {
            Tab::Game => game(s, u),
            Tab::Backups => col()
                .w_full()
                .gap(LIST_GAP)
                .child(tools::backups_section(s, u))
                .child(backups_tab(s, u)),
            Tab::Mods => col()
                .w_full()
                .gap(LIST_GAP)
                .child(mods(s, u))
                .child(section(u, "Mature content", nsfw::rows(s, u)))
                .child(downloads::section_el(s, u))
                .child(sources::section_el(s, u))
                .child(pages::section_el(s, u)),
            Tab::General => col()
                .w_full()
                .gap(LIST_GAP)
                .child(tabs::general(s, u))
                .child(tabs::pages(s, u))
                .child(tray::rows(s, u))
                .child(extra::editor_row(s, u))
                .child(onboarding::rows(s, u)),
            Tab::Appearance => appearance(s, u),
            Tab::Integrations => col()
                .w_full()
                .gap(LIST_GAP)
                .child(tabs::integrations(s, u))
                .child(extra::trackers(s, u)),
            Tab::Privacy => privacy(s, u),
            Tab::Models => models(s, u),
            Tab::Network => col()
                .w_full()
                .gap(LIST_GAP)
                .child(tabs::network(s, u))
                .child(netx::diagnostics_section(s, u)),
            Tab::Logs => col()
                .w_full()
                .gap(LIST_GAP)
                .child(tabs::logs(s, u))
                .child(section(u, "Send to Sigil", vec![telemetry::diagnostics_row(s, u)])),
            Tab::Storage => storage(s, u),
            Tab::Experimental => col()
                .w_full()
                .gap(LIST_GAP)
                .child(tabs::experimental(s, u))
                .child(tools::experimental_section(s, u)),
            Tab::About => about(s, u),
        }
    };
    let heading = if searching {
        tr("Search results")
    } else {
        tr(s.settings.tab.label())
    };
    let mut pane = col()
        .key(if searching {
            "settings-search".to_string()
        } else {
            format!("settings-{}", s.settings.tab.key())
        })
        .w_full()
        .max_w(CONTENT_MAX)
        .px(32.0)
        .pt(4.0)
        .pb(32.0)
        .gap(28.0)
        .child(content);
    if let Some((message, error)) = &s.settings.message {
        pane = pane.child(
            text(message.clone())
                .key("settings-message")
                .font(SMALL)
                .color(if *error { u.rose } else { u.muted })
                .wrap()
                .w_full(),
        );
    }
    let head = row()
        .key("settings-head")
        .w_full()
        .h(64.0)
        .pl(32.0)
        .pr(16.0)
        .center_y()
        .child(
            text(heading)
                .font(DISPLAY)
                .color(u.ink)
                .grow(1.0),
        )
        .child(
            u.icon_button(&icons::X, tr("Close settings"))
                .key("settings-close")
                .cursor(rune::Cursor::Pointer)
                .on_press(close),
        );
    let body = col()
        .key("settings-body")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .child(head)
        .child(
            col()
                .key("settings-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(pane),
        );
    let panel = row()
        .key("settings")
        .role("dialog")
        .label(tr("Settings"))
        .w(width)
        .h(height)
        .radius(RADIUS)
        .clip_rounded(RADIUS)
        .fill(u.bg)
        .border(1.0, u.hair_strong)
        .shadow(
            24.0,
            64.0,
            Color::BLACK.alpha(if u.light { 0.18 } else { 0.55 }),
        )
        .modal(true, close)
        .on_key(|st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                if st.settings.search.text.is_empty() {
                    close(st);
                } else {
                    st.settings.search = TextEdit::new("");
                }
                true
            } else {
                false
            }
        })
        .child(list(s, u))
        .child(body);
    let mut layer = el()
        .key("settings-layer")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .child(
            el().key("settings-scrim")
                .absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .fill(u.scrim)
                .on_press(close),
        )
        .child(el().w_full().h_full().center().child(panel));
    if let Some(ask) = &s.settings.confirm {
        layer = layer.child(u.modal_layer("settings-confirm", confirm_dialog(s, u, ask)));
    } else if let Some(d) = extra::dialog(s, u) {
        layer = layer.child(u.modal_layer("settings-extra", d));
    }
    Some(layer)
}

fn list(s: &State, u: &Ui) -> El {
    let mut items = col()
        .key("settings-sections")
        .role("tablist")
        .label(tr("Settings sections"))
        .w_full()
        .gap(2.0);
    for (index, (group, tabs)) in GROUPS.iter().enumerate() {
        items = items.child(
            row()
                .w_full()
                .h(if index == 0 { 28.0 } else { 40.0 })
                .px(10.0)
                .pb(6.0)
                .items_end()
                .child(text(tr(group)).font(SMALL).color(u.faint)),
        );
        for &tab in *tabs {
            items = items.child(list_item(s, u, tab));
        }
    }
    col()
        .key("settings-side")
        .w(LIST_W)
        .h_full()
        .px(8.0)
        .pt(12.0)
        .fill(u.sidebar)
        .border_sides([0.0, 1.0, 0.0, 0.0], u.hair)
        .child(
            row()
                .w_full()
                .h(40.0)
                .px(10.0)
                .center_y()
                .child(text(tr("Settings")).font(BODY_MED).color(u.ink)),
        )
        .child(search_field(s, u))
        .child(items)
}

/// Filters settings across every category; Escape clears it.
fn search_field(s: &State, u: &Ui) -> El {
    let typed = !s.settings.search.text.is_empty();
    let mut field = row()
        .key("settings-search-field")
        .w_full()
        .h(32.0)
        .px(10.0)
        .gap(8.0)
        .mb(8.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .fill(u.well)
        .border(1.0, if typed { u.hair_strong } else { u.hair })
        .child(icon(&icons::SEARCH, 14.0).color(u.faint).a11y_hidden())
        .child(
            rune::text_input(&s.settings.search, BODY)
                .key("settings-search-input")
                .label(tr("Search settings"))
                .single_line()
                .grow(1.0)
                .min_w(0.0)
                .color(u.ink)
                .placeholder(tr("Search settings"), u.faint)
                .caret(u.accent, u.accent.alpha(0.22))
                .on_edit(|st: &mut State| &mut st.settings.search),
        );
    if typed {
        field = field.child(
            el().key("settings-search-clear")
                .size(16.0, 16.0)
                .center()
                .role("button")
                .label(tr("Clear search"))
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| st.settings.search = TextEdit::new(""))
                .child(icon(&icons::X, 12.0).color(u.muted).a11y_hidden()),
        );
    }
    field
}

fn list_item(s: &State, u: &Ui, tab: Tab) -> El {
    let on = s.settings.tab == tab && s.settings.search.text.trim().is_empty();
    row()
        .key(format!("settings-tab-{}", tab.key()))
        .w_full()
        .h(32.0)
        .px(10.0)
        .gap(10.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .fill(if on { u.selected } else { Color::TRANSPARENT })
        .hover(if on { u.selected } else { u.hover })
        .focusable(true)
        .role("tab")
        .checked(on)
        .label(tr(tab.label()))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            st.settings.search = TextEdit::new("");
            select(st, tab)
        })
        .child(
            icon(tab.icon(), 16.0)
                .color(if on { u.ink } else { u.muted })
                .a11y_hidden(),
        )
        .child(
            text(tr(tab.label()))
                .font(BODY_MED)
                .color(if on { u.ink } else { u.prose })
                .truncate()
                .a11y_hidden(),
        )
}

// --- shared rows ---------------------------------------------------------------

/// A section: a title, then rows that each draw their own bottom hairline.
fn section(u: &Ui, title: &str, rows: Vec<El>) -> El {
    let mut list = col().w_full().border_sides([1.0, 0.0, 0.0, 0.0], u.hair);
    for r in rows {
        list = list.child(r);
    }
    let mut c = col().w_full().gap(8.0);
    if !title.is_empty() {
        c = c.child(u.section(tr(title)));
    }
    c.child(list)
}

/// A settings row: title, one line under it, and a trailing control. 56 points, one hairline.
fn setting_row(u: &Ui, key: &str, title: &str, detail: El, trailing: El) -> El {
    row()
        .key(key.to_string())
        .w_full()
        .min_h(ROW_H)
        .py(10.0)
        .gap(16.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(text(tr(title).to_string()).font(BODY_MED).color(u.ink))
                .child(detail),
        )
        .child(trailing)
}

/// A row whose control sits under the title, full width.
fn stacked_row(u: &Ui, key: &str, title: &str, line: &str, control: El) -> El {
    let mut c = col()
        .key(key.to_string())
        .w_full()
        .gap(10.0)
        .pt(12.0)
        .pb(16.0)
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(text(tr(title).to_string()).font(BODY_MED).color(u.ink));
    if !line.is_empty() {
        c = c.child(detail(u, line).wrap());
    }
    c.child(row().w_full().child(control))
}

fn detail(u: &Ui, s: &str) -> El {
    text(tr(s).to_string()).font(SMALL).color(u.muted).truncate()
}

fn path_detail(u: &Ui, s: &str) -> El {
    text(s.to_string())
        .font(MONO)
        .color(u.muted)
        .truncate()
        .selectable(true)
}

/// A plain status word: muted when fine, amber when it needs the user.
fn status_word(u: &Ui, label: &str, warn: bool) -> El {
    text(tr(label).to_string())
        .font(BODY)
        .color(if warn { u.amber } else { u.muted })
}

fn trailing(items: Vec<El>) -> El {
    let mut r = row().gap(8.0).center_y();
    for item in items {
        r = r.child(item);
    }
    r
}

fn toggle_row(u: &Ui, key: &str, title: &str, line: &str, on: bool, press: fn(&mut State)) -> El {
    setting_row(
        u,
        key,
        title,
        detail(u, line),
        u.toggle(on)
            .key(format!("{key}-switch"))
            .label(tr(title).to_string())
            .cursor(rune::Cursor::Pointer)
            .on_press(press),
    )
}

/// A segmented control: one pill per choice, the current one filled.
fn segmented<T: Copy + PartialEq + 'static>(
    u: &Ui,
    key: &str,
    choices: &[(T, &str)],
    current: T,
    pick: fn(&mut State, T),
) -> El {
    let mut r = row()
        .key(key.to_string())
        .h(32.0)
        .padding(2.0)
        .gap(2.0)
        .radius(crate::ui::r(8.0))
        .fill(u.well)
        .border(1.0, u.hair)
        .role("radiogroup");
    for (index, (value, label)) in choices.iter().enumerate() {
        let on = *value == current;
        let value = *value;
        r = r.child(
            row()
                .key(format!("{key}-{index}"))
                .h_full()
                .px(12.0)
                .center()
                .radius(crate::ui::r(6.0))
                .fill(if on { u.raised } else { Color::TRANSPARENT })
                .border(1.0, if on { u.hair_strong } else { Color::TRANSPARENT })
                .hover(if on { u.raised } else { u.hover })
                .focusable(true)
                .role("radio")
                .checked(on)
                .label(tr(label).to_string())
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| pick(st, value))
                .child(
                    text(tr(label).to_string())
                        .font(LABEL)
                        .color(if on { u.ink } else { u.muted })
                        .a11y_hidden(),
                ),
        );
    }
    r
}

fn button(u: &Ui, key: &str, label: &str, kind: ActionKind) -> El {
    u.action(tr(label), kind)
        .key(key.to_string())
        .cursor(rune::Cursor::Pointer)
}

fn disabled(e: El, off: bool) -> El {
    e.disabled(off).opacity(if off { 0.45 } else { 1.0 })
}

fn field(input: El, u: &Ui) -> El {
    row()
        .w(FIELD_W)
        .h(32.0)
        .px(10.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .fill(u.well)
        .border(1.0, u.hair)
        .child(input)
}

/// The status line at the foot of the pane. A message that is a known UI string is shown in
/// the current language.
fn say(st: &mut State, message: impl Into<String>, error: bool) {
    let message = message.into();
    st.settings.message = Some((tr(&message).to_string(), error));
}

// --- game ------------------------------------------------------------------------

fn game(s: &State, u: &Ui) -> El {
    let folder_known = s.game_detected && !s.paths.game.starts_with("Not");
    let game_path = if folder_known {
        s.paths.game.clone()
    } else {
        "Not found".to_string()
    };
    let game_root = PathBuf::from(&s.paths.game);
    let from_env = prefs::game_dir_from_env();
    let chosen = s.prefs.game_dir.is_some();

    let path_field = row()
        .key("settings-game-path")
        .grow(1.0)
        .min_w(0.0)
        .h(32.0)
        .px(10.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .fill(u.well)
        .border(1.0, u.hair)
        .child(
            text(game_path)
                .font(MONO)
                .color(if folder_known { u.ink } else { u.faint })
                .truncate()
                .selectable(true)
                .grow(1.0)
                .min_w(0.0),
        );
    let line = if from_env {
        "Set by DAH_GAME_DIR in your environment."
    } else if chosen && folder_known {
        "Chosen by you. Detect goes back to your Steam library."
    } else if chosen {
        "The folder you chose has no game/citadel. Choose another or detect."
    } else if folder_known {
        "Found in your Steam library."
    } else {
        "Install Deadlock through Steam, or choose its folder."
    };
    let folder = stacked_row(
        u,
        "settings-game-folder",
        "Deadlock folder",
        line,
        row()
            .w_full()
            .gap(8.0)
            .center_y()
            .child(path_field)
            .child(disabled(
                button(u, "settings-game-choose", "Choose", ActionKind::Secondary)
                    .on_press_tx(choose_game_dir),
                from_env,
            ))
            .child(disabled(
                button(u, "settings-game-detect", "Detect", ActionKind::Secondary)
                    .on_press(detect_game_dir),
                from_env,
            ))
            .child(disabled(
                u.icon_button(&icons::FOLDER, tr("Reveal the game folder"))
                    .key("settings-game-reveal")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        crate::data::reveal_path(st, game_root.display().to_string())
                    }),
                !folder_known,
            )),
    );

    // gameinfo.gi: read with the launch check, which opening Settings starts.
    let (gameinfo_word, needs_repair, gameinfo_detail) = match s.launch.read.as_ref() {
        Some(read) if read.gameinfo_addons => (
            "Configured",
            false,
            "Lists citadel/addons, so the game loads your mods.",
        ),
        Some(read) if read.gameinfo_present => (
            "Needs repair",
            true,
            "The game ignores mods until citadel/addons is listed.",
        ),
        Some(_) => (
            "Missing",
            false,
            "No gameinfo.gi. Verify the game files in Steam.",
        ),
        None if s.launch_job.is_running() => ("Checking", false, "Reading gameinfo.gi."),
        None => (
            "Not checked",
            false,
            "Game updates and Steam file checks reset this file.",
        ),
    };
    let mut gameinfo_actions = vec![status_word(u, gameinfo_word, needs_repair)];
    if needs_repair {
        gameinfo_actions.push(
            button(u, "settings-gameinfo-repair", "Repair", ActionKind::Primary).on_press(
                |st: &mut State| st.settings.confirm = Some(Confirm::RepairGameinfo),
            ),
        );
    }
    let gameinfo_file = PathBuf::from(&s.paths.game)
        .join("game")
        .join("citadel")
        .join("gameinfo.gi");
    let gameinfo_exists = folder_known && s.launch.read.as_ref().is_some_and(|r| r.gameinfo_present);
    gameinfo_actions.push(disabled(
        u.icon_button(&icons::SQUARE_PEN, tr("Open gameinfo.gi in your editor"))
            .key("settings-gameinfo-open")
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| open_in_editor(st, &gameinfo_file)),
        !gameinfo_exists,
    ));
    gameinfo_actions.push(
        button(u, "settings-gameinfo-check", "Check", ActionKind::Secondary).on_press(
            |st: &mut State| {
                if !st.launch_job.is_running() {
                    crate::data::start_launch_read(st);
                }
            },
        ),
    );
    let gameinfo = setting_row(
        u,
        "settings-gameinfo",
        "gameinfo.gi",
        detail(u, gameinfo_detail),
        trailing(gameinfo_actions),
    );

    let addons_path = s.addons_dir.clone();
    let addons_known = addons_path.is_some();
    let addons_reveal = addons_path.clone().unwrap_or_default();
    let addons = setting_row(
        u,
        "settings-addons",
        "Addon folder",
        path_detail(
            u,
            addons_path
                .as_deref()
                .unwrap_or("Appears once the game folder is found"),
        ),
        disabled(
            button(u, "settings-addons-reveal", "Reveal", ActionKind::Secondary)
                .on_press(move |st: &mut State| crate::data::reveal_path(st, addons_reveal.clone())),
            !addons_known,
        ),
    );

    let ax = &s.autoexec;
    let (ax_word, ax_detail) = if !ax.loaded {
        ("Not checked", "Crosshairs and commands load from it.")
    } else if ax.error.is_some() {
        ("Unavailable", "Needs the game folder.")
    } else if ax.exists {
        ("Present", "Crosshairs and commands load from it.")
    } else {
        ("Missing", "Sigil creates it the first time you save one.")
    };
    let autoexec = setting_row(
        u,
        "settings-autoexec",
        "autoexec.cfg",
        detail(u, ax_detail),
        trailing(vec![
            status_word(u, ax_word, false),
            button(u, "settings-autoexec-open", "Edit", ActionKind::Secondary).on_press(
                |st: &mut State| {
                    close(st);
                    st.go(Screen::Autoexec);
                },
            ),
        ]),
    );

    col()
        .w_full()
        .gap(28.0)
        .child(section(
            u,
            "",
            vec![folder, gameinfo, addons, autoexec],
        ))
        .child(launch_options(s, u))
        .child(extra::steam_rows(s, u))
        .child(watch::game_rows(s, u))
}

fn launch_options(s: &State, u: &Ui) -> El {
    let read = s.settings.launch_read.as_ref();
    let found = read.and_then(|r| r.path.clone());
    let line = match read {
        None => "Not read yet.".to_string(),
        Some(LaunchRead { error: Some(error), .. }) => error.clone(),
        Some(LaunchRead { path: None, .. }) => {
            "No Steam account folder found on this computer.".to_string()
        }
        Some(LaunchRead { value: None, .. }) => {
            "Steam has none set for Deadlock. Quit Steam before saving.".to_string()
        }
        Some(LaunchRead { value: Some(_), .. }) => {
            "Read from Steam. Quit Steam before saving.".to_string()
        }
    };
    let input = rune::text_input(&s.settings.launch_options, MONO)
        .key("settings-launch-options-input")
        .label(tr("Steam launch options"))
        .single_line()
        .grow(1.0)
        .color(u.ink)
        .placeholder("-novid -high", u.faint)
        .caret(u.accent, u.accent.alpha(0.22))
        .on_edit(|st: &mut State| &mut st.settings.launch_options);
    let current = read.and_then(|r| r.value.clone()).unwrap_or_default();
    let typed = s.settings.launch_options.text.trim().to_string();
    let unchanged = typed == current.trim();
    let control = row()
        .w_full()
        .gap(8.0)
        .center_y()
        .child(
            row()
                .grow(1.0)
                .min_w(0.0)
                .h(32.0)
                .px(10.0)
                .center_y()
                .radius(crate::ui::r(8.0))
                .fill(u.well)
                .border(1.0, u.hair)
                .child(input),
        )
        .child(disabled(
            button(u, "settings-launch-options-save", "Save to Steam", ActionKind::Secondary)
                .on_press(move |st: &mut State| {
                    let value = st.settings.launch_options.text.trim().to_string();
                    st.settings.confirm = Some(Confirm::LaunchOptions(value));
                }),
            found.is_none() || unchanged,
        ))
        .child(
            u.icon_button(&icons::REFRESH_CW, tr("Read again from Steam"))
                .key("settings-launch-options-reload")
                .cursor(rune::Cursor::Pointer)
                .on_press(read_launch_options),
        );
    section(
        u,
        "Launch options",
        vec![stacked_row(
            u,
            "settings-launch-options",
            "Steam launch options",
            &line,
            control,
        )],
    )
}

/// Open a text file with the system's default app for it. Read only from Sigil's side: the
/// user edits in their own editor.
pub fn open_in_editor(st: &mut State, path: &Path) {
    editor::open(st, path);
}

fn choose_game_dir(_st: &mut State, tx: &mut Tx) {
    let options = FileDialogOptions {
        title: tr("Choose the Deadlock folder").into(),
        ..Default::default()
    };
    tx.open_folder(options, |st: &mut State, result, _tx| match result {
        Ok(Some(path)) => set_game_dir(st, path),
        Ok(None) => {}
        Err(error) => say(st, fill(tr("The folder picker could not open: {}"), &[&error.to_string()]), true),
    });
}

/// Use a folder the user picked, once it proves to hold the game.
pub fn set_game_dir(st: &mut State, path: PathBuf) {
    let found = deadlock_core::discovery::discover_deadlock_install(
        deadlock_core::discovery::InstallDiscoveryOptions {
            game_dir: Some(path.clone()),
            ..Default::default()
        },
    );
    match found {
        Ok(found) => match found.install {
            Some(install) => {
                let root = install.game_root.display().to_string();
                prefs::edit(st, |p| p.game_dir = Some(root.clone()));
                prefs::apply_game_dir(&st.prefs);
                crate::data::rescan_game(st);
                read_launch_options(st);
                say(st, fill(tr("Using {}."), &[&root]), false);
            }
            None => say(
                st,
                format!(
                    "{} has no game/citadel folder. Pick the Deadlock folder Steam installed.",
                    path.display()
                ),
                true,
            ),
        },
        Err(error) => say(st, fill(tr("That folder could not be read: {}"), &[&error.to_string()]), true),
    }
}

fn detect_game_dir(st: &mut State) {
    prefs::edit(st, |p| p.game_dir = None);
    prefs::apply_game_dir(&st.prefs);
    crate::data::rescan_game(st);
    read_launch_options(st);
    if st.game_detected {
        say(st, "Found Deadlock in your Steam library.", false);
    } else {
        say(st, "No Deadlock install in the Steam libraries.", true);
    }
}

fn read_launch_options(st: &mut State) {
    if st.demo {
        st.settings.launch_read = Some(LaunchRead {
            path: Some(PathBuf::from(
                "~/Library/Application Support/Steam/userdata/1/config/localconfig.vdf",
            )),
            value: Some("-novid +fps_max 240".into()),
            error: None,
        });
        st.settings.launch_options = TextEdit::new("-novid +fps_max 240");
        return;
    }
    let path = deadlock_core::steam_launch::find_localconfigs(st.game_root.as_deref())
        .into_iter()
        .next();
    let read = match &path {
        None => LaunchRead {
            path: None,
            value: None,
            error: None,
        },
        Some(path) => match deadlock_core::steam_launch::read_file(path) {
            Ok(value) => LaunchRead {
                path: Some(path.clone()),
                value,
                error: None,
            },
            Err(error) => LaunchRead {
                path: None,
                value: None,
                error: Some(fill(tr("Steam's settings file could not be read: {}"), &[&error.to_string()])),
            },
        },
    };
    st.settings.launch_options = TextEdit::new(read.value.clone().unwrap_or_default());
    st.settings.launch_read = Some(read);
}

// --- backups ---------------------------------------------------------------------

fn backup_sources(st: &State) -> BackupSources {
    let mut sources = match &st.game_root {
        Some(root) => BackupSources::for_game(root),
        None => BackupSources::default(),
    };
    if let Some(root) = &st.game_root {
        sources.video_dir = deadlock_core::performance::video::locate(root, &[])
            .and_then(|path| path.parent().map(Path::to_path_buf));
    }
    sources.localconfig_dir = deadlock_core::steam_launch::find_localconfigs(st.game_root.as_deref())
        .into_iter()
        .next()
        .and_then(|path| path.parent().map(Path::to_path_buf));
    sources.workspace = Some(crate::data::workspace_dir());
    sources
}

fn read_backups(st: &mut State) {
    if st.demo {
        st.settings.backups = Some(demo_backups());
        return;
    }
    let sources = backup_sources(st);
    st.settings.backups = Some(backups::list(&sources));
}

fn backups_tab(s: &State, u: &Ui) -> El {
    let entries = s.settings.backups.as_deref().unwrap_or(&[]);
    let stamped = entries.iter().filter(|e| !e.one_time).count();
    let mut targets: Vec<Option<&PathBuf>> = Vec::new();
    for entry in entries.iter().filter(|e| !e.one_time) {
        if !targets.contains(&entry.target.as_ref()) {
            targets.push(entry.target.as_ref());
        }
    }
    let prunable = stamped > targets.len();
    let actions = row()
        .w_full()
        .gap(8.0)
        .center_y()
        .child(
            text(if entries.is_empty() {
                "Copies Sigil keeps before it edits a game or Steam file.".to_string()
            } else {
                format!(
                    "{} cop{}, {}. Restoring keeps a copy of the file it replaces.",
                    entries.len(),
                    if entries.len() == 1 { "y" } else { "ies" },
                    size_label(entries.iter().map(|e| e.bytes).sum())
                )
            })
            .font(SMALL)
            .color(u.muted)
            .grow(1.0)
            .min_w(0.0)
            .wrap(),
        )
        .child(disabled(
            button(u, "settings-backups-prune", "Delete older copies", ActionKind::Secondary)
                .on_press(|st: &mut State| st.settings.confirm = Some(Confirm::Prune)),
            !prunable,
        ))
        .child(
            u.icon_button(&icons::REFRESH_CW, tr("Refresh the list"))
                .key("settings-backups-refresh")
                .cursor(rune::Cursor::Pointer)
                .on_press(read_backups),
        );
    if entries.is_empty() {
        return col().w_full().gap(16.0).child(actions).child(
            u.empty_state(
                "",
                tr("No backups yet"),
                tr("They appear here after Sigil repairs gameinfo.gi, saves autoexec.cfg, applies a performance config or replaces a mod."),
                None,
            )
            .py(48.0),
        );
    }
    let mut rows = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        rows.push(backup_row(u, index, entry));
    }
    col().w_full().gap(16.0).child(actions).child(section(u, "", rows))
}

fn backup_row(u: &Ui, index: usize, entry: &BackupEntry) -> El {
    let name = entry
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let when = entry.modified.map(age_label).unwrap_or_default();
    let mut meta = vec![when, size_label(entry.bytes)];
    if entry.kind == BackupKind::Addons {
        meta.push(format!(
            "{} file{}",
            entry.files,
            if entry.files == 1 { "" } else { "s" }
        ));
    }
    if entry.one_time {
        meta.push("first copy, kept".to_string());
    }
    let line = row()
        .gap(8.0)
        .min_w(0.0)
        .child(text(meta.join(" · ")).font(SMALL).color(u.muted))
        .child(
            text(name)
                .font(MONO)
                .color(u.faint)
                .truncate()
                .shrink()
                .min_w(0.0),
        );
    let path = entry.path.clone();
    let reveal_path = entry.path.display().to_string();
    let mut actions = Vec::new();
    if entry.target.is_some() {
        let restore = path.clone();
        actions.push(
            button(u, &format!("settings-backup-restore-{index}"), "Restore", ActionKind::Secondary)
                .on_press(move |st: &mut State| {
                    st.settings.confirm = Some(Confirm::Restore(restore.clone()))
                }),
        );
    }
    actions.push(
        u.icon_button(&icons::FOLDER, tr("Reveal"))
            .key(format!("settings-backup-reveal-{index}"))
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| crate::data::reveal_path(st, reveal_path.clone())),
    );
    actions.push(
        u.icon_button(&icons::TRASH_2, tr("Delete this backup"))
            .key(format!("settings-backup-delete-{index}"))
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| {
                st.settings.confirm = Some(Confirm::Delete(path.clone()))
            }),
    );
    setting_row(
        u,
        &format!("settings-backup-{index}"),
        entry.kind.label(),
        line,
        trailing(actions),
    )
}

fn demo_backups() -> Vec<BackupEntry> {
    let now = SystemTime::now();
    let citadel = PathBuf::from("~/Steam/steamapps/common/Deadlock/game/citadel");
    let entry = |kind, path: PathBuf, hours: u64, bytes, one_time| BackupEntry {
        kind,
        target: backups::target_for(&path),
        path,
        bytes,
        modified: now.checked_sub(Duration::from_secs(hours * 3600)),
        files: 1,
        one_time,
    };
    vec![
        entry(
            BackupKind::Gameinfo,
            citadel.join("gameinfo.gi.dah-backup-2026-11-16T21-04-11-882Z-4410"),
            3,
            5_312,
            false,
        ),
        entry(
            BackupKind::Autoexec,
            citadel.join("cfg/autoexec.cfg.dah-backup-2026-11-15T10-12-40-101Z"),
            38,
            1_204,
            false,
        ),
        entry(
            BackupKind::Video,
            PathBuf::from("~/Steam/userdata/1/1422450/local/cfg/video.txt.sigil-bak"),
            120,
            2_880,
            true,
        ),
        BackupEntry {
            kind: BackupKind::Addons,
            path: PathBuf::from("~/Sigil/workspace/.dah/install/backups/2026-11-12T18-30-00-000Z"),
            target: None,
            bytes: 18_400_000,
            modified: now.checked_sub(Duration::from_secs(5 * 86_400)),
            files: 2,
            one_time: false,
        },
    ]
}

// --- appearance ------------------------------------------------------------------

fn appearance(s: &State, u: &Ui) -> El {
    let mut tiles = el().key("settings-themes").w_full().equal_columns(3).gap(16.0);
    for theme in Theme::ALL {
        tiles = tiles.child(theme_tile(s, u, theme));
    }

    let mut swatches = row().key("settings-accents").w_full().gap(10.0).center_y();
    for (id, label, _) in ACCENTS {
        let accent = if id == "ember" {
            Accent::Theme
        } else {
            Accent::Preset(id)
        };
        let color = accent
            .color()
            .unwrap_or_else(|| Ui::themed(s.prefs.theme, None).accent);
        let on = s.prefs.accent == accent;
        swatches = swatches.child(swatch(u, id, label, color, on, accent));
    }
    let custom_on = matches!(s.prefs.accent, Accent::Custom(_));
    let typed = prefs::parse_hex(&s.settings.accent_hex.text);
    let hex_input = rune::text_input(&s.settings.accent_hex, MONO)
        .key("settings-accent-hex-input")
        .label(tr("Custom accent, hex"))
        .single_line()
        .grow(1.0)
        .color(u.ink)
        .placeholder("#22c55e", u.faint)
        .caret(u.accent, u.accent.alpha(0.22))
        .on_edit(|st: &mut State| &mut st.settings.accent_hex);
    let custom = row()
        .gap(8.0)
        .center_y()
        .child(spacer())
        .child(
            el().size(20.0, 20.0)
                .radius(crate::ui::r(10.0))
                .fill(typed.unwrap_or(Color::TRANSPARENT))
                .border(1.0, u.hair_strong)
                .a11y_hidden(),
        )
        .child(
            row()
                .w(132.0)
                .h(32.0)
                .px(10.0)
                .center_y()
                .radius(crate::ui::r(8.0))
                .fill(u.well)
                .border(1.0, if custom_on { u.accent } else { u.hair })
                .child(hex_input),
        )
        .child(disabled(
            button(u, "settings-accent-apply", "Use", ActionKind::Secondary).on_press(
                |st: &mut State| {
                    let typed = st.settings.accent_hex.text.trim().to_lowercase();
                    match prefs::parse_hex(&typed) {
                        Some(_) => {
                            let hex = if typed.starts_with('#') {
                                typed
                            } else {
                                format!("#{typed}")
                            };
                            prefs::edit(st, |p| {
                                p.remember_accent(&hex);
                                p.accent = Accent::Custom(hex);
                            });
                        }
                        None => say(st, "Type a colour as #rgb or #rrggbb.", true),
                    }
                },
            ),
            typed.is_none(),
        ));

    let mut accent_rows = vec![stacked_row(
        u,
        "settings-accent",
        "Accent colour",
        "Marks the main action, switches that are on, and focus. Or type any hex colour.",
        swatches.child(custom),
    )];
    if !s.prefs.custom_accents.is_empty() {
        let mut mine = row().key("settings-my-accents").w_full().gap(10.0).center_y();
        for (index, hex) in s.prefs.custom_accents.iter().enumerate() {
            let color = prefs::parse_hex(hex).unwrap_or(Color::TRANSPARENT);
            let accent = Accent::Custom(hex.clone());
            let on = s.prefs.accent == accent;
            let gone = hex.clone();
            mine = mine.child(
                row()
                    .gap(2.0)
                    .center_y()
                    .child(swatch(u, &format!("mine-{index}"), hex, color, on, accent))
                    .child(
                        u.icon_button(&icons::X, tr("Remove this colour"))
                            .key(format!("settings-accent-remove-{index}"))
                            .cursor(rune::Cursor::Pointer)
                            .on_press(move |st: &mut State| {
                                let gone = gone.clone();
                                prefs::edit(st, |p| {
                                    p.custom_accents.retain(|h| *h != gone);
                                    if p.accent == Accent::Custom(gone.clone()) {
                                        p.accent = Accent::Theme;
                                    }
                                })
                            }),
                    ),
            );
        }
        accent_rows.push(stacked_row(
            u,
            "settings-my-accents-row",
            "Your colours",
            "Colours you used before. Sigil keeps the last six.",
            mine,
        ));
    }
    let zooms: Vec<(u32, &str)> = vec![(90, "90%"), (100, "100%"), (110, "110%"), (125, "125%")];
    let zoom_now = (s.prefs.zoom * 100.0).round() as u32;

    col()
        .w_full()
        .gap(28.0)
        .child(section(
            u,
            "Theme",
            vec![
                col().w_full().pt(16.0).pb(4.0).child(tiles),
            ],
        ))
        .child(section(
            u,
            "",
            accent_rows
                .into_iter()
                .chain([setting_row(
                u,
                "settings-zoom",
                "Interface size",
                detail(u, "Scales text, icons and spacing together."),
                segmented(u, "settings-zoom-pick", &zooms, zoom_now, |st, pct| {
                    prefs::edit(st, |p| p.zoom = pct as f32 / 100.0)
                }),
            ),
            toggle_row(
                u,
                "settings-transparent-sidebar",
                "Transparent sidebar",
                "The sidebar sits on the page background, without its own panel.",
                s.prefs.transparent_sidebar,
                |st| prefs::edit(st, |p| p.transparent_sidebar = !p.transparent_sidebar),
            ),
            toggle_row(
                u,
                "settings-status-bar",
                "Status bar",
                "A quiet line under every page: the game, the catalog, downloads, your profile, the preview volume and updates.",
                s.prefs.status_bar,
                |st| prefs::edit(st, |p| p.status_bar = !p.status_bar),
            )])
            .collect(),
        ))
        .child(personalize::view(s, u))
}

/// A small picture of the theme: its sidebar, a page with three lines, and the accent.
fn theme_tile(s: &State, u: &Ui, theme: Theme) -> El {
    let t = Ui::themed(theme, s.prefs.accent_color());
    let on = s.prefs.theme == theme && s.prefs.personal.theme.is_none();
    let picture = row()
        .w_full()
        .h(84.0)
        .padding(1.0)
        .radius(crate::ui::r(8.0))
        .fill(t.bg)
        .child(
            col()
                .w(36.0)
                .h_full()
                .corners([7.0, 0.0, 0.0, 7.0])
                .fill(t.sidebar)
                .border_sides([0.0, 1.0, 0.0, 0.0], t.hair)
                .pt(12.0)
                .px(7.0)
                .gap(5.0)
                .child(el().w(22.0).h(4.0).radius(2.0).fill(t.ink.alpha(0.6)))
                .child(el().w(18.0).h(4.0).radius(2.0).fill(t.muted.alpha(0.5)))
                .child(el().w(20.0).h(4.0).radius(2.0).fill(t.muted.alpha(0.5))),
        )
        .child(
            col()
                .grow(1.0)
                .h_full()
                .pt(12.0)
                .px(10.0)
                .gap(6.0)
                .child(el().w(52.0).h(6.0).radius(3.0).fill(t.ink.alpha(0.85)))
                .child(el().w_full().h(4.0).radius(2.0).fill(t.muted.alpha(0.45)))
                .child(el().w(46.0).h(4.0).radius(2.0).fill(t.muted.alpha(0.45)))
                .child(spacer())
                .child(
                    row()
                        .w_full()
                        .pb(10.0)
                        .child(spacer())
                        .child(el().w(30.0).h(12.0).radius(4.0).fill(t.accent)),
                ),
        );
    col()
        .key(format!("settings-theme-{}", theme.id()))
        .gap(8.0)
        .focusable(true)
        .role("radio")
        .checked(on)
        .label(tr(theme.label()))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            st.settings.personalize.draft = None;
            prefs::edit(st, |p| {
                p.theme = theme;
                p.personal.theme = None;
            })
        })
        .child(
            col()
                .w_full()
                .padding(3.0)
                .radius(11.0)
                .border(2.0, if on { u.accent } else { Color::TRANSPARENT })
                .child(picture.border(1.0, t.hair_strong)),
        )
        .child(
            text(tr(theme.label()))
                .font(LABEL)
                .color(if on { u.ink } else { u.muted })
                .a11y_hidden(),
        )
}

fn swatch(u: &Ui, id: &str, label: &str, color: Color, on: bool, accent: Accent) -> El {
    el().key(format!("settings-accent-{id}"))
        .size(28.0, 28.0)
        .radius(crate::ui::r(14.0))
        .border(2.0, if on { u.ink } else { Color::TRANSPARENT })
        .center()
        .focusable(true)
        .role("radio")
        .checked(on)
        .label(tr(label).to_string())
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            let accent = accent.clone();
            prefs::edit(st, |p| p.accent = accent)
        })
        .child(el().size(20.0, 20.0).radius(crate::ui::r(10.0)).fill(color).a11y_hidden())
}

// --- mods ------------------------------------------------------------------------

fn mods(s: &State, u: &Ui) -> El {
    let mature: Vec<(Mature, &str)> = Mature::ALL.iter().map(|m| (*m, m.label())).collect();
    let sorts: Vec<(BrowseSort, &str)> = prefs::SORTS
        .iter()
        .map(|sort| (*sort, prefs::sort_label(*sort)))
        .collect();
    let hidden_mods = s.browse_view.hidden_mods.len();
    let hidden_authors = s.browse_view.hidden_authors.len();
    let hidden_line = if hidden_mods + hidden_authors == 0 {
        tr("Nothing hidden. Hide a mod or creator from its menu in Browse.").to_string()
    } else {
        fill(
            tr("{} mods and {} creators hidden until Sigil restarts."),
            &[&hidden_mods.to_string(), &hidden_authors.to_string()],
        )
    };
    let mut hidden_rows = Vec::new();
    let mut authors: Vec<&String> = s.browse_view.hidden_authors.iter().collect();
    authors.sort();
    for (index, author) in authors.into_iter().enumerate() {
        let name = author.clone();
        hidden_rows.push(setting_row(
            u,
            &format!("settings-hidden-author-{index}"),
            author,
            detail(u, "Creator"),
            button(u, &format!("settings-hidden-author-show-{index}"), "Show", ActionKind::Ghost)
                .on_press(move |st: &mut State| {
                    st.browse_view.hidden_authors.remove(&name);
                }),
        ));
    }
    let mut ids: Vec<u64> = s.browse_view.hidden_mods.iter().copied().collect();
    ids.sort_unstable();
    for id in ids {
        let page = format!("https://gamebanana.com/mods/{id}");
        hidden_rows.push(setting_row(
            u,
            &format!("settings-hidden-mod-{id}"),
            &fill(tr("Mod {}"), &[&id.to_string()]),
            detail(u, "Mod"),
            trailing(vec![
                u.icon_button(&icons::EXTERNAL_LINK, tr("Open on GameBanana"))
                    .key(format!("settings-hidden-mod-open-{id}"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press_tx(move |st: &mut State, tx: &mut Tx| tabs::open_url(st, tx, &page)),
                button(u, &format!("settings-hidden-mod-show-{id}"), "Show", ActionKind::Ghost)
                    .on_press(move |st: &mut State| {
                        st.browse_view.hidden_mods.remove(&id);
                    }),
            ]),
        ));
    }
    col()
        .w_full()
        .gap(28.0)
        .child(section(
            u,
            "Installing",
            vec![toggle_row(
                u,
                "settings-land-disabled",
                "New mods start turned off",
                "Installs land in a free slot, off, until you switch them on in Installed.",
                s.prefs.land_disabled,
                |st| prefs::edit(st, |p| p.land_disabled = !p.land_disabled),
            )],
        ))
        .child(section(
            u,
            "Browse",
            vec![
                setting_row(
                    u,
                    "settings-sort",
                    "Order",
                    detail(u, "How the GameBanana catalog is sorted."),
                    segmented(u, "settings-sort-pick", &sorts, s.prefs.browse_sort, |st, sort| {
                        prefs::edit(st, |p| p.browse_sort = sort);
                        if st.browse.page.is_some() || st.screen == Screen::Browse {
                            crate::data::start_browse_fetch(st, 1);
                        }
                    }),
                ),
                setting_row(
                    u,
                    "settings-mature",
                    "Mature content",
                    detail(u, "Mods GameBanana marks as mature."),
                    segmented(u, "settings-mature-pick", &mature, s.prefs.mature, |st, m| {
                        prefs::edit(st, |p| p.mature = m)
                    }),
                ),
                setting_row(
                    u,
                    "settings-hidden",
                    "Hidden mods and creators",
                    detail(u, &hidden_line),
                    disabled(
                        button(u, "settings-hidden-clear", "Show all", ActionKind::Secondary)
                            .on_press(|st: &mut State| {
                                st.browse_view.hidden_mods.clear();
                                st.browse_view.hidden_authors.clear();
                            }),
                        hidden_mods + hidden_authors == 0,
                    ),
                ),
            ]
            .into_iter()
            .chain(hidden_rows)
            .collect(),
        ))
}

// --- privacy ---------------------------------------------------------------------

const HOSTS: [(&str, &str); 8] = [
    (
        "Sigil's telemetry server",
        "Only what you turn on under Help improve Sigil: usage events, crash reports, unknown-mod lookups, and diagnostics when you press Send.",
    ),
    (
        "gamebanana.com",
        "Browse: the catalog, mod pages, previews and the files you install. Your public name when you link your account.",
    ),
    (
        "api.deadlock-api.com",
        "Stats for the account you enter, crosshair share codes, and match data when you share it.",
    ),
    (
        "api.github.com",
        "Performance configs, and the update check when it is on.",
    ),
    ("raw.githubusercontent.com", "Performance config files."),
    ("api.deadworks.net", "Servers: the server list and joins."),
    (
        "Discord on this computer",
        "Rich Presence, through the Discord app's local connection. No network.",
    ),
    (
        "Your model provider",
        "The agent, with your own key or account, only when you use it.",
    ),
];

fn privacy(s: &State, u: &Ui) -> El {
    let mut hosts = Vec::new();
    for (index, (host, purpose)) in HOSTS.iter().enumerate() {
        hosts.push(
            row()
                .key(format!("settings-host-{index}"))
                .w_full()
                .min_h(44.0)
                .py(8.0)
                .gap(16.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(
                    text(tr(host).to_string())
                        .font(if host.contains('.') { MONO } else { BODY_MED })
                        .color(u.ink)
                        .w(200.0),
                )
                .child(
                    text(tr(purpose).to_string())
                        .font(SMALL)
                        .color(u.muted)
                        .grow(1.0)
                        .min_w(0.0)
                        .wrap(),
                ),
        );
    }
    col()
        .w_full()
        .gap(28.0)
        .child(
            text(tr("Sigil goes online only for the features below. Usage events, crash reports and lookups go to Sigil's own server only when you turn them on."))
                .font(BODY)
                .color(u.prose)
                .wrap()
                .w_full(),
        )
        .child(telemetry::privacy_section(s, u))
        .child(tabs::match_sharing(s, u))
        .child(section(
            u,
            "Optional features",
            vec![
                toggle_row(
                    u,
                    "settings-allow-stats",
                    "Player stats",
                    "Looks up public match data for the account you enter on Stats.",
                    s.prefs.allow_stats,
                    |st| prefs::edit(st, |p| p.allow_stats = !p.allow_stats),
                ),
                toggle_row(
                    u,
                    "settings-allow-share",
                    "Crosshair share codes",
                    "Sends your crosshair values to make a code, when you press Copy code.",
                    s.prefs.allow_share_codes,
                    |st| prefs::edit(st, |p| p.allow_share_codes = !p.allow_share_codes),
                ),
                toggle_row(
                    u,
                    "settings-privacy-updates",
                    "Update check",
                    "Asks GitHub for the newest Sigil release when Sigil starts.",
                    s.prefs.check_updates,
                    |st| prefs::edit(st, |p| p.check_updates = !p.check_updates),
                ),
            ],
        ))
        .child(section(u, "Where Sigil connects", hosts))
}

// --- models ----------------------------------------------------------------------

const ACCOUNTS: [(&str, &str); 2] = [("codex", "ChatGPT"), ("grok", "Grok")];

fn models(s: &State, u: &Ui) -> El {
    let pending = crate::signin::pending(s);
    let mut rows = Vec::new();
    for (id, name) in ACCOUNTS {
        rows.push(account_row(s, u, id, name, pending.as_ref()));
        if let Some(p) = pending.as_ref().filter(|p| p.provider == id) {
            rows.push(pending_block(u, p));
        }
    }
    rows.push(api_key_row(s, u));

    let mut accounts = col().w_full().gap(8.0).child(section(u, "Accounts", rows));
    if let Some((message, is_error)) = &s.signin.message {
        accounts = accounts.child(
            text(message.clone())
                .font(SMALL)
                .color(if *is_error { u.rose } else { u.muted })
                .wrap()
                .w_full(),
        );
    } else if let Some(log) = &s.subscription_log {
        accounts = accounts.child(text(log.clone()).font(SMALL).color(u.muted).wrap().w_full());
    }

    let model_field = field(
        rune::text_input(&s.host_model, MONO)
            .key("host-model")
            .label(tr("Model id"))
            .single_line()
            .grow(1.0)
            .color(u.ink)
            .placeholder("gpt-5-codex", u.faint)
            .caret(u.accent, u.accent.alpha(0.22))
            .on_edit(|st: &mut State| &mut st.host_model),
        u,
    );
    let endpoint_field = field(
        rune::text_input(&s.local_endpoint, MONO)
            .key("local-endpoint")
            .label(tr("Local endpoint"))
            .single_line()
            .grow(1.0)
            .color(u.ink)
            .placeholder("http://127.0.0.1:8080/v1", u.faint)
            .caret(u.accent, u.accent.alpha(0.22))
            .on_edit(|st: &mut State| &mut st.local_endpoint),
        u,
    );

    let model = section(
        u,
        "Model",
        vec![
            setting_row(
                u,
                "settings-model-id",
                "Model id",
                detail(u, "The exact id your account offers."),
                model_field,
            ),
            setting_row(
                u,
                "settings-local-endpoint",
                "Local endpoint",
                detail(u, "Any server that speaks the OpenAI API."),
                endpoint_field,
            ),
        ],
    );

    col().w_full().gap(28.0).child(accounts).child(model)
}

fn account_row(
    s: &State,
    u: &Ui,
    id: &'static str,
    name: &str,
    pending: Option<&crate::signin::Pending>,
) -> El {
    let signed_in = s
        .subscriptions
        .iter()
        .any(|row| row.id == id && row.signed_in);
    let waiting = pending.is_some_and(|p| p.provider == id);
    let busy_elsewhere = pending.is_some() && !waiting;
    let line = if waiting {
        "Signing in"
    } else if signed_in {
        "Signed in"
    } else {
        "Not signed in"
    };

    let actions = if signed_in {
        trailing(vec![
            button(u, &format!("settings-use-{id}"), "Use", ActionKind::Secondary)
                .on_press(move |st: &mut State| crate::data::use_subscription_for_agent(st, id)),
            button(u, &format!("settings-sign-out-{id}"), "Sign out", ActionKind::Ghost)
                .on_press(move |st: &mut State| crate::signin::sign_out(st, id)),
        ])
    } else if waiting {
        trailing(vec![])
    } else {
        disabled(
            button(u, &format!("settings-sign-in-{id}"), "Sign in", ActionKind::Secondary)
                .on_press(move |st: &mut State| crate::signin::start(st, id)),
            busy_elsewhere,
        )
    };

    let mut r = setting_row(
        u,
        &format!("settings-account-{id}"),
        name,
        detail(u, line),
        actions,
    );
    if waiting {
        r = r.border_sides([0.0, 0.0, 0.0, 0.0], u.hair);
    }
    r
}

/// The browser step: one calm block under the provider row.
fn pending_block(u: &Ui, p: &crate::signin::Pending) -> El {
    let url = p.url.clone();
    let has_url = !url.is_empty();
    let mut block = col()
        .key(format!("settings-pending-{}", p.provider))
        .w_full()
        .pb(16.0)
        .gap(12.0)
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(
            text(tr("Finish signing in in your browser."))
                .font(BODY)
                .color(u.prose),
        );
    if let Some(code) = &p.code {
        let copy = code.clone();
        block = block.child(
            row()
                .gap(8.0)
                .center_y()
                .child(
                    row()
                        .h(40.0)
                        .px(14.0)
                        .center_y()
                        .radius(crate::ui::r(8.0))
                        .fill(u.well)
                        .border(1.0, u.hair)
                        .child(
                            text(code.clone())
                                .font(
                                    rune::TextStyle::new(14.0, 600.0)
                                        .lh(20.0)
                                        .family(rune::Family::SystemMono),
                                )
                                .color(u.ink)
                                .selectable(true),
                        ),
                )
                .child(
                    u.icon_button(&icons::COPY, tr("Copy code"))
                        .key("settings-pending-copy")
                        .cursor(rune::Cursor::Pointer)
                        .on_press_tx(move |_st: &mut State, tx: &mut Tx| tx.copy(copy.clone())),
                ),
        );
    }
    let mut buttons = row().gap(8.0).center_y();
    if has_url {
        buttons = buttons.child(
            u.icon_action(&icons::EXTERNAL_LINK, tr("Open sign-in page"),
                ActionKind::Secondary,
            )
            .key("settings-pending-open")
            .cursor(rune::Cursor::Pointer)
            .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                if let Err(error) = tx.open_url(&url) {
                    st.signin.message = Some((format!("{error}"), true));
                }
            }),
        );
    } else {
        buttons = buttons.child(text(tr("Opening your browser")).font(SMALL).color(u.faint));
    }
    buttons = buttons.child(
        button(u, "settings-pending-cancel", "Cancel", ActionKind::Ghost)
            .on_press(|st: &mut State| crate::signin::cancel(st)),
    );
    block.child(buttons)
}

/// Keys for the other providers are added outside the window; this row shows which are saved.
fn api_key_row(s: &State, u: &Ui) -> El {
    let saved: Vec<&str> = s
        .providers
        .iter()
        .filter(|p| p.kind == "BYOK" && p.state == ProviderState::Configured)
        .map(|p| p.name.as_str())
        .collect();
    let line = if saved.is_empty() {
        "No key saved".to_string()
    } else {
        format!("{} key saved", saved.join(" and "))
    };
    setting_row(
        u,
        "settings-account-api",
        "API key",
        detail(u, &line),
        status_word(u, if saved.is_empty() { "None" } else { "Saved" }, false),
    )
}

// --- storage ---------------------------------------------------------------------

fn image_cache_dir() -> PathBuf {
    crate::data::data_dir().join("cache").join("images")
}

fn read_sizes(st: &mut State) {
    st.settings.image_cache = Some(dir_size(&image_cache_dir()));
    st.settings.download_cache = Some(dir_size(
        &crate::data::workspace_dir().join(deadlock_core::install::DOWNLOADS_DIR_NAME),
    ));
}

fn dir_size(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.metadata() {
            Ok(meta) if meta.is_dir() => dir_size(&entry.path()),
            Ok(meta) => meta.len(),
            Err(_) => 0,
        })
        .sum()
}

fn storage(s: &State, u: &Ui) -> El {
    let data = s.paths.data.clone();
    let sessions = s.state_sessions_dir();
    let workspace = crate::data::workspace_dir().display().to_string();
    let workspace_shown = tidy(&workspace);
    let size_or = |bytes: Option<u64>| bytes.map(size_label).unwrap_or_else(|| "Not measured".into());

    let folders = section(
        u,
        "Folders",
        vec![
            setting_row(
                u,
                "settings-data-folder",
                "Data folder",
                path_detail(u, &tidy(&s.paths.data)),
                button(u, "settings-data-reveal", "Reveal", ActionKind::Secondary)
                    .on_press(move |st: &mut State| crate::data::reveal_path(st, data.clone())),
            ),
            setting_row(
                u,
                "settings-workspace-folder",
                "Mod workspace",
                path_detail(u, &workspace_shown),
                button(u, "settings-workspace-reveal", "Reveal", ActionKind::Secondary).on_press(
                    move |st: &mut State| crate::data::reveal_path(st, workspace.clone()),
                ),
            ),
            setting_row(
                u,
                "settings-sessions-folder",
                "Agent sessions",
                detail(u, "Your conversations with the agent, one file each."),
                button(u, "settings-sessions-reveal", "Reveal", ActionKind::Secondary)
                    .on_press(move |st: &mut State| crate::data::reveal_path(st, sessions.clone())),
            ),
        ],
    );

    let caches = section(
        u,
        "Caches",
        vec![
            setting_row(
                u,
                "settings-image-cache",
                "Preview images",
                detail(
                    u,
                    &format!(
                        "{}. Browse fetches them again as you scroll.",
                        size_or(s.settings.image_cache)
                    ),
                ),
                disabled(
                    button(u, "settings-image-cache-clear", "Clear", ActionKind::Secondary)
                        .on_press(clear_image_cache),
                    s.settings.image_cache.unwrap_or(0) == 0,
                ),
            ),
            setting_row(
                u,
                "settings-download-cache",
                "Downloads",
                detail(
                    u,
                    &format!(
                        "{}. Archives kept after an install. Installed mods stay.",
                        size_or(s.settings.download_cache)
                    ),
                ),
                disabled(
                    button(u, "settings-download-cache-clear", "Clear", ActionKind::Secondary)
                        .on_press(clear_download_cache),
                    s.settings.download_cache.unwrap_or(0) == 0,
                ),
            ),
        ],
    );

    let maintenance = section(
        u,
        "Maintenance",
        vec![
            setting_row(
                u,
                "settings-rescan",
                "Rescan",
                detail(u, "Find the game again and reread your addon slots."),
                button(u, "settings-rescan-run", "Rescan", ActionKind::Secondary).on_press(
                    |st: &mut State| {
                        crate::data::rescan_game(st);
                        let line = if st.game_detected {
                            "Rescanned the game folder."
                        } else {
                            "No Deadlock install found."
                        };
                        say(st, line, !st.game_detected);
                    },
                ),
            ),
            leftover_row(s, u),
            agent_row(s, u),
            setting_row(
                u,
                "settings-toolchain",
                "Toolchain",
                detail(u, "Compiler and packing tools for building mods."),
                button(u, "settings-toolchain-open", "Open", ActionKind::Secondary).on_press(
                    |st: &mut State| {
                        close(st);
                        st.go(Screen::Toolchain);
                    },
                ),
            ),
            setting_row(
                u,
                "settings-reset",
                "Reset settings",
                detail(u, "Every setting on these pages goes back to its default."),
                button(u, "settings-reset-run", "Reset", ActionKind::Danger)
                    .on_press(|st: &mut State| st.settings.confirm = Some(Confirm::Reset)),
            ),
        ],
    );

    col()
        .w_full()
        .gap(28.0)
        .child(folders)
        .child(caches)
        .child(section(u, "Mod data", tools::storage_rows(s, u)))
        .child(maintenance)
}

fn clear_image_cache(st: &mut State) {
    let dir = image_cache_dir();
    let mut freed = 0;
    let mut failed = 0;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_file() {
                match std::fs::remove_file(entry.path()) {
                    Ok(()) => freed += meta.len(),
                    Err(_) => failed += 1,
                }
            }
        }
    }
    read_sizes(st);
    if failed > 0 {
        say(
            st,
            fill(tr("Freed {}. {} files were in use."), &[&size_label(freed), &failed.to_string()]),
            true,
        );
    } else {
        say(st, fill(tr("Freed {}."), &[&size_label(freed)]), false);
    }
}

fn clear_download_cache(st: &mut State) {
    match deadlock_core::install::prune_download_cache(&crate::data::workspace_dir()) {
        Ok(report) => {
            read_sizes(st);
            say(st, fill(tr("Freed {}."), &[&size_label(report.total_bytes)]), false);
        }
        Err(error) => say(st, fill(tr("The download cache could not be cleared: {}"), &[&error.to_string()]), true),
    }
}

fn leftover_row(s: &State, u: &Ui) -> El {
    let found = s.settings.leftover.as_deref().unwrap_or(&[]);
    let line = match &s.settings.leftover {
        None => tr("Appears once the game folder is found.").to_string(),
        Some(list) if list.is_empty() => tr("None. The addons folder holds only what the game reads.").to_string(),
        Some(list) => fill(
            tr("{} archives, {}, in the addons folder. The game ignores them."),
            &[&list.len().to_string(), &size_label(list.iter().map(|(_, b)| b).sum())],
        ),
    };
    setting_row(
        u,
        "settings-leftover",
        "Leftover archives",
        detail(u, &line),
        disabled(
            button(u, "settings-leftover-clean", "Clean up", ActionKind::Secondary)
                .on_press(|st: &mut State| st.settings.confirm = Some(Confirm::CleanupArchives)),
            found.is_empty(),
        ),
    )
}

/// The agent host in one quiet line, with restart.
fn agent_row(s: &State, u: &Ui) -> El {
    let sessions = s.agent.session_ids.len();
    let mut line = format!(
        "{} · {}",
        crate::ui::sentence(&capitalize(s.agent.status.label())),
        s.agent.model_label()
    );
    if sessions > 0 {
        line.push_str(&format!(
            " · {sessions} session{}",
            if sessions == 1 { "" } else { "s" }
        ));
    }
    let line_el = match &s.agent.error {
        Some(error) => text(error.clone()).font(SMALL).color(u.rose).truncate(),
        None => detail(u, &line),
    };
    setting_row(
        u,
        "settings-agent-host",
        "Agent",
        line_el,
        trailing(vec![
            button(u, "settings-agent-sessions", "Sessions", ActionKind::Ghost).on_press(
                |st: &mut State| {
                    if !st.agent.list_sessions() {
                        let line = st
                            .agent
                            .error
                            .clone()
                            .unwrap_or_else(|| "The agent is not running.".to_string());
                        say(st, line, true);
                    }
                },
            ),
            button(u, "settings-agent-restart", "Restart", ActionKind::Secondary)
                .on_press(|st: &mut State| st.restart_host()),
        ]),
    )
}

// --- updates ---------------------------------------------------------------------

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn updates(s: &State, u: &Ui) -> Vec<El> {
    let checking = s.settings.update_job.is_running();
    let (line, warn, offer) = match &s.settings.update {
        _ if checking => (tr("Checking GitHub").to_string(), false, None),
        None if s.prefs.offline => (tr("Offline mode is on.").to_string(), false, None),
        None => (tr("Not checked yet.").to_string(), false, None),
        Some(Err(error)) => (fill(tr("The check failed: {}"), &[error]), true, None),
        Some(Ok(UpdateCheck { latest: None, .. })) => {
            (tr("No release is published yet.").to_string(), false, None)
        }
        Some(Ok(UpdateCheck {
            latest: Some(r),
            newer: true,
        })) => (fill(tr("Sigil {} is available."), &[&r.version]), false, Some(r)),
        Some(Ok(UpdateCheck {
            latest: Some(r), ..
        })) => (fill(tr("Up to date. The newest release is {}."), &[&r.version]), false, None),
    };
    let mut rows = vec![setting_row(
        u,
        "settings-version",
        "Newest release",
        text(line).font(SMALL).color(if warn { u.rose } else { u.muted }).truncate(),
        trailing(vec![disabled(
            button(u, "settings-update-check", "Check now", ActionKind::Secondary)
                .on_press(start_update_check),
            checking || s.prefs.offline,
        )]),
    )];
    if let Some((version, back)) = &s.settings.updater.rolled_back {
        rows.push(setting_row(
            u,
            "settings-update-undone",
            "Update undone",
            text(fill(tr("Sigil {} did not start, so Sigil went back to {}."), &[version, back]))
                .font(SMALL)
                .color(u.amber)
                .wrap(),
            el(),
        ));
    }
    if let Some(offer) = offer {
        rows.push(release_notes(u, offer));
    }
    if let Some(install) = install_row(s, u, offer) {
        rows.push(install);
    }
    rows.push(tabs::channel_row(s, u));
    rows
}

/// "What's new": the release body as plain lines and a link to the release page.
fn release_notes(u: &Ui, offer: &Release) -> El {
    let url = offer.url.clone();
    let notes = crate::updater::plain_notes(&offer.notes, 14);
    let mut c = col()
        .key("settings-update-notes")
        .w_full()
        .gap(8.0)
        .pt(12.0)
        .pb(16.0)
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(
            row()
                .w_full()
                .gap(12.0)
                .center_y()
                .child(
                    text(fill(tr("What's new in {}"), &[&offer.version]))
                        .font(BODY_MED)
                        .color(u.ink)
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate(),
                )
                .child(
                    u.icon_action(&icons::EXTERNAL_LINK, tr("Release page"), ActionKind::Ghost)
                        .key("settings-update-open")
                        .cursor(rune::Cursor::Pointer)
                        .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                            if let Err(error) = tx.open_url(&url) {
                                say(st, format!("{error}"), true);
                            }
                        }),
                ),
        );
    c = c.child(if notes.is_empty() {
        text(tr("No notes for this release.")).font(SMALL).color(u.faint)
    } else {
        text(notes).font(SMALL).color(u.muted).wrap().selectable(true)
    });
    c
}

/// Download progress, "Restart to update", or why this copy cannot install the release.
fn install_row(s: &State, u: &Ui, offer: Option<&Release>) -> Option<El> {
    let up = &s.settings.updater;
    if let Some(version) = up.ready() {
        return Some(setting_row(
            u,
            "settings-update-ready",
            &fill(tr("Sigil {} is ready"), &[version]),
            detail(u, "Sigil restarts to finish. Your mods and settings stay."),
            button(u, "settings-update-restart", "Restart to update", ActionKind::Primary)
                .on_press_tx(crate::updater::restart_now),
        ));
    }
    if let Some((done, total)) = up.progress() {
        let version = up.downloading.clone().unwrap_or_default();
        let frac = if total > 0 { (done as f32 / total as f32).clamp(0.0, 1.0) } else { 0.0 };
        let bar = col()
            .w_full()
            .gap(6.0)
            .pt(4.0)
            .child(
                row()
                    .w_full()
                    .h(4.0)
                    .radius(2.0)
                    .fill(u.well)
                    .child(el().w_frac(frac).h_full().radius(2.0).fill(u.muted)),
            )
            .child(
                text(fill(
                    tr("{} of {}"),
                    &[&crate::updater::megabytes(done), &crate::updater::megabytes(total)],
                ))
                .font(SMALL)
                .color(u.muted),
            );
        return Some(setting_row(
            u,
            "settings-update-progress",
            &fill(tr("Downloading Sigil {}"), &[&version]),
            bar,
            el(),
        ));
    }
    let offer = offer?;
    let downloadable = offer.install.as_ref().ok();
    if let Some(error) = &up.error {
        let mut actions = Vec::new();
        if downloadable.is_some() {
            actions.push(disabled(
                button(u, "settings-update-retry", "Try again", ActionKind::Secondary)
                    .on_press(crate::updater::start_download),
                s.prefs.offline,
            ));
        }
        return Some(setting_row(
            u,
            "settings-update-failed",
            "Update failed",
            text(error.clone()).font(SMALL).color(u.rose).wrap(),
            trailing(actions),
        ));
    }
    Some(match downloadable {
        Some(d) => setting_row(
            u,
            "settings-update-download",
            &fill(tr("Sigil {}"), &[&offer.version]),
            detail(u, &fill(tr("{} for this system."), &[&crate::updater::megabytes(d.entry.size)])),
            disabled(
                button(u, "settings-update-download-go", "Download", ActionKind::Secondary)
                    .on_press(crate::updater::start_download),
                s.prefs.offline,
            ),
        ),
        None => setting_row(
            u,
            "settings-update-manual",
            "Install",
            text(offer.install.as_ref().err().cloned().unwrap_or_default())
                .font(SMALL)
                .color(u.muted)
                .wrap(),
            el(),
        ),
    })
}

pub fn start_update_check(st: &mut State) {
    if st.settings.update_job.is_running() {
        return;
    }
    if rune::is_headless() {
        st.settings.update = Some(Err("headless run, no network".to_string()));
        return;
    }
    let wake = st.wake.clone();
    let preview = st.prefs.channel == prefs::Channel::Preview;
    st.settings
        .update_job
        .start(wake, move || check_latest(UPDATE_REPO, VERSION, preview));
}

/// The newest release on the chosen channel, with its signed manifest verified
/// (`crate::updater::check`).
fn check_latest(repo: &str, current: &str, preview: bool) -> Result<UpdateCheck, String> {
    let mut latest = crate::updater::check(
        &crate::updater::api_base(),
        repo,
        current,
        preview,
        crate::updater::TRUSTED_KEYS,
        crate::updater::platform_key(),
    )?;
    // A development build reads the release but says why it cannot install it.
    if let Some(release) = latest.as_mut()
        && release.install.is_ok()
        && let Err(reason) = crate::updater::current_layout()
    {
        release.install = Err(reason);
    }
    Ok(UpdateCheck {
        newer: latest.as_ref().is_some_and(|r| newer_than(&r.version, current)),
        latest,
    })
}

/// `a > b` as release versions; a pre-release sorts before its release.
pub fn newer_than(a: &str, b: &str) -> bool {
    crate::updater::is_newer(a, b)
}

// --- about -----------------------------------------------------------------------

const NOTICES: &str = include_str!("../../../../../THIRD_PARTY_NOTICES.md");

/// `(name, license, source)` for every project the notices list.
fn credits() -> Vec<(String, String, String)> {
    let mut out: Vec<(String, String, String)> = Vec::new();
    for line in NOTICES.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix("**").and_then(|l| l.strip_suffix("**")) {
            out.push((name.to_string(), String::new(), String::new()));
        } else if let Some(last) = out.last_mut() {
            if let Some(license) = line.strip_prefix("- License: ") {
                if last.1.is_empty() {
                    last.1 = license.split(',').next().unwrap_or(license).to_string();
                }
            } else if let Some(source) = line.strip_prefix("- Source: ")
                && last.2.is_empty()
            {
                last.2 = source.split_whitespace().next().unwrap_or("").to_string();
            }
        }
    }
    out.retain(|(_, license, _)| !license.is_empty());
    out
}

fn about(s: &State, u: &Ui) -> El {
    let mut rows = Vec::new();
    for (index, (name, license, source)) in credits().into_iter().enumerate() {
        let link = source.clone();
        let mut r = row()
            .key(format!("settings-credit-{index}"))
            .w_full()
            .h(44.0)
            .gap(16.0)
            .center_y()
            .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
            .child(
                text(name)
                    .font(BODY_MED)
                    .color(u.ink)
                    .grow(1.0)
                    .min_w(0.0)
                    .truncate(),
            )
            .child(text(capitalize(&license)).font(SMALL).color(u.muted).w(160.0).truncate());
        r = r.child(if source.starts_with("https://") {
            u.icon_button(&icons::EXTERNAL_LINK, tr("Open source page"))
                .key(format!("settings-credit-open-{index}"))
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                    if let Err(error) = tx.open_url(&link) {
                        say(st, format!("{error}"), true);
                    }
                })
        } else {
            el().size(32.0, 32.0)
        });
        rows.push(r);
    }
    col()
        .w_full()
        .gap(28.0)
        .child(
            col()
                .w_full()
                .gap(6.0)
                .child(text(format!("Sigil {VERSION}")).font(H1).color(u.ink))
                .child(
                    text(tr("A Deadlock mod manager with an agent inside. Free software under the MIT license."))
                        .font(BODY)
                        .color(u.muted)
                        .wrap(),
                )
                .child(
                    text(tr("Deadlock and its art belong to Valve. Sigil is not affiliated with or endorsed by Valve."))
                        .font(SMALL)
                        .color(u.faint)
                        .wrap(),
                ),
        )
        .child(section(u, "Updates", updates(s, u)))
        .child(section(u, "Memory", vec![memory_row(s, u)]))
        .child(section(u, "Open source", rows))
        .child(section(u, "People", vec![extra::contributors_row(s, u)]))
}

/// What the process holds now, so a report says where the memory is. Read when the tab draws.
fn memory_row(s: &State, u: &Ui) -> El {
    let m = crate::memory::read(s);
    let part = |label: &str, bytes: Option<u64>| {
        format!("{} {}", tr(label), bytes.map_or_else(|| "-".to_string(), crate::memory::mb))
    };
    let mut parts = Vec::new();
    if m.task_manager.is_some() {
        parts.push(part("Task Manager", m.task_manager));
    }
    parts.push(part("Committed", m.process));
    parts.push(part("App heap", m.heap));
    parts.push(format!("{} {} ({})", tr("Images"), crate::memory::mb(m.images.1), m.images.0));
    let line = parts.join(" · ");
    col()
        .key("settings-memory")
        .w_full()
        .gap(8.0)
        .py(12.0)
        .child(text(line).font(BODY).color(u.ink).wrap())
        .child(
            text(tr("Beyond the app heap is the graphics driver, the window and system libraries. Copy the report to send it with a bug report."))
                .font(SMALL)
                .color(u.muted)
                .wrap(),
        )
        .child(
            row().child(
                button(u, "settings-memory-copy", "Copy memory report", ActionKind::Secondary)
                    .on_press_tx(|st: &mut State, tx: &mut Tx| {
                        tx.copy(crate::memory::report(st));
                        say(st, tr("Memory report copied.").to_string(), false);
                    }),
            ),
        )
}

// --- confirms --------------------------------------------------------------------

fn confirm_dialog(s: &State, u: &Ui, ask: &Confirm) -> El {
    let (title, body, go, danger): (&str, String, &str, bool) = match ask {
        Confirm::RepairGameinfo => (
            "Repair gameinfo.gi",
            "Adds the citadel/addons line so the game loads your mods. Sigil keeps a copy of the file first.".into(),
            "Repair",
            false,
        ),
        Confirm::LaunchOptions(value) => (
            "Save launch options",
            if value.is_empty() {
                "Clears Deadlock's launch options in Steam. Steam must be closed. Sigil keeps a copy of localconfig.vdf first.".into()
            } else {
                fill(tr("Sets Deadlock's launch options in Steam to {}. Steam must be closed. Sigil keeps a copy of localconfig.vdf first."), &[value])
            },
            "Save",
            false,
        ),
        Confirm::Restore(path) => (
            "Restore this backup",
            format!(
                "Puts {} back in place. The current file is kept as a new backup.",
                backups::target_for(path)
                    .and_then(|t| t.file_name().map(|n| n.to_string_lossy().into_owned()))
                    .unwrap_or_default()
            ),
            "Restore",
            false,
        ),
        Confirm::Delete(path) => (
            "Delete this backup",
            format!(
                "{} is removed for good.",
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            ),
            "Delete",
            true,
        ),
        Confirm::Prune => (
            "Delete older copies",
            "Keeps the newest copy of each file and Sigil's first copies, and deletes the rest.".into(),
            "Delete",
            true,
        ),
        Confirm::Reset => (
            "Reset settings",
            tr("Theme, startup, launching, mods, privacy and the game folder go back to their defaults. Sign-ins and keys stay.").into(),
            "Reset",
            true,
        ),
        Confirm::CleanupArchives => (
            "Clean up the addons folder",
            fill(
                tr("Deletes {} archive files from the addons folder. Mods the game loads stay."),
                &[&s.settings.leftover.as_ref().map_or(0, Vec::len).to_string()],
            ),
            "Delete",
            true,
        ),
        Confirm::ForgetSharing => (
            "Forget shared matches",
            tr("Deletes the log of what Sigil sent and the list of matches already sent. deadlock-api keeps what it received.").into(),
            "Forget",
            true,
        ),
        Confirm::ClearLogs => (
            "Clear the log",
            tr("Deletes Sigil's log files.").into(),
            "Clear",
            true,
        ),
    };
    let title = tr(title);
    let ask = ask.clone();
    u.dialog_shell()
        .key("settings-confirm-dialog")
        .role("alertdialog")
        .label(title)
        .w(460.0)
        .padding(24.0)
        .gap(16.0)
        .modal(true, |st: &mut State| st.settings.confirm = None)
        .on_key(|st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                st.settings.confirm = None;
                true
            } else {
                false
            }
        })
        .child(text(title).font(H1).color(u.ink))
        .child(text(tr(&body).to_string()).font(BODY).color(u.muted).wrap().w_full())
        .child(
            row()
                .w_full()
                .gap(8.0)
                .pt(8.0)
                .child(spacer())
                .child(
                    button(u, "settings-confirm-cancel", "Cancel", ActionKind::Ghost)
                        .on_press(|st: &mut State| st.settings.confirm = None),
                )
                .child(
                    button(
                        u,
                        "settings-confirm-go",
                        go,
                        if danger {
                            ActionKind::Danger
                        } else {
                            ActionKind::Primary
                        },
                    )
                    .on_press(move |st: &mut State| run_confirm(st, ask.clone())),
                ),
        )
}

fn run_confirm(st: &mut State, ask: Confirm) {
    st.settings.confirm = None;
    match ask {
        Confirm::ForgetSharing => {
            sharing::forget(st);
            log::info("Match sharing log forgotten");
            return say(st, tr("Forgotten."), false);
        }
        Confirm::ClearLogs => {
            let result = log::clear();
            st.settings.log_size = Some(log::size());
            return match result {
                Ok(()) => say(st, tr("Cleared."), false),
                Err(error) => say(st, error, true),
            };
        }
        _ => {}
    }
    if st.demo {
        say(st, tr("Demo data: nothing was changed."), false);
        return;
    }
    match ask {
        Confirm::ForgetSharing | Confirm::ClearLogs => {}
        Confirm::CleanupArchives => {
            let Some(dir) = st.addons_dir.clone() else {
                return say(st, tr("Find the game folder first."), true);
            };
            let shown = st.settings.leftover.clone().unwrap_or_default();
            match deadlock_core::cleanup::remove_leftover_archives(Path::new(&dir), &shown, true) {
                Ok((removed, freed)) => {
                    log::info(format!("Removed {} leftover archives", removed.len()));
                    st.settings.leftover = Some(deadlock_core::cleanup::leftover_archives(Path::new(&dir)));
                    say(
                        st,
                        fill(tr("Deleted {} files, {} freed."), &[&removed.len().to_string(), &size_label(freed)]),
                        false,
                    );
                }
                Err(error) => say(st, error.to_string(), true),
            }
        }
        Confirm::RepairGameinfo => {
            let Some(root) = st.game_root.clone() else {
                return say(st, "Find the game folder first.", true);
            };
            let path = root.join("game").join("citadel").join("gameinfo.gi");
            match deadlock_core::deploy::repair_gameinfo(&path, true) {
                Ok(repair) => {
                    crate::data::rescan_game(st);
                    say(
                        st,
                        if repair.changed {
                            "Repaired. The game loads mods again."
                        } else {
                            "Already configured. Nothing changed."
                        },
                        false,
                    );
                }
                Err(error) => say(st, fill(tr("Repair failed: {}"), &[&error.to_string()]), true),
            }
        }
        Confirm::LaunchOptions(value) => {
            let Some(path) = st.settings.launch_read.as_ref().and_then(|r| r.path.clone()) else {
                return say(st, "No Steam settings file was found.", true);
            };
            match deadlock_core::steam_launch::write_file(&path, &value, true) {
                Ok(write) => {
                    read_launch_options(st);
                    say(
                        st,
                        if write.changed {
                            "Saved. Steam uses them the next time it starts the game."
                        } else {
                            "Steam already had these. Nothing changed."
                        },
                        false,
                    );
                }
                Err(error) => say(st, error.to_string(), true),
            }
        }
        Confirm::Restore(path) => match backups::restore(&path, true) {
            Ok(result) => {
                read_backups(st);
                if result
                    .target
                    .file_name()
                    .is_some_and(|n| n.eq_ignore_ascii_case("gameinfo.gi"))
                {
                    crate::data::rescan_game(st);
                }
                if result
                    .target
                    .file_name()
                    .is_some_and(|n| n.eq_ignore_ascii_case("autoexec.cfg"))
                {
                    st.autoexec.loaded = false;
                }
                say(st, fill(tr("Restored {}."), &[&result.target.display().to_string()]), false);
            }
            Err(error) => say(st, fill(tr("Restore failed: {}"), &[&error.to_string()]), true),
        },
        Confirm::Delete(path) => {
            match backups::delete(&path, Some(&crate::data::workspace_dir())) {
                Ok(()) => {
                    read_backups(st);
                    say(st, "Deleted.", false);
                }
                Err(error) => say(st, fill(tr("Delete failed: {}"), &[&error.to_string()]), true),
            }
        }
        Confirm::Prune => {
            let entries = st.settings.backups.clone().unwrap_or_default();
            match backups::prune(&entries, 1, Some(&crate::data::workspace_dir())) {
                Ok(count) => {
                    read_backups(st);
                    say(
                        st,
                        fill(tr("Deleted {} older copies."), &[&count.to_string()]),
                        false,
                    );
                }
                Err(error) => say(st, fill(tr("Delete failed: {}"), &[&error.to_string()]), true),
            }
        }
        Confirm::Reset => {
            let had_dir = st.prefs.game_dir.is_some();
            st.prefs = Prefs::default();
            i18n::set(st.prefs.language);
            log::set_level(st.prefs.log_level);
            apply_offline(false);
            let _ = deadlock_core::net::set_proxy(None);
            st.settings.proxy_edit = TextEdit::new("");
            apply_flags(st);
            let saved = st.prefs.save();
            prefs::apply_game_dir(&st.prefs);
            if had_dir {
                crate::data::rescan_game(st);
            }
            st.settings.accent_hex = TextEdit::new("");
            options::reset();
            downloads::apply();
            pages::apply();
            tray::sync();
            deadlock_core::net::set_proxy_auth(None);
            deadlock_core::net::set_proxy_bypass(Vec::new());
            match saved {
                Ok(()) => say(st, "Settings are back to their defaults.", false),
                Err(error) => say(st, fill(tr("Settings could not be saved: {}"), &[&error.to_string()]), true),
            }
        }
    }
}

// --- labels ----------------------------------------------------------------------

pub fn size_label(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KB {
        format!("{bytes} B")
    } else if b < KB * KB {
        format!("{:.0} KB", b / KB)
    } else if b < KB * KB * KB {
        format!("{:.1} MB", b / KB / KB)
    } else {
        format!("{:.1} GB", b / KB / KB / KB)
    }
}

fn age_label(at: SystemTime) -> String {
    let seconds = SystemTime::now()
        .duration_since(at)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    match seconds {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{} min ago", seconds / 60),
        3600..=86_399 => format!("{} h ago", seconds / 3600),
        _ => format!("{} d ago", seconds / 86_400),
    }
}

/// A path with the home folder written as `~`.
fn tidy(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_string(),
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

// --- scenes ----------------------------------------------------------------------

/// Open the sheet over a demo library on one category.
pub fn scene_tab(s: &mut State, tab: Tab) {
    s.load_demo();
    s.screen = Screen::Installed;
    s.settings.tab = tab;
    open(s);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(newer_than("0.10.0", "0.9.9"));
        assert!(newer_than("1.0.0", "0.99"));
        assert!(!newer_than("0.3.0", "0.3.0"));
        assert!(!newer_than("0.3.0-beta", "0.3.0"));
        assert!(!newer_than("0.2.9", "0.3"));
    }

    #[test]
    fn credits_come_from_the_notices() {
        let credits = credits();
        assert!(credits.iter().any(|(name, license, source)| name == "Rune"
            && license == "MIT"
            && source.starts_with("https://")));
        assert!(credits.iter().any(|(name, _, _)| name == "Grimoire"));
    }
}
