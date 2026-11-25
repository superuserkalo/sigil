//! Performance: community `gameinfo.gi` configs that trade graphics detail for
//! frame rate.
//!
//! The list groups configs by how much they cut, lighter first, under the config
//! that is on. A row opens the config's page: what it changes, the author's
//! visibility, camera and developer switches, the in-game graphics settings it
//! was tuned for, what Sigil leaves out and why, and where it comes from.
//!
//! Configs are fetched from their authors' GitHub repositories and cached, so the
//! page works offline after the first fetch. Every read, fetch and write runs on
//! a worker. Applying, turning off and writing video.txt are game writes behind a
//! confirm dialog here.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use deadlock_core::performance::{
    self, Applied, Catalog, Config, ConvarState, Group, Options, Tier, catalog,
    edits::{self, Edits},
    exchange, video,
};
use rune::{Color, El, Key, KeyEvent, ScrollSpec, TextEdit, col, el, icon, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::jobs::Job;
use crate::model::Screen;
use crate::screens::settings::i18n::fill;
use crate::screens::subpage;
use crate::ui::{
    ActionKind, BODY, BODY_MED, DISPLAY, H1, H2, LABEL, MONO, PAGE_TOP, PAGE_X, SMALL, SMALL_NUM, Ui,
};

mod de;

pub mod benchmark;
pub mod custom;
pub mod detail;
pub mod editor;
pub mod hosted;
mod scale;

/// This page's German table first, then Sigil's shared one.
pub fn tr(text: &str) -> &str {
    use crate::screens::settings::i18n::{self, Lang};
    if i18n::current() == Lang::German
        && let Some(de) = de::get(text)
    {
        return de;
    }
    i18n::tr(text)
}

/// How long a cached catalog is trusted before opening the page checks GitHub.
const STALE_HOURS: i64 = 12;

#[derive(Default)]
pub struct View {
    /// What the install's `gameinfo.gi` and video.txt have applied, once read.
    pub status: Option<Result<Status, String>>,
    pub loaded: bool,
    /// The cached configs, once loaded from disk.
    pub catalog: Option<Catalog>,
    pub fetch: Fetch,
    /// The tier filter. `None` shows every tier.
    pub tier: Option<Tier>,
    /// The stop the slider's thumb is being dragged to (`Some(None)` is "All"); the list
    /// filters on release.
    pub drag_tier: Option<Option<Tier>>,
    /// The config whose page is open.
    pub open: Option<String>,
    /// Switches the player changed on a config's page, not yet applied.
    pub options: HashMap<String, Options>,
    /// Show every ignored setting on a config's page, not the first few.
    pub show_all_ignored: bool,
    pub dialog: Option<Dialog>,
    /// One line after a write: the outcome, and whether it is an error.
    pub message: Option<(String, bool)>,
    pub read_job: Job<Status>,
    pub load_job: Job<Catalog>,
    pub fetch_job: Job<(Catalog, String)>,
    pub write_job: Job<String>,
    /// The player's saved convar edits, read once.
    pub edits: Edits,
    pub edits_loaded: bool,
    pub edits_job: Job<Edits>,
    pub save_job: Job<()>,
    /// The edits changed while a save was running; write them again after it.
    pub edits_dirty: bool,
    /// An exported settings file being written, and the line it leaves.
    pub export_job: Job<String>,
    /// A settings file being read for import, with its path.
    pub import_job: Job<(String, exchange::Exchange)>,
    /// A field for edits that outlive their dialog for a frame.
    pub scratch: TextEdit,
    /// The convar editor, when it is open.
    pub editor: Option<editor::Editor>,
    /// A GameBanana config's page, when it is open.
    pub hosted: Option<hosted::Hosted>,
    /// Performance > Benchmark.
    pub benchmark: benchmark::Page,
    /// Custom presets from GitHub and Apply as written (`custom.rs`).
    pub custom: custom::Custom,
}

/// What the game files say, read together so the page never shows half a state.
#[derive(Debug, Clone)]
pub struct Status {
    pub applied: Applied,
    /// What Sigil wrote to video.txt, if anything.
    pub video: Option<video::VideoRecord>,
    /// The video.txt the game reads, once the game has made one.
    pub video_path: Option<PathBuf>,
    /// How many written video values the file still holds.
    pub video_kept: usize,
    /// The ConVars values `gameinfo.gi` holds now, for the editor.
    pub values: BTreeMap<String, String>,
}

#[derive(Default, Clone, PartialEq)]
pub enum Fetch {
    #[default]
    Idle,
    Running,
    /// The last check failed: the page shows what the cache has.
    Failed(String),
}

#[derive(Clone)]
pub enum Dialog {
    Apply(String),
    TurnOff,
    WriteVideo(String),
    KeepVideo,
    RestoreVideo,
}

/// Open the install's gameinfo.gi in the editor chosen in Settings.
fn open_gameinfo_in_editor(st: &mut State) {
    let Some(root) = st.game_root.clone() else {
        st.performance.message = Some((tr("Set the game folder in Settings first.").to_string(), true));
        return;
    };
    let path = performance::gameinfo_path(&root);
    if !path.is_file() {
        st.performance.message = Some((
            tr("gameinfo.gi could not be read").to_string(),
            true,
        ));
        return;
    }
    crate::screens::settings::open_in_editor(st, &path);
}

fn data_dir() -> PathBuf {
    crate::data::data_dir()
}

fn simulated(st: &State) -> bool {
    st.demo || rune::is_headless()
}

/// Read the game files, load the cache, and check GitHub when the cache is old.
pub fn start_read(st: &mut State) {
    if st.performance.catalog.is_none() && !simulated(st) {
        let wake = st.wake.clone();
        st.performance
            .load_job
            .start(wake, || Ok(performance::load(&data_dir())));
    }
    if !st.performance.edits_loaded && !st.performance.edits_job.is_running() {
        let wake = st.wake.clone();
        st.performance.edits_job.start(wake, || {
            Ok(edits::load(&crate::data::data_dir()))
        });
    }
    let Some(root) = st.game_root.clone() else {
        st.performance.loaded = true;
        st.performance.status = None;
        return;
    };
    if simulated(st) {
        st.performance.loaded = true;
        return;
    }
    let wake = st.wake.clone();
    st.performance.read_job.start(wake, move || {
        let path = performance::gameinfo_path(&root);
        let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let applied = performance::applied(&text);
        let values = performance::convar_values(&text);
        let data = data_dir();
        let video = video::read_record(&data);
        let video_path = video::locate(&root, &deadlock_core::discovery::default_steam_roots());
        let video_kept = video
            .as_ref()
            .and_then(|record| {
                std::fs::read_to_string(&record.path)
                    .ok()
                    .map(|text| video::still_written(&text, record))
            })
            .unwrap_or(0);
        Ok(Status {
            applied,
            video,
            video_path,
            video_kept,
            values,
        })
    });
}

/// Check GitHub for newer versions, then reload the cache.
pub fn start_fetch(st: &mut State) {
    if simulated(st) || st.performance.fetch_job.is_running() {
        return;
    }
    st.performance.fetch = Fetch::Running;
    let wake = st.wake.clone();
    st.performance.fetch_job.start(wake, || {
        let data = data_dir();
        let report = performance::refresh(&data);
        let catalog = performance::load(&data);
        match report {
            Ok(report) => {
                let line = if report.updated.is_empty() {
                    String::new()
                } else {
                    fill(tr("Updated {}."), &[&list_names(&report.updated)])
                };
                Ok((catalog, line))
            }
            Err(error) if !catalog.is_empty() => Ok((catalog, format!("!{error}"))),
            Err(error) => Err(error.to_string()),
        }
    });
}

fn list_names(names: &[String]) -> String {
    match names.len() {
        0 => String::new(),
        1 => names[0].clone(),
        2 => format!("{} and {}", names[0], names[1]),
        n => format!("{}, {} and {} more", names[0], names[1], n - 2),
    }
}

/// The switches for a config: the player's edits, else what the file records
/// for the config that is on, else the author's defaults.
pub fn options_for(s: &State, config: &Config) -> Options {
    let mut options = match s.performance.options.get(config.id()) {
        Some(options) => options.clone(),
        None => applied_options(s, config).unwrap_or_else(|| config.default_options()),
    };
    options.edits = edits_for(s, config.id()).values.clone();
    options
}

/// The player's edits over one config.
pub fn edits_for<'a>(s: &'a State, id: &str) -> &'a performance::edits::ConfigEdits {
    s.performance.edits.for_config(id)
}

