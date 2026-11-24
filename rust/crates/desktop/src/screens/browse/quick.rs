//! One-press installs from Browse.
//!
//! Install on a card, a list row or the mod details starts at once: the archive goes through
//! the download queue (`deadlock_core::download_queue`, the rows on the Downloads page), then a
//! worker plans and applies it with the same validation, backups and atomic writes as the
//! install dialog. The press is the approval (ADR 0011).
//!
//! State is kept per mod in [`Installs::items`], keyed by the GameBanana id, so one install
//! never changes another card. A button reads [`button`]: Install, Queued, Downloading NN%,
//! Installing, then Installed (or Update when a newer file is out, Retry after a failure, Enable
//! when it is installed but every file of it is switched off: installs land off by default, and
//! a press turns it on through Installed's switch, with its hero and requirement questions).
//! Installs run one at a time so two plans never pick the same free slot; downloads queue.
//! Updates of an installed mod download here too, then hand the slot to Installed's update
//! queue, which replaces the file in place.

use std::collections::{BTreeMap, HashMap};

use deadlock_core::download_queue::{Command, State as Row};
use deadlock_core::install::InstallJournalEntry;

use super::*;
use crate::screens::settings::i18n::{fill, tr};

#[path = "settings_review.rs"]
pub mod settings_review;

/// Where one mod's install is.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Sent to the download queue and waiting for its turn.
    Queued,
    /// The archive is streaming. The fraction is unknown until the size is.
    Downloading(Option<f32>),
    /// Paused on the Downloads page.
    Paused(Option<f32>),
    /// Downloaded, then planned and written by the install worker (or Installed's update
    /// queue for an update).
    Installing,
    Done,
    Failed(String),
}

/// One mod's install.
pub struct Entry {
    pub name: String,
    /// What the queue and the plan read: the mod id (its newest file) or a file's link.
    pub target: String,
    /// The file row it was started from in the details, when it was not the mod's newest.
    pub file: Option<u64>,
    /// Slots of an installed copy to replace in place. Empty for a new install.
    pub update: Vec<u16>,
    pub step: Step,
    /// The download queue row, once the worker added it.
    row: Option<String>,
    /// When the `Add` was sent, so an older row for the same target is never taken for it.
    sent_ms: i64,
    archive: Option<PathBuf>,
    /// The install worker (or the update queue) has this entry.
    started: bool,
    /// Name, author and preview for the Installed card, from the card or the details.
    author: Option<String>,
    thumbnail: Option<String>,
    /// Set by Cancel while the install worker plans this mod (`install::PlanOptions`).
    cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// Place in the download line while it waits, 1 runs next (wave 4, Grimoire's "Queued #2").
    pub place: Option<usize>,
    /// The file server the download streams from (wave 4, Grimoire's "Automatic · server").
    pub server: Option<String>,
    /// Cancel was pressed after the update went to Installed's queue: it ends as cancelled.
    cancelled: bool,
    /// The file server it failed over from, and when (Grimoire's "Switching from X").
    pub switched: Option<(String, std::time::Instant)>,
}

impl Entry {
    pub(super) fn new(name: &str, target: String, file: Option<u64>, update: Vec<u16>, step: Step) -> Self {
        Entry {
            name: name.to_string(),
            target,
            file,
            update,
            step,
            row: None,
            sent_ms: now_ms(),
            archive: None,
            started: false,
            author: None,
            thumbnail: None,
            cancel: None,
            place: None,
            server: None,
            cancelled: false,
            switched: None,
        }
    }

    fn downloading(&self) -> bool {
        matches!(self.step, Step::Queued | Step::Downloading(_) | Step::Paused(_))
    }

    /// Still moving: a second press does nothing.
    pub fn busy(&self) -> bool {
        self.downloading() || self.step == Step::Installing
    }
}

/// Every Browse install of the session, and which mods are installed.
#[derive(Default)]
pub struct Installs {
    pub items: BTreeMap<u64, Entry>,
    /// The one install worker. Its result carries the mod it was for.
    pub job: Job<(u64, Result<Finished, String>)>,
    running: Option<u64>,
    /// Installed GameBanana mods and their slot: the library store, then the install journal
    /// for slots the store does not name. Rebuilt when either or the addon folder changes.
    pub owned: HashMap<u64, u16>,
    owned_sig: Option<(bool, usize, usize, u64)>,
    /// The details' file menu beside its Install button, for this mod.
    pub files_menu: Option<u64>,
    /// A one-press Install that first asks GameBanana how many files the mod has: the mod id
    /// and its name, then the count of current files.
    ask_job: Job<(u64, String, usize)>,
    /// The mod whose details open with the file menu, once they have loaded.
    pub choose: Option<u64>,
    /// The review of a gameinfo.gi, .cfg or video.txt that came inside a mod.
    pub settings: settings_review::SettingsReview,
}

/// What the install worker reports.
pub struct Finished {
    pub slot: u16,
    pub journal: Vec<InstallJournalEntry>,
    /// It was to land on but stayed off for the mod safety gate: what the gate asks about.
    pub gated: Option<Vec<String>>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// --- what a button shows -------------------------------------------------------------------

/// One mod's install button.
#[derive(Clone, Debug, PartialEq)]
pub enum Button {
    Install,
    Retry(String),
    Queued,
    Downloading(Option<f32>),
    Paused(Option<f32>),
    Installing,
    Installed,
    Update,
    /// Installed, but switched off. The press turns it on.
    Enable,
}

impl Button {
    /// The button is mid-install: pressing it again does nothing.
    pub fn busy(&self) -> bool {
        matches!(
            self,
            Button::Queued | Button::Downloading(_) | Button::Paused(_) | Button::Installing
        )
    }

    /// The full words: "Downloading 40%".
    pub fn label(&self) -> String {
        match self {
            Button::Install => tr("Install").to_string(),
            Button::Retry(_) => tr("Retry").to_string(),
            Button::Queued => tr("Queued").to_string(),
            Button::Downloading(Some(f)) => fill(tr("Downloading {}%"), &[&percent(*f).to_string()]),
            Button::Downloading(None) => tr("Downloading").to_string(),
            Button::Paused(Some(f)) => fill(tr("Paused at {}%"), &[&percent(*f).to_string()]),
            Button::Paused(None) => tr("Paused").to_string(),
            Button::Installing => tr("Installing").to_string(),
            Button::Installed => tr("Installed").to_string(),
            Button::Update => tr("Update").to_string(),
            Button::Enable => tr("Enable").to_string(),
        }
    }

    /// The short form a card's button has room for: "40%". The card says the rest under it.
    pub fn short(&self) -> String {
        match self {
            Button::Downloading(Some(f)) | Button::Paused(Some(f)) => format!("{}%", percent(*f)),
            Button::Downloading(None) => "...".to_string(),
            Button::Paused(None) => tr("Paused").to_string(),
            other => other.label(),
        }
    }

