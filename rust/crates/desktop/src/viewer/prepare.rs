//! Export jobs: "show this hero with these mods" turned into files the viewport opens.
//!
//! A screen asks for a viewport under a key ([`request`]): a hero as the player has it against
//! the unmodded game (Locker), one mod's effect on a hero, on or off (Installed, a mod's
//! details, the Locker's skin rows), or the Hideout with the player's mods. A worker thread
//! runs `deadlock_core::viewer::export_hero` / `export_hideout` into `<data>/viewer-cache`
//! (read only on the game folder), reports its stage, and the poll opens the viewport when the
//! files are ready. A second ask with unchanged mods is a cache hit and opens at once.
//!
//! Every ask has an owner (the Locker's hero page, the preview dialog, the full-screen
//! Hideout). When the owner goes away, or another hero is picked under the same key, the
//! export is cancelled and the viewport closed. When the installed mods change (a skin turned
//! on in the Locker) an open ask is made again, so the view follows the game.
//!
//! Demo mode and headless scenes use the bundled stand-in hero and room instead of a game.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

use deadlock_core::import::Library;
use deadlock_core::viewer::{self as core, HeroExportRequest, HeroSidecar, HideoutExportRequest, Variant as Look};

use super::{CompareMode, Source, Status, Variant};
use crate::app::State;
use crate::model::Screen;
use crate::screens::settings::i18n::{fill, tr};

/// What a viewport shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Ask {
    /// The hero with the player's enabled mods, against the unmodded game.
    Hero { hero: String },
    /// One mod's effect on a hero: with that slot (switched on or not) against the hero as it is
    /// without it.
    Mod { hero: String, slot: u16, name: String },
    /// The Hideout with the player's mods and this hero at the spawn.
    Hideout { hero: Option<String> },
}

/// Who keeps a viewport open. When the owner is gone the export is cancelled and the view closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    /// The Locker's hero page with the 3D view on.
    Locker,
    /// The Preview in 3D dialog.
    Dialog,
    /// Shown only full screen (the Hideout).
    FullScreen,
}

/// An ask that is open, pending or failed, kept to refresh it and to know its owner.
pub struct Asked {
    pub ask: Ask,
    pub owner: Owner,
    pub title: String,
    /// The installed mods when it was asked (slot, size, on).
    sig: Vec<(u16, u64, bool)>,
    /// A picture to show when the machine has no GPU (an image cache key).
    pub still: Option<String>,
}

/// The export running for a key.
pub struct Pending {
    cancel: Arc<AtomicBool>,
    rx: Receiver<Msg>,
    /// The stage line, an English key for `tr`, its hole, and the step of steps.
    pub stage: &'static str,
    pub arg: String,
    pub step: (u8, u8),
    /// How far the current step is, 0 to 1 (the export's own progress report).
    pub fraction: f32,
}

