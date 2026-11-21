//! Repair the mods folder, and mods installed twice.
//!
//! - Deadlock Mod Manager (unmerged, 2026-11): a "Repair mod folder" tool that moves files it did
//!   not install to a backup and cleans up unfinished changes; mods whose VPKs were deleted
//!   outside the app are dropped from the library; VPK fingerprints that leave the manager's
//!   own stamp out, so a copied file is told apart from the same download.
//! - Grimoire: an orphan cleanup for files no mod owns.
//! - Sigil: Installed > More > Repair the mods folder reads `folder_repair::review` on a worker
//!   and lists each problem in plain words, ticked when the fix is safe: missing parts, files
//!   that cannot be read, leftover parts, a `.disabled` copy beside the enabled one, library
//!   entries whose files are gone, VPKs the game ignores (wrong name, subfolder, a Sigil folder
//!   out of range), an extra folder `gameinfo.gi` stopped mounting, a stopped reorder, and the
//!   same mod installed twice (`duplicates`). Fix runs `folder_repair::apply` on a worker:
//!   removals go to Sigil's trash, strays move into a free slot switched off, and one Undo puts
//!   it all back. A worker also prints every slot while the page is open, so a copy wears an
//!   "Installed twice" chip and its card menu offers Remove this copy (the normal delete, to the
//!   trash with Undo), and a toast says so once per session.
//!
//! Game writes run only on the player's press, and never while the game runs when the block is
//! on.

use std::collections::{BTreeMap, BTreeSet};

use deadlock_core::folder_repair::{self, Finding, Fix, Kind, Report, Review};
use rune::{El, col, icon, row, spacer, text};

use crate::app::State;
use crate::jobs::Job;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{ActionKind, BODY, BODY_MED, H1, H2, MICRO, MONO, SMALL, Ui};

#[derive(Default)]
pub struct FolderFix {
    pub open: bool,
    pub job: Job<Review>,
    pub review: Option<Result<Review, String>>,
    /// Finding ids the player ticked.
    pub ticked: BTreeSet<String>,
    pub apply_job: Job<Report>,
    pub undo_job: Job<folder_repair::UndoReport>,
    /// List memberships to move from a removed copy to the one kept: `(finding id, from uid,
    /// to uid)`.
    carry: Vec<(String, String, String)>,
    /// Copies found by the page's worker: extra slot to the slot kept.
    pub copies: BTreeMap<u16, u16>,
    copies_job: Job<BTreeMap<u16, u16>>,
    copies_sig: Option<Vec<(u16, u64, bool)>>,
    /// Extra slots the toast already named this session.
    told: BTreeSet<u16>,
}

fn library() -> deadlock_core::import::Library {
    deadlock_core::import::read_library(&super::library_file()).unwrap_or_default()
}

/// Open the review and read the folders on a worker.
pub fn open(st: &mut State) {
    st.installed.menu = None;
    st.installed.folder_fix.open = true;
    start(st);
}

fn start(st: &mut State) {
    if st.demo || rune::is_headless() || st.installed.folder_fix.job.is_running() {
        return;
    }
    let dir = super::addons_folder(st);
    st.installed.folder_fix.review = None;
    st.installed.folder_fix.job.start(st.wake.clone(), move || {
        folder_repair::review(&dir, &library()).map_err(|e| e.to_string())
    });
}

fn close(st: &mut State) {
    st.installed.folder_fix.open = false;
}

/// Take finished work, and print the slots again when the scan changed. Called from Installed's
/// poll.
pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.installed.folder_fix.job.try_take() {
        if let Ok(review) = &result {
            st.installed.folder_fix.ticked =
                review.findings.iter().filter(|f| f.suggested && f.fix != Fix::None).map(|f| f.id.clone()).collect();
        }
        st.installed.folder_fix.review = Some(result);
        changed = true;
    }
    if let Some(result) = st.installed.folder_fix.apply_job.try_take() {
        finish(st, result);
        changed = true;
    }
    if let Some(result) = st.installed.folder_fix.undo_job.try_take() {
        use crate::notify::{self, Notice};
        match result {
            Ok(done) if done.skipped.is_empty() => notify::push(st, Notice::success(tr("The repair was undone"))),
            Ok(done) => notify::push(
                st,
                Notice::warning(tr("Part of the repair was undone")).detail(fill(tr("Left as they are: {}"), &[&done.skipped.join("; ")])),
            ),
            Err(error) => notify::push(st, Notice::error(tr("Could not undo the repair")).detail(error)),
        }
        super::rescan(st);
        changed = true;
    }
    changed |= poll_copies(st);
    changed
}