    /// The download share, for the fill behind the label.
    pub fn fraction(&self) -> Option<f32> {
        match self {
            Button::Downloading(f) | Button::Paused(f) => Some(f.unwrap_or(0.0)),
            Button::Installing => Some(1.0),
            _ => None,
        }
    }
}

fn percent(f: f32) -> u32 {
    (f.clamp(0.0, 1.0) * 100.0).floor() as u32
}

/// The button for mod `id`: its own install first, then what Installed knows.
pub fn button(s: &State, id: u64) -> Button {
    if let Some(entry) = s.browse_view.installs.items.get(&id) {
        match &entry.step {
            Step::Queued => return Button::Queued,
            Step::Downloading(f) => return Button::Downloading(*f),
            Step::Paused(f) => return Button::Paused(*f),
            Step::Installing => return Button::Installing,
            Step::Failed(error) => return Button::Retry(error.clone()),
            Step::Done => return Button::Installed,
        }
    }
    if let Some(slot) = owned_slot(s, id) {
        let updates = &s.installed.updates;
        if updates.running == Some(slot) {
            return Button::Installing;
        }
        if updates.queue.contains(&slot) {
            return Button::Queued;
        }
        // Settings > Mods > Show mod update marks (DMM) hides the mark; Installed still updates.
        if s.shell.w4.update_badges && available_update(s, id).is_some() && !update_ignored(s, id) {
            return Button::Update;
        }
        if switched_off(s, id) {
            return Button::Enable;
        }
        return Button::Installed;
    }
    Button::Install
}

/// Every file of mod `id` in the addon folder is off. False before the folder is read.
pub fn switched_off(s: &State, id: u64) -> bool {
    if !s.addons.scanned {
        return false;
    }
    let slots = owned_slots(s, id);
    let present: Vec<&crate::model::AddonRow> =
        s.addons.addons.iter().filter(|a| slots.contains(&a.slot)).collect();
    !present.is_empty() && present.iter().all(|a| !a.enabled)
}

/// The slot an installed mod sits in.
pub fn owned_slot(s: &State, id: u64) -> Option<u16> {
    let installs = &s.browse_view.installs;
    if installs.owned_sig.is_some() {
        return installs.owned.get(&id).copied();
    }
    // Before the first poll: the library store alone, as Browse always read it.
    s.installed
        .library
        .mods
        .iter()
        .find(|meta| meta.gamebanana_id == Some(id))
        .map(|meta| meta.slot)
}

/// Every slot that holds mod `id`, for an update.
fn owned_slots(s: &State, id: u64) -> Vec<u16> {
    let mut slots: Vec<u16> = s
        .installed
        .library
        .mods
        .iter()
        .filter(|meta| meta.gamebanana_id == Some(id))
        .map(|meta| meta.slot)
        .filter(|slot| !s.addons.scanned || s.addons.addons.iter().any(|a| a.slot == *slot))
        .collect();
    if slots.is_empty() {
        slots.extend(owned_slot(s, id));
    }
    slots.sort_unstable();
    slots.dedup();
    slots
}

/// Rebuild [`Installs::owned`] when the library store, the journal or the addon folder changed.
pub fn refresh_owned(st: &mut State) -> bool {
    let addons = &st.addons;
    let sig = (
        addons.scanned,
        st.installed.library.mods.len(),
        st.installed.updates.journal.len(),
        addons
            .addons
            .iter()
            .map(|a| a.size_bytes.wrapping_add(u64::from(a.slot) << 48))
            .fold(addons.addons.len() as u64, u64::wrapping_add),
    );
    if st.browse_view.installs.owned_sig == Some(sig) {
        return false;
    }
    let library = &st.installed.library;
    let mut owned = HashMap::new();
    if addons.scanned {
        for addon in &addons.addons {
            let named = library
                .mods
                .iter()
                .rev()
                .find(|meta| meta.slot == addon.slot && meta.gamebanana_id.is_some())
                .and_then(|meta| meta.gamebanana_id);
            let journaled = || {
                st.installed
                    .updates
                    .journal
                    .iter()
                    .rev()
                    .find(|entry| entry.slot == addon.slot && entry.vpk_bytes == addon.size_bytes)
                    .and_then(deadlock_core::mod_updates::journal_mod_id)
            };
            if let Some(id) = named.or_else(journaled) {
                owned.entry(id).or_insert(addon.slot);
            }
        }
    } else {
        for meta in &library.mods {
            if let Some(id) = meta.gamebanana_id {
                owned.entry(id).or_insert(meta.slot);
            }
        }
    }
    st.browse_view.installs.owned = owned;
    st.browse_view.installs.owned_sig = Some(sig);
    true
}

// --- presses -------------------------------------------------------------------------------

/// A card's, a row's or the details' main button: install the newest file, update, retry, or
/// open Installed once it is there.
pub fn press(st: &mut State, id: u64, name: &str) {
    match button(st, id) {
        Button::Install => install_or_choose(st, id, name),
        Button::Retry(_) => retry(st, id),
        Button::Update => {
            let slots = owned_slots(st, id);
            start(st, id, name, id.to_string(), None, slots);
        }
        Button::Installed => {
            st.browse_view.context = None;
            st.go(Screen::Installed);
        }
        Button::Enable => {
            st.browse_view.context = None;
            if let Some(slot) = owned_slots(st, id).first().copied() {
                crate::screens::installed::enable::switch(st, slot, true);
            }
        }
        _ => {}
    }
}

/// One file of a mod, from the details: the newest goes by the mod id, any other by its link.
pub fn press_file(st: &mut State, id: u64, name: &str, target: String, file: u64) {
    if st.browse_view.installs.items.get(&id).is_some_and(Entry::busy) {
        return;
    }
    st.browse_view.installs.files_menu = None;
    start(st, id, name, target, Some(file), Vec::new());
}

/// The current files of a mod: what the file menu lists and Install picks from.
fn current_files(profile: &ModProfile) -> usize {
    profile.files.iter().filter(|f| !f.archived && !f.download_url.is_empty()).count()
}

/// A one-press Install by mod id. With one file it starts at once; with several the player
/// picks in the details' file menu instead of Sigil taking the newest (Deadlock Mod Manager's
/// file choice). The details, when open on this mod, already show the files: the main button
/// there installs the newest, the menu the others.
fn install_or_choose(st: &mut State, id: u64, name: &str) {
    let open = st.browse_view.details.as_ref().is_some_and(|d| d.id == id && d.profile.is_some());
    if open || st.game_root.is_none() || st.demo || rune::is_headless() || st.prefs.offline || st.browse_view.installs.ask_job.is_running() {
        start(st, id, name, id.to_string(), None, Vec::new());
        return;
    }
    let name = name.to_string();
    let wake = st.wake.clone();
    st.status = fill(tr("Checking the files of {}."), &[&name]);
    st.browse_view.installs.ask_job.start(wake, move || {
        // A failed read installs the newest file, as before; the queue reports its own errors.
        let count = gamebanana::fetch_mod_profile(id).map(|p| current_files(&p)).unwrap_or(1);
        Ok((id, name, count))
    });
}

/// The file count came back: install, or open the details with the file menu.
fn take_ask(st: &mut State) -> bool {
    let Some(Ok((id, name, count))) = st.browse_view.installs.ask_job.try_take() else {
        return false;
    };
    if count > 1 {
        st.status = fill(tr("{} has {} files. Pick the one to install."), &[&name, &count.to_string()]);
        super::detail_state::open_submission(st, BrowseSection::Mods, id);
        st.browse_view.installs.choose = Some(id);
    } else {
        start(st, id, &name, id.to_string(), None, Vec::new());
    }
    true
}

/// Open the file menu on the details once they have loaded (after a press, or a link that names
/// a mod with several files and no file).
fn open_chooser(st: &mut State) -> bool {
    let Some(id) = st.browse_view.installs.choose else { return false };
    let Some(d) = st.browse_view.details.as_ref() else {
        st.browse_view.installs.choose = None;
        return false;
    };
    if d.id != id {
        st.browse_view.installs.choose = None;
        return false;
    }
    if d.loading {
        return false;
    }
    st.browse_view.installs.choose = None;
    if d.profile.as_ref().is_some_and(|p| current_files(p) > 1) {
        st.browse_view.installs.files_menu = Some(id);
    }
    true
}

/// A link to a mod with no file named: open its file menu when it has several.
pub fn choose_after_load(st: &mut State, id: u64) {
    st.browse_view.installs.choose = Some(id);
}

fn retry(st: &mut State, id: u64) {
    let Some(entry) = st.browse_view.installs.items.remove(&id) else {
        return;
    };
    start(st, id, &entry.name, entry.target, entry.file, entry.update);
}

/// Send `target` to the download queue for mod `id`. The queue row and the install that
/// follows report on this mod's button only.
pub fn start(st: &mut State, id: u64, name: &str, target: String, file: Option<u64>, update: Vec<u16>) {
    crate::screens::settings::telemetry::event(st, "install:start", Some("page:browse"));
    let target = target.trim().to_string();
    if st.game_root.is_none() {
        fail(st, id, name, target, file, update, tr("Set the game folder in Settings to install.").to_string());
        return;
    }
    // Settings > Game > Block changes while the game runs. The notice says why.
    if crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    let mut entry = Entry::new(name, target.clone(), file, update, Step::Queued);
    let card = card_by_id(st, id);
    let details = st.browse_view.details.as_ref().filter(|d| d.id == id);
    entry.author = card
        .map(|c| c.author.clone())
        .or_else(|| details.map(|d| d.author.clone()))
        .filter(|a| !a.is_empty());
    entry.thumbnail = card
        .and_then(|c| c.thumbnail_url.clone())
        .or_else(|| details.and_then(|d| d.thumbnail_url.clone()));
    st.launch
        .downloads
        .ensure(crate::data::data_dir(), crate::data::workspace_dir(), st.wake.clone());
    if let Some(handle) = &st.launch.downloads.handle {
        // Marked for install, so a restart before it lands can still install it.
        handle.send(Command::AddInstall(target));
    }
    st.browse_view.installs.items.insert(id, entry);
    st.status = fill(tr("Downloading {}."), &[name]);
}

#[allow(clippy::too_many_arguments)]
fn fail(st: &mut State, id: u64, name: &str, target: String, file: Option<u64>, update: Vec<u16>, error: String) {
    let entry = Entry::new(name, target, file, update, Step::Failed(error.clone()));
    st.browse_view.installs.items.insert(id, entry);
    st.status = error.clone();
    crate::notify::push(
        st,
        crate::notify::Notice::warning(fill(tr("{} was not installed"), &[name])).detail(error),
    );
}

/// Take a queue row back after a restart: a one-press install whose download was cut off, or
/// landed and was never installed. Its button follows the row again and it installs when done.
pub fn adopt(st: &mut State, item: &deadlock_core::download_queue::Item) {
    let id = item.mod_id.or_else(|| item.target.trim().parse::<u64>().ok());
    let Some(id) = id else { return };
    if st.browse_view.installs.items.get(&id).is_some_and(Entry::busy) {
        return;
    }
    let step = match item.state {
        Row::Done => Step::Installing,
        Row::Paused => Step::Paused(item.fraction()),
        _ => Step::Queued,
    };
    let name = if item.name.trim().is_empty() { id.to_string() } else { item.name.clone() };
    let mut entry = Entry::new(&name, item.target.clone(), None, Vec::new(), step);
    entry.row = Some(item.id.clone());
    entry.archive = item.path.clone();
    st.browse_view.installs.items.insert(id, entry);
}

// --- the frame loop ------------------------------------------------------------------------

/// Follow the queue rows, take the worker's result and start the next install. Called from
/// Browse's poll every frame.
pub fn poll(st: &mut State) -> bool {
    let mut changed = take_result(st);
    changed |= take_ask(st);
    changed |= settings_review::poll(st);
    changed |= open_chooser(st);
    changed |= super::multi::poll(st);
    // The update check and the install journal, so cards can say Installed and Update.
    if st.screen == Screen::Browse {
        crate::screens::installed::check_updates(st, false);
    }
    changed |= refresh_owned(st);
    changed |= follow_queue(st);
    changed |= follow_updates(st);
    changed |= start_next(st);
    // Wave 4: featured picks, creator links and the rate-limit notice.
    changed |= super::w4::refresh_featured(st);
    changed |= super::w4::poll_author(st);
    changed |= super::w4::poll_rate_limit(st);
    changed |= super::paste::poll(st);
    changed
}

fn follow_queue(st: &mut State) -> bool {
    let installs = &mut st.browse_view.installs;
    if !installs.items.values().any(Entry::downloading) {
        return false;
    }
    let Some(handle) = st.launch.downloads.handle.as_ref() else {
        return false;
    };
    let queue = handle.snapshot();
    let mut changed = false;
    let mut gone = Vec::new();
    for (id, entry) in installs.items.iter_mut().filter(|(_, e)| e.downloading()) {
        let item = match &entry.row {
            Some(row) => queue.get(row),
            None => queue.items.iter().find(|item| {
                item.target == entry.target && item.added_ms().is_some_and(|at| at >= entry.sent_ms)
            }),
        };
        let Some(item) = item else {
            continue;
        };
        if entry.row.is_none() {
            entry.row = Some(item.id.clone());
        }
        let place = queue.position(&item.id);
        let server = item.server.as_deref().map(server_name);
        if entry.place != place || entry.server != server {
            if let (Some(before), Some(now)) = (&entry.server, &server)
                && before != now
            {
                entry.switched = Some((before.clone(), std::time::Instant::now()));
            }
            entry.place = place;
            entry.server = server;
            changed = true;
        }
        let next = match item.state {
            Row::Queued => Step::Queued,
            Row::Running => Step::Downloading(item.fraction()),
            Row::Paused => Step::Paused(item.fraction()),
            Row::Done => {
                entry.archive = item.path.clone();
                Step::Installing
            }
            Row::Failed => Step::Failed(
                item.error
                    .clone()
                    .unwrap_or_else(|| tr("The download stopped.").to_string()),
            ),
            Row::Cancelled => {
                gone.push(*id);
                continue;
            }
        };
        if next != entry.step {
            entry.step = next;
            changed = true;
        }
    }
    for id in gone {
        installs.items.remove(&id);
        changed = true;
    }
    changed
}

/// An update handed to Installed's queue is done when its slots left that queue.
fn follow_updates(st: &mut State) -> bool {
    let updates = &st.installed.updates;
    let mut changed = false;
    let mut gone = Vec::new();
    for (id, entry) in st.browse_view.installs.items.iter_mut() {
        if entry.update.is_empty() || !entry.started || entry.step != Step::Installing {
            continue;
        }
        let waiting = entry
            .update
            .iter()
            .any(|slot| updates.running == Some(*slot) || updates.queue.contains(slot));
        if waiting {
            // A stop asked from Installed counts the same as one asked here.
            if updates.running.is_some_and(|slot| entry.update.contains(&slot))
                && updates.cancel.load(std::sync::atomic::Ordering::Relaxed)
            {
                entry.cancelled = true;
            }
            continue;
        }
        if entry.cancelled {
            // Cancelled: the card goes back to its Update button, never Done.
            gone.push(*id);
            changed = true;
            continue;
        }
        entry.step = match entry.update.iter().find_map(|slot| updates.failed.get(slot)) {
            Some(error) => Step::Failed(error.clone()),
            None => Step::Done,
        };
        changed = true;
    }
    for id in gone {
        st.browse_view.installs.items.remove(&id);
    }
    changed
}

/// Start the oldest downloaded install, one at a time. The dialog's own install also holds
/// the line, so two plans never pick the same free slot.
fn start_next(st: &mut State) -> bool {
    if st.browse_view.installs.job.is_running() || st.install_job.is_running() {
        return false;
    }
    let Some(id) = st
        .browse_view
        .installs
        .items
        .iter()
        .find(|(_, e)| e.step == Step::Installing && !e.started)
        .map(|(id, _)| *id)
    else {
        return false;
    };
    let Some(entry) = st.browse_view.installs.items.get_mut(&id) else {
        return false;
    };
    entry.started = true;
    if !entry.update.is_empty() {
        // The archive is in the download cache now: Installed's update reuses it and swaps
        // the file in place, keeping the slot, its on or off state and an undo.
        let slots = entry.update.clone();
        // The update installs exactly what was downloaded: the mod id (its newest file, so the
        // journal keeps the mod), or for any other file that very archive.
        let target = match (&entry.archive, entry.target.parse::<u64>().is_ok()) {
            (Some(path), false) => path.display().to_string(),
            _ => entry.target.clone(),
        };
        // Only a file picked by hand is pinned; a plain Update lets Installed match each file
        // of a multi-file mod to its own successor.
        if entry.file.is_some() {
            for slot in &slots {
                st.installed.updates.targets.insert(*slot, target.clone());
            }
        }
        crate::screens::installed::queue_updates(st, slots.clone());
        let updates = &st.installed.updates;
        let moving = slots
            .iter()
            .any(|slot| updates.running == Some(*slot) || updates.queue.contains(slot));
        if !moving && let Some(entry) = st.browse_view.installs.items.get_mut(&id) {
            entry.step = Step::Failed(
                slots
                    .iter()
                    .find_map(|slot| updates.failed.get(slot).cloned())
                    .unwrap_or_else(|| tr("The update could not start. Open Installed to see why.").to_string()),
            );
        }
        return true;
    }
    if crate::screens::settings::watch::refuse_while_running(st) {
        if let Some(entry) = st.browse_view.installs.items.get_mut(&id) {
            entry.step = Step::Failed(tr("Deadlock is running").to_string());
        }
        return true;
    }
    let Some(game_root) = st.game_root.clone() else {
        if let Some(entry) = st.browse_view.installs.items.get_mut(&id) {
            entry.step = Step::Failed(tr("Set the game folder in Settings to install.").to_string());
        }
        return true;
    };
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    if let Some(entry) = st.browse_view.installs.items.get_mut(&id) {
        entry.cancel = Some(cancel.clone());
    }
    let entry = &st.browse_view.installs.items[&id];
    // A mod id plans by id, so the journal keeps where it came from (the update check reads
    // that) and the plan reuses the archive the queue just downloaded. A file link plans the
    // downloaded archive itself.
    let target = if entry.target.parse::<u64>().is_ok() {
        entry.target.clone()
    } else {
        match &entry.archive {
            Some(path) => path.display().to_string(),
            None => entry.target.clone(),
        }
    };
    let work = Work {
        id,
        target,
        name: entry.name.trim().to_string(),
        author: entry.author.clone(),
        thumbnail: entry.thumbnail.clone(),
        game_root,
        workspace: crate::data::workspace_dir(),
        library: deadlock_core::import::library_path(&crate::data::data_dir()),
        land_disabled: st.prefs.land_disabled,
        switch_variants: crate::screens::settings::options::switch_variants(),
        cancel,
    };
    st.browse_view.installs.running = Some(id);
    st.browse_view.installs.job.start(st.wake.clone(), move || {
        let id = work.id;
        Ok((id, install(work)))
    });
    true
}

/// Everything the install worker needs, read on the UI thread.
struct Work {
    id: u64,
    target: String,
    name: String,
    author: Option<String>,
    thumbnail: Option<String>,
    game_root: PathBuf,
    workspace: PathBuf,
    library: PathBuf,
    land_disabled: bool,
    switch_variants: bool,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// Plan, validate and write one install, then record it for Installed. The same steps as the
/// install dialog's Install press, without the stop between them.
fn install(work: Work) -> Result<Finished, String> {
    use deadlock_core::install::{
        ApplyOptions, InstallRequest, PlanOptions, apply_install_with, plan_install, read_journal,
        store_plan,
    };
    let request = InstallRequest::new(work.target.clone(), work.workspace.clone())
        .map_err(|e| e.to_string())?
        .with_one_click(true);
    let plan = plan_install(
        &request,
        &PlanOptions {
            game_root: Some(work.game_root.clone()),
            cancel: Some(work.cancel.clone()),
            ..Default::default()
        },
    )
    .map_err(super::plan_error)?;
    store_plan(&plan).map_err(|e| e.to_string())?;
    // Always written switched off; one that should land on is turned on only after the mod
    // safety scan has nothing to ask.
    let record = apply_install_with(&plan, true, ApplyOptions { land_disabled: true }).map_err(|e| e.to_string())?;
    let bytes = std::fs::metadata(&record.target).map(|m| m.len()).unwrap_or(0);
    let gated = if work.land_disabled {
        None
    } else {
        crate::screens::installed::safety_gate::land_checked(&work.game_root, plan.slot, &record.target, Some(work.id))
    };
    // The card or details showed no picture (a rail, a menu, a link): ask GameBanana, so the
    // Installed card has one.
    let thumbnail = work.thumbnail.clone().or_else(|| crate::screens::installed::preview_of(work.id));
    let meta = deadlock_core::import::ModMeta {
        slot: plan.slot,
        size_bytes: bytes,
        name: work.name.clone(),
        author: work.author,
        thumbnail_url: thumbnail,
        gamebanana_id: Some(work.id),
        source: "browse".to_string(),
        recorded_at: String::new(),
    };
    if !work.name.is_empty() {
        let _ = deadlock_core::import::record_meta(&work.library, meta.clone());
    }
    // Settings > Mods > Switch variants: other files of this mod turn off.
    if work.switch_variants
        && let Ok(lib) = deadlock_core::import::read_library(&work.library)
    {
        if let Ok(switched) = deadlock_core::mod_data::switch_variants(&work.game_root, &lib, work.id, plan.slot, deadlock_core::mod_data::may_enable_after_install(work.land_disabled, gated.is_some()), true) {
            crate::screens::installed::waiting::hold_from_worker(&work.game_root, &lib, &switched.held);
        }
    }
    // Settings > Experimental > Imprint installed mods.
    crate::screens::settings::tools::imprint_after_install(&work.game_root, plan.slot, &meta);
    Ok(Finished {
        slot: plan.slot,
        journal: read_journal(&work.workspace).unwrap_or_default(),
        gated,
    })
}

fn take_result(st: &mut State) -> bool {
    let Some(result) = st.browse_view.installs.job.try_take() else {
        return false;
    };
    let running = st.browse_view.installs.running.take();
    let (id, outcome) = match result {
        Ok(pair) => pair,
        Err(error) => match running {
            Some(id) => (id, Err(error)),
            None => return true,
        },
    };
    let Some(entry) = st.browse_view.installs.items.get(&id) else {
        return true;
    };
    let name = entry.name.clone();
    let row = entry.row.clone();
    let unpacked = settings_review::extracted_dir(&entry.target);
    match outcome {
        Ok(done) => {
            // A gameinfo.gi, .cfg or video.txt beside the VPK: offer the review.
            if let Some(dir) = unpacked.clone() {
                settings_review::start(st, dir, &name, Some(id), false);
            }
            if let Some(entry) = st.browse_view.installs.items.get_mut(&id) {
                entry.step = Step::Done;
            }
            // Browse and Installed read the new slot this frame, not after a rescan.
            st.installed.library = deadlock_core::import::read_library(
                &deadlock_core::import::library_path(&crate::data::data_dir()),
            )
            .unwrap_or_default();
            st.installed.library_sig = None;
            if st.installed.updates.loaded {
                st.installed.updates.journal = done.journal.clone();
            }
            crate::data::scan_addons_now(st);
            crate::screens::settings::telemetry::event(st, "install:done", Some("page:browse"));
            // Unknown-mod lookup (opt-in): add this file's hash to the index.
            crate::screens::settings::telemetry::contribute(st, id, done.journal.iter().rev().find(|entry| entry.slot == done.slot));
            st.browse_view.installs.owned.insert(id, done.slot);
            st.browse_view.installs.owned_sig = None;
            refresh_owned(st);
            crate::screens::one_skin::after_install(st, done.slot, &name);
            // The queue row did its job; Installed shows the mod from here.
            if let (Some(row), Some(handle)) = (row, &st.launch.downloads.handle) {
                handle.send(Command::Remove(row));
            }
            if let Some(findings) = done.gated.clone() {
                // Held off for the mod safety gate: say so, with the question one press away.
                st.status = fill(tr("{} was installed switched off"), &[&name]);
                crate::screens::installed::safety_gate::landed_off(st, done.slot, &name, findings);
            } else {
                let slot = format!("{:02}", done.slot);
                let detail = if st.prefs.land_disabled {
                    fill(tr("Slot {}. It is switched off: turn it on in Installed."), &[&slot])
                } else {
                    fill(tr("Slot {}. It loads the next time you start Deadlock."), &[&slot])
                };
                st.status = format!("{} {}", fill(tr("Installed {}."), &[&name]), detail);
                crate::notify::push(
                    st,
                    crate::notify::Notice::success(fill(tr("Installed {}"), &[&name]))
                        .detail(detail)
                        .action(tr("Open Installed"), |st: &mut State| st.go(Screen::Installed)),
                );
            }
        }
        Err(error) if error == super::CANCELLED => {
            crate::screens::settings::telemetry::event(st, "install:cancel", Some("page:browse"));
            // The player stopped it: the card goes back to Install, quietly.
            st.browse_view.installs.items.remove(&id);
            if let (Some(row), Some(handle)) = (row, &st.launch.downloads.handle) {
                handle.send(Command::Remove(row));
            }
            crate::notify::push(st, crate::notify::Notice::info(fill(tr("{} was cancelled"), &[&name])));
        }
        Err(error) if error.contains("more than one VPK") => {
            // A real choice: the archive holds several mods. The picker installs the one
            // picked straight away.
            let entry = st.browse_view.installs.items.remove(&id);
            let target = entry
                .as_ref()
                .map(|e| match (&e.archive, e.target.parse::<u64>().is_ok()) {
                    (Some(path), false) => path.display().to_string(),
                    _ => e.target.clone(),
                })
                .unwrap_or_else(|| id.to_string());
            let mut view = InstallView::for_mod(id, &name);
            view.target = target;
            view.auto = true;
            st.install = view;
            crate::data::start_vpk_scan(st);
        }
        Err(error) => {
            // Nothing to install but settings files: the review is the way to use them.
            if error.contains(deadlock_core::install::NOTHING_INSTALLABLE)
                && let Some(dir) = unpacked
            {
                settings_review::start(st, dir, &name, Some(id), true);
            }
            if let Some(entry) = st.browse_view.installs.items.get_mut(&id) {
                entry.step = Step::Failed(error.clone());
            }
            st.status = error.clone();
            crate::notify::push(
                st,
                crate::notify::Notice::warning(fill(tr("{} was not installed"), &[&name])).detail(error),
            );
        }
    }
    true
}

/// True while any install writes or waits to: the dialog's, Browse's, or an update.
pub fn installs_busy(s: &State) -> bool {
    s.install_job.is_running() || s.browse_view.installs.job.is_running()
}

// --- the look --------------------------------------------------------------------------------

/// How much room a button has.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Size {
    /// A card's action row: fills its slot, "40%" on the button, the words under the title.
    Fill,
    /// A card too narrow for words: the icon, or the percentage.
    Icon,
    /// A list row or a file row: the full words.
    Row,
    /// The details' main button: the full words, a little taller.
    Wide,
    /// A details file row: 28 high, 88 wide, a ghost while it only offers Install.
    File,
}

fn glyph(b: &Button) -> &'static Icon {
    match b {
        Button::Install | Button::Downloading(_) | Button::Paused(_) => &icons::DOWNLOAD,
        Button::Retry(_) => &icons::ROTATE_CCW,
        Button::Queued => &CLOCK,
        Button::Installing => &icons::LOADER,
        Button::Installed => &icons::CHECK,
        Button::Update => &icons::REFRESH_CW,
        Button::Enable => &icons::POWER,
    }
}

