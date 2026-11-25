//! "Help build Sigil": opt-in product data, the app side (ADR 0017). The core
//! (`deadlock_core::product_data`) owns the schema, the scrubbers, the consent gate, the queue,
//! the log and every send; this file asks, records and shows.
//!
//! - Asking: the privacy step of the first-run setup (`onboarding.rs`), and once as a dialog for installs
//!   that finished setup before this existed. "Share" and "Not now" are equal buttons.
//! - Settings > Privacy: the switch, "What Sigil shared" (the local log) and "Delete my data".
//! - Recording: [`record`] is one push onto the core's in-memory list. Persisting runs on a short
//!   worker every 30 seconds while something waits, and sending on a worker every 10 minutes, so
//!   nothing touches disk or the network on the UI thread and nothing runs per frame.
//! - Only the desktop app calls `product_data::init`; the CLI and the agent host never do.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use deadlock_core::product_data::{self as pd, Code, ModAction, ModId, Record};
use deadlock_core::telemetry::{self, Consent};
use rune::{El, Key, KeyEvent, Tx, col, row, spacer, text};

use super::i18n::{fill, tr};
use super::{button, detail, section, setting_row, toggle_row};
use crate::app::State;
use crate::jobs::Job;
use crate::ui::{ActionKind, BODY, H1, MONO, SMALL, Ui};

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// How often waiting records move to the disk queue.
const PERSIST_EVERY: Duration = Duration::from_secs(30);
/// How often the queue is sent.
const SEND_EVERY: Duration = Duration::from_secs(600);

#[derive(Default)]
pub struct Product {
    /// The one-time question for installs that finished setup before it existed.
    pub asking: bool,
    started: Option<Instant>,
    last_persist: Option<Instant>,
    last_send: Option<Instant>,
    /// A persist or send worker is running.
    busy: Arc<AtomicBool>,
    /// "What Sigil shared": the log lines while the dialog is open.
    pub log: Option<Vec<String>>,
    pub deleting: Job<u64>,
    /// Records sent at most once per run (stats, crash suspect, pairs, findings).
    once: HashSet<String>,
    /// The enabled set went this run. A flag, not a `once` key: it is checked every poll.
    set_sent: bool,
    /// The last Browse query recorded, so a repaint of the same page records nothing.
    last_query: Option<String>,
}

fn data() -> std::path::PathBuf {
    crate::data::data_dir()
}

fn quiet(st: &State) -> bool {
    rune::is_headless() || cfg!(test) || st.demo
}

/// Startup: make the recorder from the consent file. Hardware class and settings go once per
/// version, read on a worker.
pub fn on_startup(st: &mut State) {
    if quiet(st) {
        return;
    }
    let recorder = pd::init(&data());
    recorder.apply_consent(&st.settings.telemetry.consent);
    st.settings.telemetry.product.started = Some(Instant::now());
    once_per_version(st);
}

/// Hardware class and settings chosen, at most once per Sigil version each (settings again when
/// they change). The machine read runs on a worker.
fn once_per_version(st: &State) {
    let Some(recorder) = pd::global() else { return };
    if !recorder.enabled() {
        return;
    }
    let settings = settings_record(st);
    let game_root = st.game_root.clone();
    // The marks file and the machine read are disk work: both on the worker.
    let _ = std::thread::Builder::new().name("sigil-product-hw".into()).spawn(move || {
        let key = format!("settings:{}", serde_json::to_string(&settings).unwrap_or_default());
        if recorder.once_per_version(&key, VERSION) {
            recorder.record(settings);
        }
        if !recorder.once_per_version("hardware", VERSION) {
            return;
        }
        let machine = deadlock_core::benchmark::system::read_machine();
        let steam_roots = deadlock_core::discovery::default_steam_roots();
        let video = game_root
            .as_deref()
            .map(|root| deadlock_core::benchmark::system::read_settings(root, &steam_roots));
        let (width, height) = video.as_ref().and_then(|v| v.resolution()).and_then(|r| parse_resolution(&r)).unwrap_or((0, 0));
        recorder.record(Record::Hardware {
            gpu: machine.gpu.as_deref().and_then(pd::model_name),
            cpu: machine.cpu.as_deref().and_then(pd::model_name),
            ram_gb: pd::round_ram_gb(machine.ram_mb.unwrap_or(0)),
            width: width.min(u16::MAX as u32) as u16,
            height: height.min(u16::MAX as u32) as u16,
            // Sigil does not read the monitor's refresh rate yet.
            refresh_hz: 0,
        });
    });
}

