//! Installed: the addon slots on disk as image cards or rows, grouped into Enabled and Disabled.
//!
//! Slots come from `deadlock_core::addons::read_conflict_report`. Names and previews come from
//! Sigil's library store (`deadlock_core::import`), written when a mod is installed from Browse
//! or imported; a slot without an entry shows a calm placeholder. The switch on a card is the
//! user's explicit action: it renames that one slot file in the addon folder.
//!
//! The page is split into modules beside this file: `notes` (hero, category, tag and list
//! filters, tags, the local mod editor), `layout` (the three looks and variant groups),
//! `search` (solo test, find a broken mod, the bulk recap) and `tools` (safety scan,
//! identification, compatibility review, merge, fonts). This file keeps the page, the cards,
//! the menus and the writes the older rounds built.

pub mod backups;
pub mod checks;
pub mod imprint;
pub mod safety_gate;
pub mod safety_review;
mod selection;
pub mod list_sections;
pub mod locales_ux;
pub mod sound;
mod drag;
pub mod enable;
pub mod folder_fix;
mod layout;
pub mod notes;
pub mod order;
pub mod pack_all;
mod overlap;
pub mod repair;
mod req_order;
mod search;
pub mod tools;
pub mod update_match;
pub mod wave_scenes;

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::PathBuf;

use deadlock_core::addons::AddonToggleResult;
use deadlock_core::import::{self, Library, ModMeta, SourceKind};
use deadlock_core::install::InstallJournalEntry;
use deadlock_core::load_order::{self, ReorderPlan};
use deadlock_core::mod_updates::{self, RemoteFile, UpdateCache, UpdateState};
use deadlock_core::trash::{self, TrashEntry};
use rune::{Align, Color, El, ScrollSpec, Side, SurfaceFit, TextEdit, Tx, col, el, icon, row, spacer, text};

use crate::app::State;
use crate::data::ThumbCrew;
use crate::icons;
use crate::jobs::Job;
use crate::model::{AddonRow, Screen};
use crate::screens::import as importer;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{ActionKind, BODY, BODY_MED, H1, H2, LABEL, MICRO, PAGE_TOP, PAGE_X, SMALL, Ui};

const GRID_GAP: f32 = 16.0;
/// An empty 24 point glyph, so unchecked menu rows keep the checked rows' text edge.
pub(super) static NO_ICON: rune::Icon = rune::Icon {
    name: "sigil-none",
    view: 24.0,
    body: "",
};
pub(super) const CARD_RADIUS: f32 = 12.0;

#[derive(Default)]
pub struct View {
    /// Names and previews per slot, read from the library store.
    pub library: Library,
    /// `(slot, size)` of the scan the library was last read for. `None` forces a reread.
    pub library_sig: Option<Vec<(u16, u64)>>,
    pub thumbs: ThumbCrew,
    /// URLs in the current thumbnail batch, and ones that failed (never asked again).
    pub batch: HashSet<String>,
    pub failed: HashSet<String>,
    /// Previews looked up for installs recorded without one: `(slot, size, url)` written back.
    pub preview_fill: crate::jobs::Job<Vec<(u16, u64, String)>>,
    /// GameBanana ids already looked up this run, found or not.
    pub preview_tried: HashSet<u64>,
    pub collapsed_enabled: bool,
    pub collapsed_disabled: bool,
    /// Cards, small cards or a list.
    pub layout: layout::Layout,
    /// The open variant picker, and the label being renamed in it.
    pub variants: layout::Variants,
    /// Hero, category, tag and list filters, tags, pins and the local mod editor.
    pub notes: notes::Notes,
    /// The running solo test.
    pub solo: Option<search::Solo>,
    /// The broken mod search.
    pub search: search::SearchFlow,
    /// The report of the last bulk action.
    pub recap: Option<search::Recap>,
    /// Safety scan, identification, compatibility, merge and fonts.
    pub tools: tools::Tools,
    /// Compatibility repairs: the dry run, the records Revert reads, the work (`repair.rs`).
    pub repairs: repair::Repairs,
    /// The card under the pointer, which shows its "more" button.
    pub hovered: Option<u16>,
    pub menu: Option<Menu>,
    pub import: importer::Flow,
    /// Which group the page shows.
    pub show: Show,
    pub source: Source,
    pub sort: Sort,
    /// A bulk switch waiting for its confirm: `Some(true)` turns every shown mod on.
    pub bulk: Option<bool>,
    /// Files are being dragged over the page.
    pub drop_hover: bool,
    /// Cards picked for a bulk action, by slot, and the last one picked (the shift-click
    /// anchor).
    pub selected: BTreeSet<u16>,
    pub anchor: Option<u16>,
    /// A delete, reorder or update waiting for its confirm.
    pub confirm: Option<Confirm>,
    /// "Don't ask again" is ticked in the open confirm.
    pub dont_ask: bool,
    pub trash: TrashView,
    /// Moving deleted mods to the trash runs on a worker.
    pub trash_job: Job<TrashDone>,
    pub updates: Updates,
    /// The last change that can be put back, for the Undo of a notice.
    pub last_undo: Option<Undo>,
    /// The hero, requirement and dependents questions a switch asks first.
    pub gates: enable::Gates,
    /// Slots whose model another enabled mod hides, from the last scan.
    pub hidden: BTreeSet<u16>,
    /// A card being dragged to a new load order.
    pub drag: Option<drag::Dragging>,
    /// Enabled slots that share game paths with other enabled mods, from the last scan.
    pub overlaps: overlap::Overlaps,
    /// Mods that load after a mod their GameBanana page requires, so it hides them.
    pub order_problems: Vec<deadlock_core::requirements::OrderProblem>,
    /// Update all's scope (DMM): every mod with an update, only the ones on, or only the off ones.
    pub update_scope: UpdateScope,
    /// Sound samples for sound mods' Play buttons (`sound.rs`).
    pub sounds: sound::Sounds,
    /// View > Sections by list (DMM's groups as sections).
    pub by_list: bool,
    /// Card style options (View): hide the details line, hide the slot chip.
    pub hide_details: bool,
    pub hide_slot: bool,
    /// View imprint (Grimoire's card menu item).
    pub imprint: imprint::Imprint,
    /// Repair the mods folder, and the copies of mods installed twice.
    pub folder_fix: folder_fix::FolderFix,
    /// Analyze addons and the HUD check (C11).
    pub checks: checks::Checks,
    /// The open Backups dialog of one slot (C10).
    pub backups: Option<backups::Dialog>,
    /// Game paths of the mods in an order problem, read on a worker (C6).
    pub order_files: req_order::Files,
    /// The open load order dialog.
    pub order: Option<order::OrderDialog>,
    /// `last_undo` was just set: the next poll turns the status line into a toast with Undo.
    pub undo_fresh: bool,
}

/// Sigil's trash of deleted mods.
#[derive(Default)]
pub struct TrashView {
    pub open: bool,
    pub entries: Vec<TrashEntry>,
    /// "Empty trash" was pressed once and asks again.
    pub confirm_empty: bool,
}

/// Update checks and the update queue.
#[derive(Default)]
pub struct Updates {
    pub cache: UpdateCache,
    pub journal: Vec<InstallJournalEntry>,
    /// Repaired files and the installed file each one replaced (C3), so a repaired mod keeps
    /// its update check.
    pub aliases: Vec<deadlock_core::compat_repair::RepairAlias>,
    /// The cache and the install journal were read from disk.
    pub loaded: bool,
    /// The GameBanana ids of the last check this session (or of a fresh cache).
    pub checked: Option<Vec<u64>>,
    pub check: Job<(UpdateCache, Vec<InstallJournalEntry>)>,
    /// Slots waiting to update, and the one updating now.
    pub queue: Vec<u16>,
    pub running: Option<u16>,
    pub job: Job<UpdateDone>,
    /// Slots queued as a reinstall of the same file, not an update (A4).
    pub reinstall: BTreeSet<u16>,
    /// Reinstalls that clear the slot first ("Delete and reinstall", DMM's nuke): the worker
    /// moves the slot's files to the trash, then installs into the empty slot.
    pub nuke: BTreeSet<u16>,
    /// Cancel for the running update's download and extract (A5); a fresh flag per update.
    pub cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub failed: BTreeMap<u16, String>,
    /// What a queued slot installs from, when it is not the mod's newest file: a file link for
    /// a matched variant, or a downloaded archive for a Replace from Browse.
    pub targets: BTreeMap<u16, String>,
    /// GameBanana changelogs for the update confirm, by mod id.
    pub changelog: BTreeMap<u64, Result<Vec<deadlock_core::gamebanana::ModUpdate>, String>>,
    pub changelog_job: Job<Vec<(u64, Result<Vec<deadlock_core::gamebanana::ModUpdate>, String>)>>,
}

/// What the trash worker made: the entries and the slots that failed.
pub type TrashDone = (Vec<TrashEntry>, Vec<(u16, String)>);

/// Which mods Update all takes (DMM's Only enabled / Only disabled).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UpdateScope {
    #[default]
    All,
    Enabled,
    Disabled,
}

impl UpdateScope {
    pub fn keeps(self, enabled: bool) -> bool {
        match self {
            UpdateScope::All => true,
            UpdateScope::Enabled => enabled,
            UpdateScope::Disabled => !enabled,
        }
    }
}

/// The Update all slots the scope keeps.
fn scoped(s: &State, slots: &[u16]) -> Vec<u16> {
    let scope = s.installed.update_scope;
    slots
        .iter()
        .copied()
        .filter(|slot| s.addons.addons.iter().find(|a| a.slot == *slot).is_some_and(|a| scope.keeps(a.enabled)))
        .collect()
}

/// When the installed file of a mod was archived on GameBanana: the mod's current files, to
/// pick a replacement from (DMM). `None` when the file is not archived or nothing replaces it.
pub fn archived_choices(s: &State, addon: &AddonRow) -> Option<Vec<RemoteFile>> {
    let meta = meta_for(s, addon)?;
    let id = meta.gamebanana_id?;
    let updates = &s.installed.updates;
    let remote = updates.cache.mods.get(&id)?;
    let base = mod_updates::baseline_for_repaired(&updates.journal, &updates.aliases, id, addon.size_bytes, None, &meta.recorded_at);
    let file_id = base.file_id?;
    let archived = remote.files.iter().any(|f| f.file_id == file_id && f.archived);
    let current: Vec<RemoteFile> = remote.files.iter().filter(|f| !f.archived).cloned().collect();
    (archived && !current.is_empty()).then_some(current)
}

/// Install `file` into the slot in place of the archived one, through the update queue.
fn replace_archived(st: &mut State, slot: u16, file_id: u64) {
    st.installed.menu = None;
    st.installed.updates.targets.insert(slot, format!("https://gamebanana.com/dl/{file_id}"));
    queue_updates(st, vec![slot]);
}

/// One finished update, from the worker.
pub struct UpdateDone {
    pub slot: u16,
    pub name: String,
    pub backups: Vec<PathBuf>,
    pub old_meta: Option<ModMeta>,
    pub journal: Vec<InstallJournalEntry>,
}

/// A change that asks first, unless the user ticked "Don't ask again".
#[derive(Clone, Debug)]
pub enum Confirm {
    Delete(Vec<u16>),
    Reorder(ReorderPlan),
    UpdateAll(Vec<u16>),
}

impl Confirm {
    fn id(&self) -> &'static str {
        match self {
            Confirm::Delete(_) => SKIP_DELETE,
            Confirm::Reorder(_) => SKIP_REORDER,
            Confirm::UpdateAll(_) => SKIP_UPDATE_ALL,
        }
    }
}

const SKIP_DELETE: &str = "installed.delete";
const SKIP_REORDER: &str = "installed.reorder";
const SKIP_UPDATE_ALL: &str = "installed.update_all";
const SKIP_BULK: &str = "installed.bulk";

/// What it takes to put a change back. Each arm maps to one core call: see [`apply_undo`].
#[derive(Clone, Debug)]
pub enum Undo {
    /// Trash entries to restore (`deadlock_core::trash::restore`).
    Delete(Vec<PathBuf>),
    /// Switches to rename back (`deadlock_core::addons::revert_toggles`).
    Toggle(Vec<AddonToggleResult>),
    /// The applied plan; its reverse runs (`load_order::reverse_plan`, `apply_reorder`).
    Reorder(ReorderPlan),
    /// The install backups of an update, and the library entry from before it
    /// (`deadlock_core::trash::undo_update`).
    Update {
        slot: u16,
        backups: Vec<PathBuf>,
        meta: Option<ModMeta>,
    },
    /// A merge: its sources go back on and its slot to the trash
    /// (`deadlock_core::mod_merge::unmerge`).
    Unmerge(Box<deadlock_core::mod_merge::MergedMod>),
    /// Fonts Sigil added for a slot (`deadlock_core::fonts::remove_fonts`).
    FontsInstalled(u16),
    /// Fonts Sigil removed from a slot (`deadlock_core::fonts::install_fonts`).
    FontsRemoved(u16),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Show {
    #[default]
    All,
    Enabled,
    Disabled,
}

/// Where a mod came from, as the library store records it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Source {
    #[default]
    Any,
    GameBanana,
    Local,
    Imported,
    Unknown,
}

impl Source {
    const ALL: [Source; 5] = [
        Source::Any,
        Source::GameBanana,
        Source::Local,
        Source::Imported,
        Source::Unknown,
    ];
    fn label(self) -> &'static str {
        match self {
            Source::Any => "Any source",
            Source::GameBanana => "GameBanana",
            Source::Local => "Local files",
            Source::Imported => "Other managers",
            Source::Unknown => "Not recognised",
        }
    }
    fn matches(self, meta: Option<&ModMeta>) -> bool {
        match self {
            Source::Any => true,
            Source::GameBanana => meta.is_some_and(|m| m.gamebanana_id.is_some()),
            Source::Local => meta.is_some_and(|m| m.source == "local"),
            Source::Imported => meta.is_some_and(|m| m.source == "grimoire" || m.source == "dmm"),
            Source::Unknown => meta.is_none(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    /// Slot order: the order the game loads them, lower wins.
    #[default]
    LoadOrder,
    Name,
    Recent,
    Size,
    /// By hero, A to Z, then name (A3); mods without a hero last.
    Hero,
}

impl Sort {
    const ALL: [Sort; 5] = [Sort::LoadOrder, Sort::Name, Sort::Hero, Sort::Recent, Sort::Size];
    fn label(self) -> &'static str {
        match self {
            Sort::LoadOrder => "Load order",
            Sort::Name => "Name",
            Sort::Recent => "Recently added",
            Sort::Size => "Largest first",
            Sort::Hero => "Hero",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Menu {
    /// The "more" menu of one card.
    Card(u16),
    /// The Import dropdown in the toolbar: local files and other managers.
    Import,
    /// The "Import from another mod manager" dropdown in the empty state.
    Managers,
    Source,
    Sort,
    /// The look of the page.
    View,
    /// The tools of one card: safety scan, fonts, identification, compatibility.
    Tools(u16),
    /// The less used actions of one card, behind "More actions".
    CardMore(u16),
    Hero,
    Category,
    Tag,
    List,
    /// Bulk actions over the shown mods.
    More,
    /// Who one card overlaps, from its overlap badge.
    Overlap(u16),
    /// Why one card loads too late for the mod it needs, and Fix it.
    Order(u16),
    /// The selection bar's More: group as one mod, add to a Locker category.
    Selection,
}

/// The mods the page shows: search, source and group applied, in the chosen order.
fn shown_addons(s: &State) -> Vec<&AddonRow> {
    let query = s.installed_query.text.trim().to_lowercase();
    let v = &s.installed;
    let mut shown: Vec<&AddonRow> = s
        .addons
        .addons
        .iter()
        .filter(|addon| matches_query(s, addon, &query))
        .filter(|addon| v.source.matches(meta_for(s, addon)))
        .filter(|addon| notes::matches(s, addon))
        .filter(|addon| match v.show {
            Show::All => true,
            Show::Enabled => addon.enabled,
            Show::Disabled => !addon.enabled,
        })
        .collect();
    match v.sort {
        Sort::LoadOrder => shown.sort_by_key(|addon| addon.slot),
        Sort::Name => shown.sort_by_key(|addon| display_name(s, addon).to_lowercase()),
        Sort::Recent => shown.sort_by(|a, b| {
            let at = |addon: &AddonRow| {
                meta_for(s, addon)
                    .map(|meta| meta.recorded_at.clone())
                    .unwrap_or_default()
            };
            at(b).cmp(&at(a)).then(a.slot.cmp(&b.slot))
        }),
        Sort::Size => shown.sort_by_key(|addon| std::cmp::Reverse(addon.size_bytes)),
        Sort::Hero => shown.sort_by_key(|addon| {
            let hero = notes::extra(s, addon).hero.map(|h| h.to_lowercase());
            (hero.is_none(), hero.unwrap_or_default(), display_name(s, addon).to_lowercase())
        }),
    }
    // Pinned mods sit at the top of the disabled group, in the chosen order within it. Saved
    // mods (Grimoire #593) sort first within each group.
    shown.sort_by_key(|addon| {
        let extra = notes::extra(s, addon);
        let group = if addon.enabled {
            0
        } else if extra.pinned {
            1
        } else {
            2
        };
        (group, !extra.favorite)
    });
    shown
}

pub(crate) fn display_name(s: &State, addon: &AddonRow) -> String {
    meta_for(s, addon)
        .map(|meta| meta.name.clone())
        .unwrap_or_else(|| slot_file_name(addon))
}

pub fn view(s: &State, u: &Ui) -> El {
    let shown = shown_addons(s);
    let cards = layout::groups(s, &shown);
    let enabled: Vec<&layout::Group> = cards.iter().filter(|card| card.enabled()).collect();
    let disabled: Vec<&layout::Group> = cards.iter().filter(|card| !card.enabled()).collect();
    let has_mods = s.game_detected && s.addons.scanned && !s.addons.addons.is_empty();

    let body = if !s.game_detected {
        u.empty_state(
            "",
            "Deadlock was not found",
            "Set the game folder in Settings and your mods show up here.",
            Some(
                u.action("Open Settings", ActionKind::Secondary)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| st.go(Screen::Settings)),
            ),
        )
    } else if let Some(error) = &s.addons.error {
        u.empty_state(
            "",
            "The addon folder could not be read",
            error,
            Some(crate::notify::feedback::button(u, "installed-error-feedback", "The addon folder could not be read", error)),
        )
    } else if !s.addons.scanned {
        loading(u)
    } else if s.addons.addons.is_empty() {
        empty(u)
    } else if shown.is_empty() {
        u.empty_state(
            "",
            "Nothing matches",
            "Try a mod name, an author or a slot number.",
            Some(
                u.action("Clear filters", ActionKind::Secondary)
                    .key("installed-clear")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| {
                        st.installed_query = rune::TextEdit::new("");
                        st.installed.show = Show::All;
                        st.installed.source = Source::Any;
                        notes::clear(st);
                    }),
            ),
        )
    } else if s.installed.by_list && !s.installed.notes.extras.lists.is_empty() {
        list_sections::body(s, u, &shown, |members| {
            let groups = layout::groups(s, members);
            let refs: Vec<&layout::Group> = groups.iter().collect();
            cards_block(s, u, &refs)
        })
    } else {
        let mut list = col().w_full().gap(24.0).pb(32.0);
        if !enabled.is_empty() {
            list = list.child(section(s, u, Section::Enabled, &enabled));
        }
        if !disabled.is_empty() {
            list = list.child(section(s, u, Section::Disabled, &disabled));
        }
        list
    };

    let trailing = if has_mods { Some(toolbar(s, u)) } else { None };

    let mut page = col()
        .key("installed-page")
        .w_full()
        .px(PAGE_X)
        .pt(PAGE_TOP)
        .gap(16.0)
        .child(u.page_head("", "Installed", "", trailing));
    if has_mods {
        page = page.child(if s.installed.selected.is_empty() {
            filter_bar(s, u)
        } else {
            selection_bar(s, u)
        });
        if let Some(line) = notes::filter_row(s, u) {
            page = page.child(line);
        }
        if let Some(banner) = search::solo_banner(s, u) {
            page = page.child(banner);
        }
        if let Some(notice) = safety_review::notice(s, u) {
            page = page.child(notice);
        }
    }
    let scroll = col()
        .key("installed-scroll")
        .w_full()
        .h_full()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(page.child(body));

    // Files dropped anywhere on the page open the import review, which asks before writing.
    let mut root = col()
        .key("installed-drop")
        .w_full()
        .h_full()
        .min_h(0.0)
        .on_drop_files(|st: &mut State, files: &[PathBuf], _tx: &mut Tx| {
            st.installed.drop_hover = false;
            let mods: Vec<PathBuf> = files.iter().filter(|path| droppable(path)).cloned().collect();
            if mods.is_empty() {
                st.status = "Drop a .vpk, .zip, .7z or .rar file to import it.".to_string();
                return;
            }
            importer::open_local(st, mods);
        })
        .on_drop_hover(|st: &mut State, over: bool, _tx: &mut Tx| st.installed.drop_hover = over)
        // Ctrl+A (Cmd+A) selects every mod the page shows, as a file manager does; a text field
        // with focus keeps the chord for its own text.
        .on_key(|st: &mut State, ev: &rune::KeyEvent, _tx: &mut Tx| select_all_key(st, ev))
        .child(scroll);
    if s.installed.drop_hover {
        root = root.child(drop_veil(u));
    }
    root
}

/// Installed's Ctrl+A (every mod the filters show, A3) and Escape (clear). True when used. `app::on_key` can call it too, so the chord
/// works with nothing focused.
pub fn select_all_key(st: &mut State, ev: &rune::KeyEvent) -> bool {
    if st.screen != Screen::Installed || st.installed.menu.is_some() {
        return false;
    }
    match ev.key {
        rune::Key::Command('a') if !ev.mods.shift && !ev.mods.alt => {
            select_all_shown(st);
            true
        }
        rune::Key::Escape if !st.installed.selected.is_empty() => {
            st.installed.selected.clear();
            true
        }
        _ => false,
    }
}

/// Mod files the import review reads.
fn droppable(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ["vpk", "zip", "7z", "rar"].iter().any(|e| ext.eq_ignore_ascii_case(e)))
}

fn drop_veil(u: &Ui) -> El {
    col()
        .key("installed-drop-veil")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .padding(16.0)
        .child(
            col()
                .w_full()
                .h_full()
                .center()
                .gap(8.0)
                .radius(crate::ui::r(12.0))
                .fill(u.bg.alpha(0.85))
                .border(1.0, u.accent.alpha(0.6))
                .child(icon(&icons::DOWNLOAD, 24.0).color(u.ink).a11y_hidden())
                .child(text("Drop to import").font(H1).color(u.ink))
                .child(
                    text("You review the mods before anything is copied.")
                        .font(BODY)
                        .color(u.muted),
                ),
        )
}

// --- filters -----------------------------------------------------------------

fn filter_bar(s: &State, u: &Ui) -> El {
    let v = &s.installed;
    let on = s.addons.addons.iter().filter(|a| a.enabled).count();
    let off = s.addons.addons.len() - on;
    let mut seg = row()
        .key("installed-show")
        .h(32.0)
        .padding(2.0)
        .gap(2.0)
        .radius(crate::ui::r(8.0))
        .fill(u.well)
        .border(1.0, u.hair)
        .role("radiogroup");
    for (show, label) in [
        (Show::All, format!("All {}", s.addons.addons.len())),
        (Show::Enabled, format!("Enabled {on}")),
        (Show::Disabled, format!("Disabled {off}")),
    ] {
        let active = v.show == show;
        seg = seg.child(
            row()
                .key(format!("installed-show-{show:?}"))
                .h_full()
                .px(12.0)
                .center()
                .radius(crate::ui::r(6.0))
                .fill(if active { u.raised } else { Color::TRANSPARENT })
                .border(1.0, if active { u.hair_strong } else { Color::TRANSPARENT })
                .hover(if active { u.raised } else { u.hover })
                .focusable(true)
                .role("radio")
                .label(label.clone())
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| st.installed.show = show)
                .child(
                    text(label)
                        .font(LABEL)
                        .color(if active { u.ink } else { u.muted })
                        .a11y_hidden(),
                ),
        );
    }
    let source = dropdown_button(
        u,
        "installed-source",
        v.source.label(),
        None,
    )
    .on_press(|st: &mut State| open_menu(st, Menu::Source));
    let sort = dropdown_button(u, "installed-sort", v.sort.label(), None)
        .on_press(|st: &mut State| open_menu(st, Menu::Sort));
    let more = u
        .icon_button(&icons::ELLIPSIS, "Bulk actions")
        .key("installed-bulk")
        .border(1.0, u.hair_strong)
        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| open_menu(st, Menu::More));
    let view = dropdown_button(u, "installed-view", v.layout.label(), Some(&icons::LAYOUT_GRID))
        .on_press(|st: &mut State| open_menu(st, Menu::View));
    row()
        .key("installed-filters")
        .w_full()
        .gap(8.0)
        .center_y()
        .child(seg)
        .child(source)
        .child(spacer())
        .child(sort)
        .child(view)
        .children(order::open_button(s, u))
        .child(more)
}

