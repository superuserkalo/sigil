//! Profiles: saved mod setups you can apply, share and import. Ported from Grimoire's
//! Profiles page and its export and import dialogs (MIT), drawn in Sigil's rows.
//!
//! Every value comes from `deadlock_core::profiles`. Saving, renaming, duplicating and
//! deleting touch the workspace only. Applying a profile renames slot files in the addon
//! folder and may rewrite `autoexec.cfg`; importing may download missing mods into the addon
//! folder. Both run only from an explicit press inside a dialog that shows what will change.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};

use deadlock_core::crosshair::Crosshair;
use deadlock_core::profiles::{
    self as core, AddonIdentity, EntryState, ImportEntry, ImportStatus, PortableProfile, Profile,
};
use rune::{
    Align, El, FileDialogOptions, Key, KeyEvent, ScrollSpec, Side, TextEdit, Tx, col, el, icon, row,
    spacer, text,
};

use crate::app::State;
use crate::icons;
use crate::jobs::Job;
use crate::model::Screen;
use crate::notify::Notice;
use crate::screens::settings::i18n::fill;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, MONO, PAGE_TOP, PAGE_X, SMALL, SMALL_NUM, Ui};


#[path = "profiles_de.rs"]
mod de;

/// The UI text in the player's language: this page's German table first, then Sigil's.
pub(crate) fn tr(text: &str) -> &str {
    use crate::screens::settings::i18n::{self, Lang};
    if i18n::current() == Lang::German
        && let Some(de) = de::get(text)
    {
        return de;
    }
    i18n::tr(text)
}
// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LineState {
    Ready,
    Missing,
    Reused,
    /// No game install to check against.
    Unknown,
}

#[derive(Clone)]
pub struct ModLine {
    pub label: String,
    pub slot: u16,
    pub enabled: bool,
    pub state: LineState,
}

#[derive(Clone)]
pub struct Row {
    pub name: String,
    pub slug: String,
    pub updated: String,
    pub mods: Vec<ModLine>,
    pub missing: usize,
    pub crosshair: Option<Crosshair>,
    pub autoexec: Option<Vec<String>>,
    pub author: Option<String>,
    /// Whether the install matches this profile right now. Only computed for the active one.
    pub in_sync: Option<bool>,
    /// The profile keeps its mods in its own folder under `citadel/addons`.
    pub isolated: bool,
}

/// One recovery point in the collapsible list.
#[derive(Clone, PartialEq, Eq)]
pub struct SnapshotRow {
    pub id: String,
    pub label: String,
    pub created: String,
    /// "Before a profile switch" and the like.
    pub reason: String,
    pub mods: usize,
}

/// What the snapshot worker did, plus the list as it stands after it.
pub struct SnapshotDone {
    pub snapshots: Vec<SnapshotRow>,
    pub message: Option<(String, bool)>,
}

pub struct Load {
    pub rows: Vec<Row>,
    pub active: Option<String>,
    /// Mods on right now, when there is a game install.
    pub current_mods: Option<usize>,
    pub current_crosshair: bool,
    pub snapshots: Vec<SnapshotRow>,
}

#[derive(Clone, PartialEq, Eq)]
pub enum Dialog {
    None,
    Create,
    Apply(String),
    Update(String),
    Rename(String),
    Delete(String),
    Export(String),
    Import,
    SnapshotRestore(String),
    SnapshotDelete(Vec<String>),
}

/// What an apply would change, for the confirm dialog.
#[derive(Clone)]
pub struct PlanView {
    pub turn_on: Vec<String>,
    pub turn_off: Vec<String>,
    pub missing: Vec<String>,
    pub autoexec: Option<String>,
    pub blocked: Option<String>,
}

impl PlanView {
    fn in_sync(&self) -> bool {
        self.turn_on.is_empty() && self.turn_off.is_empty() && self.autoexec.is_none()
    }
}

#[derive(Clone)]
pub struct ExportView {
    pub name: String,
    pub included: usize,
    pub skipped: Vec<String>,
    pub crosshair: bool,
    pub autoexec: usize,
    pub share_code: String,
    pub json: String,
    pub copied: bool,
    pub saved: Option<PathBuf>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Input,
    Resolving,
    Review,
    Installing,
}

#[derive(Clone, PartialEq, Eq)]
pub enum RowStatus {
    /// Its GameBanana lookup has not returned yet.
    LookingUp,
    Idle,
    Queued,
    Downloading,
    Installed,
    Failed(String),
}

pub struct ImportView {
    pub stage: Stage,
    pub input: TextEdit,
    pub error: Option<String>,
    /// Profile files found in Downloads and the exports folder: (file name, path, folder line).
    pub files: Vec<(String, PathBuf, String)>,
    pub portable: Option<PortableProfile>,
    pub entries: Vec<ImportEntry>,
    pub keep: Vec<bool>,
    pub status: Vec<RowStatus>,
    pub name: TextEdit,
    pub include_autoexec: bool,
    /// A recovery point was taken before the first download of this import.
    pub snapshot_taken: bool,
    pub resolve_job: Job<(PortableProfile, Receiver<ImportEntry>)>,
    /// Entries arriving one by one as their lookups return, so a download can start while
    /// the next mod is still looked up (DMM 2.4.0).
    pub lookups: Option<Receiver<ImportEntry>>,
    pub install_job: Job<(usize, Result<AddonIdentity, String>)>,
}

impl Default for ImportView {
    fn default() -> Self {
        ImportView {
            stage: Stage::Input,
            input: TextEdit::new(""),
            error: None,
            files: Vec::new(),
            portable: None,
            entries: Vec::new(),
            keep: Vec::new(),
            status: Vec::new(),
            name: TextEdit::new(""),
            include_autoexec: true,
            snapshot_taken: false,
            resolve_job: Job::default(),
            lookups: None,
            install_job: Job::default(),
        }
    }
}

pub struct View {
    pub loaded: bool,
    pub error: Option<String>,
    pub rows: Vec<Row>,
    pub active: Option<String>,
    pub current_mods: Option<usize>,
    pub current_crosshair: bool,
    pub expanded: Option<String>,
    /// The profile whose "..." menu is open.
    pub more: Option<String>,
    pub dialog: Dialog,
    pub name: TextEdit,
    pub include_crosshair: bool,
    pub include_isolated: bool,
    pub plan: Option<PlanView>,
    pub export: Option<ExportView>,
    pub import: ImportView,
    pub busy: bool,
    /// The outcome of the last action and whether it went wrong.
    pub last: Option<(String, bool)>,
    /// Fixture scenes: actions report instead of touching disk.
    pub fixture: bool,
    pub snapshots: Vec<SnapshotRow>,
    pub snapshots_open: bool,
    pub snapshot_selected: Vec<String>,
    /// What restoring the open snapshot would change.
    pub snapshot_plan: Option<PlanView>,
    pub titlebar_open: bool,
    pub apply_job: Job<(String, Result<core::ProfileApplyResult, String>)>,
    pub undo_job: Job<(Result<core::UndoReport, String>, String)>,
    pub snapshot_job: Job<SnapshotDone>,
    pub restore_plan_job: Job<(String, Result<core::ProfileApplyPlan, String>)>,
    pub export_job: Job<(String, Result<PathBuf, String>)>,
}

