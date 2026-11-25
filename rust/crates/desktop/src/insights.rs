//! What "Help build Sigil" gives back (ADR 0017): "Players who use this also use", mod health
//! chips and frame rates on PCs like yours. They share, they get these.
//!
//! Rules this module keeps:
//!
//! - Nothing is asked of Sigil's server unless the player turned Help build Sigil on
//!   ([`may_ask`]): no consent, no request. Turning it off drops every answer, here and on disk.
//! - Nothing renders without data. Every reader returns `None` or an empty list until the server
//!   sent results that passed its floor, so a view adds no box, label or gap before that.
//! - Requests run one at a time on a worker, never on the UI thread, and each is asked at most
//!   once a run: one per opened mod for related mods, batches of up to 40 mods for health, one for
//!   frame rates. Answers are kept in memory and on disk for a day.
//! - The poll is cheap: it recomputes what to ask only when a fingerprint of the screens changes.

use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use deadlock_core::product_data::{self as pd, FpsExpectation, HealthChip, ModHealth, ModId};
use serde_json::{Value, json};

use crate::app::State;
use crate::jobs::Job;
use crate::model::BrowseCard;

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Answers are trusted this long, then asked again.
pub const KEEP_SECS: i64 = 24 * 60 * 60;
/// Related mods shown on a mod's details.
pub const RELATED_SHOWN: usize = 8;
/// Enabled mods whose related mods feed For you, per run.
const FOR_YOU_SOURCES: usize = 8;
const FILE_NAME: &str = "given.json";

/// One answer with the time it came, Unix seconds.
#[derive(Clone, Debug)]
struct Dated<T> {
    at: i64,
    value: T,
}

/// The frame rates answer and the hardware class it was asked for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FpsAnswer {
    pub gpu: String,
    pub cpu: Option<String>,
    pub height: u16,
    pub configs: Vec<FpsExpectation>,
}

/// What is kept on disk: answers only, no record of what the player did.
#[derive(Default)]
struct Disk {
    related: HashMap<u64, Dated<Vec<u64>>>,
    health: HashMap<u64, Dated<Option<ModHealth>>>,
    fps: Option<Dated<FpsAnswer>>,
    rising: Option<Dated<Vec<u64>>>,
}

impl Disk {
    fn to_json(&self) -> Value {
        let related: serde_json::Map<String, Value> =
            self.related.iter().map(|(id, d)| (id.to_string(), json!({ "at": d.at, "value": d.value }))).collect();
        let health: serde_json::Map<String, Value> =
            self.health.iter().map(|(id, d)| (id.to_string(), json!({ "at": d.at, "value": d.value }))).collect();
        let fps = self.fps.as_ref().map(|d| {
            json!({ "at": d.at, "gpu": d.value.gpu, "cpu": d.value.cpu, "height": d.value.height, "configs": d.value.configs })
        });
        let rising = self.rising.as_ref().map(|d| json!({ "at": d.at, "value": d.value }));
        json!({ "related": related, "health": health, "fps": fps, "rising": rising })
    }

    /// Read back entry by entry: an entry that does not parse is dropped, the rest survive.
    fn from_json(text: &str) -> Disk {
        let Ok(v) = serde_json::from_str::<Value>(text) else { return Disk::default() };
        let at = |e: &Value| e.get("at").and_then(Value::as_i64);
        let mut disk = Disk::default();
        for (id, e) in v.get("related").and_then(Value::as_object).into_iter().flatten() {
            let (Ok(id), Some(at), Some(list)) = (id.parse::<u64>(), at(e), e.get("value").and_then(Value::as_array)) else { continue };
            disk.related.insert(id, Dated { at, value: list.iter().filter_map(Value::as_u64).collect() });
        }
        for (id, e) in v.get("health").and_then(Value::as_object).into_iter().flatten() {
            let (Ok(id), Some(at)) = (id.parse::<u64>(), at(e)) else { continue };
            let value = e.get("value").cloned().and_then(|h| serde_json::from_value::<Option<ModHealth>>(h).ok()).flatten();
            disk.health.insert(id, Dated { at, value });
        }
        if let Some(e) = v.get("fps").filter(|e| !e.is_null())
            && let Some(at) = at(e)
        {
            let configs = e.get("configs").cloned().and_then(|c| serde_json::from_value::<Vec<FpsExpectation>>(c).ok()).unwrap_or_default();
            disk.fps = Some(Dated {
                at,
                value: FpsAnswer {
                    gpu: e.get("gpu").and_then(Value::as_str).unwrap_or_default().to_string(),
                    cpu: e.get("cpu").and_then(Value::as_str).map(str::to_string),
                    height: e.get("height").and_then(Value::as_u64).unwrap_or(0).min(u16::MAX as u64) as u16,
                    configs,
                },
            });
        }
        if let Some(e) = v.get("rising").filter(|e| !e.is_null())
            && let (Some(at), Some(list)) = (at(e), e.get("value").and_then(Value::as_array))
        {
            disk.rising = Some(Dated { at, value: list.iter().filter_map(Value::as_u64).collect() });
        }
        disk
    }
}

