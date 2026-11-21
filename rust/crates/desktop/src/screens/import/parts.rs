//! The Add mods part of the local import review: what the files say, and several loose VPKs
//! as one mod with parts.
//!
//! - Deadlock Mod Manager (behaviour only): the Add mods dialog reads the dropped files to
//!   suggest a category, says what each part holds, finds a map's name and shows a cover;
//!   several loose VPKs import as one mod whose parts switch in Manage files.
//! - Grimoire: names each member of a pack from its own imprint (`import::local_mods`).
//! - Sigil: when a local review lands, a worker reads each loose VPK's tree
//!   (`deadlock_core::import::parts`) and stages a cover in the cache folder. The review then
//!   shows a summary per file, a category (detected, the player can change it), the hero, the
//!   map's console name, the cover, and for two or more files the "Import as one mod" switch
//!   with the mod's name. Pressing Import is still the only confirm; after the copy, the
//!   answers go into Sigil's notes (`library-extra.json`): one variant group for the parts, a
//!   label per part, the category and the cover. Nothing here writes to the game folder.

use std::path::{Path, PathBuf};

use deadlock_core::import::parts::{
    self, CoverSource, LocalCategory, PartContents, PartWrite,
};
use deadlock_core::import::{ImportAction, ImportReport, SourceKind};
use rune::{El, FileDialogOptions, SurfaceFit, TextEdit, Tx, col, el, icon, row, spacer, text};

use super::locales::t;
use crate::app::State;
use crate::icons;
use crate::jobs::Job;
use crate::screens::settings::i18n::fill;
use crate::ui::{ActionKind, BODY, LABEL, SMALL, Ui};

/// Where a staged cover came from, for the line under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoverFrom {
    Picture,
    CardArt,
    Render,
}

/// One file's answer from the worker: its plan index, what it holds, a staged cover.
pub type ReadPart = (usize, PartContents, Option<(PathBuf, CoverFrom)>);

pub struct Parts {
    /// The paths the player picked or dropped, so Add files can plan them again with more.
    pub paths: Vec<PathBuf>,
    /// Per plan item: what its file holds, once read. `None` for archives and skips.
    pub contents: Vec<Option<PartContents>>,
    /// Per plan item: a cover staged in the cache folder.
    pub covers: Vec<Option<(PathBuf, CoverFrom)>>,
    pub job: Job<Vec<ReadPart>>,
    pub reading: bool,
    /// Import the chosen loose files as one mod with parts.
    pub as_one: bool,
    pub name: TextEdit,
    /// The player's own pick; `None` follows what the files say.
    pub picked: Option<LocalCategory>,
    /// Set by Import: what to record for each slot once the copy lands.
    pub pending: Vec<(u16, PartWrite)>,
    pub pending_group: Option<String>,
}

impl Default for Parts {
    fn default() -> Self {
        Parts {
            paths: Vec::new(),
            contents: Vec::new(),
            covers: Vec::new(),
            job: Job::default(),
            reading: false,
            as_one: false,
            name: TextEdit::new(""),
            picked: None,
            pending: Vec::new(),
            pending_group: None,
        }
    }
}

// --- hooks from the import flow ------------------------------------------------

/// `import::open_local`: remember what was picked.
pub fn on_open_local(st: &mut State, paths: &[PathBuf]) {
    st.installed.import.parts = Parts {
        paths: merge_paths(Vec::new(), paths),
        ..Parts::default()
    };
}

fn merge_paths(mut have: Vec<PathBuf>, more: &[PathBuf]) -> Vec<PathBuf> {
    for path in more {
        if !have.contains(path) {
            have.push(path.clone());
        }
    }
    have
}