impl Default for View {
    fn default() -> Self {
        View {
            loaded: false,
            error: None,
            rows: Vec::new(),
            active: None,
            current_mods: None,
            current_crosshair: false,
            expanded: None,
            more: None,
            dialog: Dialog::None,
            name: TextEdit::new(""),
            include_crosshair: false,
            include_isolated: false,
            plan: None,
            export: None,
            import: ImportView::default(),
            busy: false,
            last: None,
            fixture: false,
            snapshots: Vec::new(),
            snapshots_open: false,
            snapshot_selected: Vec::new(),
            snapshot_plan: None,
            titlebar_open: false,
            apply_job: Job::default(),
            undo_job: Job::default(),
            snapshot_job: Job::default(),
            restore_plan_job: Job::default(),
            export_job: Job::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

/// Read every saved profile and check each one against the install. Runs on a worker.
pub fn load(workspace: &Path, game_root: Option<&Path>) -> Result<Load, String> {
    let profiles = core::list_profiles(workspace).map_err(|error| error.to_string())?;
    let active = core::active_profile(workspace);
    let installed = game_root.and_then(|root| core::installed_identities(root, workspace).ok());
    let current = game_root.and_then(|root| core::read_current_setup(root, workspace).ok());

    let rows = profiles
        .iter()
        .map(|profile| {
            let in_sync = match (game_root, active.as_deref()) {
                (Some(root), Some(slug)) if slug == profile.slug => {
                    core::plan_profile_apply(root, workspace, profile)
                        .ok()
                        .map(|plan| plan.in_sync())
                }
                _ => None,
            };
            row_from(profile, installed.as_deref(), in_sync)
        })
        .collect();

    Ok(Load {
        rows,
        active,
        current_mods: current.as_ref().map(|setup| setup.slots.len()),
        current_crosshair: current.is_some_and(|setup| setup.crosshair.is_some()),
        snapshots: core::list_snapshots(workspace)
            .into_iter()
            .map(snapshot_row_from)
            .collect(),
    })
}

fn snapshot_row_from(summary: core::SnapshotSummary) -> SnapshotRow {
    SnapshotRow {
        id: summary.id,
        label: summary.label,
        created: short_date(&summary.created_at),
        reason: summary.trigger.default_label().to_string(),
        mods: summary.mods,
    }
}

fn row_from(profile: &Profile, installed: Option<&[AddonIdentity]>, in_sync: Option<bool>) -> Row {
    let states = installed.map(|installed| core::resolve_entries(&profile.slots, installed));
    let mods: Vec<ModLine> = profile
        .slots
        .iter()
        .enumerate()
        .map(|(index, slot)| {
            let state = match states.as_ref().map(|states| states[index]) {
                None => LineState::Unknown,
                Some(EntryState::Ready { .. }) => LineState::Ready,
                Some(EntryState::Missing) => LineState::Missing,
                Some(EntryState::SlotReused { .. }) => LineState::Reused,
            };
            let label = match (state, installed, states.as_ref()) {
                (LineState::Ready, Some(installed), Some(states)) if slot.name.is_none() => {
                    match states[index] {
                        EntryState::Ready { slot: at, .. } => installed
                            .iter()
                            .find(|addon| addon.slot == at)
                            .and_then(|addon| addon.name.clone())
                            .unwrap_or_else(|| slot.label()),
                        _ => slot.label(),
                    }
                }
                _ => slot.label(),
            };
            ModLine {
                label,
                slot: slot.slot,
                enabled: slot.enabled,
                state,
            }
        })
        .collect();
    let missing = mods
        .iter()
        .filter(|line| line.enabled && matches!(line.state, LineState::Missing | LineState::Reused))
        .count();
    Row {
        name: profile.name.clone(),
        slug: profile.slug.clone(),
        updated: short_date(&profile.updated_at),
        mods,
        missing,
        crosshair: profile.crosshair.clone(),
        autoexec: profile.autoexec.clone(),
        author: profile.author.clone(),
        in_sync,
        isolated: profile.isolated,
    }
}

pub fn apply_load(st: &mut State, load: Load) {
    let v = &mut st.profiles;
    v.rows = load.rows;
    v.active = load.active;
    v.current_mods = load.current_mods;
    v.current_crosshair = load.current_crosshair;
    v.snapshots = load.snapshots;
    v.snapshot_selected
        .retain(|id| v.snapshots.iter().any(|snapshot| &snapshot.id == id));
    v.loaded = true;
    v.error = None;
    if let Some(slug) = &v.expanded
        && !v.rows.iter().any(|row| &row.slug == slug)
    {
        v.expanded = None;
    }
}

/// Worker results for the import dialog. Called from the app's poll.
pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.profiles.import.resolve_job.try_take() {
        let import = &mut st.profiles.import;
        match result {
            Ok((portable, lookups)) => {
                import.name = TextEdit::new(&portable.profile.name);
                import.include_autoexec = true;
                let count = portable.mods.len();
                import.entries = (0..count)
                    .filter_map(|index| core::pending_import_entry(&portable, index))
                    .collect();
                import.keep = vec![true; count];
                import.status = vec![RowStatus::LookingUp; count];
                import.lookups = Some(lookups);
                import.portable = Some(portable);
                import.stage = Stage::Review;
            }
            Err(error) => {
                import.error = Some(error);
                import.stage = Stage::Input;
            }
        }
        changed = true;
    }
    changed |= take_lookups(st);
    if let Some(result) = st.profiles.import.install_job.try_take() {
        let import = &mut st.profiles.import;
        match result {
            Ok((index, Ok(_))) => import.status[index] = RowStatus::Installed,
            Ok((index, Err(error))) => import.status[index] = RowStatus::Failed(friendly(&error)),
            Err(error) => {
                if let Some(index) = import
                    .status
                    .iter()
                    .position(|status| *status == RowStatus::Downloading)
                {
                    import.status[index] = RowStatus::Failed(friendly(&error));
                }
            }
        }
        next_install(st);
        changed = true;
    }
    if let Some(result) = st.profiles.apply_job.try_take() {
        st.profiles.busy = false;
        match result {
            Ok((name, Ok(applied))) => finish_apply(st, name, applied),
            Ok((_, Err(error))) | Err(error) => {
                crate::notify::push(st, Notice::error(friendly(&error)))
            }
        }
        changed = true;
    }
    if let Some(result) = st.profiles.undo_job.try_take() {
        match result {
            Ok((Ok(undo), name)) => {
                let mut line = format!("Switched back from {name}");
                if !undo.skipped.is_empty() {
                    line.push_str(&format!(
                        ". {} left as they are",
                        plural(undo.skipped.len(), "mod")
                    ));
                }
                crate::notify::push(st, Notice::success(line));
                crate::data::scan_addons_now(st);
                crate::data::load_autoexec_now(st);
            }
            Ok((Err(error), _)) | Err(error) => {
                crate::notify::push(st, Notice::error(friendly(&error)))
            }
        }
        reload(st);
        changed = true;
    }
    if let Some(result) = st.profiles.snapshot_job.try_take() {
        match result {
            Ok(done) => {
                st.profiles.snapshots = done.snapshots;
                st.profiles.snapshot_selected.retain(|id| {
                    st.profiles
                        .snapshots
                        .iter()
                        .any(|snapshot| &snapshot.id == id)
                });
                if let Some((message, bad)) = done.message {
                    report(st, message, bad);
                }
            }
            Err(error) => report(st, friendly(&error), true),
        }
        changed = true;
    }
    if let Some(result) = st.profiles.restore_plan_job.try_take() {
        match result {
            Ok((id, Ok(plan))) => {
                st.profiles.snapshot_plan = Some(plan_view(&plan));
                st.profiles.dialog = Dialog::SnapshotRestore(id);
            }
            Ok((_, Err(error))) | Err(error) => report(st, friendly(&error), true),
        }
        changed = true;
    }
    if let Some(result) = st.profiles.export_job.try_take() {
        if let Some(export) = st.profiles.export.as_mut() {
            match result {
                Ok((_, Ok(path))) => {
                    export.saved = Some(path);
                    export.error = None;
                }
                Ok((_, Err(error))) | Err(error) => export.error = Some(friendly(&error)),
            }
        }
        changed = true;
    }
    changed
}

/// An apply finished: say what happened, offer Undo, and refresh the pages that changed.
// --- the Locker shuffle a profile keeps (Grimoire #587) -------------------------------------

/// The Locker's shuffle switches, pool and Equip mode as a profile stores them.
pub fn locker_shuffle_of(prefs: &deadlock_core::locker::LockerPrefs) -> core::LockerShuffle {
    core::LockerShuffle {
        shuffle: prefs.shuffle.clone(),
        shuffle_pool: prefs.shuffle_pool.clone(),
        multi_skin: prefs.multi_skin,
    }
}

/// Put a profile's Locker shuffle into the Locker. Sigil's data folder only.
fn set_locker_shuffle(st: &mut State, shuffle: &core::LockerShuffle) {
    if locker_shuffle_of(&st.locker.prefs) == *shuffle {
        return;
    }
    st.locker.prefs.shuffle = shuffle.shuffle.clone();
    st.locker.prefs.shuffle_pool = shuffle.shuffle_pool.clone();
    st.locker.prefs.multi_skin = shuffle.multi_skin;
    if !st.demo && !st.profiles.fixture && !rune::is_headless() {
        crate::data::write_locker_prefs(st);
    }
}

/// After a Locker shuffle edit: the active profile keeps the new state, so each profile has
/// its own shuffle without an Update (Grimoire's rule). Nothing happens without an active
/// profile. Workspace only.
pub fn sync_locker_to_active_profile(st: &State) {
    if st.demo || st.profiles.fixture || rune::is_headless() {
        return;
    }
    let workspace = crate::data::workspace_dir();
    if let Some(slug) = core::active_profile(&workspace) {
        let _ = core::store_locker_shuffle(&workspace, &slug, &locker_shuffle_of(&st.locker.prefs));
    }
}

fn finish_apply(st: &mut State, name: String, applied: core::ProfileApplyResult) {
    // The profile's Locker shuffle comes with it; a profile saved before profiles kept one
    // leaves the Locker as it is. Undo puts the earlier shuffle back too.
    let locker_before = locker_shuffle_of(&st.locker.prefs);
    let workspace = crate::data::workspace_dir();
    if let Some(shuffle) = core::active_profile(&workspace)
        .and_then(|slug| core::read_profile(&workspace, &slug).ok())
        .and_then(|profile| profile.locker_shuffle())
    {
        set_locker_shuffle(st, &shuffle);
    }
    // Each profile keeps its own favourites (DMM); Undo puts the earlier set back.
    let favorites_before = crate::screens::profile_favorites::bring_back(st);
    let switched = applied.changed.len().max(applied.copied);
    let mut detail = Vec::new();
    if applied.copied > 0 {
        detail.push(format!("{} copied to its own folder", plural(applied.copied, "mod")));
    } else if switched > 0 {
        detail.push(format!("{} switched", plural(switched, "mod")));
    }
    if applied.autoexec_written {
        detail.push("autoexec updated".to_string());
    }
    if applied.folder.is_some() {
        detail.push("its own folder is mounted".to_string());
    }
    // The crosshair note is not a missing mod (Grimoire #504).
    let missing = applied.warnings.iter().filter(|w| w.as_str() != core::CROSSHAIR_SKIPPED).count();
    if missing > 0 {
        detail.push(format!(
            "{} not installed",
            plural(missing, "mod")
        ));
    }
    if missing < applied.warnings.len() {
        detail.push(tr("crosshair left out while the Crosshair page is off").to_string());
    }
    let detail = sentence_case(&detail.join(", "));
    let mut notice = if missing == 0 {
        Notice::success(format!("Applied {name}"))
    } else {
        Notice::warning(format!("Applied {name}"))
    };
    if !detail.is_empty() {
        notice = notice.detail(detail);
    }
    let root = st.game_root.clone();
    let for_undo = applied.clone();
    let undo_name = name.clone();
    notice = notice.undo(move |st: &mut State| {
        // Undo renames slot files back; the game holds enabled ones open, and a half done
        // undo has no rollback. Refuse up front, like the other held actions.
        use crate::screens::installed::waiting;
        if waiting::refuse_held(st, waiting::HELD_UNDO) {
            return;
        }
        let Some(root) = root.clone() else {
            crate::notify::push(st, Notice::error("Deadlock folder not found. Nothing was changed."));
            return;
        };
        set_locker_shuffle(st, &locker_before);
        crate::screens::profile_favorites::set(st, &favorites_before);
        let workspace = crate::data::workspace_dir();
        let result = for_undo.clone();
        let undo_name = undo_name.clone();
        let wake = st.wake.clone();
        st.profiles.undo_job.start(wake, move || {
            Ok((
                core::undo_profile_apply(&root, &workspace, &result, true)
                    .map_err(|error| error.to_string()),
                undo_name,
            ))
        });
    });
    crate::notify::push(st, notice);
    crate::data::scan_addons_now(st);
    if applied.autoexec_written {
        crate::data::load_autoexec_now(st);
    }
    reload(st);
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

pub fn view(s: &State, u: &Ui) -> El {
    let v = &s.profiles;
    let compact = crate::ui::fit() != crate::ui::Fit::Full;
    let labelled = |glyph: &'static rune::Icon, label: &str, key: &str| {
        if compact {
            u.icon_button(glyph, label).key(key.to_string())
        } else {
            u.icon_action(glyph, label, ActionKind::Secondary).key(key.to_string())
        }
    };
    let actions = vec![
        u.icon_button(&icons::REFRESH_CW, "Reload profiles")
            .key("profiles-reload")
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| {
                if !st.profiles.fixture {
                    crate::data::start_profile_list(st);
                }
            }),
        labelled(&icons::DOWNLOAD, tr("Import"), "profiles-import")
            .cursor(rune::Cursor::Pointer)
            .on_press(open_import),
        labelled(&icons::PLUS, "New profile", "profiles-create")
            .cursor(rune::Cursor::Pointer)
            .on_press(open_create),
    ];

    // The last result lives in a fixed status row, so it never pushes the list.
    let status = match &v.last {
        Some((message, bad)) => vec![u.status_line(
            if *bad { &icons::TRIANGLE_ALERT } else { &icons::CHECK },
            message,
            if *bad { crate::ui::Tone::Warn } else { crate::ui::Tone::Quiet },
        )
        .key("profiles-result")],
        None => Vec::new(),
    };
    let top = col()
        .w_full()
        .px(PAGE_X)
        .pt(PAGE_TOP)
        .pb(16.0)
        .gap(8.0)
        // The last result sits on the toolbar line after the title: fixed height, no extra band.
        .child(u.page_toolbar(
            u.page_title(tr("Profiles")),
            vec![row().key("profiles-status").gap(8.0).center_y().min_w(0.0).ml(8.0).children(status)],
            None,
            actions,
        ));

    let body = if let Some(error) = &v.error {
        u.empty_state("", tr("Profiles could not be read"), error, None)
    } else if !v.loaded {
        loading(u)
    } else if v.rows.is_empty() {
        u.empty_state(
            "",
            tr("No profiles yet"),
            "Save the mods you have on, then switch setups in one step.",
            Some(
                u.icon_action(&icons::PLUS, "New profile", ActionKind::Secondary)
                    .key("profiles-create-empty")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(open_create),
            ),
        )
        .pt(0.0)
    } else {
        let mut list = col().w_full();
        for profile in &v.rows {
            let open = v.expanded.as_deref() == Some(profile.slug.as_str());
            let active = v.active.as_deref() == Some(profile.slug.as_str());
            list = list.child(profile_row(u, profile, active, open));
        }
        list
    };

    let content = col()
        .key("profiles-page")
        .w_full()
        .gap(24.0)
        .child(body)
        .child(snapshots_section(s, u));

    let mut frame = el().key("profiles-stage").w_full().grow(1.0).min_h(0.0).child(
        col()
            .key("profiles-scroll")
            .w_full()
            .h_full()
            .min_h(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            // 72 at the foot so the last row never hides under the selection bar.
            .child(col().w_full().px(PAGE_X).pb(72.0).child(content)),
    );
    if !v.snapshot_selected.is_empty() {
        let ids = v.snapshot_selected.clone();
        frame = frame.child(u.selection_bar(
            "snapshots-selection",
            vec![
                u.close_button(tr("Clear the selection"))
                    .key("snapshots-clear-selection")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| st.profiles.snapshot_selected.clear()),
                text(crate::screens::settings::i18n::fill(tr("{} picked"), &[&ids.len().to_string()]))
                    .font(BODY_MED)
                    .color(u.ink)
                    .min_w(96.0),
            ],
            vec![
                u.action_m(tr("Delete"), ActionKind::Danger)
                    .key("snapshots-delete-selected")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        st.profiles.dialog = Dialog::SnapshotDelete(ids.clone())
                    }),
            ],
        ));
    }
    col()
        .key("profiles-frame")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(top)
        .child(frame)
}

/// The collapsible recovery points under the profile list.
fn snapshots_section(s: &State, u: &Ui) -> El {
    let v = &s.profiles;
    let count = v.snapshots.len();
    let now = u
        .icon_action(&icons::HISTORY, tr("Snapshot now"), ActionKind::Ghost)
        .h(24.0)
        .pl(6.0)
        .pr(8.0)
        .key("snapshots-now")
        .cursor(rune::Cursor::Pointer)
        .on_press(snapshot_now);
    let toggle = row()
        .key("snapshots-toggle")
        .grow(1.0)
        .min_w(0.0)
        .h(24.0)
        .gap(6.0)
        .center_y()
        .focusable(true)
        .role("button")
        .label(fill(tr("Snapshots ({})"), &[&count.to_string()]))
        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| st.profiles.snapshots_open = !st.profiles.snapshots_open)
        .child(
            icon(
                if v.snapshots_open {
                    &icons::CHEVRON_DOWN
                } else {
                    &icons::CHEVRON_RIGHT
                },
                12.0,
            )
            .color(u.muted)
            .a11y_hidden(),
        )
        .child(u.section_head(tr("Snapshots"), Some(count), None));
    let head = row()
        .key("snapshots-head")
        .w_full()
        .h(24.0)
        .mb(8.0)
        .gap(8.0)
        .center_y()
        .child(toggle)
        .child(now);

    let mut section = col().w_full().child(head);
    if !v.snapshots_open {
        return section;
    }
    if v.snapshots.is_empty() {
        return section.child(
            row()
                .w_full()
                .h(32.0)
                .center_y()
                .child(
                    text(tr("None yet. One is saved before every switch."))
                        .font(SMALL)
                        .color(u.muted),
                ),
        );
    }
    let mut list = col().w_full();
    for snapshot in &v.snapshots {
        list = list.child(snapshot_row(u, snapshot, v.snapshot_selected.contains(&snapshot.id)));
    }
    section = section.child(list);
    section
}

fn snapshot_row(u: &Ui, snapshot: &SnapshotRow, selected: bool) -> El {
    let pick = snapshot.id.clone();
    let meta = format!(
        "{} · {} · {}",
        snapshot.reason,
        snapshot.created,
        crate::screens::settings::i18n::fill(tr(if snapshot.mods == 1 { "{} mod" } else { "{} mods" }), &[&snapshot.mods.to_string()])
    );
    let restore = {
        let id = snapshot.id.clone();
        u.action_m(tr("Restore"), ActionKind::Secondary)
            .key(format!("snapshot-restore-{}", snapshot.id))
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| open_snapshot_restore(st, &id))
    };
    let delete = {
        let id = snapshot.id.clone();
        u.icon_button(&icons::TRASH_2, "Delete snapshot")
            .size(28.0, 28.0)
            .key(format!("snapshot-delete-{}", snapshot.id))
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| st.profiles.dialog = Dialog::SnapshotDelete(vec![id.clone()]))
    };
    row()
        .key(format!("snapshot-{}", snapshot.id))
        .w_full()
        .h(48.0)
        .gap(10.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(
            el()
                .size(16.0, 16.0)
                .radius(crate::ui::r(4.0))
                .center()
                .role("checkbox")
                .checked(selected)
                .label(format!("Pick {}", snapshot.label))
                .focusable(true)
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| toggle_snapshot(st, &pick))
                .child(if selected {
                    icon(&icons::CHECK, 12.0).color(u.ink).a11y_hidden()
                } else {
                    el().size(0.0, 0.0)
                })
                .border(1.0, if selected { u.ink } else { rune::Color::WHITE.alpha(0.22) }),
        )
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(
                    text(snapshot.label.clone())
                        .font(BODY_MED)
                        .color(u.ink)
                        .truncate(),
                )
                .child(text(meta).font(SMALL_NUM).color(u.muted).truncate()),
        )
        .child(restore)
        .child(delete)
}