fn applied_options(s: &State, config: &Config) -> Option<Options> {
    match applied(s) {
        Some(Applied::Config {
            id,
            switches: Some((on, engine)),
            ..
        }) if id == config.id() => Some(Options {
            opt_ins: on
                .iter()
                .filter(|key| config.switchable().any(|opt| &opt.key == *key))
                .cloned()
                .collect(),
            engine: *engine,
            video: config.default_options().video,
            edits: BTreeMap::new(),
        }),
        _ => None,
    }
}

fn applied(s: &State) -> Option<&Applied> {
    match &s.performance.status {
        Some(Ok(status)) => Some(&status.applied),
        _ => None,
    }
}

fn applied_id(s: &State) -> Option<String> {
    match applied(s) {
        Some(Applied::Config { id, .. }) => Some(id.clone()),
        _ => None,
    }
}

fn status(s: &State) -> Option<&Status> {
    match &s.performance.status {
        Some(Ok(status)) => Some(status),
        _ => None,
    }
}

fn config<'a>(s: &'a State, id: &str) -> Option<&'a Config> {
    s.performance.catalog.as_ref()?.get(id)
}

/// The switches on the config's page differ from what is in the file.
fn pending(s: &State, config: &Config) -> bool {
    let current = options_for(s, config);
    let switches = match applied_options(s, config) {
        Some(on) => on.opt_ins != current.opt_ins || on.engine != current.engine,
        None => applied_id(s).as_deref() == Some(config.id()),
    };
    switches || edits_pending(s, config)
}

/// An edit the file does not hold: a new value, or one reset after an apply.
fn edits_pending(s: &State, config: &Config) -> bool {
    let edits = edits_for(s, config.id());
    if edits.is_empty() {
        return false;
    }
    if applied_id(s).as_deref() != Some(config.id()) {
        return true;
    }
    let Some(values) = status(s).map(|status| &status.values) else {
        return true;
    };
    let options = options_for(s, config);
    let keys: std::collections::BTreeSet<&str> = edits
        .values
        .keys()
        .map(String::as_str)
        .chain(edits.history.iter().map(|entry| entry.key.as_str()))
        .collect();
    keys.into_iter().any(|key| {
        let author = author_value(config, &options, key);
        let wanted = options
            .edits
            .get(key)
            .map(String::as_str)
            .or(author.as_deref());
        values.get(key).map(String::as_str) != wanted
    })
}