// --- toolbar -----------------------------------------------------------------

fn toolbar(s: &State, u: &Ui) -> El {
    let mut actions = row().gap(8.0).center_y();
    if !s.addons.conflicts.is_empty() {
        let count = s.addons.conflicts.len();
        let label = format!("{count} overlap{}", if count == 1 { "" } else { "s" });
        actions = actions.child(
            row()
                .key("installed-overlaps")
                .h(32.0)
                .pl(10.0)
                .pr(12.0)
                .gap(6.0)
                .center_y()
                .radius(crate::ui::r(8.0))
                .border(1.0, u.amber.alpha(0.28))
                .fill(u.amber.alpha(0.06))
                .hover(u.amber.alpha(0.10))
                .focusable(true)
                .role("button")
                .label(label.clone())
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| st.go(Screen::Conflicts))
                .child(icon(&icons::TRIANGLE_ALERT, 14.0).color(u.amber).a11y_hidden())
                .child(text(label).font(BODY_MED).color(u.amber).a11y_hidden()),
        );
    }
    if let Some(button) = update_all_button(s, u) {
        actions = actions.child(button);
    }
    actions
        .child(
            dropdown_button(u, "installed-import", "Import", Some(&icons::DOWNLOAD))
                .on_press(|st: &mut State| open_menu(st, Menu::Import)),
        )
        .child(
            u.icon_button(&icons::FOLDER_OPEN, "Open the addon folder")
                .key("installed-open-folder")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| {
                    let folder = addons_folder(st);
                    reveal(&mut st.status, folder);
                }),
        )
        .child(
            u.icon_button(&icons::REFRESH_CW, "Rescan")
                .key("installed-rescan")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| {
                    if !st.demo {
                        st.installed.library_sig = None;
                        crate::data::start_addon_scan(st);
                    }
                }),
        )
}

/// A 32 point secondary button with an optional leading icon and a trailing chevron.
fn dropdown_button(u: &Ui, key: &str, label: &str, glyph: Option<&'static rune::Icon>) -> El {
    let mut b = row()
        .key(key)
        .h(32.0)
        .pl(if glyph.is_some() { 10.0 } else { 12.0 })
        .pr(10.0)
        .gap(6.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .fill(Color::WHITE.alpha(0.035))
        .border(1.0, u.hair_strong)
        .hover(u.hover)
        .press(Color::WHITE.alpha(0.08))
        .focusable(true)
        .role("button")
        .label(label)
        .cursor(rune::Cursor::Pointer);
    if let Some(glyph) = glyph {
        b = b.child(icon(glyph, 16.0).color(u.ink).a11y_hidden());
    }
    b.child(text(label.to_string()).font(BODY_MED).color(u.ink).a11y_hidden())
        .child(icon(&icons::CHEVRON_DOWN, 14.0).color(u.muted).a11y_hidden())
}

/// The mods a bulk switch changes: the selection when there is one, else every shown mod,
/// leaving out the ones already in the wanted state.
fn bulk_targets(s: &State, enable: bool) -> Vec<&AddonRow> {
    let base: Vec<&AddonRow> = if s.installed.selected.is_empty() {
        shown_addons(s)
    } else {
        s.addons
            .addons
            .iter()
            .filter(|addon| s.installed.selected.contains(&addon.slot))
            .collect()
    };
    base.into_iter().filter(|addon| addon.enabled != enable).collect()
}

/// Ask before a bulk switch, unless the user said not to.
fn ask_bulk(st: &mut State, enable: bool) {
    st.installed.menu = None;
    if bulk_targets(st, enable).is_empty() {
        return;
    }
    if st.prefs.skip_confirm.contains(SKIP_BULK) {
        apply_bulk(st, enable);
    } else {
        st.installed.dont_ask = false;
        st.installed.bulk = Some(enable);
    }
}

/// Turn the bulk targets on or off. Each one is the same single rename the card switch does
/// (`deadlock_core::addons::set_enabled`), reported once in a recap.
fn apply_bulk(st: &mut State, enable: bool) {
    st.installed.bulk = None;
    if st.demo {
        st.status = "Demo library: nothing was changed.".to_string();
        return;
    }
    if crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    let targets: Vec<(u16, String)> = bulk_targets(st, enable)
        .into_iter()
        .map(|addon| (addon.slot, display_name(st, addon)))
        .collect();
    switch_slots(st, targets, enable);
    st.installed.selected.clear();
}

/// Turn these slots on or off in one pass: the same single rename the card switch does
/// (`deadlock_core::addons::set_many`), reported once in a recap and put behind Undo. The
/// caller has already checked demo mode and a running game. Returns the slots that changed.
pub(super) fn switch_slots(st: &mut State, targets: Vec<(u16, String)>, enable: bool) -> Vec<u16> {
    let Some(game_root) = st.game_root.clone() else {
        st.status = "Set the game folder in Settings first.".to_string();
        return Vec::new();
    };
    let wanted: BTreeMap<u16, bool> = targets.iter().map(|(slot, _)| (*slot, enable)).collect();
    let names: BTreeMap<u16, String> = targets.into_iter().collect();
    let mut recap = search::Recap::new(if enable {
        format!("Turned on {} mods", wanted.len())
    } else {
        format!("Turned off {} mods", wanted.len())
    });
    let mut changed = Vec::new();
    match deadlock_core::addons::set_many(&game_root, &wanted, true) {
        Ok(report) => {
            changed = report.done.iter().map(|done| done.slot).collect();
            for done in &report.done {
                recap
                    .done
                    .push(names.get(&done.slot).cloned().unwrap_or_else(|| done.to.clone()));
            }
            for (slot, error) in &report.failed {
                recap.failed.push((
                    names.get(slot).cloned().unwrap_or_else(|| format!("Slot {}", deadlock_core::addons::slot_label(*slot))),
                    error.clone(),
                ));
            }
            if !report.done.is_empty() {
                st.installed.last_undo = Some(Undo::Toggle(report.done));
                st.installed.undo_fresh = true;
            }
        }
        Err(error) => recap.stopped = Some(error.to_string()),
    }
    st.status = format!(
        "Turned {} {} mods.",
        if enable { "on" } else { "off" },
        recap.done.len()
    );
    if recap.any() {
        st.installed.recap = Some(recap);
    }
    rescan(st);
    changed
}

pub(super) fn open_menu(st: &mut State, menu: Menu) {
    st.installed.menu = if st.installed.menu == Some(menu) {
        None
    } else {
        Some(menu)
    };
    if matches!(menu, Menu::Import | Menu::Managers) {
        importer::start_detect(st);
    }
    if menu == Menu::More && !st.demo {
        st.installed.trash.entries = trash::list_trash(&trash_dir());
    }
}

// --- empty state ---------------------------------------------------------------

fn empty(u: &Ui) -> El {
    col()
        .key("installed-empty")
        .w_full()
        .pt(96.0)
        .pb(64.0)
        .gap(8.0)
        .items_center()
        .child(
            el().size(48.0, 48.0)
                .radius(crate::ui::r(12.0))
                .border(1.0, u.hair_strong)
                .center()
                .mb(8.0)
                .child(icon(&icons::PACKAGE, 22.0).color(u.muted).a11y_hidden()),
        )
        .child(
            text("No mods installed")
                .font(H1)
                .color(u.ink)
                .align(rune::Align::Center),
        )
        .child(
            text("Find mods in Browse, or bring in the ones you already have.")
                .font(BODY)
                .color(u.muted)
                .align(rune::Align::Center),
        )
        .child(
            row()
                .mt(16.0)
                .gap(8.0)
                .center_y()
                .child(
                    u.icon_action(&icons::COMPASS, "Browse mods", ActionKind::Primary)
                        .key("installed-browse")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| st.go(Screen::Browse)),
                )
                .child(
                    u.icon_action(&icons::FILE, "Import local mods", ActionKind::Secondary)
                        .key("installed-import-local")
                        .cursor(rune::Cursor::Pointer)
                        .on_press_tx(|st: &mut State, tx: &mut Tx| importer::pick_local(st, tx)),
                )
                .child(
                    dropdown_button(u, "installed-import-managers", "Import from another manager", None)
                        .on_press(|st: &mut State| open_menu(st, Menu::Managers)),
                ),
        )
}

// --- groups and cards ----------------------------------------------------------

/// Which section of the page a card sits in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Enabled,
    Disabled,
}

/// One section: the head with its count, then the cards or rows.
fn section(s: &State, u: &Ui, which: Section, cards: &[&layout::Group]) -> El {
    let collapsed = match which {
        Section::Enabled => s.installed.collapsed_enabled,
        Section::Disabled => s.installed.collapsed_disabled,
    };
    let label = match which {
        Section::Enabled => "Enabled",
        Section::Disabled => "Disabled",
    };
    let count: usize = cards.iter().map(|card| card.addons.len()).sum();
    let head = row()
        .key(format!("group-{}", label.to_lowercase()))
        .h(28.0)
        .gap(6.0)
        .center_y()
        .focusable(true)
        .role("button")
        .checked(!collapsed)
        .label(format!("{label}, {count} mods"))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| match which {
            Section::Enabled => st.installed.collapsed_enabled = !st.installed.collapsed_enabled,
            Section::Disabled => st.installed.collapsed_disabled = !st.installed.collapsed_disabled,
        })
        .child(
            icon(
                if collapsed {
                    &icons::CHEVRON_RIGHT
                } else {
                    &icons::CHEVRON_DOWN
                },
                16.0,
            )
            .color(u.faint)
            .a11y_hidden(),
        )
        .child(
            text(format!("{label} ({count})"))
                .font(BODY_MED)
                .color(u.muted)
                .a11y_hidden(),
        );
    let c = col().w_full().gap(12.0).child(head);
    if collapsed {
        return c;
    }
    c.child(cards_block(s, u, cards))
}

/// A set of cards drawn the page's way: the grid, or the list rows.
fn cards_block(s: &State, u: &Ui, cards: &[&layout::Group]) -> El {
    let mut c = col().w_full();
    if s.installed.layout.is_cards() {
        let cols = s.installed.layout.columns();
        // One card and its gap, from the last layout, so a drag steps card by card.
        let pitch = cards
            .first()
            .and_then(|card| rune::memo::layout_size(&format!("addon-{:02}", card.primary().slot)))
            .map(|[w, h]| [w + GRID_GAP, h + GRID_GAP])
            .unwrap_or([260.0, 220.0]);
        let mut grid = el()
            .w_full()
            .equal_columns(cols)
            .gap(GRID_GAP);
        for card in cards {
            grid = grid.child(card_view(s, u, card, pitch, cols));
        }
        c = c.child(grid);
    } else {
        let mut list = col().w_full();
        for card in cards {
            list = list.child(layout::list_row(s, u, card));
        }
        c = c.child(list);
    }
    c
}

/// The library entry for a slot, if its file still matches.
pub fn meta_for<'a>(s: &'a State, addon: &AddonRow) -> Option<&'a ModMeta> {
    s.installed.library.find(addon.slot, addon.size_bytes)
}

/// One card: a 16:9 preview with the selection box, warning buttons and the "more" button,
/// the slot number and an update chip on it; the name, a line of detail and the switch below.
/// A variant group shows the file it is playing and how many it holds.
fn card_view(s: &State, u: &Ui, card: &layout::Group, pitch: [f32; 2], cols: usize) -> El {
    let addon = card.primary();
    let slot = addon.slot;
    let enable = !addon.enabled;
    let meta = meta_for(s, addon);
    let compact = s.installed.layout == layout::Layout::Compact;
    let inner = CARD_RADIUS - 1.0;
    let name = meta
        .map(|meta| meta.name.clone())
        .unwrap_or_else(|| "Unnamed mod".to_string());
    let mut detail = match meta.and_then(|meta| meta.author.clone()) {
        Some(author) => format!("by {author}"),
        None => format!("{}  ·  {}", slot_file_name(addon), addon.size_label()),
    };
    if card.is_variants() {
        let on = card.addons.iter().filter(|addon| addon.enabled).count();
        detail = if on > 1 {
            crate::screens::settings::i18n::fill(
                crate::screens::settings::i18n::tr("{} of {} files on"),
                &[&on.to_string(), &card.addons.len().to_string()],
            )
        } else {
            format!("{} files  ·  {}", card.addons.len(), notes::label_for(s, addon))
        };
    } else if let Some(hero) = notes::extra(s, addon).hero {
        detail = format!("{detail}  ·  {hero}");
    }
    // A3: when it was installed, from the install journal (or the library entry).
    if !compact && let Some(date) = install_date(s, addon) {
        detail = format!("{detail}  ·  {date}");
    }
    let menu_open = s.installed.menu == Some(Menu::Card(slot));
    let hovered = s.installed.hovered == Some(slot);
    let selecting = !s.installed.selected.is_empty();
    let selected = s.installed.selected.contains(&slot);
    let show_more = menu_open || hovered;

    // The preview: image, or a placeholder; dimmed while the mod is off.
    let preview_key = notes::preview_key(s, addon);
    let pixels = preview_key
        .as_ref()
        .and_then(|key| s.thumbnails.get(key));
    let base = el()
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .corners([inner, inner, 0.0, 0.0])
        .a11y_hidden();
    let image = match pixels {
        Some(pixels) => base
            .key(format!("installed-thumb-{slot:02}"))
            .pixels_fit(pixels, SurfaceFit::Fill),
        None => base
            .key(format!("installed-thumb-none-{slot:02}"))
            .fill(Color::WHITE.alpha(0.03))
            .center()
            .child(icon(&icons::PACKAGE, if compact { 18.0 } else { 22.0 }).color(u.faint.alpha(0.55))),
    };
    let chip = row()
        .h(20.0)
        .px(6.0)
        .center_y()
        .radius(crate::ui::r(6.0))
        .fill(Color::BLACK.alpha(0.6))
        .a11y_hidden()
        .child(
            text(deadlock_core::addons::slot_label(slot))
                .font(MICRO)
                .color(Color::WHITE.alpha(0.86)),
        );
    // The "more" button sits on the image's corner and shows on hover, so the name keeps
    // the footer's full width.
    let more = el()
        .key(format!("installed-more-{slot:02}"))
        .size(28.0, 28.0)
        .center()
        .radius(crate::ui::r(8.0))
        .fill(Color::BLACK.alpha(if menu_open { 0.78 } else { 0.6 }))
        .hover(Color::BLACK.alpha(0.78))
        .opacity(if show_more { 1.0 } else { 0.0 })
        .focusable(true)
        .role("button")
        .label(format!("More for {name}"))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| open_menu(st, Menu::Card(slot)))
        .child(icon(&icons::ELLIPSIS, 16.0).color(Color::WHITE.alpha(0.9)).a11y_hidden());
    // The selection box shows on hover, and on every card once one is picked.
    let pick_box = el()
        .key(format!("installed-pick-{slot:02}"))
        .size(28.0, 28.0)
        .center()
        .opacity(if hovered || selecting { 1.0 } else { 0.0 })
        .focusable(true)
        .role("checkbox")
        .checked(selected)
        .label(format!("Select {name}"))
        .cursor(rune::Cursor::Pointer)
        .on_press_tx(move |st: &mut State, tx: &mut Tx| pick(st, slot, tx.mods().shift))
        .child(check_box(u, selected, true));
    let mut media = el()
        .key(format!("installed-media-{slot:02}"))
        .w_full()
        .aspect_ratio(16.0 / 9.0)
        .child(image);
    if selecting {
        // While picking, a press anywhere on the preview picks the card.
        media = media
            .cursor(rune::Cursor::Pointer)
            .on_press_tx(move |st: &mut State, tx: &mut Tx| pick(st, slot, tx.mods().shift));
    } else if card.is_variants() {
        // A multi-file card opens its file list (or its page, as Settings chooses).
        let key = card.key.clone();
        media = media
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| layout::open_group(st, key.clone(), slot));
    }
    if !addon.enabled && pixels.is_some() {
        // Off mods read quieter: the preview sits under a dark wash.
        media = media.child(
            el().key(format!("installed-wash-{slot:02}"))
                .absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .corners([inner, inner, 0.0, 0.0])
                .fill(Color::BLACK.alpha(0.42))
                .a11y_hidden(),
        );
    }
    let mut bottom = row().w_full().gap(6.0).center_y();
    if !s.installed.hide_slot {
        bottom = bottom.child(chip);
    }
    if let Some(update) = update_chip(s, u, addon) {
        bottom = bottom.child(update);
    }
    if let Some(merged) = tools::merged_chip(s, u, slot) {
        bottom = bottom.child(merged);
    }
    if let Some(twice) = folder_fix::chip(s, u, slot) {
        bottom = bottom.child(twice);
    }
    let mut top = row().w_full().items_start().gap(4.0).child(pick_box);
    let dragging = s.installed.drag.is_some_and(|d| d.slot == slot);
    if drag::allowed(s) {
        top = top.child(drag::grip(u, slot, addon.enabled, pitch, cols, dragging, hovered));
    }
    top = top.child(spacer());
    for mark in card_marks(s, u, addon) {
        top = top.child(mark);
    }
    for warning in warning_buttons(s, u, addon) {
        top = top.child(warning);
    }
    let media = media.child(
        col()
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .padding(6.0)
            .child(top.child(el().w(6.0)).child(more))
            .child(spacer())
            .child(bottom.pl(2.0).pb(2.0)),
    );
    let members: Vec<u16> = card.addons.iter().map(|addon| addon.slot).collect();
    let chosen = slot;
    let is_variants = card.is_variants();
    let switch = u
        .toggle(addon.enabled)
        .key(format!("toggle-{slot:02}"))
        .label(format!(
            "{} {name}",
            if addon.enabled { "Turn off" } else { "Turn on" }
        ))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            if is_variants {
                layout::group_switch(st, &members, enable, chosen)
            } else {
                enable::switch(st, slot, enable)
            }
        });

    let mut text_col = col().grow(1.0).min_w(0.0).gap(2.0).child(
        text(name.clone())
            .font(if compact { BODY_MED } else { H2 })
            .color(if addon.enabled { u.ink } else { u.prose })
            .truncate(),
    );
    if !compact && !s.installed.hide_details {
        text_col = text_col.child(text(detail).font(SMALL).color(u.faint).truncate());
    }
    let mut footer = row()
        .w_full()
        .pl(12.0)
        .pr(10.0)
        .pt(if compact { 8.0 } else { 10.0 })
        .pb(if compact { 8.0 } else { 12.0 })
        .gap(6.0)
        .center_y()
        .child(text_col);
    if card.is_variants() {
        let key = card.key.clone();
        footer = footer.child(
            u.icon_button(&icons::LAYERS, "Choose the file")
                .key(format!("installed-variants-{slot:02}"))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| layout::open_group(st, key.clone(), slot)),
        );
    }
    if let Some(play) = sound::button(s, u, addon) {
        footer = footer.child(play);
    }
    footer = footer.child(el().w(2.0)).child(switch);

    let drop = drag::drop_mark(s, slot, addon.enabled).is_some();
    col()
        .key(format!("addon-{slot:02}"))
        .min_w(0.0)
        .padding(1.0)
        .radius(CARD_RADIUS)
        .opacity(if dragging { 0.55 } else { 1.0 })
        .fill(if selected { u.selected } else { u.well })
        .border(
            if drop { 2.0 } else { 1.0 },
            if drop || selected { u.accent.alpha(0.85) } else { Color::WHITE.alpha(0.07) },
        )
        .on_hover_change(move |st: &mut State, inside: bool, _tx: &mut Tx| {
            if inside {
                st.installed.hovered = Some(slot);
            } else if st.installed.hovered == Some(slot) {
                st.installed.hovered = None;
            }
        })
        .child(media)
        .child(footer)
}