/// "2560 x 1440" as numbers.
pub fn parse_resolution(text: &str) -> Option<(u32, u32)> {
    let (w, h) = text.split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

fn settings_record(st: &State) -> Record {
    let code = |value: String| Code::new(&value).unwrap_or_else(|| Code::new("other").expect("code"));
    Record::Settings {
        theme: code(format!("{:?}", st.prefs.theme)),
        density: code(format!("{:?}", st.prefs.personal.density)),
        language: code(format!("{:?}", st.prefs.language)),
        mature: st.prefs.mature != crate::prefs::Mature::Hide,
    }
}

/// Record one event. A no-op without consent (the core's gate), in the demo, a headless run or a
/// test. Cheap: one atomic read and a push.
pub fn record(st: &State, record: Record) {
    if quiet(st) {
        return;
    }
    if let Some(recorder) = pd::global() {
        recorder.record(record);
    }
}

/// True the first time `key` is seen this run, so a record goes once per session.
pub fn first(st: &mut State, key: impl Into<String>) -> bool {
    st.settings.telemetry.product.once.insert(key.into())
}

/// The GameBanana id of an installed slot, if Sigil knows one.
pub fn mod_of_slot(st: &State, slot: u16) -> Option<ModId> {
    let addon = st.addons.addons.iter().find(|a| a.slot == slot)?;
    crate::screens::installed::meta_for(st, addon)
        .and_then(|meta| meta.gamebanana_id)
        .and_then(ModId::new)
}

/// One mod action, for a GameBanana mod only.
pub fn action(st: &State, id: Option<u64>, action: ModAction) {
    if let Some(mod_id) = id.and_then(ModId::new) {
        record(st, Record::ModAction { mod_id, action });
    }
}

/// A page opened: counted as feature use.
pub fn feature(st: &State, name: &str) {
    if let Some(feature) = Code::new(name) {
        record(st, Record::FeatureUse { feature, count: 1 });
    }
}

/// An install's failure text as a reason code.
pub fn reason_code(error: &str) -> Code {
    let lower = error.to_lowercase();
    let code = if error.contains(crate::screens::browse::CANCELLED) {
        "cancelled"
    } else if error.contains(deadlock_core::install::NOTHING_INSTALLABLE) {
        "nothing_installable"
    } else if lower.contains("more than one vpk") {
        "several_vpks"
    } else if lower.contains("deadlock is running") {
        "game_running"
    } else if deadlock_core::error::mentions_in_use(error) {
        "file_in_use"
    } else if lower.contains("worker stopped") {
        "worker_stopped"
    } else if lower.contains("download") || lower.contains("network") || lower.contains("http") {
        "download_failed"
    } else if lower.contains("game folder") || lower.contains("not found") {
        "game_missing"
    } else if lower.contains("space") {
        "disk_full"
    } else {
        "other"
    };
    Code::new(code).expect("reason code")
}

/// The current game build (`ClientVersion` in steam.inf) as a number. A file read: call it at an
/// action, never per frame.
pub fn game_build(st: &State) -> Option<u32> {
    let root = st.game_root.as_deref()?;
    deadlock_core::host::read_build(root)?.trim().parse().ok()
}

/// The enabled mods with GameBanana ids and their slots, once per run after the first scan.
pub fn enabled_set(st: &mut State) {
    if st.settings.telemetry.product.set_sent || quiet(st) || !st.addons.scanned || !pd::global().is_some_and(|r| r.enabled()) {
        return;
    }
    st.settings.telemetry.product.set_sent = true;
    let mods: Vec<pd::SlotMod> = st
        .addons
        .addons
        .iter()
        .filter(|a| a.enabled)
        .filter_map(|a| mod_of_slot(st, a.slot).map(|mod_id| pd::SlotMod { mod_id, slot: a.slot }))
        .collect();
    if !mods.is_empty() {
        record(st, Record::EnabledSet { mods });
    }
}

/// An install from GameBanana finished: the result with a reason code, and for a success the
/// install itself and, when it was a For you pick, that outcome.
pub fn install_done(st: &State, id: Option<u64>, error: Option<&str>) {
    let Some(mod_id) = id.and_then(ModId::new) else { return };
    let ok = error.is_none();
    record(st, Record::InstallResult { mod_id, ok, reason: error.map(reason_code) });
    if ok {
        record(st, Record::ModAction { mod_id, action: ModAction::Installed });
        rec_outcome(st, mod_id.get(), pd::RecOutcome::Installed);
    }
}

/// A For you pick was opened or installed: its row and position.
pub fn rec_outcome(st: &State, id: u64, outcome: pd::RecOutcome) {
    let fy = &st.browse_view.shelves.for_you;
    let Some(position) = fy.picks.iter().position(|c| c.id == id) else { return };
    let (Some(mod_id), Some(row)) = (ModId::new(id), Code::new(if fy.personal { "for_you" } else { "rising" })) else { return };
    record(st, Record::RecShown { row, position: position.min(100) as u8, mod_id, outcome });
}

/// The first page of For you picks was shown (once per run, where the app counts it).
pub fn rec_shown(st: &State) {
    let fy = &st.browse_view.shelves.for_you;
    let Some(row) = Code::new(if fy.personal { "for_you" } else { "rising" }) else { return };
    for (position, card) in fy.picks.iter().take(5).enumerate() {
        if let Some(mod_id) = ModId::new(card.id) {
            record(st, Record::RecShown { row: row.clone(), position: position as u8, mod_id, outcome: pd::RecOutcome::Shown });
        }
    }
}

/// A mod removed from Installed: the removal and how long it stayed.
pub fn removed(st: &State, meta: Option<&deadlock_core::import::ModMeta>) {
    let Some(meta) = meta else { return };
    let Some(mod_id) = meta.gamebanana_id.and_then(ModId::new) else { return };
    record(st, Record::ModAction { mod_id, action: ModAction::Removed });
    if let Some(secs) = pd::seconds_since(&meta.recorded_at) {
        record(st, Record::ModKept { mod_id, kept: pd::KeptBucket::from_secs(secs) });
    }
}

/// A mod turned on or off. Off while the last game session ended in a crash also counts as
/// "turned off after a crash".
pub fn toggled(st: &State, slot: u16, enable: bool) {
    let Some(mod_id) = mod_of_slot(st, slot) else { return };
    record(st, Record::ModAction { mod_id, action: if enable { ModAction::Enabled } else { ModAction::Disabled } });
    let crashed = st.launch.read.as_ref().is_some_and(|read| read.crash.is_some());
    if !enable && crashed {
        record(st, Record::OffAfterCrash { mod_id });
    }
}

/// Several mods switched at once (bulk toggles, Locker's "only this one", profiles of a map):
/// one record per mod that actually changed.
pub fn switched(st: &State, slots: impl Iterator<Item = u16>, wanted: &std::collections::BTreeMap<u16, bool>) {
    for slot in slots {
        if let Some(on) = wanted.get(&slot) {
            toggled(st, slot, *on);
        }
    }
}

/// Overlap pairs between enabled mods, each pair once per run.
pub fn overlaps(st: &mut State) {
    if quiet(st) || !pd::global().is_some_and(|r| r.enabled()) {
        return;
    }
    let pairs: Vec<(u16, u16)> = st
        .installed
        .overlaps
        .iter()
        .flat_map(|(a, others)| others.keys().filter(move |b| a < *b).map(move |b| (*a, *b)))
        .collect();
    for (a, b) in pairs {
        let (Some(a), Some(b)) = (mod_of_slot(st, a), mod_of_slot(st, b)) else { continue };
        if first(st, format!("overlap:{}:{}", a.get(), b.get())) {
            record(st, Record::ModPair { a, b, kind: pd::PairKind::Overlap });
        }
    }
}

/// The player settled a conflict between two mods.
pub fn conflict(st: &State, winner: u16, loser: u16) {
    if let (Some(a), Some(b)) = (mod_of_slot(st, winner), mod_of_slot(st, loser)) {
        record(st, Record::ModPair { a, b, kind: pd::PairKind::Conflict });
    }
}

/// Health review findings, by mod and game build, once per run per finding.
pub fn health_findings(st: &mut State, findings: Vec<(u16, String)>) {
    if quiet(st) || findings.is_empty() || !pd::global().is_some_and(|r| r.enabled()) {
        return;
    }
    let build = game_build(st);
    for (slot, kind) in findings {
        let (Some(mod_id), Some(finding)) = (mod_of_slot(st, slot), Code::new(&kind)) else { continue };
        if first(st, format!("finding:{}:{}", mod_id.get(), finding.as_str())) {
            record(st, Record::CompatFinding { mod_id, finding, game_build: build });
        }
    }
}

/// A compatibility repair applied or failed.
pub fn repair(st: &State, slot: u16, ok: bool) {
    let Some(mod_id) = mod_of_slot(st, slot) else { return };
    let Some(repair) = Code::new("compat_repair") else { return };
    record(st, Record::RepairResult { mod_id, repair, ok, game_build: game_build(st) });
}

/// The crash card's suspect mod, once per crash.
pub fn crash_suspect(st: &mut State) {
    let Some((at, slot)) = st
        .launch
        .read
        .as_ref()
        .and_then(|read| read.crash.as_ref())
        .and_then(|crash| crash.suspect.as_ref().map(|s| (crash.at_secs, s.slot)))
    else {
        return;
    };
    let Some(mod_id) = mod_of_slot(st, slot) else { return };
    if first(st, format!("suspect:{at}")) {
        record(st, Record::CrashSuspect { mod_id });
    }
}

/// A Browse page landed: filters, sort, scrubbed search and the result count. Once per query.
pub fn browse_query(st: &mut State, results: usize) {
    if quiet(st) || !pd::global().is_some_and(|r| r.enabled()) {
        return;
    }
    let f = &st.browse_view.filters;
    let mut filters = vec![format!("section_{:?}", f.section)];
    for (on, name) in [
        (f.category.is_some(), "category"),
        (f.sub.is_some(), "subcategory"),
        (f.hero.is_some(), "hero"),
        (f.featured, "featured"),
        (f.creator.is_some(), "creator"),
        (f.hide_outdated, "hide_outdated"),
        (f.exclude, "exclude"),
        (f.favorites, "favorites"),
        (f.for_you, "for_you"),
        (f.rising, "rising"),
        (f.date_from > 0 || f.date_to > 0, "date_range"),
    ] {
        if on {
            filters.push(name.to_string());
        }
    }
    filters.push(format!("period_{:?}", f.period));
    filters.push(format!("owned_{:?}", f.owned));
    let filters: Vec<Code> = filters.iter().filter_map(|name| Code::new(name)).collect();
    let sort = Code::new(&format!("{:?}", f.order.unwrap_or_else(|| deadlock_core::gamebanana::BrowseOrder::from_sort(st.prefs.browse_sort))));
    let search = pd::scrub_search(&st.browse_view.applied_search);
    let key = format!("{filters:?}{sort:?}{search:?}");
    if st.settings.telemetry.product.last_query.as_deref() == Some(key.as_str()) {
        return;
    }
    st.settings.telemetry.product.last_query = Some(key);
    record(st, Record::BrowseQuery { sort, filters, search, results: results.min(u32::MAX as usize) as u32 });
}

/// The player's ten most played heroes in rough hours, and the rank tier, once per run.
pub fn stats(st: &mut State) {
    if quiet(st) || !pd::global().is_some_and(|r| r.enabled()) || !first(st, "stats") {
        return;
    }
    let Some(snapshot) = st.stats.snapshot.as_ref() else { return };
    let mut heroes: Vec<_> = snapshot.heroes.iter().filter(|h| h.time_played_seconds > 0).collect();
    heroes.sort_by(|a, b| b.time_played_seconds.cmp(&a.time_played_seconds));
    let heroes: Vec<pd::HeroPlay> = heroes
        .iter()
        .take(pd::MAX_HEROES)
        .filter_map(|h| {
            Some(pd::HeroPlay { hero_id: u16::try_from(h.hero_id).ok()?, playtime: pd::PlaytimeBucket::from_secs(h.time_played_seconds) })
        })
        .collect();
    let tier = (snapshot.rank.badge > 0).then(|| pd::rank_tier(snapshot.rank.badge)).flatten();
    if !heroes.is_empty() {
        record(st, Record::TopHeroes { heroes });
    }
    if let Some(tier) = tier {
        record(st, Record::RankTier { tier });
    }
}

/// A performance config applied: its id and version.
pub fn perf_config(st: &State, id: &str, version: &str) {
    let Some(config) = Code::new(id) else { return };
    let version = version.split(|c: char| !c.is_ascii_digit()).find(|p| !p.is_empty()).and_then(|p| p.parse().ok()).unwrap_or(0);
    record(st, Record::PerfConfig { config, version });
}

/// A finished benchmark: average and 1% low frame rates of the first measured pass, the scene
/// (map name, never the match id), the resolution and the performance config as the preset.
pub fn benchmark(st: &State, run: &deadlock_core::benchmark::store::RunRecord) {
    let Some(summary) = run.passes.iter().filter(|p| !p.warmup).find_map(|p| p.summary.as_ref()) else { return };
    let Some((width, height)) = run.settings.resolution().and_then(|r| parse_resolution(&r)) else { return };
    let scene = run
        .spec
        .demo
        .as_ref()
        .and_then(|d| d.map.as_deref())
        .and_then(Code::new)
        .or_else(|| Code::new("custom"))
        .expect("code");
    let fps = |v: f64| v.clamp(0.0, 2000.0).round() as u16;
    record(
        st,
        Record::Benchmark {
            avg_fps: fps(summary.avg_fps),
            low_fps: fps(summary.low_1.unwrap_or(0.0)).min(fps(summary.avg_fps)),
            scene,
            width: width.min(u16::MAX as u32) as u16,
            height: height.min(u16::MAX as u32) as u16,
            preset: run.settings.performance_config.as_deref().and_then(Code::new),
        },
    );
}

// --- the worker side -----------------------------------------------------------------------

/// Persist and send on workers; the one-time question; the delete result.
pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.settings.telemetry.product.deleting.try_take() {
        changed = true;
        match result {
            Ok(_) => crate::notify::push(
                st,
                crate::notify::Notice::success(tr("Your data was deleted")).detail(tr("Sigil's server deleted the records from this computer. A new id is made if you share again.")),
            ),
            Err(error) => crate::notify::push(
                st,
                crate::notify::Notice::error(tr("Your data was not deleted")).detail(fill(tr("Try again later: {}"), &[&error])),
            ),
        }
    }
    if quiet(st) || !st.settings.telemetry.loaded {
        return changed;
    }
    let t = &st.settings.telemetry;
    if !t.consent.product_asked
        && !t.product.asking
        && !t.asking
        && !t.crash.asking
        && t.consent.asked
        && st.settings.extra.onboarding.is_none()
        && super::options::get().onboarded
    {
        st.settings.telemetry.product.asking = true;
        changed = true;
    }
    enabled_set(st);
    let Some(recorder) = pd::global() else { return changed };
    let p = &mut st.settings.telemetry.product;
    if p.busy.load(Ordering::Relaxed) || !recorder.enabled() {
        return changed;
    }
    let send = !st.prefs.offline && p.last_send.is_none_or(|at| at.elapsed() >= SEND_EVERY);
    let persist = recorder.pending_len() > 0 && p.last_persist.is_none_or(|at| at.elapsed() >= PERSIST_EVERY);
    if !send && !persist {
        return changed;
    }
    p.last_persist = Some(Instant::now());
    if send {
        p.last_send = Some(Instant::now());
    }
    let busy = Arc::clone(&p.busy);
    busy.store(true, Ordering::Relaxed);
    let game_root = st.game_root.clone();
    let spawned = std::thread::Builder::new().name("sigil-product".into()).spawn(move || {
        if send {
            let machine = deadlock_core::benchmark::system::read_machine();
            let context = pd::Context {
                version: VERSION.to_string(),
                os_build: pd::OsBuild::from_text(&machine.os),
                game_build: game_root
                    .as_deref()
                    .and_then(deadlock_core::host::read_build)
                    .and_then(|b| b.trim().parse().ok()),
            };
            let _ = recorder.flush(&context);
        } else {
            let _ = recorder.persist();
        }
        busy.store(false, Ordering::Relaxed);
    });
    if spawned.is_err() {
        st.settings.telemetry.product.busy.store(false, Ordering::Relaxed);
    }
    changed
}