/// `import::set_plan`: read the loose VPKs of a local review on a worker.
pub fn on_plan(st: &mut State) {
    let Some(dialog) = st.installed.import.dialog.as_ref() else {
        return;
    };
    if dialog.kind != SourceKind::Local {
        return;
    }
    let Some(plan) = dialog.plan.as_ref() else {
        return;
    };
    let files: Vec<(usize, PathBuf)> = plan
        .items
        .iter()
        .enumerate()
        .filter(|(_, item)| matches!(item.action, ImportAction::Copy { .. }) && !item.source.archive)
        .map(|(index, item)| (index, item.source.file.clone()))
        .collect();
    let count = plan.items.len();
    let chosen: Vec<&deadlock_core::import::ForeignMod> = files
        .iter()
        .filter_map(|(index, _)| plan.items.get(*index).map(|item| &item.source))
        .collect();
    let as_one = parts::default_as_one(&chosen);
    let name = parts::group_name(&chosen);
    let p = &mut st.installed.import.parts;
    p.contents = vec![None; count];
    p.covers = vec![None; count];
    p.as_one = as_one;
    p.name = TextEdit::new(&name);
    p.picked = None;
    if files.is_empty() || st.demo || rune::is_headless() {
        return;
    }
    p.reading = true;
    let cache = crate::data::data_dir().join("cache").join("import-covers");
    let wake = st.wake.clone();
    p.job.start(wake, move || Ok(read_all(&files, &cache)));
}

/// The worker: each file's tree, and a cover staged as a file the image cache can open.
fn read_all(files: &[(usize, PathBuf)], cache: &Path) -> Vec<ReadPart> {
    let assets = deadlock_core::locker::LockerAssetPaths::discover();
    let _ = std::fs::create_dir_all(cache);
    files
        .iter()
        .map(|(index, file)| {
            let contents = parts::read_part(file);
            let cover = parts::find_cover(std::slice::from_ref(&contents), &assets)
                .and_then(|source| stage_cover(&source, cache));
            (*index, contents, cover)
        })
        .collect()
}

fn stage_cover(source: &CoverSource, cache: &Path) -> Option<(PathBuf, CoverFrom)> {
    let uid = deadlock_core::mod_extras::new_uid();
    match source {
        CoverSource::File(path) | CoverSource::Render(path) => {
            let extension = path
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("png")
                .to_ascii_lowercase();
            let target = cache.join(format!("{uid}.{extension}"));
            std::fs::copy(path, &target).ok()?;
            let from = if matches!(source, CoverSource::File(_)) {
                CoverFrom::Picture
            } else {
                CoverFrom::Render
            };
            Some((target, from))
        }
        CoverSource::Texture { vpk, entry } => {
            let thumb = parts::decode_texture(vpk, entry, 512).ok()?;
            let image = image::RgbaImage::from_raw(thumb.width, thumb.height, thumb.rgba)?;
            let target = cache.join(format!("{uid}.png"));
            image.save_with_format(&target, image::ImageFormat::Png).ok()?;
            Some((target, CoverFrom::CardArt))
        }
    }
}

/// `import::poll`: take the worker's answer and load the covers' pixels.
pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.installed.import.parts.job.try_take() {
        let p = &mut st.installed.import.parts;
        p.reading = false;
        for (index, contents, cover) in result.unwrap_or_default() {
            if let Some(slot) = p.contents.get_mut(index) {
                *slot = Some(contents);
            }
            if let Some(slot) = p.covers.get_mut(index) {
                *slot = cover;
            }
        }
        changed = true;
    }
    let covers: Vec<PathBuf> = st
        .installed
        .import
        .parts
        .covers
        .iter()
        .flatten()
        .map(|(path, _)| path.clone())
        .collect();
    for path in covers {
        let key = path.to_string_lossy().into_owned();
        if st.thumbnails.contains_key(&key) {
            continue;
        }
        match st.images.as_ref() {
            Some(cache) if path.is_file() => {
                let image = cache.file(&path, 1.0, 1.0);
                st.thumbnails.insert(key, image.pixels().clone());
                changed = true;
            }
            Some(_) => {}
            None => {
                if !st.pending_thumbs.iter().any(|(known, _)| *known == key)
                    && let Ok(bytes) = std::fs::read(&path)
                {
                    st.pending_thumbs.push((key, bytes));
                }
            }
        }
    }
    changed
}

