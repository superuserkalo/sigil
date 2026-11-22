//! The tools that look inside a mod: safety scan, identification, compatibility review, merge
//! and fonts. Each one reads on a worker and shows its result in a dialog; the two that write
//! (merge, fonts) do it behind their own button and put the change behind Undo.

use std::collections::{BTreeMap, BTreeSet};

use deadlock_core::fonts::{self, Font};
use deadlock_core::gamebanana::{self, ModProfile};
use deadlock_core::identify::{self, Identities, Match};
use deadlock_core::import::{self, ModMeta};
use deadlock_core::mod_health::{self, SlotFile, SlotReview};
use deadlock_core::mod_merge::{self, MergeInput, MergePreview, MergedMod, MergedStore};
use deadlock_core::mod_safety::{self, ScanReport};
use rune::{El, TextEdit, col, icon, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::jobs::Job;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{ActionKind, BODY, BODY_MED, H1, MICRO, SMALL, Ui};

use super::notes;
use super::{meta_for, writable_game};

/// Everything these tools remember between frames.
#[derive(Default)]
pub struct Tools {
    pub scan: Option<ScanUi>,
    pub scan_job: Job<(u16, ScanReport)>,
    /// Scan results by `(slot, size)`, so a chip does not scan twice.
    pub scans: BTreeMap<(u16, u64), ScanReport>,
    /// The mod safety review list (`safety_review.rs`).
    pub review: super::safety_review::Review,
    pub identify: Option<IdentifyUi>,
    pub health: Option<HealthUi>,
    pub merge: Option<MergeUi>,
    pub merge_preview: Job<MergePreview>,
    pub merge_run: Job<MergedMod>,
    /// Pack all enabled into N (`pack_all.rs`).
    pub pack_all: super::pack_all::PackAll,
    /// The second-skin check after an install (`one_skin.rs`).
    pub one_skin: crate::screens::one_skin::Check,
    pub fonts: Option<FontUi>,
    pub fonts_job: Job<Vec<Font>>,
    pub font_install: Job<fonts::FontReport>,
    pub merged: MergedStore,
    /// The merges of this addon folder, read once.
    pub merged_loaded: bool,
}

pub struct ScanUi {
    pub slot: u16,
    pub report: Option<ScanReport>,
}

pub struct IdentifyUi {
    pub slot: u16,
    pub link: TextEdit,
    pub found: Option<Match>,
    pub stage: Stage,
    pub error: Option<String>,
    pub job: Job<(Match, Option<ModProfile>)>,
    /// A match from Sigil's shared index waits here until the player confirms it: anyone can
    /// contribute to that index, so it is never applied on its own.
    pub pending: Option<(Match, ModProfile)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Looking,
    Found,
    Failed,
}

pub struct HealthUi {
    pub reviews: Vec<SlotReview>,
    pub running: bool,
    /// "Turn off mods with outdated data" is asking before it switches them.
    pub confirm_off: bool,
    pub error: Option<String>,
    pub job: Job<Vec<SlotReview>>,
    pub checked: usize,
}

pub struct MergeUi {
    pub picked: BTreeSet<u16>,
    pub name: TextEdit,
    pub strict: bool,
    pub preview: Option<MergePreview>,
    pub stage: MergeStage,
    pub error: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MergeStage {
    Picking,
    Previewing,
    Running,
    Done,
}

pub struct FontUi {
    pub slot: u16,
    pub fonts: Vec<Font>,
    pub stage: Stage,
    pub error: Option<String>,
    pub installed: bool,
}

// --- poll ---------------------------------------------------------------------

/// Take finished work, and read the merge record the first time the page opens.
pub fn poll(st: &mut State) -> bool {
    let mut changed = super::safety_review::poll(st);
    if !st.demo && !st.installed.tools.merged_loaded && st.addons.scanned {
        st.installed.tools.merged = mod_merge::read_merged(&mod_merge::merged_path(
            &crate::data::data_dir(),
        ));
        st.installed.tools.merged_loaded = true;
        changed = true;
    }
    if let Some(result) = st.installed.tools.scan_job.try_take() {
        match result {
            Ok((slot, report)) => {
                if let Some(addon) = st.addons.addons.iter().find(|addon| addon.slot == slot) {
                    st.installed
                        .tools
                        .scans
                        .insert((slot, addon.size_bytes), report.clone());
                }
                if let Some(ui) = &mut st.installed.tools.scan
                    && ui.slot == slot
                {
                    ui.report = Some(report);
                }
            }
            Err(error) => {
                if let Some(ui) = &mut st.installed.tools.scan {
                    ui.report = Some(ScanReport {
                        error: Some(error),
                        ..ScanReport::default()
                    });
                }
            }
        }
        changed = true;
    }
    if let Some(result) = st.installed.tools.health.as_mut().and_then(|ui| ui.job.try_take()) {
        if let Some(ui) = &mut st.installed.tools.health {
            ui.running = false;
            match result {
                Ok(reviews) => {
                    ui.checked = reviews.len();
                    ui.reviews = reviews;
                }
                Err(error) => ui.error = Some(error),
            }
        }
        changed = true;
    }
    if let Some(result) = st.installed.tools.merge_preview.try_take() {
        if let Some(ui) = &mut st.installed.tools.merge {
            ui.stage = MergeStage::Picking;
            match result {
                Ok(preview) => ui.preview = Some(preview),
                Err(error) => ui.error = Some(error),
            }
        }
        changed = true;
    }
    if let Some(result) = st.installed.tools.merge_run.try_take() {
        match result {
            Ok(merged) => {
                apply_merge_record(st, &merged);
                let mut recap = super::search::Recap::new(format!("Merged into {}", merged.name));
                recap.done.push(format!(
                    "Slot {:02}: {} files, {}",
                    merged.slot,
                    merged.entries,
                    crate::model::format_size(merged.bytes)
                ));
                for source in &merged.sources {
                    recap.skipped.push(format!("{:02} {}", source.slot, source.name));
                }
                st.installed.recap = Some(recap);
                st.installed.tools.merged =
                    mod_merge::read_merged(&mod_merge::merged_path(&crate::data::data_dir()));
                if let Some(ui) = &mut st.installed.tools.merge {
                    ui.stage = MergeStage::Done;
                    ui.error = None;
                }
                st.status = format!("Merged into slot {:02}. It is switched off.", merged.slot);
                super::rescan(st);
            }
            Err(error) => {
                if let Some(ui) = &mut st.installed.tools.merge {
                    ui.stage = MergeStage::Picking;
                    ui.error = Some(error);
                }
            }
        }
        changed = true;
    }
    if let Some(result) = st.installed.tools.identify.as_mut().and_then(|ui| ui.job.try_take()) {
        match result {
            Ok((found, profile)) => {
                let slot = st.installed.tools.identify.as_ref().map(|ui| ui.slot).unwrap_or(0);
                if let Some(ui) = &mut st.installed.tools.identify {
                    ui.found = Some(found.clone());
                    ui.stage = if found.found() {
                        Stage::Found
                    } else {
                        Stage::Failed
                    };
                }
                if let Some(profile) = profile {
                    if matches!(found.source.as_str(), "lookup" | "crc" | "name") {
                        if let Some(ui) = &mut st.installed.tools.identify {
                            ui.pending = Some((found.clone(), profile));
                        }
                    } else {
                        apply_identification(st, slot, &found, &profile);
                    }
                }
            }
            Err(error) => {
                if let Some(ui) = &mut st.installed.tools.identify {
                    ui.stage = Stage::Failed;
                    ui.error = Some(error);
                }
            }
        }
        changed = true;
    }
    if let Some(result) = st.installed.tools.fonts_job.try_take() {
        if let Some(ui) = &mut st.installed.tools.fonts {
            match result {
                Ok(fonts) => {
                    ui.fonts = fonts;
                    ui.stage = Stage::Found;
                }
                Err(error) => {
                    ui.stage = Stage::Failed;
                    ui.error = Some(error);
                }
            }
        }
        changed = true;
    }
    if let Some(result) = st.installed.tools.font_install.try_take() {
        match result {
            Ok(report) => {
                let n = report.installed.len();
                let removed = !report.removed.is_empty();
                let slot = st.installed.tools.fonts.as_ref().map(|ui| ui.slot).unwrap_or(0);
                if let Some(ui) = &mut st.installed.tools.fonts {
                    ui.installed = !removed;
                    ui.error = None;
                }
                if removed || n == 0 {
                    st.installed.last_undo = Some(super::Undo::FontsRemoved(slot));
                    st.status = format!("Took the fonts of slot {} out.", deadlock_core::addons::slot_label(slot));
                } else {
                    st.installed.last_undo = Some(super::Undo::FontsInstalled(slot));
                    st.status = format!(
                        "Added {n} font{} for slot {slot:02}.",
                        if n == 1 { "" } else { "s" }
                    );
                }
                st.installed.undo_fresh = true;
            }
            Err(error) => {
                if let Some(ui) = &mut st.installed.tools.fonts {
                    ui.error = Some(error);
                }
            }
        }
        changed = true;
    }
    changed
}

// --- chips ---------------------------------------------------------------------

// --- safety scan ---------------------------------------------------------------

/// Scan a slot on a worker and open the report.
pub fn open_scan(st: &mut State, slot: u16) {
    st.installed.menu = None;
    st.installed.tools.scan = Some(ScanUi { slot, report: None });
    if let Some(report) = st
        .addons
        .addons
        .iter()
        .find(|addon| addon.slot == slot)
        .and_then(|addon| st.installed.tools.scans.get(&(slot, addon.size_bytes)))
        .cloned()
    {
        st.installed.tools.scan = Some(ScanUi {
            slot,
            report: Some(report),
        });
        return;
    }
    if st.demo || rune::is_headless() {
        return;
    }
    let Some(addon) = st.addons.addons.iter().find(|addon| addon.slot == slot) else {
        return;
    };
    let path = addon.path(&super::addons_folder(st));
    let wake = st.wake.clone();
    st.installed.tools.scan_job.start(wake, move || {
        Ok((slot, mod_safety::scan_vpk(&path)))
    });
}

pub fn scan_dialog(s: &State, u: &Ui) -> Option<El> {
    let ui = s.installed.tools.scan.as_ref()?;
    let name = s
        .addons
        .addons
        .iter()
        .find(|addon| addon.slot == ui.slot)
        .map(|addon| super::display_name(s, addon))
        .unwrap_or_else(|| format!("Slot {:02}", ui.slot));
    let close = |st: &mut State| st.installed.tools.scan = None;
    let mut list = col().w_full().gap(4.0);
    let body: String;
    match &ui.report {
        None => body = "Reading the mod's files.".to_string(),
        Some(report) if report.error.is_some() => {
            body = report.error.clone().unwrap_or_default();
        }
        Some(report) if report.is_clear() => {
            body = format!(
                "{} files and nothing unusual inside: no programs, no loose scripts, no network calls.",
                report.entries
            );
        }
        Some(report) => {
            body = format!(
                "{} of {} files are worth a look. This is a warning, not a block.",
                report.findings.len() + report.more,
                report.entries
            );
            for finding in report.findings.iter().take(10) {
                let path = finding.path.clone();
                // Exactly why the file is listed, for example the API it calls.
                let line = finding.reason();
                list = list.child(
                    row()
                        .w_full()
                        .min_h(32.0)
                        .gap(8.0)
                        .items_start()
                        .pt(4.0)
                        .pb(4.0)
                        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                        .child(
                            icon(&icons::SHIELD, 14.0)
                                .color(if finding.risk == mod_safety::Risk::Program {
                                    u.rose
                                } else {
                                    u.amber
                                })
                                .a11y_hidden()
                                .pt(2.0),
                        )
                        .child(
                            col()
                                .grow(1.0)
                                .min_w(0.0)
                                .gap(1.0)
                                .child(
                                    text(format!("{}  ·  {}", finding.risk.label(), path))
                                        .font(BODY)
                                        .color(u.prose)
                                        .truncate(),
                                )
                                .child(text(line).font(SMALL).color(u.faint).truncate()),
                        ),
                );
            }
            if report.findings.len() + report.more > 10 {
                list = list.child(
                    text(format!(
                        "and {} more",
                        report.findings.len() + report.more - 10
                    ))
                    .font(SMALL)
                    .color(u.faint),
                );
            }
        }
    }
    Some(
        u.dialog_shell()
            .key("installed-safety")
            .role("dialog")
            .label(format!("What is inside {name}"))
            .w(520.0)
            .padding(24.0)
            .gap(16.0)
            .modal(true, close)
            .child(text(name).font(H1).color(u.ink).wrap())
            .child(text(body).font(BODY).color(u.muted).wrap())
            .child(list)
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(spacer())
                    .children(
                        ui.report
                            .as_ref()
                            .filter(|report| !report.is_clear())
                            .map(|_| {
                                u.action(tr("Mod safety"), ActionKind::Secondary)
                                    .key("safety-open-review")
                                    .cursor(rune::Cursor::Pointer)
                                    .on_press(super::safety_review::open)
                            }),
                    )
                    .child(
                        u.action("Done", ActionKind::Primary)
                            .key("safety-done")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(close),
                    ),
            ),
    )
}

// --- identification -------------------------------------------------------------

/// What the identify worker was asked for.
enum Ask {
    Offline,
    Link(u64, Option<u64>),
    /// Sigil's hash index, with unknown-mod lookup consent (`settings::telemetry::lookup`).
    Lookup,
    /// GameBanana name search, then Grimoire's archive CRC probe over the hits (C8) when the
    /// player turned it on. Search words come from the name Sigil shows and the file name.
    Name(Vec<String>, bool),
}

// --- CRC opt-in (Grimoire: an off-by-default switch) -------------------------------------------
//
// The CRC probe reads parts of every matching archive from GameBanana. It stays off until the
// player ticks "Also compare checksums" in the identify dialog; the choice is remembered in
// `<data>/identify-crc.json`.

fn crc_path() -> std::path::PathBuf {
    crate::data::data_dir().join("identify-crc.json")
}

static CRC_ON: std::sync::OnceLock<std::sync::atomic::AtomicBool> = std::sync::OnceLock::new();

fn crc_cell() -> &'static std::sync::atomic::AtomicBool {
    CRC_ON.get_or_init(|| {
        let on = !rune::is_headless()
            && !cfg!(test)
            && std::fs::read_to_string(crc_path()).is_ok_and(|text| text.trim() == "true");
        std::sync::atomic::AtomicBool::new(on)
    })
}

