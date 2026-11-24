//! Packs: share the mods you run so friends can run the same ones (core: `deadlock_core::packs`).
//!
//! - Grimoire (MIT): a profile shares as an `mp1:` code or a `.modprofile.json` file; import
//!   looks each mod up on GameBanana, shows exact, upgraded or unavailable per row, skips NSFW
//!   on request, downloads, then saves a profile.
//! - Deadlock Mod Manager (behaviour only): a profile is uploaded to its server and shared by id;
//!   import installs in the shared load order and lands mods on.
//! - Sigil: one feature called Pack, in the same `mp1:` format so both readers understand it,
//!   with no server. Make a pack from every enabled mod (Installed > More > Make a pack) or from
//!   a selection (the selection bar's More), share it as a code, a `sigil://pack/` link or a
//!   file. A pasted code (Browse > Paste mods, Installed > More > Packs > Paste a code) or a
//!   link opens a preview before anything happens: each mod with its picture, size, NSFW tag
//!   and state. Install pack queues the downloads on the Downloads page through the normal
//!   one-press install (download queue, install checks, backups, safety gate), in pack order so
//!   the load order holds, with the picked files. Mods land switched off unless "Turn on after
//!   installing" is ticked; even then a mod the safety gate asks about waits off.
//!
//! Where it lives: Installed's More menu, "Lists and packs" group (Packs, Make a pack), and the
//! selection bar's More. The Packs list is a dialog, so it opens over Installed without moving
//! the page.

use std::path::PathBuf;

use deadlock_core::gamebanana;
use deadlock_core::packs::{self as core, Candidate, Facts, Missing, Pack, ReadPack, RowState, SavedPack};
use rune::{Color, El, FileDialogOptions, Key, KeyEvent, SurfaceFit, TextEdit, Tx, col, el, icon, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::model::Screen;
use crate::prefs::Mature;
use crate::screens::settings::i18n::fill;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, MONO, SMALL, SMALL_NUM, Ui};

#[path = "packs_de.rs"]
pub mod locales;

/// The UI text in the player's language: this feature's German table first, then Sigil's.
pub(crate) fn tr(text: &str) -> &str {
    use crate::screens::settings::i18n::{self, Lang};
    if i18n::current() == Lang::German
        && let Some(de) = locales::get(text)
    {
        return de;
    }
    i18n::tr(text)
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dialog {
    /// The saved packs.
    List,
    /// Name and note for a new pack (or "Update from current mods" when `update` is set).
    Create,
    /// Code, link and file for one pack.
    Share,
    /// Paste a code or open a file.
    Paste,
    /// What a received pack holds, before anything is installed.
    Preview,
    Rename(String),
    Delete(String),
}

/// Where a new pack's mods come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// Every enabled mod, in load order.
    Enabled,
    /// The slots picked on Installed, in load order.
    Selection(Vec<u16>),
}

/// The Create dialog.
#[derive(Default)]
pub struct Create {
    pub name: TextEdit,
    pub note: TextEdit,
    pub source: Option<Source>,
    /// The mods that go in, once read (file ids come from the install journal).
    pub candidates: Option<Vec<Candidate>>,
    pub job: crate::jobs::Job<Vec<Candidate>>,
    /// Set for Update from current mods: the saved pack to replace.
    pub update: Option<String>,
    pub error: Option<String>,
}

/// The Share dialog's pack.
pub struct Share {
    pub pack: Pack,
    pub code: Result<String, String>,
    pub link: Option<String>,
    pub saved: Option<Result<PathBuf, String>>,
}

/// A received pack under review.
pub struct Preview {
    pub read: ReadPack,
    /// The saved pack this came from, if any (a pack from the list is not saved again).
    pub slug: Option<String>,
    /// GameBanana facts per row, in pack order; `None` while the lookup runs.
    pub facts: Vec<Option<Result<Facts, String>>>,
    /// The next row to look up.
    pub next: usize,
    pub turn_on: bool,
    pub skip_nsfw: bool,
}

impl Preview {
    pub fn new(read: ReadPack, slug: Option<String>, skip_nsfw: bool) -> Preview {
        let rows = read.pack.mods.len();
        Preview { read, slug, facts: vec![None; rows], next: 0, turn_on: false, skip_nsfw }
    }

    /// Every lookup returned.
    pub fn checked(&self) -> bool {
        self.facts.iter().all(Option::is_some)
    }
}

#[derive(Default)]
pub struct View {
    pub dialog: Option<Dialog>,
    pub saved: Vec<SavedPack>,
    pub loaded: bool,
    pub list_job: crate::jobs::Job<Vec<SavedPack>>,
    pub create: Create,
    pub share: Option<Share>,
    pub preview: Option<Preview>,
    pub lookup: crate::jobs::Job<(usize, Result<Facts, String>)>,
    pub paste: TextEdit,
    pub paste_error: Option<String>,
    pub rename: TextEdit,
    /// A file write or read for Share or Paste.
    pub file_job: crate::jobs::Job<FileDone>,
    pub error: Option<String>,
}

pub enum FileDone {
    Saved(Result<PathBuf, String>),
    Opened(Result<ReadPack, String>),
}

// ---------------------------------------------------------------------------
// Opening
// ---------------------------------------------------------------------------

fn workspace() -> PathBuf {
    crate::data::workspace_dir()
}

fn quiet(st: &State) -> bool {
    st.demo || rune::is_headless()
}

/// Installed > More > Packs.
pub fn open_list(st: &mut State) {
    st.installed.menu = None;
    st.packs.dialog = Some(Dialog::List);
    st.packs.error = None;
    reload(st);
}

fn reload(st: &mut State) {
    if quiet(st) {
        st.packs.loaded = true;
        return;
    }
    let ws = workspace();
    let wake = st.wake.clone();
    st.packs.list_job.start(wake, move || Ok(core::list_packs(&ws)));
}

/// Installed > More > Make a pack (every enabled mod), or the selection bar's More (the picked
/// mods). Reads the mods' file ids on a worker, then asks for a name.
pub fn open_create(st: &mut State, source: Source, update: Option<String>) {
    st.installed.menu = None;
    let rows = rows_for(st, &source);
    let name = update
        .as_ref()
        .and_then(|slug| st.packs.saved.iter().find(|p| &p.slug == slug))
        .map(|p| (p.pack.name.clone(), p.pack.note.clone().unwrap_or_default()));
    st.packs.create = Create {
        name: TextEdit::new(name.as_ref().map(|n| n.0.clone()).unwrap_or_default()),
        note: TextEdit::new(name.map(|n| n.1).unwrap_or_default()),
        source: Some(source),
        update,
        ..Create::default()
    };
    st.packs.dialog = Some(Dialog::Create);
    if quiet(st) {
        st.packs.create.candidates = Some(rows.into_iter().map(|(_, c)| c).collect());
        return;
    }
    let ws = workspace();
    let wake = st.wake.clone();
    st.packs.create.job.start(wake, move || {
        let journal = deadlock_core::install::read_journal(&ws).unwrap_or_default();
        Ok(rows
            .into_iter()
            .map(|(slot, mut candidate)| {
                if candidate.mod_id.is_some() {
                    candidate.file_id = journal
                        .iter()
                        .rev()
                        .find(|entry| entry.slot == slot)
                        .and_then(|entry| entry.download_url.as_deref())
                        .and_then(file_id_of);
                }
                candidate
            })
            .collect())
    });
}

