//! What turning a mod on or off asks first: a second mod for the same hero (Deadlock Mod
//! Manager's hero conflict warning), the mods its GameBanana page says it needs, and the mods
//! that need it when it goes off.
//!
//! Every switch in Installed and the Enable button in Browse go through [`switch`]. When nothing
//! needs asking it is the same single rename as before; otherwise one dialog opens and its
//! buttons make the one write the player picked, through `apply_switch`, as one Undo.
//!
//! Requirements come from `deadlock_core::requirements` (a GameBanana page's `_aRequirements`).
//! They are read in the background for the installed GameBanana mods while Installed is open and
//! kept in `cache/requirements.json`, so a switch never waits on the network: a mod whose page
//! was not read yet turns on at once. The checks themselves are core's `missing_on_enable` and
//! `broken_on_disable`, run against the mods on in the addon folder (the active profile).
//! Settings > Mods > "Show what a mod needs" turns these checks off, and "Warn about a second
//! mod for one hero" the hero one.
//!
//! A mod's heroes are its GameBanana hero tag plus the heroes whose model and material files it
//! replaces (`deadlock_core::mod_heroes::model_heroes_in_vpk`, Deadlock Mod Manager's rule), so
//! two untagged skins for one hero still ask. The VPK directories are read on a worker while
//! Installed or Browse is open, once per slot file (`(slot, size)`), and kept in [`Gates`]; a
//! switch only looks the answer up and never waits on a read.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;

use deadlock_core::requirements::{
    self as needs, MissingRequirement, ModRequirements, Requirement, RequirementState,
};
use rune::{Color, El, Key, KeyEvent, Tx, col, icon, row, spacer, text};
use serde_json::{Value, json};

use crate::app::State;
use crate::icons;
use crate::jobs::Job;
use crate::model::{AddonRow, ConflictRow};
use crate::screens::settings::i18n::{fill, tr};
use crate::screens::settings::options;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, MICRO, SMALL, Ui};

use super::{display_name, meta_for, notes};

/// A requirement page is read again after a week.
const STALE_SECS: i64 = 7 * 24 * 60 * 60;
/// Pages read per background pass.
const BATCH: usize = 6;
/// VPK directories read per background pass for their model heroes.
const MODEL_BATCH: usize = 64;

/// The open question, if any.
#[derive(Clone, Debug, PartialEq)]
pub enum Gate {
    /// Turning `slot` on while `others` are on for the same hero.
    Hero { slot: u16, hero: String, others: Vec<u16> },
    /// `slot` needs mods that are off or not installed. `also_off` is what the hero question
    /// already chose to turn off; `with` the installed requirements ticked to turn on too.
    Needs {
        slot: u16,
        also_off: Vec<u16>,
        missing: Vec<MissingRequirement>,
        with: BTreeSet<u16>,
    },
    /// Turning `slot` off while the mods in `dependents` (slots) list it as a requirement.
    Breaks { slot: u16, dependents: Vec<u16> },
    /// Grimoire's mod safety gate: `slot` ships programs, scripts or network calls and this
    /// version is not trusted yet. `findings` are "Risk: path" lines.
    Safety { slot: u16, findings: Vec<String> },
}

/// One slot file's model heroes, from the worker: `((slot, size), heroes)`.
pub type ModelHeroes = ((u16, u64), Vec<String>);

/// The gate's state, kept with Installed.
#[derive(Default)]
pub struct Gates {
    pub open: Option<Gate>,
    /// Requirements by GameBanana mod id, and when each page was read (unix seconds).
    pub known: HashMap<u64, (i64, Vec<Requirement>)>,
    pub loaded: bool,
    pub job: Job<Vec<(u64, Vec<Requirement>)>>,
    /// Ids asked for this session, found or not.
    pub tried: HashSet<u64>,
    /// Heroes each slot file's model and material paths replace, by `(slot, size)`.
    pub model_heroes: HashMap<(u16, u64), Vec<String>>,
    pub model_job: Job<Vec<ModelHeroes>>,
    /// Slot files given to the worker this session, read or not.
    pub model_tried: HashSet<(u16, u64)>,
}

// --- pure rules -------------------------------------------------------------------------------

/// What the hero check needs to know about one installed mod.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Facts {
    pub slot: u16,
    pub enabled: bool,
    /// The GameBanana hero tag and the heroes its model paths replace, without case duplicates.
    pub heroes: Vec<String>,
    /// The variant group key: files of one mod never count against each other.
    pub group: Option<String>,
}