/// Indices of the plan's chosen loose files, in plan order.
fn chosen(s: &State) -> Vec<usize> {
    let Some(dialog) = s.installed.import.dialog.as_ref() else {
        return Vec::new();
    };
    let Some(plan) = dialog.plan.as_ref() else {
        return Vec::new();
    };
    plan.items
        .iter()
        .enumerate()
        .filter(|(index, item)| {
            dialog.selected.get(*index).copied().unwrap_or(false)
                && matches!(item.action, ImportAction::Copy { .. })
                && !item.source.archive
        })
        .map(|(index, _)| index)
        .collect()
}

fn read_contents(s: &State, indices: &[usize]) -> Vec<PartContents> {
    indices
        .iter()
        .filter_map(|index| s.installed.import.parts.contents.get(*index).cloned().flatten())
        .collect()
}

/// The category the chosen files get: the player's pick, else what they say.
pub fn category(s: &State) -> Option<LocalCategory> {
    let p = &s.installed.import.parts;
    p.picked.or_else(|| parts::suggest_category(&read_contents(s, &chosen(s))))
}

fn grouping(s: &State) -> bool {
    s.installed.import.parts.as_one && chosen(s).len() >= 2
}

/// `import::confirm`, before the plan is copied: name every part after the mod when they
/// import as one, and keep what to record once the copy lands.
pub fn prepare(st: &mut State) {
    let indices = chosen(st);
    let group = grouping(st);
    let picked = category(st);
    let name = st.installed.import.parts.name.text.trim().to_string();
    let contents: Vec<Option<PartContents>> = st.installed.import.parts.contents.clone();
    let covers = st.installed.import.parts.covers.clone();
    let Some(dialog) = st.installed.import.dialog.as_mut() else {
        return;
    };
    if dialog.kind != SourceKind::Local {
        return;
    }
    let Some(plan) = dialog.plan.as_mut() else {
        return;
    };
    let mods: Vec<&deadlock_core::import::ForeignMod> =
        indices.iter().filter_map(|i| plan.items.get(*i).map(|item| &item.source)).collect();
    let labels = parts::part_labels(&mods);
    let read: Vec<PartContents> = indices.iter().filter_map(|i| contents.get(*i).cloned().flatten()).collect();
    let hero = parts::common_hero(&read);
    let group_cover = indices.iter().find_map(|i| covers.get(*i).cloned().flatten());
    let mut pending = Vec::new();
    for (n, index) in indices.iter().enumerate() {
        let Some(item) = plan.items.get_mut(*index) else { continue };
        let ImportAction::Copy { slot } = item.action else { continue };
        let own = contents.get(*index).cloned().flatten();
        let (category, part_hero) = if group {
            (picked, hero.clone())
        } else {
            let one = own.as_ref().map(std::slice::from_ref).unwrap_or_default();
            (parts::suggest_category(one), own.as_ref().and_then(|c| c.hero().map(str::to_string)))
        };
        let cover = if group { group_cover.clone() } else { covers.get(*index).cloned().flatten() };
        pending.push((
            slot,
            PartWrite {
                slot,
                size_bytes: 0,
                gamebanana_id: item.source.gamebanana_id,
                label: group.then(|| labels.get(n).cloned()).flatten(),
                category,
                hero: part_hero,
                image: cover.map(|(path, _)| path),
            },
        ));
        if group && !name.is_empty() {
            item.source.name = name.clone();
        }
    }
    let p = &mut st.installed.import.parts;
    p.pending = pending;
    p.pending_group = group.then(|| format!("parts-{}", deadlock_core::mod_extras::new_uid()));
}