/// The GameBanana file id in a download link (`/dl/<id>` or `/mmdl/<id>`).
pub fn file_id_of(url: &str) -> Option<u64> {
    let rest = url.split_once("/dl/").or_else(|| url.split_once("/mmdl/"))?.1;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok().filter(|id| (1..=core::MAX_ID).contains(id))
}

/// The installed mods a pack would hold, in load order, with what Sigil knows of each.
fn rows_for(st: &State, source: &Source) -> Vec<(u16, Candidate)> {
    let mut addons: Vec<&crate::model::AddonRow> = st
        .addons
        .addons
        .iter()
        .filter(|addon| match source {
            Source::Enabled => addon.enabled,
            Source::Selection(slots) => slots.contains(&addon.slot),
        })
        .collect();
    addons.sort_by_key(|addon| addon.slot);
    addons
        .into_iter()
        .map(|addon| {
            let meta = crate::screens::installed::meta_for(st, addon);
            let mod_id = meta.and_then(|m| m.gamebanana_id).or_else(|| {
                st.browse_view.installs.owned.iter().find(|(_, slot)| **slot == addon.slot).map(|(id, _)| *id)
            });
            let label = meta.map(|m| m.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| addon.file.clone());
            (addon.slot, Candidate { label, mod_id, file_id: None, enabled: addon.enabled })
        })
        .collect()
}

/// Open the preview for pasted text: a code, a link or pack JSON. Reads nothing from disk and
/// installs nothing. Returns false when the text is not a pack (the caller says why).
pub fn open_text(st: &mut State, text: &str) -> Result<(), String> {
    let read = core::read_pack_text(text).map_err(|e| e.to_string())?;
    open_preview(st, read, None);
    Ok(())
}

/// A pack pasted somewhere else (Browse > Paste mods): the preview, or why not in the status.
pub fn open_pasted(st: &mut State, text: &str) {
    if let Err(error) = open_text(st, text) {
        st.status = fill(tr("Could not open the pack: {}"), &[&error]);
    }
}

/// `sigil://pack/<code>`: the link only ever opens the preview.
pub fn open_link(st: &mut State, link: &str) {
    match open_text(st, link) {
        Ok(()) => st.go(Screen::Installed),
        Err(error) => st.status = fill(tr("Could not open the pack: {}"), &[&error]),
    }
}

fn open_preview(st: &mut State, read: ReadPack, slug: Option<String>) {
    let skip = st.prefs.mature == Mature::Hide;
    st.packs.preview = Some(Preview::new(read, slug, skip));
    st.packs.dialog = Some(Dialog::Preview);
    st.packs.error = None;
    next_lookup(st);
}

/// Look the next row up on GameBanana (the only host a preview asks), one at a time.
fn next_lookup(st: &mut State) {
    if st.packs.lookup.is_running() {
        return;
    }
    let offline = quiet(st) || st.prefs.offline;
    let Some(preview) = st.packs.preview.as_mut() else { return };
    while preview.next < preview.facts.len() {
        let index = preview.next;
        preview.next += 1;
        let entry = preview.read.pack.mods[index];
        // An installed mod needs no lookup to say so, but its name and picture are welcome.
        if offline {
            preview.facts[index] = Some(Err(tr("Sigil is offline.").to_string()));
            continue;
        }
        let wake = st.wake.clone();
        st.packs.lookup.start(wake, move || {
            let facts = gamebanana::fetch_mod_profile(entry.mod_id)
                .map(|profile| core::facts_from_profile(&profile, entry.file_id))
                .map_err(|e| e.to_string());
            Ok((index, facts))
        });
        return;
    }
}

/// Take finished lookups, list loads and file work. Called every frame from `app::poll`.
pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.packs.list_job.try_take() {
        st.packs.loaded = true;
        match result {
            Ok(list) => st.packs.saved = list,
            Err(error) => st.packs.error = Some(error),
        }
        changed = true;
    }
    if let Some(result) = st.packs.create.job.try_take() {
        match result {
            Ok(candidates) => st.packs.create.candidates = Some(candidates),
            Err(error) => st.packs.create.error = Some(error),
        }
        changed = true;
    }
    if let Some(result) = st.packs.lookup.try_take() {
        if let (Ok((index, facts)), Some(preview)) = (result, st.packs.preview.as_mut())
            && index < preview.facts.len()
        {
            if let Ok(facts) = &facts
                && let Some(url) = facts.thumbnail.clone()
            {
                crate::data::queue_images(st, vec![url], false);
            }
            if let Some(preview) = st.packs.preview.as_mut() {
                preview.facts[index] = Some(facts);
            }
        }
        next_lookup(st);
        changed = true;
    }
    if let Some(result) = st.packs.file_job.try_take() {
        match result {
            Ok(FileDone::Saved(saved)) => {
                if let Some(share) = st.packs.share.as_mut() {
                    share.saved = Some(saved);
                }
            }
            Ok(FileDone::Opened(Ok(read))) => open_preview(st, read, None),
            Ok(FileDone::Opened(Err(error))) => st.packs.paste_error = Some(error),
            Err(error) => st.packs.error = Some(error),
        }
        changed = true;
    }
    changed
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

fn close(st: &mut State) {
    st.packs.dialog = None;
    st.packs.preview = None;
    st.packs.share = None;
    st.packs.paste_error = None;
}

/// Back to the list from a dialog opened there, or closed when nothing else was open.
fn back(st: &mut State) {
    if st.packs.loaded && !st.packs.saved.is_empty() {
        st.packs.dialog = Some(Dialog::List);
        st.packs.preview = None;
        st.packs.share = None;
    } else {
        close(st);
    }
}

fn create(st: &mut State) {
    let c = &st.packs.create;
    let Some(candidates) = c.candidates.clone() else { return };
    let made = core::from_candidates(&c.name.text, &c.note.text, &candidates);
    let (pack, _) = match made {
        Ok(made) => made,
        Err(error) => {
            st.packs.create.error = Some(error.to_string());
            return;
        }
    };
    let saved = if quiet(st) {
        Ok(None)
    } else {
        match &st.packs.create.update {
            Some(slug) => core::store(&workspace(), slug, &pack).map(Some),
            None => core::save_new(&workspace(), &pack, true).map(Some),
        }
    };
    match saved {
        Ok(_) => {
            reload(st);
            open_share(st, pack);
        }
        Err(error) => st.packs.create.error = Some(error.to_string()),
    }
}