/// The quiet marks on a preview corner: pinned, loads first, mature. Not buttons.
fn card_marks(s: &State, u: &Ui, addon: &AddonRow) -> Vec<El> {
    let extra = notes::extra(s, addon);
    let mut marks = Vec::new();
    for (glyph, label, on) in [
        (&icons::PIN, "Pinned", extra.pinned),
        (&icons::ARROW_UP, "Loads first", extra.always_first),
        (&icons::EYE, "Mature", extra.is_nsfw()),
    ] {
        if !on || marks.len() >= 2 {
            continue;
        }
        marks.push(
            el()
                .key(format!("installed-mark-{label}-{:02}", addon.slot))
                .size(28.0, 28.0)
                .center()
                .radius(crate::ui::r(8.0))
                .fill(Color::BLACK.alpha(0.6))
                .a11y_hidden()
                .child(icon(glyph, 14.0).color(u.ink.alpha(0.9)).a11y_hidden()),
        );
    }
    marks
}

/// The small warning buttons on a preview corner: the safety scan and the compatibility
/// review, when either has something to say about this mod.
fn warning_buttons(s: &State, u: &Ui, addon: &AddonRow) -> Vec<El> {
    let mut buttons = Vec::new();
    if let Some(badge) = enable::hidden_badge(s, u, addon.slot) {
        buttons.push(badge);
    }
    if let Some(badge) = req_order::badge(s, u, addon.slot, true) {
        buttons.push(badge);
    }
    if let Some(badge) = overlap::badge(s, u, addon.slot, true) {
        buttons.push(badge);
    }
    let scan = s
        .installed
        .tools
        .scans
        .get(&(addon.slot, addon.size_bytes))
        .filter(|report| report.error.is_none() && report.worst().is_some());
    if let Some(report) = scan {
        let worst = report.worst().unwrap_or(deadlock_core::mod_safety::Risk::Script);
        let slot = addon.slot;
        buttons.push(
            el()
                .key(format!("installed-safety-{:02}", addon.slot))
                .size(28.0, 28.0)
                .center()
                .radius(crate::ui::r(8.0))
                .fill(Color::BLACK.alpha(0.6))
                .hover(Color::BLACK.alpha(0.78))
                .focusable(true)
                .role("button")
                .label(format!(
                    "{} files inside",
                    report.findings.len() + report.more
                ))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| tools::open_scan(st, slot))
                .child(
                    icon(&icons::SHIELD, 14.0)
                        .color(if worst == deadlock_core::mod_safety::Risk::Program {
                            u.rose
                        } else {
                            u.amber
                        })
                        .a11y_hidden(),
                ),
        );
    }
    if s.installed
        .tools
        .health
        .as_ref()
        .is_some_and(|health| {
            health
                .reviews
                .iter()
                .any(|review| review.slot == addon.slot && !review.is_clear())
        })
    {
        buttons.push(
            el()
                .key(format!("installed-health-{:02}", addon.slot))
                .size(28.0, 28.0)
                .center()
                .radius(crate::ui::r(8.0))
                .fill(Color::BLACK.alpha(0.6))
                .hover(Color::BLACK.alpha(0.78))
                .focusable(true)
                .role("button")
                .label("Paths the game no longer has")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| tools::open_health(st))
                .child(icon(&icons::TRIANGLE_ALERT, 14.0).color(u.amber).a11y_hidden()),
        );
    }
    buttons
}

/// A 16 point check box. `on_image` draws it over a preview, with a dark fill.
pub(super) fn check_box(u: &Ui, on: bool, on_image: bool) -> El {
    let mut b = el()
        .size(18.0, 18.0)
        .center()
        .radius(5.0)
        .a11y_hidden();
    b = if on {
        b.fill(u.accent).border(1.0, u.accent)
    } else if on_image {
        b.fill(Color::BLACK.alpha(0.45)).border(1.5, Color::WHITE.alpha(0.8))
    } else {
        b.border(1.5, u.hair_strong)
    };
    if on {
        b = b.child(icon(&icons::CHECK, 14.0).color(u.accent_ink));
    }
    b
}

/// "Update", "Queued", "Updating" or "Update failed" on the preview, when it applies.
fn update_chip(s: &State, u: &Ui, addon: &AddonRow) -> Option<El> {
    let slot = addon.slot;
    let updates = &s.installed.updates;
    let cancellable = updates.running == Some(slot) || updates.queue.contains(&slot);
    let (label, glyph, live) = if updates.running == Some(slot) {
        (if update_cancelling(s) { tr("Cancelling") } else { tr("Updating") }, &icons::LOADER, false)
    } else if updates.queue.contains(&slot) {
        (tr("Queued"), &icons::HISTORY, false)
    } else if !s.shell.w4.update_badges {
        // Settings > Show mod update badges is off: no Update mark (a running update still shows).
        return None;
    } else if update_for(s, addon).is_some() {
        if updates.failed.contains_key(&slot) {
            ("Update failed", &icons::TRIANGLE_ALERT, true)
        } else {
            ("Update", &icons::ARROW_UP, true)
        }
    } else {
        return None;
    };
    let mut chip = row()
        .key(format!("installed-update-{slot:02}"))
        .h(20.0)
        .pl(5.0)
        .pr(7.0)
        .gap(4.0)
        .center_y()
        .radius(crate::ui::r(6.0))
        .fill(Color::BLACK.alpha(0.6))
        .label(format!("{label} {}", display_name(s, addon)))
        .child(icon(glyph, 12.0).color(Color::WHITE.alpha(0.9)).a11y_hidden())
        .child(text(label).font(MICRO).color(Color::WHITE.alpha(0.9)).a11y_hidden());
    if live {
        chip = chip
            .hover(Color::BLACK.alpha(0.78))
            .focusable(true)
            .role("button")
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| queue_updates(st, vec![slot]));
    } else if cancellable && !update_cancelling(s) {
        chip = chip
            .label(fill(tr("Cancel the update of {}"), &[&display_name(s, addon)]))
            .hover(Color::BLACK.alpha(0.78))
            .focusable(true)
            .role("button")
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| cancel_update(st, slot))
            .child(icon(&icons::X, 11.0).color(Color::WHITE.alpha(0.75)).a11y_hidden());
    }
    let _ = u;
    Some(chip)
}

/// A slot's file for another page's list: its name and variant label, and the slot with on or
/// off.
pub fn file_label(s: &State, slot: u16) -> (String, String) {
    let Some(addon) = s.addons.addons.iter().find(|addon| addon.slot == slot) else {
        return (format!("{slot:02}"), String::new());
    };
    let name = display_name(s, addon);
    let label = notes::label_for(s, addon);
    let state = crate::screens::settings::i18n::tr(if addon.enabled { "On" } else { "Off" });
    (
        if label.is_empty() || label == name { name } else { format!("{name}  ·  {label}") },
        format!("{slot:02}  ·  {state}"),
    )
}

/// The page's check box, for another page's dialog.
pub fn check_box_el(u: &Ui, on: bool) -> El {
    check_box(u, on, false)
}

pub(super) fn slot_file_name(addon: &AddonRow) -> String {
    addon
        .file
        .strip_suffix(deadlock_core::addons::DISABLED_SUFFIX)
        .unwrap_or(&addon.file)
        .to_string()
}

fn matches_query(s: &State, addon: &AddonRow, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let meta = meta_for(s, addon);
    addon.file.to_lowercase().contains(query)
        || addon.slot_label().contains(query)
        || meta.is_some_and(|meta| {
            meta.name.to_lowercase().contains(query)
                || meta
                    .author
                    .as_ref()
                    .is_some_and(|author| author.to_lowercase().contains(query))
        })
}

fn loading(u: &Ui) -> El {
    let mut grid = el().w_full().equal_columns(4).gap(GRID_GAP).pt(40.0);
    for _ in 0..8 {
        grid = grid.child(
            col()
                .min_w(0.0)
                .padding(1.0)
                .radius(CARD_RADIUS)
                .fill(u.well)
                .border(1.0, Color::WHITE.alpha(0.07))
                .child(
                    el().w_full()
                        .aspect_ratio(16.0 / 9.0)
                        .corners([CARD_RADIUS - 1.0, CARD_RADIUS - 1.0, 0.0, 0.0])
                        .fill(u.hover),
                )
                .child(
                    col()
                        .px(12.0)
                        .pt(14.0)
                        .pb(18.0)
                        .gap(10.0)
                        .child(el().h(10.0).w(120.0).radius(3.0).fill(u.hover))
                        .child(el().h(8.0).w(72.0).radius(3.0).fill(u.hover)),
                ),
        );
    }
    grid
}

// --- menus -----------------------------------------------------------------------