/// A mod's heroes: the tag first, then the model heroes, blanks and case duplicates dropped.
pub fn hero_union(tag: Option<&str>, models: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for hero in tag.into_iter().chain(models.iter().map(String::as_str)) {
        let hero = hero.trim();
        if !hero.is_empty() && !out.iter().any(|h| h.eq_ignore_ascii_case(hero)) {
            out.push(hero.to_string());
        }
    }
    out
}

/// The heroes two mods share, in `a`'s order.
pub fn shared_heroes(a: &Facts, b: &Facts) -> Vec<String> {
    a.heroes
        .iter()
        .filter(|hero| b.heroes.iter().any(|h| h.eq_ignore_ascii_case(hero)))
        .cloned()
        .collect()
}

/// The mods that are on for any hero of `target`, outside its own variant group. Empty when
/// the target has no hero.
pub fn same_hero(target: &Facts, mods: &[Facts]) -> Vec<u16> {
    if target.heroes.is_empty() {
        return Vec::new();
    }
    mods.iter()
        .filter(|other| other.slot != target.slot && other.enabled)
        .filter(|other| target.group.is_none() || other.group != target.group)
        .filter(|other| !shared_heroes(target, other).is_empty())
        .map(|other| other.slot)
        .collect()
}

/// The heroes `target` shares with any of `others`, for the question's title.
pub fn heroes_in_question(target: &Facts, mods: &[Facts], others: &[u16]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for other in mods.iter().filter(|m| others.contains(&m.slot)) {
        for hero in shared_heroes(target, other) {
            if !out.iter().any(|h| h.eq_ignore_ascii_case(&hero)) {
                out.push(hero);
            }
        }
    }
    out
}

/// Slots whose model another enabled mod hides (Deadlock Mod Manager's rule): the mod ships a
/// `.vmdl_c` that a mod loading before it also ships, so the game uses the other one's model.
pub fn hidden_models(conflicts: &[ConflictRow], enabled: &BTreeSet<u16>) -> BTreeSet<u16> {
    let mut hidden = BTreeSet::new();
    for row in conflicts {
        if !row.path.to_ascii_lowercase().ends_with(".vmdl_c") {
            continue;
        }
        for slot in &row.slots {
            if *slot != row.winner && enabled.contains(slot) {
                hidden.insert(*slot);
            }
        }
    }
    hidden
}

/// The switches one answer writes: the target, what goes off with it, and what comes on with it.
pub fn wanted(slot: u16, enable: bool, also_off: &[u16], also_on: &[u16]) -> BTreeMap<u16, bool> {
    let mut wanted = BTreeMap::new();
    for other in also_off {
        wanted.insert(*other, false);
    }
    for other in also_on {
        wanted.insert(*other, true);
    }
    wanted.insert(slot, enable);
    wanted
}

// --- reading the library -----------------------------------------------------------------------

fn facts(st: &State) -> Vec<Facts> {
    st.addons
        .addons
        .iter()
        .map(|addon| {
            let extra = notes::extra(st, addon);
            let id = meta_for(st, addon).and_then(|meta| meta.gamebanana_id);
            let models = st
                .installed
                .gates
                .model_heroes
                .get(&(addon.slot, addon.size_bytes))
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            Facts {
                slot: addon.slot,
                enabled: addon.enabled,
                heroes: hero_union(extra.hero.as_deref(), models),
                group: deadlock_core::mod_extras::variant_key(Some(&extra), id),
            }
        })
        .collect()
}

fn addon(st: &State, slot: u16) -> Option<&AddonRow> {
    st.addons.addons.iter().find(|addon| addon.slot == slot)
}

fn gamebanana_id(st: &State, slot: u16) -> Option<u64> {
    addon(st, slot).and_then(|addon| meta_for(st, addon)).and_then(|meta| meta.gamebanana_id)
}

/// The installed slot of a GameBanana mod: one that is on, else the lowest.
fn slot_of_id(st: &State, id: u64) -> Option<u16> {
    let mut slots: Vec<&AddonRow> = st
        .addons
        .addons
        .iter()
        .filter(|addon| meta_for(st, addon).and_then(|meta| meta.gamebanana_id) == Some(id))
        .collect();
    slots.sort_by_key(|addon| (!addon.enabled, addon.slot));
    slots.first().map(|addon| addon.slot)
}