/// What the config writes for a key with no edit.
fn author_value(config: &Config, options: &Options, key: &str) -> Option<String> {
    if let Some((_, value)) = config.body.convars.iter().find(|(k, _)| k == key) {
        return Some(value.clone());
    }
    config
        .switchable()
        .find(|opt| opt.key == key)
        .filter(|opt| options.opt_ins.contains(&opt.key))
        .map(|opt| opt.value.clone())
}

/// A newer cached version than the one in the file.
fn update_for(s: &State) -> Option<&Config> {
    let Some(Applied::Config {
        id,
        version,
        commit,
        ..
    }) = applied(s)
    else {
        return None;
    };
    let config = config(s, id)?;
    let same_commit = !commit.is_empty() && config.commit.starts_with(commit.as_str());
    (config.version != *version && !same_commit).then_some(config)
}

fn start_apply(st: &mut State, id: String) {
    st.performance.dialog = None;
    let Some(config) = config(st, &id).cloned() else {
        return;
    };
    let options = options_for(st, &config);
    st.performance.options.remove(&id);
    crate::screens::settings::product::perf_config(st, config.id(), &config.version);
    let Some(root) = st.game_root.clone() else {
        return;
    };
    if simulated(st) {
        let commit = config.commit[..12.min(config.commit.len())].to_string();
        set_applied(
            st,
            Applied::Config {
                id: config.id().to_string(),
                version: config.version.clone(),
                commit,
                switches: Some((options.opt_ins.clone(), options.engine)),
            },
        );
        if let Some(Ok(status)) = &mut st.performance.status {
            status.values = exchange::effective(&config, &options).into_iter().collect();
        }
        st.performance.message = Some((applied_line(config.name(), &[]), false));
        return;
    }
    let wake = st.wake.clone();
    st.performance.write_job.start(wake, move || {
        let patched =
            performance::apply_to_install(&root, &config, &options).map_err(|e| e.to_string())?;
        Ok(applied_line(config.name(), &patched.skipped))
    });
}

fn applied_line(name: &str, skipped: &[String]) -> String {
    if skipped.is_empty() {
        format!("{name} is on. It takes effect the next time you start the game.")
    } else {
        format!(
            "{name} is on. {} engine edit{} skipped because the game no longer has that section.",
            skipped.len(),
            if skipped.len() == 1 { " was" } else { "s were" }
        )
    }
}

fn set_applied(st: &mut State, applied: Applied) {
    match &mut st.performance.status {
        Some(Ok(status)) => status.applied = applied,
        other => {
            *other = Some(Ok(Status {
                applied,
                video: None,
                video_path: None,
                video_kept: 0,
                values: BTreeMap::new(),
            }))
        }
    }
}

fn start_turn_off(st: &mut State) {
    st.performance.dialog = None;
    let Some(root) = st.game_root.clone() else {
        return;
    };
    if simulated(st) {
        set_applied(st, Applied::None);
        if let Some(Ok(status)) = &mut st.performance.status {
            status.video = None;
        }
        st.performance.message = Some((tr("Stock settings are back.").into(), false));
        return;
    }
    let wake = st.wake.clone();
    st.performance.write_job.start(wake, move || {
        performance::remove_from_install(&root).map_err(|e| e.to_string())?;
        let video = video::revert_install(&data_dir()).map_err(|e| e.to_string())?;
        Ok(if video {
            tr("Stock settings are back, in gameinfo.gi and video.txt.").to_string()
        } else {
            tr("Stock settings are back.").to_string()
        })
    });
}

fn start_write_video(st: &mut State, id: String) {
    st.performance.dialog = None;
    let Some(config) = config(st, &id).cloned() else {
        return;
    };
    let options = options_for(st, &config);
    let Some(set) = config.video(options.video.as_deref()).cloned() else {
        return;
    };
    let Some(path) = status(st).and_then(|status| status.video_path.clone()) else {
        return;
    };
    if simulated(st) {
        if let Some(Ok(status)) = &mut st.performance.status {
            status.video_kept = set.settings.len();
            status.video = Some(video::VideoRecord {
                config: id,
                file: set.file.clone(),
                path,
                original: Default::default(),
                written: set.settings.iter().cloned().collect(),
            });
        }
        st.performance.message = Some((
            tr("Video settings written. Your old video.txt is backed up.").into(),
            false,
        ));
        return;
    }
    let wake = st.wake.clone();
    st.performance.write_job.start(wake, move || {
        let count = video::apply_install(&data_dir(), &path, &id, &set.file, &set.settings)
            .map_err(|e| e.to_string())?;
        Ok(format!(
            "{count} video setting{} written. Your old video.txt is backed up next to it.",
            if count == 1 { "" } else { "s" }
        ))
    });
}

fn start_keep_video(st: &mut State) {
    st.performance.dialog = None;
    if simulated(st) {
        if let Some(Ok(status)) = &mut st.performance.status {
            status.video = None;
        }
        st.performance.message = Some((tr("Kept. Sigil will not change these values back.").into(), false));
        return;
    }
    let wake = st.wake.clone();
    st.performance.write_job.start(wake, || {
        video::keep_install(&data_dir())
            .map(|_| tr("Kept. Sigil will not change these values back.").to_string())
            .map_err(|error| error.to_string())
    });
}