/// The open dropdown, mounted at the root so it draws over the page and escapes its scroll.
/// A long menu (the card's, once tools are in it) scrolls inside the room the window has.
pub fn overlay(s: &State, u: &Ui) -> Option<El> {
    if s.screen != Screen::Installed {
        return None;
    }
    let menu = s.installed.menu?;
    let (anchor, align, width) = match menu {
        Menu::Card(slot) | Menu::Tools(slot) | Menu::CardMore(slot) => {
            (format!("installed-more-{slot:02}"), Align::End, 232.0)
        }
        Menu::Import => ("installed-import".to_string(), Align::End, 288.0),
        Menu::Managers => ("installed-import-managers".to_string(), Align::Start, 288.0),
        Menu::Source => ("installed-source".to_string(), Align::Start, 224.0),
        Menu::Sort => ("installed-sort".to_string(), Align::End, 200.0),
        Menu::View => ("installed-view".to_string(), Align::End, 200.0),
        Menu::Hero => ("installed-hero".to_string(), Align::Start, 220.0),
        Menu::Category => ("installed-category".to_string(), Align::Start, 220.0),
        Menu::Tag => ("installed-tag".to_string(), Align::Start, 320.0),
        Menu::List => ("installed-list".to_string(), Align::Start, 240.0),
        Menu::More => ("installed-bulk".to_string(), Align::End, 260.0),
        Menu::Overlap(slot) => (format!("installed-overlap-{slot:02}"), Align::End, 340.0),
        Menu::Order(slot) => (format!("installed-order-{slot:02}"), Align::End, 340.0),
        Menu::Selection => ("installed-selection-more".to_string(), Align::End, 280.0),
    };
    let mut items: Vec<El> = Vec::new();

    match menu {
        Menu::Card(slot) => {
            let addon = s.addons.addons.iter().find(|addon| addon.slot == slot)?;
            let extra = notes::extra(s, addon);
            let path = addon.path(&PathBuf::from(s.addons_dir.clone().unwrap_or_default()));
            items.push(
                menu_item(u, "menu-reveal", Some(&icons::FOLDER_OPEN), "Show in folder", "", true)
                    .on_press(move |st: &mut State| {
                        st.installed.menu = None;
                        reveal(&mut st.status, path.clone());
                    }),
            );
            if let Some(id) = meta_for(s, addon).and_then(|meta| meta.gamebanana_id) {
                let url = format!("https://gamebanana.com/mods/{id}");
                items.push(
                    menu_item(u, "menu-page", Some(&icons::EXTERNAL_LINK), "Open on GameBanana", "", true)
                        .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                            st.installed.menu = None;
                            if let Err(error) = tx.open_url(&url) {
                                st.status = format!("The browser could not open {url}: {error}");
                            }
                        }),
                );
                // DMM's library author link: the creator's page in Browse. The creator id is
                // looked up from the mod's page, so rows saved before Sigil kept it work too.
                let author = meta_for(s, addon).and_then(|meta| meta.author.clone()).unwrap_or_default();
                let label = crate::screens::browse::creator_page::more_label(&author);
                items.push(
                    menu_item(u, "menu-creator", Some(&icons::USER), &label, "", true).on_press(move |st: &mut State| {
                        st.installed.menu = None;
                        crate::screens::browse::creator_page::open_for_mod(st, id);
                    }),
                );
            }
            if let Some(files) = archived_choices(s, addon) {
                // DMM: the installed file was archived; pick what replaces it.
                items.push(menu_rule(u));
                for file in files.into_iter().take(6) {
                    let file_id = file.file_id;
                    items.push(
                        menu_item(
                            u,
                            &format!("menu-replace-{file_id}"),
                            Some(&icons::ARROW_UP),
                            &fill(tr("Replace with {}"), &[&file.file_name]),
                            tr("the old file was archived"),
                            s.installed.updates.running != Some(slot),
                        )
                        .on_press(move |st: &mut State| replace_archived(st, slot, file_id)),
                    );
                }
            }
            if let Some(update) = update_for(s, addon) {
                if s.installed.updates.running != Some(slot) {
                    items.push(
                        menu_item(u, "menu-update", Some(&icons::ARROW_UP), "Update", "", true)
                            .on_press(move |st: &mut State| {
                                st.installed.menu = None;
                                queue_updates(st, vec![slot]);
                            }),
                    );
                }
                items.push(
                    menu_item(
                        u,
                        "menu-ignore-update",
                        Some(&icons::CIRCLE_X),
                        "Ignore this update",
                        "",
                        true,
                    )
                    .on_press(move |st: &mut State| {
                        st.installed.menu = None;
                        notes::ignore_update(st, slot, Some(update.file_id));
                    }),
                );
            } else if extra.ignored_update.is_some() {
                items.push(
                    menu_item(u, "menu-unignore", Some(&icons::ROTATE_CCW), "Show the update again", "", true)
                        .on_press(move |st: &mut State| {
                            st.installed.menu = None;
                            notes::unignore_update(st, slot);
                        }),
                );
            }
            // Load order inside the card's own group: lower numbers load first.
            let group = group_slots(s, addon.enabled);
            let at = group.iter().position(|&g| g == slot).unwrap_or(0);
            if group.len() > 1 {
                items.push(menu_rule(u));
                let moves: [(&str, &str, &'static rune::Icon, Option<usize>); 3] = [
                    ("menu-first", "Load first", &ARROW_UP_TO_LINE, (at > 0).then_some(0)),
                    ("menu-earlier", "Load earlier", &icons::ARROW_UP, at.checked_sub(1)),
                    (
                        "menu-later",
                        "Load later",
                        &ARROW_DOWN,
                        (at + 1 < group.len()).then_some(at + 1),
                    ),
                ];
                for (key, label, glyph, index) in moves {
                    let group = group.clone();
                    let item = menu_item(u, key, Some(glyph), label, "", index.is_some());
                    items.push(match index {
                        Some(index) => item.on_press(move |st: &mut State| {
                            st.installed.menu = None;
                            // Next to it, this is a swap of two numbers; to the top, the
                            // ones above each move down one.
                            let wanted = load_order::move_to_index(&group, slot, index);
                            ask_reorder(st, &group, &wanted);
                        }),
                        None => item,
                    });
                }
            }
            items.push(menu_rule(u));
            // Custom maps (experimental): the map's name, Launch map and Invite friends.
            if crate::screens::maps::on(s) && crate::screens::launch_extras::is_map(s, addon) {
                items.push(
                    menu_item(u, "menu-map-play", Some(&icons::PLAY), crate::screens::maps::HOW_TO_PLAY, "", true)
                        .on_press(move |st: &mut State| {
                            st.installed.menu = None;
                            crate::screens::maps::open_slot(st, slot);
                        }),
                );
            }
            if addon.enabled
                && st_on(s) > 1
            {
                items.push(
                    menu_item(u, "menu-solo", Some(&icons::POWER), "Test this mod alone", "", true)
                        .on_press(move |st: &mut State| {
                            st.installed.menu = None;
                            search::solo_start(st, slot);
                        }),
                );
            }
            // The hero this mod changes, in 3D, even while it is off.
            if !crate::viewer::dialog::heroes_of(s, slot).is_empty() {
                let name = display_name(s, addon);
                items.push(
                    menu_item(u, "menu-preview-3d", Some(&icons::BOX), tr(crate::viewer::dialog::LABEL), if addon.enabled { "" } else { tr("while off") }, true)
                        .on_press(move |st: &mut State| {
                            st.installed.menu = None;
                            crate::viewer::dialog::open(st, slot, &name);
                        }),
                );
            }
            // Grimoire #593: one saved flag shared with the Locker.
            let saved = extra.favorite;
            items.push(
                menu_item(
                    u,
                    "menu-favorite",
                    Some(&icons::HEART),
                    if saved { tr("Remove from saved") } else { tr("Save") },
                    "",
                    true,
                )
                .on_press(move |st: &mut State| {
                    st.installed.menu = None;
                    notes::toggle_favorite(st, slot);
                }),
            );
            let pinned = extra.pinned;
            items.push(
                menu_item(
                    u,
                    "menu-pin",
                    Some(if pinned { &icons::PIN_OFF } else { &icons::PIN }),
                    if pinned { "Unpin" } else { "Pin to the top" },
                    "",
                    true,
                )
                .on_press(move |st: &mut State| {
                    st.installed.menu = None;
                    notes::toggle_pin(st, slot);
                }),
            );
            items.push(menu_rule(u));
            items.push(
                menu_item(u, "menu-more-actions", Some(&icons::ELLIPSIS), "More actions", "", true)
                    .on_press(move |st: &mut State| {
                        st.installed.menu = Some(Menu::CardMore(slot));
                    }),
            );
        }
        Menu::CardMore(slot) => {
            let addon = s.addons.addons.iter().find(|addon| addon.slot == slot)?;
            let extra = notes::extra(s, addon);
            let first = extra.always_first;
            items.push(
                menu_item(
                    u,
                    "menu-always-first",
                    Some(&icons::ARROW_UP),
                    if first { "Stop loading first" } else { "Always load first" },
                    "",
                    true,
                )
                .on_press(move |st: &mut State| {
                    st.installed.menu = None;
                    notes::toggle_always_first(st, slot);
                }),
            );
            items.push(
                menu_item(u, "menu-edit", Some(&icons::SQUARE_PEN), "Edit name and picture", "", true)
                    .on_press(move |st: &mut State| {
                        st.installed.menu = None;
                        notes::open_edit(st, slot);
                    }),
            );
            items.push(
                menu_item(u, "menu-tools", Some(&icons::WRENCH), "Check this mod", "", true)
                    .on_press(move |st: &mut State| {
                        st.installed.menu = Some(Menu::Tools(slot));
                    }),
            );
            items.push(
                menu_item(u, "menu-imprint", Some(&icons::FILE), "View imprint", "", true)
                    .on_press(move |st: &mut State| imprint::open(st, slot)),
            );
            items.push(
                menu_item(u, "menu-backups", Some(&icons::ROTATE_CCW), "Backups", "", true)
                    .on_press(move |st: &mut State| backups::open(st, slot)),
            );
            if meta_for(s, addon).and_then(|meta| meta.gamebanana_id).is_some() {
                let busy = s.installed.updates.running == Some(slot) || s.installed.updates.queue.contains(&slot);
                items.push(
                    menu_item(u, "menu-reinstall", Some(&icons::DOWNLOAD), "Reinstall", "", !busy)
                        .on_press(move |st: &mut State| reinstall(st, slot)),
                );
                items.push(
                    menu_item(u, "menu-nuke", Some(&icons::TRASH_2), "Delete and reinstall", "a clean copy", !busy)
                        .on_press(move |st: &mut State| nuke_and_reinstall(st, slot)),
                );
            }
            items.push(menu_rule(u));
            let selected = s.installed.selected.contains(&slot);
            items.push(
                menu_item(
                    u,
                    "menu-select",
                    Some(&icons::CIRCLE_CHECK),
                    if selected { "Deselect" } else { "Select" },
                    "",
                    true,
                )
                .on_press(move |st: &mut State| {
                    st.installed.menu = None;
                    pick(st, slot, false);
                }),
            );
            if let Some(copy) = folder_fix::menu_item(s, u, slot) {
                items.push(copy);
            }
            items.push(
                danger_item(u, "menu-delete", "Delete")
                    .on_press(move |st: &mut State| {
                        st.installed.menu = None;
                        ask(st, Confirm::Delete(vec![slot]));
                    }),
            );
            items.push(menu_rule(u));
            items.push(
                menu_item(u, "cardmore-back", Some(&icons::CHEVRON_LEFT), "Back", "", true)
                    .on_press(move |st: &mut State| {
                        st.installed.menu = Some(Menu::Card(slot));
                    }),
            );
        }
        Menu::Tools(slot) => {
            let addon = s.addons.addons.iter().find(|addon| addon.slot == slot)?;
            let unknown = meta_for(s, addon)
                .and_then(|meta| meta.gamebanana_id)
                .is_none();
            items.push(
                menu_item(u, "tools-safety", Some(&icons::SHIELD), "Check what is inside", "scripts and programs", true)
                    .on_press(move |st: &mut State| tools::open_scan(st, slot)),
            );
            items.push(
                menu_item(
                    u,
                    "tools-fonts",
                    Some(&icons::FILE),
                    "Fonts",
                    if tools::has_fonts(s, slot) { "added" } else { "add to the game" },
                    true,
                )
                    .on_press(move |st: &mut State| tools::open_fonts(st, slot)),
            );
            items.push(
                menu_item(
                    u,
                    "tools-identify",
                    Some(&icons::SEARCH),
                    if unknown { "Find which mod this is" } else { "Check the GameBanana link" },
                    "",
                    true,
                )
                .on_press(move |st: &mut State| tools::open_identify(st, slot)),
            );
            items.push(
                menu_item(
                    u,
                    "tools-health",
                    Some(&icons::TRIANGLE_ALERT),
                    "Check compatibility",
                    "paths the game moved",
                    true,
                )
                .on_press(|st: &mut State| tools::open_health(st)),
            );
            items.push(menu_rule(u));
            items.push(
                menu_item(u, "tools-back", Some(&icons::CHEVRON_LEFT), "Back", "", true)
                    .on_press(move |st: &mut State| {
                        st.installed.menu = Some(Menu::CardMore(slot));
                    }),
            );
        }
        Menu::Import => {
            items.push(
                menu_item(u, "menu-local", None, "Local files", ".vpk or .zip", true)
                    .on_press_tx(|st: &mut State, tx: &mut Tx| importer::pick_local(st, tx)),
            );
            items.push(menu_rule(u));
            items.push(menu_label(u, "From another mod manager"));
            items.push(manager_item(s, u, SourceKind::Grimoire));
            items.push(manager_item(s, u, SourceKind::Dmm));
        }
        Menu::Managers => {
            items.push(manager_item(s, u, SourceKind::Grimoire));
            items.push(manager_item(s, u, SourceKind::Dmm));
        }
        Menu::Source => {
            for source in Source::ALL {
                let on = s.installed.source == source;
                items.push(
                    menu_item(
                        u,
                        &format!("menu-source-{source:?}"),
                        Some(if on { &icons::CHECK } else { &NO_ICON }),
                        source.label(),
                        "",
                        true,
                    )
                    .on_press(move |st: &mut State| {
                        st.installed.menu = None;
                        st.installed.source = source;
                    }),
                );
            }
        }
        Menu::Sort => {
            for sort in Sort::ALL {
                let on = s.installed.sort == sort;
                items.push(
                    menu_item(
                        u,
                        &format!("menu-sort-{sort:?}"),
                        Some(if on { &icons::CHECK } else { &NO_ICON }),
                        sort.label(),
                        "",
                        true,
                    )
                    .on_press(move |st: &mut State| {
                        st.installed.menu = None;
                        st.installed.sort = sort;
                    }),
                );
            }
        }
        Menu::View => {
            for look in layout::Layout::ALL {
                let on = s.installed.layout == look;
                items.push(
                    menu_item(
                        u,
                        &format!("menu-view-{}", look.label()),
                        Some(if on { &icons::CHECK } else { &NO_ICON }),
                        look.label(),
                        "",
                        true,
                    )
                    .on_press(move |st: &mut State| {
                        st.installed.menu = None;
                        st.installed.layout = look;
                    }),
                );
            }
            // Card style and sections (A3).
            items.push(menu_rule(u));
            let v = &s.installed;
            for (key, label, on, flip) in [
                ("menu-view-details", tr("Show the details line"), !v.hide_details, 0u8),
                ("menu-view-slot", tr("Show the slot number"), !v.hide_slot, 1),
                ("menu-view-by-list", tr("Sections by list"), v.by_list, 2),
            ] {
                items.push(
                    menu_item(u, key, Some(if on { &icons::CHECK } else { &NO_ICON }), label, "", flip != 2 || !s.installed.notes.extras.lists.is_empty())
                        .on_press(move |st: &mut State| match flip {
                            0 => st.installed.hide_details = !st.installed.hide_details,
                            1 => st.installed.hide_slot = !st.installed.hide_slot,
                            _ => st.installed.by_list = !st.installed.by_list,
                        }),
                );
            }
        }
        Menu::Hero | Menu::Category | Menu::Tag | Menu::List => {
            items.extend(notes::menu_rows(s, u, menu));
        }
        Menu::Overlap(slot) => items = overlap::menu_items(s, u, slot),
        Menu::Order(slot) => items = req_order::menu_items(s, u, slot),
        Menu::Selection => items = selection::menu_items(s, u),
        Menu::More => {
            let shown = shown_addons(s);
            let picked = !s.installed.selected.is_empty();
            let off = bulk_targets(s, true).len();
            let on = bulk_targets(s, false).len();
            let scope = if picked { "selected" } else { "shown" };
            let count = shown.len();
            items.push(
                menu_item(
                    u,
                    "menu-select-all",
                    Some(&icons::CIRCLE_CHECK),
                    &format!("Select {count} shown"),
                    "",
                    count > 0,
                )
                .on_press(|st: &mut State| {
                    st.installed.menu = None;
                    select_all_shown(st);
                }),
            );
            items.push(menu_rule(u));
            items.push(
                menu_item(
                    u,
                    "menu-bulk-on",
                    Some(&icons::POWER),
                    &format!("Turn on {off} {scope}"),
                    "",
                    off > 0,
                )
                .on_press(|st: &mut State| ask_bulk(st, true)),
            );
            items.push(
                menu_item(
                    u,
                    "menu-bulk-off",
                    Some(&icons::POWER),
                    &format!("Turn off {on} {scope}"),
                    "",
                    on > 0,
                )
                .on_press(|st: &mut State| ask_bulk(st, false)),
            );
            items.push(menu_rule(u));
            items.push(
                menu_item(u, "menu-copy-list", Some(&icons::COPY), "Copy the mod list", "to the clipboard", true)
                    .on_press_tx(|st: &mut State, tx: &mut Tx| {
                        st.installed.menu = None;
                        let text = copy_lines(st);
                        tx.copy(text);
                        st.status = "Mod list copied.".to_string();
                    }),
            );
            items.push(
                menu_item(u, "menu-lists", Some(&icons::LIST), "Mod lists", "", true)
                    .on_press(|st: &mut State| {
                        st.installed.menu = None;
                        st.installed.notes.dialog = Some(notes::Dialog::Lists);
                    }),
            );
            items.push(
                menu_item(u, "menu-merge-open", Some(&icons::LAYERS), "Merge mods", "one VPK from several", true)
                    .on_press(|st: &mut State| tools::open_merge(st)),
            );
            items.push(
                menu_item(u, "menu-pack-all", Some(&icons::LAYERS), tr("Pack all enabled"), tr("into 2 to 8 VPKs"), true)
                    .on_press(pack_all::open),
            );
            items.push(
                menu_item(u, "menu-analyze", Some(&icons::WRENCH), "Analyze the addons folder", "and check HUD mods", true)
                    .on_press(|st: &mut State| checks::open(st)),
            );
            items.push(
                menu_item(u, "menu-safety", Some(&icons::SHIELD), "Mod safety", "review and allowed mods", true)
                    .on_press(safety_review::open),
            );
            items.push(
                menu_item(u, "menu-folder-fix", Some(&icons::WRENCH), "Repair the mods folder", &folder_fix::menu_detail(s), true)
                    .on_press(folder_fix::open),
            );
            items.push(
                menu_item(u, "menu-search", Some(&icons::SEARCH), "Find a broken mod", "halve the set", true)
                    .on_press(|st: &mut State| {
                        st.installed.menu = None;
                        search::search_open(st);
                    }),
            );
            items.push(menu_rule(u));
            let checking = s.installed.updates.check.is_running();
            items.push(
                menu_item(
                    u,
                    "menu-check-updates",
                    Some(&icons::REFRESH_CW),
                    if checking { "Checking for updates" } else { "Check for updates" },
                    "",
                    !checking,
                )
                .on_press(|st: &mut State| {
                    st.installed.menu = None;
                    check_updates(st, true);
                }),
            );
            items.push(
                menu_item(
                    u,
                    "menu-health",
                    Some(&icons::TRIANGLE_ALERT),
                    "Compatibility review",
                    "paths the game moved",
                    true,
                )
                .on_press(|st: &mut State| {
                    st.installed.menu = None;
                    tools::open_health(st);
                }),
            );
            items.push({
                let n = s.installed.trash.entries.len();
                menu_item(
                    u,
                    "menu-trash",
                    Some(&icons::TRASH_2),
                    "Trash",
                    &if n == 0 { String::new() } else { n.to_string() },
                    true,
                )
                .on_press(|st: &mut State| {
                    st.installed.menu = None;
                    open_trash(st);
                })
            });
            if let Some(undo) = &s.installed.last_undo {
                items.push(
                    menu_item(u, "menu-undo", Some(&icons::ROTATE_CCW), &undo_label(undo), "", true)
                        .on_press(|st: &mut State| {
                            st.installed.menu = None;
                            if let Some(undo) = st.installed.last_undo.take() {
                                apply_undo(st, undo);
                            }
                        }),
                );
            }
            if s.prefs.skip_confirm.iter().any(|id| id.starts_with("installed.")) {
                items.push(
                    menu_item(u, "menu-ask-again", Some(&icons::INFO), "Ask before changes again", "", true)
                        .on_press(|st: &mut State| {
                            st.installed.menu = None;
                            crate::prefs::edit(st, |p| p.skip_confirm.retain(|id| !id.starts_with("installed.")));
                            st.status = "Sigil asks again before deleting, reordering or changing several mods.".to_string();
                        }),
                );
            }
        }
    }

    let mut list = col()
        .key("installed-menu-list")
        .w_full()
        .shrink()
        .min_h(0.0)
        .max_h(560.0)
        .scroll(ScrollSpec { stick_bottom: false });
    for item in items {
        list = list.child(item);
    }
    let panel = col()
        .key("installed-menu")
        .role("menu")
        .w(width)
        .padding(4.0)
        .shrink()
        .min_h(0.0)
        .fill(u.raised)
        .radius(crate::ui::r(10.0))
        .border(1.0, u.hair_strong)
        .shadow(12.0, 32.0, Color::BLACK.alpha(0.5))
        .anchored_fit(anchor, Side::Bottom, align, 6.0)
        .modal(true, |st: &mut State| st.installed.menu = None)
        .child(list);
    Some(panel)
}

/// How many mods are on.
fn st_on(s: &State) -> usize {
    s.addons.addons.iter().filter(|addon| addon.enabled).count()
}

/// The mod list as text for the clipboard: the name, author and slot, one mod a line, the
/// mods on first.
fn copy_lines(s: &State) -> String {
    copy_lines_of(s, None)
}

/// The same lines for the picked mods only, or every shown mod when `only` is `None`.
fn copy_lines_of(s: &State, only: Option<&BTreeSet<u16>>) -> String {
    let mut lines = Vec::new();
    for enabled in [true, false] {
        for addon in shown_addons(s)
            .into_iter()
            .filter(|addon| addon.enabled == enabled)
            .filter(|addon| only.is_none_or(|only| only.contains(&addon.slot)))
        {
            let name = display_name(s, addon);
            let author = meta_for(s, addon)
                .and_then(|meta| meta.author.clone())
                .map(|author| format!(" by {author}"))
                .unwrap_or_default();
            lines.push(format!(
                "{:02}\t{name}{author}{}",
                addon.slot,
                if addon.enabled { "" } else { "\t(off)" }
            ));
        }
    }
    lines.join("\n")
}

fn menu_label(u: &Ui, label: &str) -> El {
    row()
        .w_full()
        .h(28.0)
        .px(10.0)
        .center_y()
        .child(text(label.to_string()).font(SMALL).color(u.faint))
}

fn manager_item(s: &State, u: &Ui, kind: SourceKind) -> El {
    let (detail, ready) = importer::source_detail(s, kind);
    let item = menu_item(
        u,
        &format!("menu-{}", kind.key()),
        None,
        kind.label(),
        &detail,
        ready,
    );
    if ready {
        item.on_press(move |st: &mut State| importer::open_source(st, kind))
    } else {
        item
    }
}

pub(super) fn menu_item(
    u: &Ui,
    key: &str,
    glyph: Option<&'static rune::Icon>,
    label: &str,
    detail: &str,
    enabled: bool,
) -> El {
    let mut r = row()
        .key(key)
        .w_full()
        .h(36.0)
        .px(10.0)
        .gap(10.0)
        .center_y()
        .radius(crate::ui::r(6.0))
        .role("menuitem")
        .label(label);
    if enabled {
        r = r
            .focusable(true)
            .hover(u.hover)
            .press(Color::WHITE.alpha(0.08))
            .cursor(rune::Cursor::Pointer);
    } else {
        r = r.disabled(true);
    }
    if let Some(glyph) = glyph {
        r = r.child(icon(glyph, 16.0).color(u.muted).a11y_hidden());
    }
    r = r.child(
        text(label.to_string())
            .font(BODY)
            .color(if enabled { u.ink } else { u.faint })
            .a11y_hidden(),
    );
    r.child(spacer())
        .child(text(detail.to_string()).font(SMALL).color(u.faint).a11y_hidden())
}

// --- library and previews -------------------------------------------------------

/// Reread the library after a scan changed the slots, and fetch previews that are missing.
/// Previews Installed shows: library entries for slots on disk, plus the open import review.
fn preview_urls(st: &State) -> Vec<String> {
    let mut wanted: Vec<String> = st
        .addons
        .addons
        .iter()
        .filter_map(|addon| meta_for(st, addon).and_then(|meta| meta.thumbnail_url.clone()))
        .collect();
    wanted.extend(importer::wanted_thumbs(st));
    wanted
}

/// Leaving Installed: drop the decoded previews and forget the batch, so coming back fetches
/// them again (from the disk cache).
pub fn release_previews(st: &mut State) {
    let mut keep: HashSet<String> =
        crate::screens::settings::personalize::in_use(&st.prefs.personal).into_iter().collect();
    // A Browse card may show the same GameBanana thumbnail.
    if let Some(page) = st.browse.page.as_ref() {
        keep.extend(page.mods.iter().filter_map(|card| card.thumbnail_url.clone()));
    }
    for url in preview_urls(st) {
        if !keep.contains(&url) {
            st.thumbnails.remove(&url);
        }
    }
    st.installed.batch.clear();
    st.installed.thumbs.clear();
}

/// A mod's first gallery image from its GameBanana page, for an Installed card recorded
/// without a preview. Runs on a worker; `None` when the page or its images are missing.
pub fn preview_of(id: u64) -> Option<String> {
    let profile = deadlock_core::gamebanana::fetch_profile(deadlock_core::gamebanana::BrowseSection::Mods, id).ok()?;
    profile.images.first().map(|image| image.url.clone()).filter(|url| allowed_image(url))
}

/// Installs recorded without a preview (from an older Sigil, or from a page that showed none)
/// get one: up to eight GameBanana lookups per pass while Installed is open, each written back
/// to the library only when the slot still holds that mod.
fn fill_previews(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(Ok(found)) = st.installed.preview_fill.try_take() {
        for (slot, size, url) in found {
            if let Some(meta) = st
                .installed
                .library
                .mods
                .iter_mut()
                .find(|m| m.slot == slot && m.size_bytes == size && m.thumbnail_url.is_none())
            {
                meta.thumbnail_url = Some(url);
                changed = true;
            }
        }
    }
    if st.demo || rune::is_headless() || st.screen != Screen::Installed || st.installed.preview_fill.is_running() {
        return changed;
    }
    let wanted: Vec<(u16, u64, u64)> = st
        .installed
        .library
        .mods
        .iter()
        .filter(|m| m.thumbnail_url.is_none())
        .filter_map(|m| m.gamebanana_id.map(|id| (m.slot, m.size_bytes, id)))
        .filter(|(_, _, id)| !st.installed.preview_tried.contains(id))
        .take(8)
        .collect();
    if wanted.is_empty() {
        return changed;
    }
    st.installed.preview_tried.extend(wanted.iter().map(|(_, _, id)| *id));
    let library = deadlock_core::import::library_path(&crate::data::data_dir());
    let wake = st.wake.clone();
    st.installed.preview_fill.start(wake, move || {
        let mut found = Vec::new();
        for (slot, size, id) in wanted {
            if let Some(url) = preview_of(id)
                && deadlock_core::import::set_thumbnail(&library, slot, size, id, &url).unwrap_or(false)
            {
                found.push((slot, size, url));
            }
        }
        Ok(found)
    });
    changed
}

