//! In-game card: the player's own image on a hero's cards in the game (hero select, scoreboard,
//! minimap), built into a mod.
//!
//! - Grimoire: Locker hero page "Hero Card": one tile per base-game variant at its true shape
//!   with the default art dimmed and "W x H"; a cropper per tile; Apply builds and installs at
//!   once, Export saves a standalone VPK.
//! - Deadlock Mod Manager: Foundry Cards, every slot at its shape with "W × H" and a "Replaced"
//!   mark; the image is centre-cropped to the slot and re-encoded; it leaves through the export
//!   sheet.
//! - Sigil: an In-game card action on the hero page opens one dialog with every card variant the
//!   game ships for that hero, each at its shape with the game's art underneath and its size.
//!   Choose image takes PNG, JPEG or WebP; the image is centre-cropped to the variant's shape (a
//!   note says when it is smaller and will look soft); variants left empty keep the game's art.
//!   Build mod packs the chosen ones into a VPK in app data and opens the import review, so the
//!   mod lands switched off.

use std::path::{Path, PathBuf};

use rune::{El, FileDialogOptions, SurfaceFit, Tx, col, el, row, spacer, text};

use super::assets::{self, CardSlot, Load, thumb_key};
use crate::app::State;
use crate::icons;
use crate::jobs::Job;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{ActionKind, BODY, BODY_MED, H1, SMALL, Ui};

/// The tallest a variant tile is drawn.
const TILE_H: f32 = 128.0;

pub struct Pick {
    pub slot: String,
    pub file: PathBuf,
    pub size: Option<(u32, u32)>,
}

pub struct CardArt {
    pub display_name: String,
    pub slots: Load<Vec<CardSlot>>,
    pub slot_job: Job<Vec<CardSlot>>,
    pub picks: Vec<Pick>,
    pub name: rune::TextEdit,
    pub build_job: Job<PathBuf>,
    pub error: Option<String>,
    /// One texture from Game art (an ability icon), by its label, instead of the hero's cards.
    pub single: Option<String>,
}

/// Open the dialog for the hero page on screen.
pub fn open_current(st: &mut State) {
    let super::Page::Hero(name) = st.locker.page.clone() else { return };
    let Some(hero) = st.locker.heroes.iter().find(|h| h.display_name == name).cloned() else { return };
    open(st, &hero.codename, &hero.display_name);
}

pub fn open(st: &mut State, codename: &str, display_name: &str) {
    let mut card = CardArt {
        display_name: display_name.to_string(),
        slots: Load::Loading,
        slot_job: Job::default(),
        picks: Vec::new(),
        name: rune::TextEdit::new(""),
        build_job: Job::default(),
        error: None,
        single: None,
    };
    if !st.demo && !rune::is_headless() {
        match st.game_root.clone() {
            Some(game_root) => {
                let code = codename.to_string();
                card.slot_job.start(st.wake.clone(), move || assets::api::card_slots(&game_root, &code));
            }
            None => card.slots = Load::Failed(tr("Set the game folder in Settings first.").to_string()),
        }
    }
    st.locker.assets.card = Some(card);
}

/// One game texture (an ability icon from Game art) in the same dialog: one tile, one PNG.
pub fn open_single(st: &mut State, path: &str, label: &str, size: Option<(u32, u32)>) {
    let super::Page::Hero(name) = st.locker.page.clone() else { return };
    let Some(hero) = st.locker.heroes.iter().find(|h| h.display_name == name).cloned() else { return };
    st.locker.assets.card = Some(CardArt {
        display_name: hero.display_name.clone(),
        slots: Load::Ready(vec![CardSlot { variant: label.to_string(), path: path.to_string(), size: size.unwrap_or((256, 256)) }]),
        slot_job: Job::default(),
        picks: Vec::new(),
        name: rune::TextEdit::new(""),
        build_job: Job::default(),
        error: None,
        single: Some(label.to_string()),
    });
}

fn default_name(card: &CardArt) -> String {
    match &card.single {
        Some(label) => format!("{} {label}", card.display_name),
        None => fill(tr("{} card art"), &[&card.display_name]),
    }
}

fn close(st: &mut State) {
    if st.locker.assets.card.as_ref().is_some_and(|c| c.build_job.is_running()) {
        return;
    }
    st.locker.assets.card = None;
}

/// The base art paths the dialog draws, for the thumbnail cap.
pub fn shown_paths(st: &State) -> Vec<String> {
    st.locker
        .assets
        .card
        .as_ref()
        .and_then(|c| c.slots.ready())
        .map(|slots| slots.iter().map(|s| s.path.clone()).collect())
        .unwrap_or_default()
}

