//! Regions: the relay region picker, the fourth tab of Servers. Reachable as
//! `crate::screens::servers::regions`.
//!
//! The page lists Valve's relay locations by region with their ping, lets the player tick the
//! ones to keep Deadlock away from, saves and loads presets, and applies the choice as Windows
//! Firewall rules behind Windows' admin prompt (`deadlock_core::relays`). Reset removes every
//! Sigil relay rule. Off by default: nothing changes until Apply. The catalog loads when the tab
//! opens (cached for a day), pings run on a press, and the rules are read back (no prompt) when
//! the tab opens and after every change. On macOS and Linux the list and pings work; blocking
//! says why it needs Windows.
//!
//! While any region is blocked, the status bar shows it on every page and a modded or vanilla
//! launch says so in a notice, so a forgotten block is not mistaken for broken matchmaking.
//! No agent tool reaches any of this.

pub mod scenes;
mod view;

#[path = "../settings/locales_regions.rs"]
pub(crate) mod locales;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use deadlock_core::relays::{self, Catalog, CatalogSource, Coverage, Installed, firewall};
use rune::{TextEdit, Tx};

use crate::app::State;
use crate::jobs::Job;
use crate::model::Screen;
use crate::screens::settings::i18n::{fill, tr};
use crate::screens::settings::options;

pub use view::{dialog, view};

/// A firewall change the page waits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Apply,
    Reset,
}

/// The open confirm dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confirm {
    Apply,
    Reset,
}

pub struct RegionsState {
    /// Blocking works here (Windows). A field so headless scenes can show either page.
    pub windows: bool,
    pub catalog: Option<Catalog>,
    pub source: Option<CatalogSource>,
    pub catalog_error: Option<String>,
    pub catalog_job: Job<(Catalog, CatalogSource)>,
    /// What Windows Firewall holds, read back. None until the first read.
    pub installed: Option<Result<Installed, String>>,
    pub read_job: Job<Installed>,
    pub change: Option<Change>,
    pub change_job: Job<Installed>,
    /// The ticked locations: what Apply will block.
    pub draft: BTreeSet<String>,
    /// The draft was edited since it was last set from the firewall.
    pub edited: bool,
    /// Pings land here from the probe's workers.
    pub live: Arc<Mutex<BTreeMap<String, Option<u32>>>>,
    pub pings: BTreeMap<String, Option<u32>>,
    pub cancel: Arc<AtomicBool>,
    pub probe_job: Job<()>,
    /// Unix seconds of the last finished sweep.
    pub probed_at: Option<i64>,
    pub preset_name: TextEdit,
    /// The "Save as preset" field is open.
    pub naming: bool,
    pub confirm: Option<Confirm>,
    pub error: Option<String>,
    pub loaded: bool,
    /// The one startup read back (when the options say rules were applied) was started.
    pub checked_at_start: bool,
}

impl Default for RegionsState {
    fn default() -> Self {
        RegionsState {
            windows: firewall::blocking_supported(),
            catalog: None,
            source: None,
            catalog_error: None,
            catalog_job: Job::default(),
            installed: None,
            read_job: Job::default(),
            change: None,
            change_job: Job::default(),
            draft: BTreeSet::new(),
            edited: false,
            live: Arc::new(Mutex::new(BTreeMap::new())),
            pings: BTreeMap::new(),
            cancel: Arc::new(AtomicBool::new(false)),
            probe_job: Job::default(),
            probed_at: None,
            preset_name: TextEdit::new(""),
            naming: false,
            confirm: None,
            error: None,
            loaded: false,
            checked_at_start: false,
        }
    }
}

impl RegionsState {
    /// The read back measured against the catalog, when there is a read back.
    pub fn coverage(&self) -> Option<Coverage> {
        match &self.installed {
            Some(Ok(installed)) => Some(relays::coverage(self.catalog.as_ref(), &installed.rules)),
            _ => None,
        }
    }