/// `import::finish`, after a copy landed: record the parts, categories and covers in Sigil's
/// notes, for the slots that were really written.
pub fn after_import(st: &mut State, report: &ImportReport) {
    let pending = std::mem::take(&mut st.installed.import.parts.pending);
    let group = st.installed.import.parts.pending_group.take();
    if pending.is_empty() || st.demo || rune::is_headless() {
        return;
    }
    let Some(game_root) = st.game_root.clone() else { return };
    let addons = deadlock_core::deploy::addons_dir(&game_root);
    let images = crate::data::data_dir().join("mod-images");
    let mut writes = Vec::new();
    for (slot, mut write) in pending {
        if !report.imported.iter().any(|(done, _)| *done == slot) {
            continue;
        }
        write.size_bytes = slot_size(&addons, slot);
        if let Some(staged) = write.image.take() {
            write.image = keep_cover(&staged, &images, slot);
        }
        writes.push(write);
    }
    let group = group.filter(|_| writes.len() >= 2);
    if writes.is_empty() {
        return;
    }
    if let Err(error) = parts::write_parts(&crate::screens::installed::extras_path(), group.as_deref(), &writes) {
        st.status = fill(t("The mods were imported, but their details could not be saved: {}"), &[&error.to_string()]);
    }
}

fn slot_size(addons: &Path, slot: u16) -> u64 {
    let size = |path: PathBuf| std::fs::metadata(path).map(|m| m.len()).ok();
    size(deadlock_core::addons::slot_path(addons, slot, "_dir.vpk"))
        .or_else(|| size(deadlock_core::addons::slot_path(addons, slot, "_dir.vpk.disabled")))
        .unwrap_or(0)
}

/// Copy a staged cover into Sigil's picture folder, as the picture editor does.
fn keep_cover(staged: &Path, images: &Path, slot: u16) -> Option<PathBuf> {
    let extension = staged.extension().and_then(|ext| ext.to_str()).unwrap_or("png");
    std::fs::create_dir_all(images).ok()?;
    let target = images.join(format!("slot{slot:02}-{}.{extension}", deadlock_core::mod_extras::new_uid()));
    std::fs::copy(staged, &target).ok()?;
    Some(target)
}

/// Add files to an open local review: pick one, then plan everything again.
fn add_files(_st: &mut State, tx: &mut Tx) {
    let options = FileDialogOptions {
        title: t("Add files").to_string(),
        ..FileDialogOptions::default()
    }
    .filter("Mods", &["vpk", "zip", "7z", "rar"]);
    tx.open_file(options, |st: &mut State, result, _tx| match result {
        Ok(Some(path)) => replan(st, vec![path]),
        Ok(None) => {}
        Err(error) => st.status = format!("The file picker could not open: {error}"),
    });
}

fn add_folder(_st: &mut State, tx: &mut Tx) {
    let options = FileDialogOptions {
        title: t("Add a folder").to_string(),
        ..FileDialogOptions::default()
    };
    tx.open_folder(options, |st: &mut State, result, _tx| match result {
        Ok(Some(path)) => replan(st, vec![path]),
        Ok(None) => {}
        Err(error) => st.status = format!("The file picker could not open: {error}"),
    });
}

fn replan(st: &mut State, more: Vec<PathBuf>) {
    let paths = merge_paths(st.installed.import.parts.paths.clone(), &more);
    super::open_local(st, paths);
}

// --- view ----------------------------------------------------------------------

/// What one file holds, for its row in the review.
pub fn row_note(s: &State, index: usize) -> Option<String> {
    let p = &s.installed.import.parts;
    let contents = p.contents.get(index)?.as_ref()?;
    Some(summary(contents))
}

