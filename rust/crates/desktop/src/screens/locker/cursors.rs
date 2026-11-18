//! The Locker's cursor packs (Grimoire's Cursor tab, MIT). Deadlock Mod Manager has none.
//!
//! Cursor mods are loose BMP files over the game's `citadel/resource/cursors` folder, not VPKs,
//! so one pack (or Deadlock's own set) is on at a time. Packs live in `<data>/cursor-packs`
//! (`deadlock_core::cursors`). The page lists them as cards with a preview, imports a folder, a
//! cursor file's folder or an archive, and deletes packs. Pressing a card is the player's
//! request to apply it: `cursors::apply` backs up Deadlock's files and writes the pack, and the
//! Deadlock card runs `cursors::restore`. Both run on a worker and refuse while the game runs.
//!
//! Previews are decoded on the same worker from the pack's BMPs into the shared thumbnail
//! cache, once per load, so a frame only draws pixels it already has.

use std::path::{Path, PathBuf};

use deadlock_core::cursors::{self, CursorPacksState};
use rune::{El, FileDialogOptions, SurfaceFit, Tx, col, el, icon, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::jobs::Job;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{ActionKind, BODY, BODY_MED, SMALL, Ui};

/// A decoded preview: thumbnail key, width, height, RGBA.
pub type Preview = (String, u32, u32, Vec<u8>);

/// What one read of the store returns.
pub struct Read {
    pub state: CursorPacksState,
    pub previews: Vec<Preview>,
    /// Deadlock's own cursor files, for its card.
    pub stock: Vec<String>,
}

#[derive(Default)]
pub struct Cursors {
    pub state: Option<CursorPacksState>,
    /// Deadlock's own cursor files the last read found.
    pub stock: Vec<String>,
    pub error: Option<String>,
    pub read: Job<Read>,
    pub loaded: bool,
    /// An import, apply, restore or delete in flight, with the line it reports.
    pub work: Job<String>,
    /// The pack whose delete is being asked about.
    pub confirm_delete: Option<String>,
}

pub fn store() -> PathBuf {
    crate::data::data_dir().join(cursors::CURSOR_PACKS_DIR)
}

/// The thumbnail key of one preview image.
fn preview_key(pack: &str, name: &str) -> String {
    format!("cursor-preview:{pack}:{name}")
}

/// Decode a 24 or 32 bit uncompressed BMP (or one with bit field masks) into RGBA. Cursor files
/// are tiny, so this stays in the app instead of pulling another image codec in. A 32 bit file
/// whose alpha is all zero reads as opaque, as Windows treats it.
pub fn decode_bmp(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let u16_at = |at: usize| bytes.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |at: usize| bytes.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    if bytes.get(0..2)? != b"BM" {
        return None;
    }
    let offset = u32_at(10)? as usize;
    let width = u32_at(18)? as i32;
    let raw_height = u32_at(22)? as i32;
    let bits = u16_at(28)?;
    let compression = u32_at(30)?;
    if width <= 0 || width > 512 || raw_height == 0 || raw_height.abs() > 512 || !(compression == 0 || compression == 3) {
        return None;
    }
    let (w, h) = (width as usize, raw_height.unsigned_abs() as usize);
    let bpp = match bits {
        24 => 3,
        32 => 4,
        _ => return None,
    };
    let stride = (w * bpp).div_ceil(4) * 4;
    if bytes.len() < offset + stride * h {
        return None;
    }
    let mut out = vec![0u8; w * h * 4];
    let mut any_alpha = false;
    for y in 0..h {
        let src_row = if raw_height > 0 { h - 1 - y } else { y };
        let row_at = offset + src_row * stride;
        for x in 0..w {
            let p = row_at + x * bpp;
            let o = (y * w + x) * 4;
            out[o] = bytes[p + 2];
            out[o + 1] = bytes[p + 1];
            out[o + 2] = bytes[p];
            out[o + 3] = if bpp == 4 { bytes[p + 3] } else { 255 };
            any_alpha |= bpp == 4 && bytes[p + 3] != 0;
        }
    }
    if bpp == 4 && !any_alpha {
        for alpha in out.iter_mut().skip(3).step_by(4) {
            *alpha = 255;
        }
    }
    Some((w as u32, h as u32, out))
}

/// The images a card shows: `cursor.bmp` first, then up to three others, size variants left out.
fn preview_names(files: &[String]) -> Vec<String> {
    let mut names: Vec<String> = files
        .iter()
        .filter(|name| name.ends_with(".bmp") && !name.contains("_vsz"))
        .cloned()
        .collect();
    names.sort_by_key(|name| (name != "cursor.bmp", name.clone()));
    names.truncate(4);
    names
}

fn previews_in(folder: &Path, key: &str, names: &[String]) -> Vec<Preview> {
    names
        .iter()
        .filter_map(|name| {
            let bytes = std::fs::read(folder.join(name)).ok()?;
            let (w, h, rgba) = decode_bmp(&bytes)?;
            Some((preview_key(key, name), w, h, rgba))
        })
        .collect()
}

/// Read the store and decode previews on a worker. Deadlock's own set previews from the stock
/// copies when a pack is on, else from the game folder.
pub fn start_read(st: &mut State) {
    if st.locker.cursors.read.is_running() || st.demo || rune::is_headless() {
        return;
    }
    let game = st.game_root.clone();
    let wake = st.wake.clone();
    st.locker.cursors.read.start(wake, move || {
        let store = store();
        let state = cursors::list(&store).map_err(|e| e.to_string())?;
        let mut previews = Vec::new();
        for pack in &state.packs {
            previews.extend(previews_in(&cursors::pack_folder(&store, &pack.id), &pack.id, &preview_names(&pack.files)));
        }
        let stock_folder = if state.active_id.is_some() {
            Some(store.join("stock"))
        } else {
            game.as_deref().map(cursors::cursor_folder)
        };
        let mut stock = Vec::new();
        if let Some(folder) = stock_folder {
            let names: Vec<String> = std::fs::read_dir(&folder)
                .map(|entries| {
                    entries
                        .filter_map(|e| e.ok()?.file_name().into_string().ok())
                        .map(|n| n.to_lowercase())
                        .filter(|n| cursors::is_cursor_file_name(n))
                        .collect()
                })
                .unwrap_or_default();
            previews.extend(previews_in(&folder, "stock", &preview_names(&names)));
            stock = names;
        }
        Ok(Read { state, previews, stock })
    });
}

/// Take finished work. Called from the Locker's poll.
pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if st.screen == crate::model::Screen::Locker && !st.locker.cursors.loaded {
        st.locker.cursors.loaded = true;
        start_read(st);
    }
    if let Some(result) = st.locker.cursors.read.try_take() {
        match result {
            Ok(read) => {
                if let Some(cache) = st.images.as_ref() {
                    for (key, w, h, rgba) in read.previews {
                        let image = cache.rgba(w, h, rgba);
                        st.thumbnails.insert(key, image.pixels().clone());
                    }
                }
                st.locker.cursors.state = Some(read.state);
                st.locker.cursors.stock = read.stock;
                st.locker.cursors.error = None;
            }
            Err(error) => st.locker.cursors.error = Some(error),
        }
        changed = true;
    }
    if let Some(result) = st.locker.cursors.work.try_take() {
        match result {
            Ok(line) => crate::notify::push(st, crate::notify::Notice::success(line)),
            Err(error) => crate::notify::push(st, crate::notify::Notice::error(error)),
        }
        start_read(st);
        changed = true;
    }
    changed
}