    /// The locations blocked now: the read back when there is one, the saved record otherwise.
    pub fn blocked_now(&self) -> BTreeSet<String> {
        match self.coverage() {
            Some(cover) => cover.blocked.into_iter().chain(cover.partial).collect(),
            None => options::get().relay_regions.applied.into_iter().collect(),
        }
    }

    /// Locations whose state Apply would change.
    pub fn changes(&self) -> usize {
        self.blocked_now().symmetric_difference(&self.draft).count()
    }

    pub fn busy(&self) -> bool {
        self.change_job.is_running() || self.read_job.is_running()
    }

    /// The firewall is off for the active network, so the rules do nothing.
    pub fn firewall_off(&self) -> bool {
        matches!(&self.installed, Some(Ok(i)) if i.firewall_on == Some(false))
    }

    /// The read back is not the clean result of the ticks: partial, stale or turned off rules.
    pub fn needs_repair(&self) -> bool {
        self.coverage()
            .is_some_and(|c| c.any() && !c.matches(&self.draft.iter().cloned().collect::<Vec<_>>()))
    }

    /// Every location ticked: Deadlock would have nowhere to connect.
    pub fn all_blocked(&self) -> bool {
        self.catalog
            .as_ref()
            .is_some_and(|c| !c.locations.is_empty() && c.locations.iter().all(|l| self.draft.contains(&l.id)))
    }
}

/// How many locations the status bar and the launch notice report: Sigil rules read back from
/// the firewall, or the saved record before the first read. Zero means default routing.
pub fn blocked_count(s: &State) -> usize {
    let regions = &s.servers.regions;
    match regions.coverage() {
        Some(cover) if cover.any() => (cover.blocked.len() + cover.partial.len() + cover.unknown.len()).max(1),
        Some(_) => 0,
        None => options::get().relay_regions.applied.len(),
    }
}

fn idle(st: &State) -> bool {
    st.demo || rune::is_headless()
}

fn now() -> i64 {
    deadlock_core::gamebanana::now_unix()
}

// ---------------------------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------------------------

/// The first visit: the catalog (cache first) and the firewall read back.
pub fn load(st: &mut State) {
    st.servers.regions.loaded = true;
    st.servers.regions.draft = options::get().relay_regions.applied.into_iter().collect();
    start_catalog(st, false);
    start_read(st);
}

fn start_catalog(st: &mut State, force: bool) {
    if idle(st) || st.servers.regions.catalog_job.is_running() {
        return;
    }
    let dir = crate::data::data_dir();
    let wake = st.wake.clone();
    st.servers.regions.catalog_error = None;
    st.servers
        .regions
        .catalog_job
        .start(wake, move || relays::load_catalog(&dir, force, now()).map_err(|e| e.to_string()));
}

/// Read the rules back. No prompt; Windows only.
fn start_read(st: &mut State) {
    if idle(st) || !st.servers.regions.windows || st.servers.regions.read_job.is_running() {
        return;
    }
    let wake = st.wake.clone();
    st.servers
        .regions
        .read_job
        .start(wake, || firewall::read_installed().map_err(|e| e.to_string()));
}

/// Refresh: Valve's list again, and the rules.
pub fn refresh(st: &mut State) {
    start_catalog(st, true);
    start_read(st);
}

// ---------------------------------------------------------------------------------------------
// Selection and presets
// ---------------------------------------------------------------------------------------------

pub fn toggle(st: &mut State, id: &str) {
    let regions = &mut st.servers.regions;
    if !regions.draft.remove(id) {
        regions.draft.insert(id.to_string());
    }
    regions.edited = true;
}

/// Tick or untick every location in a region.
pub fn set_region(st: &mut State, region: relays::Region, block: bool) {
    let regions = &mut st.servers.regions;
    let Some(catalog) = &regions.catalog else { return };
    for location in catalog.locations.iter().filter(|l| l.region == region) {
        if block {
            regions.draft.insert(location.id.clone());
        } else {
            regions.draft.remove(&location.id);
        }
    }
    regions.edited = true;
}