pub fn crc_on() -> bool {
    crc_cell().load(std::sync::atomic::Ordering::Relaxed)
}

fn set_crc(on: bool) {
    crc_cell().store(on, std::sync::atomic::Ordering::Relaxed);
    if !rune::is_headless() && !cfg!(test) {
        let _ = std::fs::write(crc_path(), if on { "true" } else { "false" });
    }
}

/// Look at a slot's own metadata and Sigil's records. No network until a mod is known or a link
/// is given; the page itself is fetched once so the name and picture can be filled in.
pub fn open_identify(st: &mut State, slot: u16) {
    st.installed.menu = None;
    st.installed.tools.identify = Some(IdentifyUi {
        slot,
        link: TextEdit::new(""),
        found: None,
        stage: Stage::Looking,
        error: None,
        job: Job::default(),
        pending: None,
    });
    start_identify(st, slot, Ask::Offline);
}

fn start_identify(st: &mut State, slot: u16, ask: Ask) {
    if st.demo || rune::is_headless() {
        return;
    }
    let Some(addon) = st.addons.addons.iter().find(|addon| addon.slot == slot) else {
        return;
    };
    let path = addon.path(&super::addons_folder(st));
    let store: Identities = identify::read_identities(&identify::identities_path(
        &crate::data::data_dir(),
    ));
    let journal =
        deadlock_core::install::read_journal(&crate::data::workspace_dir()).unwrap_or_default();
    let wake = st.wake.clone();
    if let Some(ui) = &mut st.installed.tools.identify {
        ui.stage = Stage::Looking;
        ui.pending = None;
        ui.error = None;
    }
    if let Some(ui) = &mut st.installed.tools.identify {
        ui.job.start(wake, move || {
            let found = match ask {
                Ask::Offline => identify::identify(&path, &store, &journal),
                Ask::Link(id, file_id) => Match {
                    gamebanana_id: Some(id),
                    file_id,
                    ..Match::default()
                },
                Ask::Name(terms, crc) => {
                    let mut ids: Vec<u64> = Vec::new();
                    let mut first: Option<deadlock_core::gamebanana::BrowseMod> = None;
                    for term in terms.iter().take(2) {
                        for hit in deadlock_core::mod_lookup::search_by_name(term, 10).map_err(|e| e.to_string())? {
                            if !ids.contains(&hit.id) {
                                ids.push(hit.id);
                                if first.is_none() {
                                    first = Some(hit);
                                }
                            }
                        }
                    }
                    if ids.is_empty() {
                        return Err(tr("GameBanana has no mod by that name. Paste its link instead.").to_string());
                    }
                    let probe = if crc {
                        deadlock_core::mod_lookup::crc_detect(&path, &ids, None).map_err(|e| e.to_string())?
                    } else {
                        None
                    };
                    match probe {
                        // The file's checksum is inside one of the hits' archives: a sure match.
                        Some(hit) => Match {
                            gamebanana_id: Some(hit.gamebanana_id),
                            file_id: Some(hit.file_id),
                            source: "crc".to_string(),
                            ..Match::default()
                        },
                        // No archive holds it (or they are not ZIP files): the best name match, to
                        // confirm by eye.
                        None => Match {
                            gamebanana_id: first.as_ref().map(|hit| hit.id),
                            name: first.as_ref().map(|hit| hit.name.clone()),
                            source: "name".to_string(),
                            ..Match::default()
                        },
                    }
                }
                Ask::Lookup => {
                    let best = crate::screens::settings::telemetry::lookup(&path)?
                        .into_iter()
                        .max_by_key(|m| m.reports)
                        .ok_or_else(|| "Sigil's index does not know this file yet.".to_string())?;
                    Match {
                        gamebanana_id: Some(best.gamebanana_mod_id),
                        file_id: Some(best.file_id),
                        author: Some(format!("{} reports", best.reports)),
                        source: "lookup".to_string(),
                        ..Match::default()
                    }
                }
            };
            let profile = match found.gamebanana_id {
                Some(id) => gamebanana::fetch_mod_profile(id).ok(),
                None => None,
            };
            Ok((found, profile))
        });
    }
}