/// Quit: the session length, then the waiting records go to disk (a small write, once).
pub fn on_quit(st: &mut State) {
    if quiet(st) {
        return;
    }
    if let Some(started) = st.settings.telemetry.product.started {
        record(st, Record::Session { length: pd::SessionBucket::from_secs(started.elapsed().as_secs()) });
    }
    if let Some(recorder) = pd::global() {
        let _ = recorder.persist();
    }
}

// --- consent -------------------------------------------------------------------------------

/// The player's answer, from the setup step, the dialog or the switch.
pub fn set(st: &mut State, share: bool) {
    let mut consent: Consent = st.settings.telemetry.consent;
    consent.product_asked = true;
    consent.product = share;
    st.settings.telemetry.consent = consent;
    st.settings.telemetry.product.asking = false;
    if quiet(st) {
        return;
    }
    if let Err(error) = telemetry::write_consent(&data(), &consent) {
        st.settings.message = Some((fill(tr("Settings could not be saved: {}"), &[&error.to_string()]), true));
    }
    if let Some(recorder) = pd::global() {
        recorder.apply_consent(&consent);
    }
    if share {
        once_per_version(st);
    }
}

fn delete(st: &mut State) {
    if st.settings.telemetry.product.deleting.is_running() || quiet(st) {
        return;
    }
    let Some(recorder) = pd::global() else { return };
    st.settings.telemetry.product.log = None;
    let wake = st.wake.clone();
    st.settings.telemetry.product.deleting.start(wake, move || recorder.delete_my_data(VERSION).map_err(|e| e.to_string()));
}