fn open_share(st: &mut State, pack: Pack) {
    let code = core::to_code(&pack).map_err(|e| e.to_string());
    let link = code.as_ref().ok().map(|code| format!("{}{code}", core::LINK_PREFIX));
    st.packs.share = Some(Share { pack, code, link, saved: None });
    st.packs.dialog = Some(Dialog::Share);
}

fn save_file(st: &mut State, tx: &mut Tx) {
    let Some(share) = st.packs.share.as_ref() else { return };
    let pack = share.pack.clone();
    let options = FileDialogOptions {
        title: tr("Save pack").to_string(),
        directory: Some(downloads_dir()),
        file_name: Some(core::file_name(&pack.name)),
        ..FileDialogOptions::default()
    }
    .filter(tr("Sigil pack"), &["json"]);
    tx.save_file(options, move |st: &mut State, result, _tx| match result {
        Ok(Some(path)) => {
            let pack = pack.clone();
            let wake = st.wake.clone();
            st.packs.file_job.start(wake, move || Ok(FileDone::Saved(core::write_file(&path, &pack).map_err(|e| e.to_string()))));
        }
        Ok(None) => {}
        Err(error) => st.packs.error = Some(fill(tr("The file picker could not open: {}"), &[&error.to_string()])),
    });
}

fn open_file(_st: &mut State, tx: &mut Tx) {
    let options = FileDialogOptions {
        title: tr("Open a pack").to_string(),
        directory: Some(downloads_dir()),
        ..FileDialogOptions::default()
    }
    .filter(tr("Sigil pack"), &["json"]);
    tx.open_file(options, |st: &mut State, result, _tx| match result {
        Ok(Some(path)) => {
            let wake = st.wake.clone();
            st.packs.file_job.start(wake, move || Ok(FileDone::Opened(core::read_pack_file(&path).map_err(|e| e.to_string()))));
        }
        Ok(None) => {}
        Err(error) => st.packs.paste_error = Some(fill(tr("The file picker could not open: {}"), &[&error.to_string()])),
    });
}

fn downloads_dir() -> PathBuf {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" });
    home.map(|home| PathBuf::from(home).join("Downloads"))
        .filter(|path| path.is_dir())
        .unwrap_or_else(|| crate::data::workspace_dir().join("exports"))
}

fn rename(st: &mut State, slug: &str) {
    let Some(saved) = st.packs.saved.iter().find(|p| p.slug == slug).cloned() else { return };
    let name = match core::check_name(&st.packs.rename.text) {
        Ok(name) => name,
        Err(error) => {
            st.packs.error = Some(error.to_string());
            return;
        }
    };
    let pack = Pack { name, ..saved.pack };
    if !quiet(st)
        && let Err(error) = core::store(&workspace(), slug, &pack)
    {
        st.packs.error = Some(error.to_string());
        return;
    }
    st.packs.error = None;
    st.packs.dialog = Some(Dialog::List);
    reload(st);
}

fn delete(st: &mut State, slug: &str) {
    if !quiet(st)
        && let Err(error) = core::delete(&workspace(), slug)
    {
        st.packs.error = Some(error.to_string());
        return;
    }
    st.packs.saved.retain(|p| p.slug != slug);
    st.packs.dialog = Some(Dialog::List);
    reload(st);
}

/// Each row's state, from what is installed now and what GameBanana said.
pub fn states(s: &State, preview: &Preview) -> Vec<RowState> {
    preview
        .read
        .pack
        .mods
        .iter()
        .zip(&preview.facts)
        .map(|(entry, facts)| {
            let installed = s.browse_view.installs.owned.contains_key(&entry.mod_id)
                || crate::screens::browse::quick::owned_slot(s, entry.mod_id).is_some();
            core::row_state(installed, facts.as_ref())
        })
        .collect()
}

fn skipped(preview: &Preview) -> Vec<bool> {
    preview
        .facts
        .iter()
        .map(|facts| preview.skip_nsfw && facts.as_ref().and_then(|f| f.as_ref().ok()).is_some_and(|f| f.nsfw))
        .collect()
}

/// Install pack: the player's press is the approval. Each mod goes to the download queue in
/// pack order, through the same one-press install every Browse install uses.
pub fn install(st: &mut State) {
    let Some(preview) = st.packs.preview.as_ref() else { return };
    if !preview.checked() {
        return;
    }
    let order = core::install_order(&states(st, preview), &skipped(preview));
    let turn_on = preview.turn_on;
    let jobs: Vec<(u64, String, String, u64, bool, Option<String>)> = order
        .iter()
        .filter_map(|index| {
            let entry = preview.read.pack.mods[*index];
            let facts = preview.facts[*index].as_ref()?.as_ref().ok()?;
            let url = facts.download_url.clone()?;
            let file = facts.file_id?;
            let name = if facts.name.is_empty() { fill(tr("Mod {}"), &[&entry.mod_id.to_string()]) } else { facts.name.clone() };
            Some((entry.mod_id, name, url, file, turn_on && entry.enabled, facts.thumbnail.clone()))
        })
        .collect();
    let pack = preview.read.pack.clone();
    let from_list = preview.slug.is_some();
    if !from_list && !quiet(st) {
        // Kept in the Packs list as received, so it can be installed again or shared on; the
        // same pack received twice is kept once.
        let ws = workspace();
        if !core::list_packs(&ws).iter().any(|saved| saved.pack == pack) {
            let _ = core::save_new(&ws, &pack, false);
        }
    }
    let count = jobs.len();
    let ids: Vec<u64> = jobs.iter().map(|job| job.0).collect();
    for (id, name, url, file, land_on, thumbnail) in jobs {
        crate::screens::browse::quick::start_pack(st, id, &name, url, file, land_on, thumbnail);
    }
    crate::screens::browse::quick::set_pack_order(st, ids);
    close(st);
    reload(st);
    let detail = if turn_on {
        tr("Each mod turns on once it is installed, unless the safety check has a question.").to_string()
    } else {
        tr("They land switched off. Turn them on in Installed.").to_string()
    };
    if count == 0 {
        st.status = fill(tr("Nothing in {} needs installing."), &[&pack.name]);
        return;
    }
    crate::notify::push(
        st,
        crate::notify::Notice::info(fill(tr("Installing {} mods from {}"), &[&count.to_string(), &pack.name]))
            .detail(detail)
            .action(tr("Open Downloads"), |st: &mut State| st.go(Screen::Downloads)),
    );
}