fn ids(st: &State, enabled_only: bool) -> Vec<u64> {
    let mut ids: Vec<u64> = st
        .addons
        .addons
        .iter()
        .filter(|addon| !enabled_only || addon.enabled)
        .filter_map(|addon| meta_for(st, addon).and_then(|meta| meta.gamebanana_id))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn requirements_of(st: &State, id: u64) -> Option<ModRequirements> {
    let (_, list) = st.installed.gates.known.get(&id)?;
    Some(ModRequirements::new(Some(id), list.clone()))
}

/// What turning `slot` on still needs, with `also_off` going off at the same time.
fn missing_for(st: &State, slot: u16, also_off: &[u16]) -> Vec<MissingRequirement> {
    let Some(target) = gamebanana_id(st, slot).and_then(|id| requirements_of(st, id)) else {
        return Vec::new();
    };
    let off: Vec<u64> = also_off.iter().filter_map(|s| gamebanana_id(st, *s)).collect();
    let enabled: Vec<u64> = ids(st, true).into_iter().filter(|id| !off.contains(id)).collect();
    needs::missing_on_enable(&target, &enabled, &ids(st, false))
        .into_iter()
        .filter(|m| m.state != RequirementState::Unknown)
        .collect()
}

/// The enabled slots that need `slot`, by their GameBanana pages.
fn dependents_of(st: &State, slot: u16) -> Vec<u16> {
    let Some(id) = gamebanana_id(st, slot) else {
        return Vec::new();
    };
    // Another installed file of the same mod that stays on still satisfies them.
    let still_on = st
        .addons
        .addons
        .iter()
        .any(|a| a.enabled && a.slot != slot && meta_for(st, a).and_then(|m| m.gamebanana_id) == Some(id));
    if still_on {
        return Vec::new();
    }
    let enabled: Vec<(u64, ModRequirements)> = ids(st, true)
        .into_iter()
        .filter_map(|other| requirements_of(st, other).map(|r| (other, r)))
        .collect();
    let mut slots: Vec<u16> = needs::broken_on_disable(id, &enabled)
        .into_iter()
        .flat_map(|other| {
            st.addons
                .addons
                .iter()
                .filter(move |a| a.enabled)
                .filter(move |a| meta_for(st, a).and_then(|m| m.gamebanana_id) == Some(other))
                .map(|a| a.slot)
                .collect::<Vec<_>>()
        })
        .collect();
    slots.sort_unstable();
    slots.dedup();
    slots
}

// --- the switch -----------------------------------------------------------------------------------

/// A switch press on one slot: ask first when something needs asking, else switch it.
pub fn switch(st: &mut State, slot: u16, enable: bool) {
    st.installed.menu = None;
    // Refuse before any question opens: an answer would only be refused at the write.
    if crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    if enable && let Some(findings) = super::safety_gate::needs_review(st, slot) {
        st.installed.gates.open = Some(Gate::Safety { slot, findings });
        return;
    }
    switch_reviewed(st, slot, enable);
}

/// The switch after the safety gate: the hero and requirement questions, then the write.
pub fn switch_reviewed(st: &mut State, slot: u16, enable: bool) {
    st.installed.gates.open = None;
    if enable {
        let (others, heroes) = if options::hero_conflict_warning() {
            let all = facts(st);
            match all.iter().find(|f| f.slot == slot) {
                Some(target) => {
                    let others = same_hero(target, &all);
                    let heroes = heroes_in_question(target, &all, &others);
                    (others, heroes)
                }
                None => (Vec::new(), Vec::new()),
            }
        } else {
            (Vec::new(), Vec::new())
        };
        if !others.is_empty() {
            let hero = heroes.join(", ");
            st.installed.gates.open = Some(Gate::Hero { slot, hero, others });
            return;
        }
        check_needs(st, slot, Vec::new());
    } else {
        let dependents = if options::prompt_requirements() { dependents_of(st, slot) } else { Vec::new() };
        if !dependents.is_empty() {
            st.installed.gates.open = Some(Gate::Breaks { slot, dependents });
            return;
        }
        super::toggle_one(st, slot, false);
    }
}

/// After the hero question: ask about requirements, or write.
fn check_needs(st: &mut State, slot: u16, also_off: Vec<u16>) {
    let missing = if options::prompt_requirements() { missing_for(st, slot, &also_off) } else { Vec::new() };
    if missing.is_empty() {
        return apply(st, slot, true, &also_off, &[]);
    }
    // The installed ones are ticked to come on with it.
    let with: BTreeSet<u16> = missing
        .iter()
        .filter(|m| m.state == RequirementState::InstalledOff)
        .filter_map(|m| m.gamebanana_id.and_then(|id| slot_of_id(st, id)))
        .collect();
    st.installed.gates.open = Some(Gate::Needs { slot, also_off, missing, with });
}

/// One write for the answer: a plain switch when only the target changes.
fn apply(st: &mut State, slot: u16, enable: bool, also_off: &[u16], also_on: &[u16]) {
    st.installed.gates.open = None;
    if also_off.is_empty() && also_on.is_empty() {
        return super::toggle_one(st, slot, enable);
    }
    let Some(game_root) = super::writable_game(st) else { return };
    let wanted = wanted(slot, enable, also_off, also_on);
    let name = addon(st, slot).map(|a| display_name(st, a)).unwrap_or_else(|| format!("{slot:02}"));
    let changed = wanted.len().saturating_sub(1).to_string();
    let line = if enable {
        fill(tr("Turned on {} and switched {} other mods."), &[&name, &changed])
    } else {
        fill(tr("Turned off {} and {} mods that need it."), &[&name, &changed])
    };
    super::apply_switch(st, &game_root, &wanted, line);
}

fn close(st: &mut State) {
    st.installed.gates.open = None;
}

// --- the requirement cache ------------------------------------------------------------------------

fn cache_file() -> PathBuf {
    crate::data::data_dir().join("cache").join("requirements.json")
}

fn now() -> i64 {
    deadlock_core::gamebanana::now_unix()
}

/// Parse the cache file. Unknown or broken entries are skipped one by one.
pub fn parse_cache(text: &str) -> HashMap<u64, (i64, Vec<Requirement>)> {
    let mut out = HashMap::new();
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(text) else {
        return out;
    };
    let Some(Value::Object(mods)) = map.get("mods") else {
        return out;
    };
    for (key, entry) in mods {
        let Ok(id) = key.parse::<u64>() else { continue };
        let at = entry.get("at").and_then(Value::as_i64).unwrap_or(0);
        let list: Vec<Requirement> = entry
            .get("list")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|pair| {
                        let name = pair.get(0)?.as_str()?.trim().to_string();
                        let url = pair.get(1).and_then(Value::as_str).unwrap_or("").trim().to_string();
                        (!name.is_empty()).then(|| Requirement { mod_id: needs::mod_id_of(&url), name, url })
                    })
                    .take(32)
                    .collect()
            })
            .unwrap_or_default();
        out.insert(id, (at, list));
    }
    out
}