/// The page's duplicate scan: one tree print per slot, on a worker, when the slots change.
fn poll_copies(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.installed.folder_fix.copies_job.try_take() {
        if let Ok(copies) = result {
            st.installed.folder_fix.copies = copies;
            tell_copies(st);
        }
        changed = true;
    }
    if st.demo || rune::is_headless() || !st.addons.scanned || st.installed.folder_fix.copies_job.is_running() {
        return changed;
    }
    let sig: Vec<(u16, u64, bool)> = st.addons.addons.iter().map(|a| (a.slot, a.size_bytes, a.enabled)).collect();
    if st.installed.folder_fix.copies_sig.as_ref() == Some(&sig) {
        return changed;
    }
    st.installed.folder_fix.copies_sig = Some(sig);
    let dir = super::addons_folder(st);
    let known: BTreeSet<u16> = st.installed.library.mods.iter().map(|m| m.slot).collect();
    st.installed.folder_fix.copies_job.start(st.wake.clone(), move || {
        let prints = deadlock_core::duplicates::scan(&dir).map_err(|e| e.to_string())?;
        Ok(copies_of(&prints, &known))
    });
    changed
}

/// Extra slot to kept slot, from the printed slots.
pub fn copies_of(prints: &[(u16, bool, deadlock_core::duplicates::ContentPrint, u64)], known: &BTreeSet<u16>) -> BTreeMap<u16, u16> {
    let copies: Vec<deadlock_core::duplicates::SlotCopy> = prints
        .iter()
        .map(|(slot, enabled, print, bytes)| deadlock_core::duplicates::SlotCopy {
            slot: *slot,
            enabled: *enabled,
            known: known.contains(slot),
            print: print.hash.clone(),
            disk_bytes: *bytes,
        })
        .collect();
    let mut out = BTreeMap::new();
    for set in deadlock_core::duplicates::find_duplicates(&copies) {
        for extra in set.extra {
            out.insert(extra, set.keep);
        }
    }
    out
}

/// One toast per session for copies it has not named yet.
fn tell_copies(st: &mut State) {
    let fresh: Vec<u16> = st.installed.folder_fix.copies.keys().copied().filter(|s| !st.installed.folder_fix.told.contains(s)).collect();
    if fresh.is_empty() {
        return;
    }
    st.installed.folder_fix.told.extend(fresh.iter().copied());
    let title = if fresh.len() == 1 {
        tr("A mod is installed twice").to_string()
    } else {
        fill(tr("{} mods are installed twice"), &[&fresh.len().to_string()])
    };
    let names: Vec<String> = fresh.iter().map(|slot| slot_name(st, *slot)).collect();
    crate::notify::push(
        st,
        crate::notify::Notice::info(title).detail(names.join(", ")).action(tr("Review"), |st: &mut State| {
            st.go(crate::model::Screen::Installed);
            open(st);
        }),
    );
}

/// The slot an extra copy duplicates, for the card chip and menu.
pub fn copy_of(s: &State, slot: u16) -> Option<u16> {
    s.installed.folder_fix.copies.get(&slot).copied()
}

/// The card chip of an extra copy, styled like the merged chip.
pub fn chip(s: &State, u: &Ui, slot: u16) -> Option<El> {
    copy_of(s, slot)?;
    Some(
        row()
            .h(20.0)
            .pl(5.0)
            .pr(7.0)
            .gap(4.0)
            .center_y()
            .radius(crate::ui::r(6.0))
            .fill(u.raised.alpha(0.92))
            .border(1.0, u.hair)
            .a11y_hidden()
            .child(icon(&crate::icons::COPY, 12.0).color(u.muted).a11y_hidden())
            .child(text(tr("Installed twice")).font(MICRO).color(u.prose).a11y_hidden()),
    )
}

/// The card menu item: remove this copy through the normal delete (trash, Undo).
pub fn menu_item(s: &State, u: &Ui, slot: u16) -> Option<El> {
    let keep = copy_of(s, slot)?;
    let detail = fill(tr("same as slot {}"), &[&deadlock_core::addons::slot_label(keep)]);
    Some(
        super::menu_item(u, "menu-remove-copy", Some(&crate::icons::TRASH_2), tr("Remove this copy"), &detail, true).on_press(move |st: &mut State| {
            st.installed.menu = None;
            super::ask(st, super::Confirm::Delete(vec![slot]));
        }),
    )
}