fn profile_row(u: &Ui, profile: &Row, active: bool, open: bool) -> El {
    let slug = profile.slug.clone();

    let mut title = row()
        .gap(8.0)
        .center_y()
        .min_w(0.0)
        .child(
            text(profile.name.clone())
                .font(BODY_MED)
                .color(u.ink)
                .truncate(),
        );
    if active {
        title = title.child(
            row()
                .key(format!("profile-active-{slug}"))
                .h(20.0)
                .px(8.0)
                .gap(6.0)
                .center_y()
                .radius(crate::ui::r(4.0))
                .fill(u.over.alpha(0.06))
                .child(u.mark(u.teal))
                .child(text(tr("Active")).font(crate::ui::LABEL).color(u.muted)),
        );
    }

    let mut meta = vec![crate::screens::settings::i18n::fill(tr(if profile.mods.len() == 1 { "{} mod" } else { "{} mods" }), &[&profile.mods.len().to_string()])];
    if profile.crosshair.is_some() {
        meta.push(tr("Crosshair").to_string());
    }
    if let Some(lines) = profile.autoexec.as_ref().filter(|lines| !lines.is_empty()) {
        meta.push(crate::screens::settings::i18n::fill(tr(if lines.len() == 1 { "{} autoexec line" } else { "{} autoexec lines" }), &[&lines.len().to_string()]));
    }
    if profile.isolated {
        meta.push(tr("Own folder").to_string());
    }
    if let Some(author) = &profile.author {
        meta.push(crate::screens::settings::i18n::fill(tr("By {}"), &[author.as_str()]));
    }
    meta.push(crate::screens::settings::i18n::fill(tr("Saved {}"), &[&profile.updated]));
    let changed = active && profile.in_sync == Some(false);
    if changed {
        meta.push(tr("Changed since applied").to_string());
    }
    let mut meta_line = row()
        .gap(6.0)
        .min_w(0.0)
        .center_y()
        .child(
            text(meta.join(" · "))
                .font(SMALL_NUM)
                .color(u.muted)
                .truncate()
                .shrink(),
        );
    if profile.missing > 0 {
        meta_line = meta_line
            .child(icon(&icons::TRIANGLE_ALERT, 12.0).color(u.amber).a11y_hidden())
            .child(text(crate::screens::settings::i18n::fill(tr("{} not installed"), &[&profile.missing.to_string()])).font(SMALL).color(u.amber));
    }

    let apply_label = if active { tr("Re-apply") } else { tr("Apply") };
    let head = row()
        .key(format!("profile-{slug}"))
        .w_full()
        .h(56.0)
        .gap(8.0)
        .center_y()
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(title)
                .child(meta_line),
        )
        .child({
            // A fixed 72 slot: Update is drawn only when this active profile changed.
            let slug = slug.clone();
            row().w(72.0).justify_end().child(if changed {
                u.action_m(tr("Update"), ActionKind::Ghost)
                    .key(format!("profile-update-{slug}"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        // Settings > Mods > Confirm before updating a profile.
                        if crate::screens::settings::options::get().confirm_profile_update {
                            st.profiles.dialog = Dialog::Update(slug.clone());
                        } else {
                            update(st, &slug);
                        }
                    })
            } else {
                el()
            })
        })
        .child({
            let slug = slug.clone();
            // Wide enough for "Re-apply" in every language, the same on every row.
            u.action_m(apply_label, ActionKind::Secondary)
                .w(120.0)
                .key(format!("profile-apply-{slug}"))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| open_apply(st, &slug))
        })
        .child({
            let slug = slug.clone();
            // Share, Rename, Duplicate and Delete live in this menu.
            u.more_button(&format!("More for {}", profile.name))
                .key(format!("profile-more-{slug}"))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    st.profiles.more = if st.profiles.more.as_deref() == Some(slug.as_str()) {
                        None
                    } else {
                        Some(slug.clone())
                    }
                })
        })
        .child({
            let slug = slug.clone();
            u.icon_button(
                if open {
                    &icons::CHEVRON_DOWN
                } else {
                    &icons::CHEVRON_RIGHT
                },
                if open { tr("Hide details") } else { tr("Show details") },
            )
            .size(28.0, 28.0)
            .key(format!("profile-expand-{slug}"))
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| {
                st.profiles.expanded = if st.profiles.expanded.as_deref() == Some(slug.as_str()) {
                    None
                } else {
                    Some(slug.clone())
                };
            })
        });

    let mut group = col()
        .w_full()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(head);
    if open {
        group = group.child(details(u, profile));
    }
    group
}

/// The expanded part of a row: its mods, crosshair, autoexec lines and the quieter actions.
fn details(u: &Ui, profile: &Row) -> El {
    let slug = profile.slug.clone();
    let mut body = col()
        .key(format!("profile-details-{slug}"))
        .w_full()
        .pt(4.0)
        .pb(20.0)
        .gap(24.0);

    let mut mods = col().w_full().gap(6.0).child(sub_head(u, tr("Mods")));
    if profile.mods.is_empty() {
        mods = mods.child(text("No mods were on when this was saved.").font(BODY).color(u.muted));
    } else {
        let mut list = col().w_full();
        for line in &profile.mods {
            list = list.child(mod_line(u, line));
        }
        mods = mods.child(list);
    }
    body = body.child(mods);

    if let Some(crosshair) = &profile.crosshair {
        let summary = format!(
            "Gap {}  ·  Height {}  ·  Width {}",
            crosshair.pip_gap, crosshair.pip_height, crosshair.pip_width
        );
        let slug = slug.clone();
        body = body.child(
            col().w_full().gap(4.0).child(sub_head(u, tr("Crosshair"))).child(
                row()
                    .w_full()
                    .h(32.0)
                    .gap(10.0)
                    .center_y()
                    .child(swatch(crosshair))
                    .child(text(summary).font(BODY).color(u.prose).grow(1.0).truncate())
                    .child(
                        u.action(tr("Remove"), ActionKind::Ghost)
                            .key(format!("profile-crosshair-remove-{slug}"))
                            .cursor(rune::Cursor::Pointer)
                            .on_press(move |st: &mut State| remove_crosshair(st, &slug)),
                    ),
            ),
        );
    }

    if let Some(lines) = profile.autoexec.as_ref().filter(|lines| !lines.is_empty()) {
        body = body.child(
            col()
                .w_full()
                .gap(8.0)
                .child(sub_head(u, tr("Autoexec")))
                .child(command_well(u, lines, 6)),
        );
    }

    let folder = core::profile_folder_name(&profile.slug)
        .unwrap_or_else(|_| "its own folder".to_string());
    let toggle_slug = slug.clone();
    let isolated = profile.isolated;
    body = body.child(
        row()
            .key(format!("profile-isolated-{slug}"))
            .w_full()
            .py(4.0)
            .gap(12.0)
            .center_y()
            .focusable(true)
            .role("switch")
            .checked(isolated)
            .label(tr("Own addons folder"))
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| toggle_isolated(st, &toggle_slug))
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(2.0)
                    .child(
                        text(tr("Own addons folder"))
                            .font(BODY)
                            .color(u.prose)
                            .a11y_hidden(),
                    )
                    .child(
                        text(format!(
                            "Keeps its mods in citadel/addons/{folder}. Other profiles do not see them."
                        ))
                        .font(SMALL)
                        .color(u.faint)
                        .wrap(),
                    ),
            )
            .child(u.toggle_mark(isolated)),
    );

    body
}

/// A profile's "..." menu: Share, Rename, Duplicate, Delete.
fn more_menu(u: &Ui, slug: &str) -> El {
    let close = |st: &mut State| st.profiles.more = None;
    let (share, rename, dup, delete) = (slug.to_string(), slug.to_string(), slug.to_string(), slug.to_string());
    let menu = u
        .menu("profile-more-menu", &format!("profile-more-{slug}"), 200.0)
        .child(u.menu_item(&format!("profile-share-{slug}"), &icons::EXTERNAL_LINK, tr("Share"), false).on_press(
            move |st: &mut State| {
                st.profiles.more = None;
                open_export(st, &share)
            },
        ))
        .child(u.menu_item(&format!("profile-rename-{slug}"), &icons::SQUARE_PEN, tr("Rename"), false).on_press(
            move |st: &mut State| {
                st.profiles.more = None;
                open_rename(st, &rename)
            },
        ))
        .child(u.menu_item(&format!("profile-duplicate-{slug}"), &icons::COPY, tr("Duplicate"), false).on_press(
            move |st: &mut State| {
                st.profiles.more = None;
                duplicate(st, &dup)
            },
        ))
        .child(u.menu_item(&format!("profile-delete-{slug}"), &icons::TRASH_2, tr("Delete"), true).on_press(
            move |st: &mut State| {
                st.profiles.more = None;
                st.profiles.dialog = Dialog::Delete(delete.clone())
            },
        ))
        .on_key(move |st: &mut State, ev: &KeyEvent, _| {
            if ev.key == rune::Key::Escape {
                close(st);
                true
            } else {
                false
            }
        });
    u.popover_layer("profile-more-layer", el().on_press(close), menu)
}

fn sub_head(u: &Ui, label: &str) -> El {
    text(label.to_string()).font(SMALL).color(u.muted)
}

fn mod_line(u: &Ui, line: &ModLine) -> El {
    let (note, color) = match line.state {
        LineState::Missing => (tr("Not installed"), u.amber),
        LineState::Reused => ("Slot holds another mod", u.amber),
        _ if !line.enabled => ("Off", u.faint),
        _ => ("", u.faint),
    };
    row()
        .w_full()
        .h(32.0)
        .gap(12.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(text(format!("{:02}", line.slot)).font(MONO).color(u.faint).w(20.0))
        .child(
            text(line.label.clone())
                .font(BODY)
                .color(if line.state == LineState::Ready || line.state == LineState::Unknown {
                    u.prose
                } else {
                    u.muted
                })
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        )
        .child(text(note).font(SMALL).color(color))
}

fn swatch(crosshair: &Crosshair) -> El {
    let channel = |value: i32| value.clamp(0, 255) as f32 / 255.0;
    el().size(14.0, 14.0)
        .radius(crate::ui::r(4.0))
        .fill(rune::Color {
            r: channel(crosshair.color_r),
            g: channel(crosshair.color_g),
            b: channel(crosshair.color_b),
            a: 1.0,
        })
        .border(1.0, rune::Color::WHITE.alpha(0.12))
}

/// Command lines in a mono well, at most `max` before a count of the rest.
fn command_well(u: &Ui, lines: &[String], max: usize) -> El {
    let mut well = col()
        .w_full()
        .px(12.0)
        .py(10.0)
        .gap(2.0)
        .radius(crate::ui::r(8.0))
        .fill(u.well)
        .border(1.0, u.hair);
    for line in lines.iter().take(max) {
        well = well.child(text(line.clone()).font(MONO).color(u.prose).truncate());
    }
    if lines.len() > max {
        well = well.child(
            text(format!("and {} more", lines.len() - max))
                .font(SMALL)
                .color(u.faint),
        );
    }
    well
}

fn loading(u: &Ui) -> El {
    let mut list = col().w_full();
    for n in 0..3 {
        list = list.child(
            col()
                .w_full()
                .h(56.0)
                .gap(8.0)
                .justify_center()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(
                    el().h(10.0)
                        .w(140.0 + (n % 2) as f32 * 60.0)
                        .radius(crate::ui::r(3.0))
                        .fill(u.hover),
                )
                .child(el().h(8.0).w(220.0).radius(crate::ui::r(3.0)).fill(u.hover)),
        );
    }
    list
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

fn find_row<'a>(st: &'a State, slug: &str) -> Option<&'a Row> {
    st.profiles.rows.iter().find(|row| row.slug == slug)
}

fn report(st: &mut State, message: impl Into<String>, bad: bool) {
    st.profiles.last = Some((message.into(), bad));
}

fn close(st: &mut State) {
    st.profiles.dialog = Dialog::None;
    st.profiles.plan = None;
    st.profiles.export = None;
    st.profiles.snapshot_plan = None;
    st.profiles.titlebar_open = false;
}

fn open_create(st: &mut State) {
    st.profiles.name = TextEdit::new("");
    st.profiles.include_crosshair = false;
    st.profiles.include_isolated = false;
    st.profiles.dialog = Dialog::Create;
}

fn open_rename(st: &mut State, slug: &str) {
    let name = find_row(st, slug).map(|row| row.name.clone()).unwrap_or_default();
    st.profiles.name = TextEdit::new(&name);
    st.profiles.dialog = Dialog::Rename(slug.to_string());
}

fn reload(st: &mut State) {
    if !st.profiles.fixture {
        crate::data::start_profile_list(st);
    }
}