pub fn cache_json(known: &HashMap<u64, (i64, Vec<Requirement>)>) -> String {
    let mut mods = serde_json::Map::new();
    let mut ids: Vec<&u64> = known.keys().collect();
    ids.sort();
    for id in ids {
        let (at, list) = &known[id];
        let list: Vec<Value> = list.iter().map(|r| json!([r.name, r.url])).collect();
        mods.insert(id.to_string(), json!({"at": at, "list": list}));
    }
    serde_json::to_string_pretty(&json!({"schema": 1, "mods": mods})).unwrap_or_default()
}

fn save_cache(known: &HashMap<u64, (i64, Vec<Requirement>)>) {
    let path = cache_file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, cache_json(known)).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// Read the model heroes of slot files not read yet, a batch at a time on a worker, while
/// Installed or Browse is open and the hero warning is on. Local reads only (the VPK directory
/// tree), so it runs offline too; never in the demo or a headless run.
fn poll_model_heroes(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.installed.gates.model_job.try_take() {
        if let Ok(found) = result {
            st.installed.gates.model_heroes.extend(found);
        }
        changed = true;
    }
    if st.demo || rune::is_headless() || !options::hero_conflict_warning() {
        return changed;
    }
    let open = matches!(st.screen, crate::model::Screen::Installed | crate::model::Screen::Browse);
    if !open || !st.addons.scanned || st.installed.gates.model_job.is_running() {
        return changed;
    }
    let dir = super::addons_folder(st);
    let wanted: Vec<((u16, u64), PathBuf)> = st
        .addons
        .addons
        .iter()
        .map(|addon| ((addon.slot, addon.size_bytes), addon.path(&dir)))
        .filter(|(key, _)| !st.installed.gates.model_heroes.contains_key(key) && !st.installed.gates.model_tried.contains(key))
        .take(MODEL_BATCH)
        .collect();
    if wanted.is_empty() {
        return changed;
    }
    st.installed.gates.model_tried.extend(wanted.iter().map(|(key, _)| *key));
    // C2: the app data dir keeps a cache keyed by path, size and mtime, so a mod is read once,
    // not on every launch.
    let cache = crate::data::data_dir().join(deadlock_core::mod_heroes::HERO_CACHE_FILE);
    st.installed.gates.model_job.start(st.wake.clone(), move || {
        let paths: Vec<PathBuf> = wanted.iter().map(|(_, path)| path.clone()).collect();
        let found = deadlock_core::mod_heroes::model_heroes_cached(&cache, &paths);
        Ok(wanted
            .into_iter()
            .filter_map(|(key, path)| found.get(&path).cloned().map(|heroes| (key, heroes)))
            .collect())
    });
    changed
}