impl Pending {
    /// The whole job's share done, over every step.
    pub fn overall(&self) -> f32 {
        let (at, of) = (self.step.0.max(1) as f32, self.step.1.max(1) as f32);
        ((at - 1.0 + self.fraction.clamp(0.0, 1.0)) / of).clamp(0.0, 1.0)
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Why a key shows no view.
#[derive(Clone, Debug, PartialEq)]
pub enum Problem {
    /// No Deadlock folder: nothing to read.
    NoGame,
    /// The export failed, with its error.
    Failed(String),
}

enum Msg {
    Stage(&'static str, String, (u8, u8)),
    /// The current step's export reported this share done.
    Progress(f32),
    Done(Result<Ready, String>),
}

/// What the worker made.
pub struct Ready {
    pub source: Source,
    pub before: Option<Source>,
    pub notes: Vec<String>,
    pub labels: [&'static str; 2],
}

/// Labels of the compare buttons for a mod preview.
pub const MOD_LABELS: [&str; 2] = ["With this mod", "Without this mod"];
/// The cache folder's byte budget: the least recently used entries past it are removed.
pub const CACHE_BUDGET: u64 = 2 * 1024 * 1024 * 1024;

pub fn cache_dir() -> PathBuf {
    crate::data::data_dir().join("viewer-cache")
}

/// The installed mods, as a change detector for open asks.
fn mods_sig(s: &State) -> Vec<(u16, u64, bool)> {
    s.addons.addons.iter().map(|a| (a.slot, a.size_bytes, a.enabled)).collect()
}

/// A hero's display name for a codename or a name.
pub fn hero_name(hero: &str) -> String {
    deadlock_core::locker::roster_hero(hero)
        .or_else(|| deadlock_core::locker::hero_for_codename(hero))
        .map(|h| h.name.to_string())
        .unwrap_or_else(|| hero.to_string())
}

fn title_of(ask: &Ask) -> String {
    match ask {
        Ask::Hero { hero } => hero_name(hero),
        Ask::Mod { hero, name, .. } => fill(tr("{} on {}"), &[name, &hero_name(hero)]),
        Ask::Hideout { .. } => tr("Hideout").to_string(),
    }
}

/// True when views use the bundled stand-ins (demo mode, headless scenes).
pub fn stand_in(s: &State) -> bool {
    s.demo || s.viewers.stand_in
}

/// Opens (or replaces) the viewport `key` for `ask`. Cancels whatever ran under the key.
pub fn request(s: &mut State, key: &str, ask: Ask, owner: Owner, still: Option<String>) {
    super::close(s, key);
    let title = title_of(&ask);
    s.viewers.asks.insert(key.to_string(), Asked { ask: ask.clone(), owner, title: title.clone(), sig: mods_sig(s), still });
    if owner == Owner::FullScreen {
        s.viewers.fullscreen = Some(key.to_string());
    }
    if stand_in(s) {
        open_stand_in(s, key, &title, &ask);
        return;
    }
    let Some(game) = s.game_root.clone().filter(|root| root.join("game").join("citadel").is_dir()) else {
        s.viewers.problems.insert(key.to_string(), Problem::NoGame);
        return;
    };
    let enabled = match &ask {
        Ask::Mod { slot, .. } => s.addons.addons.iter().find(|a| a.slot == *slot).map(|a| a.enabled),
        _ => None,
    };
    let work = Work { game, cache: cache_dir(), library: s.installed.library.clone(), ask, enabled };
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let wake = s.wake.clone();
    let spawned = std::thread::Builder::new().name("sigil-viewer-export".into()).spawn(move || {
        let out = Out { tx: &tx, wake: wake.as_ref() };
        let result = work.run(&flag, &out);
        if !flag.load(Ordering::Relaxed) {
            out.send(Msg::Done(result));
        }
    });
    match spawned {
        Ok(_) => {
            let (stage, arg) = first_stage(&work_ask(s, key));
            s.viewers.pending.insert(key.to_string(), Pending { cancel, rx, stage, arg, step: (0, 0), fraction: 0.0 });
        }
        Err(e) => {
            s.viewers.problems.insert(key.to_string(), Problem::Failed(e.to_string()));
        }
    }
}

fn work_title(s: &State, key: &str) -> String {
    s.viewers.asks.get(key).map(|a| a.title.clone()).unwrap_or_default()
}

fn work_ask(s: &State, key: &str) -> Option<Ask> {
    s.viewers.asks.get(key).map(|a| a.ask.clone())
}

/// The stage line before the worker reports its first.
fn first_stage(ask: &Option<Ask>) -> (&'static str, String) {
    match ask {
        Some(Ask::Hero { hero }) | Some(Ask::Mod { hero, .. }) => ("Reading your mods for {}", hero_name(hero)),
        _ => ("Building the Hideout with your mods", String::new()),
    }
}

/// A hold-still pending entry for headless scenes: the loading panel at a given stage.
pub fn stage_loading(s: &mut State, key: &str, ask: Ask, owner: Owner, stage: &'static str, step: (u8, u8)) {
    super::close(s, key);
    let title = title_of(&ask);
    let arg = first_stage(&Some(ask.clone())).1;
    s.viewers.asks.insert(key.to_string(), Asked { ask, owner, title, sig: mods_sig(s), still: None });
    if owner == Owner::FullScreen {
        s.viewers.fullscreen = Some(key.to_string());
    }
    let (_tx, rx) = mpsc::channel();
    s.viewers.pending.insert(key.to_string(), Pending { cancel: Arc::new(AtomicBool::new(false)), rx, stage, arg, step, fraction: 0.42 });
}

/// A failed or game-less key for headless scenes.
pub fn stage_problem(s: &mut State, key: &str, ask: Ask, owner: Owner, problem: Problem) {
    super::close(s, key);
    let title = title_of(&ask);
    s.viewers.asks.insert(key.to_string(), Asked { ask, owner, title, sig: mods_sig(s), still: None });
    s.viewers.problems.insert(key.to_string(), problem);
}

/// A view whose render thread is loading or failed, for headless scenes.
pub fn stage_view(s: &mut State, key: &str, ask: Ask, owner: Owner, status: Status, progress: Option<sigil_viewer::LoadProgress>) {
    super::close(s, key);
    let title = title_of(&ask);
    s.viewers.asks.insert(key.to_string(), Asked { ask, owner, title: title.clone(), sig: mods_sig(s), still: None });
    if owner == Owner::FullScreen {
        s.viewers.fullscreen = Some(key.to_string());
    }
    super::stage_status(s, key, &title, status, progress);
}

fn open_stand_in(s: &mut State, key: &str, title: &str, ask: &Ask) {
    match ask {
        Ask::Hero { .. } => {
            super::open_hero(s, key, title, Source::StandIn(Variant::Modded), Some(Source::StandIn(Variant::Stock)));
        }
        Ask::Mod { .. } => {
            super::open_hero(s, key, title, Source::StandIn(Variant::Modded), Some(Source::StandIn(Variant::Stock)));
            if let Some(v) = s.viewers.map.get_mut(key) {
                v.labels = MOD_LABELS;
            }
        }
        Ask::Hideout { .. } => {
            let dir = PathBuf::from(&s.paths.data).join("viewer-demo").join("hideout");
            match sigil_viewer::fixture::write_hideout(&dir, Variant::Modded).and_then(|scene| sigil_viewer::fixture_room::upgrade_hideout(&scene).map(|_| scene)) {
                Ok(scene) => super::open_scene(s, key, title, scene, None),
                Err(e) => {
                    s.viewers.problems.insert(key.to_string(), Problem::Failed(e.to_string()));
                }
            }
        }
    }
}

struct Out<'a> {
    tx: &'a Sender<Msg>,
    wake: Option<&'a rune::Waker>,
}

impl Out<'_> {
    fn send(&self, msg: Msg) {
        let _ = self.tx.send(msg);
        if let Some(wake) = self.wake {
            wake();
        }
    }

    fn stage(&self, line: &'static str, arg: &str, step: (u8, u8)) {
        self.send(Msg::Stage(line, arg.to_string(), step));
    }

    /// A progress callback for an export: reports whole percents only, so the UI wakes a
    /// hundred times at most per step.
    fn meter(&self) -> impl Fn(f32) + Sync + '_ {
        let last = std::sync::atomic::AtomicU32::new(u32::MAX);
        move |f: f32| {
            let percent = (f.clamp(0.0, 1.0) * 100.0) as u32;
            if last.swap(percent, Ordering::Relaxed) != percent {
                self.send(Msg::Progress(percent as f32 / 100.0));
            }
        }
    }
}

/// Everything the worker needs, owned.
struct Work {
    game: PathBuf,
    cache: PathBuf,
    library: Library,
    ask: Ask,
    /// For a mod: whether its slot is switched on now.
    enabled: Option<bool>,
}

impl Work {
    fn run(&self, cancel: &AtomicBool, out: &Out) -> Result<Ready, String> {
        let result = match &self.ask {
            Ask::Hero { hero } => self.hero(hero, cancel, out),
            Ask::Mod { hero, slot, name } => self.one_mod(hero, *slot, name, cancel, out),
            Ask::Hideout { hero } => self.hideout(hero.as_deref(), cancel, out),
        };
        result.map_err(|e| e.to_string())
    }