/// The More menu line under the item, when copies are known.
pub fn menu_detail(s: &State) -> String {
    match s.installed.folder_fix.copies.len() {
        0 => tr("broken slots and strays").to_string(),
        n => fill(tr("{} installed twice"), &[&n.to_string()]),
    }
}

fn slot_name(s: &State, slot: u16) -> String {
    if let Some(addon) = s.addons.addons.iter().find(|a| a.slot == slot) {
        return super::display_name(s, addon);
    }
    s.installed
        .library
        .mods
        .iter()
        .rev()
        .find(|m| m.slot == slot)
        .map(|m| m.name.clone())
        .unwrap_or_else(|| fill(tr("Slot {}"), &[&deadlock_core::addons::slot_label(slot)]))
}

/// GAME WRITE, from the Fix press: the ticked findings, on a worker.
fn apply(st: &mut State) {
    if crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    let Some(game_root) = super::writable_game(st) else { return };
    let Some(Ok(review)) = &st.installed.folder_fix.review else { return };
    let reviewed = review.findings.clone();
    let chosen = st.installed.folder_fix.ticked.clone();
    if chosen.is_empty() || st.installed.folder_fix.apply_job.is_running() {
        return;
    }
    // A removed copy hands its list memberships to the copy kept.
    let uid = |st: &State, slot: u16| {
        st.addons.addons.iter().find(|a| a.slot == slot).map(|a| super::notes::extra(st, a).uid).filter(|u| !u.is_empty())
    };
    let mut carry = Vec::new();
    for finding in reviewed.iter().filter(|f| chosen.contains(&f.id)) {
        if let (Kind::Duplicate { keep }, Some(slot)) = (&finding.kind, finding.slot)
            && let (Some(from), Some(to)) = (uid(st, slot), uid(st, *keep))
        {
            carry.push((finding.id.clone(), from, to));
        }
    }
    st.installed.folder_fix.carry = carry;
    let trash = super::trash_dir();
    let library = super::library_file();
    st.installed.folder_fix.apply_job.start(st.wake.clone(), move || {
        folder_repair::apply(&game_root, &trash, &library, &reviewed, &chosen, true).map_err(|e| e.to_string())
    });
}

fn finish(st: &mut State, result: Result<Report, String>) {
    use crate::notify::{self, Notice};
    let report = match result {
        Ok(report) => report,
        Err(error) => {
            notify::push(st, Notice::error(tr("Could not repair the mods folder")).detail(error));
            return;
        }
    };
    let carry = std::mem::take(&mut st.installed.folder_fix.carry);
    let moves: Vec<(String, String)> =
        carry.into_iter().filter(|(id, _, _)| report.fixed.contains(id)).map(|(_, from, to)| (from, to)).collect();
    if !moves.is_empty() && !st.demo {
        let path = super::notes::extras_file();
        let _ = deadlock_core::mod_extras::edit_extras(&path, |x| carry_members(x, &moves));
        carry_members(&mut st.installed.notes.extras, &moves);
    }
    let fixed = report.fixed.len();
    let title = if fixed == 1 { tr("Fixed 1 problem").to_string() } else { fill(tr("Fixed {} problems"), &[&fixed.to_string()]) };
    let mut detail = Vec::new();
    if !report.trashed.is_empty() {
        detail.push(fill(tr("{} moved to the trash"), &[&report.trashed.len().to_string()]));
    }
    if !report.adopted.is_empty() {
        detail.push(fill(tr("{} moved into a free slot, switched off"), &[&report.adopted.len().to_string()]));
    }
    if !report.forgotten.is_empty() {
        detail.push(fill(tr("{} gone mods removed from the library"), &[&report.forgotten.len().to_string()]));
    }
    for (_, error) in &report.errors {
        detail.push(error.clone());
    }
    let mut notice = if report.errors.is_empty() { Notice::success(title) } else { Notice::warning(title) };
    if !detail.is_empty() {
        notice = notice.detail(detail.join(". "));
    }
    if fixed > 0 {
        let game_root = st.game_root.clone();
        notice = notice.undo(move |st: &mut State| {
            if crate::screens::settings::watch::refuse_while_running(st) {
                return;
            }
            let Some(game_root) = game_root.clone() else { return };
            let library = super::library_file();
            let report = report.clone();
            st.installed.folder_fix.undo_job.start(st.wake.clone(), move || {
                folder_repair::undo(&game_root, &library, &report, true).map_err(|e| e.to_string())
            });
        });
    }
    notify::push(st, notice);
    st.installed.notes.sig = None;
    super::rescan(st);
    start(st);
}