fn apply_identification(st: &mut State, slot: u16, found: &Match, profile: &ModProfile) {
    let Some(size_bytes) = st
        .addons
        .addons
        .iter()
        .find(|addon| addon.slot == slot)
        .map(|addon| addon.size_bytes)
    else {
        return;
    };
    let library = super::library_file();
    let existing = st
        .addons
        .addons
        .iter()
        .find(|addon| addon.slot == slot)
        .and_then(|addon| meta_for(st, addon))
        .cloned();
    let name = if profile.name.trim().is_empty() {
        found
            .name
            .clone()
            .unwrap_or_else(|| format!("Mod {}", found.gamebanana_id.unwrap_or_default()))
    } else {
        profile.name.clone()
    };
    let meta = ModMeta {
        slot,
        size_bytes,
        name,
        author: profile.submitter.as_ref().map(|member| member.name.clone()),
        thumbnail_url: profile.images.first().map(|image| image.url.clone()),
        gamebanana_id: found.gamebanana_id,
        source: existing
            .as_ref()
            .map(|meta| meta.source.clone())
            .unwrap_or_else(|| "browse".to_string()),
        recorded_at: existing
            .as_ref()
            .map(|meta| meta.recorded_at.clone())
            .unwrap_or_default(),
    };
    let _ = import::record_meta(&library, meta);
    st.installed.library_sig = None;

    let heroes = notes::heroes(st).into_iter().collect();
    notes::edit_slot(st, slot, |entry| {
        entry.gamebanana_id = found.gamebanana_id;
        entry.fetched = true;
        notes::apply_profile(entry, profile, &heroes);
    });
    // Remember the bytes, so the next copy of this mod is recognised with no network.
    let file = st
        .addons
        .addons
        .iter()
        .find(|addon| addon.slot == slot)
        .map(|addon| addon.file.clone())
        .unwrap_or_default();
    let folder = deadlock_core::addons::slot_folder(&super::addons_folder(st), slot);
    if let Ok(sha256) = identify::sha256_file(&folder.join(&file)) {
        let path = identify::identities_path(&crate::data::data_dir());
        let mut store = identify::read_identities(&path);
        store.remember(identify::IdentityEntry {
            sha256,
            size_bytes,
            gamebanana_id: found.gamebanana_id,
            file_id: found.file_id,
            name: Some(profile.name.clone()),
            source: "linked".to_string(),
        });
        let _ = identify::write_identities(&path, &store);
    }
    st.status = format!("Linked slot {slot:02} to {}.", profile.name);
}