/// A file server's short name: `filecache9` for `filecache9.gamebanana.com`, as Grimoire
/// names it; any other host as it is.
pub fn server_name(host: &str) -> String {
    let host = host.trim().trim_start_matches("https://").trim_start_matches("http://");
    let host = host.split('/').next().unwrap_or(host);
    match host.split_once('.') {
        Some((first, rest)) if rest.ends_with("gamebanana.com") && first != "gamebanana" => first.to_string(),
        _ => host.to_string(),
    }
}

/// The fill behind a downloading label, `fraction` of the width from the left.
fn progress_fill(fraction: f32, color: Color) -> El {
    let f = fraction.clamp(0.0, 1.0);
    row()
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .child(el().h_full().grow(f).fill(color))
        .child(el().h_full().grow(1.0 - f))
        .a11y_hidden()
}

/// The install button for mod `id` in state `b`. `key` keeps it apart from every other
/// card's; pressing it acts on `id` alone.
pub fn button_el(u: &Ui, b: &Button, key: String, id: u64, name: &str, size: Size) -> El {
    let (ink, glyph_ink, fill_color, border) = match b {
        Button::Install if size == Size::File => (u.prose, u.muted, Color::TRANSPARENT, u.hair),
        // The details window's main button is its one solid action.
        Button::Install | Button::Update | Button::Enable if size == Size::Wide => (u.accent_ink, u.accent_ink, u.accent, u.accent),
        Button::Install => (u.ink, u.ink, Color::WHITE.alpha(0.035), u.hair_strong),
        Button::Retry(_) => (u.ink, u.rose, u.rose.alpha(0.08), u.rose.alpha(0.45)),
        Button::Queued => (u.muted, u.faint, Color::TRANSPARENT, u.hair_strong),
        Button::Downloading(_) | Button::Installing => (u.ink, u.accent, u.accent.alpha(0.08), u.accent.alpha(0.45)),
        Button::Paused(_) => (u.muted, u.muted, Color::WHITE.alpha(0.02), u.hair_strong),
        Button::Installed => (u.muted, u.teal, Color::TRANSPARENT, u.hair),
        Button::Update => (u.ink, u.ink, Color::WHITE.alpha(0.035), u.hair_strong),
        Button::Enable => (u.ink, u.teal, Color::WHITE.alpha(0.035), u.hair_strong),
    };
    let (h, words) = match size {
        Size::Fill => (28.0, Some(b.short())),
        Size::Icon => (
            28.0,
            matches!(b, Button::Downloading(Some(_)) | Button::Paused(Some(_))).then(|| b.short()),
        ),
        Size::Row => (32.0, Some(b.label())),
        Size::File => (28.0, Some(b.short())),
        Size::Wide => (32.0, Some(b.label())),
    };
    let radius = crate::ui::r(8.0);
    let mut button = row()
        .key(key)
        .h(h)
        .gap(6.0)
        .center()
        .radius(radius)
        .clip_rounded(radius)
        .fill(fill_color)
        .border(1.0, border)
        .role("button")
        .label(format!("{} {name}", b.label()));
    button = match (&words, size) {
        (None, _) => button.w(h),
        (Some(_), Size::Wide) => button.px(16.0).min_w(148.0),
        (Some(_), Size::Row) => button.pl(10.0).pr(12.0).min_w(112.0),
        (Some(_), Size::File) => button.w(88.0).px(8.0),
        (Some(_), Size::Fill) => button.grow(1.0).shrink().min_w(0.0).px(8.0),
        (Some(_), _) => button.pl(8.0).pr(10.0),
    };
    if let Some(fraction) = b.fraction() {
        let color = if matches!(b, Button::Paused(_)) {
            Color::WHITE.alpha(0.08)
        } else {
            u.accent.alpha(if *b == Button::Installing { 0.10 } else { 0.26 })
        };
        button = button.child(progress_fill(fraction, color));
    }
    let size_px = if size == Size::Wide { 16.0 } else { 14.0 };
    if !(size == Size::Icon && words.is_some()) {
        button = button.child(icon(glyph(b), size_px).color(glyph_ink).a11y_hidden());
    }
    if let Some(words) = words {
        let label = text(words)
            .font(if size == Size::Wide { BODY_MED } else { LABEL })
            .color(ink)
            .a11y_hidden();
        // A long German state truncates inside the slot; the line above the button has it whole.
        button = button.child(if size == Size::Fill { label.truncate().min_w(0.0).shrink() } else { label });
    }
    if b.busy() {
        // A press on a busy button cancels it: the queue row while it downloads, the plan
        // while it installs. It stays a target, so the card behind it does not open.
        let label = fill(tr("Cancel {}"), &[name]);
        return button
            .label(label)
            .hover(u.hover)
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .child(icon(&icons::X, 12.0).color(u.faint).a11y_hidden())
            .on_press(move |st: &mut State| cancel(st, id));
    }
    let name = name.to_string();
    button
        .hover(u.hover)
        .press(Color::WHITE.alpha(0.08))
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| press(st, id, &name))
}