/// Read the cache once, take finished lookups, and look up a few more installed mods while
/// Installed or Browse is open. Never in the demo, offline or a headless run.
pub fn poll(st: &mut State) -> bool {
    let mut changed = poll_model_heroes(st);
    if let Some(result) = st.installed.gates.job.try_take() {
        if let Ok(found) = result {
            let at = now();
            for (id, list) in found {
                st.installed.gates.known.insert(id, (at, list));
            }
            save_cache(&st.installed.gates.known);
        }
        changed = true;
    }
    if st.demo || rune::is_headless() || st.prefs.offline || !options::prompt_requirements() {
        return changed;
    }
    if !st.installed.gates.loaded {
        st.installed.gates.loaded = true;
        if let Ok(text) = std::fs::read_to_string(cache_file()) {
            st.installed.gates.known = parse_cache(&text);
        }
        changed = true;
    }
    let open = matches!(st.screen, crate::model::Screen::Installed | crate::model::Screen::Browse);
    if !open || !st.addons.scanned || st.installed.gates.job.is_running() {
        return changed;
    }
    let at = now();
    let wanted: Vec<u64> = ids(st, false)
        .into_iter()
        .filter(|id| !st.installed.gates.tried.contains(id))
        .filter(|id| st.installed.gates.known.get(id).is_none_or(|(when, _)| at - when > STALE_SECS))
        .take(BATCH)
        .collect();
    if wanted.is_empty() {
        return changed;
    }
    st.installed.gates.tried.extend(wanted.iter().copied());
    st.installed.gates.job.start(st.wake.clone(), move || {
        Ok(wanted
            .into_iter()
            .filter_map(|id| needs::fetch_requirements(id).ok().map(|list| (id, list)))
            .collect())
    });
    changed
}

// --- the dialogs ----------------------------------------------------------------------------------

/// The open question, mounted with Installed's dialogs (and over Browse, for its Enable).
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    let gate = s.installed.gates.open.as_ref()?;
    Some(match gate {
        Gate::Hero { slot, hero, others } => hero_dialog(s, u, *slot, hero, others),
        Gate::Needs { slot, also_off, missing, with } => needs_dialog(s, u, *slot, also_off, missing, with),
        Gate::Breaks { slot, dependents } => breaks_dialog(s, u, *slot, dependents),
        Gate::Safety { slot, findings } => super::safety_gate::dialog(s, u, *slot, findings),
    })
}

fn shell(u: &Ui, key: &str, title: &str) -> El {
    u.dialog_shell()
        .key(key)
        .role("dialog")
        .label(title.to_string())
        .w(480.0)
        .padding(24.0)
        .gap(16.0)
        .modal(true, close)
        .on_key(|st: &mut State, ev: &KeyEvent, _tx: &mut Tx| {
            if ev.key == Key::Escape {
                close(st);
                true
            } else {
                false
            }
        })
        .child(text(title.to_string()).font(H1).color(u.ink).wrap())
}

fn name_of(s: &State, slot: u16) -> String {
    addon(s, slot).map(|a| display_name(s, a)).unwrap_or_else(|| format!("{slot:02}"))
}