/// Back to what is blocked now.
pub fn discard(st: &mut State) {
    let now = st.servers.regions.blocked_now();
    let regions = &mut st.servers.regions;
    regions.draft = now;
    regions.edited = false;
}

/// Load a preset into the selection. Apply makes it real.
pub fn load_preset(st: &mut State, name: &str) {
    let Some(preset) = options::get().relay_regions.presets.into_iter().find(|p| p.name == name) else {
        return;
    };
    let regions = &mut st.servers.regions;
    regions.draft = preset.blocked.into_iter().collect();
    regions.edited = true;
    st.status = fill(tr("Loaded {}. Press Apply to block these regions."), &[name]);
}

pub fn save_preset(st: &mut State) {
    let name = st.servers.regions.preset_name.text.trim().to_string();
    let blocked: Vec<String> = st.servers.regions.draft.iter().cloned().collect();
    let mut result = Ok(());
    options::edit(st, |o| result = o.relay_regions.save_preset(&name, &blocked));
    match result {
        Ok(()) => {
            st.servers.regions.naming = false;
            st.servers.regions.preset_name = TextEdit::new("");
            st.servers.regions.error = None;
        }
        Err(error) => st.servers.regions.error = Some(tr(&error.to_string()).to_string()),
    }
}

pub fn delete_preset(st: &mut State, name: &str) {
    let name = name.to_string();
    options::edit(st, |o| o.relay_regions.delete_preset(&name));
}

// ---------------------------------------------------------------------------------------------
// Latency
// ---------------------------------------------------------------------------------------------

/// Measure every location, or stop the sweep that runs.
pub fn toggle_probe(st: &mut State) {
    if st.servers.regions.probe_job.is_running() {
        st.servers.regions.cancel.store(true, Ordering::SeqCst);
        return;
    }
    let Some(catalog) = st.servers.regions.catalog.clone() else { return };
    if idle(st) {
        return;
    }
    if st.prefs.offline {
        st.servers.regions.error = Some(tr("Offline mode is on. Turn it off in Settings to measure pings.").to_string());
        return;
    }
    let cancel = Arc::new(AtomicBool::new(false));
    st.servers.regions.cancel = cancel.clone();
    let live = Arc::new(Mutex::new(BTreeMap::new()));
    st.servers.regions.live = live.clone();
    let wake = st.wake.clone();
    let each = wake.clone();
    st.servers.regions.probe_job.start(wake, move || {
        relays::probe::sweep(&catalog.locations, &cancel, &relays::probe::ping_once, &|id, ms| {
            if let Ok(mut map) = live.lock() {
                map.insert(id.to_string(), ms);
            }
            if let Some(wake) = &each {
                wake();
            }
        });
        Ok(())
    });
}

// ---------------------------------------------------------------------------------------------
// Apply and Reset (the presses are the approval)
// ---------------------------------------------------------------------------------------------

/// Open the Apply dialog when there is something to apply.
pub fn ask_apply(st: &mut State) {
    let regions = &st.servers.regions;
    if !regions.windows
        || regions.busy()
        || regions.catalog.is_none()
        || regions.all_blocked()
        || (regions.changes() == 0 && !regions.needs_repair())
    {
        return;
    }
    st.servers.regions.confirm = Some(Confirm::Apply);
}

pub fn ask_reset(st: &mut State) {
    if !st.servers.regions.windows || st.servers.regions.busy() {
        return;
    }
    st.servers.regions.confirm = Some(Confirm::Reset);
}