fn open_log(st: &mut State) {
    let lines = pd::global()
        .map(|recorder| {
            recorder
                .shared_log()
                .iter()
                .rev()
                .map(|s| format!("{}  {}", s.when(), serde_json::to_string(&s.record).unwrap_or_default()))
                .collect()
        })
        .unwrap_or_default();
    st.settings.telemetry.product.log = Some(lines);
}

// --- UI ------------------------------------------------------------------------------------

pub const TITLE: &str = "Help build Sigil";
const INTRO: &str = "Share how you use Sigil so it can suggest mods, warn about mods that break after a patch and show what frame rates to expect. Nothing is sent unless you choose Share, and you can turn it off any time in Settings, Privacy.";
const SENT: [&str; 6] = [
    "Mods from GameBanana: the mod id when you install, remove, turn on or off, update, open, save or hide one, and which ones you use together.",
    "Mod health: failed installs, overlaps, conflicts and repairs, by mod id and game build.",
    "Browse: filters, sort, your search text (short, lower case, never emails, paths, links or long numbers) and which suggestions you open.",
    "Play: your ten most played heroes in rough hours, and your rank tier.",
    "Performance: configs you apply, benchmark frame rates, your GPU and CPU model, RAM rounded and screen size.",
    "App use: pages you open, theme, language, launches, how long Sigil stays open, and the Sigil, system and game version.",
];
/// [`NEVER`] for a build without the agent, which has no agent to name.
const NEVER_NO_AGENT: &str = "Never sent: your Steam id or name, friends, matches, chat, file names or paths, local mods, mod files, keys or your IP address. Sent with a random id the server keeps only as a hash, and kept until you delete it with Delete my data in Settings. After two years with nothing new it is cut loose from your id.";
const NEVER: &str = "Never sent: your Steam id or name, friends, matches, chat, file names or paths, local mods, mod files, keys, your IP address, or anything from the AI agent. Sent with a random id the server keeps only as a hash, and kept until you delete it with Delete my data in Settings. After two years with nothing new it is cut loose from your id.";