pub fn identify_dialog(s: &State, u: &Ui) -> Option<El> {
    let ui = s.installed.tools.identify.as_ref()?;
    let name = s
        .addons
        .addons
        .iter()
        .find(|addon| addon.slot == ui.slot)
        .map(|addon| super::display_name(s, addon))
        .unwrap_or_else(|| format!("Slot {:02}", ui.slot));
    let close = |st: &mut State| st.installed.tools.identify = None;
    let mut body = col().w_full().gap(12.0);
    match ui.stage {
        Stage::Looking => {
            body = body.child(
                text("Looking at the mod's own metadata and Sigil's records.")
                    .font(BODY)
                    .color(u.muted)
                    .wrap(),
            );
        }
        Stage::Found if ui.pending.is_some() => {
            if let Some((found, profile)) = &ui.pending {
                let reports = found.author.clone().unwrap_or_default();
                body = body
                    .child(
                        text(format!("Sigil's shared index says this is {} (GameBanana mod {}).", profile.name, profile.id))
                            .font(BODY_MED)
                            .color(u.ink)
                            .wrap(),
                    )
                    .child(
                        text(format!("Players add to this index, so check it before you use it ({reports}). Nothing changes until you press Use this match."))
                            .font(SMALL)
                            .color(u.muted)
                            .wrap(),
                    )
                    .child(
                        u.action("Use this match", ActionKind::Primary)
                            .key("identify-use-lookup")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(|st: &mut State| {
                                let Some(ui) = &mut st.installed.tools.identify else { return };
                                let slot = ui.slot;
                                if let Some((found, profile)) = ui.pending.take() {
                                    apply_identification(st, slot, &found, &profile);
                                }
                            }),
                    );
            }
        }
        Stage::Found => {
            if let Some(found) = &ui.found
                && let Some(id) = found.gamebanana_id
            {
                body = body
                    .child(
                        text(format!("This is GameBanana mod {id}."))
                            .font(BODY_MED)
                            .color(u.ink)
                            .wrap(),
                    )
                    .child(
                        text("The name, author, picture and hero are filled in from its page.")
                            .font(SMALL)
                            .color(u.muted)
                            .wrap(),
                    );
                let url = identify::mod_url(id);
                let row = row()
                    .w_full()
                    .h(32.0)
                    .center_y()
                    .child(
                        text(url.clone())
                            .font(SMALL)
                            .color(u.faint)
                            .truncate()
                            .grow(1.0)
                            .min_w(0.0),
                    )
                    .child(
                        u.action("Copy link", ActionKind::Ghost)
                            .key("identify-copy")
                            .cursor(rune::Cursor::Pointer)
                            .on_press_tx(move |_st: &mut State, tx: &mut rune::Tx| tx.copy(url.clone())),
                    );
                body = body.child(row);
            }
        }
        Stage::Failed => {
            body = body.child(
                text("Sigil does not know this file. Paste its GameBanana link to link it.")
                    .font(BODY)
                    .color(u.muted)
                    .wrap(),
            );
            if let Some(error) = &ui.error {
                body = body.child(crate::notify::feedback::error_line(u, "installed-tool-error", error));
            }
            let field = rune::text_input(&ui.link, BODY)
                .key("identify-link")
                .single_line()
                .label("GameBanana link or id")
                .w_full()
                .color(u.ink)
                .placeholder("https://gamebanana.com/mods/...", u.faint)
                .caret(u.accent, u.accent.alpha(0.20))
                .on_edit(rune::edit_access(|st: &mut State| {
                    &mut st.installed.tools.identify.as_mut().expect("identify is open").link
                }))
                .on_submit(|st: &mut State, _tx: &mut rune::Tx| identify_from_link(st));
            body = body.child(field);
        }
    }
    let mut actions = row().w_full().gap(8.0).center_y();
    // Unknown-mod lookup (opt-in, Settings > Privacy): ask Sigil's hash index.
    let lookup = s.settings.telemetry.consent;
    if ui.stage == Stage::Failed && lookup.asked && lookup.hash_lookup && !s.prefs.offline {
        let slot = ui.slot;
        actions = actions.child(
            u.action(tr("Look it up"), ActionKind::Ghost)
                .key("identify-lookup")
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    crate::screens::settings::telemetry::event(st, "identify:lookup", Some("page:installed"));
                    start_identify(st, slot, Ask::Lookup);
                }),
        );
    }
    let mut search_row = row().w_full().gap(8.0).center_y();
    if ui.stage == Stage::Failed && !s.prefs.offline {
        let slot = ui.slot;
        search_row = search_row.child(
            u.action(tr("Search GameBanana by name"), ActionKind::Ghost)
                .key("identify-name")
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    let terms = st
                        .addons
                        .addons
                        .iter()
                        .find(|a| a.slot == slot)
                        .map(|a| {
                            let shown = super::display_name(st, a);
                            deadlock_core::mod_lookup::search_terms(Some(&shown), &[a.file.as_str()])
                        })
                        .unwrap_or_default();
                    start_identify(st, slot, Ask::Name(terms, crc_on()));
                }),
        );
        let on = crc_on();
        search_row = search_row.child(
            row()
                .key("identify-crc")
                .gap(6.0)
                .center_y()
                .focusable(true)
                .role("checkbox")
                .checked(on)
                .label(tr("Also compare checksums"))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |_st: &mut State| set_crc(!on))
                .child(rune::icon(if on { &icons::SQUARE_CHECK } else { &icons::SQUARE }, 14.0).color(if on { u.accent } else { u.faint }).a11y_hidden())
                .child(text(tr("Also compare checksums")).font(SMALL).color(u.muted).a11y_hidden()),
        );
    }
    actions = actions.child(spacer());
    if ui.stage == Stage::Failed {
        actions = actions.child(
            u.action("Link this mod", ActionKind::Primary)
                .key("identify-link-go")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| identify_from_link(st)),
        );
    }
    actions = actions.child(
        u.action("Done", ActionKind::Secondary)
            .key("identify-done")
            .cursor(rune::Cursor::Pointer)
            .on_press(close),
    );
    Some(
        u.dialog_shell()
            .key("installed-identify")
            .role("dialog")
            .label(format!("Which mod is {name}"))
            .w(500.0)
            .padding(24.0)
            .gap(16.0)
            .modal(true, close)
            .child(text(name).font(H1).color(u.ink).wrap())
            .child(body)
            .child(search_row)
            .child(actions),
    )
}