/// "Haze skin, 3 models", "Map street_test", "12 voice lines", "Not a readable VPK".
pub fn summary(c: &PartContents) -> String {
    if !c.readable {
        return t("Not a readable VPK").to_string();
    }
    let mut bits: Vec<String> = Vec::new();
    if let Some(map) = c.map_names.first() {
        bits.push(fill(t("Map {}"), &[map]));
    } else if !c.skin_heroes.is_empty() {
        bits.push(fill(t("{} skin"), &[&c.skin_heroes.join(", ")]));
    }
    let counts = [
        (c.models, t("1 model"), t("{} models")),
        (c.particles, t("1 effect"), t("{} effects")),
        (c.voice_lines, t("1 voice line"), t("{} voice lines")),
        (c.ability_sounds, t("1 ability sound"), t("{} ability sounds")),
        (c.weapon_sounds, t("1 weapon sound"), t("{} weapon sounds")),
        (c.other_sounds, t("1 sound"), t("{} sounds")),
        (c.hud_files, t("1 HUD file"), t("{} HUD files")),
    ];
    for (count, one, many) in counts {
        match count {
            0 => {}
            1 => bits.push(one.to_string()),
            n => bits.push(fill(many, &[&n.to_string()])),
        }
    }
    if bits.is_empty() {
        bits.push(fill(t("{} files"), &[&c.entries.to_string()]));
    }
    bits.truncate(3);
    bits.join(", ")
}

/// The panel above the review list for a local import: one mod or several, name, category,
/// hero, map and cover. `None` for other sources and before the plan.
pub fn panel(s: &State, u: &Ui) -> Option<El> {
    let dialog = s.installed.import.dialog.as_ref()?;
    if dialog.kind != SourceKind::Local {
        return None;
    }
    dialog.plan.as_ref()?;
    let p = &s.installed.import.parts;
    let indices = chosen(s);
    if indices.is_empty() {
        return None;
    }
    let group = grouping(s);
    let mut panel = col()
        .key("import-parts")
        .w_full()
        .gap(12.0)
        .padding(14.0)
        .radius(crate::ui::r(10.0))
        .fill(u.over.alpha(0.03))
        .border(1.0, u.hair);

    if indices.len() >= 2 {
        let line = if p.as_one {
            fill(t("{} files become the parts of one mod. Switch each part in Manage files."), &[&indices.len().to_string()])
        } else {
            t("Each file becomes a mod of its own.").to_string()
        };
        panel = panel.child(
            row()
                .key("import-parts-one")
                .w_full()
                .gap(12.0)
                .center_y()
                .focusable(true)
                .role("switch")
                .checked(p.as_one)
                .label(t("Import as one mod"))
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| st.installed.import.parts.as_one = !st.installed.import.parts.as_one)
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(text(t("Import as one mod")).font(LABEL).color(u.ink))
                        .child(text(line).font(SMALL).color(u.muted).wrap()),
                )
                .child(u.toggle(p.as_one).a11y_hidden()),
        );
        if p.as_one {
            panel = panel.child(
                row()
                    .w_full()
                    .gap(12.0)
                    .center_y()
                    .child(text(t("Name")).font(SMALL).color(u.muted).w(72.0))
                    .child(
                        row()
                            .grow(1.0)
                            .min_w(0.0)
                            .h(32.0)
                            .px(10.0)
                            .center_y()
                            .radius(crate::ui::r(6.0))
                            .fill(u.field)
                            .border(1.0, u.hair)
                            .child(
                                rune::text_input(&p.name, BODY)
                                    .key("import-parts-name")
                                    .label(t("Name"))
                                    .single_line()
                                    .grow(1.0)
                                    .color(u.ink)
                                    .placeholder(t("Name of the mod"), u.faint)
                                    .caret(u.accent, u.accent.alpha(0.20))
                                    .on_edit(|st: &mut State| &mut st.installed.import.parts.name),
                            ),
                    ),
            );
        }
    }

    let read = read_contents(s, &indices);
    if p.reading && read.is_empty() {
        panel = panel.child(text(t("Reading the files.")).font(SMALL).color(u.faint));
        return Some(panel);
    }

    if group || indices.len() == 1 {
        let suggested = parts::suggest_category(&read);
        let shown = p.picked.or(suggested);
        let mut chips = row().grow(1.0).min_w(0.0).gap(6.0).flex_wrap();
        for option in LocalCategory::ALL {
            let on = shown == Some(option);
            chips = chips.child(
                row()
                    .key(format!("import-parts-cat-{}", option.id()))
                    .h(26.0)
                    .px(10.0)
                    .center_y()
                    .radius(crate::ui::r(13.0))
                    .fill(if on { u.accent.alpha(0.18) } else { u.over.alpha(0.03) })
                    .border(1.0, if on { u.accent.alpha(0.5) } else { u.hair })
                    .hover(u.hover)
                    .focusable(true)
                    .role("radio")
                    .checked(on)
                    .label(t(option.label()))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        let p = &mut st.installed.import.parts;
                        p.picked = if p.picked == Some(option) { None } else { Some(option) };
                    })
                    .child(text(t(option.label())).font(SMALL).color(if on { u.ink } else { u.muted }).a11y_hidden()),
            );
        }
        let hint = match (p.picked, suggested) {
            (Some(_), _) => t("Your pick."),
            (None, Some(_)) => t("Read from the files."),
            (None, None) => t("The files could be several things. Pick one, or leave it."),
        };
        panel = panel.child(
            row()
                .w_full()
                .gap(12.0)
                .items_start()
                .child(text(t("Category")).font(SMALL).color(u.muted).w(72.0).mt(5.0))
                .child(col().grow(1.0).min_w(0.0).gap(6.0).child(chips).child(text(hint).font(SMALL).color(u.faint))),
        );
        let hero = parts::common_hero(&read);
        if let Some(hero) = hero {
            panel = panel.child(fact(u, t("Hero"), hero));
        }
        if let Some(map) = parts::map_name(&read) {
            panel = panel.child(fact(u, t("Map"), fill(t("{}: start it with map {} in the console."), &[&map, &map])));
        }
        let cover = indices
            .iter()
            .find_map(|i| p.covers.get(*i).cloned().flatten());
        panel = panel.child(cover_row(s, u, cover));
    }
    Some(panel)
}