/// Save the mods that are on now as a new profile. Workspace only.
fn create(st: &mut State) {
    let name = st.profiles.name.text.trim().to_string();
    if name.is_empty() {
        return;
    }
    close(st);
    if st.profiles.fixture {
        report(st, format!("Saved {name}."), false);
        return;
    }
    let Some(game_root) = st.game_root.clone() else {
        report(st, "Deadlock was not found, so there is nothing to save.", true);
        return;
    };
    let include = st.profiles.include_crosshair;
    let isolated = st.profiles.include_isolated;
    let workspace = crate::data::workspace_dir();
    match core::create_profile(&game_root, &workspace, &name, include) {
        Ok(profile) => {
            if isolated {
                let _ = core::set_profile_isolated(&workspace, &profile.slug, true);
            }
            let _ = core::store_locker_shuffle(&workspace, &profile.slug, &locker_shuffle_of(&st.locker.prefs));
            crate::screens::profile_favorites::remember(st, &profile.slug);
            report(
                st,
                format!("Saved {} with {}.", profile.name, plural(profile.slots.len(), "mod")),
                false,
            );
        }
        Err(error) => report(st, error.to_string(), true),
    }
    reload(st);
}

fn update(st: &mut State, slug: &str) {
    close(st);
    let name = find_row(st, slug).map(|row| row.name.clone()).unwrap_or_default();
    if st.profiles.fixture {
        report(st, format!("Updated {name}."), false);
        return;
    }
    let Some(game_root) = st.game_root.clone() else {
        report(st, "Deadlock was not found, so there is nothing to save.", true);
        return;
    };
    match core::update_profile(&game_root, &crate::data::workspace_dir(), slug) {
        Ok(profile) => {
            let _ = core::store_locker_shuffle(
                &crate::data::workspace_dir(),
                &profile.slug,
                &locker_shuffle_of(&st.locker.prefs),
            );
            crate::screens::profile_favorites::remember(st, &profile.slug);
            report(
            st,
            format!("Updated {} with {}.", profile.name, plural(profile.slots.len(), "mod")),
            false,
            )
        }
        Err(error) => report(st, error.to_string(), true),
    }
    reload(st);
}

fn rename(st: &mut State, slug: &str) {
    let name = st.profiles.name.text.trim().to_string();
    if name.is_empty() {
        return;
    }
    close(st);
    if st.profiles.fixture {
        if let Some(row) = st.profiles.rows.iter_mut().find(|row| row.slug == slug) {
            row.name = name;
        }
        return;
    }
    if let Err(error) = core::rename_profile(&crate::data::workspace_dir(), slug, &name) {
        report(st, error.to_string(), true);
    }
    reload(st);
}

fn duplicate(st: &mut State, slug: &str) {
    if st.profiles.fixture {
        report(st, "Duplicated.", false);
        return;
    }
    match core::duplicate_profile(&crate::data::workspace_dir(), slug) {
        Ok(profile) => report(st, format!("Saved {}.", profile.name), false),
        Err(error) => report(st, error.to_string(), true),
    }
    reload(st);
}

fn delete(st: &mut State, slug: &str) {
    close(st);
    let name = find_row(st, slug).map(|row| row.name.clone()).unwrap_or_default();
    if st.profiles.fixture {
        st.profiles.rows.retain(|row| row.slug != slug);
        report(st, format!("Deleted {name}."), false);
        return;
    }
    match core::delete_profile(&crate::data::workspace_dir(), slug) {
        Ok(()) => report(st, format!("Deleted {name}. Your mods stay as they are."), false),
        Err(error) => report(st, error.to_string(), true),
    }
    reload(st);
}

fn remove_crosshair(st: &mut State, slug: &str) {
    if st.profiles.fixture {
        if let Some(row) = st.profiles.rows.iter_mut().find(|row| row.slug == slug) {
            row.crosshair = None;
        }
        return;
    }
    if let Err(error) = core::remove_profile_crosshair(&crate::data::workspace_dir(), slug) {
        report(st, error.to_string(), true);
    }
    reload(st);
}

/// Work out what applying would change, then show it. Reads only.
fn open_apply(st: &mut State, slug: &str) {
    if st.profiles.fixture {
        st.profiles.dialog = Dialog::Apply(slug.to_string());
        return;
    }
    let Some(game_root) = st.game_root.clone() else {
        report(st, "Deadlock was not found, so the profile cannot be applied.", true);
        return;
    };
    let workspace = crate::data::workspace_dir();
    let plan = core::read_profile(&workspace, slug)
        .and_then(|profile| core::plan_profile_apply(&game_root, &workspace, &profile));
    match plan {
        Ok(plan) => {
            st.profiles.plan = Some(plan_view(&plan));
            st.profiles.dialog = Dialog::Apply(slug.to_string());
        }
        Err(error) => report(st, error.to_string(), true),
    }
}

pub fn plan_view(plan: &core::ProfileApplyPlan) -> PlanView {
    let autoexec = plan.autoexec.as_ref().map(|text| {
        let parts = core::split_autoexec(text);
        let mut summary = plural(parts.commands.len(), "line");
        if parts.block.is_some() {
            summary.push_str(" and a crosshair");
        }
        summary
    });
    PlanView {
        turn_on: plan.turn_on.iter().map(|change| change.label.clone()).collect(),
        turn_off: plan.turn_off.iter().map(|change| change.label.clone()).collect(),
        missing: plan
            .entries
            .iter()
            .filter(|entry| {
                entry.wants_enabled && !matches!(entry.state, EntryState::Ready { .. })
            })
            .map(|entry| entry.label.clone())
            .collect(),
        autoexec,
        blocked: plan.blocked.clone(),
    }
}

/// Apply a profile on a worker. Game install write: renames slot files, copies into a profile
/// folder and may rewrite autoexec.cfg. Runs only from a press: the confirm dialog or the
/// titlebar switcher.
fn apply(st: &mut State, slug: &str) {
    crate::screens::settings::telemetry::event(st, "profile:apply", Some("page:profiles"));
    close(st);
    let name = find_row(st, slug).map(|row| row.name.clone()).unwrap_or_default();
    if st.profiles.fixture {
        st.profiles.active = Some(slug.to_string());
        report(st, format!("Applied {name}."), false);
        return;
    }
    let Some(game_root) = st.game_root.clone() else {
        crate::notify::push(st, Notice::error("Deadlock was not found. Nothing was changed."));
        return;
    };
    st.profiles.busy = true;
    let workspace = crate::data::workspace_dir();
    let slug = slug.to_string();
    let wake = st.wake.clone();
    // Grimoire #504: while the Crosshair page is off its crosshair is not written.
    let choices = core::ApplyChoices { skip_crosshair: !st.prefs.page_on(crate::model::Screen::Crosshair) };
    st.profiles.apply_job.start(wake, move || {
        Ok((
            name,
            core::apply_profile_choosing(&game_root, &workspace, &slug, true, choices)
                .map_err(|error| error.to_string()),
        ))
    });
}

/// Keep or drop a profile's own addons folder. Workspace only; the next apply copies the mods
/// in and mounts it.
fn toggle_isolated(st: &mut State, slug: &str) {
    if st.profiles.fixture {
        if let Some(row) = st.profiles.rows.iter_mut().find(|row| row.slug == slug) {
            row.isolated = !row.isolated;
        }
        return;
    }
    let on = find_row(st, slug).map(|row| !row.isolated).unwrap_or(false);
    match core::set_profile_isolated(&crate::data::workspace_dir(), slug, on) {
        Ok(_) => {}
        Err(error) => report(st, error.to_string(), true),
    }
    reload(st);
}

// --- snapshots ---------------------------------------------------------------------

fn start_snapshot_job(st: &mut State, work: SnapshotWork) {
    if st.profiles.snapshot_job.is_running() {
        return;
    }
    let workspace = crate::data::workspace_dir();
    let game_root = st.game_root.clone();
    let wake = st.wake.clone();
    st.profiles.snapshot_job.start(wake, move || {
        let message: Option<(String, bool)>;
        match &work {
            SnapshotWork::Create => {
                let Some(root) = game_root.as_deref() else {
                    return Ok(SnapshotDone {
                        snapshots: core::list_snapshots(&workspace)
                            .into_iter()
                            .map(snapshot_row_from)
                            .collect(),
                        message: Some((
                            "Deadlock was not found, so nothing was saved.".to_string(),
                            true,
                        )),
                    });
                };
                match core::capture_snapshot(
                    root,
                    &workspace,
                    core::SnapshotTrigger::Manual,
                    None,
                ) {
                    Ok(point) => {
                        message = Some((
                            format!("Saved a recovery point with {}.", plural(point.mod_count(), "mod")),
                            false,
                        ))
                    }
                    Err(error) => message = Some((error.to_string(), true)),
                }
            }
            SnapshotWork::Delete(ids) => match core::delete_snapshots(&workspace, ids) {
                Ok(count) => {
                    message = Some((format!("Deleted {}.", plural(count, "snapshot")), false))
                }
                Err(error) => message = Some((error.to_string(), true)),
            },
        }
        Ok(SnapshotDone {
            snapshots: core::list_snapshots(&workspace)
                .into_iter()
                .map(snapshot_row_from)
                .collect(),
            message,
        })
    });
}

enum SnapshotWork {
    Create,
    Delete(Vec<String>),
}

fn snapshot_now(st: &mut State) {
    if st.profiles.fixture {
        st.profiles.snapshots.insert(
            0,
            SnapshotRow {
                id: "fixture-now".to_string(),
                label: tr("Snapshot").to_string(),
                created: "Nov 17".to_string(),
                reason: tr("Snapshot").to_string(),
                mods: 4,
            },
        );
        st.profiles.snapshots_open = true;
        report(st, "Saved a recovery point with 4 mods.", false);
        return;
    }
    start_snapshot_job(st, SnapshotWork::Create);
}

fn open_snapshot_restore(st: &mut State, id: &str) {
    let id = id.to_string();
    if st.profiles.fixture {
        st.profiles.snapshot_plan = Some(PlanView {
            turn_on: vec!["Quiet Footsteps".into()],
            turn_off: Vec::new(),
            missing: Vec::new(),
            autoexec: None,
            blocked: None,
        });
        st.profiles.dialog = Dialog::SnapshotRestore(id);
        return;
    }
    let Some(game_root) = st.game_root.clone() else {
        report(st, "Deadlock was not found, so a snapshot cannot be restored.", true);
        return;
    };
    let workspace = crate::data::workspace_dir();
    let wake = st.wake.clone();
    st.profiles.restore_plan_job.start(wake, move || {
        Ok((
            id.clone(),
            core::plan_snapshot_restore(&game_root, &workspace, &id)
                .map_err(|error| error.to_string()),
        ))
    });
}

fn restore_snapshot(st: &mut State, id: &str) {
    close(st);
    if st.profiles.fixture {
        report(st, "Put the recovery point back.", false);
        return;
    }
    let Some(game_root) = st.game_root.clone() else {
        crate::notify::push(st, Notice::error("Deadlock was not found. Nothing was changed."));
        return;
    };
    st.profiles.busy = true;
    let workspace = crate::data::workspace_dir();
    let slug = id.to_string();
    let wake = st.wake.clone();
    st.profiles.apply_job.start(wake, move || {
        Ok((
            "the recovery point".to_string(),
            core::restore_snapshot(&game_root, &workspace, &slug, true)
                .map_err(|error| error.to_string()),
        ))
    });
}

fn delete_snapshots(st: &mut State, ids: Vec<String>) {
    close(st);
    if ids.is_empty() {
        return;
    }
    if st.profiles.fixture {
        st.profiles.snapshots.retain(|row| !ids.contains(&row.id));
        st.profiles.snapshot_selected.retain(|id| !ids.contains(id));
        report(st, format!("Deleted {}.", plural(ids.len(), "snapshot")), false);
        return;
    }
    start_snapshot_job(st, SnapshotWork::Delete(ids));
}

fn toggle_snapshot(st: &mut State, id: &str) {
    if let Some(at) = st
        .profiles
        .snapshot_selected
        .iter()
        .position(|selected| selected == id)
    {
        st.profiles.snapshot_selected.remove(at);
    } else {
        st.profiles.snapshot_selected.push(id.to_string());
    }
}

/// Build the share code and file for a profile. Reads only.
fn open_export(st: &mut State, slug: &str) {
    if st.profiles.fixture {
        st.profiles.dialog = Dialog::Export(slug.to_string());
        return;
    }
    let workspace = crate::data::workspace_dir();
    let installed = st
        .game_root
        .clone()
        .and_then(|root| core::installed_identities(&root, &workspace).ok())
        .unwrap_or_default();
    let built = core::read_profile(&workspace, slug).and_then(|profile| {
        core::export_portable(&profile, &installed, env!("CARGO_PKG_VERSION"))
            .map(|export| (profile, export))
    });
    match built {
        Ok((profile, export)) => {
            st.profiles.export = Some(export_view(&profile, &export));
            st.profiles.dialog = Dialog::Export(slug.to_string());
        }
        Err(error) => report(st, error.to_string(), true),
    }
}

pub fn export_view(profile: &Profile, export: &core::PortableExport) -> ExportView {
    ExportView {
        name: profile.name.clone(),
        included: export.profile.mods.len(),
        skipped: export.skipped.clone(),
        crosshair: export.profile.crosshair().is_some(),
        autoexec: export.profile.autoexec_commands().len(),
        share_code: export.share_code.clone(),
        json: export.json.clone(),
        copied: false,
        saved: None,
        error: None,
    }
}