// ---------------------------------------------------------------------------
// Views
// ---------------------------------------------------------------------------

pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    match s.packs.dialog.as_ref()? {
        Dialog::List => Some(list_layer(s, u)),
        Dialog::Create => Some(create_layer(s, u)),
        Dialog::Share => s.packs.share.as_ref().map(|share| share_layer(s, u, share)),
        Dialog::Paste => Some(paste_layer(s, u)),
        Dialog::Preview => s.packs.preview.as_ref().map(|preview| preview_layer(s, u, preview)),
        Dialog::Rename(slug) => Some(rename_layer(s, u, slug)),
        Dialog::Delete(slug) => Some(delete_layer(s, u, slug)),
    }
}

fn shell(u: &Ui, key: &str, title: &str, width: f32) -> El {
    u.dialog_shell()
        .key(key.to_string())
        .role("dialog")
        .label(title.to_string())
        .w(width)
        .max_w(width)
        .padding(24.0)
        .gap(16.0)
        .modal(true, close)
        .on_key(move |st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                close(st);
                true
            } else {
                false
            }
        })
}

fn title(u: &Ui, title: &str, line: &str) -> El {
    let mut block = col().w_full().gap(4.0).child(text(title.to_string()).font(H1).color(u.ink).truncate());
    if !line.is_empty() {
        block = block.child(text(line.to_string()).font(BODY).color(u.muted).wrap());
    }
    block
}

fn footer(actions: Vec<El>) -> El {
    let mut bar = row().w_full().gap(8.0).center_y().pt(4.0);
    for action in actions {
        bar = bar.child(action);
    }
    bar
}

fn button(u: &Ui, label: &str, kind: ActionKind, key: &str) -> El {
    u.action(label, kind).key(key.to_string()).cursor(rune::Cursor::Pointer)
}

fn error_line(u: &Ui, key: &str, error: &Option<String>) -> Option<El> {
    error.as_ref().map(|error| crate::notify::feedback::error_line(u, key, error))
}

fn field(u: &Ui, edit: &TextEdit, key: &str, label: &str, placeholder: &str, bind: fn(&mut State) -> &mut TextEdit) -> El {
    col().w_full().gap(6.0).child(text(label.to_string()).font(SMALL).color(u.muted)).child(
        row().w_full().h(32.0).px(10.0).center_y().radius(crate::ui::r(4.0)).fill(u.well).border(1.0, u.hair).child(
            rune::text_input(edit, BODY)
                .key(key.to_string())
                .label(label.to_string())
                .grow(1.0)
                .color(u.ink)
                .placeholder(placeholder, u.faint)
                .caret(u.accent, u.accent.alpha(0.22))
                .on_edit(bind),
        ),
    )
}

fn checkbox(u: &Ui, on: bool) -> El {
    let base = el().size(16.0, 16.0).radius(crate::ui::r(4.0)).center().a11y_hidden();
    if on {
        base.fill(u.ink).child(icon(&icons::CHECK, 12.0).color(u.bg).a11y_hidden())
    } else {
        base.border(1.0, u.hair_strong)
    }
}

fn check_row(u: &Ui, key: &str, label: &str, on: bool, press: impl Fn(&mut State) + 'static) -> El {
    row()
        .key(key.to_string())
        .h(28.0)
        .gap(8.0)
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

/// The selection bar's More item.
pub fn from_selection_label(count: usize) -> String {
    fill(tr("Make a pack from {} mods"), &[&count.to_string()])
}

fn mods_label(n: usize) -> String {
    if n == 1 { tr("1 mod").to_string() } else { fill(tr("{} mods"), &[&n.to_string()]) }
}

// --- list ------------------------------------------------------------------------------

fn list_layer(s: &State, u: &Ui) -> El {
    let v = &s.packs;
    let head = row()
        .w_full()
        .gap(8.0)
        .center_y()
        .child(col().grow(1.0).min_w(0.0).child(title(u, tr("Packs"), tr("Share the mods you run. Friends install the same set with one code."))));
    let actions = row()
        .w_full()
        .gap(8.0)
        .center_y()
        .child(
            button(u, tr("Make a pack"), ActionKind::Primary, "packs-new")
                .on_press(|st: &mut State| open_create(st, Source::Enabled, None)),
        )
        .child(
            button(u, tr("Paste a code"), ActionKind::Secondary, "packs-paste").on_press(|st: &mut State| {
                st.packs.paste = TextEdit::default();
                st.packs.paste_error = None;
                st.packs.dialog = Some(Dialog::Paste);
            }),
        )
        .child(button(u, tr("Open a file"), ActionKind::Secondary, "packs-open-file").on_press_tx(open_file));
    let mut list = col().key("packs-list").w_full().max_h(360.0).scroll(rune::ScrollSpec { stick_bottom: false });
    if !v.loaded {
        list = list.child(text(tr("Reading your packs")).font(BODY).color(u.muted).h(40.0));
    } else if v.saved.is_empty() {
        list = list.child(
            text(tr("No packs yet. Make one from your enabled mods, or paste a code a friend sent."))
                .font(BODY)
                .color(u.muted)
                .wrap()
                .py(12.0),
        );
    }
    for saved in &v.saved {
        list = list.child(pack_row(u, saved));
    }
    let mut body = shell(u, "packs", tr("Packs"), 640.0).child(head).child(actions).child(
        col().w_full().border_sides([1.0, 0.0, 0.0, 0.0], u.hair).child(u.scroll_area(list)),
    );
    if let Some(error) = error_line(u, "packs-error", &v.error) {
        body = body.child(error);
    }
    body.child(footer(vec![spacer(), button(u, tr("Close"), ActionKind::Ghost, "packs-close").on_press(close)]))
}

fn pack_row(u: &Ui, saved: &SavedPack) -> El {
    let slug = saved.slug.clone();
    let origin = if saved.mine { tr("Yours") } else { tr("Received") };
    let date = crate::screens::profiles::short_date(&saved.updated_at);
    let sub = format!("{} · {} · {}", mods_label(saved.pack.mods.len()), origin, date);
    let icon_action = |glyph: &'static rune::Icon, label: &str, key: String| {
        u.icon_button(glyph, label).key(key).size(28.0, 28.0).cursor(rune::Cursor::Pointer)
    };
    let (s1, s2, s3, s4, s5) = (slug.clone(), slug.clone(), slug.clone(), slug.clone(), slug.clone());
    let install_pack = saved.pack.clone();
    let share_pack = saved.pack.clone();
    let name = saved.pack.name.clone();
    row()
        .key(format!("packs-row-{slug}"))
        .w_full()
        .h(48.0)
        .gap(8.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(icon(&icons::PACKAGE, 16.0).color(u.muted).a11y_hidden())
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .pl(4.0)
                .child(text(saved.pack.name.clone()).font(BODY_MED).color(u.ink).truncate())
                .child(text(sub).font(SMALL_NUM).color(u.muted).truncate()),
        )
        .child(
            u.action(tr("Install"), ActionKind::Secondary)
                .key(format!("packs-install-{s1}"))
                .h(28.0)
                .label(fill(tr("Install {}"), &[&name]))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    let read = ReadPack { pack: install_pack.clone(), unsupported: 0, duplicates: 0 };
                    open_preview(st, read, Some(s1.clone()));
                }),
        )
        .child(icon_action(&icons::UPLOAD, tr("Share"), format!("packs-share-{s2}")).on_press(move |st: &mut State| open_share(st, share_pack.clone())))
        .child(
            icon_action(&icons::REFRESH_CW, tr("Update from current mods"), format!("packs-update-{s3}"))
                .on_press(move |st: &mut State| open_create(st, Source::Enabled, Some(s3.clone()))),
        )
        .child(icon_action(&icons::SQUARE_PEN, tr("Rename"), format!("packs-rename-{s4}")).on_press(move |st: &mut State| {
            let name = st.packs.saved.iter().find(|p| p.slug == s4).map(|p| p.pack.name.clone()).unwrap_or_default();
            st.packs.rename = TextEdit::new(name);
            st.packs.error = None;
            st.packs.dialog = Some(Dialog::Rename(s4.clone()));
        }))
        .child(
            icon_action(&icons::TRASH_2, tr("Delete"), format!("packs-delete-{s5}"))
                .on_press(move |st: &mut State| st.packs.dialog = Some(Dialog::Delete(s5.clone()))),
        )
}