fn fact(u: &Ui, label: &str, value: String) -> El {
    row()
        .w_full()
        .gap(12.0)
        .center_y()
        .child(text(label.to_string()).font(SMALL).color(u.muted).w(72.0))
        .child(text(value).font(SMALL).color(u.ink).grow(1.0).min_w(0.0).wrap())
}

fn cover_row(s: &State, u: &Ui, cover: Option<(PathBuf, CoverFrom)>) -> El {
    let pixels = cover
        .as_ref()
        .and_then(|(path, _)| s.thumbnails.get(&path.to_string_lossy().into_owned()));
    let frame = el().size(96.0, 54.0).radius(crate::ui::r(6.0)).a11y_hidden();
    let preview = match pixels {
        Some(pixels) => frame.pixels_fit(pixels, SurfaceFit::Fit).fill(u.over.alpha(0.04)),
        None => frame
            .fill(u.over.alpha(0.04))
            .center()
            .child(icon(&icons::PACKAGE, 16.0).color(u.faint.alpha(0.6))),
    };
    let line = match cover.as_ref().map(|(_, from)| *from) {
        Some(CoverFrom::Picture) => t("A picture that came with the files."),
        Some(CoverFrom::CardArt) => t("The hero card this skin replaces."),
        Some(CoverFrom::Render) => t("The hero's art, since the skin has no picture of its own."),
        None => t("No picture in the files. Pick one later with Edit on the card."),
    };
    row()
        .key("import-parts-cover")
        .w_full()
        .gap(12.0)
        .center_y()
        .child(text(t("Cover")).font(SMALL).color(u.muted).w(72.0))
        .child(preview)
        .child(text(line).font(SMALL).color(u.faint).grow(1.0).min_w(0.0).wrap())
}