/// Ask where to save the export, then write it on a worker. Not a game write.
fn pick_export(st: &mut State, tx: &mut Tx) {
    let Some(export) = st.profiles.export.as_ref() else {
        return;
    };
    let options = FileDialogOptions {
        title: "Save profile".to_string(),
        directory: Some(downloads_dir()),
        file_name: Some(core::portable_file_name(&export.name)),
        ..FileDialogOptions::default()
    }
    .filter("Sigil profile", &["json"]);
    let json = export.json.clone();
    let name = export.name.clone();
    tx.save_file(options, move |st: &mut State, result, _tx| match result {
        Ok(Some(path)) => {
            let json = json.clone();
            let name = name.clone();
            let wake = st.wake.clone();
            st.profiles.export_job.start(wake, move || {
                Ok((
                    name,
                    core::write_portable_to(&path, &json).map_err(|error| error.to_string()),
                ))
            });
        }
        Ok(None) => {}
        Err(error) => {
            if let Some(export) = st.profiles.export.as_mut() {
                export.error = Some(format!("The file picker could not open: {error}"));
            }
        }
    });
}

/// The user's Downloads folder, else the workspace's exports folder.
fn downloads_dir() -> PathBuf {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" });
    home.map(|home| PathBuf::from(home).join("Downloads"))
        .filter(|path| path.is_dir())
        .unwrap_or_else(|| crate::data::workspace_dir().join("exports"))
}

fn short_path(path: &Path) -> String {
    let text = path.display().to_string();
    let home = std::env::var(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default();
    if !home.is_empty()
        && let Some(rest) = text.strip_prefix(&home)
    {
        return format!("~{rest}");
    }
    text
}

fn open_import(st: &mut State) {
    let fixture = st.profiles.fixture;
    st.profiles.import = ImportView::default();
    if !fixture {
        let folders = [downloads_dir(), crate::data::workspace_dir().join("exports")];
        st.profiles.import.files = core::find_portable_files(&folders, 5)
            .into_iter()
            .map(|path| {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let folder = path
                    .parent()
                    .and_then(|parent| parent.file_name())
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                (name, path, folder)
            })
            .collect();
    }
    st.profiles.dialog = Dialog::Import;
}

/// Read what the user gave (a share code, JSON or a file path) and look its mods up on
/// GameBanana, both on a worker. Nothing waits on the frame.
fn start_resolve(st: &mut State, input: String) {
    let import = &mut st.profiles.import;
    import.error = None;
    import.stage = Stage::Resolving;
    let workspace = crate::data::workspace_dir();
    let game_root = st.game_root.clone();
    let wake = st.wake.clone();
    let lookup_wake = wake.clone();
    st.profiles.import.resolve_job.start(wake, move || {
        let portable = core::read_portable_input(&input).map_err(|error| error.to_string())?;
        let installed = game_root
            .and_then(|root| core::installed_identities(&root, &workspace).ok())
            .unwrap_or_default();
        // Lookups run on their own thread and hand each entry over as it returns. A closed
        // dialog drops the receiver, which stops the remaining lookups.
        let (tx, rx) = std::sync::mpsc::channel();
        let shared = portable.clone();
        std::thread::Builder::new()
            .name("dah-profile-lookups".to_string())
            .spawn(move || {
                let closed = std::cell::Cell::new(false);
                let mut lookup = |id: u64| {
                    if closed.get() {
                        return Err(deadlock_core::Error::Network("The import was closed.".into()));
                    }
                    core::fetch_mod_files(id)
                };
                core::resolve_import_each(&shared, &installed, &mut lookup, &mut |entry| {
                    if tx.send(entry).is_err() {
                        closed.set(true);
                    } else if let Some(wake) = &lookup_wake {
                        wake();
                    }
                });
            })
            .map_err(|error| error.to_string())?;
        Ok((portable, rx))
    });
}

/// Take the entries whose lookups returned since the last frame. Once Import was pressed, a
/// kept entry that needs a download is queued right away. Returns true when anything changed.
fn take_lookups(st: &mut State) -> bool {
    let Some(rx) = st.profiles.import.lookups.as_ref() else {
        return false;
    };
    let mut arrived = Vec::new();
    let mut stopped = false;
    loop {
        match rx.try_recv() {
            Ok(entry) => arrived.push(entry),
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                stopped = true;
                break;
            }
        }
    }
    if arrived.is_empty() && !stopped {
        return false;
    }
    let import = &mut st.profiles.import;
    let installing = import.stage == Stage::Installing;
    for entry in arrived {
        let index = entry.index;
        if index >= import.entries.len() {
            continue;
        }
        import.keep[index] = entry.status != ImportStatus::Unavailable;
        import.status[index] = if installing && import.keep[index] && entry.needs_download() {
            RowStatus::Queued
        } else {
            RowStatus::Idle
        };
        import.entries[index] = entry;
    }
    if stopped {
        import.lookups = None;
        for (index, status) in import.status.iter_mut().enumerate() {
            if *status == RowStatus::LookingUp {
                *status = RowStatus::Idle;
                import.keep[index] = false;
                import.entries[index].reason = Some(tr("The lookup stopped before this mod was found.").to_string());
            }
        }
    }
    if installing && !import.install_job.is_running() {
        next_install(st);
    }
    true
}

/// Pick a profile file with the system dialog, then import it.
fn pick_import_file(st: &mut State, tx: &mut Tx) {
    let options = FileDialogOptions {
        title: "Open a profile".to_string(),
        ..FileDialogOptions::default()
    }
    .filter("Sigil profile", &["json"]);
    if let Some(last) = st.profiles.import.files.first() {
        let mut options = options.clone();
        if let Some(parent) = last.1.parent() {
            options.directory = Some(parent.to_path_buf());
        }
        tx.open_file(options, |st: &mut State, result, _tx| match result {
            Ok(Some(path)) => {
                st.profiles.import.input = TextEdit::new(path.display().to_string());
                start_resolve(st, path.display().to_string());
            }
            Ok(None) => {}
            Err(error) => {
                st.profiles.import.error = Some(format!("The file picker could not open: {error}"));
            }
        });
    } else {
        tx.open_file(options, |st: &mut State, result, _tx| match result {
            Ok(Some(path)) => {
                st.profiles.import.input = TextEdit::new(path.display().to_string());
                start_resolve(st, path.display().to_string());
            }
            Ok(None) => {}
            Err(error) => {
                st.profiles.import.error = Some(format!("The file picker could not open: {error}"));
            }
        });
    }
}

/// The import button: download what is missing (if anything is), then save the profile.
fn confirm_import(st: &mut State) {
    let import = &mut st.profiles.import;
    if import.name.text.trim().is_empty() {
        return;
    }
    let mut queued = false;
    for (index, entry) in import.entries.iter().enumerate() {
        if import.status[index] == RowStatus::LookingUp {
            // Queued as soon as its lookup returns.
            queued = true;
        } else if import.keep[index] && entry.needs_download() {
            import.status[index] = RowStatus::Queued;
            queued = true;
        }
    }
    if queued {
        import.stage = Stage::Installing;
        next_install(st);
    } else {
        finish_import(st);
    }
}

/// Start the next queued download, or save the profile when none are left.
/// Game install write per entry, through `core::install_import_entry`.
fn next_install(st: &mut State) {
    let import = &mut st.profiles.import;
    let Some(index) = import.status.iter().position(|status| *status == RowStatus::Queued) else {
        let looking_up = import.status.contains(&RowStatus::LookingUp);
        if import.stage == Stage::Installing && !looking_up {
            finish_import(st);
        }
        return;
    };
    import.status[index] = RowStatus::Downloading;
    if st.profiles.fixture {
        return;
    }
    let Some(game_root) = st.game_root.clone() else {
        st.profiles.import.status[index] =
            RowStatus::Failed("Deadlock was not found.".to_string());
        next_install(st);
        return;
    };
    let entry = st.profiles.import.entries[index].clone();
    let workspace = crate::data::workspace_dir();
    let first = !st.profiles.import.snapshot_taken;
    st.profiles.import.snapshot_taken = true;
    let label = format!("Before importing {}", st.profiles.import.name.text.trim());
    let wake = st.wake.clone();
    st.profiles.import.install_job.start(wake, move || {
        if first {
            let _ = core::capture_snapshot(
                &game_root,
                &workspace,
                core::SnapshotTrigger::PreImport,
                Some(&label),
            );
        }
        Ok((
            index,
            core::install_import_entry(&game_root, &workspace, &entry, true)
                .map_err(|error| error.to_string()),
        ))
    });
}

/// Save the imported profile from the kept entries that are on disk now. Workspace only.
fn finish_import(st: &mut State) {
    let import = &st.profiles.import;
    let Some(portable) = import.portable.clone() else {
        return;
    };
    let name = import.name.text.trim().to_string();
    let failed = import
        .status
        .iter()
        .filter(|status| matches!(status, RowStatus::Failed(_)))
        .count();
    let keep: Vec<bool> = import
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            import.keep[index]
                && (entry.installed_slot.is_some() || import.status[index] == RowStatus::Installed)
        })
        .collect();
    let include_autoexec = import.include_autoexec;
    let entries = import.entries.clone();
    close(st);
    st.profiles.import = ImportView::default();

    if st.profiles.fixture {
        report(st, format!("Imported {name}."), false);
        return;
    }
    let workspace = crate::data::workspace_dir();
    let installed = st
        .game_root
        .clone()
        .and_then(|root| core::installed_identities(&root, &workspace).ok())
        .unwrap_or_default();
    match core::create_profile_from_import(
        &workspace,
        &portable,
        &entries,
        &keep,
        &installed,
        &name,
        include_autoexec,
    ) {
        Ok(profile) => {
            let mut line = format!(
                "Imported {} with {}. Apply it to switch.",
                profile.name,
                plural(profile.slots.len(), "mod")
            );
            if failed > 0 {
                line.push_str(&format!(" {} could not be downloaded.", plural(failed, "mod")));
            }
            report(st, line, failed > 0);
            st.profiles.expanded = Some(profile.slug.clone());
            crate::data::scan_addons_now(st);
        }
        Err(error) => report(st, error.to_string(), true),
    }
    reload(st);
}

// ---------------------------------------------------------------------------
// Dialogs
// ---------------------------------------------------------------------------

/// Mounted at the root so the dialogs overlay the shell.
/// The row "..." menu, mounted at the root without a dimming layer.
pub fn overlay(s: &State, u: &Ui) -> Option<El> {
    if s.screen != Screen::Profiles || !matches!(s.profiles.dialog, Dialog::None) {
        return None;
    }
    s.profiles.more.as_deref().map(|slug| more_menu(u, slug))
}

pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    let v = &s.profiles;
    match &v.dialog {
        Dialog::None => None,
        Dialog::Create => Some(create_layer(s, u)),
        Dialog::Apply(slug) => find_row(s, slug).map(|row| apply_layer(s, u, row)),
        Dialog::Update(slug) => find_row(s, slug).map(|row| update_layer(s, u, row)),
        Dialog::Rename(slug) => find_row(s, slug).map(|row| rename_layer(s, u, row)),
        Dialog::Delete(slug) => find_row(s, slug).map(|row| delete_layer(u, row)),
        Dialog::Export(_) => v.export.as_ref().map(|export| export_layer(u, export)),
        Dialog::Import => Some(import_layer(s, u)),
        Dialog::SnapshotRestore(id) => Some(snapshot_restore_layer(s, u, id)),
        Dialog::SnapshotDelete(ids) => Some(snapshot_delete_layer(u, ids)),
    }
}

fn name_field(edit: &TextEdit, key: &str, placeholder: &str, u: &Ui, bind: fn(&mut State) -> &mut TextEdit) -> El {
    col()
        .w_full()
        .gap(6.0)
        .child(text(tr("Name")).font(SMALL).color(u.muted))
        .child(
            row()
                .w_full()
                .h(32.0)
                .px(10.0)
                .center_y()
                .radius(crate::ui::r(8.0))
                .fill(u.well)
                .border(1.0, u.hair)
                .child(
                    rune::text_input(edit, BODY)
                        .key(key.to_string())
                        .label("Profile name")
                        .single_line()
                        .grow(1.0)
                        .min_w(0.0)
                        .color(u.ink)
                        .placeholder(placeholder.to_string(), u.faint)
                        .caret(u.accent, u.accent.alpha(0.20))
                        .on_edit(bind),
                ),
        )
}

fn checkbox(u: &Ui, on: bool) -> El {
    let base = el().size(16.0, 16.0).radius(crate::ui::r(4.0)).center().a11y_hidden();
    if on {
        base.fill(u.ink)
            .child(icon(&icons::CHECK, 12.0).color(u.bg).a11y_hidden())
    } else {
        base.border(1.0, rune::Color::WHITE.alpha(0.22))
    }
}

fn check_row(u: &Ui, key: &str, label: &str, on: bool, press: impl Fn(&mut State) + 'static) -> El {
    row()
        .key(key.to_string())
        .h(32.0)
        .gap(10.0)
        .center_y()
        .role("checkbox")
        .checked(on)
        .label(label.to_string())
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .on_press(press)
        .child(checkbox(u, on))
        .child(text(label.to_string()).font(BODY).color(u.prose).a11y_hidden())
}

