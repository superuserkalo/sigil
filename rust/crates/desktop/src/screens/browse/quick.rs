//! One-press installs from Browse.
//!
//! Install on a card, a list row or the mod details starts at once: the archive goes through
//! the download queue (`deadlock_core::download_queue`, the rows on the Downloads page), then a
//! worker plans and applies it with the same validation, backups and atomic writes as the
//! install dialog. The press is the approval (ADR 0011).
//!
//! State is kept per mod in [`Installs::items`], keyed by the GameBanana id, so one install
//! never changes another card. A button reads [`button`]: Install, Queued, Downloading NN%,
//! Installing, then Installed (or Update when a newer file is out, Retry after a failure).
//! Installs run one at a time so two plans never pick the same free slot; downloads queue.
//! Updates of an installed mod download here too, then hand the slot to Installed's update
//! queue, which replaces the file in place.

use std::collections::{BTreeMap, HashMap};

use deadlock_core::download_queue::{Command, State as Row};
use deadlock_core::install::InstallJournalEntry;

use super::*;
use crate::screens::settings::i18n::{fill, tr};

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
    pub update: Vec<u8>,
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
}

impl Entry {
    fn new(name: &str, target: String, file: Option<u64>, update: Vec<u8>, step: Step) -> Self {
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
    pub owned: HashMap<u64, u8>,
    owned_sig: Option<(bool, usize, usize, u64)>,
    /// The details' file menu beside its Install button, for this mod.
    pub files_menu: Option<u64>,
}

/// What the install worker reports.
pub struct Finished {
    pub slot: u8,
    pub journal: Vec<InstallJournalEntry>,
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
        if available_update(s, id).is_some() && !update_ignored(s, id) {
            return Button::Update;
        }
        return Button::Installed;
    }
    Button::Install
}