/// The Apply dialog's button: write the rules behind the admin prompt.
pub fn apply(st: &mut State) {
    st.servers.regions.confirm = None;
    let regions = &st.servers.regions;
    let Some(catalog) = regions.catalog.clone() else { return };
    if regions.busy() || !regions.windows {
        return;
    }
    let Some(game_root) = st.game_root.clone() else {
        st.servers.regions.error = Some(tr("Set the game folder in Settings first: the rules apply to Deadlock only.").to_string());
        return;
    };
    let blocked: Vec<String> = regions.draft.iter().cloned().collect();
    st.servers.regions.error = None;
    if idle(st) {
        return;
    }
    options::edit(st, |o| o.relay_regions.pending = true);
    st.servers.regions.change = Some(Change::Apply);
    let work = crate::data::data_dir();
    let wake = st.wake.clone();
    st.servers.regions.change_job.start(wake, move || {
        firewall::apply(&catalog, &blocked, &game_root, &work, true).map_err(|e| e.to_string())
    });
}

/// The Reset dialog's button: remove every Sigil relay rule behind the admin prompt.
pub fn reset(st: &mut State) {
    st.servers.regions.confirm = None;
    if st.servers.regions.busy() || !st.servers.regions.windows || idle(st) {
        return;
    }
    st.servers.regions.error = None;
    options::edit(st, |o| o.relay_regions.pending = true);
    st.servers.regions.change = Some(Change::Reset);
    let work = crate::data::data_dir();
    let wake = st.wake.clone();
    st.servers
        .regions
        .change_job
        .start(wake, move || firewall::reset(&work, true).map_err(|e| e.to_string()));
}

/// Record a read back: the saved record follows the firewall, and an unedited selection does too.
fn take_installed(st: &mut State, installed: Installed) {
    let cover = relays::coverage(st.servers.regions.catalog.as_ref(), &installed.rules);
    let applied: Vec<String> = cover.blocked.iter().chain(&cover.partial).cloned().collect();
    st.servers.regions.installed = Some(Ok(installed));
    if options::get().relay_regions.applied != relays::normalize_selection(&applied) {
        options::edit_quiet(|o| o.relay_regions.applied = relays::normalize_selection(&applied));
    }
    // A change that never reported back (Sigil closed during the admin prompt): the read back
    // is the truth now, so say so once and clear the mark.
    if options::get().relay_regions.pending && !st.servers.regions.change_job.is_running() {
        options::edit_quiet(|o| o.relay_regions.pending = false);
        crate::notify::push(
            st,
            crate::notify::Notice::warning(tr(
                "The last firewall change did not report back. What is shown is what Windows holds now.",
            )),
        );
    }
    if !st.servers.regions.edited {
        st.servers.regions.draft = applied.into_iter().collect();
    }
}

// ---------------------------------------------------------------------------------------------
// Poll
// ---------------------------------------------------------------------------------------------

pub fn poll(st: &mut State, _now: f64, _tx: &mut Tx) -> bool {
    let on_page = st.screen == Screen::Servers && st.servers.tab == crate::screens::servers::Tab::Regions;
    let mut changed = false;
    if on_page && !st.servers.regions.loaded {
        load(st);
        changed = true;
    }
    // Once per run: read the rules back when the saved record says some were applied, so the
    // status bar is right from the first minutes even if the tab is never opened.
    if !st.servers.regions.checked_at_start && !idle(st) {
        st.servers.regions.checked_at_start = true;
        let saved = options::get().relay_regions;
        if !saved.applied.is_empty() || saved.pending {
            start_read(st);
        }
    }
    if let Some(result) = st.servers.regions.catalog_job.try_take() {
        match result {
            Ok((catalog, source)) => {
                let regions = &mut st.servers.regions;
                regions.catalog = Some(catalog);
                regions.source = Some(source);
                // The read back is measured against the catalog: take it again for the record.
                if let Some(Ok(installed)) = regions.installed.clone() {
                    take_installed(st, installed);
                }
            }
            Err(error) => st.servers.regions.catalog_error = Some(error),
        }
        changed = true;
    }
    if let Some(result) = st.servers.regions.read_job.try_take() {
        match result {
            Ok(installed) => take_installed(st, installed),
            Err(error) => st.servers.regions.installed = Some(Err(error)),
        }
        changed = true;
    }
    if let Some(result) = st.servers.regions.change_job.try_take() {
        let change = st.servers.regions.change.take();
        options::edit_quiet(|o| o.relay_regions.pending = false);
        match result {
            Ok(installed) => {
                let firewall_off = installed.firewall_on == Some(false);
                st.servers.regions.edited = false;
                take_installed(st, installed);
                report(st, change, firewall_off);
            }
            Err(error) => {
                st.servers.regions.error = Some(error);
                start_read(st);
            }
        }
        changed = true;
    }
    let regions = &mut st.servers.regions;
    if regions.probe_job.is_running()
        && let Ok(live) = regions.live.lock()
        && *live != regions.pings
    {
        regions.pings = live.clone();
        changed = true;
    }
    if let Some(result) = regions.probe_job.try_take() {
        if result.is_ok() && !regions.cancel.load(Ordering::SeqCst) {
            regions.probed_at = Some(now());
        }
        if let Ok(live) = regions.live.lock() {
            regions.pings = live.clone();
        }
        changed = true;
    }
    changed
}