fn identify_from_link(st: &mut State) {
    let Some(ui) = st.installed.tools.identify.as_ref() else { return };
    let slot = ui.slot;
    let text = ui.link.text.clone();
    match identify::link_target(&text) {
        Ok((id, file_id)) => start_identify(st, slot, Ask::Link(id, file_id)),
        Err(error) => {
            if let Some(ui) = &mut st.installed.tools.identify {
                ui.error = Some(error);
            }
        }
    }
}

// --- compatibility review ---------------------------------------------------------

/// Review every enabled mod against the game's own file list. The game index and each VPK are
/// read on a worker.
pub fn open_health(st: &mut State) {
    st.installed.menu = None;
    let mut wanted: Vec<SlotFile> = st
        .addons
        .addons
        .iter()
        .filter(|addon| addon.enabled)
        .map(|addon| SlotFile {
            slot: addon.slot,
            path: addon.path(&super::addons_folder(st)),
        })
        .collect();
    let mut ui = HealthUi {
        reviews: Vec::new(),
        running: true,
        confirm_off: false,
        error: None,
        job: Job::default(),
        checked: 0,
    };
    if st.demo || rune::is_headless() {
        ui.running = false;
        st.installed.tools.health = Some(ui);
        return;
    }
    let Some(game_root) = st.game_root.clone() else {
        ui.running = false;
        ui.error = Some("Set the game folder in Settings first.".to_string());
        st.installed.tools.health = Some(ui);
        return;
    };
    super::repair::start_check(st);
    let cache = crate::data::data_dir().join("cache");
    let wake = st.wake.clone();
    ui.job.start(wake, move || {
        let paths = mod_health::game_paths(&game_root, &cache).map_err(|error| error.to_string())?;
        wanted.sort_by_key(|slot| slot.slot);
        Ok(mod_health::review_against(&wanted, &paths))
    });
    st.installed.tools.health = Some(ui);
}

/// Mods the review found with outdated game data: an old hero model or a game data file the
/// game no longer has (Grimoire's "outdated data" issue). These are the ones that can crash or
/// look wrong, so the bulk action turns them off; a moved or gone path only goes unused.
pub fn outdated_slots(reviews: &[SlotReview]) -> Vec<u16> {
    let mut slots: Vec<u16> = reviews
        .iter()
        .filter(|review| review.issues.iter().any(|issue| issue.kind == mod_health::IssueKind::Skeleton))
        .map(|review| review.slot)
        .collect();
    slots.sort_unstable();
    slots.dedup();
    slots
}

/// Turn off every enabled mod with outdated data, behind Undo (Grimoire's Issues panel bulk
/// action). The review keeps the rows of the mods that stayed on.
fn turn_off_outdated(st: &mut State) {
    let Some(ui) = st.installed.tools.health.as_mut() else { return };
    ui.confirm_off = false;
    let slots = outdated_slots(&ui.reviews);
    if st.demo {
        st.status = tr("Demo library: nothing was changed.").to_string();
        return;
    }
    if crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    let targets: Vec<(u16, String)> = st
        .addons
        .addons
        .iter()
        .filter(|addon| addon.enabled && slots.contains(&addon.slot))
        .map(|addon| (addon.slot, super::display_name(st, addon)))
        .collect();
    if targets.is_empty() {
        return;
    }
    let changed = super::switch_slots(st, targets, false);
    if let Some(ui) = st.installed.tools.health.as_mut() {
        ui.reviews.retain(|review| !changed.contains(&review.slot));
        ui.checked = ui.checked.saturating_sub(changed.len());
    }
}

/// The footer: the bulk action and its confirm, then Done.
fn health_footer(ui: &HealthUi, u: &Ui) -> El {
    let close = |st: &mut State| st.installed.tools.health = None;
    let outdated = if ui.running || ui.error.is_some() { 0 } else { outdated_slots(&ui.reviews).len() };
    let mut footer = row().w_full().gap(8.0).center_y();
    if ui.confirm_off && outdated > 0 {
        let line = if outdated == 1 {
            tr("Turn off 1 mod? It stays installed, and Undo turns it back on.").to_string()
        } else {
            fill(tr("Turn off {} mods? They stay installed, and Undo turns them back on."), &[&outdated.to_string()])
        };
        return footer
            .child(text(line).font(SMALL).color(u.muted).wrap().grow(1.0).min_w(0.0))
            .child(
                u.action(tr("Cancel"), ActionKind::Ghost)
                    .key("health-off-cancel")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| {
                        if let Some(ui) = st.installed.tools.health.as_mut() {
                            ui.confirm_off = false;
                        }
                    }),
            )
            .child(
                u.action(tr("Turn off"), ActionKind::Danger)
                    .key("health-off-confirm")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(turn_off_outdated),
            );
    }
    if outdated > 0 {
        let label = if outdated == 1 {
            tr("Turn off 1 mod with outdated data").to_string()
        } else {
            fill(tr("Turn off {} mods with outdated data"), &[&outdated.to_string()])
        };
        footer = footer.child(
            u.icon_action(&icons::POWER, &label, ActionKind::Secondary)
                .key("health-off-outdated")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| {
                    if let Some(ui) = st.installed.tools.health.as_mut() {
                        ui.confirm_off = true;
                    }
                }),
        );
    }
    if !ui.running && ui.error.is_none() {
        footer = footer.child(super::repair::copy_all(u));
    }
    footer.child(spacer()).child(
        u.action(tr("Done"), ActionKind::Primary)
            .key("health-done")
            .cursor(rune::Cursor::Pointer)
            .on_press(close),
    )
}