// --- create ----------------------------------------------------------------------------

fn create_layer(s: &State, u: &Ui) -> El {
    let c = &s.packs.create;
    let updating = c.update.is_some();
    let heading = if updating { tr("Update pack") } else { tr("Make a pack") };
    let from = match &c.source {
        Some(Source::Selection(slots)) => fill(tr("From the {} mods you picked, in load order."), &[&slots.len().to_string()]),
        _ => tr("From every enabled mod, in load order.").to_string(),
    };
    let mut body = shell(u, "packs-create", heading, 520.0)
        .child(title(u, heading, &from))
        .child(field(u, &c.name, "packs-create-name", tr("Name"), tr("Friday stack"), |st| &mut st.packs.create.name))
        .child(field(u, &c.note, "packs-create-note", tr("Note (optional)"), tr("What it is for"), |st| &mut st.packs.create.note));
    let ready = match &c.candidates {
        None => {
            body = body.child(text(tr("Reading your mods")).font(BODY).color(u.muted));
            false
        }
        Some(candidates) => {
            let shared = candidates.iter().filter(|c| c.mod_id.is_some()).count();
            let local: Vec<&str> = candidates.iter().filter(|c| c.mod_id.is_none()).map(|c| c.label.as_str()).collect();
            body = body.child(text(fill(tr("{} go in the pack."), &[&mods_label(shared)])).font(BODY_MED).color(u.ink));
            if !local.is_empty() {
                let mut names = local.iter().take(6).copied().collect::<Vec<_>>().join(", ");
                if local.len() > 6 {
                    names.push_str(&fill(tr(" and {} more"), &[&(local.len() - 6).to_string()]));
                }
                body = body.child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .items_start()
                        .child(el().pt(2.0).child(icon(&icons::INFO, 14.0).color(u.muted).a11y_hidden()))
                        .child(
                            col()
                                .grow(1.0)
                                .min_w(0.0)
                                .gap(2.0)
                                .child(
                                    text(fill(tr("Not shareable: {} local files are left out. Only mods from GameBanana can go in a pack."), &[&local.len().to_string()]))
                                        .font(BODY)
                                        .color(u.prose)
                                        .wrap(),
                                )
                                .child(text(names).font(SMALL).color(u.faint).wrap()),
                        ),
                );
            }
            shared > 0 && shared <= core::MAX_MODS
        }
    };
    if let Some(error) = error_line(u, "packs-create-error", &c.error) {
        body = body.child(error);
    }
    let named = !c.name.text.trim().is_empty();
    body.child(footer(vec![
        spacer(),
        button(u, tr("Cancel"), ActionKind::Ghost, "packs-create-cancel").on_press(back),
        button(u, if updating { tr("Update pack") } else { tr("Make pack") }, ActionKind::Primary, "packs-create-save")
            .disabled(!(ready && named))
            .opacity(if ready && named { 1.0 } else { 0.45 })
            .on_press(create),
    ]))
}

// --- share -----------------------------------------------------------------------------

fn share_layer(s: &State, u: &Ui, share: &Share) -> El {
    let line = fill(tr("{}. Friends paste the code in Sigil (Installed, More, Packs) or open the link."), &[&mods_label(share.pack.mods.len())]);
    let mut body = shell(u, "packs-share", tr("Share pack"), 560.0).child(title(u, &share.pack.name, &line));
    match &share.code {
        Ok(code) => {
            body = body.child(
                col()
                    .key("packs-share-code")
                    .w_full()
                    .px(12.0)
                    .py(10.0)
                    .radius(crate::ui::r(4.0))
                    .fill(u.well)
                    .border(1.0, u.hair)
                    .child(text(code.clone()).font(MONO).color(u.muted).wrap_anywhere().max_lines(4)),
            );
            let code = code.clone();
            let link = share.link.clone().unwrap_or_default();
            body = body.child(
                row()
                    .w_full()
                    .gap(8.0)
                    .child(
                        button(u, tr("Copy code"), ActionKind::Primary, "packs-copy-code").on_press_tx(move |st: &mut State, tx: &mut Tx| {
                            tx.copy(code.clone());
                            st.status = tr("Pack code copied.").to_string();
                        }),
                    )
                    .child(
                        button(u, tr("Copy link"), ActionKind::Secondary, "packs-copy-link").on_press_tx(move |st: &mut State, tx: &mut Tx| {
                            tx.copy(link.clone());
                            st.status = tr("Pack link copied.").to_string();
                        }),
                    )
                    .child(button(u, tr("Save as file"), ActionKind::Secondary, "packs-save-file").on_press_tx(save_file)),
            );
        }
        Err(error) => body = body.child(crate::notify::feedback::error_line(u, "packs-share-error", error)),
    }
    match &share.saved {
        Some(Ok(path)) => {
            body = body.child(text(fill(tr("Saved to {}"), &[&path.display().to_string()])).font(SMALL).color(u.muted).truncate());
        }
        Some(Err(error)) => body = body.child(crate::notify::feedback::error_line(u, "packs-save-error", error)),
        None => {}
    }
    if let Some(error) = error_line(u, "packs-error", &s.packs.error) {
        body = body.child(error);
    }
    body.child(footer(vec![spacer(), button(u, tr("Done"), ActionKind::Ghost, "packs-share-done").on_press(back)]))
}