pub fn poll(st: &mut State) -> bool {
    let mut changed = fill_previews(st);
    changed |= importer::poll(st);
    changed |= notes::poll(st);
    changed |= tools::poll(st);
    changed |= pack_all::poll(st);
    changed |= crate::screens::one_skin::poll(st);
    changed |= poll_changes(st);
    changed |= enable::poll(st);
    changed |= enable::refresh_hidden(st);
    changed |= overlap::refresh(st);
    changed |= req_order::refresh(st);
    changed |= repair::poll(st);
    changed |= checks::poll(st);
    changed |= imprint::poll(st);
    changed |= folder_fix::poll(st);
    changed |= sound::poll(st);
    crate::screens::conflicts::sync_scope(st);
    changed |= crate::screens::locker::art::poll(st);

    if !st.demo && st.addons.scanned {
        let sig: Vec<(u16, u64)> = st
            .addons
            .addons
            .iter()
            .map(|addon| (addon.slot, addon.size_bytes))
            .collect();
        if st.installed.library_sig.as_ref() != Some(&sig) {
            st.installed.library =
                import::read_library(&import::library_path(&crate::data::data_dir()))
                    .unwrap_or_default();
            st.installed.library_sig = Some(sig);
            changed = true;
        }
    }

    if st.screen == Screen::Installed
        && st.addons.scanned
        && st.addons.addons.is_empty()
        && st.game_detected
    {
        importer::start_detect(st);
    }

    // Previews: library entries for slots on disk, plus the open import review.
    let wanted = preview_urls(st);
    let missing: Vec<String> = wanted
        .into_iter()
        .filter(|url| {
            allowed_image(url)
                && !st.thumbnails.contains_key(url)
                && !st.installed.failed.contains(url)
        })
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let fresh = missing.iter().any(|url| !st.installed.batch.contains(url));
    // Previews decode only while Installed shows them (about 1 MB each); they go on leave.
    let shown = st.screen == Screen::Installed || !importer::wanted_thumbs(st).is_empty();
    if !st.demo && !rune::is_headless() && shown && fresh && st.images.is_some() {
        st.installed.batch = missing.iter().cloned().collect();
        st.installed.thumbs.start(missing, st.wake.clone());
    }
    while let Some((url, bytes)) = st.installed.thumbs.try_next() {
        match (st.images.as_ref(), bytes) {
            (Some(cache), Some(bytes)) => {
                let image = match bytes {
                    crate::data::ThumbBytes::Encoded(bytes) => cache.load(bytes),
                    crate::data::ThumbBytes::Rgba(w, h, pixels) => cache.rgba(w, h, pixels),
                };
                st.thumbnails.insert(url, image.pixels().clone());
            }
            _ => {
                st.installed.failed.insert(url);
            }
        }
        changed = true;
    }
    changed
}

/// Previews come from GameBanana's image hosts only, the one image source on the network
/// allowlist. Another manager's record could name any URL; those stay placeholders.
fn allowed_image(url: &str) -> bool {
    deadlock_core::gamebanana::is_gamebanana_url(url)
}

// --- fixtures ----------------------------------------------------------------------

/// Names and previews for the demo library. Previews are read from the local image cache so
/// the scene needs no network; with an empty cache the cards show placeholders.
pub fn stage_demo(st: &mut State) {
    let names: [(u16, &str, Option<&str>); 10] = [
        (1, "Chromamancer Violet", Some("tselky")),
        (2, "Asa Mitaka over Paige", Some("toast068")),
        (3, "Random Respawn Dances", Some("Bailey31")),
        (4, "The Rebelcapin Collection", Some("dunder_duck")),
        (5, "Mirage Sinclair", Some("MOJKQUEIRA")),
        (7, "Celeste Strawberries over Healing Snacks", Some("Aceman3000")),
        (9, "Taokaka over Drifter", Some("_Elar_")),
        (11, "Kurumi Tokisaki over Paradox", Some("10385")),
        (12, "Soul Container but it is my friend", None),
        (18, "", None),
    ];
    let thumbs = demo_thumbs(st, 9);
    let mut library = Library::default();
    for (n, (slot, name, author)) in names.into_iter().enumerate() {
        if name.is_empty() {
            continue;
        }
        let size = st
            .addons
            .addons
            .iter()
            .find(|addon| addon.slot == slot)
            .map(|addon| addon.size_bytes)
            .unwrap_or(0);
        library.mods.push(ModMeta {
            slot,
            size_bytes: size,
            name: name.to_string(),
            author: author.map(str::to_string),
            thumbnail_url: thumbs.get(n).cloned(),
            gamebanana_id: Some(600_000 + u64::from(slot)),
            source: "browse".to_string(),
            recorded_at: String::new(),
        });
    }
    st.installed.library = library;
    stage_extras(st);
}

/// Hero, category, tags, a pin and a named list for the demo library.
fn stage_extras(st: &mut State) {
    use deadlock_core::mod_extras::{Extras, ModExtra, ModList};
    type Fact<'a> = (u16, &'a str, &'a str, &'a str, &'a [&'a str], bool);
    let facts: [Fact; 9] = [
        (1, "Violet", "Skins", "Violet", &["main"], false),
        (2, "Paige", "Skins", "Paige", &[], true),
        (3, "Yamato", "Skins", "Yamato", &["comp"], false),
        (4, "HUD", "HUD", "", &["comp"], false),
        (5, "Sinclair", "Skins", "Sinclair", &[], false),
        (7, "HUD", "HUD", "", &["ranked"], true),
        (9, "Drifter", "Skins", "Drifter", &[], false),
        (11, "Paradox", "Skins", "Paradox", &["comp"], false),
        (12, "Soul containers", "Misc", "", &[], false),
    ];
    let mut extras = Extras::default();
    for (n, (slot, category, root, hero, tags, pinned)) in facts.into_iter().enumerate() {
        let size = st
            .addons
            .addons
            .iter()
            .find(|addon| addon.slot == slot)
            .map(|addon| addon.size_bytes)
            .unwrap_or(0);
        extras.mods.push(ModExtra {
            uid: format!("demo-{n:02}"),
            slot,
            size_bytes: size,
            gamebanana_id: Some(600_000 + u64::from(slot)),
            fetched: true,
            hero: (!hero.is_empty()).then(|| hero.to_string()),
            category: Some(category.to_string()),
            root_category: Some(root.to_string()),
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
            pinned,
            always_first: slot == 1,
            remote_nsfw: slot == 12,
            ..ModExtra::default()
        });
    }
    extras.lists.push(ModList {
        id: "demo-comp".to_string(),
        name: "Comp".to_string(),
        members: vec!["demo-02".to_string(), "demo-03".to_string(), "demo-07".to_string()],
        random_each_launch: false,
        last_pick: None,
    });
    st.installed.notes.extras = extras;
}

/// Up to `count` cached images as `demo-thumb:N` keys, queued for decode in `setup`.
pub fn demo_thumbs(st: &mut State, count: usize) -> Vec<String> {
    let dir = crate::data::data_dir().join("cache").join("images");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    let name = path.to_string_lossy().to_lowercase();
                    name.ends_with(".jpg") || name.ends_with(".png") || name.ends_with(".webp")
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    let mut keys = Vec::new();
    for (n, path) in files.into_iter().take(count).enumerate() {
        if let Ok(bytes) = std::fs::read(&path) {
            let key = format!("demo-thumb:{n}");
            st.pending_thumbs.push((key.clone(), bytes));
            keys.push(key);
        }
    }
    keys
}

/// The `installed-empty` scene: a detected game with an empty addon folder.
pub fn stage_empty(st: &mut State) {
    st.load_demo();
    st.screen = Screen::Installed;
    st.addons = crate::model::AddonLibrary {
        scanned: true,
        ..Default::default()
    };
    importer::stage_demo_sources(st);
}

// --- folder actions -------------------------------------------------------------------

pub(super) fn addons_folder(s: &State) -> PathBuf {
    s.addons_dir
        .clone()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&s.paths.game).join("game/citadel/addons"))
}

/// Reveal a file or folder in the system file manager. This is a read-only app
/// action: no game file is renamed, copied or written.
pub(super) fn reveal(status: &mut String, path: PathBuf) {
    if rune::is_headless() {
        *status = format!("Headless run: reveal was not sent for {}.", path.display());
        return;
    }
    if !path.exists() {
        *status = format!("{} does not exist on disk.", path.display());
        return;
    }
    let result = reveal_command(&path);
    *status = match result {
        Ok(()) => format!("Revealed {} in the file manager.", path.display()),
        Err(error) => format!(
            "The file manager could not open {}: {error}",
            path.display()
        ),
    };
}

#[cfg(target_os = "macos")]
fn reveal_command(path: &PathBuf) -> Result<(), String> {
    std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "windows")]
fn reveal_command(path: &PathBuf) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("explorer")
        .raw_arg(deadlock_core::autoexec::windows_select_argument(path))
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn reveal_command(path: &PathBuf) -> Result<(), String> {
    let folder = path.parent().unwrap_or(path);
    std::process::Command::new("xdg-open")
        .arg(folder)
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

// --- dialogs ---------------------------------------------------------------------------

/// The import review when one is open; otherwise the enable or disable confirm. A slot toggle
/// is one rename in the addon folder.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    if let Some(dialog) = enable::dialog(s, u) {
        return Some(dialog);
    }
    if let Some(dialog) = importer::dialog(s, u) {
        return Some(dialog);
    }
    if let Some(dialog) = search::recap_dialog(s, u) {
        return Some(dialog);
    }
    if let Some(dialog) = notes::dialog(s, u) {
        return Some(dialog);
    }
    if let Some(dialog) = tools::scan_dialog(s, u) {
        return Some(dialog);
    }
    if let Some(dialog) = safety_review::dialog(s, u) {
        return Some(dialog);
    }
    if let Some(dialog) = folder_fix::dialog(s, u) {
        return Some(dialog);
    }
    if let Some(dialog) = tools::identify_dialog(s, u) {
        return Some(dialog);
    }
    if let Some(dialog) = tools::health_dialog(s, u) {
        return Some(dialog);
    }
    if let Some(dialog) = tools::merge_dialog(s, u) {
        return Some(dialog);
    }
    if let Some(dialog) = pack_all::dialog(s, u) {
        return Some(dialog);
    }
    if let Some(dialog) = tools::fonts_dialog(s, u) {
        return Some(dialog);
    }
    if let Some(dialog) = search::search_dialog(s, u) {
        return Some(dialog);
    }
    if let Some(dialog) = layout::picker_dialog(s, u) {
        return Some(dialog);
    }
    if let Some(dialog) = order::dialog(s, u) {
        return Some(dialog);
    }
    if let Some(dialog) = backups::dialog(s, u) {
        return Some(dialog);
    }
    if let Some(dialog) = checks::dialog(s, u) {
        return Some(dialog);
    }
    if let Some(dialog) = imprint::dialog(s, u) {
        return Some(dialog);
    }
    if s.installed.trash.open {
        return Some(trash_dialog(s, u));
    }
    if let Some(confirm) = &s.installed.confirm {
        return Some(confirm_dialog(s, u, confirm));
    }
    if let Some(enable) = s.installed.bulk {
        return Some(bulk_dialog(s, u, enable));
    }
    let (slot, enable) = s.addon_dialog?;
    let addon = s.addons.addons.iter().find(|addon| addon.slot == slot);
    let name = addon
        .and_then(|addon| meta_for(s, addon))
        .map(|meta| meta.name.clone())
        .unwrap_or_else(|| format!("slot {}", deadlock_core::addons::slot_label(slot)));
    let file = addon
        .map(|addon| addon.file.clone())
        .unwrap_or_else(|| format!("pak{slot:02}_dir.vpk"));
    let title = if enable { "Turn on" } else { "Turn off" };
    let close = |st: &mut State| st.addon_dialog = None;
    Some(
        u.dialog_shell()
            .key("addon-toggle-dialog")
            .role("dialog")
            .label(format!("{title} {name}"))
            .w(440.0)
            .padding(24.0)
            .gap(16.0)
            .modal(true, close)
            .child(text(format!("{title} {name}?")).font(H1).color(u.ink).wrap())
            .child(
                text(if enable {
                    "The game loads it the next time it starts."
                } else {
                    "The game stops loading it. You can turn it back on any time."
                })
                .font(BODY)
                .color(u.muted)
                .wrap(),
            )
            .child(u.kv("Slot", &format!("{slot:02}"), u.ink))
            .child(u.kv("File", &file, u.muted))
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .child(spacer())
                    .child(
                        u.action("Cancel", ActionKind::Ghost)
                            .cursor(rune::Cursor::Pointer)
                            .on_press(close),
                    )
                    .child(
                        u.action(title, ActionKind::Primary)
                            .cursor(rune::Cursor::Pointer)
                            .on_press(move |st: &mut State| toggle_one(st, slot, enable)),
                    ),
            ),
    )
}

/// The confirm for a bulk switch: how many slot files it renames, and which.
fn bulk_dialog(s: &State, u: &Ui, enable: bool) -> El {
    let targets: Vec<&AddonRow> = bulk_targets(s, enable);
    let count = targets.len();
    let title = format!(
        "Turn {} {count} mod{}?",
        if enable { "on" } else { "off" },
        if count == 1 { "" } else { "s" }
    );
    let close = |st: &mut State| st.installed.bulk = None;
    let mut names = col().w_full().gap(4.0);
    for addon in targets.iter().take(6) {
        names = names.child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(text(deadlock_core::addons::slot_label(addon.slot)).font(MICRO).color(u.faint).w(28.0))
                .child(text(display_name(s, addon)).font(BODY).color(u.prose).truncate()),
        );
    }
    if count > 6 {
        names = names.child(text(format!("and {} more", count - 6)).font(SMALL).color(u.faint));
    }
    u.dialog_shell()
        .key("installed-bulk-dialog")
        .role("dialog")
        .label(title.clone())
        .w(460.0)
        .padding(24.0)
        .gap(16.0)
        .modal(true, close)
        .child(text(title.clone()).font(H1).color(u.ink).wrap())
        .child(
            text(if enable {
                "The game loads them the next time it starts."
            } else {
                "The game stops loading them. You can turn them back on any time."
            })
            .font(BODY)
            .color(u.muted)
            .wrap(),
        )
        .child(names)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(dont_ask_row(s, u))
                .child(spacer())
                .child(
                    u.action("Cancel", ActionKind::Ghost)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(close),
                )
                .child(
                    u.action(if enable { "Turn on" } else { "Turn off" }, ActionKind::Primary)
                        .key("installed-bulk-confirm")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| {
                            remember_dont_ask(st, SKIP_BULK);
                            apply_bulk(st, enable);
                        }),
                ),
        )
}

/// Disabled only, sorted by name, with the bulk confirm open.
pub fn scene_bulk(s: &mut State) {
    crate::app::scene_demo_installed(s);
    s.installed.show = Show::Disabled;
    s.installed.sort = Sort::Name;
    s.installed.bulk = Some(true);
}

/// The source dropdown open.
pub fn scene_source_menu(s: &mut State) {
    crate::app::scene_demo_installed(s);
    s.installed.menu = Some(Menu::Source);
}

/// Files dragged over the page.
pub fn scene_drop(s: &mut State) {
    crate::app::scene_demo_installed(s);
    s.installed.drop_hover = true;
}

/// A hero and a tag filter on, with the filter chips.
pub fn scene_filters(s: &mut State) {
    crate::app::scene_demo_installed(s);
    s.installed.notes.hero = Some("Paradox".to_string());
    s.installed.notes.tags_in = vec![(deadlock_core::mod_extras::TagKind::Custom, "comp".to_string())];
}

/// The list look.
pub fn scene_list(s: &mut State) {
    crate::app::scene_demo_installed(s);
    s.installed.layout = layout::Layout::List;
    s.installed.hovered = Some(3);
}

/// Two files of one GameBanana mod as a variant card, its picker open.
pub fn scene_variants(s: &mut State) {
    crate::app::scene_demo_installed(s);
    let size = |st: &State, slot: u16| {
        st.addons
            .addons
            .iter()
            .find(|addon| addon.slot == slot)
            .map(|addon| addon.size_bytes)
            .unwrap_or(0)
    };
    // Slot 11 and 18 are the same mod in the demo, so the card has two files.
    if let Some(meta) = s.installed.library.mods.iter_mut().find(|meta| meta.slot == 11) {
        meta.gamebanana_id = Some(640_018);
    }
    let size_18 = size(s, 18);
    s.installed.library.mods.push(ModMeta {
        slot: 18,
        size_bytes: size_18,
        name: "Kurumi Tokisaki over Paradox".to_string(),
        author: Some("10385".to_string()),
        thumbnail_url: None,
        gamebanana_id: Some(640_018),
        source: "browse".to_string(),
        recorded_at: String::new(),
    });
    stage_extras(s);
    use deadlock_core::mod_extras::ModExtra;
    s.installed.notes.extras.mods.push(ModExtra {
        uid: "demo-18".to_string(),
        slot: 18,
        size_bytes: size_18,
        gamebanana_id: Some(640_018),
        fetched: true,
        variant_label: Some("4K textures".to_string()),
        category: Some("Paradox".to_string()),
        root_category: Some("Skins".to_string()),
        hero: Some("Paradox".to_string()),
        ..ModExtra::default()
    });
    for entry in &mut s.installed.notes.extras.mods {
        if entry.slot == 11 {
            entry.variant_label = Some("2K textures".to_string());
        }
    }
    layout::open_picker(s, Some("gb:640018".to_string()));
}

/// The tags dialog for one card.
pub fn scene_tags(s: &mut State) {
    crate::app::scene_demo_installed(s);
    s.installed.notes.text = TextEdit::new("comp, ranked");
    s.installed.notes.dialog = Some(notes::Dialog::Tags(3));
}

/// The one tag menu open, one tag included and one excluded.
pub fn scene_tag_menu(s: &mut State) {
    use deadlock_core::mod_extras::TagKind;
    crate::app::scene_demo_installed(s);
    s.installed.notes.tags_in = vec![(TagKind::Custom, "comp".to_string())];
    s.installed.notes.tags_out = vec![(TagKind::Base, "Skins".to_string())];
    s.installed.menu = Some(Menu::Tag);
}

/// Repair the mods folder, with a finding in every section.
pub fn scene_folder_fix(s: &mut State) {
    crate::app::scene_demo_installed(s);
    folder_fix::stage_scene(s);
}

/// The mod lists dialog with the selection behind it.
pub fn scene_lists(s: &mut State) {
    crate::app::scene_demo_installed(s);
    s.installed.selected = [2, 3, 9].into_iter().collect();
    s.installed.notes.dialog = Some(notes::Dialog::Lists);
}

/// The local mod editor.
pub fn scene_edit(s: &mut State) {
    crate::app::scene_demo_installed(s);
    s.installed.notes.text = TextEdit::new("Chromamancer Violet");
    s.installed.notes.dialog = Some(notes::Dialog::EditLocal(1));
}

/// The card menu's "More actions" page.
pub fn scene_card_more(s: &mut State) {
    crate::app::scene_demo_installed(s);
    s.installed.menu = Some(Menu::CardMore(2));
    s.installed.hovered = Some(2);
}

/// The card menu's tools page.
pub fn scene_tools_menu(s: &mut State) {
    crate::app::scene_demo_installed(s);
    s.installed.menu = Some(Menu::Tools(2));
    s.installed.hovered = Some(2);
}

/// A safety scan report with a program and a script in it.
pub fn scene_safety(s: &mut State) {
    crate::app::scene_demo_installed(s);
    use deadlock_core::mod_safety::{Finding, Risk, ScanReport};
    let report = ScanReport {
        findings: vec![
            Finding {
                path: "bin/helper.exe".to_string(),
                risk: Risk::Program,
                call: String::new(),
            },
            Finding {
                path: "panorama/layout/hud.xml".to_string(),
                risk: Risk::Remote,
                call: "fetch(".to_string(),
            },
            Finding {
                path: "scripts/hello.js".to_string(),
                risk: Risk::Script,
                call: String::new(),
            },
        ],
        more: 0,
        entries: 214,
        error: None,
    };
    if let Some(addon) = s.addons.addons.iter().find(|addon| addon.slot == 2) {
        s.installed
            .tools
            .scans
            .insert((2, addon.size_bytes), report.clone());
    }
    s.installed.tools.scan = Some(tools::ScanUi {
        slot: 2,
        report: Some(report),
    });
}

/// The identify dialog for an unknown file, after a failed lookup.
pub fn scene_identify(s: &mut State) {
    crate::app::scene_demo_installed(s);
    s.installed.tools.identify = Some(tools::IdentifyUi {
        slot: 18,
        link: TextEdit::new("https://gamebanana.com/mods/"),
        found: None,
        stage: tools::Stage::Failed,
        error: None,
        job: Job::default(),
        pending: None,
    });
}