/// Put `to` wherever a list held `from`, once.
pub fn carry_members(extras: &mut deadlock_core::mod_extras::Extras, moves: &[(String, String)]) {
    for list in &mut extras.lists {
        for (from, to) in moves {
            if let Some(at) = list.members.iter().position(|m| m == from) {
                if list.members.contains(to) {
                    list.members.remove(at);
                } else {
                    list.members[at] = to.clone();
                }
            }
        }
    }
}

// --- the dialog ---------------------------------------------------------------------------

/// The section a finding is listed under.
fn section(kind: &Kind) -> usize {
    match kind {
        Kind::MissingChunks { .. } | Kind::Unreadable { .. } => 0,
        Kind::Duplicate { .. } | Kind::Doubled { same: true } => 1,
        Kind::Misplaced { .. } | Kind::Unmounted { .. } | Kind::Doubled { same: false } => 2,
        Kind::OrphanChunks { .. } | Kind::StoppedReorder { .. } | Kind::LibraryGone { .. } => 3,
    }
}

const SECTIONS: [&str; 4] = ["Broken mods", "Installed twice", "Not loaded", "Leftovers"];

/// The two lines a finding reads as: what is wrong, and what Fix does.
pub fn words(s: &State, finding: &Finding) -> (String, String) {
    let name = finding.slot.map(|slot| slot_name(s, slot)).unwrap_or_default();
    let label = finding.slot.map(deadlock_core::addons::slot_label).unwrap_or_default();
    match &finding.kind {
        Kind::MissingChunks { chunks } => (
            if chunks.len() == 1 {
                fill(tr("{}: 1 part is missing"), &[&name])
            } else {
                fill(tr("{}: {} parts are missing"), &[&name, &chunks.len().to_string()])
            },
            tr("The game loads it partly or not at all. Fix moves it to the trash; Reinstall gets a clean copy.").to_string(),
        ),
        Kind::Unreadable { error } => (
            fill(tr("{}: the file cannot be read"), &[&name]),
            fill(tr("Often a download that stopped halfway ({}). Fix moves it to the trash."), &[error]),
        ),
        Kind::OrphanChunks { files } => (
            fill(tr("{} leftover parts in slot {}"), &[&files.len().to_string(), &label]),
            tr("No mod uses them, so the game never opens them. Fix moves them to the trash.").to_string(),
        ),
        Kind::Doubled { same: true } => (
            fill(tr("Slot {}: the same mod on and off"), &[&label]),
            tr("A switched off copy sits beside the one that is on. Fix moves the off copy to the trash.").to_string(),
        ),
        Kind::Doubled { same: false } => (
            fill(tr("Slot {}: two mods share the slot"), &[&label]),
            if matches!(finding.fix, Fix::Adopt { .. }) {
                tr("The game reads only the one that is on. Fix moves the other into a free slot, switched off.").to_string()
            } else {
                tr("The switched off one shares parts with the other and cannot load. Fix moves it to the trash.").to_string()
            },
        ),
        Kind::LibraryGone { name } => (
            fill(tr("{}: its files are gone"), &[name]),
            tr("They were deleted outside Sigil. Fix removes the mod from your library.").to_string(),
        ),
        Kind::Misplaced { files, subfolder } => (
            fill(tr("{}: the game does not see it"), &[files.first().map(String::as_str).unwrap_or("")]),
            if finding.fix == Fix::None {
                tr("Some of its parts are missing, so Sigil cannot move it.").to_string()
            } else if *subfolder {
                tr("The game does not read subfolders. Fix moves it into a free slot, switched off. Leave it if another tool put it there.").to_string()
            } else {
                tr("The game ignores its name or folder. Fix moves it into a free slot, switched off.").to_string()
            },
        ),
        Kind::Unmounted { folder } => (
            fill(tr("{} is not loaded"), &[folder]),
            tr("gameinfo.gi no longer mounts this folder, often after a game update. Fix mounts it again.").to_string(),
        ),
        Kind::StoppedReorder { files } => (
            tr("A reorder stopped halfway").to_string(),
            fill(tr("{} files still have temporary names. Fix puts them back in their slots."), &[&files.len().to_string()]),
        ),
        Kind::Duplicate { keep } => (
            fill(tr("{} is installed twice"), &[&name]),
            fill(
                tr("Same files as {} in slot {}. Fix moves this copy to the trash; the other stays."),
                &[&slot_name(s, *keep), &deadlock_core::addons::slot_label(*keep)],
            ),
        ),
    }
}