fn create_layer(s: &State, u: &Ui) -> El {
    let v = &s.profiles;
    let line = match v.current_mods {
        Some(count) if s.game_detected || v.fixture => {
            format!("Saves the {} on now, with your autoexec.", plural(count, "mod"))
        }
        _ if s.game_detected => "Saves the mods that are on now, with your autoexec.".to_string(),
        _ => "Deadlock was not found, so there is nothing to save yet.".to_string(),
    };
    let ready = !v.name.text.trim().is_empty() && (s.game_detected || v.fixture);
    let mut fields = col()
        .w_full()
        .gap(12.0)
        .child(name_field(&v.name, "profile-name", "Ranked", u, |st| &mut st.profiles.name));
    if v.current_crosshair {
        fields = fields.child(check_row(
            u,
            "profile-include-crosshair",
            "Include my current crosshair",
            v.include_crosshair,
            |st| st.profiles.include_crosshair = !st.profiles.include_crosshair,
        ));
    }
    fields = fields.child(check_row(
        u,
        "profile-include-isolated",
        "Keep these mods in their own folder",
        v.include_isolated,
        |st| st.profiles.include_isolated = !st.profiles.include_isolated,
    ));
    if v.include_isolated {
        fields = fields.child(
            text("Switching to it mounts citadel/addons/<its folder> in gameinfo.gi. The mods are copied, so they stay where they are.")
                .font(SMALL)
                .color(u.faint)
                .wrap(),
        );
    }
    let primary = enabled_action(u, tr("Save"), ActionKind::Primary, ready, create);
    house_dialog(u, "profile-create", "New profile", &line, Some(fields), primary, 480.0)
}

fn apply_layer(s: &State, u: &Ui, profile: &Row) -> El {
    let slug = profile.slug.clone();
    let fixture_plan;
    let plan = match &s.profiles.plan {
        Some(plan) => plan,
        None => {
            fixture_plan = PlanView {
                turn_on: Vec::new(),
                turn_off: Vec::new(),
                missing: Vec::new(),
                autoexec: None,
                blocked: None,
            };
            &fixture_plan
        }
    };
    let line = if plan.in_sync() {
        "Everything already matches this profile.".to_string()
    } else {
        "Mods are switched on or off to match. Nothing is deleted.".to_string()
    };
    let mut rows = col().w_full();
    if !plan.turn_on.is_empty() {
        rows = rows.child(u.kv(tr("Turns on"), &names(&plan.turn_on), u.ink));
    }
    if !plan.turn_off.is_empty() {
        rows = rows.child(u.kv(tr("Turns off"), &names(&plan.turn_off), u.ink));
    }
    if !plan.missing.is_empty() {
        rows = rows.child(u.kv(tr("Not installed"), &names(&plan.missing), u.amber));
    }
    if let Some(autoexec) = &plan.autoexec {
        rows = rows.child(u.kv(tr("Autoexec"), &format!("Replaced with {autoexec}"), u.ink));
    }
    if profile.isolated {
        let folder = core::profile_folder_name(&profile.slug)
            .unwrap_or_else(|_| "its own folder".to_string());
        rows = rows.child(u.kv(tr("Folder"), &format!("citadel/addons/{folder}"), u.ink));
        rows = rows.child(
            text("Its mods are copied into that folder and gameinfo.gi mounts it, so other profiles do not load.")
                .font(SMALL)
                .color(u.faint)
                .wrap(),
        );
    }
    if let Some(blocked) = &plan.blocked {
        rows = rows.child(u.kv(tr("Blocked"), blocked, u.rose));
    }
    let ready = plan.blocked.is_none() && !s.profiles.busy;
    let label = if plan.in_sync() { "Re-apply" } else { tr("Apply") };
    let primary = enabled_action(u, label, ActionKind::Primary, ready, move |st| apply(st, &slug));
    house_dialog(
        u,
        "profile-apply",
        &format!("Apply {}", profile.name),
        &line,
        Some(rows),
        primary,
        480.0,
    )
}

fn update_layer(s: &State, u: &Ui, profile: &Row) -> El {
    let slug = profile.slug.clone();
    let mut rows = col().w_full();
    if let Some(count) = s.profiles.current_mods {
        rows = rows.child(u.kv("Mods on now", &plural(count, "mod"), u.ink));
    }
    rows = rows.child(u.kv("Saved before", &plural(profile.mods.len(), "mod"), u.muted));
    let ready = s.game_detected || s.profiles.fixture;
    let primary = enabled_action(u, tr("Update"), ActionKind::Primary, ready, move |st| update(st, &slug));
    house_dialog(
        u,
        "profile-update",
        &format!("Update {}", profile.name),
        "Its mods and autoexec are replaced with what you have now. The crosshair stays.",
        Some(rows),
        primary,
        460.0,
    )
}

fn rename_layer(s: &State, u: &Ui, profile: &Row) -> El {
    let slug = profile.slug.clone();
    let ready = !s.profiles.name.text.trim().is_empty();
    let field = name_field(&s.profiles.name, "profile-rename-name", &profile.name, u, |st| {
        &mut st.profiles.name
    });
    let primary = enabled_action(u, tr("Rename"), ActionKind::Primary, ready, move |st| rename(st, &slug));
    house_dialog(u, "profile-rename", "Rename profile", "", Some(field), primary, 440.0)
}

fn delete_layer(u: &Ui, profile: &Row) -> El {
    let slug = profile.slug.clone();
    let line = if profile.isolated {
        "The saved profile goes away. Its folder in citadel/addons stays, so nothing is deleted."
    } else {
        "The saved profile goes away. Your mods stay as they are."
    };
    let primary = u
        .action(tr("Delete"), ActionKind::Danger)
        .key("profile-delete-confirm")
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| delete(st, &slug));
    house_dialog(
        u,
        "profile-delete",
        &format!("Delete {}", profile.name),
        line,
        None,
        primary,
        440.0,
    )
}

fn snapshot_restore_layer(s: &State, u: &Ui, id: &str) -> El {
    let Some(snapshot) = s.profiles.snapshots.iter().find(|row| row.id == id) else {
        return el().key("snapshot-restore-missing");
    };
    let plan = s.profiles.snapshot_plan.clone().unwrap_or(PlanView {
        turn_on: Vec::new(),
        turn_off: Vec::new(),
        missing: Vec::new(),
        autoexec: None,
        blocked: None,
    });
    let line = if plan.in_sync() {
        "Everything already matches this recovery point.".to_string()
    } else {
        "Mods are switched on or off to match it. Nothing is deleted.".to_string()
    };
    let mut rows = col().w_full();
    rows = rows.child(u.kv(tr("Saved"), &snapshot.created, u.ink));
    if !plan.turn_on.is_empty() {
        rows = rows.child(u.kv(tr("Turns on"), &names(&plan.turn_on), u.ink));
    }
    if !plan.turn_off.is_empty() {
        rows = rows.child(u.kv(tr("Turns off"), &names(&plan.turn_off), u.ink));
    }
    if !plan.missing.is_empty() {
        rows = rows.child(u.kv(tr("Not installed"), &names(&plan.missing), u.amber));
    }
    if let Some(autoexec) = &plan.autoexec {
        rows = rows.child(u.kv(tr("Autoexec"), &format!("Replaced with {autoexec}"), u.ink));
    }
    if let Some(blocked) = &plan.blocked {
        rows = rows.child(u.kv(tr("Blocked"), blocked, u.rose));
    }
    let ready = plan.blocked.is_none() && !s.profiles.busy;
    let target = id.to_string();
    let primary = enabled_action(u, tr("Restore"), ActionKind::Primary, ready, move |st| {
        restore_snapshot(st, &target)
    });
    house_dialog(
        u,
        "snapshot-restore",
        &format!("Restore {}", snapshot.label),
        &line,
        Some(rows),
        primary,
        480.0,
    )
}

fn snapshot_delete_layer(u: &Ui, ids: &[String]) -> El {
    let count = ids.len();
    let title = if count == 1 {
        "Delete snapshot".to_string()
    } else {
        format!("Delete {count} snapshots")
    };
    let line = if count == 1 {
        "This recovery point goes away. Your mods stay as they are."
    } else {
        "These recovery points go away. Your mods stay as they are."
    };
    let targets = ids.to_vec();
    let primary = u
        .action(tr("Delete"), ActionKind::Danger)
        .key("snapshot-delete-confirm")
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| delete_snapshots(st, targets.clone()));
    house_dialog(u, "snapshot-delete", &title, line, None, primary, 440.0)
}

fn export_layer(u: &Ui, export: &ExportView) -> El {
    let mut summary = vec![plural(export.included, "mod")];
    if export.crosshair {
        summary.push("crosshair".to_string());
    }
    if export.autoexec > 0 {
        summary.push(plural(export.autoexec, "autoexec line"));
    }
    let line = format!("{} in this share.", sentence_case(&summary.join(", ")));

    let mut content = col().w_full().gap(16.0);
    if !export.skipped.is_empty() {
        content = content.child(
            row()
                .w_full()
                .gap(10.0)
                .items_start()
                .child(
                    el().pt(3.0).child(
                        icon(&icons::TRIANGLE_ALERT, 14.0)
                            .color(u.amber)
                            .a11y_hidden(),
                    ),
                )
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(
                            text(format!(
                                "{} left out. Only mods from GameBanana can be shared.",
                                plural(export.skipped.len(), "local mod")
                            ))
                            .font(BODY)
                            .color(u.prose)
                            .wrap(),
                        )
                        .child(
                            text(names(&export.skipped))
                                .font(SMALL)
                                .color(u.faint)
                                .wrap(),
                        ),
                ),
        );
    }
    content = content.child(
        col()
            .w_full()
            .gap(6.0)
            .child(text("Share code").font(SMALL).color(u.muted))
            .child(
                col()
                    .key("profile-share-code")
                    .w_full()
                    .px(12.0)
                    .py(10.0)
                    .radius(crate::ui::r(8.0))
                    .fill(u.well)
                    .border(1.0, u.hair)
                    .child(
                        text(export.share_code.clone())
                            .font(MONO)
                            .color(u.muted)
                            .wrap_anywhere()
                            .max_lines(4),
                    ),
            ),
    );
    if let Some(path) = &export.saved {
        let reveal = path.display().to_string();
        content = content.child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(icon(&icons::CHECK, 14.0).color(u.muted).a11y_hidden())
                .child(
                    text(format!("Saved to {}", short_path(path)))
                        .font(BODY)
                        .color(u.muted)
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate(),
                )
                .child(
                    u.action(tr("Show"), ActionKind::Ghost)
                        .key("profile-export-reveal")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| {
                            crate::data::reveal_path(st, reveal.clone())
                        }),
                ),
        );
    }
    if let Some(error) = &export.error {
        content = content.child(crate::notify::feedback::error_line(u, "profiles-export-error", error));
    }

    let code = export.share_code.clone();
    let copy = u
        .icon_action(
            if export.copied {
                &icons::CHECK
            } else {
                &icons::COPY
            },
            if export.copied { tr("Copied") } else { tr("Copy code") },
            ActionKind::Primary,
        )
        .key("profile-export-copy")
        .cursor(rune::Cursor::Pointer)
        .on_press_tx(move |st: &mut State, tx: &mut Tx| {
            tx.copy(code.clone());
            if let Some(export) = st.profiles.export.as_mut() {
                export.copied = true;
            }
        });
    let save = u
        .icon_action(&icons::DOWNLOAD, tr("Save file"), ActionKind::Secondary)
        .key("profile-export-save")
        .cursor(rune::Cursor::Pointer)
        .on_press_tx(pick_export);

    dialog_shell(u, "profile-export", &format!("Share {}", export.name), 520.0)
        .child(title_block(u, &format!("Share {}", export.name), &line))
        .child(content)
        .child(footer(u, tr("Close"), vec![save, copy]))
}

fn import_layer(s: &State, u: &Ui) -> El {
    let import = &s.profiles.import;
    match import.stage {
        Stage::Input => import_input(s, u),
        Stage::Resolving => {
            let count = import
                .portable
                .as_ref()
                .map(|portable| portable.mods.len())
                .unwrap_or(0);
            dialog_shell(u, "profile-import", "Import profile", 520.0)
                .child(title_block(u, "Import profile", ""))
                .child(
                    row()
                        .w_full()
                        .h(32.0)
                        .gap(10.0)
                        .center_y()
                        .child(icon(&icons::LOADER, 16.0).color(u.muted).a11y_hidden())
                        .child(
                            text(format!("Looking up {} on GameBanana", plural(count, "mod")))
                                .font(BODY)
                                .color(u.muted),
                        ),
                )
                .child(footer(u, tr("Cancel"), Vec::new()))
        }
        Stage::Review | Stage::Installing => import_review(s, u),
    }
}