// --- paste -----------------------------------------------------------------------------

fn paste_layer(s: &State, u: &Ui) -> El {
    let v = &s.packs;
    let mut body = shell(u, "packs-paste", tr("Paste a pack"), 520.0)
        .child(title(u, tr("Paste a pack"), tr("Paste the code or link a friend sent. You see every mod before anything installs.")))
        .child(
            row().w_full().min_h(96.0).px(10.0).py(8.0).radius(crate::ui::r(4.0)).fill(u.well).border(1.0, u.hair).child(
                rune::text_input(&v.paste, MONO)
                    .key("packs-paste-input")
                    .label(tr("Pack code"))
                    .grow(1.0)
                    .color(u.ink)
                    .placeholder("mp1:H4sI...", u.faint)
                    .caret(u.accent, u.accent.alpha(0.22))
                    .on_edit(|st: &mut State| &mut st.packs.paste)
                    .autofocus(),
            ),
        );
    if let Some(error) = error_line(u, "packs-paste-error", &v.paste_error) {
        body = body.child(error);
    }
    let empty = v.paste.text.trim().is_empty();
    body.child(footer(vec![
        button(u, tr("Open a file"), ActionKind::Ghost, "packs-paste-file").on_press_tx(open_file),
        spacer(),
        button(u, tr("Cancel"), ActionKind::Ghost, "packs-paste-cancel").on_press(back),
        button(u, tr("Preview"), ActionKind::Primary, "packs-paste-preview")
            .disabled(empty)
            .opacity(if empty { 0.45 } else { 1.0 })
            .on_press(|st: &mut State| {
                let text = st.packs.paste.text.clone();
                if let Err(error) = open_text(st, &text) {
                    st.packs.paste_error = Some(error);
                }
            }),
    ]))
}

// --- preview ---------------------------------------------------------------------------

/// The art box of one row: the picture, blurred or left out for an NSFW mod per Settings.
fn art(s: &State, u: &Ui, facts: Option<&Facts>, key: String) -> El {
    let frame = el().size(56.0, 32.0).radius(crate::ui::r(4.0)).clip_rounded(crate::ui::r(4.0)).fill(u.well).border(1.0, u.hair);
    let nsfw = facts.is_some_and(|f| f.nsfw);
    let hide = nsfw && s.prefs.mature == Mature::Hide;
    match facts.and_then(|f| f.thumbnail.as_ref()).and_then(|url| s.thumbnails.get(url)).filter(|_| !hide) {
        Some(px) => {
            let image = el().key(key).pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).pixels_fit(px, SurfaceFit::Fill).a11y_hidden();
            let image = if nsfw && s.prefs.mature == Mature::Blur { image.blur(crate::screens::settings::nsfw::card_blur()) } else { image };
            frame.child(image).a11y_hidden()
        }
        None => frame.center().child(icon(&icons::PACKAGE, 14.0).color(u.faint).a11y_hidden()).a11y_hidden(),
    }
}

fn state_words(u: &Ui, state: &RowState, skipped: bool, upgraded: bool) -> (String, Color, String) {
    if skipped {
        return (tr("Skipped: NSFW").to_string(), u.faint, String::new());
    }
    match state {
        RowState::Checking => (tr("Checking").to_string(), u.faint, String::new()),
        RowState::Installed => (tr("Installed already").to_string(), u.muted, tr("Kept as it is: nothing downloads.").to_string()),
        RowState::WillInstall if upgraded => (tr("Will install").to_string(), u.teal, tr("The shared file is gone; the newest file installs instead.").to_string()),
        RowState::WillInstall => (tr("Will install").to_string(), u.teal, String::new()),
        RowState::NeedsReview => (
            tr("Needs review").to_string(),
            u.amber,
            tr("A script mod: the safety check shows what it holds before it can turn on.").to_string(),
        ),
        RowState::Missing(Missing::Removed) => (tr("Removed from GameBanana").to_string(), u.rose, String::new()),
        RowState::Missing(Missing::NoFiles) => (tr("No file to download").to_string(), u.rose, String::new()),
        RowState::Missing(Missing::Unreachable(why)) => (tr("Could not check").to_string(), u.amber, why.clone()),
    }
}

