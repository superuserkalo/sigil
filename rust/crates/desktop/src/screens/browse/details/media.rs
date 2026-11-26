//! Gallery, carousel and lightbox for the mod details.

use super::*;

pub(crate) fn group_digits(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A small tag over an image, for example `1 / 3`.
pub(crate) fn image_badge(u: &Ui, label: String) -> El {
    row()
        .h(22.0)
        .px(8.0)
        .center()
        .radius(crate::ui::r(6.0))
        .fill(Color::BLACK.alpha(0.55))
        .border(1.0, Color::WHITE.alpha(0.10))
        .child(text(label).font(SMALL).color(u.prose))
}

/// The gallery: the main image in its fixed 16:9 box, then one row of thumbs, paged so the
/// current image's page shows. The box is there from the first frame.
pub(crate) fn carousel(s: &State, u: &Ui, d: &Details) -> El {
    let images = d.images();
    let count = images.len();
    let main = carousel_frame(s, u, d);
    if count < 2 || veiled(s, d.id, d.nsfw) {
        return main;
    }
    // The strip spans the image's width in six equal 16:9 slots, so it shares both edges with
    // the image. Past six, the last slot of a page shows how many follow and pages on.
    const PER: usize = 6;
    let index = d.image.min(count - 1);
    let start = (index / PER) * PER;
    let more = count.saturating_sub(start + PER);
    let mut thumbs = el().key("carousel-thumbs").w_full().equal_columns(PER).gap(8.0);
    for (slot, (i, image)) in images.iter().enumerate().skip(start).take(PER).enumerate() {
        let on = i == index;
        let pager = more > 0 && slot + 1 == PER;
        let target = if pager { start + PER } else { i };
        let mut t = el()
            .key(format!("carousel-thumb-{i}"))
            .w_full()
            .min_w(0.0)
            .aspect_ratio(16.0 / 9.0)
            .radius(crate::ui::r(4.0))
            .clip_rounded(crate::ui::r(4.0))
            .fill(u.hover)
            .focusable(true)
            .role("button")
            .label(if pager { format!("{more} more images") } else { format!("Image {}", i + 1) })
            .checked(on)
            .cursor(rune::Cursor::Pointer)
            .hover(Color::WHITE.alpha(0.06))
            .on_press(move |st: &mut State| {
                if let Some(details) = st.browse_view.details.as_mut() {
                    details.image = target;
                }
            })
            .child(image_fill(s, format!("carousel-thumb-img-{i}"), Some(&image.url), [crate::ui::r(4.0); 4], false));
        if pager {
            t = t.child(
                el().absolute(0.0, 0.0)
                    .w_full()
                    .h_full()
                    .center()
                    .fill(Color::BLACK.alpha(0.6))
                    .child(text(format!("+{more}")).font(LABEL).color(Color::WHITE).a11y_hidden()),
            );
        } else if !on {
            // The open image is the one at full strength.
            t = t.child(el().absolute(0.0, 0.0).w_full().h_full().fill(Color::BLACK.alpha(0.35)).a11y_hidden());
        }
        t = t.child(
            el().absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .radius(crate::ui::r(4.0))
                .border(if on { 2.0 } else { 1.0 }, if on { u.ink } else { u.hair })
                .a11y_hidden(),
        );
        thumbs = thumbs.child(t);
    }
    // A short last page keeps the slots' size: empty cells stay empty.
    col().key("carousel-gallery").w_full().gap(8.0).child(main).child(thumbs)
}

fn carousel_frame(s: &State, u: &Ui, d: &Details) -> El {
    let images = d.images();
    let veil = veiled(s, d.id, d.nsfw);
    let count = images.len();
    let index = d.image.min(count.saturating_sub(1));
    let url = images
        .get(index)
        .map(|image| &image.url)
        .or(d.thumbnail_url.as_ref());
    let mut frame = el()
        .key("carousel")
        .w_full()
        .aspect_ratio(16.0 / 9.0)
        .radius(crate::ui::r(8.0))
        .fill(u.well)
        .border(1.0, u.hair)
        .clip_rounded(crate::ui::r(8.0))
        .child(image_fill(
            s,
            format!("carousel-{index}"),
            url,
            [8.0; 4],
            veil,
        ));
    if veil {
        return frame.child(nsfw_veil(u, "carousel", d.id, false));
    }
    if count > 0 {
        frame = frame
            .focusable(true)
            .role("button")
            .label("View full size")
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| open_lightbox(st, index));
    }
    if count > 1 {
        let step = |next: bool| {
            el().key(if next { "carousel-next" } else { "carousel-prev" })
                .size(28.0, 28.0)
                .center()
                .radius(crate::ui::r(8.0))
                .fill(Color::BLACK.alpha(0.55))
                .border(1.0, Color::WHITE.alpha(0.15))
                .hover(Color::BLACK.alpha(0.8))
                .focusable(true)
                .role("button")
                .label(if next { "Next image" } else { "Previous image" })
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| step_carousel(st, next))
                // On a darkened image in every theme, so the ink is always white.
                .child(
                    icon(
                        if next {
                            &icons::CHEVRON_RIGHT
                        } else {
                            &icons::CHEVRON_LEFT
                        },
                        16.0,
                    )
                    .color(Color::WHITE)
                    .a11y_hidden(),
                )
        };
        frame = frame
            .child(
                row()
                    .pin(Some(8.0), Some(0.0), Some(8.0), Some(0.0))
                    .center_y()
                    .child(step(false))
                    .child(spacer())
                    .child(step(true)),
            )
            .child(
                image_badge(u, format!("{} / {count}", index + 1)).pin(
                    None,
                    None,
                    Some(8.0),
                    Some(8.0),
                ),
            );
    }
    frame
}