/// Add files and Add a folder, for the review's footer.
pub fn add_buttons(s: &State, u: &Ui) -> Option<El> {
    let dialog = s.installed.import.dialog.as_ref()?;
    if dialog.kind != SourceKind::Local || dialog.stage != super::Stage::Review {
        return None;
    }
    Some(
        row()
            .grow(1.0)
            .gap(8.0)
            .center_y()
            .child(
                u.icon_action(&icons::PLUS, t("Add files"), ActionKind::Ghost)
                    .key("import-add-files")
                    .cursor(rune::Cursor::Pointer)
                    .on_press_tx(add_files),
            )
            .child(
                u.icon_action(&icons::FOLDER_OPEN, t("Add a folder"), ActionKind::Ghost)
                    .key("import-add-folder")
                    .cursor(rune::Cursor::Pointer)
                    .on_press_tx(add_folder),
            )
            .child(spacer()),
    )
}

// --- fixtures ------------------------------------------------------------------

/// A local review of three loose files from one folder, read as a Haze skin with voice lines,
/// for the `gaps-add-mods` scene. No file is read or written.
pub fn stage_scene(st: &mut State, separate: bool) {
    use deadlock_core::import::{ForeignMod, ImportPlan, PlannedImport};
    let folder = PathBuf::from("/Mods/Neon Haze");
    let files = [
        ("Neon Haze body", "body_dir.vpk", 6),
        ("Neon Haze voice", "voice_dir.vpk", 8),
        ("Neon Haze card", "card_dir.vpk", 10),
    ];
    let items: Vec<PlannedImport> = files
        .iter()
        .map(|(name, file, slot)| PlannedImport {
            source: ForeignMod {
                name: name.to_string(),
                file: folder.join(file),
                ..ForeignMod::default()
            },
            action: ImportAction::Copy { slot: *slot },
        })
        .collect();
    let plan = ImportPlan {
        kind: SourceKind::Local,
        addons_dir: PathBuf::new(),
        items,
    };
    let mut dialog = super::Dialog::new(SourceKind::Local);
    dialog.selected = vec![true; plan.items.len()];
    dialog.plan = Some(plan);
    dialog.stage = super::Stage::Review;
    st.installed.import.dialog = Some(dialog);
    on_plan(st);
    let mk = |paths: &[&str]| {
        let mut part = PartContents {
            readable: true,
            ..PartContents::default()
        };
        for path in paths {
            part.count(path);
        }
        part.skin_heroes = deadlock_core::mod_heroes::model_heroes(paths.iter().copied());
        part
    };
    let body = mk(&[
        "models/heroes_wip/haze/haze.vmdl_c",
        "models/heroes_wip/haze/haze_gun.vmdl_c",
        "materials/models/heroes_wip/haze/haze_body_color_psd.vtex_c",
    ]);
    let voice = mk(&["sounds/vo/haze/haze_kill_01.vsnd_c", "sounds/vo/haze/haze_kill_02.vsnd_c"]);
    let card = mk(&["panorama/images/heroes/haze_card_psd.vtex_c"]);
    let p = &mut st.installed.import.parts;
    p.contents = vec![Some(body), Some(voice), Some(card)];
    if separate {
        p.as_one = false;
    }
    let art = deadlock_core::locker::LockerAssetPaths::discover();
    let render = deadlock_core::locker::roster_hero("Haze").and_then(|roster| {
        deadlock_core::locker::resolve_render(
            &deadlock_core::locker::LockerHero {
                codename: roster.codenames.first().copied().unwrap_or_default().to_string(),
                display_name: roster.name.to_string(),
                render_path: None,
                name_art_path: None,
                mod_count: 0,
                asset_count: 0,
                in_catalog: true,
                recolor_paths: Vec::new(),
            },
            &art,
        )
    });
    p.covers = vec![render.map(|path| (path, CoverFrom::Render)), None, None];
}