fn preview_layer(s: &State, u: &Ui, preview: &Preview) -> El {
    let pack = &preview.read.pack;
    let states = states(s, preview);
    let skip = skipped(preview);
    let installs = core::install_order(&states, &skip).len();
    let installed = states.iter().filter(|st| **st == RowState::Installed).count();
    let missing = states.iter().filter(|st| matches!(st, RowState::Missing(_))).count();
    let checked = preview.facts.iter().filter(|f| f.is_some()).count();
    let total = pack.mods.len();

    let mut summary = vec![mods_label(total)];
    if preview.checked() {
        summary.push(fill(tr("{} to install"), &[&installs.to_string()]));
        if installed > 0 {
            summary.push(fill(tr("{} installed already"), &[&installed.to_string()]));
        }
        if missing > 0 {
            summary.push(fill(tr("{} missing"), &[&missing.to_string()]));
        }
    } else {
        summary.push(fill(tr("checking {} of {}"), &[&checked.to_string(), &total.to_string()]));
    }
    let mut body = shell(u, "packs-preview", tr("Install pack"), 640.0).child(title(u, &fill(tr("Pack: {}"), &[&pack.name]), &summary.join(" · ")));
    if let Some(note) = &pack.note {
        body = body.child(text(note.clone()).font(BODY).color(u.prose).wrap().max_lines(4));
    }
    let mut left_out = Vec::new();
    if preview.read.unsupported > 0 {
        left_out.push(if preview.read.unsupported == 1 {
            tr("1 entry from another site is left out").to_string()
        } else {
            fill(tr("{} entries from other sites are left out"), &[&preview.read.unsupported.to_string()])
        });
    }
    if preview.read.duplicates > 0 {
        left_out.push(fill(tr("{} duplicates left out"), &[&preview.read.duplicates.to_string()]));
    }
    if !left_out.is_empty() {
        body = body.child(text(left_out.join(" · ")).font(SMALL).color(u.faint));
    }

    let mut list = col().key("packs-preview-rows").w_full().max_h(320.0).scroll(rune::ScrollSpec { stick_bottom: false });
    for (index, entry) in pack.mods.iter().enumerate() {
        let facts = preview.facts[index].as_ref().and_then(|f| f.as_ref().ok());
        let name = facts
            .map(|f| f.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| fill(tr("Mod {}"), &[&entry.mod_id.to_string()]));
        let mut sub = Vec::new();
        sub.push(format!("{}.", index + 1));
        if let Some(size) = facts.and_then(|f| f.size) {
            sub.push(crate::model::format_size(size));
        }
        if let Some(file) = facts.and_then(|f| f.file_name.clone()).filter(|f| !f.is_empty()) {
            sub.push(file);
        }
        if !entry.enabled {
            sub.push(tr("off in the pack").to_string());
        }
        let (word, color, help) = state_words(u, &states[index], skip[index], facts.is_some_and(|f| f.upgraded));
        let mut state = text(word).font(SMALL).color(color).align(rune::Align::End);
        if !help.is_empty() {
            state = state.help(help);
        }
        let nsfw = facts.is_some_and(|f| f.nsfw);
        list = list.child(
            row()
                .key(format!("packs-preview-row-{}", entry.mod_id))
                .w_full()
                .h(48.0)
                .gap(12.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(art(s, u, facts, format!("packs-art-{}", entry.mod_id)))
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .child(text(name).font(BODY_MED).color(u.ink).truncate())
                        .child(text(sub.join(" · ")).font(SMALL_NUM).color(u.muted).truncate()),
                )
                // The NSFW slot is kept on every row, so the state column never moves.
                .child(row().w(40.0).center().child(if nsfw {
                    text(tr("NSFW")).font(SMALL).color(u.rose)
                } else {
                    el()
                }))
                .child(el().w(176.0).child(state.w_full().truncate())),
        );
    }
    body = body.child(col().w_full().border_sides([1.0, 0.0, 0.0, 0.0], u.hair).child(u.scroll_area(list)));
    let turn_on = preview.turn_on;
    let skip_nsfw = preview.skip_nsfw;
    body = body.child(
        row()
            .w_full()
            .gap(24.0)
            .center_y()
            .child(check_row(u, "packs-turn-on", tr("Turn on after installing"), turn_on, |st: &mut State| {
                if let Some(p) = st.packs.preview.as_mut() {
                    p.turn_on = !p.turn_on;
                }
            }))
            .child(check_row(u, "packs-skip-nsfw", tr("Skip NSFW mods"), skip_nsfw, |st: &mut State| {
                if let Some(p) = st.packs.preview.as_mut() {
                    p.skip_nsfw = !p.skip_nsfw;
                }
            })),
    );
    body = body.child(
        text(tr("Every mod goes through the download queue and the same checks as any install, in the pack's load order."))
            .font(SMALL)
            .color(u.faint)
            .wrap(),
    );
    if let Some(error) = error_line(u, "packs-error", &s.packs.error) {
        body = body.child(error);
    }
    let can = preview.checked() && installs > 0;
    let label = if !preview.checked() {
        fill(tr("Checking {} of {}"), &[&checked.to_string(), &total.to_string()])
    } else if installs == 0 {
        tr("Install pack").to_string()
    } else {
        fill(tr("Install pack ({})"), &[&installs.to_string()])
    };
    body.child(footer(vec![
        spacer(),
        button(u, tr("Cancel"), ActionKind::Ghost, "packs-preview-cancel").on_press(back),
        button(u, &label, ActionKind::Primary, "packs-preview-install")
            .min_w(160.0)
            .disabled(!can)
            .opacity(if can { 1.0 } else { 0.45 })
            .on_press(install),
    ]))
}

// --- rename and delete -----------------------------------------------------------------

fn rename_layer(s: &State, u: &Ui, slug: &str) -> El {
    let key = slug.to_string();
    let mut body = shell(u, "packs-rename", tr("Rename pack"), 440.0)
        .child(title(u, tr("Rename pack"), ""))
        .child(field(u, &s.packs.rename, "packs-rename-name", tr("Name"), "", |st| &mut st.packs.rename));
    if let Some(error) = error_line(u, "packs-error", &s.packs.error) {
        body = body.child(error);
    }
    body.child(footer(vec![
        spacer(),
        button(u, tr("Cancel"), ActionKind::Ghost, "packs-rename-cancel").on_press(|st: &mut State| st.packs.dialog = Some(Dialog::List)),
        button(u, tr("Rename"), ActionKind::Primary, "packs-rename-save").on_press(move |st: &mut State| rename(st, &key)),
    ]))
}

fn delete_layer(s: &State, u: &Ui, slug: &str) -> El {
    let key = slug.to_string();
    let name = s.packs.saved.iter().find(|p| p.slug == slug).map(|p| p.pack.name.clone()).unwrap_or_default();
    shell(u, "packs-delete", tr("Delete pack"), 440.0)
        .child(title(u, &fill(tr("Delete {}?"), &[&name]), tr("Only the saved pack goes. Installed mods stay as they are.")))
        .child(footer(vec![
            spacer(),
            button(u, tr("Cancel"), ActionKind::Ghost, "packs-delete-cancel").on_press(|st: &mut State| st.packs.dialog = Some(Dialog::List)),
            button(u, tr("Delete"), ActionKind::Danger, "packs-delete-confirm").on_press(move |st: &mut State| delete(st, &key)),
        ]))
}

// ---------------------------------------------------------------------------
// Scenes
// ---------------------------------------------------------------------------

fn fixture_pack() -> Pack {
    Pack {
        name: "Friday stack".to_string(),
        note: Some("What we run for scrims: the clean HUD, quiet guns and the Haze set.".to_string()),
        mods: [(741_478, true), (741_608, true), (612_004, false), (598_310, true), (703_221, true), (688_190, true), (655_002, true)]
            .iter()
            .map(|(id, on)| core::PackMod { mod_id: *id, file_id: Some(id + 700_000), enabled: *on })
            .collect(),
    }
}

fn fixture_facts(name: &str, size: u64, nsfw: bool) -> Facts {
    Facts {
        name: name.to_string(),
        nsfw,
        thumbnail: None,
        file_id: Some(1),
        file_name: Some(format!("{}.zip", name.to_lowercase().replace(' ', "_"))),
        size: Some(size),
        download_url: Some("https://gamebanana.com/dl/1".to_string()),
        upgraded: false,
        missing: None,
        script_like: false,
    }
}