pub fn poll(st: &mut State) -> bool {
    let Some(card) = st.locker.assets.card.as_mut() else { return false };
    let mut changed = false;
    if let Some(result) = card.slot_job.try_take() {
        card.slots = match result {
            Ok(slots) => Load::Ready(slots),
            Err(error) => Load::Failed(error),
        };
        changed = true;
        let paths = shown_paths(st);
        assets::want_thumbs(st, &paths);
    }
    let Some(card) = st.locker.assets.card.as_mut() else { return changed };
    if let Some(result) = card.build_job.try_take() {
        match result {
            Ok(path) => {
                st.locker.assets.card = None;
                crate::screens::import::open_local(st, vec![path]);
            }
            Err(error) => card.error = Some(error),
        }
        changed = true;
    }
    changed
}

/// Header size of a chosen image.
fn image_size(path: &Path) -> Option<(u32, u32)> {
    image::image_dimensions(path).ok()
}

fn choose(slot: CardSlot) -> impl Fn(&mut State, &mut Tx) + 'static {
    move |_st: &mut State, tx: &mut Tx| {
        let options = FileDialogOptions {
            title: fill(tr("Choose an image for {}"), &[&slot.variant]),
            ..FileDialogOptions::default()
        }
        .filter("Images", &["png", "jpg", "jpeg", "webp"]);
        let slot_path = slot.path.clone();
        tx.open_file(options, move |st: &mut State, result, _tx| {
            let Ok(Some(file)) = result else { return };
            let size = image_size(&file);
            let Some(card) = st.locker.assets.card.as_mut() else { return };
            if size.is_none() {
                card.error = Some(tr("That file is not a PNG, JPEG or WebP image.").to_string());
                return;
            }
            card.error = None;
            card.picks.retain(|p| p.slot != slot_path);
            card.picks.push(Pick { slot: slot_path.clone(), file: file.clone(), size });
            crate::data::request_locker_art(st, &file.display().to_string());
        });
    }
}

/// The largest centred box of the target's shape inside a `w` by `h` image.
pub fn center_crop(w: u32, h: u32, target: (u32, u32)) -> (u32, u32, u32, u32) {
    let (tw, th) = (target.0.max(1) as u64, target.1.max(1) as u64);
    let (w64, h64) = (w as u64, h as u64);
    if w64 * th > h64 * tw {
        let cw = (h64 * tw / th) as u32;
        ((w - cw) / 2, 0, cw.max(1), h)
    } else {
        let ch = (w64 * th / tw) as u32;
        (0, (h - ch) / 2, w, ch.max(1))
    }
}

/// Read an image, crop it to the slot's shape from the centre, and encode it as PNG at the
/// slot's size (the core takes PNG and splices it into the game's texture).
pub fn crop_png(file: &Path, target: (u32, u32)) -> Result<Vec<u8>, String> {
    let image = image::open(file).map_err(|_| tr("That file is not a PNG, JPEG or WebP image.").to_string())?;
    let (x, y, w, h) = center_crop(image.width(), image.height(), target);
    let cropped = image.crop_imm(x, y, w, h).resize_exact(target.0.max(1), target.1.max(1), image::imageops::FilterType::Lanczos3);
    let mut out = std::io::Cursor::new(Vec::new());
    cropped
        .write_to(&mut out, image::ImageFormat::Png)
        .map_err(|error| error.to_string())?;
    Ok(out.into_inner())
}

fn build(st: &mut State) {
    let Some(card) = st.locker.assets.card.as_mut() else { return };
    if card.build_job.is_running() {
        return;
    }
    if card.picks.is_empty() {
        card.error = Some(tr("Choose an image for at least one card.").to_string());
        return;
    }
    let Some(slots) = card.slots.ready().cloned() else { return };
    let jobs: Vec<(String, PathBuf, (u32, u32))> = card
        .picks
        .iter()
        .filter_map(|p| slots.iter().find(|s| s.path == p.slot).map(|s| (s.path.clone(), p.file.clone(), s.size)))
        .collect();
    let typed = card.name.text.trim().to_string();
    let name = if typed.is_empty() { default_name(card) } else { typed };
    card.error = None;
    if st.demo || rune::is_headless() {
        return;
    }
    let Some(game_root) = st.game_root.clone() else { return };
    let wake = st.wake.clone();
    let Some(card) = st.locker.assets.card.as_mut() else { return };
    card.build_job.start(wake, move || {
        let mut picks = Vec::new();
        for (path, file, size) in jobs {
            picks.push((path, crop_png(&file, size)?));
        }
        assets::api::build_card(&game_root, &name, picks)
    });
}