/// One mod in a dialog list: its slot, name and a quiet note on the right.
fn mod_line(s: &State, u: &Ui, slot: u16, note: &str, strong: bool) -> El {
    row()
        .key(format!("gate-mod-{slot:02}"))
        .w_full()
        .h(36.0)
        .gap(12.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(text(deadlock_core::addons::slot_label(slot)).font(MICRO).color(u.faint).w(28.0))
        .child(
            text(name_of(s, slot))
                .font(if strong { BODY_MED } else { BODY })
                .color(if strong { u.ink } else { u.prose })
                .truncate()
                .grow(1.0)
                .min_w(0.0),
        )
        .child(text(note.to_string()).font(SMALL).color(u.faint))
}

fn buttons(items: Vec<El>) -> El {
    let mut r = row().w_full().gap(8.0).center_y().pt(4.0).child(spacer());
    for item in items {
        r = r.child(item);
    }
    r
}

fn cancel(u: &Ui) -> El {
    u.action(tr("Cancel"), ActionKind::Ghost)
        .key("gate-cancel")
        .cursor(rune::Cursor::Pointer)
        .on_press(close)
}

fn hero_dialog(s: &State, u: &Ui, slot: u16, hero: &str, others: &[u16]) -> El {
    let title = fill(tr("{} already has a mod on"), &[hero]);
    let mut list = col().w_full().border_sides([1.0, 0.0, 0.0, 0.0], u.hair);
    for other in others {
        list = list.child(mod_line(s, u, *other, tr("On now"), false));
    }
    list = list.child(mod_line(s, u, slot, tr("Turning on"), true));
    let off = others.to_vec();
    let switch_label = if others.len() == 1 {
        tr("Switch to this one").to_string()
    } else {
        fill(tr("Switch, turning {} off"), &[&others.len().to_string()])
    };
    shell(u, "gate-hero", &title)
        .child(
            text(tr("Two mods for one hero usually replace the same model, and the lower slot wins. Switch to turn the other off, or keep both when they are made to go together."))
                .font(BODY)
                .color(u.muted)
                .wrap(),
        )
        .child(list)
        .child(buttons(vec![
            cancel(u),
            u.action(tr("Keep both"), ActionKind::Secondary)
                .key("gate-hero-both")
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| check_needs(st, slot, Vec::new())),
            u.action(&switch_label, ActionKind::Primary)
                .key("gate-hero-switch")
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| check_needs(st, slot, off.clone())),
        ]))
}

fn needs_dialog(
    s: &State,
    u: &Ui,
    slot: u16,
    also_off: &[u16],
    missing: &[MissingRequirement],
    with: &BTreeSet<u16>,
) -> El {
    let title = fill(tr("{} needs other mods"), &[&name_of(s, slot)]);
    let mut list = col().w_full().border_sides([1.0, 0.0, 0.0, 0.0], u.hair);
    for (index, need) in missing.iter().enumerate() {
        let installed = need.gamebanana_id.and_then(|id| slot_of_id(s, id));
        let mut line = row()
            .key(format!("gate-need-{index}"))
            .w_full()
            .min_h(44.0)
            .py(6.0)
            .gap(10.0)
            .center_y()
            .border_sides([0.0, 0.0, 1.0, 0.0], u.hair);
        let state = match need.state {
            RequirementState::InstalledOff => tr("Installed, off"),
            RequirementState::NotInstalled => tr("Not installed"),
            RequirementState::Unknown => tr("Not on GameBanana"),
        };
        line = line.child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(text(need.name.clone()).font(BODY_MED).color(u.ink).truncate())
                .child(text(state.to_string()).font(SMALL).color(u.faint)),
        );
        match (need.state, installed, need.gamebanana_id) {
            (RequirementState::InstalledOff, Some(req), _) => {
                let on = with.contains(&req);
                line = line.child(
                    row()
                        .key(format!("gate-need-with-{index}"))
                        .h(32.0)
                        .gap(8.0)
                        .center_y()
                        .focusable(true)
                        .role("checkbox")
                        .checked(on)
                        .label(tr("Turn on too"))
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| {
                            if let Some(Gate::Needs { with, .. }) = st.installed.gates.open.as_mut()
                                && !with.remove(&req)
                            {
                                with.insert(req);
                            }
                        })
                        .child(super::check_box(u, on, false))
                        .child(text(tr("Turn on too")).font(BODY).color(u.muted).a11y_hidden()),
                );
            }
            (RequirementState::NotInstalled, _, Some(id)) => {
                let name = need.name.clone();
                let b = crate::screens::browse::quick::button(s, id);
                line = line.child(
                    crate::screens::browse::quick::button_el(
                        u,
                        &b,
                        format!("gate-need-install-{index}"),
                        id,
                        &name,
                        crate::screens::browse::quick::Size::Row,
                    ),
                );
            }
            _ => {
                if let Some(url) = need.url.clone() {
                    line = line.child(
                        u.icon_button(&icons::EXTERNAL_LINK, tr("Open the link"))
                            .key(format!("gate-need-open-{index}"))
                            .cursor(rune::Cursor::Pointer)
                            .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                                if let Err(error) = tx.open_url(&url) {
                                    st.status = error.to_string();
                                }
                            }),
                    );
                }
            }
        }
        list = list.child(line);
    }
    let off = also_off.to_vec();
    let on: Vec<u16> = with.iter().copied().collect();
    let label = if on.is_empty() {
        tr("Turn on anyway").to_string()
    } else {
        fill(tr("Turn on all {}"), &[&(on.len() + 1).to_string()])
    };
    shell(u, "gate-needs", &title)
        .child(
            text(tr("Its GameBanana page lists these. Without them it may not work in game."))
                .font(BODY)
                .color(u.muted)
                .wrap(),
        )
        .child(list)
        .child(buttons(vec![
            cancel(u),
            u.action(&label, ActionKind::Primary)
                .key("gate-needs-apply")
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| apply(st, slot, true, &off, &on)),
        ]))
}

