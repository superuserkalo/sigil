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

/// Every preview stacked at its own aspect ratio; a press opens the lightbox there.
pub(crate) fn gallery_stack(s: &State, u: &Ui, d: &Details) -> El {
    let images = d.images();
    let veil = veiled(s, d.id, d.nsfw);
    if images.is_empty() {
        // Before the profile lands, the card image holds the first slot.
        let mut frame = el()
            .key("gallery-pending")
            .w_full()
            .aspect_ratio(16.0 / 9.0)
            .radius(crate::ui::r(8.0))
            .border(1.0, u.hair)
            .clip_rounded(crate::ui::r(8.0))
            .child(image_fill(
                s,
                "gallery-card".to_string(),
                d.thumbnail_url.as_ref(),
                [8.0; 4],
                veil,
            ));
        if veil {
            frame = frame.child(nsfw_veil(u, "pending", d.id, false));
        }
        let mut c = col().w_full().gap(12.0).child(frame);
        if d.loading {
            c = c.child(
                el().w_full()
                    .aspect_ratio(16.0 / 10.0)
                    .radius(crate::ui::r(8.0))
                    .fill(u.hover),
            );
        } else if d.profile.is_some() {
            c = c.child(text("No preview images.").font(SMALL).color(u.faint));
        }
        return c;
    }
    let count = images.len();
    let mut c = col().w_full().gap(12.0);
    for (index, image) in images.iter().enumerate() {
        let aspect = if image.aspect > 0.2 {
            image.aspect
        } else {
            16.0 / 9.0
        };
        let mut frame = el()
            .key(format!("gallery-{index}"))
            .w_full()
            .aspect_ratio(aspect)
            .radius(crate::ui::r(8.0))
            .fill(Color::WHITE.alpha(0.03))
            .border(1.0, u.hair)
            .clip_rounded(crate::ui::r(8.0))
            .focusable(true)
            .role("button")
            .label(format!("View image {} of {count} full size", index + 1))
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| open_lightbox(st, index))
            .child(image_fill(
                s,
                format!("gallery-image-{index}"),
                Some(&image.url),
                [8.0; 4],
                veil,
            ));
        if veil {
            frame = frame.child(nsfw_veil(u, &format!("gallery{index}"), d.id, false));
        } else {
            if count > 1 {
                frame = frame.child(
                    image_badge(u, format!("{} / {count}", index + 1)).absolute(8.0, 8.0),
                );
            }
            frame = frame.child(
                el().pin(None, Some(8.0), Some(8.0), None)
                    .size(28.0, 28.0)
                    .center()
                    .radius(crate::ui::r(6.0))
                    .fill(Color::BLACK.alpha(0.55))
                    .border(1.0, Color::WHITE.alpha(0.10))
                    .child(icon(&MAXIMIZE_2, 14.0).color(u.prose).a11y_hidden()),
            );
        }
        c = c.child(frame);
    }
    c
}

/// The docked view's single image slot with arrows and a counter.
pub(crate) fn carousel(s: &State, u: &Ui, d: &Details) -> El {
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
        .fill(Color::WHITE.alpha(0.03))
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
                .size(30.0, 30.0)
                .center()
                .radius(15.0)
                .fill(Color::BLACK.alpha(0.55))
                .border(1.0, Color::WHITE.alpha(0.15))
                .hover(Color::BLACK.alpha(0.8))
                .focusable(true)
                .role("button")
                .label(if next { "Next image" } else { "Previous image" })
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| step_carousel(st, next))
                .child(
                    icon(
                        if next {
                            &icons::CHEVRON_RIGHT
                        } else {
                            &icons::CHEVRON_LEFT
                        },
                        16.0,
                    )
                    .color(u.ink)
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
    if let Some(details) = st.browse_view.details.as_mut() {
        details.lightbox = None;
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
    let mut stage = el().key("lightbox-stage").w_full().grow(1.0).min_h(0.0);
    if let Some(pixels) = pixels {
        let picture = el()
            .key(format!("lightbox-image-{index}"))
            .w_full()
            .h_full()
            .pixels_fit(pixels, SurfaceFit::Fit)
            .a11y_hidden();
        stage = stage
            .clip_rounded(0.0)
            .child(if veil { picture.blur(crate::screens::settings::nsfw::lightbox_blur()) } else { picture });
    }
    let round = |key: &str, glyph: &'static Icon, label: &str| {
        el().key(key.to_string())
            .size(40.0, 40.0)
            .center()
            .radius(20.0)
            .fill(Color::BLACK.alpha(0.6))
            .border(1.0, Color::WHITE.alpha(0.15))
            .hover(Color::BLACK.alpha(0.85))
            .focusable(true)
            .role("button")
            .label(label.to_string())
            .cursor(rune::Cursor::Pointer)
            .child(icon(glyph, 20.0).color(u.ink).a11y_hidden())
    };
    let mut layer = full_layer("browse-lightbox", Color::BLACK.alpha(0.95))
        .role("dialog")
        .label(format!("{} full size", d.name))
        .padding(16.0)
        .pt(64.0)
        .gap(12.0)
        .items_center()
        .modal(true, close_lightbox)
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
    layer = layer
        .child(
            round("lightbox-close", &icons::X, "Close full size view")
                .pin(None, Some(16.0), Some(16.0), None)
                .on_press(close_lightbox),
        )
        .child(image_badge(u, format!("{} / {count}", index + 1)).pin(
            Some(16.0),
            Some(24.0),
            None,
            None,
        ));
    if count > 1 {
        layer = layer.child(
            row()
                .key("lightbox-steps")
                .pin(Some(16.0), Some(72.0), Some(16.0), Some(72.0))
                .center_y()
                .child(
                    round("lightbox-prev", &icons::CHEVRON_LEFT, "Previous image")
                        .on_press(|st: &mut State| step_lightbox(st, false)),
                )
                .child(spacer())
                .child(
                    round("lightbox-next", &icons::CHEVRON_RIGHT, "Next image")
                        .on_press(|st: &mut State| step_lightbox(st, true)),
                ),
        );
    }
    layer
}