enum Ask {
    Related(u64),
    Health(Vec<u64>),
    Fps,
    Rising,
}

enum Answer {
    Related(u64, Vec<u64>),
    Health(Vec<u64>, Vec<ModHealth>, Option<u32>),
    Fps(Option<FpsAnswer>),
    Rising(Vec<u64>),
}

#[derive(Default)]
pub struct Insights {
    /// Mod id to related mod ids, best first.
    related: HashMap<u64, Dated<Vec<u64>>>,
    /// Mod id to its health; `None` once asked and nothing was published.
    health: HashMap<u64, Dated<Option<ModHealth>>>,
    fps: Option<Dated<FpsAnswer>>,
    /// Mods whose installs grew most among players who share, best first.
    rising: Option<Dated<Vec<u64>>>,
    /// Requests already made this run, so each is asked once.
    asked: HashSet<String>,
    queue: VecDeque<Ask>,
    job: Job<Answer>,
    load: Job<(Disk, Option<u32>)>,
    loaded: bool,
    /// Bumped whenever an answer lands, so dependants (For you) redo their work.
    pub generation: u64,
    /// What the wanted lists were last worked out from.
    fingerprint: u64,
    /// Cards for related mods, resolved once per answer from what Browse already holds.
    cards: HashMap<u64, BrowseCard>,
    /// The player's installed GameBanana mods, for the conflict chip.
    have: HashSet<u64>,
    /// The game build the player has now, read on a worker.
    pub build: Option<u32>,
    /// The Performance list sorted by expected frame rate instead of by tier.
    pub sort_fps: bool,
    /// The details mod whose related covers were last queued.
    queued_for: Option<u64>,
}

fn path() -> PathBuf {
    crate::data::data_dir().join("telemetry").join("product").join(FILE_NAME)
}

fn now() -> i64 {
    deadlock_core::gamebanana::now_unix()
}

fn fresh<T>(d: &Dated<T>, at: i64) -> bool {
    at - d.at < KEEP_SECS && d.at <= at + 60
}

/// May Sigil ask its server anything right now? Only with Help build Sigil on, online, in the
/// real app (never a demo, a headless snap or a test).
pub fn may_ask(st: &State) -> bool {
    pd::allowed(&st.settings.telemetry.consent)
        && st.settings.telemetry.loaded
        && !st.prefs.offline
        && !quiet(st)
}

fn quiet(st: &State) -> bool {
    rune::is_headless() || cfg!(test) || st.demo
}

/// The Browse grid's current page.
fn grid(st: &State) -> &[BrowseCard] {
    st.browse.page.as_ref().map(|p| p.mods.as_slice()).unwrap_or(&[])
}

// --- readers for the views -------------------------------------------------------------------

/// Up to [`RELATED_SHOWN`] related mods for `id` that pass the content gate (hidden mods, hidden
/// creators, the mature setting), never `id` itself. Empty until the server answered with some.
pub fn related_cards<'a>(s: &'a State, id: u64) -> Vec<&'a BrowseCard> {
    let Some(list) = s.insights.related.get(&id) else { return Vec::new() };
    list.value
        .iter()
        .filter(|other| **other != id)
        .filter_map(|other| s.insights.cards.get(other))
        .filter(|c| crate::screens::browse::content_allowed(s, c.id, c.nsfw, &c.author))
        .take(RELATED_SHOWN)
        .collect()
}

/// The health chips mod `id` earns now. Empty without strong reports. `updated` is the mod's
/// last update on GameBanana (Unix seconds, 0 when unknown): older reports do not count.
pub fn chips(s: &State, id: u64, updated: i64) -> Vec<HealthChip> {
    match s.insights.health.get(&id).and_then(|d| d.value.as_ref()) {
        Some(health) => health.chips(s.insights.build, &s.insights.have, updated),
        None => Vec::new(),
    }
}

/// The strongest chip, for a card that has room for one.
pub fn chip(s: &State, id: u64, updated: i64) -> Option<HealthChip> {
    chips(s, id, updated).into_iter().next()
}