fn import_input(s: &State, u: &Ui) -> El {
    let import = &s.profiles.import;
    let field = col()
        .w_full()
        .h(96.0)
        .px(12.0)
        .py(10.0)
        .radius(crate::ui::r(8.0))
        .fill(u.well)
        .border(1.0, u.hair)
        .child(
            rune::text_input(&import.input, MONO)
                .key("profile-import-input")
                .label("Share code or profile JSON")
                .w_full()
                .h_full()
                .color(u.ink)
                .placeholder("mp1:...".to_string(), u.faint)
                .caret(u.accent, u.accent.alpha(0.20))
                .on_edit(|st: &mut State| &mut st.profiles.import.input),
        );
    let mut content = col().w_full().gap(16.0).child(field);
    if let Some(error) = &import.error {
        content = content.child(crate::notify::feedback::error_line(u, "profiles-import-error", error));
    }
    if !import.files.is_empty() {
        let mut list = col().w_full();
        for (name, path, folder) in &import.files {
            let input = path.display().to_string();
            list = list.child(
                row()
                    .key(format!("profile-import-file-{name}"))
                    .w_full()
                    .h(40.0)
                    .px(8.0)
                    .gap(10.0)
                    .center_y()
                    .radius(crate::ui::r(8.0))
                    .hover(u.hover)
                    .focusable(true)
                    .role("button")
                    .label(format!("Import {name}"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        st.profiles.import.input = TextEdit::new(&input);
                        start_resolve(st, input.clone());
                    })
                    .child(icon(&icons::FILE, 16.0).color(u.muted).a11y_hidden())
                    .child(
                        text(name.clone())
                            .font(BODY)
                            .color(u.prose)
                            .grow(1.0)
                            .min_w(0.0)
                            .truncate(),
                    )
                    .child(text(folder.clone()).font(SMALL).color(u.faint)),
            );
        }
        content = content.child(
            col()
                .w_full()
                .gap(6.0)
                .child(text("Profile files").font(SMALL).color(u.muted))
                .child(el().ml(-8.0).mr(-8.0).child(list)),
        );
    }
    let ready = !import.input.text.trim().is_empty();
    let primary = enabled_action(u, tr("Continue"), ActionKind::Primary, ready, |st| {
        let input = st.profiles.import.input.text.clone();
        start_resolve(st, input);
    });
    let pick = u
        .icon_action(&icons::FILE, tr("Choose file"), ActionKind::Secondary)
        .key("profile-import-pick")
        .cursor(rune::Cursor::Pointer)
        .on_press_tx(pick_import_file);
    let line = if import.files.is_empty() {
        "Paste a share code or profile JSON, or pick a .modprofile.json file."
    } else {
        "Paste a share code or profile JSON, or pick a file below."
    };
    dialog_shell(u, "profile-import", "Import profile", 520.0)
        .child(title_block(u, "Import profile", line))
        .child(content)
        .child(footer(u, tr("Cancel"), vec![pick, primary]))
}

fn import_review(s: &State, u: &Ui) -> El {
    let import = &s.profiles.import;
    let installing = import.stage == Stage::Installing;
    let portable = import.portable.as_ref();
    let title = format!(
        "Import {}",
        portable.map(|p| p.profile.name.as_str()).unwrap_or("profile")
    );

    let kept: Vec<&ImportEntry> = import
        .entries
        .iter()
        .enumerate()
        .filter(|(index, _)| import.keep[*index])
        .map(|(_, entry)| entry)
        .collect();
    let on_disk = kept.iter().filter(|entry| entry.installed_slot.is_some()).count();
    let downloads = kept.iter().filter(|entry| entry.needs_download()).count();
    let looking_up = import.status.iter().filter(|status| **status == RowStatus::LookingUp).count();
    let unavailable = import
        .entries
        .iter()
        .zip(&import.status)
        .filter(|(entry, status)| entry.status == ImportStatus::Unavailable && **status != RowStatus::LookingUp)
        .count();

    let line = if installing {
        let done = import
            .status
            .iter()
            .filter(|status| matches!(status, RowStatus::Installed | RowStatus::Failed(_)))
            .count();
        let total = import
            .status
            .iter()
            .filter(|status| !matches!(status, RowStatus::Idle | RowStatus::LookingUp))
            .count();
        let mut line = format!("Downloading {} of {total}.", (done + 1).min(total));
        if looking_up > 0 {
            line.push(' ');
            line.push_str(&fill(tr("Looking up {} more."), &[&looking_up.to_string()]));
        }
        line
    } else {
        let mut parts = Vec::new();
        if looking_up > 0 {
            parts.push(fill(tr("looking up {}"), &[&looking_up.to_string()]));
        }
        if on_disk > 0 {
            parts.push(format!("{on_disk} already installed"));
        }
        if downloads > 0 {
            parts.push(format!("{downloads} to download"));
        }
        if unavailable > 0 {
            parts.push(format!("{unavailable} unavailable"));
        }
        let mut line = if parts.is_empty() {
            "Nothing selected.".to_string()
        } else {
            format!("{}.", sentence_case(&parts.join(", ")))
        };
        if let Some(author) = portable.and_then(|p| p.profile.author.as_deref()) {
            line = format!("Shared by {author}. {line}");
        }
        line
    };

    let mut list = col().w_full();
    for (index, entry) in import.entries.iter().enumerate() {
        list = list.child(import_row(u, entry, import.keep[index], &import.status[index], index, installing));
    }
    let list = col()
        .key("profile-import-list")
        .w_full()
        .max_h(264.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .border_sides([1.0, 0.0, 1.0, 0.0], u.hair)
        .child(list);

    let mut content = col()
        .w_full()
        .gap(16.0)
        .child(name_field(&import.name, "profile-import-name", "Shared profile", u, |st| {
            &mut st.profiles.import.name
        }))
        .child(list);

    let commands = portable.map(PortableProfile::autoexec_commands).unwrap_or_default();
    let crosshair = portable.and_then(PortableProfile::crosshair).is_some();
    if !commands.is_empty() {
        let on = import.include_autoexec;
        content = content.child(
            col()
                .w_full()
                .gap(4.0)
                .child(check_row(
                    u,
                    "profile-import-autoexec",
                    &format!("Include {}", plural(commands.len(), "autoexec line")),
                    on,
                    |st| st.profiles.import.include_autoexec = !st.profiles.import.include_autoexec,
                ))
                .child(command_well(u, &commands, 8)),
        );
    }
    if crosshair {
        content = content.child(
            row()
                .gap(8.0)
                .center_y()
                .child(icon(&icons::CROSSHAIR, 14.0).color(u.muted).a11y_hidden())
                .child(
                    text("Includes a crosshair, written when you apply the profile.")
                        .font(BODY)
                        .color(u.muted),
                ),
        );
    }

    let label = if looking_up > 0 {
        tr("Download and import").to_string()
    } else if downloads > 0 {
        format!("Download {downloads} and import")
    } else {
        tr("Import").to_string()
    };
    let ready = !installing && !import.name.text.trim().is_empty() && !s.profiles.busy;
    let primary = enabled_action(u, &label, ActionKind::Primary, ready, confirm_import);

    dialog_shell(u, "profile-import", &title, 560.0)
        .child(title_block(u, &title, &line))
        .child(content)
        .child(footer(u, tr("Cancel"), vec![primary]))
}

fn import_row(
    u: &Ui,
    entry: &ImportEntry,
    keep: bool,
    status: &RowStatus,
    index: usize,
    installing: bool,
) -> El {
    let looking_up = *status == RowStatus::LookingUp;
    let unavailable = entry.status == ImportStatus::Unavailable && !looking_up;
    let sub = if let Some(reason) = &entry.reason {
        Some((reason.clone(), u.faint))
    } else if entry.status == ImportStatus::Upgraded {
        Some(("The shared file is gone. The newest one is used.".to_string(), u.faint))
    } else if !entry.enabled {
        Some(("Off in this profile".to_string(), u.faint))
    } else {
        None
    };
    let (state, state_color) = match status {
        RowStatus::LookingUp => (tr("Looking up").to_string(), u.faint),
        RowStatus::Queued => ("Queued".to_string(), u.faint),
        RowStatus::Downloading => ("Downloading".to_string(), u.muted),
        RowStatus::Installed => ("Installed".to_string(), u.muted),
        RowStatus::Failed(_) => ("Failed".to_string(), u.rose),
        RowStatus::Idle if unavailable => ("Unavailable".to_string(), u.amber),
        RowStatus::Idle if entry.installed_slot.is_some() => ("Installed".to_string(), u.faint),
        RowStatus::Idle if keep => ("Download".to_string(), u.muted),
        RowStatus::Idle => (String::new(), u.faint),
    };
    let sub = match status {
        RowStatus::Failed(error) => Some((error.clone(), u.rose)),
        _ => sub,
    };

    let mut label_col = col().grow(1.0).min_w(0.0).gap(0.0).child(
        text(entry.label.clone())
            .font(BODY)
            .color(if unavailable { u.muted } else { u.prose })
            .truncate(),
    );
    if let Some((sub, color)) = sub {
        label_col = label_col.child(text(sub).font(SMALL).color(color).truncate());
    }

    let mut line = row()
        .key(format!("profile-import-row-{index}"))
        .w_full()
        .h(48.0)
        .gap(12.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair);
    line = if unavailable {
        line.child(el().size(16.0, 16.0).radius(crate::ui::r(4.0)).border(1.0, rune::Color::WHITE.alpha(0.10)))
    } else if installing || looking_up {
        line.child(checkbox(u, keep).opacity(0.5))
    } else {
        line.role("checkbox")
            .checked(keep)
            .label(format!("Keep {}", entry.label))
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| {
                if let Some(value) = st.profiles.import.keep.get_mut(index) {
                    *value = !*value;
                }
            })
            .child(checkbox(u, keep))
    };
    let mut right = row().gap(6.0).center_y();
    if matches!(status, RowStatus::Downloading | RowStatus::LookingUp) {
        right = right.child(icon(&icons::LOADER, 14.0).color(u.muted).a11y_hidden());
    }
    right = right.child(text(state).font(SMALL).color(state_color));
    line.child(label_col).child(right)
}

// ---------------------------------------------------------------------------
// Dialog parts
// ---------------------------------------------------------------------------

fn close_import(st: &mut State) {
    if st.profiles.import.stage == Stage::Installing {
        return;
    }
    st.profiles.import = ImportView::default();
    close(st);
}

/// Escape, the scrim and Cancel: the import dialog keeps going while it downloads.
fn dismiss(st: &mut State) {
    if matches!(st.profiles.dialog, Dialog::Import) {
        close_import(st);
    } else {
        close(st);
    }
}

fn dialog_shell(u: &Ui, key: &str, title: &str, width: f32) -> El {
    u.dialog_shell()
        .key(key.to_string())
        .role("dialog")
        .label(title.to_string())
        .w(width)
        .padding(24.0)
        .gap(20.0)
        .modal(true, dismiss)
        .on_key(move |st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                dismiss(st);
                true
            } else {
                false
            }
        })
}

fn title_block(u: &Ui, title: &str, line: &str) -> El {
    let mut block = col()
        .w_full()
        .gap(6.0)
        .child(text(title.to_string()).font(H1).color(u.ink).wrap());
    if !line.is_empty() {
        block = block.child(text(line.to_string()).font(BODY).color(u.muted).wrap());
    }
    block
}

fn footer(u: &Ui, cancel: &str, actions: Vec<El>) -> El {
    let mut bar = row().w_full().gap(8.0).pt(4.0).child(spacer()).child(
        u.action(cancel, ActionKind::Ghost)
            .key("profile-dialog-cancel")
            .cursor(rune::Cursor::Pointer)
            .on_press(dismiss),
    );
    for action in actions {
        bar = bar.child(action);
    }
    bar
}

fn house_dialog(
    u: &Ui,
    key: &str,
    title: &str,
    line: &str,
    content: Option<El>,
    primary: El,
    width: f32,
) -> El {
    let mut shell = dialog_shell(u, key, title, width).child(title_block(u, title, line));
    if let Some(content) = content {
        shell = shell.child(content);
    }
    shell.child(footer(u, tr("Cancel"), vec![primary]))
}

fn enabled_action(
    u: &Ui,
    label: &str,
    kind: ActionKind,
    ready: bool,
    press: impl Fn(&mut State) + 'static,
) -> El {
    u.action(label, kind)
        .key(format!("profile-dialog-{}", label.to_lowercase().replace(' ', "-")))
        .disabled(!ready)
        .opacity(if ready { 1.0 } else { 0.5 })
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            if ready {
                press(st);
            }
        })
}

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

fn plural(n: usize, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

/// `A, B, C and 2 more`.
fn names(list: &[String]) -> String {
    match list.len() {
        0 => String::new(),
        1..=3 => list.join(", "),
        n => format!("{} and {} more", list[..3].join(", "), n - 3),
    }
}

fn sentence_case(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// A network or install error without the plumbing prefix.
pub(crate) fn friendly(error: &str) -> String {
    let trimmed = error.trim_start_matches("network error: ");
    trimmed.split(". ").next().unwrap_or(trimmed).trim_end_matches('.').to_string()
}

/// `2026-11-17T10:22:01.000Z` as `Nov 17`, with the year when it is not this year.
pub fn short_date(iso: &str) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let year = iso.get(0..4).unwrap_or("");
    let month = iso.get(5..7).and_then(|m| m.parse::<usize>().ok()).unwrap_or(0);
    let day = iso.get(8..10).and_then(|d| d.parse::<u32>().ok()).unwrap_or(0);
    if month == 0 || month > 12 || day == 0 {
        return iso.to_string();
    }
    let this_year = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|age| 1970 + age.as_secs() / 31_556_952)
        .unwrap_or(0)
        .to_string();
    if year == this_year {
        format!("{} {day}", MONTHS[month - 1])
    } else {
        format!("{} {day}, {year}", MONTHS[month - 1])
    }
}

// ---------------------------------------------------------------------------
// Titlebar profile switcher
// ---------------------------------------------------------------------------