fn run(st: &mut State, work: impl FnOnce(&Path) -> Result<String, String> + Send + 'static) {
    if st.locker.cursors.work.is_running() {
        return;
    }
    if st.demo {
        st.status = tr("Demo library: nothing was changed.").to_string();
        return;
    }
    let wake = st.wake.clone();
    st.locker.cursors.work.start(wake, move || work(&store()));
}

/// Apply a pack, or Deadlock's own set with `None`. The press is the approval.
fn select(st: &mut State, id: Option<String>) {
    let active = st.locker.cursors.state.as_ref().and_then(|s| s.active_id.clone());
    if active == id {
        return;
    }
    if crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    let Some(game_root) = st.game_root.clone() else {
        st.status = tr("Set the game folder in Settings first.").to_string();
        return;
    };
    run(st, move |store| match id {
        Some(id) => cursors::apply(store, &id, &game_root, true)
            .map(|plan| fill(tr("{} is on. Deadlock uses it from the next start."), &[&plan.pack_name]))
            .map_err(|e| e.to_string()),
        None => cursors::restore(store, &game_root, true)
            .map(|_| tr("Deadlock's own cursors are back.").to_string())
            .map_err(|e| e.to_string()),
    });
}

/// Pick an archive or a cursor file; a file imports every cursor file in its folder.
fn pick_file(_st: &mut State, tx: &mut Tx) {
    let options = FileDialogOptions { title: tr("Import a cursor pack").to_string(), ..FileDialogOptions::default() }
        .filter(tr("Cursor packs"), &["zip", "7z", "rar", "bmp", "res"]);
    tx.open_file(options, |st: &mut State, result, _tx| match result {
        Ok(Some(path)) => import_path(st, path),
        Ok(None) => {}
        Err(error) => st.status = fill(tr("The file picker could not open: {}"), &[&error.to_string()]),
    });
}