/// The window height, noted by the view each frame (a store, no work), so the list in
/// [`consent_body`] scrolls instead of pushing the buttons off a short window.
static WINDOW_H: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

pub fn note_window(height: f32) {
    WINDOW_H.store(height.to_bits(), Ordering::Relaxed);
}

/// Height left for the list of what is sent: the window minus the card's other parts (title,
/// intro, the never-sent note, the link, the buttons and the setup's step line).
/// `reserve` is what the caller adds around the body on top of that.
fn list_max_h(reserve: f32) -> f32 {
    let window = f32::from_bits(WINDOW_H.load(Ordering::Relaxed));
    let window = if window > 0.0 { window } else { 800.0 };
    (window - 500.0 - reserve).clamp(120.0, 600.0)
}

/// What is sent and what never is, for the dialog.
pub fn consent_body(u: &Ui) -> El {
    consent_body_reserving(u, 0.0)
}

/// [`consent_body`] inside a card that shows `reserve` points more above it (the setup's
/// privacy step).
pub fn consent_body_reserving(u: &Ui, reserve: f32) -> El {
    let item = |line: &str| {
        row()
            .w_full()
            .gap(8.0)
            .child(text("·").font(BODY).color(u.faint))
            .child(text(tr(line).to_string()).font(BODY).color(u.prose).wrap().grow(1.0).min_w(0.0))
    };
    let mut list = col().w_full().gap(6.0);
    for line in SENT {
        list = list.child(item(line));
    }
    col()
        .w_full()
        .gap(12.0)
        .child(text(tr(INTRO).to_string()).font(BODY).color(u.muted).wrap().w_full())
        .child(u.scroll_area(
            col()
                .key("product-sent-list")
                .w_full()
                .max_h(list_max_h(reserve))
                .scroll(rune::ScrollSpec { stick_bottom: false })
                .child(list),
        ))
        .child(text(tr(if crate::prefs::AGENT { NEVER } else { NEVER_NO_AGENT }).to_string()).font(SMALL).color(u.faint).wrap().w_full())
        .child(
            row().child(
                u.action(tr("What Sigil sends"), ActionKind::Ghost)
                    .key("product-doc")
                    .role("link")
                    .cursor(rune::Cursor::Pointer)
                    .on_press_tx(|st: &mut State, tx: &mut Tx| super::tabs::open_url(st, tx, super::telemetry::PRIVACY_DOC)),
            ),
        )
}