fn breaks_dialog(s: &State, u: &Ui, slot: u16, dependents: &[u16]) -> El {
    let title = fill(tr("Other mods need {}"), &[&name_of(s, slot)]);
    let mut list = col().w_full().border_sides([1.0, 0.0, 0.0, 0.0], u.hair);
    for other in dependents {
        list = list.child(mod_line(s, u, *other, tr("Needs it"), false));
    }
    let deps = dependents.to_vec();
    shell(u, "gate-breaks", &title)
        .child(
            text(tr("Their GameBanana pages list it as a requirement. They may stop working while it is off."))
                .font(BODY)
                .color(u.muted)
                .wrap(),
        )
        .child(list)
        .child(buttons(vec![
            cancel(u),
            u.action(tr("Turn those off too"), ActionKind::Secondary)
                .key("gate-breaks-all")
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    // Turning them off is the same write with every slot going off.
                    st.installed.gates.open = None;
                    let Some(game_root) = super::writable_game(st) else { return };
                    let wanted = wanted(slot, false, &deps, &[]);
                    let name = name_of(st, slot);
                    let line = fill(tr("Turned off {} and {} mods that need it."), &[&name, &deps.len().to_string()]);
                    super::apply_switch(st, &game_root, &wanted, line);
                }),
            u.action(tr("Turn off"), ActionKind::Primary)
                .key("gate-breaks-apply")
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| apply(st, slot, false, &[], &[])),
        ]))
}

/// The hidden-model mark on a card: the game shows another mod's model instead of this one.
/// A press opens Overlap, where the winner is chosen.
pub fn hidden_badge(s: &State, u: &Ui, slot: u16) -> Option<El> {
    if !s.installed.hidden.contains(&slot) {
        return None;
    }
    Some(
        rune::el()
            .key(format!("installed-hidden-{slot:02}"))
            .size(28.0, 28.0)
            .center()
            .radius(crate::ui::r(8.0))
            .fill(Color::BLACK.alpha(0.6))
            .hover(Color::BLACK.alpha(0.78))
            .focusable(true)
            .role("button")
            .label(tr("Model hidden. Another mod's model is used instead of this one."))
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| st.go(crate::model::Screen::Conflicts))
            .child(icon(&EYE_OFF, 14.0).color(u.amber).a11y_hidden()),
    )
}

/// Lucide eye-off (ISC), drawn like `icons.rs`.
pub static EYE_OFF: rune::Icon = rune::Icon {
    name: "sigil-installed-eye-off",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M10.733 5.076a10.744 10.744 0 0 1 11.205 6.575 1 1 0 0 1 0 .696 10.747 10.747 0 0 1-1.444 2.49"/><path d="M14.084 14.158a3 3 0 0 1-4.242-4.242"/><path d="M17.479 17.499a10.75 10.75 0 0 1-15.417-5.151 1 1 0 0 1 0-.696 10.75 10.75 0 0 1 4.446-5.143"/><path d="m2 2 20 20"/></g>"#,
};