pub fn health_dialog(s: &State, u: &Ui) -> Option<El> {
    let ui = s.installed.tools.health.as_ref()?;
    let close = |st: &mut State| st.installed.tools.health = None;
    let mut list = col().w_full().gap(4.0);
    let body: String;
    if ui.running {
        body = "Reading the game's file list and every mod. This takes a moment.".to_string();
    } else if let Some(error) = &ui.error {
        body = error.clone();
    } else {
        let issues: usize = ui.reviews.iter().map(|review| review.issues.len() + review.more).sum();
        body = if issues == 0 {
            format!(
                "{} mod{} checked and every path still exists in the game.",
                ui.checked,
                if ui.checked == 1 { "" } else { "s" }
            )
        } else {
            format!(
                "{} mod{} checked, {issues} thing{} to look at.",
                ui.checked,
                if ui.checked == 1 { "" } else { "s" },
                if issues == 1 { "" } else { "s" }
            )
        };
        let mut reviews: Vec<&SlotReview> = ui.reviews.iter().filter(|review| !review.is_clear()).collect();
        reviews.sort_by_key(|review| review.slot);
        for review in reviews {
            let name = s
                .addons
                .addons
                .iter()
                .find(|addon| addon.slot == review.slot)
                .map(|addon| super::display_name(s, addon))
                .unwrap_or_else(|| format!("Slot {:02}", review.slot));
            list = list.child(
                row()
                    .w_full()
                    .h(32.0)
                    .gap(8.0)
                    .center_y()
                    .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                    .child(text(deadlock_core::addons::slot_label(review.slot)).font(MICRO).color(u.faint).w(28.0))
                    .child(text(name.clone()).font(BODY_MED).color(u.ink).truncate().grow(1.0).min_w(0.0))
                    .child(
                        text(format!(
                            "{} path{} gone",
                            review.missing,
                            if review.missing == 1 { "" } else { "s" }
                        ))
                        .font(SMALL)
                        .color(u.amber),
                    ),
            );
            let first = review
                .issues
                .first()
                .map(|issue| format!("{}: {}", issue.kind.label(), issue.path))
                .unwrap_or_default();
            if !first.is_empty() {
                list = list.child(
                    row()
                        .w_full()
                        .pb(6.0)
                        .pl(28.0)
                        .child(text(first).font(SMALL).color(u.faint).truncate()),
                );
            }
        }
    }
    Some(
        u.dialog_shell()
            .key("installed-health")
            .role("dialog")
            .label("Compatibility review")
            .w(540.0)
            .padding(24.0)
            .gap(16.0)
            .modal(true, close)
            .child(text("Compatibility review").font(H1).color(u.ink))
            .child(text(body).font(BODY).color(u.muted).wrap())
            .child(list)
            .children(super::repair::block(s, u))
            .child(health_footer(ui, u)),
    )
}

// --- merge ---------------------------------------------------------------------

/// Open the merge dialog with nothing picked.
pub fn open_merge(st: &mut State) {
    st.installed.menu = None;
    st.installed.tools.merge = Some(MergeUi {
        picked: BTreeSet::new(),
        name: TextEdit::new(""),
        strict: false,
        preview: None,
        stage: MergeStage::Picking,
        error: None,
    });
}

fn merge_inputs(s: &State) -> Vec<MergeInput> {
    let Some(ui) = &s.installed.tools.merge else {
        return Vec::new();
    };
    ui.picked
        .iter()
        .filter_map(|slot| {
            s.addons.addons.iter().find(|addon| addon.slot == *slot).map(|addon| {
                MergeInput {
                    slot: *slot,
                    name: super::display_name(s, addon),
                }
            })
        })
        .collect()
}