/// The slot an installed mod sits in.
pub fn owned_slot(s: &State, id: u64) -> Option<u8> {
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
fn owned_slots(s: &State, id: u64) -> Vec<u8> {
    let mut slots: Vec<u8> = s
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
        Button::Install => start(st, id, name, id.to_string(), None, Vec::new()),
        Button::Retry(_) => retry(st, id),
        Button::Update => {
            let slots = owned_slots(st, id);
            start(st, id, name, id.to_string(), None, slots);
        }
        Button::Installed => {
            st.browse_view.context = None;
            st.go(Screen::Installed);
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

fn retry(st: &mut State, id: u64) {
    let Some(entry) = st.browse_view.installs.items.remove(&id) else {
        return;
    };
    start(st, id, &entry.name, entry.target, entry.file, entry.update);
}

/// Send `target` to the download queue for mod `id`. The queue row and the install that
/// follows report on this mod's button only.
pub fn start(st: &mut State, id: u64, name: &str, target: String, file: Option<u64>, update: Vec<u8>) {
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
        handle.send(Command::Add(target));
    }
    st.browse_view.installs.items.insert(id, entry);
    st.status = fill(tr("Downloading {}."), &[name]);
}

#[allow(clippy::too_many_arguments)]
fn fail(st: &mut State, id: u64, name: &str, target: String, file: Option<u64>, update: Vec<u8>, error: String) {
    let entry = Entry::new(name, target, file, update, Step::Failed(error.clone()));
    st.browse_view.installs.items.insert(id, entry);
    st.status = error.clone();
    crate::notify::push(
        st,
        crate::notify::Notice::warning(fill(tr("{} was not installed"), &[name])).detail(error),
    );
}

// --- the frame loop ------------------------------------------------------------------------

/// Follow the queue rows, take the worker's result and start the next install. Called from
/// Browse's poll every frame.
pub fn poll(st: &mut State) -> bool {
    let mut changed = take_result(st);
    // The update check and the install journal, so cards can say Installed and Update.
    if st.screen == Screen::Browse {
        crate::screens::installed::check_updates(st, false);
    }
    changed |= refresh_owned(st);
    changed |= follow_queue(st);
    changed |= follow_updates(st);
    changed |= start_next(st);
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
    for entry in st.browse_view.installs.items.values_mut() {
        if entry.update.is_empty() || !entry.started || entry.step != Step::Installing {
            continue;
        }
        let waiting = entry
            .update
            .iter()
            .any(|slot| updates.running == Some(*slot) || updates.queue.contains(slot));
        if waiting {
            continue;
        }
        entry.step = match entry.update.iter().find_map(|slot| updates.failed.get(slot)) {
            Some(error) => Step::Failed(error.clone()),
            None => Step::Done,
        };
        changed = true;
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
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())?;
    store_plan(&plan).map_err(|e| e.to_string())?;
    let record = apply_install_with(
        &plan,
        true,
        ApplyOptions {
            land_disabled: work.land_disabled,
        },
    )
    .map_err(|e| e.to_string())?;
    let bytes = std::fs::metadata(&record.target).map(|m| m.len()).unwrap_or(0);
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
        let _ = deadlock_core::mod_data::switch_variants(&work.game_root, &lib, work.id, plan.slot, true);
    }
    // Settings > Experimental > Imprint installed mods.
    crate::screens::settings::tools::imprint_after_install(&work.game_root, plan.slot, &meta);
    Ok(Finished {
        slot: plan.slot,
        journal: read_journal(&work.workspace).unwrap_or_default(),
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
    match outcome {
        Ok(done) => {
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
                st.installed.updates.journal = done.journal;
            }
            crate::data::scan_addons_now(st);
            st.browse_view.installs.owned.insert(id, done.slot);
            st.browse_view.installs.owned_sig = None;
            refresh_owned(st);
            // The queue row did its job; Installed shows the mod from here.
            if let (Some(row), Some(handle)) = (row, &st.launch.downloads.handle) {
                handle.send(Command::Remove(row));
            }
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
        Err(error) if error.contains("more than one pakNN_dir.vpk") => {
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
    /// A card's footer: "40%" on the button, the words under the title.
    Card,
    /// A card too narrow for words: the icon, or the percentage.
    Icon,
    /// A list row or a file row: the full words.
    Row,
    /// The details' main button: the full words, a little taller.
    Wide,
}

fn glyph(b: &Button) -> &'static Icon {
    match b {
        Button::Install | Button::Downloading(_) | Button::Paused(_) => &icons::DOWNLOAD,
        Button::Retry(_) => &icons::ROTATE_CCW,
        Button::Queued => &CLOCK,
        Button::Installing => &icons::LOADER,
        Button::Installed => &icons::CHECK,
        Button::Update => &icons::REFRESH_CW,
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
        Button::Install => (u.ink, u.ink, Color::WHITE.alpha(0.035), u.hair_strong),
        Button::Retry(_) => (u.ink, u.rose, u.rose.alpha(0.08), u.rose.alpha(0.45)),
        Button::Queued => (u.muted, u.faint, Color::TRANSPARENT, u.hair_strong),
        Button::Downloading(_) | Button::Installing => (u.ink, u.accent, u.accent.alpha(0.08), u.accent.alpha(0.45)),
        Button::Paused(_) => (u.muted, u.muted, Color::WHITE.alpha(0.02), u.hair_strong),
        Button::Installed => (u.muted, u.teal, Color::TRANSPARENT, u.hair),
        Button::Update => (u.ink, u.accent, u.accent.alpha(0.10), u.accent.alpha(0.55)),
    };
    let (h, words) = match size {
        Size::Card => (28.0, Some(b.short())),
        Size::Icon => (
            28.0,
            matches!(b, Button::Downloading(Some(_)) | Button::Paused(Some(_))).then(|| b.short()),
        ),
        Size::Row => (32.0, Some(b.label())),
        Size::Wide => (36.0, Some(b.label())),
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
        button = button.child(
            text(words)
                .font(if size == Size::Wide { BODY_MED } else { LABEL })
                .color(ink)
                .a11y_hidden(),
        );
    }
    if b.busy() {
        // Still a target, so a press on a busy button does not open the card behind it.
        return button.on_press(|_st: &mut State| {});
    }
    let name = name.to_string();
    button
        .hover(u.hover)
        .press(Color::WHITE.alpha(0.08))
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| press(st, id, &name))
}

/// The line a card or row shows instead of its byline while an install moves or after it
/// failed: "Downloading 40%", "Installing", the error.
pub fn status_line(s: &State, u: &Ui, id: u64) -> Option<El> {
    let b = button(s, id);
    let (words, color) = match &b {
        Button::Retry(error) => (error.clone(), u.rose),
        Button::Queued => (tr("Waiting for its turn").to_string(), u.muted),
        Button::Downloading(_) | Button::Installing => (b.label(), u.accent),
        Button::Paused(_) => (b.label(), u.muted),
        _ => return None,
    };
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
            .child(el().h_full().grow(fraction.clamp(0.0, 1.0)).fill(u.accent))
            .child(el().h_full().grow(1.0 - fraction.clamp(0.0, 1.0)))
            .a11y_hidden(),
    )
}

/// The Installed tag on a card's image.
pub fn installed_badge(s: &State, id: u64) -> Option<El> {
    let b = button(s, id);
    let (glyph, words) = match b {
        Button::Installed => (&icons::CHECK, tr("Installed")),
        Button::Update => (&icons::REFRESH_CW, tr("Update")),
        _ => return None,
    };
    Some(
        row()
            .key(format!("installed-badge-{id}"))
            .pin(None, None, Some(8.0), Some(8.0))
            .h(22.0)
            .px(8.0)
            .gap(4.0)
            .center()
            .radius(crate::ui::r(6.0))
            .fill(Color::BLACK.alpha(0.62))
            .border(1.0, Color::WHITE.alpha(0.16))
            // On a darkened image in every theme, so the ink is always white.
            .child(icon(glyph, 12.0).color(Color::WHITE).a11y_hidden())
            .child(text(words.to_string()).font(MICRO_CAPS).color(Color::WHITE)),
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
    let others = profile.files.len() > 1;
    let mut line = row().key("details-install-row").w_full().gap(8.0).center_y().child(main);
    if others {
        let open = s.browse_view.installs.files_menu == Some(id);
        line = line.child(
            el().key(format!("details-install-more-{id}"))
                .size(36.0, 36.0)
                .center()
                .radius(crate::ui::r(8.0))
                .border(1.0, u.hair_strong)
                .fill(if open { u.hover } else { Color::TRANSPARENT })
                .hover(u.hover)
                .press(Color::WHITE.alpha(0.08))
                .focusable(true)
                .role("button")
                .label(tr("Choose another file"))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    let menu = &mut st.browse_view.installs.files_menu;
                    *menu = if *menu == Some(id) { None } else { Some(id) };
                })
                .child(icon(&icons::CHEVRON_DOWN, 16.0).color(u.muted).a11y_hidden()),
        );
    }
    let which = s
        .browse_view
        .installs
        .items
        .get(&id)
        .and_then(|e| e.file)
        .and_then(|file| profile.files.iter().find(|f| f.id == file))
        .unwrap_or(primary);
    line = line.child(
        text(which.name.clone())
            .font(SMALL)
            .color(u.faint)
            .truncate()
            .min_w(0.0)
            .shrink(),
    );
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
        let mut el = button_el(u, &b, key, id, &file.name, Size::Row);
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
    let el = button_el(u, &b, key, id, &file.name, Size::Row);
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
            .shadow(12.0, 32.0, Color::BLACK.alpha(0.5))
            .anchored(format!("details-install-more-{id}"), Side::Bottom, Align::End, 6.0)
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
        assert_eq!(Button::Install.label(), "Install");
        assert_eq!(Button::Downloading(Some(0.4)).label(), "Downloading 40%");
        assert_eq!(Button::Downloading(Some(0.4)).short(), "40%");
        assert_eq!(Button::Installing.label(), "Installing");
        assert_eq!(Button::Installed.label(), "Installed");
        assert!(Button::Queued.busy() && !Button::Installed.busy());
    }
}