pub(crate) fn step_carousel(st: &mut State, next: bool) {
    let Some(details) = st.browse_view.details.as_mut() else {
        return;
    };
    let count = details.images().len();
    if count == 0 {
        return;
    }
    details.image = if next {
        (details.image + 1) % count
    } else {
        (details.image + count - 1) % count
    };
}

pub(crate) fn open_lightbox(st: &mut State, index: usize) {
    let Some(details) = st.browse_view.details.as_mut() else {
        return;
    };
    if details.images().is_empty() {
        return;
    }
    details.lightbox = Some(index);
    let full = details.images().get(index).map(|image| image.full_url.clone());
    // One original at a time: the others go (their gallery copies stay).
    let others: Vec<String> = details
        .images()
        .iter()
        .filter(|image| image.full_url != image.url && Some(&image.full_url) != full.as_ref())
        .map(|image| image.full_url.clone())
        .collect();
    for url in others {
        st.thumbnails.remove(&url);
    }
    super::super::w4::reset_zoom(st);
    crate::data::queue_images(st, full.into_iter().collect(), true);
}

pub(crate) fn step_lightbox(st: &mut State, next: bool) {
    let Some(details) = st.browse_view.details.as_mut() else {
        return;
    };
    let count = details.images().len();
    let Some(index) = details.lightbox.filter(|_| count > 0) else {
        return;
    };
    let index = if next {
        (index + 1) % count
    } else {
        (index + count - 1) % count
    };
    open_lightbox(st, index);
}

pub(crate) fn close_lightbox(st: &mut State) {
    super::super::w4::reset_zoom(st);
    let Some(details) = st.browse_view.details.as_mut() else {
        return;
    };
    // The original it showed goes with it.
    let shown = details.lightbox.take().and_then(|index| details.images().get(index).cloned());
    if let Some(image) = shown
        && image.full_url != image.url
    {
        st.thumbnails.remove(&image.full_url);
    }
}