/// The install button drawn on art (the billboard, a row tile's hover layer): the same
/// states, labels, progress and presses as `button_el`, in white on any picture. Install is a
/// solid white fill with near-black ink, the strongest control on the landing without taking
/// the shell's orange.
pub fn button_on_art(u: &Ui, b: &Button, key: String, id: u64, name: &str, h: f32, min_w: f32) -> El {
    let dark = crate::screens::browse::top::INK_ON_WHITE;
    let (ink, fill_color, border) = match b {
        Button::Install | Button::Update | Button::Enable => (dark, Color::WHITE, Color::WHITE),
        Button::Retry(_) => (dark, Color::WHITE, u.rose),
        Button::Installed => (Color::WHITE, Color::WHITE.alpha(0.16), Color::WHITE.alpha(0.16)),
        Button::Downloading(_) | Button::Installing | Button::Queued | Button::Paused(_) => {
            (Color::WHITE, Color::BLACK.alpha(0.45), Color::WHITE.alpha(0.18))
        }
    };
    let radius = crate::ui::r(8.0);
    let mut button = row()
        .key(key)
        .h(h)
        .min_w(min_w)
        .px(12.0)
        .gap(6.0)
        .center()
        .radius(radius)
        .clip_rounded(radius)
        .fill(fill_color)
        .border(1.0, border)
        .role("button")
        .label(format!("{} {name}", b.label()));
    if let Some(fraction) = b.fraction() {
        let color = if matches!(b, Button::Paused(_)) { Color::WHITE.alpha(0.10) } else { u.accent.alpha(0.45) };
        button = button.child(progress_fill(fraction, color));
    }
    button = button
        .child(icon(glyph(b), 14.0).color(ink).a11y_hidden())
        .child(text(b.label()).font(LABEL).color(ink).truncate().min_w(0.0).shrink().a11y_hidden());
    if b.busy() {
        let label = fill(tr("Cancel {}"), &[name]);
        return button
            .label(label)
            .hover(Color::WHITE.alpha(0.10))
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .child(icon(&icons::X, 12.0).color(Color::WHITE.alpha(0.7)).a11y_hidden())
            .on_press(move |st: &mut State| cancel(st, id));
    }
    let name = name.to_string();
    button
        .hover(Color::BLACK.alpha(0.10))
        .press(Color::BLACK.alpha(0.16))
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| press(st, id, &name))
}