/// Pick a folder of cursor files.
fn pick_folder(_st: &mut State, tx: &mut Tx) {
    let options = FileDialogOptions { title: tr("Import a cursor folder").to_string(), ..FileDialogOptions::default() };
    tx.open_folder(options, |st: &mut State, result, _tx| match result {
        Ok(Some(path)) => import_path(st, path),
        Ok(None) => {}
        Err(error) => st.status = fill(tr("The file picker could not open: {}"), &[&error.to_string()]),
    });
}

/// Import an archive, a folder, or the folder a cursor file sits in. The pack is named after
/// the folder, as Grimoire names a loose-file pack.
pub fn import_path(st: &mut State, path: PathBuf) {
    run(st, move |store| {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        if ["zip", "7z", "rar"].contains(&ext.as_str()) {
            let packs = cursors::import_archive(store, &path).map_err(|e| e.to_string())?;
            return match packs.len() {
                0 => Err(tr("That archive has no cursor images.").to_string()),
                1 => Ok(fill(tr("Imported {}."), &[&packs[0].name])),
                n => Ok(fill(tr("Imported {} cursor packs."), &[&n.to_string()])),
            };
        }
        let folder = if path.is_dir() { path.clone() } else { path.parent().map(Path::to_path_buf).unwrap_or_default() };
        let files: Vec<PathBuf> = std::fs::read_dir(&folder)
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(cursors::is_cursor_file_name))
            .collect();
        let name = folder.file_name().and_then(|n| n.to_str()).unwrap_or("Cursors").to_string();
        let pack = cursors::import_files(store, &name, &files).map_err(|e| e.to_string())?;
        Ok(fill(tr("Imported {}."), &[&pack.name]))
    });
}

fn delete(st: &mut State, id: String) {
    st.locker.cursors.confirm_delete = None;
    run(st, move |store| {
        cursors::delete(store, &id).map(|_| tr("Cursor pack deleted.").to_string()).map_err(|e| e.to_string())
    });
}

// --- view ---------------------------------------------------------------------------------

fn preview(s: &State, u: &Ui, key: &str, files: &[String]) -> El {
    let names = preview_names(files);
    let mut main = el().size(64.0, 64.0).a11y_hidden();
    if let Some(px) = names.first().and_then(|n| s.thumbnails.get(&preview_key(key, n))) {
        main = main.pixels_fit(px, SurfaceFit::Fit);
    }
    let mut extras = col().gap(6.0);
    for name in names.iter().skip(1) {
        if let Some(px) = s.thumbnails.get(&preview_key(key, name)) {
            extras = extras.child(el().size(28.0, 28.0).pixels_fit(px, SurfaceFit::Fit).a11y_hidden());
        }
    }
    row()
        .w_full()
        .h(112.0)
        .gap(18.0)
        .center()
        .radius(crate::ui::r(8.0))
        .fill(u.well)
        .child(main)
        .child(extras)
}

/// What one card shows: its key, name, line, whether it is on, its files, and the pack id
/// (`None` for Deadlock's own set).
struct Card<'a> {
    key: String,
    name: String,
    line: String,
    active: bool,
    files: &'a [String],
    pack: Option<String>,
}

fn card(s: &State, u: &Ui, c: Card) -> El {
    let Card { key, name, line, active, files, pack } = c;
    let busy = s.locker.cursors.work.is_running();
    let id = pack.clone();
    let mut c = col()
        .key(format!("locker-cursor-{key}"))
        .min_w(0.0)
        .padding(10.0)
        .gap(8.0)
        .radius(crate::ui::r(10.0))
        .border(1.0, if active { u.accent } else { u.hair })
        .hover(u.hover)
        .focusable(true)
        .role("radio")
        .checked(active)
        .label(name.clone())
        .cursor(if busy { rune::Cursor::Default } else { rune::Cursor::Pointer })
        .on_press(move |st: &mut State| select(st, id.clone()))
        .child(preview(s, u, if pack.is_some() { &key } else { "stock" }, files));
    let mut foot = row()
        .w_full()
        .gap(8.0)
        .center_y()
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .child(text(name.clone()).font(BODY_MED).color(u.ink).truncate())
                .child(text(line).font(SMALL).color(u.faint).truncate()),
        );
    if active {
        foot = foot.child(icon(&icons::CIRCLE_CHECK, 16.0).color(u.accent).label(tr("On")));
    }
    if let Some(id) = pack {
        foot = foot.child(
            u.icon_button(&icons::TRASH_2, &fill(tr("Delete {}"), &[&name]))
                .key(format!("locker-cursor-delete-{id}"))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| st.locker.cursors.confirm_delete = Some(id.clone())),
        );
    }
    c = c.child(foot);
    c
}