/// "Not now" and "Share", equal in size and weight, each answering for good.
pub fn answer_buttons(u: &Ui, after: fn(&mut State)) -> El {
    let choice = |key: &'static str, label: &str, share: bool| {
        u.action(tr(label), ActionKind::Secondary)
            .key(key)
            .w(100.0)
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| {
                set(st, share);
                after(st);
            })
    };
    row()
        .gap(8.0)
        .center_y()
        .child(choice("product-not-now", "Not now", false))
        .child(choice("product-share", "Share", true))
}

/// The one-time question for installs that finished setup before it existed.
pub fn question(s: &State, u: &Ui) -> Option<El> {
    if !s.settings.telemetry.product.asking {
        return None;
    }
    Some(
        u.dialog_shell()
            .key("product-consent")
            .role("dialog")
            .label(tr(TITLE))
            .w(520.0)
            .padding(24.0)
            .gap(16.0)
            // An answer is required: no scrim press and no Escape close it.
            .modal(true, |_st: &mut State| {})
            .on_key(|_st: &mut State, ev: &KeyEvent, _| ev.key == Key::Escape)
            .child(text(tr(TITLE)).font(H1).color(u.ink))
            .child(consent_body(u))
            .child(row().w_full().pt(4.0).child(spacer()).child(answer_buttons(u, |_st: &mut State| {}))),
    )
}