/// The full size image over the whole window, with arrows, a counter and the caption.
pub(crate) fn lightbox(s: &State, u: &Ui, d: &Details, index: usize) -> El {
    let images = d.images();
    let count = images.len();
    let image = images.get(index);
    let pixels = image.and_then(|image| {
        s.thumbnails
            .get(&image.full_url)
            .or_else(|| s.thumbnails.get(&image.url))
    });
    let veil = veiled(s, d.id, d.nsfw);
    let round = |key: &str, glyph: &'static Icon, label: &str, size: f32| {
        el().key(key.to_string())
            .size(size, size)
            .center()
            .radius(size / 2.0)
            .fill(Color::BLACK.alpha(0.6))
            .border(1.0, Color::WHITE.alpha(0.15))
            .hover(Color::BLACK.alpha(0.85))
            .focusable(true)
            .role("button")
            .label(crate::screens::settings::i18n::tr(label).to_string())
            .cursor(rune::Cursor::Pointer)
            .child(icon(glyph, size / 2.0).color(u.ink).a11y_hidden())
    };
    // The picture box is fitted to the image's own ratio, so its close button and counter sit
    // on the picture, clear of the title bar and the window's own close button (issue #3).
    let ratio = pixels
        .map(|p| p.size())
        .filter(|(w, h)| *w > 0 && *h > 0)
        .map(|(w, h)| w as f32 / h as f32)
        .or_else(|| image.map(|image| image.aspect).filter(|a| *a > 0.2))
        .unwrap_or(16.0 / 9.0);
    let mut picture = el()
        .key(format!("lightbox-image-{index}"))
        .clip_rounded(0.0);
    if let Some(pixels) = pixels {
        picture = picture.pixels_fit(pixels, SurfaceFit::Fill);
        if veil {
            picture = picture.blur(crate::screens::settings::nsfw::lightbox_blur());
        }
    } else {
        picture = picture.fill(Color::WHITE.alpha(0.04));
    }
    let menu_url = image.map(|image| if image.full_url.is_empty() { image.url.clone() } else { image.full_url.clone() }).unwrap_or_default();
    // Wave 4: zoom and pan (wheel, pinch, drag, double click, the zoom bar).
    let zoom = super::super::w4::zoom(s);
    let pan = s.browse_view.w4.pan;
    let picture = picture.scale(zoom).offset(pan.0, pan.1);
    let frame = super::super::w4::zoomable(el())
        .key("lightbox-frame")
        .aspect_fit(ratio)
        .clip_rounded(0.0)
        .on_press(|_: &mut State| {})
        // Wave 4: Grimoire's image menu on the full size picture.
        .on_secondary_press(move |st: &mut State, _press, _tx: &mut Tx| {
            super::super::w4::open_image_menu(st, "lightbox-frame".to_string(), menu_url.clone())
        })
        .child(picture.absolute(0.0, 0.0).w_full().h_full().a11y_hidden())
        .child(
            round("lightbox-close", &icons::X, "Close full size view", 32.0)
                .pin(None, Some(10.0), Some(10.0), None)
                .on_press(close_lightbox),
        )
        .child(image_badge(u, format!("{} / {count}", index + 1)).pin(
            Some(10.0),
            Some(10.0),
            None,
            None,
        ))
        .child(super::super::w4::zoom_bar(s, u).pin(None, None, Some(10.0), Some(10.0)));
    let stage = el()
        .key("lightbox-stage")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .center()
        .child(frame);
    let mut layer = full_layer("browse-lightbox", Color::BLACK.alpha(0.95))
        .role("dialog")
        .label(crate::screens::settings::i18n::fill(
            crate::screens::settings::i18n::tr("{} full size"),
            &[&d.name],
        ))
        .padding(16.0)
        .pt(64.0)
        .gap(12.0)
        .items_center()
        .modal(true, close_lightbox)
        // A press on the backdrop closes; the picture swallows its own presses.
        .on_press(close_lightbox)
        .on_key(|st: &mut State, ev: &KeyEvent, _| match ev.key {
            Key::Escape => {
                close_lightbox(st);
                true
            }
            Key::Left => {
                step_lightbox(st, false);
                true
            }
            Key::Right => {
                step_lightbox(st, true);
                true
            }
            _ => false,
        })
        .child(stage);
    if let Some(caption) = image.map(|image| image.caption.trim()).filter(|c| !c.is_empty()) {
        layer = layer.child(
            text(caption.to_string())
                .font(BODY)
                .color(u.muted)
                .max_w(720.0)
                .wrap()
                .align(rune::Align::Center),
        );
    }
    if count > 1 {
        // Each arrow in its own edge strip, so the picture itself is never under a step row.
        let strip = |key: &str, left: bool| {
            col()
                .key(key.to_string())
                .w(40.0)
                .center()
                .pin(
                    left.then_some(16.0),
                    Some(72.0),
                    (!left).then_some(16.0),
                    Some(72.0),
                )
        };
        layer = layer
            .child(strip("lightbox-steps", true).child(
                round("lightbox-prev", &icons::CHEVRON_LEFT, "Previous image", 40.0)
                    .on_press(|st: &mut State| step_lightbox(st, false)),
            ))
            .child(strip("lightbox-steps-next", false).child(
                round("lightbox-next", &icons::CHEVRON_RIGHT, "Next image", 40.0)
                    .on_press(|st: &mut State| step_lightbox(st, true)),
            ));
    }
    layer
}