/// Cancel one Browse install: its download queue row while it waits or streams (the queue
/// removes the partial file), or the install worker's plan while it extracts and validates.
pub fn cancel(st: &mut State, id: u64) {
    // An update started from Browse runs in Installed's queue: cancel it there (A5). A running
    // update that was asked to stop shows Cancelling, never Done.
    if !st.browse_view.installs.items.get(&id).is_some_and(Entry::busy) {
        let updates = &st.installed.updates;
        if let Some(slot) = owned_slots(st, id)
            .into_iter()
            .find(|slot| updates.running == Some(*slot) || updates.queue.contains(slot))
        {
            crate::screens::installed::cancel_update(st, slot);
            return;
        }
    }
    let Some(entry) = st.browse_view.installs.items.get(&id) else { return };
    if entry.started && !entry.update.is_empty() && entry.step == Step::Installing {
        let slots = entry.update.clone();
        for slot in slots {
            crate::screens::installed::cancel_update(st, slot);
        }
        if let Some(entry) = st.browse_view.installs.items.get_mut(&id) {
            entry.cancelled = true;
        }
        return;
    }
    match entry.step {
        Step::Queued | Step::Downloading(_) | Step::Paused(_) => match entry.row.clone() {
            Some(row) => st.launch.downloads.send(Command::Cancel(row)),
            None => {
                // The queue has not named its row yet: forget the entry, and the row it makes
                // is cancelled by the target on the next follow.
                let target = entry.target.clone();
                st.browse_view.installs.items.remove(&id);
                if let Some(row) = st.launch.downloads.snapshot().items.iter().rev().find(|item| item.target == target && !item.state.finished()) {
                    st.launch.downloads.send(Command::Cancel(row.id.clone()));
                }
            }
        },
        Step::Installing => {
            if let Some(flag) = &entry.cancel {
                flag.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        _ => {}
    }
}

/// The line a card or row shows instead of its byline while an install moves or after it
/// failed: "Downloading 40%", "Installing", the error.
pub fn status_line(s: &State, u: &Ui, id: u64) -> Option<El> {
    let (words, color) = status_words(s, u, id)?;
    Some(
        row()
            .key(format!("install-status-{id}"))
            .w_full()
            .min_w(0.0)
            .gap(6.0)
            .center_y()
            .child(
                text(words.clone())
                    .font(SMALL)
                    .color(color)
                    .truncate()
                    .min_w(0.0)
                    .shrink()
                    .label(words),
            ),
    )
}

/// A paused or failed install as a tag along the bottom of a card's image, so the card's text
/// keeps its fields in their places. Queued, downloading and installing say so on the card's
/// own button and bar, so they get no tag. An error wraps to two lines.
pub fn status_chip(s: &State, u: &Ui, id: u64) -> Option<El> {
    if !matches!(button(s, id), Button::Paused(_) | Button::Retry(_)) {
        return None;
    }
    let (words, color) = status_words(s, u, id)?;
    // On a darkened image in every theme: muted words read as white.
    let color = if color == u.muted { Color::WHITE.alpha(0.86) } else { color };
    Some(
        row()
            .key(format!("install-chip-{id}"))
            .pin(Some(8.0), None, Some(8.0), Some(11.0))
            .child(
                row()
                    .px(6.0)
                    .py(2.0)
                    .min_w(0.0)
                    .shrink()
                    .center_y()
                    .radius(crate::ui::r(4.0))
                    .fill(Color::BLACK.alpha(0.72))
                    .border(1.0, Color::WHITE.alpha(0.12))
                    .child(
                        text(words.clone())
                            .font(SMALL)
                            .color(color)
                            .wrap()
                            .max_lines(2)
                            .min_w(0.0)
                            .shrink()
                            .label(words),
                    ),
            ),
    )
}

fn status_words(s: &State, u: &Ui, id: u64) -> Option<(String, Color)> {
    let b = button(s, id);
    let (words, color) = match &b {
        Button::Retry(error) => (error.clone(), u.rose),
        Button::Queued => match s.browse_view.installs.items.get(&id).and_then(|e| e.place) {
            Some(place) => (fill(tr("Queued #{}, waiting for its turn"), &[&place.to_string()]), u.muted),
            None => (tr("Waiting for its turn").to_string(), u.muted),
        },
        Button::Downloading(_) => {
            let entry = s.browse_view.installs.items.get(&id);
            let switched = entry
                .and_then(|e| e.switched.as_ref())
                .filter(|(_, at)| at.elapsed() < crate::screens::downloads::SWITCH_NOTE)
                .map(|(from, _)| from.clone());
            match (entry.and_then(|e| e.server.clone()), switched) {
                (Some(server), Some(from)) => (fill(tr("{} from {}, switched from {}"), &[&b.label(), &server, &from]), u.accent),
                (Some(server), None) => (fill(tr("{} from {}"), &[&b.label(), &server]), u.accent),
                _ => (b.label(), u.accent),
            }
        }
        Button::Installing => (b.label(), u.accent),
        Button::Paused(_) => (b.label(), u.muted),
        _ => return None,
    };
    Some((words, color))
}

/// A thin bar along the bottom of a card's image while its archive downloads or installs.
pub fn image_bar(s: &State, u: &Ui, id: u64) -> Option<El> {
    let b = button(s, id);
    let fraction = b.fraction()?;
    Some(
        row()
            .key(format!("install-bar-{id}"))
            .pin(Some(0.0), None, Some(0.0), Some(0.0))
            .h(3.0)
            .fill(Color::BLACK.alpha(0.35))
            // A paused download keeps its place but not the accent: only a moving one is orange.
            .child(el().h_full().grow(fraction.clamp(0.0, 1.0)).fill(if matches!(b, Button::Paused(_)) { u.muted } else { u.accent }))
            .child(el().h_full().grow(1.0 - fraction.clamp(0.0, 1.0)))
            .a11y_hidden(),
    )
}

// --- the details ---------------------------------------------------------------------------

/// The newest file that is not archived: what the main button installs.
pub fn primary_file(profile: &ModProfile) -> Option<&ModFile> {
    let mut best: Option<&ModFile> = None;
    for file in profile.files.iter().filter(|f| !f.archived) {
        if best.is_none_or(|b| file.added_ts > b.added_ts) {
            best = Some(file);
        }
    }
    best
}

/// What a file installs from: the mod id for the newest file of a mod, the file's link for
/// any other file and for every sound or WiP file.
pub fn file_target(d: &Details, profile: &ModProfile, file: &ModFile) -> String {
    let newest = primary_file(profile).is_some_and(|p| p.id == file.id);
    if newest && d.section == BrowseSection::Mods {
        d.id.to_string()
    } else {
        file.download_url.clone()
    }
}

/// The details' main row: one Install button that starts at once, a chevron with the other
/// files when there are several, and the error of a failed try under it.
pub fn details_install(s: &State, u: &Ui, d: &Details, profile: &ModProfile) -> Option<El> {
    let primary = primary_file(profile)?;
    let id = d.id;
    let b = button(s, id);
    // A sound or WiP has no newest-file shortcut by id: its main button installs the newest
    // file's link.
    let by_link = d.section != BrowseSection::Mods;
    let mut main = button_el(u, &b, format!("details-install-{id}"), id, &d.name, Size::Wide);
    if by_link && matches!(b, Button::Install | Button::Retry(_)) {
        let name = d.name.clone();
        let target = file_target(d, profile, primary);
        let file = primary.id;
        main = main.on_press(move |st: &mut State| {
            st.browse_view.installs.items.remove(&id);
            press_file(st, id, &name, target.clone(), file);
        });
    }
    // One full-width button; the Files list under it picks any other file.
    let main = main.w_full();
    let which = s
        .browse_view
        .installs
        .items
        .get(&id)
        .and_then(|e| e.file)
        .and_then(|file| profile.files.iter().find(|f| f.id == file))
        .unwrap_or(primary);
    let mut line = col().key("details-install-row").w_full().gap(6.0).child(main);
    // Which file it installs, when no Files list below names it.
    if profile.files.len() == 1 {
        line = line.child(
            text(format!("{} · {}", which.name, crate::model::format_size(which.size_bytes)))
                .font(crate::ui::SMALL_NUM)
                .color(u.muted)
                .truncate(),
        );
    }
    let mut c = col().key("details-install").w_full().gap(6.0).child(line);
    if let Button::Retry(error) = &b {
        c = c.child(
            row()
                .w_full()
                .gap(8.0)
                .items_start()
                .child(icon(&icons::TRIANGLE_ALERT, 14.0).color(u.rose).mt(2.0).a11y_hidden())
                .child(
                    text(error.clone())
                        .font(SMALL)
                        .color(u.prose)
                        .wrap_anywhere()
                        .selectable(true)
                        .grow(1.0)
                        .min_w(0.0),
                ),
        );
    }
    if b == Button::Installed && s.prefs.land_disabled && s.browse_view.installs.items.contains_key(&id) {
        c = c.child(
            text(tr("It is switched off. Turn it on in Installed."))
                .font(SMALL)
                .color(u.muted)
                .wrap(),
        );
    }
    Some(c)
}

/// A file row's button: this file's own state while it installs, Install otherwise.
pub fn file_button(s: &State, u: &Ui, d: &Details, profile: &ModProfile, file: &ModFile) -> El {
    let id = d.id;
    let entry = s.browse_view.installs.items.get(&id);
    let newest = primary_file(profile).is_some_and(|p| p.id == file.id);
    let mine = entry.is_some_and(|e| e.file == Some(file.id) || (e.file.is_none() && newest));
    let b = if mine { button(s, id) } else { Button::Install };
    let key = format!("file-install-{}", file.id);
    if mine && !matches!(b, Button::Install | Button::Retry(_)) {
        let mut el = button_el(u, &b, key, id, &file.name, Size::File);
        if b == Button::Installed {
            // The file is in; its button rests.
            el = el.on_press(|_st: &mut State| {});
        }
        return el;
    }
    let busy = entry.is_some_and(Entry::busy);
    let name = d.name.clone();
    let target = file_target(d, profile, file);
    let file_id = file.id;
    let el = button_el(u, &b, key, id, &file.name, Size::File);
    if busy {
        // Another file of this mod is on its way; one at a time.
        return el.opacity(0.45).on_press(|_st: &mut State| {});
    }
    el.on_press(move |st: &mut State| {
        st.browse_view.installs.items.remove(&id);
        press_file(st, id, &name, target.clone(), file_id);
    })
}

/// The chevron's menu: every current file, newest first, each one installs at once.
pub fn files_menu(s: &State, u: &Ui) -> Option<El> {
    let id = s.browse_view.installs.files_menu?;
    let d = s.browse_view.details.as_ref().filter(|d| d.id == id)?;
    let profile = d.profile.as_ref()?;
    let mut files: Vec<&ModFile> = profile.files.iter().collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.added_ts));
    let newest = primary_file(profile).map(|f| f.id);
    let mut items = col().w_full();
    for file in files {
        let target = file_target(d, profile, file);
        let name = d.name.clone();
        let file_id = file.id;
        let mut detail = crate::model::format_size(file.size_bytes);
        if file.added_ts > 0 {
            detail = format!("{detail} · {}", gamebanana::calendar_date(file.added_ts));
        }
        items = items.child(
            row()
                .key(format!("details-file-pick-{file_id}"))
                .w_full()
                .min_h(44.0)
                .px(10.0)
                .py(6.0)
                .gap(10.0)
                .center_y()
                .radius(crate::ui::r(6.0))
                .hover(u.hover)
                .press(u.over.alpha(0.08))
                .focusable(true)
                .role("menuitem")
                .label(fill(tr("Install {}"), &[&file.name]))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    st.browse_view.installs.items.remove(&id);
                    press_file(st, id, &name, target.clone(), file_id);
                })
                .child(icon(&icons::DOWNLOAD, 14.0).color(u.muted).a11y_hidden())
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(text(file.name.clone()).font(BODY).color(u.ink).truncate())
                        .child(text(detail).font(SMALL).color(u.faint).truncate()),
                )
                .child(if Some(file_id) == newest {
                    text(tr("Newest")).font(SMALL).color(u.muted)
                } else {
                    el()
                }),
        );
    }
    Some(
        col()
            .key("details-files-menu")
            .role("menu")
            .w(320.0)
            .padding(4.0)
            .fill(u.raised)
            .radius(crate::ui::r(10.0))
            .border(1.0, u.hair_strong)
            .shadow(12.0, 32.0, crate::ui::soft(Color::BLACK.alpha(0.5)))
            .anchored(format!("details-install-more-{id}"), Side::Bottom, Align::End, 6.0)
            .transition(crate::motion::pop(false))
            .modal(true, |st: &mut State| st.browse_view.installs.files_menu = None)
            .on_key(|st: &mut State, ev: &KeyEvent, _| {
                if ev.key == Key::Escape {
                    st.browse_view.installs.files_menu = None;
                    true
                } else {
                    false
                }
            })
            .child(items),
    )
}