/// Settings > Privacy > Help build Sigil.
pub fn privacy_section(s: &State, u: &Ui) -> El {
    let on = pd::allowed(&s.settings.telemetry.consent);
    let deleting = s.settings.telemetry.product.deleting.is_running();
    section(
        u,
        TITLE,
        vec![
            toggle_row(
                u,
                "settings-product",
                "Share product data",
                "Mods, Browse, play, performance and app use, to Sigil's own server, kept until you delete it. Off unless you turn it on.",
                on,
                |st| {
                    let share = !pd::allowed(&st.settings.telemetry.consent);
                    set(st, share);
                },
            ),
            setting_row(
                u,
                "settings-product-log",
                "What Sigil shared",
                detail(u, "The last records sent from this computer."),
                button(u, "settings-product-log-open", "Show", ActionKind::Secondary).on_press(open_log),
            ),
            setting_row(
                u,
                "settings-product-delete",
                "Delete my data",
                detail(u, "Deletes the records Sigil's server keeps from this computer, clears what waits to be sent and starts a new id. Totals already counted with no id stay."),
                super::disabled(
                    button(u, "settings-product-delete-run", if deleting { "Deleting" } else { "Delete" }, ActionKind::Secondary)
                        .on_press(delete),
                    deleting || s.prefs.offline,
                ),
            ),
        ],
    )
}