fn toggle(st: &mut State, id: &str) {
    let ticked = &mut st.installed.folder_fix.ticked;
    if !ticked.remove(id) {
        ticked.insert(id.to_string());
    }
}

fn finding_row(s: &State, u: &Ui, finding: &Finding) -> El {
    let (title, line) = words(s, finding);
    let fixable = finding.fix != Fix::None;
    let on = s.installed.folder_fix.ticked.contains(&finding.id);
    let id = finding.id.clone();
    let mut tick = row()
        .key(format!("repair-tick-{}", finding.id))
        .w(28.0)
        .pt(2.0)
        .role("checkbox")
        .checked(on)
        .label(title.clone());
    tick = if fixable {
        tick.focusable(true).cursor(rune::Cursor::Pointer).on_press(move |st: &mut State| toggle(st, &id)).child(super::check_box(u, on, false))
    } else {
        tick.disabled(true).opacity(0.35).child(super::check_box(u, false, false))
    };
    let mut out = row()
        .w_full()
        .gap(10.0)
        .py(6.0)
        .child(tick)
        .child(
            col()
                .grow(1.0)
                .shrink()
                .min_w(0.0)
                .gap(2.0)
                .child(text(title).font(BODY_MED).color(u.ink).wrap())
                .child(text(line).font(SMALL).color(u.muted).wrap()),
        );
    if let (Kind::MissingChunks { .. }, Some(slot)) = (&finding.kind, finding.slot) {
        let known = s
            .addons
            .addons
            .iter()
            .find(|a| a.slot == slot)
            .and_then(|a| super::meta_for(s, a))
            .and_then(|m| m.gamebanana_id)
            .is_some();
        if known {
            out = out.child(
                u.action(tr("Reinstall"), ActionKind::Ghost)
                    .key(format!("repair-reinstall-{slot}"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        close(st);
                        super::reinstall(st, slot);
                    }),
            );
        }
    }
    out
}

pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    let f = &s.installed.folder_fix;
    if !f.open {
        return None;
    }
    let mut body = col().w_full().gap(4.0);
    let mut count = 0;
    match &f.review {
        None => body = body.child(text(tr("Reading the addon folders")).font(SMALL).color(u.muted)),
        Some(Err(error)) => body = body.child(crate::notify::feedback::error_box(u, "repair-error", error)),
        Some(Ok(review)) if review.is_clean() => {
            body = body.child(
                text(tr("Nothing to fix: every slot is whole, nothing is installed twice, and the game sees every mod."))
                    .font(BODY)
                    .color(u.muted)
                    .wrap(),
            )
        }
        Some(Ok(review)) => {
            for (index, heading) in SECTIONS.iter().enumerate() {
                let mut rows: Vec<&Finding> = review.findings.iter().filter(|x| section(&x.kind) == index).collect();
                if rows.is_empty() {
                    continue;
                }
                rows.sort_by_key(|x| x.slot.unwrap_or(u16::MAX));
                body = body.child(text(tr(heading)).font(H2).color(u.ink).pt(8.0));
                for finding in rows {
                    body = body.child(finding_row(s, u, finding));
                }
            }
            count = review.findings.iter().filter(|x| f.ticked.contains(&x.id)).count();
        }
    }
    let busy = f.job.is_running() || f.apply_job.is_running();
    let fix_label = if f.apply_job.is_running() {
        tr("Fixing").to_string()
    } else if count == 1 {
        tr("Fix 1 problem").to_string()
    } else {
        fill(tr("Fix {} problems"), &[&count.to_string()])
    };
    Some(
        u.dialog_shell()
            .key("installed-folder-fix")
            .role("dialog")
            .label(tr("Repair the mods folder"))
            .w(640.0)
            .max_h(700.0)
            .padding(24.0)
            .gap(14.0)
            .modal(true, close)
            .child(text(tr("Repair the mods folder")).font(H1).color(u.ink))
            .child(
                text(tr("What is broken or out of place in the addon folders. Tick what to fix: removed files go to Sigil's trash, and Undo puts everything back."))
                    .font(BODY)
                    .color(u.muted)
                    .wrap(),
            )
            .child(col().w_full().shrink().min_h(0.0).scroll(rune::ScrollSpec { stick_bottom: false }).child(body))
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(
                        u.action(tr("Read again"), ActionKind::Ghost)
                            .key("repair-again")
                            .disabled(busy)
                            .cursor(rune::Cursor::Pointer)
                            .on_press(start),
                    )
                    .child(spacer())
                    .child(text(tr("Close Deadlock first")).font(MONO).color(u.faint).opacity(if crate::screens::settings::watch::changes_blocked() { 1.0 } else { 0.0 }))
                    .child(u.action(tr("Close"), ActionKind::Ghost).key("repair-close").cursor(rune::Cursor::Pointer).on_press(close))
                    .child(
                        u.action(&fix_label, ActionKind::Primary)
                            .key("repair-fix")
                            .disabled(busy || count == 0)
                            .cursor(rune::Cursor::Pointer)
                            .on_press(apply),
                    ),
            ),
    )
}

