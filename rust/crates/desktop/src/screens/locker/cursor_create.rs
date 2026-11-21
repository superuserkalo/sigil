//! Create a cursor pack from your own images (Grimoire's unreleased `feat/cursor-create`, MIT):
//! a tile per stock cursor, an image picked for each, the click point guessed from the art and
//! nudged with arrows, then Create stores it as an ordinary pack (`deadlock_core::cursors_create`)
//! that the Locker applies like any other. App data only until the player applies it.
//!
//! Each picked image is scaled to fit the stock cursor's size (32 when that is unknown); there
//! is no free cropper yet.

use std::path::PathBuf;

use deadlock_core::cursors_create::{self, BuiltCursor, DEFAULT_EDGE, Hotspot, STOCK_CURSORS};
use rune::{El, FileDialogOptions, SurfaceFit, TextEdit, Tx, col, el, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{ActionKind, BODY, BODY_MED, SMALL, Ui};

/// The up chevron the hotspot arrows need; `icons.rs` has the other three.
static CHEVRON_UP: rune::Icon = rune::Icon {
    name: "sigil-chevron-up",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="m18 15-6-6-6 6"/></g>"#,
};

#[derive(Default)]
pub struct Create {
    pub open: bool,
    pub name: TextEdit,
    /// One per [`STOCK_CURSORS`] entry, in that order.
    pub tiles: Vec<Option<BuiltCursor>>,
    pub error: Option<String>,
}

fn thumb_key(index: usize) -> String {
    format!("cursor-create:{index}")
}

pub fn open(st: &mut State) {
    let create = &mut st.locker.cursors.create;
    create.open = true;
    create.name = TextEdit::new("");
    create.tiles = vec![None; STOCK_CURSORS.len()];
    create.error = None;
}

/// The stock cursor's size, read from the game folder, else [`DEFAULT_EDGE`].
fn stock_edge(st: &State, file_name: &str) -> u32 {
    st.game_root
        .as_ref()
        .and_then(|root| std::fs::read(deadlock_core::cursors::cursor_folder(root).join(file_name)).ok())
        .and_then(|bytes| super::cursors::decode_bmp(&bytes))
        .map(|(w, h, _)| w.max(h))
        .filter(|edge| (8..=256).contains(edge))
        .unwrap_or(DEFAULT_EDGE)
}

fn pick(index: usize) -> impl Fn(&mut State, &mut Tx) {
    move |_st: &mut State, tx: &mut Tx| {
        let options = FileDialogOptions { title: tr("Pick an image for this cursor").to_string(), ..FileDialogOptions::default() }
            .filter(tr("Images"), &["png", "jpg", "jpeg", "webp"]);
        tx.open_file(options, move |st: &mut State, result, _tx| match result {
            Ok(Some(path)) => load(st, index, path),
            Ok(None) => {}
            Err(error) => st.status = fill(tr("The file picker could not open: {}"), &[&error.to_string()]),
        });
    }
}

/// Decode, scale to fit the stock size, guess the hotspot and show it.
fn load(st: &mut State, index: usize, path: PathBuf) {
    let Some((file_name, _)) = STOCK_CURSORS.get(index) else { return };
    let edge = stock_edge(st, file_name);
    let image = match image::open(&path) {
        Ok(image) => image.thumbnail(edge, edge).to_rgba8(),
        Err(_) => {
            st.locker.cursors.create.error = Some(tr("That file is not a PNG, JPEG or WebP image.").to_string());
            return;
        }
    };
    let (width, height) = image.dimensions();
    let rgba = image.into_raw();
    let hotspot = cursors_create::detect_hotspot(width, height, &rgba);
    set_preview(st, index, width, height, &rgba);
    let create = &mut st.locker.cursors.create;
    create.error = None;
    if let Some(tile) = create.tiles.get_mut(index) {
        *tile = Some(BuiltCursor { file_name: file_name.to_string(), width, height, rgba, hotspot });
    }
}

fn set_preview(st: &mut State, index: usize, width: u32, height: u32, rgba: &[u8]) {
    if let Some(cache) = st.images.as_ref() {
        let image = cache.rgba(width, height, rgba.to_vec());
        st.thumbnails.insert(thumb_key(index), image.pixels().clone());
    }
}

fn nudge(st: &mut State, index: usize, dx: i32, dy: i32) {
    if let Some(Some(tile)) = st.locker.cursors.create.tiles.get_mut(index) {
        tile.hotspot.x = (tile.hotspot.x + dx).clamp(0, tile.width as i32 - 1);
        tile.hotspot.y = (tile.hotspot.y + dy).clamp(0, tile.height as i32 - 1);
    }
}

fn create(st: &mut State) {
    let name = st.locker.cursors.create.name.text.trim().to_string();
    let images: Vec<BuiltCursor> = st.locker.cursors.create.tiles.iter().flatten().cloned().collect();
    if name.is_empty() || images.is_empty() {
        return;
    }
    if st.demo || rune::is_headless() {
        st.status = tr("Demo library: nothing was changed.").to_string();
        return;
    }
    // The stock hotspots: the game's file while no pack is on, else Sigil's stock copy.
    let stock_res = st
        .game_root
        .as_ref()
        .and_then(|root| deadlock_core::cursors::stock_preview_folder(&super::cursors::store(), root).ok())
        .and_then(|folder| std::fs::read_to_string(folder.join("cursor.res")).ok());
    match cursors_create::create_pack(&super::cursors::store(), stock_res.as_deref(), &name, &images) {
        Ok(pack) => {
            crate::notify::push(st, crate::notify::Notice::success(fill(tr("Created {}. Press its card to use it."), &[&pack.name])));
            st.locker.cursors.create.open = false;
            super::cursors::start_read(st);
        }
        Err(error) => st.locker.cursors.create.error = Some(error.to_string()),
    }
}

fn tile(s: &State, u: &Ui, index: usize, label: &str, built: Option<&BuiltCursor>) -> El {
    let mut art = el().size(64.0, 64.0).a11y_hidden();
    if built.is_some()
        && let Some(px) = s.thumbnails.get(&thumb_key(index))
    {
        art = art.pixels_fit(px, SurfaceFit::Fit);
    }
    let mut well = el().size(96.0, 96.0).center().radius(crate::ui::r(8.0)).fill(u.well).child(art);
    if let Some(cursor) = built {
        // The click point, drawn over the art at its scale (64 px for the longer edge).
        let scale = 64.0 / cursor.width.max(cursor.height).max(1) as f32;
        let x = 16.0 + (64.0 - cursor.width as f32 * scale) / 2.0 + cursor.hotspot.x as f32 * scale;
        let y = 16.0 + (64.0 - cursor.height as f32 * scale) / 2.0 + cursor.hotspot.y as f32 * scale;
        well = well.child(
            el().absolute(x - 3.0, y - 3.0).size(6.0, 6.0).radius(3.0).fill(u.accent).a11y_hidden(),
        );
    }
    let mut info = col().gap(4.0).grow(1.0).min_w(0.0).child(text(label.to_string()).font(BODY_MED).color(u.ink));
    match built {
        Some(cursor) => {
            info = info.child(
                text(fill(tr("{} by {}, clicks at {}, {}"), &[
                    &cursor.width.to_string(),
                    &cursor.height.to_string(),
                    &cursor.hotspot.x.to_string(),
                    &cursor.hotspot.y.to_string(),
                ]))
                .font(SMALL)
                .color(u.muted),
            );
            let mut arrows = row().gap(4.0);
            for (glyph, name, dx, dy) in [
                (&icons::CHEVRON_LEFT, "left", -1, 0),
                (&CHEVRON_UP, "up", 0, -1),
                (&icons::CHEVRON_DOWN, "down", 0, 1),
                (&icons::CHEVRON_RIGHT, "right", 1, 0),
            ] {
                arrows = arrows.child(
                    u.icon_button(glyph, &fill(tr("Move the click point {}"), &[tr(name)]))
                        .key(format!("cursor-create-{index}-{name}"))
                        .on_press(move |st: &mut State| nudge(st, index, dx, dy)),
                );
            }
            info = info.child(arrows);
        }
        None => info = info.child(text(tr("Deadlock's own stays")).font(SMALL).color(u.faint)),
    }
    row()
        .key(format!("cursor-create-tile-{index}"))
        .w_full()
        .gap(12.0)
        .padding(10.0)
        .center_y()
        .radius(crate::ui::r(10.0))
        .border(1.0, u.hair)
        .child(well)
        .child(info)
        .child(
            u.action(if built.is_some() { tr("Change") } else { tr("Pick image") }, ActionKind::Secondary)
                .key(format!("cursor-create-pick-{index}"))
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(pick(index)),
        )
}

/// The create panel, drawn in place of the pack grid while it is open.
pub fn panel(s: &State, u: &Ui) -> Option<El> {
    let c = &s.locker.cursors.create;
    if !c.open {
        return None;
    }
    let mut tiles = col().w_full().gap(10.0);
    for (index, (_, label)) in STOCK_CURSORS.iter().enumerate() {
        tiles = tiles.child(tile(s, u, index, tr(label), c.tiles.get(index).and_then(Option::as_ref)));
    }
    let picked = c.tiles.iter().flatten().count();
    let ready = picked > 0 && !c.name.text.trim().is_empty();
    let name = rune::text_input(&c.name, BODY)
        .key("cursor-create-name")
        .single_line()
        .label(tr("Name for the pack"))
        .w_full()
        .color(u.ink)
        .placeholder(tr("Like My cursors"), u.faint)
        .caret(u.accent, u.accent.alpha(0.20))
        .on_edit(rune::edit_access(|st: &mut State| &mut st.locker.cursors.create.name));
    let mut out = col()
        .w_full()
        .gap(14.0)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(text(tr("Create a cursor pack")).font(crate::ui::H1).color(u.ink))
                .child(spacer())
                .child(
                    u.action(tr("Cancel"), ActionKind::Ghost)
                        .key("cursor-create-cancel")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| st.locker.cursors.create.open = false),
                )
                .child(
                    u.action(tr("Create"), ActionKind::Primary)
                        .key("cursor-create-go")
                        .disabled(!ready)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(create),
                ),
        )
        .child(
            text(tr("Pick an image for any cursor you want to change. Sigil guesses where it clicks from the art; move the dot if it guessed wrong. Cursors you leave alone stay Deadlock's own."))
                .font(BODY)
                .color(u.muted)
                .wrap(),
        )
        .child(
            row()
                .w(320.0)
                .h(36.0)
                .px(10.0)
                .center_y()
                .radius(crate::ui::r(8.0))
                .fill(u.well)
                .border(1.0, u.hair)
                .child(name),
        )
        .child(tiles);
    if let Some(error) = &c.error {
        out = out.child(crate::notify::feedback::error_line(u, "cursor-create-error", error));
    }
    Some(out)
}

/// Headless scene: the Locker's Cursors tab creating a pack, two cursors picked, one with a
/// nudged click point.
pub fn scene(st: &mut State) {
    crate::screens::wave3_scenes::locker_cursors(st);
    open(st);
    st.locker.cursors.create.name = TextEdit::new("Ember");
    for (index, edge) in [(0usize, 32u32), (1, 24)] {
        let mut rgba = vec![0u8; (edge * edge * 4) as usize];
        for y in 0..edge {
            for x in 0..=y / 2 {
                let o = ((y * edge + x) * 4) as usize;
                rgba[o..o + 4].copy_from_slice(&[240, 120, 40, 255]);
            }
        }
        let hotspot = if index == 1 { Hotspot { x: 3, y: 2 } } else { cursors_create::detect_hotspot(edge, edge, &rgba) };
        let mut png = Vec::new();
        use image::ImageEncoder;
        if image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&rgba, edge, edge, image::ExtendedColorType::Rgba8)
            .is_ok()
        {
            st.pending_thumbs.push((thumb_key(index), png));
        }
        st.locker.cursors.create.tiles[index] =
            Some(BuiltCursor { file_name: STOCK_CURSORS[index].0.to_string(), width: edge, height: edge, rgba, hotspot });
    }
}