// --- scenes ----------------------------------------------------------------------------------

/// Put mod `id` in `step` without a queue or a worker, for the scenes and the tests.
pub fn stage(st: &mut State, id: u64, name: &str, step: Step) {
    let mut entry = Entry::new(name, id.to_string(), None, Vec::new(), step);
    entry.started = true;
    st.browse_view.installs.items.insert(id, entry);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installing_one_mod_leaves_every_other_button_alone() {
        let mut st = State::default();
        st.game_root = Some(PathBuf::from("/nonexistent/sigil-test-game"));
        assert_eq!(button(&st, 1), Button::Install);
        assert_eq!(button(&st, 2), Button::Install);
        stage(&mut st, 1, "Mod A", Step::Downloading(Some(0.4)));
        assert_eq!(button(&st, 1), Button::Downloading(Some(0.4)));
        assert_eq!(button(&st, 2), Button::Install, "mod B keeps its own button");
        stage(&mut st, 1, "Mod A", Step::Installing);
        assert_eq!(button(&st, 2), Button::Install);
        stage(&mut st, 2, "Mod B", Step::Failed("no network".into()));
        assert_eq!(button(&st, 1), Button::Installing, "a failure on B leaves A installing");
        assert_eq!(button(&st, 2), Button::Retry("no network".into()));
        stage(&mut st, 1, "Mod A", Step::Done);
        assert_eq!(button(&st, 1), Button::Installed);
        assert_eq!(button(&st, 2), Button::Retry("no network".into()));
    }

    #[test]
    fn a_press_on_a_busy_button_does_nothing() {
        let mut st = State::default();
        stage(&mut st, 1, "Mod A", Step::Downloading(Some(0.4)));
        press(&mut st, 1, "Mod A");
        assert_eq!(button(&st, 1), Button::Downloading(Some(0.4)));
        assert_eq!(st.browse_view.installs.items.len(), 1);
    }

    #[test]
    fn without_a_game_folder_the_press_fails_on_that_card_only() {
        let mut st = State::default();
        st.game_root = None;
        press(&mut st, 7, "Mod A");
        assert!(matches!(button(&st, 7), Button::Retry(_)));
        assert_eq!(button(&st, 8), Button::Install);
    }

    #[test]
    fn library_and_journal_mods_read_as_installed() {
        let mut st = State::default();
        st.installed.library.mods.push(deadlock_core::import::ModMeta {
            slot: 3,
            size_bytes: 10,
            name: "Kept".into(),
            author: None,
            thumbnail_url: None,
            gamebanana_id: Some(41),
            source: "browse".into(),
            recorded_at: String::new(),
        });
        // Before the addon folder is read, the store alone answers.
        assert_eq!(owned_slot(&st, 41), Some(3));
        assert_eq!(button(&st, 41), Button::Installed);
        assert_eq!(button(&st, 42), Button::Install);
    }

    #[test]
    fn labels_read_as_one_sequence() {
        assert_eq!(server_name("filecache9.gamebanana.com"), "filecache9");
        assert_eq!(server_name("https://files.gamebanana.com/mods/x.zip"), "files");
        assert_eq!(server_name("gamebanana.com"), "gamebanana.com");
        assert_eq!(server_name("mirror.example.org"), "mirror.example.org");
        assert_eq!(Button::Install.label(), "Install");
        assert_eq!(Button::Downloading(Some(0.4)).label(), "Downloading 40%");
        assert_eq!(Button::Downloading(Some(0.4)).short(), "40%");
        assert_eq!(Button::Installing.label(), "Installing");
        assert_eq!(Button::Installed.label(), "Installed");
        assert!(Button::Queued.busy() && !Button::Installed.busy());
        assert_eq!(Button::Enable.label(), "Enable");
        assert!(!Button::Enable.busy());
    }

    #[test]
    fn an_installed_mod_that_is_off_offers_enable() {
        let mut st = State::default();
        st.installed.library.mods.push(deadlock_core::import::ModMeta {
            slot: 4,
            size_bytes: 10,
            name: "Off one".into(),
            author: None,
            thumbnail_url: None,
            gamebanana_id: Some(77),
            source: "browse".into(),
            recorded_at: String::new(),
        });
        st.addons.scanned = true;
        st.addons.addons.push(crate::model::AddonRow {
            slot: 4,
            file: "pak04_dir.vpk.disabled".into(),
            size_bytes: 10,
            chunk_files: Vec::new(),
            entry_count: None,
            enabled: false,
        });
        refresh_owned(&mut st);
        assert_eq!(button(&st, 77), Button::Enable);
        st.addons.addons[0].enabled = true;
        st.browse_view.installs.owned_sig = None;
        refresh_owned(&mut st);
        assert_eq!(button(&st, 77), Button::Installed, "on again, it is just installed");
    }
}