/// The pane beside the General rail when Cursors is picked.
pub fn pane(s: &State, u: &Ui) -> El {
    let c = &s.locker.cursors;
    let state = c.state.clone().unwrap_or_default();
    let busy = c.work.is_running();
    let head = row()
        .w_full()
        .gap(8.0)
        .center_y()
        .child(text(tr("Cursors")).font(crate::ui::H1).color(u.ink))
        .child(text(fill(tr("{} packs"), &[&state.packs.len().to_string()])).font(SMALL).color(u.faint))
        .child(spacer())
        .child(
            u.icon_action(&icons::FOLDER_OPEN, tr("Import folder"), ActionKind::Secondary)
                .key("locker-cursor-import-folder")
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(pick_folder),
        )
        .child(
            u.icon_action(&icons::PLUS, tr("Import"), ActionKind::Secondary)
                .key("locker-cursor-import")
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(pick_file),
        );
    let mut grid = el().w_full().equal_columns(2).gap(12.0);
    grid = grid.child(card(
        s,
        u,
        Card {
            key: "stock".to_string(),
            name: tr("Deadlock").to_string(),
            line: tr("The game's own cursors").to_string(),
            active: state.active_id.is_none(),
            files: &c.stock,
            pack: None,
        },
    ));
    for pack in &state.packs {
        let name = match &pack.variant {
            Some(variant) => format!("{} ({variant})", pack.name),
            None => pack.name.clone(),
        };
        let line = if pack.files.len() == 1 {
            tr("1 cursor file").to_string()
        } else {
            fill(tr("{} cursor files"), &[&pack.files.len().to_string()])
        };
        grid = grid.child(card(
            s,
            u,
            Card {
                key: pack.id.clone(),
                name,
                line,
                active: state.active_id.as_deref() == Some(pack.id.as_str()),
                files: &pack.files,
                pack: Some(pack.id.clone()),
            },
        ));
    }
    let mut body = col()
        .w_full()
        .gap(12.0)
        .child(head)
        .child(
            text(tr("Pick a pack to use it. Sigil keeps a copy of Deadlock's own cursors and puts them back when you pick Deadlock. A game update can bring the stock files back; pick the pack again."))
                .font(BODY)
                .color(u.muted)
                .wrap(),
        );
    if busy {
        body = body.child(text(tr("Working on the cursor folder")).font(SMALL).color(u.muted));
    }
    if let Some(error) = &c.error {
        body = body.child(text(error.clone()).font(SMALL).color(u.rose).wrap());
    }
    if let Some(id) = c.confirm_delete.clone() {
        let name = state.packs.iter().find(|p| p.id == id).map(|p| p.name.clone()).unwrap_or_default();
        let active = state.active_id.as_deref() == Some(id.as_str());
        body = body.child(
            row()
                .key("locker-cursor-delete-confirm")
                .w_full()
                .padding(12.0)
                .gap(8.0)
                .center_y()
                .radius(crate::ui::r(8.0))
                .border(1.0, u.hair_strong)
                .child(
                    text(if active {
                        fill(tr("{} is on. Pick Deadlock first, then delete it."), &[&name])
                    } else {
                        fill(tr("Delete {}? Its files leave Sigil's store."), &[&name])
                    })
                    .font(SMALL)
                    .color(u.prose)
                    .wrap()
                    .grow(1.0)
                    .min_w(0.0),
                )
                .child(
                    u.action(tr("Cancel"), ActionKind::Ghost)
                        .key("locker-cursor-delete-cancel")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| st.locker.cursors.confirm_delete = None),
                )
                .child(if active {
                    el()
                } else {
                    u.action(tr("Delete"), ActionKind::Danger)
                        .key("locker-cursor-delete-yes")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| delete(st, id.clone()))
                }),
        );
    }
    body = body.child(grid);
    if state.packs.is_empty() {
        body = body.child(
            row().w_full().gap(8.0).center_y().child(text(tr("No cursor packs yet. Import one, or find some on GameBanana.")).font(BODY).color(u.muted)).child(spacer()).child(
                u.action(tr("Find cursors"), ActionKind::Ghost)
                    .key("locker-cursor-find")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| {
                        st.browse_query = rune::TextEdit::new("cursor");
                        st.go(crate::model::Screen::Browse);
                    }),
            ),
        );
    }
    body
}