/// A chip's label and the tooltip that says what it rests on. Words about reports, not facts.
pub fn chip_words(s: &State, chip: &HealthChip) -> (String, String) {
    use crate::screens::settings::i18n::{fill, tr};
    match chip {
        HealthChip::FailsToInstall { failed, attempts } => (
            tr("Often fails to install").to_string(),
            fill(tr("{} of {} installs reported by players who share data failed."), &[&failed.to_string(), &attempts.to_string()]),
        ),
        HealthChip::BrokenOnBuild { reports, .. } => (
            tr("Reported broken on the current game build").to_string(),
            fill(tr("{} players who share data reported problems with it on this game build."), &[&reports.to_string()]),
        ),
        HealthChip::ConflictsWith { other, reports } => {
            let name = mod_name(s, *other).unwrap_or_else(|| tr("a mod you have").to_string());
            (
                fill(tr("Often conflicts with {}"), &[&name]),
                fill(tr("{} players who share data and use both reported a conflict."), &[&reports.to_string()]),
            )
        }
    }
}

/// The chip as a small squared tag: amber for a warning, neutral for a conflict note. On art it
/// sits on a dark plate so it reads over any image. The tooltip names the label in full and what
/// it rests on. `max_w` truncates the label where a card has little room.
pub fn chip_el(s: &State, u: &crate::ui::Ui, key: String, chip: &HealthChip, on_image: bool, max_w: Option<f32>) -> rune::El {
    use rune::{Color, row, text};
    let (label, tip) = chip_words(s, chip);
    let warns = chip_warns(chip);
    // On a card's art the label is the short form; the tooltip keeps the whole sentence.
    let shown = if on_image { short_label(chip) } else { label.clone() };
    let ink = match (warns, on_image) {
        (true, _) => u.amber,
        (false, true) => Color::WHITE.alpha(0.9),
        (false, false) => u.muted,
    };
    let mut t = text(shown).font(crate::ui::MICRO).color(ink).truncate().min_w(0.0);
    if let Some(w) = max_w {
        t = t.max_w(w);
    }
    let mut b = row().key(key).h(20.0).px(6.0).center_y().radius(crate::ui::r(4.0)).min_w(0.0).help(format!("{label}. {tip}")).label(format!("{label}. {tip}"));
    b = if on_image {
        b.fill(Color::BLACK.alpha(0.66))
    } else if warns {
        b.fill(u.amber.alpha(0.12)).border(1.0, u.amber.alpha(0.40))
    } else {
        b.fill(u.over.alpha(0.08))
    };
    b.child(t)
}

/// The short label for a card or a tile.
pub fn short_label(chip: &HealthChip) -> String {
    use crate::screens::settings::i18n::tr;
    match chip {
        HealthChip::FailsToInstall { .. } => tr("Often fails to install"),
        HealthChip::BrokenOnBuild { .. } => tr("Reported broken"),
        HealthChip::ConflictsWith { .. } => tr("Often conflicts"),
    }
    .to_string()
}

/// Is the chip a warning (amber) or a note (neutral)?
pub fn chip_warns(chip: &HealthChip) -> bool {
    !matches!(chip, HealthChip::ConflictsWith { .. })
}

fn mod_name(s: &State, id: u64) -> Option<String> {
    s.installed
        .library
        .mods
        .iter()
        .find(|m| m.gamebanana_id == Some(id))
        .map(|m| m.name.clone())
        .filter(|n| !n.is_empty())
}

/// Frame rates on PCs like this one for a config, when other players measured it.
pub fn fps_for<'a>(s: &'a State, config_id: &str, version: &str) -> Option<&'a FpsExpectation> {
    let code = pd::Code::new(config_id)?;
    let version = pd::config_version(version);
    s.insights
        .fps
        .as_ref()?
        .value
        .configs
        .iter()
        .find(|e| e.config == code.as_str() && e.version == version)
}

/// Mods rising by real installs among players who share, best first, for Browse's Rising row
/// to blend with like velocity. Empty until the server answered with some.
#[allow(dead_code)] // Read by the Rising row once Browse blends it in.
pub fn rising(s: &State) -> &[u64] {
    s.insights.rising.as_ref().map(|d| d.value.as_slice()).unwrap_or(&[])
}

/// Any frame rate to show at all: the sort switch appears only then.
pub fn has_fps(s: &State) -> bool {
    s.insights.fps.as_ref().is_some_and(|d| !d.value.configs.is_empty())
}