fn start_restore_video(st: &mut State) {
    st.performance.dialog = None;
    if simulated(st) {
        if let Some(Ok(status)) = &mut st.performance.status {
            status.video = None;
        }
        st.performance.message = Some((tr("Your video settings are back.").into(), false));
        return;
    }
    let wake = st.wake.clone();
    st.performance.write_job.start(wake, || {
        video::revert_install(&data_dir())
            .map(|_| tr("Your video settings are back.").to_string())
            .map_err(|e| e.to_string())
    });
}

/// Take finished jobs. Returns true when the screen changed.
pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.performance.edits_job.try_take() {
        st.performance.edits = result.unwrap_or_default();
        st.performance.edits_loaded = true;
        changed = true;
    }
    if let Some(result) = st.performance.save_job.try_take() {
        if let Err(error) = result {
            st.performance.message = Some((error, true));
        }
        if st.performance.edits_dirty {
            editor::start_save(st);
        }
        changed = true;
    }
    if let Some(result) = st.performance.load_job.try_take() {
        if let Ok(catalog) = result {
            let stale = performance::needs_refresh(&data_dir(), STALE_HOURS);
            st.performance.catalog = Some(catalog);
            if stale {
                start_fetch(st);
            }
        }
        changed = true;
    }
    if let Some(result) = st.performance.fetch_job.try_take() {
        match result {
            Ok((catalog, line)) => {
                st.performance.catalog = Some(catalog);
                st.performance.fetch = match line.strip_prefix('!') {
                    Some(error) => Fetch::Failed(error.to_string()),
                    None => Fetch::Idle,
                };
            }
            Err(error) => st.performance.fetch = Fetch::Failed(error),
        }
        crate::screens::autoexec_convars::reload_if_empty(st);
        changed = true;
    }
    if let Some(result) = st.performance.read_job.try_take() {
        st.performance.status = Some(result);
        st.performance.loaded = true;
        changed = true;
    }
    if let Some(result) = st.performance.write_job.try_take() {
        st.performance.message = Some(match result {
            Ok(line) => (line, false),
            Err(error) => (error, true),
        });
        start_read(st);
        changed = true;
    }
    changed |= editor::poll(st);
    changed |= hosted::poll(st);
    changed |= benchmark::poll(st);
    changed |= custom::poll(st);
    changed
}

pub fn view(s: &State, u: &Ui) -> El {
    if s.performance.benchmark.open {
        return scroll("perf-bench", benchmark::view(s, u));
    }
    if s.performance.hosted.is_some() {
        return scroll("perf-hosted", hosted::view(s, u));
    }
    if s.performance.editor.is_some() {
        return scroll("perf-editor", editor::view(s, u));
    }
    if let Some(id) = s.performance.open.clone()
        && let Some(config) = config(s, &id)
    {
        return scroll("perf-detail", detail::page(s, u, config));
    }
    scroll("perf-list", list_page(s, u))
}

fn scroll(key: &str, page: El) -> El {
    crate::ui::Ui::new().scroll_area(col()
        .key(format!("{key}-scroll"))
        .w_full()
        .h_full()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(page))
}

fn page(key: &str) -> El {
    col()
        .key(key.to_string())
        .w_full()
        .px(PAGE_X)
        .pt(PAGE_TOP)
        .pb(48.0)
        .gap(24.0)
}

// --- list -------------------------------------------------------------------

fn list_page(s: &State, u: &Ui) -> El {
    let fetching = s.performance.fetch == Fetch::Running;
    let refresh = u
        .icon_button(&icons::REFRESH_CW, tr("Check GitHub for new versions"))
        .key("perf-refresh")
        .opacity(if fetching { 0.4 } else { 1.0 })
        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| {
            start_fetch(st);
            start_read(st);
        });
    let mut actions = row().gap(8.0).center_y();
    if s.game_detected && s.game_root.is_some() {
        actions = actions.child(
            u.icon_action(&icons::GAUGE, tr("Benchmark"), ActionKind::Secondary)
                .key("perf-benchmark")
                .cursor(rune::Cursor::Pointer)
                .on_press(benchmark::open),
        );
        actions = actions.child(
            u.icon_action(&icons::SQUARE_PEN, tr("Edit in editor"), ActionKind::Secondary)
                .key("perf-edit-external")
                .cursor(rune::Cursor::Pointer)
                .on_press(open_gameinfo_in_editor),
        );
    }
    actions = actions.child(custom::add_button(u)).child(refresh);
    let mut page = page("perf-page").child(u.page_head(
        "",
        tr("Performance"),
        "",
        Some(actions),
    ));

    if !s.game_detected {
        return page.child(u.empty_state(
            "",
            tr("Deadlock was not found"),
            tr("Performance configs are written into the game's gameinfo.gi. Set the game folder in Settings."),
            Some(
                u.action(tr("Open Settings"), ActionKind::Secondary)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| st.go(Screen::Settings)),
            ),
        ));
    }

    page = page.child(current(s, u));
    let Some(catalog) = s.performance.catalog.as_ref().filter(|c| !c.is_empty()) else {
        return page.child(first_run(s, u));
    };
    if let Some(hide) = custom::hide_choice(s, u) {
        page = page.child(hide);
    }
    page.child(toolbar(s, u, catalog))
        .child(groups(s, u, catalog))
        .child(hosted_section(s, u))
        .child(
            text(tr("GitHub configs come from OptimizationLock and OptiLock, shared by their authors under GPL-3.0. GameBanana configs stay on their pages."))
                .font(SMALL)
                .color(u.muted)
                .wrap(),
        )
}