pub fn merge_dialog(s: &State, u: &Ui) -> Option<El> {
    let ui = s.installed.tools.merge.as_ref()?;
    let close = |st: &mut State| st.installed.tools.merge = None;
    let picks = merge_inputs(s);
    let mut body = col().w_full().gap(12.0);
    if ui.stage == MergeStage::Done {
        body = body.child(
            text("The merged mod is a new, switched off slot. Turn it on when you want it.")
                .font(BODY)
                .color(u.muted)
                .wrap(),
        );
    } else {
        body = body.child(
            text("One VPK from several. The lower slot's files win, as they do in the game.")
                .font(BODY)
                .color(u.muted)
                .wrap(),
        );
        let mut list = col().w_full().gap(2.0);
        let mods: Vec<&crate::model::AddonRow> = s.addons.addons.iter().collect();
        for addon in mods.iter().take(12) {
            let slot = addon.slot;
            let on = ui.picked.contains(&slot);
            list = list.child(
                row()
                    .key(format!("merge-pick-{slot:02}"))
                    .w_full()
                    .h(32.0)
                    .gap(8.0)
                    .center_y()
                    .focusable(true)
                    .role("checkbox")
                    .checked(on)
                    .label(format!("Merge {}", super::display_name(s, addon)))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        if let Some(ui) = &mut st.installed.tools.merge {
                            if !ui.picked.remove(&slot) {
                                ui.picked.insert(slot);
                            }
                            ui.preview = None;
                        }
                    })
                    .child(super::check_box(u, on, false))
                    .child(text(deadlock_core::addons::slot_label(slot)).font(MICRO).color(u.faint).w(28.0))
                    .child(
                        text(super::display_name(s, addon))
                            .font(BODY)
                            .color(if on { u.ink } else { u.prose })
                            .truncate()
                            .grow(1.0)
                            .min_w(0.0),
                    ),
            );
        }
        if mods.len() > 12 {
            list = list.child(
                text(format!("and {} more", mods.len() - 12))
                    .font(SMALL)
                    .color(u.faint),
            );
        }
        body = body.child(list);
        if let Some(preview) = &ui.preview {
            let mut facts = col().w_full().gap(4.0);
            if let Some(error) = &preview.error {
                facts = facts.child(crate::notify::feedback::error_line(u, "installed-tool-preview-error", error));
            } else {
                facts = facts.child(u.kv("Paths", &preview.paths.to_string(), u.ink)).child(u.kv(
                    "Same file in several",
                    &preview.collisions.len().to_string(),
                    if preview.collisions.is_empty() { u.muted } else { u.amber },
                ));
                if !preview.collisions.is_empty() {
                    facts = facts.child(
                        text("The lowest slot's file is the one that loads.")
                            .font(SMALL)
                            .color(u.faint)
                            .wrap(),
                    );
                }
                for (path, slots) in preview.collisions.iter().take(4) {
                    facts = facts.child(
                        row()
                            .w_full()
                            .h(24.0)
                            .gap(8.0)
                            .center_y()
                            .child(text(path.clone()).font(SMALL).color(u.faint).truncate().grow(1.0).min_w(0.0))
                            .child(
                                text(
                                    slots
                                        .iter()
                                        .map(|slot| format!("{slot:02}"))
                                        .collect::<Vec<_>>()
                                        .join(" > "),
                                )
                                .font(MICRO)
                                .color(u.muted),
                            ),
                    );
                }
            }
            body = body.child(facts);
        }
        let name_field = rune::text_input(&ui.name, BODY)
            .key("merge-name")
            .single_line()
            .label("Name for the merged mod")
            .w_full()
            .color(u.ink)
            .placeholder("Like All my skins", u.faint)
            .caret(u.accent, u.accent.alpha(0.20))
            .on_edit(rune::edit_access(|st: &mut State| {
                &mut st.installed.tools.merge.as_mut().expect("merge is open").name
            }));
        let strict = ui.strict;
        body = body
            .child(name_field)
            .child(
                row()
                    .w_full()
                    .h(32.0)
                    .gap(10.0)
                    .center_y()
                    .focusable(true)
                    .role("checkbox")
                    .checked(strict)
                    .label("Refuse a shared file")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| {
                        if let Some(ui) = &mut st.installed.tools.merge {
                            ui.strict = !ui.strict;
                        }
                    })
                    .child(super::check_box(u, strict, false))
                    .child(
                        text("Refuse when two mods replace the same file")
                            .font(SMALL)
                            .color(u.muted)
                            .a11y_hidden(),
                    ),
            );
        if let Some(error) = &ui.error {
            body = body.child(crate::notify::feedback::error_line(u, "installed-tool-apply-error", error));
        }
    }
    let mut actions = row().w_full().gap(8.0).center_y().child(spacer());
    match ui.stage {
        MergeStage::Done => {
            actions = actions.child(
                u.action("Done", ActionKind::Primary)
                    .key("merge-done")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(close),
            );
        }
        MergeStage::Running | MergeStage::Previewing => {
            actions = actions.child(
                u.action("Merging", ActionKind::Secondary)
                    .key("merge-running")
                    .disabled(true),
            );
        }
        MergeStage::Picking => {
            let ready = picks.len() >= 2 && !ui.name.text.trim().is_empty();
            actions = actions
                .child(
                    u.action("Cancel", ActionKind::Ghost)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(close),
                )
                .child(
                    u.action("Preview", ActionKind::Secondary)
                        .key("merge-preview")
                        .disabled(picks.len() < 2)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| preview_merge(st)),
                )
                .child(
                    u.action("Merge", ActionKind::Primary)
                        .key("merge-go")
                        .disabled(!ready)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| run_merge(st)),
                );
        }
    }
    Some(
        u.dialog_shell()
            .key("installed-merge")
            .role("dialog")
            .label("Merge mods")
            .w(520.0)
            .padding(24.0)
            .gap(16.0)
            .modal(true, close)
            .child(text("Merge mods").font(H1).color(u.ink))
            .child(body)
            .child(actions),
    )
}

fn preview_merge(st: &mut State) {
    let inputs = merge_inputs(st);
    if inputs.len() < 2 || st.demo || rune::is_headless() {
        return;
    }
    let addons = super::addons_folder(st);
    if let Some(ui) = &mut st.installed.tools.merge {
        ui.stage = MergeStage::Previewing;
        ui.error = None;
    }
    let wake = st.wake.clone();
    st.installed.tools.merge_preview.start(wake, move || {
        Ok(mod_merge::preview(&inputs, &addons))
    });
}

fn run_merge(st: &mut State) {
    if crate::screens::installed::waiting::refuse_held(st, crate::screens::installed::waiting::HELD_MERGE) {
        return;
    }
    if crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    let Some(game_root) = writable_game(st) else { return };
    let inputs = merge_inputs(st);
    let Some(ui) = &st.installed.tools.merge else { return };
    let name = ui.name.text.trim().to_string();
    let strict = ui.strict;
    if inputs.len() < 2 || name.is_empty() {
        return;
    }
    let workspace = crate::data::workspace_dir();
    let store = mod_merge::merged_path(&crate::data::data_dir());
    let library = super::library_file();
    if let Some(ui) = &mut st.installed.tools.merge {
        ui.stage = MergeStage::Running;
        ui.error = None;
    }
    let wake = st.wake.clone();
    st.installed.tools.merge_run.start(wake, move || {
        let merged =
            mod_merge::merge_mods(&game_root, &workspace, &store, &inputs, &name, strict, true)
                .map_err(|error| error.to_string())?;
        // The library entry is written on the worker too, so the frame only draws.
        let mut data = import::read_library(&library).unwrap_or_default();
        data.mods.retain(|meta| meta.slot != merged.slot);
        data.mods.push(ModMeta {
            slot: merged.slot,
            size_bytes: merged.bytes.max(1),
            name: merged.name.clone(),
            author: None,
            thumbnail_url: None,
            gamebanana_id: None,
            source: "merge".to_string(),
            recorded_at: String::new(),
        });
        let _ = import::write_library(&library, &data);
        Ok(merged)
    });
}

/// Remember the merged slot for Undo, now that the worker wrote its library entry.
fn apply_merge_record(st: &mut State, merged: &MergedMod) {
    st.installed.library_sig = None;
    st.installed.last_undo = Some(super::Undo::Unmerge(Box::new(merged.clone())));
    st.installed.undo_fresh = true;
}

/// Take a merge apart: its slot to the trash, its sources back on.
pub fn unmerge(st: &mut State, merged: &MergedMod) {
    if crate::screens::installed::waiting::refuse_held(st, crate::screens::installed::waiting::HELD_UNMERGE) {
        return;
    }
    if crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    let Some(game_root) = writable_game(st) else { return };
    let data = crate::data::data_dir();
    match mod_merge::unmerge(
        &game_root,
        &deadlock_core::trash::trash_root(&data),
        &mod_merge::merged_path(&data),
        merged,
        true,
    ) {
        Ok(back) => {
            st.installed.tools.merged = mod_merge::read_merged(&mod_merge::merged_path(&data));
            st.installed.library_sig = None;
            st.status = format!(
                "Took the merge apart. {} mod{} back on.",
                back.len(),
                if back.len() == 1 { "" } else { "s" }
            );
        }
        Err(error) => st.status = error.to_string(),
    }
    super::rescan(st);
}