/// Weight per candidate mod: how strongly players with the player's enabled mods also use it,
/// 0 to 1. Empty without answers.
pub fn related_weights(s: &State, enabled: &[u64]) -> HashMap<u64, f32> {
    let mut out: HashMap<u64, f32> = HashMap::new();
    for source in enabled {
        let Some(list) = s.insights.related.get(source) else { continue };
        for (rank, other) in list.value.iter().enumerate() {
            *out.entry(*other).or_default() += 1.0 / (1.0 + rank as f32 * 0.25);
        }
    }
    let max = out.values().copied().fold(0.0f32, f32::max);
    if max > 0.0 {
        for v in out.values_mut() {
            *v /= max;
        }
    }
    out
}

/// The player's enabled GameBanana mods, in slot order.
pub fn enabled_mods(s: &State) -> Vec<u64> {
    s.addons
        .addons
        .iter()
        .filter(|a| a.enabled)
        .filter_map(|a| crate::screens::settings::product::mod_of_slot(s, a.slot))
        .map(ModId::get)
        .collect()
}

// --- the poll ------------------------------------------------------------------------------

/// Load, ask, take answers. Cheap when nothing changed.
pub fn poll(st: &mut State) -> bool {
    if quiet(st) {
        return false;
    }
    if !pd::allowed(&st.settings.telemetry.consent) {
        return forget(st);
    }
    if !st.settings.telemetry.loaded {
        return false;
    }
    let mut changed = false;
    if !st.insights.loaded {
        if let Some(result) = st.insights.load.try_take() {
            st.insights.loaded = true;
            let (disk, build) = result.unwrap_or_default();
            let at = now();
            let i = &mut st.insights;
            i.build = build;
            i.related = disk.related.into_iter().filter(|(_, d)| fresh(d, at)).collect();
            i.health = disk.health.into_iter().filter(|(_, d)| fresh(d, at)).collect();
            i.fps = disk.fps.filter(|d| fresh(d, at));
            i.rising = disk.rising.filter(|d| fresh(d, at));
            i.generation += 1;
            i.fingerprint = 0;
            resolve_cards(st);
            changed = true;
        } else {
            let wake = st.wake.clone();
            let root = st.game_root.clone();
            st.insights.load.start(wake, move || {
                let disk = std::fs::read_to_string(path()).map(|t| Disk::from_json(&t)).unwrap_or_default();
                // The game build rides along with the first read: a file read, never on the UI thread.
                Ok((disk, read_build(root.as_deref())))
            });
            return false;
        }
    }
    if let Some(result) = st.insights.job.try_take() {
        if let Ok(answer) = result {
            take(st, answer);
            save(st);
        }
        changed = true;
    }
    let key = fingerprint(st);
    if key != st.insights.fingerprint {
        st.insights.fingerprint = key;
        plan(st);
        resolve_cards(st);
        queue_related_covers(st);
    }
    if st.insights.job.is_running() || !may_ask(st) {
        return changed;
    }
    if let Some(ask) = st.insights.queue.pop_front() {
        start(st, ask);
    }
    changed
}

/// Consent went off: drop every answer here and on disk at once.
fn forget(st: &mut State) -> bool {
    let i = &mut st.insights;
    let had = !i.related.is_empty() || !i.health.is_empty() || i.fps.is_some() || i.rising.is_some() || i.loaded;
    if !had {
        return false;
    }
    *i = Insights { generation: i.generation + 1, ..Insights::default() };
    let _ = std::thread::Builder::new().name("sigil-given-forget".into()).spawn(|| {
        let _ = std::fs::remove_file(path());
    });
    true
}

/// A cheap fingerprint of what the screens show: the open mod, the grid page, the landing, the
/// library and the screen.
fn fingerprint(st: &State) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    st.browse_view.details.as_ref().map(|d| d.id).hash(&mut h);
    grid(st).len().hash(&mut h);
    grid(st).first().map(|c| c.id).hash(&mut h);
    st.browse_view.shelves.feeds.as_ref().map(|f| (f.recent.len(), f.liked.len())).hash(&mut h);
    st.browse_view.shelves.for_you.picks.len().hash(&mut h);
    st.browse_view.mirror.as_ref().map(|m| m.entries.len()).hash(&mut h);
    st.installed.library.mods.len().hash(&mut h);
    for a in &st.addons.addons {
        (a.slot, a.enabled).hash(&mut h);
    }
    std::mem::discriminant(&st.screen).hash(&mut h);
    st.insights.generation.hash(&mut h);
    h.finish()
}