/// No cached configs yet: fetching, or the first fetch failed.
fn first_run(s: &State, u: &Ui) -> El {
    match &s.performance.fetch {
        Fetch::Failed(error) => u.empty_state(
            "",
            tr("Could not reach GitHub"),
            &if error.contains("rate limit") {
                error.clone()
            } else {
                tr("Sigil downloads the configs from their authors once, then works offline. Check your connection and try again.").to_string()
            },
            Some(
                u.action(tr("Try again"), ActionKind::Secondary)
                    .key("perf-retry")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| start_fetch(st)),
            ),
        ),
        _ => u.empty_state(
            "",
            tr("Getting configs from GitHub"),
            tr("This happens once. After that the page works offline."),
            None,
        ),
    }
}

/// The config that is on, or stock. The state the whole page is about.
fn current(s: &State, u: &Ui) -> El {
    let busy = s.performance.write_job.is_running();
    let mut actions: Vec<El> = Vec::new();
    let (title, detail, dot): (String, String, Color) = match &s.performance.status {
        None if !s.performance.loaded => (
            tr("Reading gameinfo.gi").into(),
            String::new(),
            Color::WHITE.alpha(0.22),
        ),
        None => (
            tr("Stock settings").into(),
            String::new(),
            Color::WHITE.alpha(0.22),
        ),
        Some(Err(error)) => (
            tr("gameinfo.gi could not be read").into(),
            error.clone(),
            u.rose,
        ),
        Some(Ok(status)) => match &status.applied {
            Applied::Foreign => (
                tr("Grimoire has a config on").into(),
                tr("Turn it off in Grimoire before picking one here.").into(),
                u.amber,
            ),
            Applied::None => (
                tr("Stock settings").into(),
                if s.performance
                    .catalog
                    .as_ref()
                    .is_some_and(|c| !c.is_empty())
                {
                    tr("The game runs as Valve ships it. Pick a config below to raise your frame rate.").into()
                } else {
                    tr("The game runs as Valve ships it.").into()
                },
                Color::WHITE.alpha(0.22),
            ),
            Applied::Config {
                id, switches, ..
            } => {
                let found = config(s, id);
                let def = catalog::config(id);
                let name = def
                    .map(|d| d.name.to_string())
                    .unwrap_or_else(|| id.clone());
                let mut parts = Vec::new();
                if let Some(def) = def {
                    parts.push(tr(def.tier.label()).to_string());
                }
                parts.retain(|part| !part.is_empty());
                if let Some(config) = found {
                    parts.push(display_version(config));
                    parts.retain(|part| !part.is_empty());
                    let options =
                        applied_options(s, config).unwrap_or_else(|| config.default_options());
                    parts.push(fill(tr("{} settings"), &[&config.setting_count(&options).to_string()]));
                    let on = switches.as_ref().map(|(on, _)| on.len()).unwrap_or(0);
                    if on > 0 {
                        parts.push(fill(
                            if on == 1 { tr("{} author switch on") } else { tr("{} author switches on") },
                            &[&on.to_string()],
                        ));
                    }
                }
                if status
                    .video
                    .as_ref()
                    .is_some_and(|record| &record.config == id)
                {
                    parts.push("video settings written".into());
                }
                let open_id = id.clone();
                actions.push(
                    u.action(tr("Details"), ActionKind::Ghost)
                        .key("perf-current-details")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| {
                            st.performance.open = Some(open_id.clone())
                        }),
                );
                if let Some(newer) = update_for(s) {
                    parts.push(format!("{} is out", display_version(newer)));
                    let update_id = newer.id().to_string();
                    actions.push(
                        u.action(tr("Update"), ActionKind::Primary)
                            .key("perf-update")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(move |st: &mut State| {
                                if !st.performance.write_job.is_running() {
                                    st.performance.dialog = Some(Dialog::Apply(update_id.clone()));
                                }
                            }),
                    );
                }
                actions.push(
                    u.icon_action(&icons::POWER, tr("Turn off"), ActionKind::Secondary)
                        .key("perf-turn-off")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| {
                            if !st.performance.write_job.is_running() {
                                st.performance.dialog = Some(Dialog::TurnOff);
                            }
                        }),
                );
                (
                    fill(tr("{} is on"), &[&name]),
                    {
                        parts.push(tr("starts with the next launch").to_string());
                        parts.join(" · ")
                    },
                    crate::ui::switch_green(),
                )
            }
        },
    };
    // Stock reads as an empty square; on, foreign and errors fill it with their colour.
    let stock = dot == Color::WHITE.alpha(0.22);
    let mark = el()
        .size(8.0, 8.0)
        .radius(1.0)
        .fill(if stock { Color::TRANSPARENT } else { dot })
        .border(1.0, if stock { u.muted } else { dot })
        .a11y_hidden();
    let text_col = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(0.0)
        .child(
            row()
                .h(20.0)
                .gap(10.0)
                .center_y()
                .child(mark)
                .child(text(title).font(BODY_MED).color(u.ink).truncate()),
        )
        .child(
            row()
                .h(16.0)
                .pl(18.0)
                .child(text(detail).font(SMALL).color(u.muted).truncate()),
        );
    // The action slot keeps its width when empty or busy, so the text never reflows.
    let mut slot = row().min_w(216.0).h(32.0).gap(8.0).center_y().justify_end();
    if busy {
        slot = slot.child(text(tr("Writing")).font(SMALL).color(u.muted));
    } else {
        for action in actions {
            slot = slot.child(action);
        }
    }
    let bar = row()
        .key("perf-current")
        .w_full()
        .h(56.0)
        .px(12.0)
        .gap(12.0)
        .center_y()
        .radius(crate::ui::r(4.0))
        .fill(u.raised)
        .border(1.0, u.hair)
        .child(text_col)
        .child(slot);
    let mut out = col().w_full().gap(8.0).child(bar);
    if let Some((line, error)) = &s.performance.message {
        out = out.child(
            text(line.clone())
                .font(SMALL)
                .color(if *error { u.rose } else { u.muted })
                .wrap(),
        );
    }
    out
}