/// Scenes: two packs, the second on, with drawn previews.
pub fn stage_scene(st: &mut State) {
    use deadlock_core::cursors::CursorPack;
    let pack = |id: &str, name: &str, files: &[&str]| CursorPack {
        id: id.to_string(),
        name: name.to_string(),
        variant: None,
        files: files.iter().map(|f| f.to_string()).collect(),
        installed_at: "2026-11-18T12:00:00.000Z".to_string(),
    };
    let mut state = CursorPacksState::default();
    state.packs.push(pack("ember", "Ember Arrow", &["cursor.bmp", "cursor_ping.bmp"]));
    state.packs.push(pack("frost", "Frost Pointer", &["cursor.bmp", "cursor_ping.bmp", "cursor_wait.bmp"]));
    state.active_id = Some("frost".to_string());
    // The image cache starts after a scene is staged, so the drawn arrows go in as PNG bytes.
    for (key, tint) in [("ember", [240u8, 120, 40]), ("frost", [90, 180, 255]), ("stock", [235, 235, 235])] {
        for (name, size) in [("cursor.bmp", 32u32), ("cursor_ping.bmp", 24), ("cursor_wait.bmp", 24)] {
            let mut rgba = vec![0u8; (size * size * 4) as usize];
            for y in 0..size {
                for x in 0..size {
                    // An arrow: the lower left triangle of the square.
                    if x <= y {
                        let o = ((y * size + x) * 4) as usize;
                        rgba[o..o + 4].copy_from_slice(&[tint[0], tint[1], tint[2], 255]);
                    }
                }
            }
            let mut png = Vec::new();
            use image::ImageEncoder;
            if image::codecs::png::PngEncoder::new(&mut png)
                .write_image(&rgba, size, size, image::ExtendedColorType::Rgba8)
                .is_ok()
            {
                st.pending_thumbs.push((preview_key(key, name), png));
            }
        }
    }
    st.locker.cursors.state = Some(state);
    st.locker.cursors.stock = vec!["cursor.bmp".to_string(), "cursor_ping.bmp".to_string()];
    st.locker.cursors.loaded = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bmp(width: i32, height: i32, bits: u16, pixels: &[u8]) -> Vec<u8> {
        let mut out = b"BM".to_vec();
        out.extend_from_slice(&((54 + pixels.len()) as u32).to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&54u32.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&width.to_le_bytes());
        out.extend_from_slice(&height.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&bits.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&[0; 20]);
        out.extend_from_slice(pixels);
        out
    }

    #[test]
    fn a_bottom_up_32_bit_bmp_keeps_its_alpha() {
        // Two rows of one pixel, bottom row first: blue at the bottom, red at the top.
        let file = bmp(1, 2, 32, &[255, 0, 0, 128, 0, 0, 255, 255]);
        let (w, h, rgba) = decode_bmp(&file).unwrap();
        assert_eq!((w, h), (1, 2));
        assert_eq!(&rgba[0..4], &[255, 0, 0, 255], "the top row is red");
        assert_eq!(&rgba[4..8], &[0, 0, 255, 128], "the bottom row is blue, half alpha");
    }

    #[test]
    fn zero_alpha_and_24_bit_read_as_opaque() {
        let file = bmp(1, 1, 32, &[10, 20, 30, 0]);
        assert_eq!(decode_bmp(&file).unwrap().2, vec![30, 20, 10, 255]);
        // 24 bit rows pad to four bytes.
        let file = bmp(1, 1, 24, &[10, 20, 30, 0]);
        assert_eq!(decode_bmp(&file).unwrap().2, vec![30, 20, 10, 255]);
    }

    #[test]
    fn broken_files_are_refused() {
        assert!(decode_bmp(b"PNG").is_none());
        assert!(decode_bmp(&bmp(4, 4, 32, &[0; 8])).is_none(), "too short for its size");
        assert!(decode_bmp(&bmp(1, 1, 8, &[0; 4])).is_none(), "palette files are not read");
    }

    #[test]
    fn the_main_cursor_previews_first_and_sizes_are_left_out() {
        let files: Vec<String> = ["cursor_wait.bmp", "cursor_vsz2.bmp", "cursor.bmp", "cursor.res"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(preview_names(&files), vec!["cursor.bmp".to_string(), "cursor_wait.bmp".to_string()]);
    }
}