/// The active profile in the titlebar. Pressing it opens the saved profiles, so switching is
/// one press plus one. Mount it inside the titlebar row; the menu hangs under the chip.
pub fn titlebar_switcher(s: &State, u: &Ui) -> El {
    let active = s
        .profiles
        .active
        .as_ref()
        .and_then(|slug| s.profiles.rows.iter().find(|row| &row.slug == slug));
    let label = active
        .map(|row| row.name.clone())
        .unwrap_or_else(|| tr("No profile").to_string());
    let has_profiles = !s.profiles.rows.is_empty();
    let chip = row()
        .key("titlebar-profile")
        .h(28.0)
        .px(10.0)
        .gap(6.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .hover(u.hover)
        .focusable(true)
        .role("button")
        .label(format!("Profile: {label}. Switch profile"))
        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| {
            if !st.profiles.rows.is_empty() {
                st.profiles.titlebar_open = !st.profiles.titlebar_open;
            } else {
                st.go(Screen::Profiles);
            }
        })
        .child(icon(&icons::LAYERS, 14.0).color(u.muted).a11y_hidden())
        .child(
            text(label)
                .font(SMALL)
                .color(if active.is_some() { u.prose } else { u.muted })
                .max_w(150.0)
                .truncate()
                .a11y_hidden(),
        )
        .child(icon(&icons::CHEVRON_DOWN, 12.0).color(u.faint).a11y_hidden());

    if !s.profiles.titlebar_open || !has_profiles {
        return chip;
    }

    let mut items = col().w_full().gap(0.0);
    for profile in &s.profiles.rows {
        let slug = profile.slug.clone();
        let active = s.profiles.active.as_deref() == Some(profile.slug.as_str());
        items = items.child(
            row()
                .key(format!("titlebar-profile-{}", profile.slug))
                .w_full()
                .h(32.0)
                .px(10.0)
                .gap(8.0)
                .center_y()
                .radius(crate::ui::r(6.0))
                .hover(u.hover)
                .focusable(true)
                .role("menuitem")
                .checked(active)
                .label(profile.name.clone())
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| apply(st, &slug))
                .child(
                    text(profile.name.clone())
                        .font(BODY)
                        .color(if active { u.ink } else { u.prose })
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate()
                        .a11y_hidden(),
                )
                .child(
                    text(plural(profile.mods.len(), "mod"))
                        .font(SMALL)
                        .color(u.faint)
                        .a11y_hidden(),
                )
                .child(if active {
                    icon(&icons::CHECK, 14.0).color(u.muted).a11y_hidden()
                } else {
                    el().size(14.0, 14.0)
                }),
        );
    }
    items = items
        .child(el().w_full().h(1.0).my(4.0).fill(u.hair))
        .child(
            row()
                .key("titlebar-profile-manage")
                .w_full()
                .h(32.0)
                .px(10.0)
                .gap(8.0)
                .center_y()
                .radius(crate::ui::r(6.0))
                .hover(u.hover)
                .focusable(true)
                .role("menuitem")
                .label(tr("Manage profiles"))
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| {
                    st.profiles.titlebar_open = false;
                    st.go(Screen::Profiles);
                })
                .child(icon(&icons::SLIDERS_HORIZONTAL, 14.0).color(u.muted).a11y_hidden())
                .child(text(tr("Manage profiles")).font(BODY).color(u.prose).a11y_hidden()),
        );

    chip.child(
        col()
            .key("titlebar-profile-menu")
            .role("menu")
            .w(260.0)
            .padding(4.0)
            .fill(u.raised)
            .radius(crate::ui::r(10.0))
            .border(1.0, u.hair_strong)
            .shadow(12.0, 32.0, crate::ui::soft(rune::Color::BLACK.alpha(0.5)))
            .anchored("titlebar-profile", Side::Bottom, Align::Start, 6.0)
            .transition(crate::motion::pop(false))
            .modal(true, |st: &mut State| st.profiles.titlebar_open = false)
            .on_key(|st: &mut State, ev: &KeyEvent, _| {
                if ev.key == Key::Escape {
                    st.profiles.titlebar_open = false;
                    true
                } else {
                    false
                }
            })
            .child(items),
    )
}

// ---------------------------------------------------------------------------
// Fixture scenes
// ---------------------------------------------------------------------------

fn fixture_profiles() -> Vec<Profile> {
    let slot = |n: u16, name: &str, ids: Option<(u64, u64)>| core::ProfileSlot {
        slot: n,
        file: format!("pak{n:02}_dir.vpk"),
        enabled: true,
        name: Some(name.to_string()),
        gamebanana_id: ids.map(|ids| ids.0),
        gamebanana_file_id: ids.map(|ids| ids.1),
        fingerprint: Some(format!("q1:{n}")),
    };
    let base = |name: &str, slug: &str, slots: Vec<core::ProfileSlot>| Profile {
        schema: core::PROFILE_SCHEMA,
        name: name.to_string(),
        slug: slug.to_string(),
        created_at: "2026-11-02T18:22:01.000Z".to_string(),
        updated_at: "2026-11-14T18:22:01.000Z".to_string(),
        slots,
        crosshair: None,
        autoexec: None,
        author: None,
        extensions: Default::default(),
        isolated: false,
    };
    let mut ranked = base(
        "Ranked",
        "ranked",
        vec![
            slot(1, "Clean Minimal HUD", Some((581200, 1390021))),
            slot(2, "Haze: Midnight Bloom", Some((612455, 1412987))),
            slot(3, "Quiet Footsteps", Some((598110, 1402311))),
            slot(5, "Low Smoke Abilities", None),
        ],
    );
    ranked.crosshair = Some(Crosshair::default());
    ranked.autoexec = Some(vec![
        "fps_max 240".to_string(),
        "cl_showfps 1".to_string(),
        "r_fullscreen_gamma 2.2".to_string(),
    ]);
    ranked.updated_at = "2026-11-17T09:02:00.000Z".to_string();
    let mut casual = base(
        "Casual skins",
        "casual-skins",
        vec![
            slot(2, "Haze: Midnight Bloom", Some((612455, 1412987))),
            slot(4, "Lash: Golden Hour", Some((604002, 1408840))),
            slot(6, "Seven: Storm Chaser", Some((615774, 1417002))),
            slot(7, "Vindicta: Raven", Some((588120, 1396632))),
            slot(8, "Anime Kill Sounds", Some((590044, 1399870))),
            slot(9, "Paradox: Neon", Some((617300, 1419100))),
        ],
    );
    casual.author = Some("slush".to_string());
    let vanilla = base("Vanilla", "vanilla", Vec::new());
    vec![casual, ranked, vanilla]
}

fn fixture_load(st: &mut State) {
    let profiles = fixture_profiles();
    let rows: Vec<Row> = profiles
        .iter()
        .map(|profile| {
            let mut row = row_from(profile, None, None);
            for line in &mut row.mods {
                line.state = LineState::Ready;
            }
            if profile.slug == "casual-skins" {
                row.mods[3].state = LineState::Missing;
                row.mods[5].state = LineState::Missing;
                row.missing = 2;
            }
            if profile.slug == "ranked" {
                row.in_sync = Some(true);
            }
            row
        })
        .collect();
    st.screen = crate::model::Screen::Profiles;
    st.profiles = View {
        rows,
        loaded: true,
        active: Some("ranked".to_string()),
        current_mods: Some(4),
        current_crosshair: true,
        snapshots: fixture_snapshots(),
        fixture: true,
        ..View::default()
    };
}

fn fixture_snapshots() -> Vec<SnapshotRow> {
    vec![
        SnapshotRow {
            id: "snap-1".to_string(),
            label: "Before applying Ranked".to_string(),
            created: "Nov 17".to_string(),
            reason: "Before a profile switch".to_string(),
            mods: 6,
        },
        SnapshotRow {
            id: "snap-2".to_string(),
            label: "Before updating Casual skins".to_string(),
            created: "Nov 14".to_string(),
            reason: "Before a profile switch".to_string(),
            mods: 5,
        },
        SnapshotRow {
            id: "snap-3".to_string(),
            label: tr("Snapshot").to_string(),
            created: "Nov 12".to_string(),
            reason: tr("Snapshot").to_string(),
            mods: 4,
        },
    ]
}

/// A populated list with the active profile expanded.
pub fn scene_list(st: &mut State) {
    fixture_load(st);
    st.profiles.expanded = Some("ranked".to_string());
}

/// The snapshots section open, with two points picked for a bulk delete.
pub fn scene_snapshots(st: &mut State) {
    fixture_load(st);
    st.profiles.snapshots_open = true;
    st.profiles.snapshot_selected = vec!["snap-1".to_string(), "snap-2".to_string()];
}

/// The restore confirm for a recovery point that turns a mod back on.
pub fn scene_snapshot_restore(st: &mut State) {
    fixture_load(st);
    st.profiles.snapshots_open = true;
    st.profiles.snapshot_plan = Some(PlanView {
        turn_on: vec!["Quiet Footsteps".into()],
        turn_off: vec!["Paradox: Neon".into()],
        missing: Vec::new(),
        autoexec: None,
        blocked: None,
    });
    st.profiles.dialog = Dialog::SnapshotRestore("snap-1".to_string());
}

/// The titlebar switcher open over the profiles page.
pub fn scene_titlebar(st: &mut State) {
    fixture_load(st);
    st.screen = crate::model::Screen::Browse;
    st.profiles.titlebar_open = true;
}

/// The apply confirm for a profile with missing mods.
pub fn scene_apply(st: &mut State) {
    fixture_load(st);
    st.profiles.plan = Some(PlanView {
        turn_on: vec![
            "Lash: Golden Hour".into(),
            "Seven: Storm Chaser".into(),
            "Anime Kill Sounds".into(),
        ],
        turn_off: vec!["Clean Minimal HUD".into(), "Quiet Footsteps".into()],
        missing: vec!["Vindicta: Raven".into(), "Paradox: Neon".into()],
        autoexec: None,
        blocked: None,
    });
    st.profiles.dialog = Dialog::Apply("casual-skins".to_string());
}

/// The share dialog with a real share code built from the fixture profile.
pub fn scene_export(st: &mut State) {
    fixture_load(st);
    let profile = fixture_profiles()
        .into_iter()
        .find(|profile| profile.slug == "ranked")
        .expect("fixture has ranked");
    if let Ok(export) = core::export_portable(&profile, &[], env!("CARGO_PKG_VERSION")) {
        st.profiles.export = Some(export_view(&profile, &export));
    }
    st.profiles.dialog = Dialog::Export("ranked".to_string());
}

/// The import review: some mods installed, some to download, one newer file, one gone.
pub fn scene_import(st: &mut State) {
    fixture_load(st);
    let mut profile = fixture_profiles().remove(0);
    profile.autoexec = Some(vec!["fps_max 180".into(), "cl_showfps 1".into()]);
    profile.crosshair = Some(Crosshair::default());
    let Ok(export) = core::export_portable(&profile, &[], "0.1.0") else {
        return;
    };
    let portable = export.profile;
    let entry = |index: usize, label: &str, status: ImportStatus, installed: Option<u16>| ImportEntry {
        index,
        label: label.to_string(),
        enabled: true,
        nsfw: false,
        mod_id: Some(600_000 + index as u64),
        pinned_file_id: Some(1_400_000 + index as u64),
        resolved_file_id: (status != ImportStatus::Unavailable).then_some(1_400_000 + index as u64),
        download_url: (status != ImportStatus::Unavailable)
            .then(|| format!("https://gamebanana.com/dl/{}", 1_400_000 + index)),
        file_name: None,
        status,
        reason: (status == ImportStatus::Unavailable)
            .then(|| "This mod was removed from GameBanana".to_string()),
        installed_slot: installed,
    };
    let entries = vec![
        entry(0, "Haze: Midnight Bloom", ImportStatus::Exact, Some(2)),
        entry(1, "Lash: Golden Hour", ImportStatus::Exact, None),
        entry(2, "Seven: Storm Chaser", ImportStatus::Upgraded, None),
        entry(3, "Vindicta: Raven", ImportStatus::Unavailable, None),
        entry(4, "Anime Kill Sounds", ImportStatus::Exact, Some(8)),
        entry(5, "Paradox: Neon", ImportStatus::Exact, None),
    ];
    let import = &mut st.profiles.import;
    import.name = TextEdit::new(&portable.profile.name);
    import.keep = entries
        .iter()
        .map(|entry| entry.status != ImportStatus::Unavailable)
        .collect();
    import.status = vec![RowStatus::Idle; entries.len()];
    import.entries = entries;
    import.portable = Some(portable);
    import.stage = Stage::Review;
    st.profiles.dialog = Dialog::Import;
}

/// Import pressed while lookups still run: one mod installed, one downloading, one queued
/// and two still being looked up (they queue as their lookups return).
pub fn scene_import_early(st: &mut State) {
    scene_import(st);
    let import = &mut st.profiles.import;
    import.stage = Stage::Installing;
    import.status = vec![
        RowStatus::Idle,
        RowStatus::Installed,
        RowStatus::Downloading,
        RowStatus::Idle,
        RowStatus::LookingUp,
        RowStatus::LookingUp,
    ];
    for index in [4, 5] {
        if let Some(portable) = import.portable.as_ref()
            && let Some(mut pending) = core::pending_import_entry(portable, index)
        {
            pending.label = import.entries[index].label.clone();
            import.entries[index] = pending;
        }
        import.keep[index] = true;
    }
}

/// The first import step with a profile file to pick.
pub fn scene_import_input(st: &mut State) {
    fixture_load(st);
    st.profiles.import.files = vec![
        (
            "Casual_skins.modprofile.json".to_string(),
            PathBuf::from("Casual_skins.modprofile.json"),
            "Downloads".to_string(),
        ),
        (
            "Comp_Loadout.modprofile.json".to_string(),
            PathBuf::from("Comp_Loadout.modprofile.json"),
            "Downloads".to_string(),
        ),
    ];
    st.profiles.dialog = Dialog::Import;
}

/// The new profile dialog over a list that just reported an apply.
pub fn scene_create(st: &mut State) {
    fixture_load(st);
    st.profiles.last = Some((
        "Applied Casual skins: 4 mods switched. 2 mods not installed.".to_string(),
        true,
    ));
    st.profiles.dialog = Dialog::Create;
}