    #[allow(clippy::too_many_arguments)]
    fn export(&self, hero: &str, look: Look, extra: &[u16], exclude: &[u16], cancel: &AtomicBool, out: &Out) -> deadlock_core::Result<core::Exported> {
        let meter = out.meter();
        let mut request = HeroExportRequest::new(&self.game, &self.cache, hero, look);
        request.extra_slots = extra;
        request.exclude_slots = exclude;
        request.library = Some(&self.library);
        request.progress = Some(&meter);
        core::export_hero_with(&request, cancel)
    }

    fn hero(&self, hero: &str, cancel: &AtomicBool, out: &Out) -> deadlock_core::Result<Ready> {
        let name = hero_name(hero);
        out.stage("Preparing {} with your mods", &name, (1, 2));
        let after = self.export(hero, Look::Modded, &[], &[], cancel, out)?;
        out.stage("Preparing {} without mods", &name, (2, 2));
        let before = self.export(hero, Look::Original, &[], &[], cancel, out)?;
        let mut notes = hidden_changes(&after.sidecar);
        let before = if before.key == after.key {
            notes.insert(0, fill(tr("No installed mod changes {}."), &[&name]));
            None
        } else {
            Some(hero_source(&before))
        };
        Ok(Ready { source: hero_source(&after), before, notes, labels: super::DEFAULT_LABELS })
    }