/// Recompute the hidden-model slots after a scan.
pub fn refresh_hidden(st: &mut State) -> bool {
    let enabled: BTreeSet<u16> = st.addons.addons.iter().filter(|a| a.enabled).map(|a| a.slot).collect();
    let hidden = hidden_models(&st.addons.conflicts, &enabled);
    if hidden != st.installed.hidden {
        st.installed.hidden = hidden;
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(slot: u16, enabled: bool, hero: Option<&str>, group: Option<&str>) -> Facts {
        Facts {
            slot,
            enabled,
            heroes: hero.map(str::to_string).into_iter().collect(),
            group: group.map(str::to_string),
        }
    }

    fn many(slot: u16, enabled: bool, heroes: &[&str], group: &str) -> Facts {
        Facts {
            slot,
            enabled,
            heroes: heroes.iter().map(|h| h.to_string()).collect(),
            group: Some(group.to_string()),
        }
    }

    #[test]
    fn a_mod_has_its_tag_and_its_model_heroes() {
        let models = vec!["Haze".to_string(), "paradox".to_string()];
        assert_eq!(hero_union(Some("Paradox"), &models), vec!["Paradox", "Haze"], "the tag first, no case duplicates");
        assert_eq!(hero_union(None, &models), vec!["Haze", "paradox"]);
        assert_eq!(hero_union(Some("  "), &[]), Vec::<String>::new());
    }

    #[test]
    fn a_model_hero_counts_without_a_tag() {
        // Slot 2 is tagged Paradox; slot 4 has no tag but replaces Paradox's model.
        let mods = vec![
            many(2, true, &["Paradox"], "gb:2"),
            many(3, true, &["Haze", "Infernus"], "gb:3"),
            many(4, false, &["Paradox", "Infernus"], "local:4"),
            many(5, false, &[], "local:5"),
        ];
        assert_eq!(same_hero(&mods[2], &mods), vec![2, 3], "any shared hero asks");
        assert_eq!(heroes_in_question(&mods[2], &mods, &[2, 3]), vec!["Paradox", "Infernus"]);
        assert!(same_hero(&mods[3], &mods).is_empty(), "no hero, no question");
        assert_eq!(shared_heroes(&mods[1], &mods[2]), vec!["Infernus"]);
    }

    #[test]
    fn a_second_skin_for_one_hero_is_found() {
        let mods = vec![
            f(1, true, Some("Paradox"), Some("gb:1")),
            f(2, false, Some("Paradox"), Some("gb:2")),
            f(3, true, Some("Haze"), Some("gb:3")),
            f(4, false, Some("paradox"), Some("gb:4")),
        ];
        assert_eq!(same_hero(&mods[1], &mods), vec![1], "only the enabled one, not Haze");
        assert_eq!(same_hero(&mods[3], &mods), vec![1], "hero names match without case");
        assert!(same_hero(&f(9, false, None, None), &mods).is_empty(), "no hero, no warning");
    }

    #[test]
    fn files_of_one_mod_never_conflict_with_each_other() {
        let mods = vec![f(1, true, Some("Paradox"), Some("gb:7")), f(2, false, Some("Paradox"), Some("gb:7"))];
        assert!(same_hero(&mods[1], &mods).is_empty());
    }

    #[test]
    fn a_model_another_mod_wins_is_hidden() {
        let conflicts = vec![
            ConflictRow { path: "models/heroes/paradox/paradox.vmdl_c".into(), slots: vec![2, 5, 9], winner: 2 },
            ConflictRow { path: "materials/x.vmat_c".into(), slots: vec![3, 4], winner: 3 },
        ];
        let enabled: BTreeSet<u16> = [2, 3, 4, 5].into_iter().collect();
        let hidden = hidden_models(&conflicts, &enabled);
        assert_eq!(hidden.into_iter().collect::<Vec<_>>(), vec![5], "the winner and a texture loser are not hidden; 9 is off");
    }

    #[test]
    fn one_answer_is_one_write() {
        let w = wanted(4, true, &[1, 2], &[7]);
        assert_eq!(w.into_iter().collect::<Vec<_>>(), vec![(1, false), (2, false), (4, true), (7, true)]);
        let w = wanted(4, false, &[4], &[]);
        assert_eq!(w.get(&4), Some(&false), "the target's own answer wins");
    }

    #[test]
    fn the_cache_round_trips_and_skips_junk() {
        let mut known = HashMap::new();
        known.insert(
            12,
            (
                100,
                vec![Requirement { name: "Base".into(), url: "https://gamebanana.com/mods/55".into(), mod_id: Some(55) }],
            ),
        );
        known.insert(13, (200, Vec::new()));
        let back = parse_cache(&cache_json(&known));
        assert_eq!(back, known);
        assert!(parse_cache("not json").is_empty());
        let junk = parse_cache(r#"{"mods":{"x":{"at":1},"7":{"at":2,"list":[["",""],["Tool",""]]}}}"#);
        assert_eq!(junk.len(), 1);
        assert_eq!(junk[&7].1.len(), 1);
        assert_eq!(junk[&7].1[0].mod_id, None);
    }
}