fn tile(s: &State, u: &Ui, card: &CardArt, slot: &CardSlot) -> El {
    let (w, h) = slot.size;
    let ratio = if w > 0 && h > 0 { w as f32 / h as f32 } else { 0.75 };
    let tile_w = (TILE_H * ratio).clamp(56.0, 300.0);
    let tile_h = (tile_w / ratio).min(TILE_H);
    let pick = card.picks.iter().find(|p| p.slot == slot.path);
    let picked = pick.and_then(|p| s.thumbnails.get(&p.file.display().to_string())).filter(|px| px.size().0 > 0);
    let base = s.thumbnails.get(&thumb_key(&slot.path)).filter(|px| px.size().0 > 0);
    let mut frame = el()
        .size(tile_w, tile_h)
        .radius(crate::ui::r(8.0))
        .clip_rounded(crate::ui::r(8.0))
        .fill(u.well)
        .border(1.0, if pick.is_some() { u.accent } else { u.hair });
    frame = match (picked, base) {
        (Some(px), _) => frame.child(el().size(tile_w, tile_h).pixels_fit(px, SurfaceFit::Fill).a11y_hidden()),
        (None, Some(px)) => frame.child(el().size(tile_w, tile_h).pixels_fit(px, SurfaceFit::Fill).opacity(0.45).a11y_hidden()),
        (None, None) => frame.center().child(rune::icon(&icons::BOX, 16.0).color(u.faint.alpha(0.6)).a11y_hidden()),
    };
    let mut info = col()
        .w(tile_w.max(120.0))
        .gap(2.0)
        .child(text(slot.variant.clone()).font(BODY_MED).color(u.ink).truncate())
        .child(text(format!("{w} × {h}")).font(SMALL).color(u.faint));
    if let Some((pw, ph)) = pick.and_then(|p| p.size)
        && (pw < w || ph < h)
    {
        info = info.child(
            text(fill(tr("Your image is {} × {}, so it will look soft."), &[&pw.to_string(), &ph.to_string()]))
                .font(SMALL)
                .color(u.amber)
                .wrap(),
        );
    }
    let mut actions = row().gap(4.0).child(
        u.action(if pick.is_some() { tr("Change") } else { tr("Choose image") }, ActionKind::Ghost)
            .key(format!("locker-ingame-choose-{}", slot.variant))
            .label(fill(tr("Choose an image for {}"), &[&slot.variant]))
            .cursor(rune::Cursor::Pointer)
            .on_press_tx(choose(slot.clone())),
    );
    if pick.is_some() {
        let path = slot.path.clone();
        actions = actions.child(
            u.action(tr("Clear"), ActionKind::Ghost)
                .key(format!("locker-ingame-clear-{}", slot.variant))
                .label(fill(tr("Clear the image for {}"), &[&slot.variant]))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    if let Some(card) = st.locker.assets.card.as_mut() {
                        card.picks.retain(|p| p.slot != path);
                    }
                }),
        );
    }
    col()
        .key(format!("locker-ingame-slot-{}", slot.variant))
        .gap(6.0)
        .child(frame)
        .child(info)
        .child(actions)
}

pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    let card = s.locker.assets.card.as_ref()?;
    let busy = card.build_job.is_running();
    let mut body = col()
        .w_full()
        .gap(16.0)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(
                    text(match &card.single {
                        Some(label) => fill(tr("Replace {} in game"), &[label]),
                        None => fill(tr("In-game card for {}"), &[&card.display_name]),
                    })
                        .font(H1)
                        .color(u.ink)
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate(),
                )
                .child(
                    u.icon_button(&icons::X, tr("Close"))
                        .key("locker-ingame-close")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(close),
                ),
        )
        .child(
            text(tr("Takes a PNG. A JPEG or WebP is turned into a PNG first."))
                .font(SMALL)
                .color(u.faint),
        )
        .child(
            text(if card.single.is_some() {
                tr("Choose an image. It is cropped to the texture's shape from the centre and replaces the game's art.")
            } else {
                tr("Choose an image for each card the game shows. It is cropped to the card's shape from the centre. Cards you leave empty keep the game's art.")
            })
                .font(BODY)
                .color(u.muted)
                .wrap(),
        );
    match &card.slots {
        Load::Idle | Load::Loading => {
            body = body.child(text(tr("Reading the hero's cards from the game files...")).font(BODY).color(u.muted));
        }
        Load::Failed(error) => {
            body = body
                .child(text(tr("The hero's cards could not be read")).font(BODY_MED).color(u.ink))
                .child(text(error.clone()).font(SMALL).color(u.muted).wrap());
        }
        Load::Ready(slots) if slots.is_empty() => {
            body = body.child(
                text(fill(tr("The game files have no card art for {}."), &[&card.display_name]))
                    .font(BODY)
                    .color(u.muted)
                    .wrap(),
            );
        }
        Load::Ready(slots) => {
            let mut grid = row().w_full().flex_wrap().gap(16.0).items_start();
            for slot in slots {
                grid = grid.child(tile(s, u, card, slot));
            }
            body = body.child(
                col()
                    .w_full()
                    .max_h(470.0)
                    .scroll(rune::ScrollSpec { stick_bottom: false })
                    .child(grid),
            );
            let placeholder = default_name(card);
            let name = rune::text_input(&card.name, BODY)
                .key("locker-ingame-name")
                .label(tr("Mod name"))
                .single_line()
                .color(u.ink)
                .placeholder(placeholder, u.faint)
                .caret(u.accent, u.accent.alpha(0.20))
                .on_edit(|st: &mut State| match st.locker.assets.card.as_mut() {
                    Some(card) => &mut card.name,
                    None => &mut st.locker.notes_placeholder,
                })
                .on_submit(|st: &mut State, _| build(st));
            body = body.child(
                col()
                    .w_full()
                    .gap(6.0)
                    .child(u.field_label(tr("Mod name")))
                    .child(
                        row()
                            .h(32.0)
                            .w_full()
                            .px(10.0)
                            .center_y()
                            .radius(crate::ui::r(8.0))
                            .fill(u.well)
                            .border(1.0, u.hair)
                            .child(name.grow(1.0).min_w(0.0)),
                    ),
            );
        }
    }
    if let Some(error) = &card.error {
        body = body.child(text(error.clone()).font(SMALL).color(u.rose).wrap());
    }
    let chosen = card.picks.len();
    body = body.child(
        row()
            .w_full()
            .gap(8.0)
            .center_y()
            .child(
                text(if chosen == 0 {
                    tr("Sigil packs the cards into a new mod and opens the import review. It lands switched off.").to_string()
                } else {
                    fill(tr("{} of the cards will change. The mod lands switched off."), &[&chosen.to_string()])
                })
                .font(SMALL)
                .color(u.faint)
                .wrap()
                .grow(1.0)
                .min_w(0.0),
            )
            .child(spacer().w(8.0))
            .child(
                u.action(tr("Cancel"), ActionKind::Ghost)
                    .key("locker-ingame-cancel")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(close),
            )
            .child(
                u.icon_action(
                    if busy { &icons::LOADER } else { &icons::PACKAGE },
                    if busy { tr("Building...") } else { tr("Build mod") },
                    if chosen == 0 { ActionKind::Secondary } else { ActionKind::Primary },
                )
                .key("locker-ingame-build")
                .cursor(rune::Cursor::Pointer)
                .on_press(build),
            ),
    );
    Some(
        u.dialog_shell()
            .key("locker-ingame-card")
            .role("dialog")
            .label(fill(tr("In-game card for {}"), &[&card.display_name]))
            .w(720.0)
            .max_w(720.0)
            .padding(24.0)
            .modal(true, close)
            .on_key(|st: &mut State, ev: &rune::KeyEvent, _| {
                if ev.key == rune::Key::Escape {
                    close(st);
                    true
                } else {
                    false
                }
            })
            .child(body),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_crop_keeps_the_middle_at_the_card_shape() {
        // A wide image into a tall card: the sides go.
        assert_eq!(center_crop(1000, 500, (300, 400)), (312, 0, 375, 500));
        // A tall image into a wide card: top and bottom go.
        assert_eq!(center_crop(400, 1000, (400, 200)), (0, 400, 400, 200));
        // Same shape: nothing goes.
        assert_eq!(center_crop(600, 800, (300, 400)), (0, 0, 600, 800));
    }

    #[test]
    fn a_chosen_image_becomes_a_png_at_the_card_size() {
        let dir = std::env::temp_dir().join(format!("sigil-card-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("art.png");
        image::RgbaImage::from_pixel(64, 32, image::Rgba([200, 80, 20, 255])).save(&src).unwrap();
        let png = crop_png(&src, (24, 32)).unwrap();
        let back = image::load_from_memory(&png).unwrap();
        assert_eq!((back.width(), back.height()), (24, 32));
        let text = dir.join("not.png");
        std::fs::write(&text, b"hello").unwrap();
        assert!(crop_png(&text, (24, 32)).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