/// Work out what to ask next, in order: the open mod's related mods, health for what is on
/// screen and installed, frame rates on the Performance page, then related mods of the
/// player's enabled mods for For you.
fn plan(st: &mut State) {
    let at = now();
    st.insights.have = st.installed.library.mods.iter().filter_map(|m| m.gamebanana_id).collect();
    let mut queue = VecDeque::new();
    let fresh_related = |st: &State, id: u64| st.insights.related.get(&id).is_some_and(|d| fresh(d, at));
    if let Some(id) = st.browse_view.details.as_ref().map(|d| d.id)
        && !fresh_related(st, id)
        && !st.insights.asked.contains(&format!("r:{id}"))
    {
        queue.push_back(Ask::Related(id));
    }
    // Health: the open mod, the grid page, the landing, then the library.
    let mut wanted: Vec<u64> = Vec::new();
    wanted.extend(st.browse_view.details.as_ref().map(|d| d.id));
    wanted.extend(grid(st).iter().take(60).map(|c| c.id));
    if let Some(feeds) = &st.browse_view.shelves.feeds {
        wanted.extend(feeds.recent.iter().chain(feeds.liked.iter()).take(40).map(|c| c.id));
    }
    wanted.extend(st.browse_view.shelves.for_you.picks.iter().take(24).map(|c| c.id));
    wanted.extend(st.installed.library.mods.iter().filter_map(|m| m.gamebanana_id));
    let mut seen = HashSet::new();
    let need: Vec<u64> = wanted
        .into_iter()
        .filter(|id| ModId::new(*id).is_some() && seen.insert(*id))
        .filter(|id| !st.insights.health.get(id).is_some_and(|d| fresh(d, at)) && !st.insights.asked.contains(&format!("h:{id}")))
        .collect();
    for batch in need.chunks(pd::MAX_HEALTH_MODS) {
        queue.push_back(Ask::Health(batch.to_vec()));
    }
    if matches!(st.screen, crate::model::Screen::Performance)
        && !st.insights.fps.as_ref().is_some_and(|d| fresh(d, at))
        && !st.insights.asked.contains("f")
        && st.game_root.is_some()
    {
        queue.push_back(Ask::Fps);
    }
    if matches!(st.screen, crate::model::Screen::Browse)
        && !st.insights.rising.as_ref().is_some_and(|d| fresh(d, at))
        && !st.insights.asked.contains("g")
    {
        queue.push_back(Ask::Rising);
    }
    for id in enabled_mods(st).into_iter().take(FOR_YOU_SOURCES) {
        if !fresh_related(st, id) && !st.insights.asked.contains(&format!("r:{id}")) {
            queue.push_back(Ask::Related(id));
        }
    }
    st.insights.queue = queue;
}

fn start(st: &mut State, ask: Ask) {
    let wake = st.wake.clone();
    match ask {
        Ask::Related(id) => {
            st.insights.asked.insert(format!("r:{id}"));
            let Some(mod_id) = ModId::new(id) else { return };
            st.insights.job.start(wake, move || {
                let list = pd::related(mod_id, VERSION).map_err(|e| e.to_string())?;
                Ok(Answer::Related(id, list.into_iter().map(ModId::get).collect()))
            });
        }
        Ask::Health(ids) => {
            for id in &ids {
                st.insights.asked.insert(format!("h:{id}"));
            }
            let root = st.game_root.clone();
            st.insights.job.start(wake, move || {
                let mods: Vec<ModId> = ids.iter().filter_map(|id| ModId::new(*id)).collect();
                let list = pd::health(&mods, VERSION).map_err(|e| e.to_string())?;
                let build = read_build(root.as_deref());
                Ok(Answer::Health(ids, list, build))
            });
        }
        Ask::Rising => {
            st.insights.asked.insert("g".to_string());
            st.insights.job.start(wake, move || {
                let list = pd::rising(VERSION).map_err(|e| e.to_string())?;
                Ok(Answer::Rising(list.into_iter().map(ModId::get).collect()))
            });
        }
        Ask::Fps => {
            st.insights.asked.insert("f".to_string());
            let root = st.game_root.clone();
            st.insights.job.start(wake, move || {
                let machine = deadlock_core::benchmark::system::read_machine();
                let Some(gpu) = machine.gpu.as_deref().and_then(pd::model_name) else { return Ok(Answer::Fps(None)) };
                let cpu = machine.cpu.as_deref().and_then(pd::model_name);
                let steam = deadlock_core::discovery::default_steam_roots();
                let height = root
                    .as_deref()
                    .map(|r| deadlock_core::benchmark::system::read_settings(r, &steam))
                    .and_then(|v| v.resolution())
                    .and_then(|r| crate::screens::settings::product::parse_resolution(&r))
                    .map(|(_, h)| h.min(u16::MAX as u32) as u16)
                    .filter(|h| *h >= 200);
                let Some(height) = height else { return Ok(Answer::Fps(None)) };
                let configs = pd::fps(&gpu, cpu.as_ref(), height, VERSION).map_err(|e| e.to_string())?;
                Ok(Answer::Fps(Some(FpsAnswer { gpu: gpu.as_str().to_string(), cpu: cpu.map(|c| c.as_str().to_string()), height, configs })))
            });
        }
    }
}