/// The preview of a received pack: installed, will install, NSFW, needs review, removed,
/// unreachable and one still checking.
pub fn scene_preview(st: &mut State) {
    st.screen = Screen::Installed;
    let read = ReadPack { pack: fixture_pack(), unsupported: 1, duplicates: 0 };
    let mut preview = Preview::new(read, None, false);
    let mut review = fixture_facts("Bebop hook trails", 3_400_000, false);
    review.script_like = true;
    let mut upgraded = fixture_facts("Quiet guns", 12_800_000, false);
    upgraded.upgraded = true;
    preview.facts = vec![
        Some(Ok(fixture_facts("Clean HUD", 2_100_000, false))),
        Some(Ok(upgraded)),
        Some(Ok(fixture_facts("Haze swimsuit", 48_200_000, true))),
        Some(Ok(review)),
        Some(Err("GameBanana returned HTTP 404 for the mod".to_string())),
        Some(Ok(fixture_facts("Vindicta scope", 6_000_000, false))),
        Some(Err("timeout".to_string())),
    ];
    preview.next = preview.facts.len();
    // The first mod is installed already (from the library store, as before a scan).
    st.installed.library.mods.push(deadlock_core::import::ModMeta {
        slot: 3,
        size_bytes: 1,
        name: "Clean HUD".to_string(),
        author: None,
        thumbnail_url: None,
        gamebanana_id: Some(741_478),
        source: "browse".to_string(),
        recorded_at: String::new(),
    });
    st.browse_view.installs.owned.insert(741_478, 3);
    st.packs.preview = Some(preview);
    st.packs.dialog = Some(Dialog::Preview);
}

/// The Packs list with yours and received packs.
pub fn scene_list(st: &mut State) {
    st.screen = Screen::Installed;
    let mut second = fixture_pack();
    second.name = "Lane night".to_string();
    second.mods.truncate(3);
    st.packs.saved = vec![
        SavedPack { slug: "friday-stack".into(), pack: fixture_pack(), mine: true, created_at: "2026-11-20T18:00:00Z".into(), updated_at: "2026-11-24T18:00:00Z".into() },
        SavedPack { slug: "lane-night".into(), pack: second, mine: false, created_at: "2026-11-21T18:00:00Z".into(), updated_at: "2026-11-21T18:00:00Z".into() },
    ];
    st.packs.loaded = true;
    st.packs.dialog = Some(Dialog::List);
}

/// The Share dialog right after Make a pack.
pub fn scene_share(st: &mut State) {
    st.screen = Screen::Installed;
    open_share(st, fixture_pack());
}

/// Make a pack from the enabled mods, two of them local files.
pub fn scene_create(st: &mut State) {
    st.screen = Screen::Installed;
    st.packs.create = Create {
        name: TextEdit::new("Friday stack".to_string()),
        source: Some(Source::Enabled),
        candidates: Some(vec![
            Candidate { label: "Clean HUD".into(), mod_id: Some(1), file_id: None, enabled: true },
            Candidate { label: "Quiet guns".into(), mod_id: Some(2), file_id: None, enabled: true },
            Candidate { label: "my_crosshair.vpk".into(), mod_id: None, file_id: None, enabled: true },
            Candidate { label: "pak07_dir.vpk".into(), mod_id: None, file_id: None, enabled: true },
        ]),
        ..Create::default()
    };
    st.packs.dialog = Some(Dialog::Create);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_ids_come_from_download_links() {
        assert_eq!(file_id_of("https://gamebanana.com/dl/1400001"), Some(1_400_001));
        assert_eq!(file_id_of("https://gamebanana.com/mmdl/12,Mod,5"), Some(12));
        assert_eq!(file_id_of("https://gamebanana.com/mods/5"), None);
        assert_eq!(file_id_of("https://gamebanana.com/dl/0"), None);
        assert_eq!(file_id_of("https://gamebanana.com/dl/99999999999"), None);
    }

    #[test]
    fn a_pack_round_trips_from_create_to_preview() {
        let candidates = vec![
            Candidate { label: "A".into(), mod_id: Some(10), file_id: Some(11), enabled: true },
            Candidate { label: "local.vpk".into(), mod_id: None, file_id: None, enabled: true },
            Candidate { label: "B".into(), mod_id: Some(20), file_id: None, enabled: false },
        ];
        let (pack, local) = core::from_candidates("Mine", "for scrims", &candidates).unwrap();
        assert_eq!(local, vec!["local.vpk".to_string()]);
        let code = core::to_code(&pack).unwrap();
        let link = format!("{}{code}", core::LINK_PREFIX);
        let read = core::read_pack_text(&link).unwrap();
        let preview = Preview::new(read, None, false);
        assert_eq!(preview.read.pack, pack);
        assert_eq!(preview.facts.len(), 2);
        assert!(!preview.checked());
    }

    #[test]
    fn every_string_has_german() {
        let source = include_str!("packs.rs");
        let code = source.split("#[cfg(test)]").next().unwrap();
        let mut missing = Vec::new();
        for (at, _) in code.match_indices("tr(\"") {
            let rest = &code[at + 4..];
            let end = rest.find("\")").or_else(|| rest.find('"')).unwrap();
            let literal = &rest[..end];
            if !literal.contains('"') && locales::get(literal).is_none() {
                missing.push(literal.to_string());
            }
        }
        // Strings Sigil's shared tables already carry.
        missing.retain(|m| !["Sigil is offline.", "Close", "Cancel", "Delete", "Install", "Share", "Rename", "Preview", "Done"].contains(&m.as_str()));
        assert!(missing.is_empty(), "untranslated: {missing:#?}");
        for (en, de) in locales::DE {
            assert_eq!(en.matches("{}").count(), de.matches("{}").count(), "{en}");
            assert!(!en.contains('\u{2014}') && !de.contains('\u{2014}'), "em dash in {en}");
        }
    }

    #[test]
    fn states_follow_install_and_lookup() {
        let mut st = State::default();
        st.demo = true;
        scene_preview(&mut st);
        let preview = st.packs.preview.as_ref().unwrap();
        let states = states(&st, preview);
        assert_eq!(states[0], RowState::Installed);
        assert_eq!(states[1], RowState::WillInstall);
        assert_eq!(states[3], RowState::NeedsReview);
        assert_eq!(states[4], RowState::Missing(Missing::Removed));
        assert_eq!(core::install_order(&states, &skipped(preview)), vec![1, 2, 3, 5]);
    }

    #[test]
    fn skip_nsfw_leaves_mature_mods_out() {
        let mut st = State::default();
        st.demo = true;
        scene_preview(&mut st);
        st.packs.preview.as_mut().unwrap().skip_nsfw = true;
        let preview = st.packs.preview.as_ref().unwrap();
        assert_eq!(core::install_order(&states(&st, preview), &skipped(preview)), vec![1, 3, 5]);
    }

    #[test]
    fn hostile_codes_never_open_a_preview() {
        let mut st = State::default();
        st.demo = true;
        for bad in ["", "mp1:!!", "sigil://pack/../../etc", "https://evil.example/pack", &"A".repeat(core::MAX_CODE_BYTES + 10)] {
            assert!(open_text(&mut st, bad).is_err(), "{bad}");
            assert!(st.packs.preview.is_none());
        }
    }
}