/// A config's version as a player reads it. GameBanana configs keep the file
/// name, which says what the download is.
fn display_version(config: &Config) -> String {
    // A mod's own gameinfo keeps the mod's name, like a GameBanana file name.
    if config.def.gamebanana.is_some() || config.source.id == "mod" {
        if config.version.is_empty() {
            String::new()
        } else {
            config.version.clone()
        }
    } else {
        version_label(&config.version)
    }
}

/// `v3.9`, `v6.1d`; empty for a config that states no version (its cache
/// knows it by commit, which says nothing to a player: the date does).
fn version_label(version: &str) -> String {
    let hash = version.len() == 7 && version.bytes().all(|b| b.is_ascii_hexdigit());
    if hash || version.is_empty() {
        String::new()
    } else if version.starts_with('v') {
        version.to_string()
    } else {
        format!("v{version}")
    }
}

/// Non-empty parts joined with a middle dot.
fn dots(parts: &[String]) -> String {
    parts
        .iter()
        .filter(|part| !part.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The looks to frames scale, in place of the old tier filter.
fn toolbar(s: &State, u: &Ui, catalog: &Catalog) -> El {
    let on = applied_id(s)
        .and_then(|id| catalog.configs.iter().find(|c| c.id() == id))
        .map(|c| c.def.tier);
    // How fresh the configs are, under the slider's readout.
    let fresh = freshness_line(s, catalog).0;
    scale::view(s, u, catalog, on, fresh)
}

fn freshness_line(s: &State, catalog: &Catalog) -> (String, bool) {
    let checked = catalog
        .checked_at
        .as_deref()
        .and_then(performance::minutes_since)
        .map(ago);
    match (&s.performance.fetch, checked) {
        (Fetch::Running, _) => (tr("Checking GitHub for new versions").to_string(), false),
        (Fetch::Failed(_), Some(ago)) => (fill(tr("Offline. Showing configs saved {}"), &[&ago]), true),
        (Fetch::Failed(_), None) => (tr("Offline. Showing saved configs").to_string(), true),
        (Fetch::Idle, Some(ago)) => (fill(tr("Checked GitHub {}"), &[&ago]), false),
        (Fetch::Idle, None) => (String::new(), false),
    }
}

fn ago(minutes: i64) -> String {
    match minutes {
        m if m < 2 => tr("just now").into(),
        m if m < 60 => fill(tr("{} minutes ago"), &[&m.to_string()]),
        m if m < 120 => tr("an hour ago").into(),
        m if m < 60 * 48 => fill(tr("{} hours ago"), &[&(m / 60).to_string()]),
        m => fill(tr("{} days ago"), &[&(m / (60 * 24)).to_string()]),
    }
}

/// One group per tier, lighter first: the list is the scale.
fn groups(s: &State, u: &Ui, catalog: &Catalog) -> El {
    let on = applied_id(s);
    let mut out = col().w_full().gap(16.0);
    for tier in Tier::ALL {
        if s.performance.tier.is_some_and(|wanted| wanted != tier) {
            continue;
        }
        let configs: Vec<&Config> = catalog
            .configs
            .iter()
            .filter(|c| c.def.tier == tier)
            .filter(|c| !custom::hides_builtins(s) || custom::is_custom(c.id()))
            .collect();
        if configs.is_empty() {
            continue;
        }
        let count = configs.len();
        let mut rows = col().w_full();
        for config in configs {
            rows = rows.child(config_row(s, u, config, on.as_deref() == Some(config.id())));
        }
        out = out.child(
            col()
                .w_full()
                .child(
                    row()
                        .w_full()
                        .h(32.0)
                        .gap(12.0)
                        .center_y()
                        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                        .child(
                            row()
                                .gap(6.0)
                                .center_y()
                                .child(text(tr(tier.label())).font(BODY_MED).color(u.ink))
                                .child(text(count.to_string()).font(SMALL_NUM).color(u.muted)),
                        )
                        .child(text(tr(tier.blurb())).font(SMALL).color(u.muted).truncate()),
                )
                .child(rows),
        );
    }
    out
}

fn config_row(s: &State, u: &Ui, config: &Config, is_on: bool) -> El {
    let id = config.id().to_string();
    let mut name = row()
        .gap(8.0)
        .center_y()
        .child(text(config.name()).font(BODY_MED).color(u.ink).truncate());
    if config.id() == catalog::DEFAULT_ID {
        name = name.child(chip(u, tr("Recommended")));
    }
    if config.def.testing {
        name = name.child(chip(u, tr("Testing")));
    }
    let meta = dots(&[
        config.def.author.to_string(),
        display_version(config),
        short_date(&config.date),
    ]);
    let options = options_for(s, config);
    // The state slot is reserved on every row, so "On" never pushes the columns.
    let mut state = row().w(40.0).gap(6.0).center_y();
    if is_on {
        state = state
            .child(el().size(8.0, 8.0).radius(1.0).fill(crate::ui::switch_green()).a11y_hidden())
            .child(text(tr("On")).font(LABEL).color(u.ink));
    }
    let chevron = row().w(24.0).center().child(
        icon(&icons::CHEVRON_RIGHT, 14.0)
            .color(u.muted)
            .a11y_hidden(),
    );
    let error = s
        .performance
        .catalog
        .as_ref()
        .and_then(|c| c.errors.get(config.id()))
        .is_some();
    row()
        .key(format!("perf-row-{id}"))
        .w_full()
        .h(48.0)
        .gap(16.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .hover(u.hover)
        .focusable(true)
        .role("button")
        .label(format!("{}, {}", config.name(), config.def.tier.label()))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            st.performance.open = Some(id.clone());
            st.performance.show_all_ignored = false;
        })
        .child(
            col()
                .w(240.0)
                .gap(0.0)
                .child(name.h(20.0))
                .child(text(meta).font(SMALL).color(u.muted).truncate()),
        )
        .child(
            text(if error {
                tr("Could not update from GitHub. Showing the last saved version.")
            } else {
                config.def.note
            })
            .font(BODY)
            // Quieter than the name, so the name leads the row.
            .color(if error { u.amber } else { u.muted })
            .grow(1.0)
            .min_w(0.0)
            .truncate(),
        )
        .child(
            text(fill(tr("{} settings"), &[&config.setting_count(&options).to_string()]))
                .font(SMALL_NUM)
                .color(u.muted)
                .w(96.0)
                .align(rune::Align::End),
        )
        .child(state)
        .child(chevron)
}

/// A small squared tag beside a name: "Recommended", "Testing".
fn chip(u: &Ui, label: &str) -> El {
    row()
        .h(20.0)
        .px(6.0)
        .center()
        .radius(2.0)
        .fill(u.over.alpha(0.08))
        .child(text(label.to_string()).font(crate::ui::MICRO).color(u.muted))
}

fn short_date(date: &str) -> String {
    let mut parts = date.split('-');
    let (Some(year), Some(month), Some(day)) = (parts.next(), parts.next(), parts.next()) else {
        return date.to_string();
    };
    let month = match month {
        "01" => "Jan",
        "02" => "Feb",
        "03" => "Mar",
        "04" => "Apr",
        "05" => "May",
        "06" => "Jun",
        "07" => "Jul",
        "08" => "Aug",
        "09" => "Sep",
        "10" => "Oct",
        "11" => "Nov",
        "12" => "Dec",
        _ => return date.to_string(),
    };
    format!("{month} {}, {year}", day.trim_start_matches('0'))
}

/// The button that opens the convar editor.
fn edit_button(u: &Ui, config: &Config) -> El {
    let id = config.id().to_string();
    u.icon_action(
        &icons::SLIDERS_HORIZONTAL,
        tr("Edit settings"),
        ActionKind::Secondary,
    )
    .key("perf-edit-settings")
    .cursor(rune::Cursor::Pointer)
    .on_press(move |st: &mut State| editor::open(st, &id))
}

/// GameBanana configs, downloaded or not, below the GitHub tiers.
fn hosted_section(s: &State, u: &Ui) -> El {
    let hosted: Vec<&deadlock_core::performance::ConfigDef> = catalog::CONFIGS
        .iter()
        .filter(|def| def.gamebanana.is_some())
        .collect();
    if hosted.is_empty() {
        return el();
    }
    let on = applied_id(s);
    let mut rows = col().w_full();
    for def in hosted {
        let cached = config(s, def.id).is_some();
        let is_on = on.as_deref() == Some(def.id);
        let id = def.id.to_string();
        let state = if is_on {
            row()
                .w(88.0)
                .gap(6.0)
                .center_y()
                .child(icon(&icons::CHECK, 14.0).color(u.accent).a11y_hidden())
                .child(text(tr("On")).font(BODY_MED).color(u.ink))
        } else {
            row()
                .w(88.0)
                .center_y()
                .child(
                    text(if cached { tr("Downloaded") } else { tr("GameBanana") })
                        .font(SMALL)
                        .color(u.muted),
                )
        };
        rows = rows.child(
            row()
                .key(format!("perf-hosted-row-{id}"))
                .w_full()
                // The same columns and height as the GitHub rows above, so both lists share
                // one name column and one description edge.
                .h(48.0)
                .gap(16.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .hover(u.hover)
                .focusable(true)
                .role("button")
                .label(format!("{}, {}", def.name, def.tier.label()))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| hosted::open(st, &id))
                .child(
                    col()
                        .w(240.0)
                        .gap(0.0)
                        .child(text(def.name).font(BODY_MED).color(u.ink).truncate())
                        .child(
                            text(format!("{} · {}", def.author, def.tier.label()))
                                .font(SMALL)
                                .color(u.muted)
                                .truncate(),
                        ),
                )
                .child(
                    text(def.note)
                        .font(BODY)
                        .color(u.muted)
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate(),
                )
                .child(state),
        );
    }
    col()
        .w_full()
        .child(
            col()
                .w_full()
                .gap(2.0)
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .pb(8.0)
                .child(text(tr("From GameBanana")).font(BODY_MED).color(u.ink))
                .child(
                    text(tr("Configs whose authors publish on GameBanana. Sigil downloads the file from its page and lets you pick the right one."))
                        .font(SMALL)
                        .color(u.muted),
                ),
        )
        .child(rows)
}

// --- detail -----------------------------------------------------------------

fn open_url(url: &str) {
    if rune::is_headless() {
        return;
    }
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let _ = deadlock_core::util::hidden_command("cmd")
        .args(["/C", "start", "", url])
        .spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

// --- dialogs ------------------------------------------------------------------

pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    if let Some(shell) = editor::dialog(s, u) {
        return Some(shell);
    }
    if let Some(shell) = custom::dialog(s, u) {
        return Some(shell);
    }
    let dialog = s.performance.dialog.clone()?;
    let (title, lines, confirm): (String, Vec<String>, El) = match &dialog {
        Dialog::Apply(id) => {
            let config = config(s, id)?;
            let options = options_for(s, config);
            let current = applied_id(s);
            let mut lines = vec![fill(
                tr("Writes {} settings to gameinfo.gi. A backup is kept, and Turn off puts every stock value back."),
                &[&config.setting_count(&options).to_string()],
            )];
            let off = config.switchable().count()
                - config
                    .switchable()
                    .filter(|opt| options.opt_ins.contains(&opt.key))
                    .count();
            if off > 0 {
                lines.push(format!(
                    "{off} author switch{} off, as set on the config's page.",
                    if off == 1 { " stays" } else { "es stay" }
                ));
            }
            let replaces = current
                .filter(|current| current != id)
                .and_then(|current| catalog::config(&current))
                .map(|other| fill(tr("Replaces {}. "), &[other.name]))
                .unwrap_or_default();
            lines.push(format!(
                "{replaces}Takes effect the next time you start the game."
            ));
            let id = id.clone();
            (
                if applied_id(s).as_deref() == Some(config.id()) {
                    fill(tr("Apply changes to {}?"), &[config.name()])
                } else {
                    fill(tr("Apply {}?"), &[config.name()])
                },
                lines,
                u.action(tr("Apply"), ActionKind::Primary)
                    .key("perf-confirm-apply")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| start_apply(st, id.clone())),
            )
        }
        Dialog::TurnOff => {
            let mut lines = vec!["gameinfo.gi goes back to its stock values.".to_string()];
            if status(s).is_some_and(|status| status.video.is_some()) {
                lines.push(tr("The video settings Sigil wrote go back too.").into());
            }
            (
                tr("Turn off the performance config?").to_string(),
                lines,
                u.action(tr("Turn off"), ActionKind::Primary)
                    .key("perf-confirm-off")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| start_turn_off(st)),
            )
        }
        Dialog::WriteVideo(id) => {
            let config = config(s, id)?;
            let options = options_for(s, config);
            let set = config.video(options.video.as_deref())?;
            let path = status(s)?.video_path.clone()?;
            let id = id.clone();
            (
                tr("Write video settings?").to_string(),
                vec![
                    fill(
                        tr("Sets {} graphics settings in your video.txt. Resolution, window mode, V-Sync and frame cap stay yours."),
                        &[&set.settings.len().to_string()],
                    ),
                    tr("A backup is kept, and Restore puts your values back.").into(),
                    path.display().to_string(),
                ],
                u.action(tr("Write"), ActionKind::Primary)
                    .key("perf-confirm-video")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| start_write_video(st, id.clone())),
            )
        }
        Dialog::KeepVideo => (
            tr("Keep these video settings?").to_string(),
            vec![
                tr("Leaves video.txt as it is and stops Sigil from changing these values back. Restore and Turn off will no longer revert them.").into(),
            ],
            u.action(tr("Keep"), ActionKind::Primary)
                .key("perf-confirm-keep")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| start_keep_video(st)),
        ),
        Dialog::RestoreVideo => (
            tr("Restore your video settings?").to_string(),
            vec![
                tr("Puts back every video setting Sigil changed, unless you changed it in game since.").into(),
            ],
            u.action(tr("Restore"), ActionKind::Primary)
                .key("perf-confirm-restore")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| start_restore_video(st)),
        ),
    };
    let mut body = col()
        .w_full()
        .gap(8.0)
        .child(text(title.clone()).font(H1).color(u.ink));
    for line in lines {
        body = body.child(text(line).font(BODY).color(u.muted).wrap_anywhere());
    }
    body = body.child(el().h(12.0)).child(
        row()
            .w_full()
            .gap(8.0)
            .child(spacer())
            .child(
                u.action(tr("Cancel"), ActionKind::Ghost)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| st.performance.dialog = None),
            )
            .child(confirm),
    );
    let close = |st: &mut State| st.performance.dialog = None;
    Some(
        u.dialog_shell()
            .key("perf-dialog")
            .role("dialog")
            .label(title)
            .w(480.0)
            .padding(24.0)
            .modal(true, close)
            .on_key(move |st: &mut State, ev: &KeyEvent, _| {
                if ev.key == Key::Escape {
                    close(st);
                    true
                } else {
                    false
                }
            })
            .child(body),
    )
}

// --- scenes -------------------------------------------------------------------

mod scenes;

pub use scenes::scene;