/// The game build the player has now (`steam.inf`). A file read: workers only.
fn read_build(root: Option<&std::path::Path>) -> Option<u32> {
    root.and_then(deadlock_core::host::read_build).and_then(|b| b.trim().parse().ok())
}

fn take(st: &mut State, answer: Answer) {
    let at = now();
    let i = &mut st.insights;
    match answer {
        Answer::Related(id, list) => {
            i.related.insert(id, Dated { at, value: list });
        }
        Answer::Health(ids, list, build) => {
            if build.is_some() {
                i.build = build;
            }
            let mut by_id: HashMap<u64, ModHealth> = list.into_iter().map(|h| (h.mod_id, h)).collect();
            for id in ids {
                i.health.insert(id, Dated { at, value: by_id.remove(&id) });
            }
        }
        Answer::Rising(list) => {
            i.rising = Some(Dated { at, value: list });
        }
        Answer::Fps(answer) => {
            if let Some(answer) = answer {
                i.fps = Some(Dated { at, value: answer });
            }
        }
    }
    i.generation += 1;
}

/// Write the answers on a worker.
fn save(st: &State) {
    let disk = Disk {
        related: st.insights.related.clone(),
        health: st.insights.health.clone(),
        fps: st.insights.fps.clone(),
        rising: st.insights.rising.clone(),
    };
    let _ = std::thread::Builder::new().name("sigil-given-save".into()).spawn(move || {
        let path = path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(bytes) = serde_json::to_vec(&disk.to_json()) {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, bytes).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    });
}

/// Cards for every related mod known, from what Browse already holds: the grid, the landing,
/// For you and the saved catalog. A mod none of them knows is left out.
fn resolve_cards(st: &mut State) {
    let wanted: HashSet<u64> = st.insights.related.values().flat_map(|d| d.value.iter().copied()).collect();
    let mut cards: HashMap<u64, BrowseCard> = HashMap::new();
    if wanted.is_empty() {
        st.insights.cards = cards;
        return;
    }
    let mut add = |c: &BrowseCard| {
        if wanted.contains(&c.id) && c.thumbnail_url.is_some() && !c.obsolete && !cards.contains_key(&c.id) {
            cards.insert(c.id, crate::screens::browse::store::clone_card(c));
        }
    };
    for c in grid(st) {
        add(c);
    }
    if let Some(feeds) = &st.browse_view.shelves.feeds {
        for c in feeds.recent.iter().chain(feeds.liked.iter()) {
            add(c);
        }
    }
    for c in &st.browse_view.shelves.for_you.all {
        add(c);
    }
    if let Some(mirror) = &st.browse_view.mirror {
        for entry in mirror.entries.iter().filter(|e| wanted.contains(&e.id)) {
            add(&crate::screens::browse::store::mirror_card(entry));
        }
    }
    st.insights.cards = cards;
}

/// The open mod's related covers, through the content gate, once per mod.
fn queue_related_covers(st: &mut State) {
    let Some(id) = st.browse_view.details.as_ref().map(|d| d.id) else { return };
    let urls: Vec<String> = related_cards(st, id).iter().filter_map(|c| c.thumbnail_url.clone()).collect();
    if urls.is_empty() || st.insights.queued_for == Some(id) {
        return;
    }
    st.insights.queued_for = Some(id);
    crate::data::queue_images(st, urls, false);
}

// --- fixtures for scenes and tests -------------------------------------------------------------

/// Fixture answers, for scenes and tests: never fetched.
pub fn set_related(st: &mut State, id: u64, related: Vec<u64>) {
    st.insights.related.insert(id, Dated { at: now(), value: related });
    st.insights.generation += 1;
    resolve_cards(st);
}

pub fn set_health(st: &mut State, health: ModHealth) {
    st.insights.health.insert(health.mod_id, Dated { at: now(), value: Some(health) });
    st.insights.have = st.installed.library.mods.iter().filter_map(|m| m.gamebanana_id).collect();
    st.insights.generation += 1;
}