/// The scene: something in every section, ticked as the review would tick it.
pub fn stage_scene(st: &mut State) {
    use std::path::PathBuf;
    let finding = |id: &str, slot: Option<u16>, kind: Kind, fix: Fix, suggested: bool| Finding { id: id.to_string(), slot, kind, fix, suggested };
    let findings = vec![
        finding("chunks:3", Some(3), Kind::MissingChunks { chunks: vec!["pak03_001.vpk".into()] }, Fix::Trash { slot: 3, names: Vec::new() }, false),
        finding("duplicate:6", Some(6), Kind::Duplicate { keep: 2 }, Fix::Trash { slot: 6, names: Vec::new() }, true),
        finding("doubled:4", Some(4), Kind::Doubled { same: true }, Fix::Trash { slot: 4, names: vec!["pak04_dir.vpk.disabled".into()] }, true),
        finding(
            "misplaced:a",
            None,
            Kind::Misplaced { files: vec!["addons/haze_neon.vpk".into()], subfolder: false },
            Fix::Adopt { files: vec![(PathBuf::from("haze_neon.vpk"), "_dir.vpk.disabled".into())] },
            true,
        ),
        finding(
            "misplaced:b",
            None,
            Kind::Misplaced { files: vec!["addons/downloads/pak01_dir.vpk".into()], subfolder: true },
            Fix::Adopt { files: vec![(PathBuf::from("pak01_dir.vpk"), "_dir.vpk.disabled".into())] },
            false,
        ),
        finding("unmounted:sigil-addons2", None, Kind::Unmounted { folder: "sigil-addons2".into() }, Fix::Mount, true),
        finding("orphan:12", Some(12), Kind::OrphanChunks { files: vec!["pak12_000.vpk".into(), "pak12_001.vpk".into()] }, Fix::Trash { slot: 12, names: Vec::new() }, true),
        finding("gone:9", Some(9), Kind::LibraryGone { name: "Abrams Gold Armor".into() }, Fix::Forget { slot: 9 }, true),
    ];
    st.installed.folder_fix.ticked = findings.iter().filter(|f| f.suggested).map(|f| f.id.clone()).collect();
    st.installed.folder_fix.review = Some(Ok(Review { findings }));
    st.installed.folder_fix.open = true;
    st.installed.folder_fix.copies = [(6, 2)].into_iter().collect();
}

#[cfg(test)]
mod tests {
    use super::*;
    use deadlock_core::mod_extras::{Extras, ModList};

    #[test]
    fn a_removed_copy_hands_over_its_lists_once() {
        let mut extras = Extras::default();
        extras.lists.push(ModList { id: "a".into(), name: "A".into(), members: vec!["x".into(), "y".into()], ..Default::default() });
        extras.lists.push(ModList { id: "b".into(), name: "B".into(), members: vec!["x".into(), "keep".into()], ..Default::default() });
        carry_members(&mut extras, &[("x".into(), "keep".into())]);
        assert_eq!(extras.lists[0].members, vec!["keep", "y"]);
        assert_eq!(extras.lists[1].members, vec!["keep"]);
    }

    #[test]
    fn every_kind_has_a_section() {
        for kind in [
            Kind::MissingChunks { chunks: vec![] },
            Kind::Unreadable { error: String::new() },
            Kind::OrphanChunks { files: vec![] },
            Kind::Doubled { same: true },
            Kind::Doubled { same: false },
            Kind::LibraryGone { name: String::new() },
            Kind::Misplaced { files: vec![], subfolder: true },
            Kind::Unmounted { folder: String::new() },
            Kind::StoppedReorder { files: vec![] },
            Kind::Duplicate { keep: 1 },
        ] {
            assert!(section(&kind) < SECTIONS.len());
        }
    }
}