/// The compatibility review with two mods to look at.
pub fn scene_health(s: &mut State) {
    crate::app::scene_demo_installed(s);
    use deadlock_core::mod_health::{Issue, IssueKind, SlotReview};
    s.installed.tools.health = Some(tools::HealthUi {
        reviews: vec![
            SlotReview {
                slot: 2,
                issues: vec![
                    Issue {
                        kind: IssueKind::Moved,
                        path: "models/heroes/paige/paige_body.vmdl_c".to_string(),
                        now_at: Some("models/heroes_paige/paige_body.vmdl_c".to_string()),
                    },
                    Issue {
                        kind: IssueKind::Gone,
                        path: "materials/heroes/paige/old.vmat_c".to_string(),
                        now_at: None,
                    },
                ],
                more: 0,
                entries: 340,
                missing: 2,
                error: None,
            },
            SlotReview {
                slot: 9,
                issues: vec![Issue {
                    kind: IssueKind::Skeleton,
                    path: "models/heroes_wip/drifter/drifter.vmdl_c".to_string(),
                    now_at: None,
                }],
                more: 0,
                entries: 120,
                missing: 1,
                error: None,
            },
        ],
        running: false,
        confirm_off: false,
        error: None,
        job: Job::default(),
        checked: 7,
    });
}

/// The merge dialog with three mods picked and a preview.
pub fn scene_merge(s: &mut State) {
    crate::app::scene_demo_installed(s);
    s.installed.tools.merge = Some(tools::MergeUi {
        picked: [1, 2, 3].into_iter().collect(),
        name: TextEdit::new("All my skins"),
        strict: false,
        preview: Some(deadlock_core::mod_merge::MergePreview {
            sources: Vec::new(),
            paths: 612,
            collisions: vec![(
                "materials/models/heroes/atlas/atlas_body.vmat_c".to_string(),
                vec![3, 2, 1],
            )],
            bytes: 48 * 1024 * 1024,
            error: None,
        }),
        stage: tools::MergeStage::Picking,
        error: None,
    });
}

/// The solo test line above the cards.
pub fn scene_solo(s: &mut State) {
    crate::app::scene_demo_installed(s);
    s.installed.solo = Some(search::Solo {
        name: "Asa Mitaka over Paige".to_string(),
        others: vec![
            (1, 0, "demo-00".to_string()),
            (3, 0, "demo-02".to_string()),
            (4, 0, "demo-03".to_string()),
        ],
    });
}

/// The broken mod search in its second round.
pub fn scene_bisect(s: &mut State) {
    crate::app::scene_demo_installed(s);
    search::search_open(s);
    let picked: Vec<u16> = s.installed.search.picked.iter().copied().collect();
    let original = s.installed.search.original.clone();
    let mut bisect = deadlock_core::bisect::Bisect::new(&original, &picked);
    bisect.judge(deadlock_core::bisect::Verdict::Works);
    s.installed.search.search = Some(bisect);
}

/// The recap after a bulk switch.
pub fn scene_recap(s: &mut State) {
    crate::app::scene_demo_installed(s);
    let mut recap = search::Recap::new("Turned off 4 mods");
    recap.done = vec![
        "Asa Mitaka over Paige".to_string(),
        "Random Respawn Dances".to_string(),
        "Mirage Sinclair".to_string(),
    ];
    recap.failed = vec![(
        "Celeste Strawberries".to_string(),
        "Slot 07 is in use.".to_string(),
    )];
    recap.skipped = vec!["09 Taiga".to_string()];
    recap.stopped = Some("Stopped after the first failure.".to_string());
    s.installed.recap = Some(recap);
}

/// The fonts prompt for a mod that ships one.
pub fn scene_fonts(s: &mut State) {
    crate::app::scene_demo_installed(s);
    s.installed.tools.fonts = Some(tools::FontUi {
        slot: 2,
        fonts: vec![deadlock_core::fonts::Font {
            file_name: "Brotherhood.ttf".to_string(),
            family: "Brotherhood".to_string(),
            bytes: 92_000,
        }],
        stage: tools::Stage::Found,
        error: None,
        installed: false,
    });
}

// --- icons not in the generated set (Lucide, ISC) -------------------------------------

static ARROW_DOWN: rune::Icon = rune::Icon {
    name: "sigil-installed-arrow-down",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M12 5v14"/><path d="m19 12-7 7-7-7"/></g>"#,
};
static ARROW_UP_TO_LINE: rune::Icon = rune::Icon {
    name: "sigil-installed-arrow-up-to-line",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M5 3h14"/><path d="m18 13-6-6-6 6"/><path d="M12 7v14"/></g>"#,
};
static ARROW_RIGHT: rune::Icon = rune::Icon {
    name: "sigil-installed-arrow-right",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12h14"/><path d="m12 5 7 7-7 7"/></g>"#,
};

pub(super) fn menu_rule(u: &Ui) -> El {
    el().w_full().h(1.0).my(4.0).fill(u.hair)
}

/// A menu row for a destructive action: rose icon and label.
fn danger_item(u: &Ui, key: &str, label: &str) -> El {
    row()
        .key(key)
        .w_full()
        .h(36.0)
        .px(10.0)
        .gap(10.0)
        .center_y()
        .radius(crate::ui::r(6.0))
        .role("menuitem")
        .label(label)
        .focusable(true)
        .hover(u.rose.alpha(0.10))
        .press(u.rose.alpha(0.16))
        .cursor(rune::Cursor::Pointer)
        .child(icon(&icons::TRASH_2, 16.0).color(u.rose).a11y_hidden())
        .child(text(label.to_string()).font(BODY).color(u.rose).a11y_hidden())
}

// --- selection -------------------------------------------------------------------------

/// Pick or unpick one card. With Shift, everything shown between the last pick and this
/// card is picked.
pub(super) fn pick(st: &mut State, slot: u16, shift: bool) {
    let v = &st.installed;
    if shift && let Some(anchor) = v.anchor {
        let order: Vec<u16> = shown_order(st);
        let a = order.iter().position(|&s| s == anchor);
        let b = order.iter().position(|&s| s == slot);
        if let (Some(a), Some(b)) = (a, b) {
            let (lo, hi) = (a.min(b), a.max(b));
            let range: Vec<u16> = order[lo..=hi].to_vec();
            st.installed.selected.extend(range);
            st.installed.anchor = Some(slot);
            return;
        }
    }
    if !st.installed.selected.remove(&slot) {
        st.installed.selected.insert(slot);
    }
    st.installed.anchor = Some(slot);
}

/// The shown cards in the order they are drawn: the Enabled group, then Disabled.
fn shown_order(s: &State) -> Vec<u16> {
    let shown = shown_addons(s);
    let mut order: Vec<u16> = shown.iter().filter(|a| a.enabled).map(|a| a.slot).collect();
    order.extend(shown.iter().filter(|a| !a.enabled).map(|a| a.slot));
    order
}

fn select_all_shown(st: &mut State) {
    let order = shown_order(st);
    st.installed.selected.extend(order);
}

/// The bar that replaces the filters while cards are picked.
fn selection_bar(s: &State, u: &Ui) -> El {
    let count = s.installed.selected.len();
    let shown = shown_order(s);
    let all = shown.iter().all(|slot| s.installed.selected.contains(slot));
    let on = bulk_targets(s, true).len();
    let off = bulk_targets(s, false).len();
    let mut bar = row()
        .key("installed-selection")
        .w_full()
        .h(32.0)
        .gap(8.0)
        .center_y()
        .child(
            u.icon_button(&icons::X, "Clear the selection")
                .key("installed-select-clear")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| {
                    st.installed.selected.clear();
                    st.installed.anchor = None;
                }),
        )
        .child(text(format!("{count} selected")).font(BODY_MED).color(u.ink));
    if !all {
        bar = bar.child(
            row()
                .key("installed-select-all")
                .h(28.0)
                .px(8.0)
                .center_y()
                .radius(crate::ui::r(6.0))
                .hover(u.hover)
                .focusable(true)
                .role("button")
                .label(format!("Select all {}", shown.len()))
                .cursor(rune::Cursor::Pointer)
                .on_press(select_all_shown)
                .child(
                    text(format!("Select all {}", shown.len()))
                        .font(BODY)
                        .color(u.muted)
                        .a11y_hidden(),
                ),
        );
    }
    use crate::screens::settings::i18n::{fill, tr};
    let updates: Vec<u16> = s
        .installed
        .selected
        .iter()
        .copied()
        .filter(|slot| available_updates(s).contains(slot))
        .collect();
    bar = bar.child(spacer());
    if !updates.is_empty() {
        bar = bar.child(
            u.icon_action(&icons::ARROW_UP, &fill(tr("Update {}"), &[&updates.len().to_string()]), ActionKind::Secondary)
                .key("installed-selection-update")
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| ask(st, Confirm::UpdateAll(updates.clone()))),
        );
    }
    bar.child(
            u.icon_action(&icons::COPY, tr("Copy"), ActionKind::Secondary)
                .key("installed-selection-copy")
                .label(tr("Copy the selected mods as a list"))
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(|st: &mut State, tx: &mut Tx| {
                    let picked = st.installed.selected.clone();
                    tx.copy(copy_lines_of(st, Some(&picked)));
                    st.status = tr("Mod list copied.").to_string();
                }),
        )
        .child(
            u.icon_action(&icons::POWER, "Turn on", ActionKind::Secondary)
                .key("installed-selection-on")
                .disabled(on == 0)
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| ask_bulk(st, true)),
        )
        .child(
            u.icon_action(&icons::POWER, "Turn off", ActionKind::Secondary)
                .key("installed-selection-off")
                .disabled(off == 0)
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| ask_bulk(st, false)),
        )
        .child(
            u.icon_action(&icons::TRASH_2, "Delete", ActionKind::Danger)
                .key("installed-selection-delete")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| {
                    let slots: Vec<u16> = st.installed.selected.iter().copied().collect();
                    ask(st, Confirm::Delete(slots));
                }),
        )
        .child(
            u.icon_button(&icons::ELLIPSIS, tr("More for the selection"))
                .key("installed-selection-more")
                .border(1.0, u.hair_strong)
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| open_menu(st, Menu::Selection)),
        )
}

// --- writes ----------------------------------------------------------------------------

pub(super) fn trash_dir() -> PathBuf {
    trash::trash_root(&crate::data::data_dir())
}

pub(super) fn library_file() -> PathBuf {
    import::library_path(&crate::data::data_dir())
}

/// Read the addon folder again, now and on the worker, and the library after it.
pub(super) fn rescan(st: &mut State) {
    st.installed.library_sig = None;
    crate::data::scan_addons_now(st);
    crate::data::start_addon_scan(st);
}

/// The game folder, or `None` with the reason in the status line (and in the demo).
pub(super) fn writable_game(st: &mut State) -> Option<PathBuf> {
    if st.demo {
        st.status = "Demo library: nothing was changed.".to_string();
        return None;
    }
    let root = st.game_root.clone();
    if root.is_none() {
        st.status = "Set the game folder in Settings first.".to_string();
    }
    root
}

/// Switch several slots with one Undo, then warn when every slot is in use.
pub(super) fn apply_switch(
    st: &mut State,
    game_root: &std::path::Path,
    wanted: &BTreeMap<u16, bool>,
    line: String,
) {
    if crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    match deadlock_core::addons::set_many(game_root, wanted, true) {
        Ok(report) => {
            if !report.done.is_empty() {
                st.installed.last_undo = Some(Undo::Toggle(report.done));
                st.installed.undo_fresh = true;
            }
            st.status = if report.failed.is_empty() {
                line
            } else {
                format!("{line} {} could not be switched.", report.failed.len())
            };
        }
        Err(error) => st.status = error.to_string(),
    }
    rescan(st);
    warn_at_cap(st);
}

/// The slot cap. Past 99 mods Sigil fills extra addon folders (`sigil-addons2` and on), so the
/// cap is [`deadlock_core::addons::MAX_SLOT`]; say so when it is reached.
pub(super) fn warn_at_cap(st: &mut State) {
    let used = st.addons.addons.len();
    if used >= usize::from(deadlock_core::addons::MAX_SLOT) {
        crate::notify::push(
            st,
            crate::notify::Notice::warning("Every addon slot is in use")
                .detail(format!(
                    "All {} slots hold a mod. Remove one before adding another.",
                    deadlock_core::addons::MAX_SLOT
                )),
        );
    }
}

/// The card switch: one rename, remembered for Undo.
pub(super) fn toggle_one(st: &mut State, slot: u16, enable: bool) {
    st.addon_dialog = None;
    if crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    let Some(game_root) = writable_game(st) else {
        return;
    };
    match deadlock_core::addons::set_enabled(&game_root, slot, enable, true) {
        Ok(result) => {
            st.status = format!("Slot {:02} turned {}.", result.slot, if enable { "on" } else { "off" });
            st.installed.last_undo = Some(Undo::Toggle(vec![result]));
            st.installed.undo_fresh = true;
            rescan(st);
            warn_at_cap(st);
        }
        Err(error) => st.status = error.to_string(),
    }
}

/// Open a confirm, or run the change at once when the user said not to ask.
pub(super) fn ask(st: &mut State, confirm: Confirm) {
    st.installed.menu = None;
    if st.prefs.skip_confirm.contains(confirm.id()) {
        run_confirm(st, confirm);
    } else {
        st.installed.dont_ask = false;
        st.installed.confirm = Some(confirm);
    }
}

fn run_confirm(st: &mut State, confirm: Confirm) {
    st.installed.confirm = None;
    match confirm {
        Confirm::Delete(slots) => delete(st, slots),
        Confirm::Reorder(plan) => reorder(st, plan),
        Confirm::UpdateAll(slots) => {
            let slots = scoped(st, &slots);
            queue_updates(st, slots)
        }
    }
}

/// Persist "Don't ask again" for this kind of confirm when it is ticked.
fn remember_dont_ask(st: &mut State, id: &'static str) {
    if st.installed.dont_ask {
        crate::prefs::edit(st, |p| {
            p.skip_confirm.insert(id.to_string());
        });
        st.installed.dont_ask = false;
    }
}

fn dont_ask_row(s: &State, u: &Ui) -> El {
    let on = s.installed.dont_ask;
    row()
        .key("installed-dont-ask")
        .h(32.0)
        .gap(8.0)
        .center_y()
        .focusable(true)
        .role("checkbox")
        .checked(on)
        .label("Don't ask again")
        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| st.installed.dont_ask = !st.installed.dont_ask)
        .child(check_box(u, on, false))
        .child(text("Don't ask again").font(BODY).color(u.muted).a11y_hidden())
}

/// Move the slots' files to Sigil's trash (`deadlock_core::trash::trash_slots`) and drop
/// their library entries; Undo restores both.
fn delete(st: &mut State, slots: Vec<u16>) {
    if crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    let Some(game_root) = writable_game(st) else {
        return;
    };
    let items: Vec<(u16, Option<ModMeta>)> = slots
        .iter()
        .map(|&slot| {
            let meta = st
                .addons
                .addons
                .iter()
                .find(|addon| addon.slot == slot)
                .and_then(|addon| meta_for(st, addon).cloned());
            (slot, meta)
        })
        .collect();
    if st.demo || rune::is_headless() {
        let (entries, errors) = trash::trash_slots(&game_root, &trash_dir(), &items, true);
        finish_delete(st, entries, errors);
        return;
    }
    // Moving several files, and dropping their library entries, is disk work: it runs on the
    // worker, never in a frame.
    let trash_root = trash_dir();
    let library = library_file();
    let wake = st.wake.clone();
    st.installed.trash_job.start(wake, move || {
        let (entries, errors) = trash::trash_slots(&game_root, &trash_root, &items, true);
        if !entries.is_empty() {
            let mut store = import::read_library(&library).unwrap_or_default();
            store
                .mods
                .retain(|meta| !entries.iter().any(|entry| entry.manifest.slot == meta.slot));
            let _ = import::write_library(&library, &store);
        }
        Ok((entries, errors))
    });
    st.status = format!(
        "Moving {} mod{} to the trash.",
        slots.len(),
        if slots.len() == 1 { "" } else { "s" }
    );
}

/// Take the worker's result: drop the library entries, keep the Undo and say what happened.
fn finish_delete(
    st: &mut State,
    entries: Vec<TrashEntry>,
    errors: Vec<(u16, String)>,
) {
    if !entries.is_empty() {
        st.installed.last_undo = Some(Undo::Delete(entries.iter().map(|e| e.dir.clone()).collect()));
        st.installed.undo_fresh = true;
    }
    for entry in &entries {
        st.installed.selected.remove(&entry.manifest.slot);
    }
    let n = entries.len();
    st.status = if errors.is_empty() {
        format!("Moved {n} mod{} to the trash.", if n == 1 { "" } else { "s" })
    } else {
        format!(
            "Moved {n} to the trash; {} failed ({}).",
            errors.len(),
            errors
                .iter()
                .map(|(slot, error)| format!("{slot:02}: {error}"))
                .collect::<Vec<_>>()
                .join("; ")
        )
    };
    rescan(st);
}

/// A change that can be put back shows its result as a toast with Undo, instead of the plain
/// status toast. `notify::poll` calls this after every page's poll, so a change marked late in
/// a frame still gets one toast (issue #4): the handler's status line with Undo, or the toast
/// it pushed itself with Undo added.
pub fn offer_undo(st: &mut State) -> bool {
    if !std::mem::take(&mut st.installed.undo_fresh) {
        return false;
    }
    let Some(undo) = st.installed.last_undo.clone() else { return false };
    let line = std::mem::take(&mut st.status);
    if line.trim().is_empty() {
        let again = undo.clone();
        if st.notices.undo_on_last_push(move |st: &mut State| apply_undo(st, again)) {
            return true;
        }
        let notice = crate::notify::Notice::success("Done").undo(move |st: &mut State| apply_undo(st, undo));
        crate::notify::push(st, notice);
        return true;
    }
    let notice = crate::notify::from_status::<State>(&line).undo(move |st: &mut State| apply_undo(st, undo));
    crate::notify::push(st, notice);
    true
}

pub fn apply_undo(st: &mut State, undo: Undo) {
    // An Undo renames or restores files in the addon folder like the change it reverses.
    if crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    let Some(game_root) = writable_game(st) else {
        return;
    };
    let library = library_file();
    let result: Result<String, String> = match undo {
        Undo::Delete(dirs) => {
            let mut back = 0;
            let mut errors = Vec::new();
            for dir in dirs {
                match trash::restore(&dir, true) {
                    Ok(report) => {
                        back += 1;
                        if let Some(meta) = report.meta {
                            let _ = import::record_meta(&library, meta);
                        }
                    }
                    Err(error) => errors.push(error.to_string()),
                }
            }
            if errors.is_empty() {
                Ok(format!("Put back {back} mod{}.", if back == 1 { "" } else { "s" }))
            } else {
                Err(format!("Put back {back}; {}", errors.join("; ")))
            }
        }
        Undo::Toggle(results) => deadlock_core::addons::revert_toggles(&game_root, &results, true)
            .map(|done| format!("Switched {} back.", done.len()))
            .map_err(|error| error.to_string()),
        Undo::Reorder(plan) => {
            let reverse = load_order::reverse_plan(&plan);
            load_order::apply_reorder(&reverse, true)
                .map(|report| {
                    let mut lib = import::read_library(&library).unwrap_or_default();
                    load_order::remap_library(&mut lib, &report.moves);
                    let _ = import::write_library(&library, &lib);
                    "Load order put back.".to_string()
                })
                .map_err(|error| error.to_string())
        }
        Undo::Unmerge(merged) => {
            tools::unmerge(st, &merged);
            return;
        }
        Undo::FontsInstalled(slot) => {
            let backups = crate::data::data_dir().join("backups");
            deadlock_core::fonts::remove_fonts(&game_root, slot, &backups, true)
                .map(|report| {
                    format!(
                        "Took {} font{} out again.",
                        report.removed.len(),
                        if report.removed.len() == 1 { "" } else { "s" }
                    )
                })
                .map_err(|error| error.to_string())
        }
        Undo::FontsRemoved(slot) => {
            let backups = crate::data::data_dir().join("backups");
            let file = st
                .addons
                .addons
                .iter()
                .find(|addon| addon.slot == slot)
                .map(|addon| addon.path(&addons_folder(st)));
            match file {
                Some(vpk) => deadlock_core::fonts::install_fonts(
                    &game_root, &vpk, slot, &backups, true,
                )
                .map(|report| {
                    format!(
                        "Put {} font{} back.",
                        report.installed.len(),
                        if report.installed.len() == 1 { "" } else { "s" }
                    )
                })
                .map_err(|error| error.to_string()),
                None => Err(format!("Slot {} is gone, so its fonts cannot come back.", deadlock_core::addons::slot_label(slot))),
            }
        }
        Undo::Update {
            slot,
            backups,
            meta,
        } => trash::undo_update(&game_root, &trash_dir(), slot, &backups, true)
            .map(|_| {
                if let Some(meta) = meta {
                    let _ = import::record_meta(&library, meta);
                }
                format!("Slot {} is back to the version before the update.", deadlock_core::addons::slot_label(slot))
            })
            .map_err(|error| error.to_string()),
    };
    st.status = match result {
        Ok(message) | Err(message) => message,
    };
    rescan(st);
}