pub fn set_fps(st: &mut State, configs: Vec<FpsExpectation>) {
    st.insights.fps = Some(Dated { at: now(), value: FpsAnswer { configs, ..FpsAnswer::default() } });
    st.insights.generation += 1;
}

// --- scenes ------------------------------------------------------------------------------------

/// Scene `details-related`: a mod's details with "Players who use this also use" filled from
/// fixture answers (other mods of the fixture grid).
pub fn scene_details_related(st: &mut State) {
    crate::screens::browse::scene_details(st);
    // A short page, so the row is in view without scrolling.
    if let Some(d) = st.browse_view.details.as_mut() {
        if let Some(p) = d.profile.as_mut() {
            p.text = p.text.lines().next().unwrap_or_default().to_string();
            p.images.truncate(1);
        }
        d.updates = None;
        d.comments = None;
    }
    let Some(id) = st.browse_view.details.as_ref().map(|d| d.id) else { return };
    let others: Vec<u64> = grid(st).iter().filter(|c| c.id != id && c.thumbnail_url.is_some()).map(|c| c.id).take(RELATED_SHOWN).collect();
    set_related(st, id, others);
    if rune::is_headless() {
        let urls: Vec<String> = related_cards(st, id).iter().filter_map(|c| c.thumbnail_url.clone()).collect();
        for url in urls {
            if let Some(bytes) = crate::data::fetch_thumb_bytes(&url) {
                st.pending_thumbs.push((url, bytes));
            }
        }
    }
}

/// Scene `health-chips`: the Browse grid with the three chips on its first cards.
pub fn scene_health_chips(st: &mut State) {
    crate::screens::browse::scene_with(st, |f| f.order = Some(deadlock_core::gamebanana::BrowseOrder::MostLiked));
    let key = crate::screens::browse::query_key(st, &crate::screens::browse::current_query(st));
    if let Some(page) = st.browse.page.as_mut() {
        page.key = key;
    }
    st.insights.build = Some(6100);
    let ids: Vec<u64> = grid(st).iter().filter(|c| !c.nsfw).map(|c| c.id).take(4).collect();
    let [a, b, c, other] = ids[..] else { return };
    use deadlock_core::product_data::{BuildHealth, ConflictHealth, InstallHealth};
    set_health(st, ModHealth { mod_id: a, install: Some(InstallHealth { failed: 18, attempts: 52, until: 0 }), ..ModHealth::default() });
    set_health(st, ModHealth { mod_id: b, broken: vec![BuildHealth { build: 6100, reports: 14, users: 41, until: 0 }], ..ModHealth::default() });
    set_health(st, ModHealth { mod_id: c, conflicts: vec![ConflictHealth { mod_id: other, reports: 12, together: 20, until: 0 }], ..ModHealth::default() });
    st.insights.have.insert(other);
}

/// Scene `performance-expected`: the Performance list with frame rates from players with PCs
/// like this one on three configs, and the sort switch.
pub fn scene_performance_expected(st: &mut State) {
    crate::screens::performance::scene(st, "performance");
    let Some(catalog) = st.performance.catalog.as_ref() else { return };
    let expected: Vec<FpsExpectation> = catalog
        .configs
        .iter()
        .take(3)
        .enumerate()
        .filter_map(|(n, c)| {
            Some(FpsExpectation {
                config: pd::Code::new(c.id())?.as_str().to_string(),
                version: pd::config_version(&c.version),
                fps: 148 - 17 * n as u32,
                low: 92 - 11 * n as u32,
                installs: 31 - 7 * n as u32,
            })
        })
        .collect();
    set_fps(st, expected);
}

#[cfg(test)]
mod tests {
    use super::*;
    use deadlock_core::product_data::{BuildHealth, ConflictHealth, InstallHealth};

    fn consenting() -> State {
        let mut st = State::default();
        st.settings.telemetry.loaded = true;
        st.settings.telemetry.consent.product_asked = true;
        st.settings.telemetry.consent.product = true;
        st
    }

    #[test]
    fn nothing_is_asked_without_consent() {
        let mut st = State::default();
        st.settings.telemetry.loaded = true;
        st.browse_view.details = Some(crate::screens::browse::Details::from_card(&crate::screens::browse::top::card_of(&deadlock_core::gamebanana::TopSub {
            id: 42,
            name: "A".into(),
            ..Default::default()
        })));
        assert!(!may_ask(&st));
        for _ in 0..3 {
            poll(&mut st);
        }
        assert!(!st.insights.job.is_running());
        assert!(!st.insights.load.is_running());
        assert!(st.insights.asked.is_empty());
        // With consent, a test run is still quiet: nothing leaves a test.
        let st = consenting();
        assert!(!may_ask(&st));
    }