/// "What Sigil shared": the local log, newest first.
pub fn log_dialog(s: &State, u: &Ui) -> Option<El> {
    let lines = s.settings.telemetry.product.log.as_ref()?;
    let close = |st: &mut State| st.settings.telemetry.product.log = None;
    let body = if lines.is_empty() {
        text(tr("Nothing has been sent yet.")).font(BODY).color(u.muted)
    } else {
        text(lines.join("\n")).font(MONO).color(u.prose).wrap_anywhere().selectable(true).w_full()
    };
    Some(
        u.dialog_shell()
            .key("product-log")
            .role("dialog")
            .label(tr("What Sigil shared"))
            .w(620.0)
            .padding(24.0)
            .gap(14.0)
            .modal(true, close)
            .child(text(tr("What Sigil shared")).font(H1).color(u.ink))
            .child(
                text(fill(tr("The last {} records sent from this computer, newest first. Sigil keeps this list only here."), &[&pd::LOG_LIMIT.to_string()]))
                    .font(BODY)
                    .color(u.muted)
                    .wrap(),
            )
            .child(
                u.scroll_area(
                    col()
                        .key("product-log-text")
                        .w_full()
                        .h(320.0)
                        .padding(12.0)
                        .radius(crate::ui::r(4.0))
                        .fill(u.well)
                        .border(1.0, u.hair)
                        .scroll(rune::ScrollSpec { stick_bottom: false })
                        .child(body),
                ),
            )
            .child(
                row().w_full().center_y().child(spacer()).child(
                    u.action(tr("Close"), ActionKind::Secondary)
                        .key("product-log-close")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(close),
                ),
            ),
    )
}

/// Scene: the setup's privacy step, with Help build Sigil.
pub fn scene_onboarding(st: &mut State) {
    crate::app::scene_demo_installed(st);
    st.settings.extra.onboarding = Some(super::onboarding::Step::Privacy);
}

/// Scene: the one-time question for an install that finished setup before it existed.
pub fn scene_question(st: &mut State) {
    crate::screens::browse::scene_dashboard(st);
    st.settings.telemetry.loaded = true;
    st.settings.telemetry.product.asking = true;
}

/// Scene: Settings > Privacy with Help build Sigil on.
pub fn scene_privacy(st: &mut State) {
    super::scene_tab(st, super::Tab::Privacy);
    st.settings.telemetry.loaded = true;
    st.settings.telemetry.consent.product_asked = true;
    st.settings.telemetry.consent.product = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reason_codes_are_codes() {
        for error in ["Deadlock is running", "HTTP 500 while downloading", "", "The install worker stopped before reporting."] {
            let _ = reason_code(error);
        }
        assert_eq!(reason_code("Deadlock is running.").as_str(), "game_running");
    }

    #[test]
    fn nothing_is_recorded_in_a_test_run() {
        let st = State::default();
        assert!(!pd::allowed(&st.settings.telemetry.consent));
        record(&st, Record::Launch { modded: true });
        assert!(pd::global().is_none());
    }

    #[test]
    fn settings_codes_are_valid() {
        let st = State::default();
        let Record::Settings { theme, density, language, .. } = settings_record(&st) else { panic!() };
        assert_eq!(theme.as_str(), format!("{:?}", st.prefs.theme).to_lowercase());
        assert_eq!((density.as_str(), language.as_str()), ("default", "system"));
    }
}