    fn one_mod(&self, hero: &str, slot: u16, mod_name: &str, cancel: &AtomicBool, out: &Out) -> deadlock_core::Result<Ready> {
        let name = hero_name(hero);
        let on = self.enabled.unwrap_or(false);
        out.stage("Preparing {} with this mod", &name, (1, 2));
        let after = if on { self.export(hero, Look::Modded, &[], &[], cancel, out)? } else { self.export(hero, Look::Modded, &[slot], &[], cancel, out)? };
        out.stage("Preparing {} without this mod", &name, (2, 2));
        let before = if on { self.export(hero, Look::Modded, &[], &[slot], cancel, out)? } else { self.export(hero, Look::Modded, &[], &[], cancel, out)? };
        let mut notes = Vec::new();
        if !on {
            notes.push(tr("This mod is off: you see it as it would look switched on.").to_string());
        }
        notes.extend(hidden_changes(&after.sidecar));
        let before = if before.key == after.key {
            notes.insert(0, fill(tr("{} does not change how {} looks."), &[mod_name, &name]));
            None
        } else {
            Some(hero_source(&before))
        };
        Ok(Ready { source: hero_source(&after), before, notes, labels: MOD_LABELS })
    }

    fn hideout(&self, hero: Option<&str>, cancel: &AtomicBool, out: &Out) -> deadlock_core::Result<Ready> {
        out.stage("Building the Hideout with your mods", "", (1, 1));
        let meter = out.meter();
        let mut request = HideoutExportRequest::new(&self.game, &self.cache, Look::Modded);
        request.hero = hero;
        request.library = Some(&self.library);
        request.progress = Some(&meter);
        let scene = core::export_hideout_with(&request, cancel)?;
        Ok(Ready { source: Source::Scene { scene, hero: None }, before: None, notes: Vec::new(), labels: super::DEFAULT_LABELS })
    }
}

fn hero_source(e: &core::Exported) -> Source {
    Source::Hero { glb: e.glb.clone(), sidecar: Some(e.sidecar.clone()) }
}

/// Deadlock Mod Manager's warning: a mod in the stack none of whose files the view draws.
fn hidden_changes(sidecar: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(sidecar) else { return Vec::new() };
    let Ok(side) = serde_json::from_str::<HeroSidecar>(&text) else { return Vec::new() };
    hidden_in(&side)
}

pub(crate) fn hidden_in(side: &HeroSidecar) -> Vec<String> {
    side.issues
        .iter()
        .filter_map(|issue| issue.strip_prefix("changesNotShown:"))
        .map(|file| side.sources.iter().find(|s| s.file == file).map(|s| s.name.clone()).unwrap_or_else(|| file.to_string()))
        .map(|name| fill(tr("{} changes nothing this view draws."), &[&name]))
        .take(3)
        .collect()
}

/// Takes the workers' reports, closes views whose owner left, and asks again when the mods
/// changed. True when a view must run again.
pub fn poll(s: &mut State) -> bool {
    let mut changed = false;
    let keys: Vec<String> = s.viewers.pending.keys().cloned().collect();
    for key in keys {
        let mut done = None;
        if let Some(p) = s.viewers.pending.get_mut(&key) {
            loop {
                match p.rx.try_recv() {
                    Ok(Msg::Stage(line, arg, step)) => {
                        (p.stage, p.arg, p.step) = (line, arg, step);
                        p.fraction = 0.0;
                        changed = true;
                    }
                    Ok(Msg::Progress(f)) => {
                        p.fraction = f;
                        changed = true;
                    }
                    Ok(Msg::Done(result)) => {
                        done = Some(result);
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    // A headless scene's hold-still entry has no sender; only a real export ends.
                    Err(TryRecvError::Disconnected) => {
                        if !rune::is_headless() {
                            done = Some(Err(tr("The preview worker stopped before it finished.").to_string()));
                        }
                        break;
                    }
                }
            }
        }
        if let Some(result) = done {
            s.viewers.pending.remove(&key);
            changed = true;
            match result {
                Ok(ready) => open_ready(s, &key, ready),
                Err(e) => {
                    s.viewers.problems.insert(key.clone(), Problem::Failed(e));
                }
            }
        }
    }
    changed |= sync_locker(s);
    // Owners that went away.
    let gone: Vec<String> = s.viewers.asks.iter().filter(|(key, a)| !alive(s, key, a.owner)).map(|(k, _)| k.clone()).collect();
    for key in gone {
        super::close(s, &key);
        changed = true;
    }
    // The mods changed under an open ask: ask again.
    if !stand_in(s) && !rune::is_headless() {
        let now = mods_sig(s);
        let stale: Vec<(String, Ask, Owner, Option<String>)> = s
            .viewers
            .asks
            .iter()
            .filter(|(key, a)| a.sig != now && !s.viewers.pending.contains_key(*key))
            .map(|(k, a)| (k.clone(), a.ask.clone(), a.owner, a.still.clone()))
            .collect();
        for (key, ask, owner, still) in stale {
            request(s, &key, ask, owner, still);
            changed = true;
        }
    }
    // No GPU: the screen's picture instead of the message, when it gave one.
    let stills: Vec<(String, String)> = s
        .viewers
        .map
        .iter()
        .filter(|(_, v)| matches!(v.status, Status::NoGpu(_)) && v.still.is_none())
        .filter_map(|(k, _)| s.viewers.asks.get(k).and_then(|a| a.still.clone()).map(|still| (k.clone(), still)))
        .collect();
    for (key, still) in stills {
        if let Some(pixels) = s.thumbnails.get(&still).cloned() {
            super::set_still(s, &key, pixels);
            changed = true;
        }
    }
    changed
}

fn open_ready(s: &mut State, key: &str, ready: Ready) {
    let title = work_title(s, key);
    // The hero, then the unmodded side once both exist: one open, so the camera stays put.
    super::open_with(s, key, &title, ready.source, ready.before, CompareMode::After);
    if let Some(v) = s.viewers.map.get_mut(key) {
        v.notes = ready.notes;
        v.labels = ready.labels;
    }
}

fn alive(s: &State, key: &str, owner: Owner) -> bool {
    match owner {
        Owner::Locker => {
            let want = locker_want(s);
            matches!((&want, s.viewers.asks.get(key).map(|a| &a.ask)), (Some(hero), Some(Ask::Hero { hero: asked })) if hero == asked)
        }
        Owner::Dialog => s.viewers.dialog.as_ref().is_some_and(|d| d.key() == key),
        Owner::FullScreen => s.viewers.fullscreen.as_deref() == Some(key),
    }
}

/// The hero the Locker's 3D view should show now, if any.
fn locker_want(s: &State) -> Option<String> {
    // Long out of sight, the view closes and gives back its GPU device; it opens again, on the
    // same hero, once the window shows.
    if s.screen != Screen::Locker || !s.locker.view3d || super::long_out_of_sight(s) {
        return None;
    }
    match &s.locker.page {
        crate::screens::locker::Page::Hero(name) => Some(name.clone()),
        _ => None,
    }
}

pub const LOCKER_KEY: &str = "locker";
pub const DIALOG_KEY: &str = "mod-preview";
pub const HIDEOUT_KEY: &str = "hideout";

/// Opens the Locker's 3D view on the hero its page shows, when the view is on and shows another
/// hero (or none). True when it asked.
pub fn sync_locker(s: &mut State) -> bool {
    let Some(hero) = locker_want(s) else { return false };
    let current = s.viewers.asks.get(LOCKER_KEY).map(|a| a.ask.clone());
    if current == Some(Ask::Hero { hero: hero.clone() }) {
        return false;
    }
    let still = s.locker.heroes.iter().find(|h| h.display_name == hero).and_then(|h| h.render.clone().or(h.card.clone()));
    request(s, LOCKER_KEY, Ask::Hero { hero }, Owner::Locker, still);
    true
}

/// The Hideout full screen, with the player's mods and `hero` at the spawn.
pub fn open_hideout(s: &mut State, hero: Option<String>) {
    request(s, HIDEOUT_KEY, Ask::Hideout { hero }, Owner::FullScreen, None);
}

/// Leaves a key the way its owner closes: the Locker's 3D view off, the dialog shut, full
/// screen left. Then the view goes.
pub fn dismiss(s: &mut State, key: &str) {
    match s.viewers.asks.get(key).map(|a| a.owner) {
        Some(Owner::Locker) => s.locker.view3d = false,
        Some(Owner::Dialog) => s.viewers.dialog = None,
        Some(Owner::FullScreen) | None => {}
    }
    if s.viewers.fullscreen.as_deref() == Some(key) {
        s.viewers.fullscreen = None;
    }
    super::close(s, key);
}

/// Asks the same thing again (Try again).
pub fn retry(s: &mut State, key: &str) {
    if let Some(a) = s.viewers.asks.get(key) {
        let (ask, owner, still) = (a.ask.clone(), a.owner, a.still.clone());
        request(s, key, ask, owner, still);
    }
}

/// Removes the least recently used cache entries past the budget, once, on a worker, a while
/// after launch.
pub fn prune_when_idle(s: &mut State) -> bool {
    let v = &mut s.viewers;
    if let Some(Ok(line)) = v.prune.try_take()
        && !line.is_empty()
    {
        eprintln!("{line}");
    }
    if rune::is_headless() || cfg!(test) || v.pruned {
        return false;
    }
    let started = *v.started.get_or_insert_with(std::time::Instant::now);
    if started.elapsed().as_secs() < 30 || !v.pending.is_empty() {
        return false;
    }
    v.pruned = true;
    let cache = cache_dir();
    v.prune.start(None, move || {
        if !cache.is_dir() {
            return Ok(String::new());
        }
        core::prune_cache(&cache, CACHE_BUDGET)
            .map(|r| format!("viewer cache: removed {} entries, freed {} bytes, kept {} bytes", r.removed, r.freed_bytes, r.kept_bytes))
            .map_err(|e| e.to_string())
    });
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use deadlock_core::viewer::{SourceKind, SourceUsage};

    /// Out of sight past `OUT_OF_SIGHT_DROP`, the Locker's 3D view is no longer wanted, so its
    /// render thread and GPU device go; back in sight it is wanted again, on the same hero.
    #[test]
    fn the_locker_view_lets_its_device_go_while_sigil_is_long_out_of_sight() {
        let mut st = State::default();
        st.screen = Screen::Locker;
        st.locker.view3d = true;
        st.locker.page = crate::screens::locker::Page::Hero("Infernus".into());
        assert_eq!(locker_want(&st).as_deref(), Some("Infernus"));
        st.viewers.hidden_since = Some(std::time::Instant::now());
        assert!(locker_want(&st).is_some(), "a short minimize keeps it");
        st.viewers.hidden_since = std::time::Instant::now().checked_sub(super::super::OUT_OF_SIGHT_DROP);
        assert!(st.viewers.hidden_since.is_some());
        assert_eq!(locker_want(&st), None);
        st.viewers.hidden_since = None;
        assert_eq!(locker_want(&st).as_deref(), Some("Infernus"), "it opens again in sight");
    }

    #[test]
    fn a_mod_none_of_whose_files_are_drawn_is_named() {
        let side = HeroSidecar {
            schema: 1,
            hero: "inferno".into(),
            display_name: "Infernus".into(),
            variant: Look::Modded,
            clips: Vec::new(),
            idle_clip: None,
            sources: vec![SourceUsage { slot: Some(3), file: "addons/pak03_dir.vpk".into(), name: "Ember Infernus".into(), kind: SourceKind::Addon, enabled: true, used: 0 }],
            model_entry: String::new(),
            clip_info: Vec::new(),
            cloth: None,
            particles: None,
            units: "meters".into(),
            up: "+Y".into(),
            forward: "+Z".into(),
            key: "k".into(),
            issues: vec!["changesNotShown:addons/pak03_dir.vpk".into(), "clipRig:run".into(), "changesNotShown:addons/pak09_dir.vpk".into()],
            pipeline: "p".into(),
        };
        let notes = hidden_in(&side);
        assert_eq!(notes.len(), 2);
        assert!(notes[0].starts_with("Ember Infernus"), "{notes:?}");
        assert!(notes[1].starts_with("addons/pak09_dir.vpk"), "{notes:?}");
    }

    /// No game: the key says so. A game folder without its VPKs: the export fails and the poll
    /// turns that into the error panel. A view whose owner left is closed.
    #[test]
    fn asks_end_in_a_view_or_a_problem_and_close_with_their_owner() {
        let mut st = crate::data::init_state();
        st.demo = false;
        st.viewers.stand_in = false;
        st.game_root = None;
        request(&mut st, "t", Ask::Hero { hero: "Haze".into() }, Owner::FullScreen, None);
        assert_eq!(st.viewers.problems.get("t"), Some(&Problem::NoGame));
        assert_eq!(st.viewers.fullscreen.as_deref(), Some("t"));

        let root = std::env::temp_dir().join(format!("sigil-viewer-prepare-{}", std::process::id()));
        std::fs::create_dir_all(root.join("game").join("citadel")).unwrap();
        st.game_root = Some(root.clone());
        request(&mut st, "t", Ask::Hero { hero: "Haze".into() }, Owner::FullScreen, None);
        assert!(st.viewers.pending.contains_key("t"));
        let waited = std::time::Instant::now();
        while st.viewers.pending.contains_key("t") && waited.elapsed().as_secs() < 20 {
            poll(&mut st);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(matches!(st.viewers.problems.get("t"), Some(Problem::Failed(e)) if e.contains("pak01_dir.vpk")), "{:?}", st.viewers.problems.get("t"));

        // Leaving full screen closes a view that lives only there.
        st.viewers.fullscreen = None;
        poll(&mut st);
        assert!(!st.viewers.asks.contains_key("t") && !st.viewers.problems.contains_key("t"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn titles_name_the_hero_and_the_mod() {
        assert_eq!(title_of(&Ask::Hero { hero: "inferno".into() }), "Infernus");
        assert_eq!(title_of(&Ask::Mod { hero: "Haze".into(), slot: 3, name: "Neon".into() }), "Neon on Haze");
    }
}