fn undo_label(undo: &Undo) -> String {
    match undo {
        Undo::Delete(dirs) if dirs.len() == 1 => "Undo delete".to_string(),
        Undo::Delete(dirs) => format!("Undo delete of {}", dirs.len()),
        Undo::Toggle(_) => "Undo switch".to_string(),
        Undo::Reorder(_) => "Undo load order".to_string(),
        Undo::Update { .. } => "Undo update".to_string(),
        Undo::Unmerge(_) => "Undo merge".to_string(),
        Undo::FontsInstalled(_) => "Undo the fonts".to_string(),
        Undo::FontsRemoved(_) => "Put the fonts back".to_string(),
    }
}

// --- load order --------------------------------------------------------------------------

/// The slots of every mod that is on (or off), in load order.
fn group_slots(s: &State, enabled: bool) -> Vec<u16> {
    let mut slots: Vec<u16> = s
        .addons
        .addons
        .iter()
        .filter(|addon| addon.enabled == enabled)
        .map(|addon| addon.slot)
        .collect();
    slots.sort_unstable();
    slots
}

/// Plan the renames that give `group` the order `wanted`, then confirm. The demo plans
/// from the scan; a real folder is read again so the plan sees every file of each slot.
pub(super) fn ask_reorder(st: &mut State, group: &[u16], wanted: &[u16]) {
    if let Some(plan) = plan_order(st, group, wanted) {
        ask(st, Confirm::Reorder(plan));
    }
}

/// Plan and apply an order the player already confirmed (the load order dialog's Apply): the
/// same plan and write as [`ask_reorder`], without a second confirm.
pub(super) fn apply_order(st: &mut State, group: &[u16], wanted: &[u16]) {
    if let Some(plan) = plan_order(st, group, wanted) {
        reorder(st, plan);
    }
}

/// The renames that give `group` the order `wanted`, or `None` with the reason in the status
/// line (or nothing to do).
fn plan_order(st: &mut State, group: &[u16], wanted: &[u16]) -> Option<ReorderPlan> {
    plan_moves(st, load_order::moves_for_order(group, wanted))
}

/// Apply slot moves through the same plan, renames, library remap and Undo as a drag. The
/// requirement order fix uses it.
pub(super) fn apply_moves(st: &mut State, moves: Vec<load_order::SlotMove>) {
    if st.demo {
        st.status = "Demo library: nothing was changed.".to_string();
        return;
    }
    if let Some(plan) = plan_moves(st, moves) {
        reorder(st, plan);
    }
}

/// The renames for `moves`, or `None` with the reason in the status line (or nothing to do).
fn plan_moves(st: &mut State, moves: Vec<load_order::SlotMove>) -> Option<ReorderPlan> {
    if moves.is_empty() {
        return None;
    }
    let dir = addons_folder(st);
    let files = if st.demo {
        st.addons
            .addons
            .iter()
            .map(|addon| {
                let mut names = vec![addon.file.clone()];
                names.extend(addon.chunk_files.iter().cloned());
                (addon.slot, names)
            })
            .collect()
    } else {
        match load_order::read_slot_files(&dir) {
            Ok(files) => files,
            Err(error) => {
                st.status = error.to_string();
                return None;
            }
        }
    };
    match load_order::plan_reorder(&dir, &files, &moves, &load_order::new_token()) {
        Ok(plan) => Some(plan),
        Err(error) => {
            st.status = error.to_string();
            None
        }
    }
}

/// Apply a reorder plan (`deadlock_core::load_order::apply_reorder`) and move the library
/// entries with their slots.
fn reorder(st: &mut State, plan: ReorderPlan) {
    if crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    if writable_game(st).is_none() {
        return;
    }
    match load_order::apply_reorder(&plan, true) {
        Ok(report) => {
            let path = library_file();
            let mut library = import::read_library(&path).unwrap_or_default();
            load_order::remap_library(&mut library, &report.moves);
            let _ = import::write_library(&path, &library);
            let n = report.moves.len();
            st.status = format!("Load order changed for {n} mod{}.", if n == 1 { "" } else { "s" });
            st.installed.selected.clear();
            st.installed.updates.failed.clear();
            st.installed.last_undo = Some(Undo::Reorder(plan));
            st.installed.undo_fresh = true;
        }
        Err(error) => st.status = error.to_string(),
    }
    rescan(st);
}

// --- updates ---------------------------------------------------------------------------

/// The newer GameBanana file for a slot, when the last check found one.
pub fn update_for(s: &State, addon: &AddonRow) -> Option<RemoteFile> {
    let meta = meta_for(s, addon)?;
    let id = meta.gamebanana_id?;
    if crate::screens::browse::update_ignored(s, id) {
        return None;
    }
    let updates = &s.installed.updates;
    let remote = updates.cache.mods.get(&id)?;
    let file = match variant_slots(s, id).as_slice() {
        // Several files of this mod: each is matched to its own successor.
        siblings if siblings.len() > 1 => {
            let installed: Vec<update_match::Installed> = siblings
                .iter()
                .map(|other| {
                    let recorded = meta_for(s, other).map(|m| m.recorded_at.clone()).unwrap_or_default();
                    let base = mod_updates::baseline_for_repaired(&updates.journal, &updates.aliases, id, other.size_bytes, None, &recorded);
                    update_match::Installed {
                        slot: other.slot,
                        file_id: base.file_id,
                        installed_at: base.installed_at,
                    }
                })
                .collect();
            let match_remaining = crate::screens::settings::options::match_update_files();
            update_match::pair(&installed, remote, match_remaining).remove(&addon.slot)?
        }
        _ => {
            let baseline =
                mod_updates::baseline_for_repaired(&updates.journal, &updates.aliases, id, addon.size_bytes, None, &meta.recorded_at);
            match mod_updates::compare(baseline, remote) {
                UpdateState::Available(file) => file,
                _ => return None,
            }
        }
    };
    let ignored = notes::extra(s, addon).ignored_update;
    match ignored {
        Some(ignored) if ignored == 0 || ignored == file.file_id => None,
        _ => Some(file),
    }
}

/// The slots on disk that hold GameBanana mod `id`.
fn variant_slots<'a>(s: &'a State, id: u64) -> Vec<&'a AddonRow> {
    s.addons
        .addons
        .iter()
        .filter(|addon| meta_for(s, addon).and_then(|meta| meta.gamebanana_id) == Some(id))
        .collect()
}

/// Slots with an update, in load order, not already queued or running.
/// The notes and lists store (`mod_extras`), for the launch's random lists.
/// When a slot's file was installed: the newest journal entry for the slot at this size, else
/// the library entry's date. "Nov 17", with the year when it is not this year.
pub fn install_date(s: &State, addon: &AddonRow) -> Option<String> {
    // C16: slot and size, then size alone for a moved mod; a repaired file maps back first.
    let (bytes, _) = deadlock_core::compat_repair::original_of(&s.installed.updates.aliases, addon.size_bytes, None);
    let journal = deadlock_core::library_facts::install_date(&s.installed.updates.journal, addon.slot, bytes);
    let stamp = journal.or_else(|| meta_for(s, addon).map(|meta| meta.recorded_at.clone()))?;
    short_date(&stamp, this_year())
}

fn this_year() -> i64 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // Days to a civil year (Howard Hinnant's days_from_civil, inverted for the year only).
    let days = secs.div_euclid(86_400) + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    yoe + era * 400 + i64::from(month <= 2)
}

/// "Nov 17" from an RFC 3339 stamp, or "Nov 17, 2025" in another year.
pub fn short_date(stamp: &str, year_now: i64) -> Option<String> {
    let year: i64 = stamp.get(0..4)?.parse().ok()?;
    let month: usize = stamp.get(5..7)?.parse().ok()?;
    let day: u32 = stamp.get(8..10)?.parse().ok()?;
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let name = tr(MONTHS.get(month.checked_sub(1)?)?);
    Some(if year == year_now { format!("{name} {day}") } else { format!("{name} {day}, {year}") })
}

/// The picture a slot shows (its own image, else its GameBanana preview), for other pages:
/// the Locker draws the same picture on its rows.
pub fn preview_key_for_slot(s: &State, slot: u16) -> Option<String> {
    let addon = s.addons.addons.iter().find(|a| a.slot == slot)?;
    notes::preview_key(s, addon)
}

/// The library extras key of a slot, for scenes and other pages.
pub fn extras_key(s: &State, addon: &AddonRow) -> deadlock_core::mod_extras::SlotKey {
    notes::key_with_id(s, addon)
}

/// Open Installed's name and picture editor for a slot, from another page.
pub fn open_editor(st: &mut State, slot: u16) {
    notes::open_edit(st, slot);
}

/// GameBanana files this mod under Maps (its root or leaf category), from the library extras.
pub fn in_maps_category(s: &State, addon: &AddonRow) -> bool {
    let extra = notes::extra(s, addon);
    maps_category(extra.root_category.as_deref(), extra.category.as_deref())
}

pub fn maps_category(root: Option<&str>, leaf: Option<&str>) -> bool {
    [root, leaf]
        .into_iter()
        .flatten()
        .any(|name| matches!(name.trim().to_ascii_lowercase().as_str(), "maps" | "map"))
}

pub fn extras_path() -> PathBuf {
    notes::extras_file()
}

/// How many installed mods have an update waiting, for the status bar and Browse's strip.
pub fn pending_update_count(s: &State) -> usize {
    available_updates(s).len()
}

fn available_updates(s: &State) -> Vec<u16> {
    let updates = &s.installed.updates;
    s.addons
        .addons
        .iter()
        .filter(|addon| update_for(s, addon).is_some())
        .map(|addon| addon.slot)
        .filter(|slot| updates.running != Some(*slot) && !updates.queue.contains(slot))
        .collect()
}

fn update_all_button(s: &State, u: &Ui) -> Option<El> {
    let updates = &s.installed.updates;
    if let Some(slot) = updates.running {
        let left = updates.queue.len();
        let label = if update_cancelling(s) {
            tr("Cancelling").to_string()
        } else if left == 0 {
            tr("Updating").to_string()
        } else {
            fill(tr("Updating, {} more"), &[&left.to_string()])
        };
        let _ = slot;
        return Some(
            row()
                .gap(8.0)
                .center_y()
                .child(
                    u.icon_action(&icons::LOADER, &label, ActionKind::Secondary)
                        .key("installed-update-all")
                        .disabled(true),
                )
                .child(
                    u.action(if left == 0 { tr("Cancel") } else { tr("Cancel all") }, ActionKind::Ghost)
                        .key("installed-update-cancel")
                        .disabled(update_cancelling(s))
                        .cursor(rune::Cursor::Pointer)
                        .on_press(cancel_updates),
                ),
        );
    }
    let slots = available_updates(s);
    if slots.is_empty() {
        return None;
    }
    Some(
        u.icon_action(&icons::ARROW_UP, &format!("Update all ({})", slots.len()), ActionKind::Secondary)
            .key("installed-update-all")
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| ask(st, Confirm::UpdateAll(slots.clone()))),
    )
}