/// Say what the change did, from the read back.
fn report(st: &mut State, change: Option<Change>, firewall_off: bool) {
    let regions = &st.servers.regions;
    let cover = regions.coverage().unwrap_or_default();
    let notice = match change {
        Some(Change::Reset) if !cover.any() => {
            crate::notify::Notice::success(tr("Default routing is back. No relay region is blocked."))
        }
        Some(Change::Reset) => crate::notify::Notice::error(tr("Some relay rules are still there."))
            .detail(tr("Press Reset again. If it keeps failing, remove the Sigil-Relay rules in Windows Defender Firewall.")),
        Some(Change::Apply) if cover.matches(&regions.draft.iter().cloned().collect::<Vec<_>>()) => {
            let count = cover.blocked.len();
            if count == 0 {
                crate::notify::Notice::success(tr("Default routing is back. No relay region is blocked."))
            } else {
                crate::notify::Notice::success(fill(tr("{} relay locations blocked for Deadlock."), &[&count.to_string()]))
                    .detail(tr("Restart Deadlock if it is running. Reset in Servers > Regions brings default routing back."))
            }
        }
        Some(Change::Apply) => crate::notify::Notice::error(tr("Windows Firewall did not take every rule."))
            .detail(tr("Press Apply again, or Reset to remove what is there.")),
        None => return,
    };
    crate::notify::push(st, notice);
    if firewall_off {
        crate::notify::push(
            st,
            crate::notify::Notice::warning(tr("Windows Firewall is off for this network"))
                .detail(tr("The rules are in place but do nothing until the firewall is on.")),
        );
    }
}

/// The launch notice: the regions blocked right now, so a long queue has an explanation.
/// Called by every launch; silent when nothing is blocked.
pub fn remind_at_launch(st: &mut State) {
    let count = blocked_count(st);
    if count == 0 {
        return;
    }
    let notice = crate::notify::Notice::warning(fill(tr("{} relay locations are blocked"), &[&count.to_string()]))
        .detail(tr("Matchmaking only uses the open regions, so queues can be longer. Reset in Servers > Regions."))
        .action(tr("Open Regions"), open);
    crate::notify::push(st, notice);
}

/// Go to Servers > Regions (status bar, launch notice).
pub fn open(st: &mut State) {
    st.go(Screen::Servers);
    st.servers.tab = crate::screens::servers::Tab::Regions;
    st.servers.menu = None;
}

/// The status bar's mark while any region is blocked, on every page. A press opens the tab.
pub fn status_chip(s: &State, u: &crate::ui::Ui) -> Option<rune::El> {
    let count = blocked_count(s);
    if count == 0 {
        return None;
    }
    let words = fill(tr("{} relay locations blocked"), &[&count.to_string()]);
    let label = fill(tr("{} relay locations blocked. Open Servers, Regions"), &[&count.to_string()]);
    Some(crate::statusbar::part(u, "statusbar-regions", &crate::icons::SHIELD, words, u.amber, label).on_press(open))
}