// --- fonts ---------------------------------------------------------------------

/// Find a mod's fonts and offer to add them. Nothing is written until the button is pressed.
pub fn open_fonts(st: &mut State, slot: u16) {
    st.installed.menu = None;
    st.installed.tools.fonts = Some(FontUi {
        slot,
        fonts: Vec::new(),
        stage: Stage::Looking,
        error: None,
        installed: false,
    });
    if st.demo || rune::is_headless() {
        return;
    }
    let Some(addon) = st.addons.addons.iter().find(|addon| addon.slot == slot) else {
        return;
    };
    let path = addon.path(&super::addons_folder(st));
    let wake = st.wake.clone();
    st.installed.tools.fonts_job.start(wake, move || {
        fonts::fonts_in_vpk(&path).map_err(|error| error.to_string())
    });
}

pub fn fonts_dialog(s: &State, u: &Ui) -> Option<El> {
    let ui = s.installed.tools.fonts.as_ref()?;
    let name = s
        .addons
        .addons
        .iter()
        .find(|addon| addon.slot == ui.slot)
        .map(|addon| super::display_name(s, addon))
        .unwrap_or_else(|| format!("Slot {:02}", ui.slot));
    let close = |st: &mut State| st.installed.tools.fonts = None;
    let mut list = col().w_full().gap(2.0);
    for font in ui.fonts.iter().take(10) {
        list = list.child(
            row()
                .w_full()
                .h(28.0)
                .gap(8.0)
                .center_y()
                .child(text(font.family.clone()).font(BODY).color(u.prose).truncate().grow(1.0).min_w(0.0))
                .child(text(font.file_name.clone()).font(SMALL).color(u.faint).truncate()),
        );
    }
    let body = match ui.stage {
        Stage::Looking => "Looking for fonts inside the mod.".to_string(),
        Stage::Failed => ui
            .error
            .clone()
            .unwrap_or_else(|| "No fonts to add.".to_string()),
        Stage::Found => format!(
            "{} font{} under panorama/fonts. The game needs them listed in fonts.conf to use them.",
            ui.fonts.len(),
            if ui.fonts.len() == 1 { "" } else { "s" }
        ),
    };
    let installed = ui.installed;
    let has = !ui.fonts.is_empty();
    let mut actions = row().w_full().gap(8.0).center_y().child(spacer());
    if (has || installed) && ui.stage == Stage::Found {
        actions = actions.child(
            u.action(
                if installed { "Remove fonts" } else { "Add fonts" },
                if installed { ActionKind::Secondary } else { ActionKind::Primary },
            )
            .key("fonts-go")
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| {
                if installed {
                    remove_fonts(st)
                } else {
                    install_fonts(st)
                }
            }),
        );
    }
    actions = actions.child(
        u.action("Done", ActionKind::Ghost)
            .key("fonts-done")
            .cursor(rune::Cursor::Pointer)
            .on_press(close),
    );
    Some(
        u.dialog_shell()
            .key("installed-fonts")
            .role("dialog")
            .label(format!("Fonts in {name}"))
            .w(500.0)
            .padding(24.0)
            .gap(16.0)
            .modal(true, close)
            .child(text("Fonts".to_string()).font(H1).color(u.ink))
            .child(text(body).font(BODY).color(u.muted).wrap())
            .child(list)
            .child(actions),
    )
}

fn install_fonts(st: &mut State) {
    if crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    let Some(game_root) = writable_game(st) else { return };
    let Some(ui) = &st.installed.tools.fonts else { return };
    let slot = ui.slot;
    let Some(addon) = st.addons.addons.iter().find(|addon| addon.slot == slot) else {
        return;
    };
    let path = addon.path(&super::addons_folder(st));
    let backups = crate::data::data_dir().join("backups");
    let wake = st.wake.clone();
    st.installed.tools.font_install.start(wake, move || {
        fonts::install_fonts(&game_root, &path, slot, &backups, true)
            .map_err(|error| error.to_string())
    });
}

fn remove_fonts(st: &mut State) {
    if crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    let Some(game_root) = writable_game(st) else { return };
    let Some(ui) = &st.installed.tools.fonts else { return };
    let slot = ui.slot;
    let backups = crate::data::data_dir().join("backups");
    let wake = st.wake.clone();
    st.installed.tools.font_install.start(wake, move || {
        fonts::remove_fonts(&game_root, slot, &backups, true).map_err(|error| error.to_string())
    });
}

/// Whether a slot's fonts were added by Sigil, for the card menu's label.
pub fn has_fonts(st: &State, slot: u16) -> bool {
    st.game_root
        .as_ref()
        .map(|root| fonts::has_block(root, slot))
        .unwrap_or(false)
}

/// The merged record of a slot, for the card badge and the menu.
pub fn merged_for(s: &State, slot: u16) -> Option<&MergedMod> {
    let addons = super::addons_folder(s);
    s.installed.tools.merged.find_slot(&addons, slot)
}

/// A card's "Merged" chip.
pub fn merged_chip(s: &State, u: &Ui, slot: u16) -> Option<El> {
    let merged = merged_for(s, slot)?;
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
            .child(icon(&icons::LAYERS, 12.0).color(u.muted).a11y_hidden())
            .child(
                text(format!("{} mods", merged.sources.len()))
                    .font(MICRO)
                    .color(u.prose)
                    .a11y_hidden(),
            ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use deadlock_core::mod_health::{Issue, IssueKind};

    fn review(slot: u16, kinds: &[IssueKind]) -> SlotReview {
        SlotReview {
            slot,
            issues: kinds
                .iter()
                .map(|kind| Issue { kind: *kind, path: format!("p/{slot}"), now_at: None })
                .collect(),
            ..SlotReview::default()
        }
    }

    #[test]
    fn only_outdated_data_counts_for_the_bulk_switch() {
        let reviews = vec![
            review(9, &[IssueKind::Skeleton]),
            review(2, &[IssueKind::Moved, IssueKind::Gone]),
            review(4, &[IssueKind::Gone, IssueKind::Skeleton]),
            review(7, &[]),
        ];
        assert_eq!(outdated_slots(&reviews), vec![4, 9]);
        assert!(outdated_slots(&[]).is_empty());
    }
}

/// `w4a-identify-name`: an unknown mod, with Search GameBanana by name and the checksum opt-in.
pub fn scene_identify_name(st: &mut State) {
    crate::app::scene_demo_installed(st);
    st.installed.tools.identify = Some(IdentifyUi {
        slot: 9,
        link: TextEdit::new(""),
        found: None,
        stage: Stage::Failed,
        error: None,
        job: Job::default(),
        pending: None,
    });
}