/// GameBanana ids of the slots on disk that have a library entry with one.
fn gamebanana_ids(s: &State) -> Vec<u64> {
    let mut ids: Vec<u64> = s
        .addons
        .addons
        .iter()
        .filter_map(|addon| meta_for(s, addon).and_then(|meta| meta.gamebanana_id))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Read the cache once; check GameBanana when it is six hours old, a mod is new to it, or
/// `force` (the menu's "Check for updates"). Never in the demo or a headless run.
pub(crate) fn check_updates(st: &mut State, force: bool) {
    if st.demo || rune::is_headless() || !st.addons.scanned {
        return;
    }
    let data = crate::data::data_dir();
    let workspace = crate::data::workspace_dir();
    if !st.installed.updates.loaded {
        st.installed.updates.cache = mod_updates::read_cache(&mod_updates::cache_path(&data));
        st.installed.updates.journal =
            deadlock_core::install::read_journal(&workspace).unwrap_or_default();
        st.installed.updates.aliases = repair::aliases();
        st.installed.updates.loaded = true;
    }
    let ids = gamebanana_ids(st);
    let updates = &mut st.installed.updates;
    if updates.check.is_running() || ids.is_empty() {
        return;
    }
    if !force && updates.checked.as_ref() == Some(&ids) {
        return;
    }
    let now = deadlock_core::gamebanana::now_unix();
    if !force && !mod_updates::is_due(&updates.cache, &ids, now) {
        updates.checked = Some(ids);
        return;
    }
    updates.checked = Some(ids.clone());
    let previous = updates.cache.clone();
    if force {
        st.status = "Checking for mod updates.".to_string();
    }
    st.installed.updates.check.start(st.wake.clone(), move || {
        let cache = mod_updates::check_all(&previous, &ids, now, mod_updates::fetch_remote_mod);
        let _ = mod_updates::write_cache(&mod_updates::cache_path(&data), &cache);
        let journal = deadlock_core::install::read_journal(&workspace).unwrap_or_default();
        Ok((cache, journal))
    });
}

/// Queue slots for update and start the first. Each one is a button press or the Update all
/// confirm.
pub(super) fn queue_updates(st: &mut State, slots: Vec<u16>) {
    if crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    if writable_game(st).is_none() {
        return;
    }
    for slot in slots {
        let updates = &mut st.installed.updates;
        if updates.running != Some(slot) && !updates.queue.contains(&slot) {
            updates.failed.remove(&slot);
            updates.queue.push(slot);
        }
    }
    start_next_update(st);
}

/// Reinstall (A4, Deadlock Mod Manager's Reinstall): the same file again into the same slot,
/// through the update queue's install path (`plan_install` with the slot and force, the old
/// files backed up, landing on or off as the slot was). The file comes from the install journal
/// entry for this slot and size; without one, the mod's newest file.
pub fn reinstall(st: &mut State, slot: u16) {
    st.installed.menu = None;
    if st.demo {
        st.status = tr("Demo library: nothing was changed.").to_string();
        return;
    }
    let Some(addon) = st.addons.addons.iter().find(|a| a.slot == slot).cloned() else { return };
    let Some(id) = meta_for(st, &addon).and_then(|meta| meta.gamebanana_id) else {
        crate::notify::push(
            st,
            crate::notify::Notice::warning(tr("This mod cannot be reinstalled"))
                .detail(tr("Sigil reinstalls mods it knows the GameBanana page of. Link it with Check this mod first.")),
        );
        return;
    };
    let journal = deadlock_core::install::read_journal(&crate::data::workspace_dir()).unwrap_or_default();
    let target = journal
        .iter()
        .rev()
        .find(|entry| entry.slot == slot && entry.vpk_bytes == addon.size_bytes)
        .and_then(|entry| entry.download_url.clone())
        .unwrap_or_else(|| id.to_string());
    st.installed.updates.targets.insert(slot, target);
    st.installed.updates.reinstall.insert(slot);
    queue_updates(st, vec![slot]);
}

/// Delete and reinstall (DMM's nuke): the slot's files go to the trash, then the same file
/// installs fresh into the empty slot. A failure puts the old files back; Undo too.
pub fn nuke_and_reinstall(st: &mut State, slot: u16) {
    if st.demo {
        st.installed.menu = None;
        st.status = tr("Demo library: nothing was changed.").to_string();
        return;
    }
    st.installed.updates.nuke.insert(slot);
    reinstall(st, slot);
    if !st.installed.updates.queue.contains(&slot) && st.installed.updates.running != Some(slot) {
        st.installed.updates.nuke.remove(&slot);
    }
}

/// Take one slot out of the update queue, or stop it while it downloads (A5). Once its files
/// are being written the update finishes: the install step itself is not cut in half.
pub fn cancel_update(st: &mut State, slot: u16) {
    let updates = &mut st.installed.updates;
    updates.queue.retain(|&queued| queued != slot);
    updates.targets.remove(&slot);
    if updates.running != Some(slot) {
        updates.reinstall.remove(&slot);
        updates.nuke.remove(&slot);
    }
    if updates.running == Some(slot) {
        updates.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Cancel the whole queue: the running update and every waiting one.
pub fn cancel_updates(st: &mut State) {
    let updates = &mut st.installed.updates;
    let waiting = std::mem::take(&mut updates.queue);
    for slot in waiting {
        updates.targets.remove(&slot);
    }
    if updates.running.is_some() {
        updates.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// The running update was asked to stop.
pub fn update_cancelling(s: &State) -> bool {
    s.installed.updates.running.is_some() && s.installed.updates.cancel.load(std::sync::atomic::Ordering::Relaxed)
}

/// Run the next queued update on a worker: the install pipeline into the same slot, with the
/// old files backed up (`plan_install` with the slot and force, then `apply_install_with`),
/// landing on or off as the slot was. The library entry keeps its name and preview.
fn start_next_update(st: &mut State) {
    if st.installed.updates.job.is_running() || st.installed.updates.queue.is_empty() {
        return;
    }
    let slot = st.installed.updates.queue.remove(0);
    let Some(addon) = st.addons.addons.iter().find(|addon| addon.slot == slot) else {
        return start_next_update(st);
    };
    let enabled = addon.enabled;
    let meta = meta_for(st, addon).cloned();
    let Some(id) = meta.as_ref().and_then(|meta| meta.gamebanana_id) else {
        return start_next_update(st);
    };
    let Some(game_root) = st.game_root.clone() else {
        st.installed.updates.queue.clear();
        return;
    };
    let name = meta.as_ref().map(|m| m.name.clone()).unwrap_or_else(|| format!("slot {}", deadlock_core::addons::slot_label(slot)));
    // A Replace from Browse names its archive; a matched variant its own file; else the newest.
    let target = match st.installed.updates.targets.remove(&slot) {
        Some(target) => target,
        None if variant_slots(st, id).len() > 1 => {
            // One file of several: only its matched successor, never the mod's newest file.
            match st.addons.addons.iter().find(|a| a.slot == slot).and_then(|a| update_for(st, a)) {
                Some(file) => format!("https://gamebanana.com/dl/{}", file.file_id),
                None => return start_next_update(st),
            }
        }
        None => id.to_string(),
    };
    let workspace = crate::data::workspace_dir();
    let library = library_file();
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    st.installed.updates.cancel = cancel.clone();
    st.installed.updates.running = Some(slot);
    let nuke = st.installed.updates.nuke.remove(&slot);
    let trash_root = trash_dir();
    st.status = format!("Updating {name}.");
    st.installed.updates.job.start(st.wake.clone(), move || {
        use deadlock_core::install::{
            ApplyOptions, InstallRequest, PlanOptions, apply_install_with, plan_install, read_journal,
            store_plan,
        };
        let fail = |error: deadlock_core::Error| format!("{slot:02}: {error}");
        // Delete and reinstall: the slot's files go to the trash first (the existing delete,
        // `trash::trash_slot` with approve); any failure below puts them back.
        let nuked = if nuke {
            Some(trash::trash_slot(&game_root, &trash_root, slot, meta.clone(), true).map_err(fail)?)
        } else {
            None
        };
        let put_back = |error: String| -> String {
            match &nuked {
                Some(entry) => match trash::restore(&entry.dir, true) {
                    Ok(_) => format!("{error} The old files are back."),
                    Err(again) => format!("{error} The old files are in Sigil's trash ({again})."),
                },
                None => error,
            }
        };
        let run = || -> Result<deadlock_core::install::InstallRecord, String> {
            let request = InstallRequest::new(target, workspace.clone())
                .map_err(fail)?
                .with_slot(Some(slot))
                .with_force(true);
            let plan = plan_install(
                &request,
                &PlanOptions {
                    game_root: Some(game_root.clone()),
                    cancel: Some(cancel.clone()),
                    ..Default::default()
                },
            )
            .map_err(fail)?;
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(format!("{}: cancelled. Nothing was written.", deadlock_core::addons::slot_label(slot)));
            }
            if plan.slot != slot {
                return Err(format!("{slot:02}: the plan picked slot {:02}. Nothing was written.", plan.slot));
            }
            store_plan(&plan).map_err(fail)?;
            apply_install_with(&plan, true, ApplyOptions { land_disabled: !enabled }).map_err(fail)
        };
        let record = run().map_err(put_back)?;
        // The Undo of a delete-and-reinstall puts the trashed files back over the new ones.
        let backups = match &nuked {
            Some(entry) => entry.manifest.files.iter().map(|name| entry.dir.join(name)).collect(),
            None => record.backups.clone(),
        };
        let bytes = std::fs::metadata(&record.target).map(|m| m.len()).unwrap_or(0);
        if let Some(old) = meta.clone() {
            let _ = import::record_meta(
                &library,
                ModMeta {
                    slot,
                    size_bytes: bytes,
                    recorded_at: String::new(),
                    ..old
                },
            );
        }
        Ok(UpdateDone {
            slot,
            name,
            backups,
            old_meta: meta,
            journal: read_journal(&workspace).unwrap_or_default(),
        })
    });
}

/// Results of the update check and the update queue, every frame the app polls.
fn poll_changes(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.installed.trash_job.try_take() {
        if let Ok((entries, errors)) = result {
            finish_delete(st, entries, errors);
        } else if let Err(error) = result {
            st.status = format!("The delete failed: {error}");
        }
        changed = true;
    }
    if st.addons.scanned && !st.installed.selected.is_empty() {
        let on_disk: BTreeSet<u16> = st.addons.addons.iter().map(|addon| addon.slot).collect();
        let before = st.installed.selected.len();
        st.installed.selected.retain(|slot| on_disk.contains(slot));
        changed |= before != st.installed.selected.len();
    }
    if st.screen == Screen::Installed {
        check_updates(st, false);
    }
    changed |= poll_changelog(st);
    if let Some(result) = st.installed.updates.check.try_take() {
        if let Ok((cache, journal)) = result {
            let n = cache.errors.len();
            st.installed.updates.cache = cache;
            st.installed.updates.journal = journal;
            st.installed.updates.aliases = repair::aliases();
            if n > 0 && st.screen == Screen::Installed {
                st.status = format!("{n} mod{} could not be checked for updates.", if n == 1 { "" } else { "s" });
            }
        }
        changed = true;
    }
    if let Some(result) = st.installed.updates.job.try_take() {
        let slot = st.installed.updates.running.take();
        match result {
            Ok(done) => {
                st.status = if st.installed.updates.reinstall.remove(&done.slot) {
                    fill(tr("Reinstalled {}. The old file is backed up."), &[&done.name])
                } else {
                    format!("Updated {}. The old file is backed up.", done.name)
                };
                st.installed.updates.journal = done.journal;
                repair::after_update(st, done.slot);
                st.installed.last_undo = Some(Undo::Update {
                    slot: done.slot,
                    backups: done.backups,
                    meta: done.old_meta,
                });
                st.installed.undo_fresh = true;
            }
            Err(_) if st.installed.updates.cancel.load(std::sync::atomic::Ordering::Relaxed) => {
                crate::notify::push(
                    st,
                    crate::notify::Notice::info(tr("Update cancelled")).detail(tr("Nothing was changed. The mod keeps its current version.")),
                );
            }
            Err(error) => {
                if let Some(slot) = slot {
                    st.installed.updates.failed.insert(slot, error.clone());
                }
                st.status = format!("Update failed: {error}");
            }
        }
        if let Some(slot) = slot {
            st.installed.updates.reinstall.remove(&slot);
        }
        rescan(st);
        start_next_update(st);
        changed = true;
    }
    changed
}

/// Changelogs for the mods in the open update confirm: read once each, three entries a mod.
fn poll_changelog(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(Ok(found)) = st.installed.updates.changelog_job.try_take() {
        for (id, log) in found {
            st.installed.updates.changelog.insert(id, log);
        }
        changed = true;
    }
    let Some(Confirm::UpdateAll(slots)) = st.installed.confirm.as_ref() else {
        return changed;
    };
    if st.demo || rune::is_headless() || st.prefs.offline || st.installed.updates.changelog_job.is_running() {
        return changed;
    }
    let mut ids: Vec<u64> = slots
        .iter()
        .filter_map(|slot| st.addons.addons.iter().find(|a| a.slot == *slot))
        .filter_map(|addon| meta_for(st, addon).and_then(|meta| meta.gamebanana_id))
        .filter(|id| !st.installed.updates.changelog.contains_key(id))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids.truncate(6);
    if ids.is_empty() {
        return changed;
    }
    for id in &ids {
        st.installed.updates.changelog.insert(*id, Ok(Vec::new()));
    }
    st.installed.updates.changelog_job.start(st.wake.clone(), move || {
        Ok(ids
            .into_iter()
            .map(|id| {
                let log = deadlock_core::gamebanana::fetch_updates(deadlock_core::gamebanana::BrowseSection::Mods, id, 3)
                    .map(|page| page.items)
                    .map_err(|e| e.to_string());
                (id, log)
            })
            .collect())
    });
    changed
}

/// The newest changelog entry of a mod, as two short lines for the update confirm.
fn changelog_lines(s: &State, u: &Ui, slot: u16) -> Option<El> {
    let id = s.addons.addons.iter().find(|a| a.slot == slot).and_then(|a| meta_for(s, a)).and_then(|m| m.gamebanana_id)?;
    let entry = s.installed.updates.changelog.get(&id)?.as_ref().ok()?.first()?;
    let mut head = entry.title.clone();
    if let Some(version) = entry.version.as_ref().filter(|v| !v.is_empty()) {
        head = format!("{head}  ·  {version}");
    }
    let mut c = col().w_full().pl(32.0).gap(2.0).child(text(head).font(SMALL).color(u.prose).truncate());
    let lines: Vec<String> = if entry.changes.is_empty() {
        entry.text.lines().map(str::trim).filter(|l| !l.is_empty()).take(2).map(str::to_string).collect()
    } else {
        entry.changes.iter().take(3).map(|(label, line)| if label.is_empty() { line.clone() } else { format!("{label}: {line}") }).collect()
    };
    for line in lines {
        c = c.child(text(line).font(SMALL).color(u.faint).truncate());
    }
    Some(c)
}

// --- trash -------------------------------------------------------------------------------

fn open_trash(st: &mut State) {
    if !st.demo {
        st.installed.trash.entries = trash::list_trash(&trash_dir());
    }
    st.installed.trash.confirm_empty = false;
    st.installed.trash.open = true;
}

fn trash_dialog(s: &State, u: &Ui) -> El {
    let entries = &s.installed.trash.entries;
    let close = |st: &mut State| {
        st.installed.trash.open = false;
        st.installed.trash.confirm_empty = false;
    };
    let mut list = col().w_full();
    if entries.is_empty() {
        list = list.child(text("Nothing in the trash.").font(BODY).color(u.faint));
    }
    for entry in entries.iter().take(8) {
        let manifest = &entry.manifest;
        let name = if manifest.loose {
            tr("Leftover files").to_string()
        } else {
            manifest
                .meta
                .as_ref()
                .map(|meta| meta.name.clone())
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| format!("pak{:02}_dir.vpk", manifest.slot))
        };
        let detail = if manifest.loose {
            format!(
                "{}  ·  {}",
                crate::model::format_size(manifest.bytes),
                manifest.trashed_at.get(..10).unwrap_or("")
            )
        } else {
            format!(
                "Slot {:02}  ·  {}  ·  {}",
                manifest.slot,
                crate::model::format_size(manifest.bytes),
                manifest.trashed_at.get(..10).unwrap_or("")
            )
        };
        let dir = entry.dir.clone();
        list = list.child(
            row()
                .key(format!("installed-trash-{}", entry.id))
                .w_full()
                .h(52.0)
                .gap(12.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(text(name).font(BODY_MED).color(u.ink).truncate())
                        .child(text(detail).font(SMALL).color(u.faint).truncate()),
                )
                .child(
                    u.action("Restore", ActionKind::Secondary)
                        .key(format!("installed-restore-{}", entry.id))
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| {
                            apply_undo(st, Undo::Delete(vec![dir.clone()]));
                            if !st.demo {
                                st.installed.trash.entries = trash::list_trash(&trash_dir());
                            }
                        }),
                ),
        );
    }
    if entries.len() > 8 {
        list = list.child(
            text(format!("and {} more", entries.len() - 8))
                .font(SMALL)
                .color(u.faint)
                .pt(8.0),
        );
    }
    let bytes: u64 = entries.iter().map(|e| e.manifest.bytes).sum();
    let confirm_empty = s.installed.trash.confirm_empty;
    let mut footer = row().w_full().gap(8.0).center_y();
    if !entries.is_empty() {
        footer = footer.child(
            u.action(
                if confirm_empty { "Delete for good" } else { "Empty trash" },
                ActionKind::Danger,
            )
            .key("installed-empty-trash")
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| {
                if !st.installed.trash.confirm_empty {
                    st.installed.trash.confirm_empty = true;
                    return;
                }
                st.installed.trash.confirm_empty = false;
                if st.demo {
                    st.status = "Demo library: nothing was changed.".to_string();
                    return;
                }
                match trash::empty_trash(&trash_dir(), true) {
                    Ok(report) => {
                        st.status = format!(
                            "Deleted {} mod{} for good, {}.",
                            report.entries,
                            if report.entries == 1 { "" } else { "s" },
                            crate::model::format_size(report.bytes)
                        );
                        // A delete in the trash can no longer be undone.
                        if matches!(st.installed.last_undo, Some(Undo::Delete(_))) {
                            st.installed.last_undo = None;
                        }
                    }
                    Err(error) => st.status = error.to_string(),
                }
                st.installed.trash.entries = trash::list_trash(&trash_dir());
            }),
        );
        if confirm_empty {
            footer = footer.child(
                text(format!("{} can't be restored after this.", crate::model::format_size(bytes)))
                    .font(SMALL)
                    .color(u.muted),
            );
        }
    }
    footer = footer.child(spacer()).child(
        u.action("Done", ActionKind::Ghost)
            .key("installed-trash-done")
            .cursor(rune::Cursor::Pointer)
            .on_press(close),
    );
    u.dialog_shell()
        .key("installed-trash-dialog")
        .role("dialog")
        .label("Trash")
        .w(500.0)
        .padding(24.0)
        .gap(16.0)
        .modal(true, close)
        .child(text("Trash").font(H1).color(u.ink))
        .child(
            text("Deleted mods wait here. Restore puts one back in its slot.")
                .font(BODY)
                .color(u.muted)
                .wrap(),
        )
        .child(list)
        .child(footer)
}

// --- confirms ------------------------------------------------------------------------------

fn confirm_dialog(s: &State, u: &Ui, confirm: &Confirm) -> El {
    let close = |st: &mut State| st.installed.confirm = None;
    let name_of = |slot: u16| {
        s.addons
            .addons
            .iter()
            .find(|addon| addon.slot == slot)
            .map(|addon| display_name(s, addon))
            .unwrap_or_else(|| format!("Slot {}", deadlock_core::addons::slot_label(slot)))
    };
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    let (key, title, body, list, label, kind): (&str, String, &str, El, String, ActionKind) =
        match confirm {
            Confirm::Delete(slots) => {
                let title = match slots.as_slice() {
                    [one] => format!("Delete {}?", name_of(*one)),
                    many => format!("Delete {} mods?", many.len()),
                };
                let mut list = col().w_full().gap(8.0);
                for &slot in slots.iter().take(6) {
                    let files = s
                        .addons
                        .addons
                        .iter()
                        .find(|addon| addon.slot == slot)
                        .map(|addon| {
                            let mut names = vec![addon.file.clone()];
                            names.extend(addon.chunk_files.iter().cloned());
                            names.join(", ")
                        })
                        .unwrap_or_default();
                    list = list.child(
                        row()
                            .w_full()
                            .gap(12.0)
                            .items_start()
                            .child(text(deadlock_core::addons::slot_label(slot)).font(MICRO).color(u.faint).w(28.0).pt(3.0))
                            .child(
                                col()
                                    .grow(1.0)
                                    .min_w(0.0)
                                    .gap(2.0)
                                    .child(text(name_of(slot)).font(BODY).color(u.prose).truncate())
                                    .child(text(files).font(SMALL).color(u.faint).truncate()),
                            ),
                    );
                }
                if slots.len() > 6 {
                    list = list.child(text(format!("and {} more", slots.len() - 6)).font(SMALL).color(u.faint));
                }
                (
                    "installed-delete-dialog",
                    title,
                    "The files move to Sigil's trash. You can restore them until you empty it.",
                    list,
                    "Delete".to_string(),
                    ActionKind::Danger,
                )
            }
            Confirm::Reorder(plan) => {
                let mut list = col().w_full().gap(4.0);
                // In the new order: the mod that takes the lowest number first.
                let mut moves = plan.moves.clone();
                moves.sort_by_key(|m| m.to);
                for m in moves.iter().take(8) {
                    list = list.child(
                        row()
                            .w_full()
                            .h(28.0)
                            .gap(8.0)
                            .center_y()
                            .child(text(name_of(m.from)).font(BODY).color(u.prose).truncate().grow(1.0).min_w(0.0))
                            .child(text(deadlock_core::addons::slot_label(m.from)).font(MICRO).color(u.faint))
                            .child(icon(&ARROW_RIGHT, 12.0).color(u.faint).a11y_hidden())
                            .child(text(deadlock_core::addons::slot_label(m.to)).font(MICRO).color(u.ink)),
                    );
                }
                if plan.moves.len() > 8 {
                    list = list.child(text(format!("and {} more", plan.moves.len() - 8)).font(SMALL).color(u.faint));
                }
                (
                    "installed-reorder-dialog",
                    "Change the load order?".to_string(),
                    "Lower numbers load first and win when two mods change the same file.",
                    list,
                    "Apply".to_string(),
                    ActionKind::Primary,
                )
            }
            Confirm::UpdateAll(all) => {
                let slots = &scoped(s, all);
                let mut list = col().w_full().gap(4.0);
                // DMM's Only enabled / Only disabled.
                let mut scope_row = row().w_full().gap(6.0).pb(4.0);
                for (scope, words) in [
                    (UpdateScope::All, tr("All")),
                    (UpdateScope::Enabled, tr("Only enabled")),
                    (UpdateScope::Disabled, tr("Only disabled")),
                ] {
                    let on = s.installed.update_scope == scope;
                    scope_row = scope_row.child(
                        u.action(words, if on { ActionKind::Secondary } else { ActionKind::Ghost })
                            .key(format!("update-scope-{scope:?}"))
                            .cursor(rune::Cursor::Pointer)
                            .on_press(move |st: &mut State| st.installed.update_scope = scope),
                    );
                }
                list = list.child(scope_row);
                for &slot in slots.iter().take(6) {
                    list = list.child(
                        row()
                            .w_full()
                            .h(28.0)
                            .gap(12.0)
                            .center_y()
                            .child(text(deadlock_core::addons::slot_label(slot)).font(MICRO).color(u.faint).w(28.0))
                            .child(text(name_of(slot)).font(BODY).color(u.prose).truncate()),
                    );
                    // What changed, from the mod's GameBanana changelog.
                    if let Some(log) = changelog_lines(s, u, slot) {
                        list = list.child(log.pb(6.0));
                    }
                }
                if slots.len() > 6 {
                    list = list.child(text(format!("and {} more", slots.len() - 6)).font(SMALL).color(u.faint));
                }
                (
                    "installed-update-dialog",
                    format!("Update {} mod{}?", slots.len(), plural(slots.len())),
                    "Each one downloads again into its own slot. The old files are backed up first, and on or off stays as it is.",
                    list,
                    "Update".to_string(),
                    ActionKind::Primary,
                )
            }
        };
    let confirm = confirm.clone();
    let id = confirm.id();
    u.dialog_shell()
        .key(key)
        .role("dialog")
        .label(title.clone())
        .w(460.0)
        .padding(24.0)
        .gap(16.0)
        .modal(true, close)
        .child(text(title).font(H1).color(u.ink).wrap())
        .child(text(body).font(BODY).color(u.muted).wrap())
        .child(list)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(dont_ask_row(s, u))
                .child(spacer())
                .child(
                    u.action("Cancel", ActionKind::Ghost)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(close),
                )
                .child(
                    u.action(&label, kind)
                        .key("installed-confirm")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| {
                            remember_dont_ask(st, id);
                            run_confirm(st, confirm.clone());
                        }),
                ),
        )
}

// --- scenes ---------------------------------------------------------------------------------

/// Three cards picked, one hovered.
pub fn scene_select(s: &mut State) {
    crate::app::scene_demo_installed(s);
    s.installed.selected = [2, 3, 9].into_iter().collect();
    s.installed.anchor = Some(9);
    s.installed.hovered = Some(4);
}

/// The delete confirm for two picked mods.
pub fn scene_delete(s: &mut State) {
    crate::app::scene_demo_installed(s);
    s.installed.selected = [3, 9].into_iter().collect();
    s.installed.confirm = Some(Confirm::Delete(vec![3, 9]));
}

/// "Load first" on slot 05: the reorder confirm.
pub fn scene_reorder(s: &mut State) {
    crate::app::scene_demo_installed(s);
    let group = group_slots(s, true);
    let wanted = load_order::move_to_index(&group, 5, 0);
    ask_reorder(s, &group, &wanted);
}

/// Two mods with an update on GameBanana, one updating.
pub fn scene_updates(s: &mut State) {
    crate::app::scene_demo_installed(s);
    stage_updates(s);
    s.installed.updates.running = Some(4);
}

/// The Update all confirm.
pub fn scene_update_all(s: &mut State) {
    crate::app::scene_demo_installed(s);
    stage_updates(s);
    let slots = available_updates(s);
    s.installed.confirm = Some(Confirm::UpdateAll(slots));
}

/// The card menu with update and load order items.
pub fn scene_card_menu(s: &mut State) {
    crate::app::scene_demo_installed(s);
    stage_updates(s);
    s.installed.menu = Some(Menu::Card(2));
    s.installed.hovered = Some(2);
}

/// The trash with two entries.
pub fn scene_trash(s: &mut State) {
    crate::app::scene_demo_installed(s);
    let addons = PathBuf::from("/Deadlock/game/citadel/addons");
    s.installed.trash.entries = [(6u16, "Viscous Goo Recolor", 7_340_032u64), (14, "", 1_204_224)]
        .into_iter()
        .map(|(slot, name, bytes)| TrashEntry {
            id: format!("20261117-101500-000-pak{slot:02}"),
            dir: PathBuf::from(format!("/trash/{slot}")),
            manifest: trash::TrashManifest {
                schema: trash::TRASH_SCHEMA,
                slot,
                addons_dir: addons.clone(),
                files: vec![format!("pak{slot:02}_dir.vpk")],
                bytes,
                trashed_at: "2026-11-17T10:15:00.000Z".to_string(),
                enabled: true,
                meta: (!name.is_empty()).then(|| ModMeta {
                    slot,
                    name: name.to_string(),
                    ..Default::default()
                }),
                loose: false,
            },
        })
        .collect();
    s.installed.trash.open = true;
}

/// Updates on GameBanana for slots 02, 04 and 07 of the demo library.
/// The update fixture, for scenes outside this page (Browse's quick stats strip).
pub fn stage_updates_for_scene(s: &mut State) {
    stage_updates(s);
}

fn stage_updates(s: &mut State) {
    let now = deadlock_core::gamebanana::now_unix();
    let mut cache = UpdateCache {
        checked_at: now,
        ..UpdateCache::default()
    };
    for meta in &mut s.installed.library.mods {
        let id = meta.gamebanana_id.unwrap_or(0);
        meta.recorded_at = "2026-10-01T00:00:00.000Z".to_string();
        let added = if [2, 4, 7].contains(&meta.slot) { now - 86_400 } else { 1_700_000_000 };
        cache.mods.insert(
            id,
            mod_updates::RemoteMod {
                files: vec![RemoteFile {
                    file_id: id * 10,
                    file_name: format!("mod-{id}.zip"),
                    added,
                    archived: false,
                }],
            },
        );
    }
    s.installed.updates.checked = Some(gamebanana_ids(s));
    s.installed.updates.cache = cache;
    s.installed.updates.loaded = true;
}

#[cfg(test)]
mod w4a_tests {
    use super::*;

    #[test]
    fn install_dates_read_short_with_the_year_only_when_it_differs() {
        assert_eq!(short_date("2026-11-17T21:04:11Z", 2026).as_deref(), Some("Nov 17"));
        assert_eq!(short_date("2025-03-02T09:00:00Z", 2026).as_deref(), Some("Mar 2, 2025"));
        assert_eq!(short_date("", 2026), None);
        assert_eq!(short_date("2026-13-01", 2026), None);
        assert!(this_year() >= 2026);
    }

    #[test]
    fn hero_sort_is_listed() {
        assert!(Sort::ALL.contains(&Sort::Hero));
    }
}