    #[test]
    fn nothing_renders_without_data() {
        let st = consenting();
        assert!(related_cards(&st, 42).is_empty());
        assert!(chips(&st, 42, 0).is_empty());
        assert!(fps_for(&st, "optilock", "3").is_none());
        assert!(!has_fps(&st));
        assert!(rising(&st).is_empty());
        assert!(related_weights(&st, &[42]).is_empty());
    }

    #[test]
    fn related_cards_respect_the_content_gate() {
        let mut st = consenting();
        let mk = |id: u64, nsfw: bool| {
            let mut c = crate::screens::browse::top::card_of(&deadlock_core::gamebanana::TopSub {
                id,
                name: format!("Mod {id}"),
                author: format!("maker{id}"),
                image_url: Some(format!("https://images.gamebanana.com/{id}.jpg")),
                ..Default::default()
            });
            c.nsfw = nsfw;
            c
        };
        st.browse_view.shelves.feeds =
            Some(crate::screens::browse::shelf::Feeds { recent: vec![mk(1, false), mk(2, true), mk(3, false), mk(42, false)], liked: Vec::new(), featured: Vec::new() });
        st.prefs.mature = crate::prefs::Mature::Hide;
        set_related(&mut st, 42, vec![42, 1, 2, 3, 99]);
        let ids: Vec<u64> = related_cards(&st, 42).iter().map(|c| c.id).collect();
        // Not itself, not the mature one while mature is hidden, not one Browse does not know.
        assert_eq!(ids, vec![1, 3]);
    }

    #[test]
    fn chips_show_only_strong_reports() {
        let mut st = consenting();
        st.insights.build = Some(6100);
        set_health(
            &mut st,
            ModHealth {
                mod_id: 7,
                install: Some(InstallHealth { failed: 9, attempts: 10, until: 0 }),
                broken: vec![BuildHealth { build: 6100, reports: 12, users: 30, until: 0 }],
                conflicts: vec![ConflictHealth { mod_id: 8, reports: 15, together: 20, until: 0 }],
            },
        );
        let chips = chips(&st, 7, 0);
        // Nine failures are under the floor; the conflict is with a mod the player lacks.
        assert_eq!(chips, vec![HealthChip::BrokenOnBuild { reports: 12, users: 30 }]);
        let (label, tip) = chip_words(&st, &chips[0]);
        assert_eq!(label, "Reported broken on the current game build");
        assert!(tip.starts_with("12 players"), "{tip}");
        st.insights.build = Some(6101);
        assert!(super::chips(&st, 7, 0).is_empty());
    }

    #[test]
    fn frame_rates_match_config_and_version() {
        let mut st = consenting();
        set_fps(&mut st, vec![FpsExpectation { config: "opti_lock".into(), version: 3, fps: 120, low: 70, installs: 14 }]);
        assert_eq!(fps_for(&st, "opti-lock", "v3.1").map(|e| e.fps), Some(120));
        assert!(fps_for(&st, "opti-lock", "v4").is_none());
        assert!(has_fps(&st));
    }

    #[test]
    fn answers_survive_the_disk_and_drop_what_does_not_parse() {
        let mut disk = Disk::default();
        disk.rising = Some(Dated { at: 5, value: vec![600, 700] });
        disk.related.insert(1, Dated { at: 5, value: vec![2] });
        let back = Disk::from_json(&disk.to_json().to_string());
        assert_eq!(back.rising.map(|d| d.value), Some(vec![600, 700]));
        assert_eq!(back.related.get(&1).map(|d| d.value.clone()), Some(vec![2]));
        assert!(Disk::from_json("not json").rising.is_none());
    }

    #[test]
    fn turning_sharing_off_forgets_every_answer() {
        let mut st = consenting();
        set_related(&mut st, 1, vec![2]);
        st.settings.telemetry.consent.product = false;
        // The poll is quiet in tests; the forgetting itself is checked directly.
        assert!(forget(&mut st));
        assert!(st.insights.related.is_empty());
        assert!(!forget(&mut st));
    }

    #[test]
    fn related_weights_favour_mods_tied_to_many_of_yours() {
        let mut st = consenting();
        set_related(&mut st, 1, vec![10, 11]);
        set_related(&mut st, 2, vec![10, 12]);
        let w = related_weights(&st, &[1, 2]);
        assert_eq!(w.get(&10).copied(), Some(1.0));
        assert!(w[&11] < 1.0 && w[&12] < w[&11] + 0.01);
    }
}
