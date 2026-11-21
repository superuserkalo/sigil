//! Cards, the list layout and the virtual rows.

use super::*;

// --- cards -----------------------------------------------------------------------

pub(crate) fn pixels_for<'a>(s: &'a State, url: Option<&String>) -> Option<&'a rune::Pixels> {
    url.and_then(|url| s.thumbnails.get(url))
}

/// An image filling its parent. A hidden mature image is blurred under the parent's clip.
pub(crate) fn image_fill(s: &State, key: String, url: Option<&String>, radii: [f32; 4], blur: bool) -> El {
    let base = el().absolute(0.0, 0.0).w_full().h_full().corners(radii);
    match pixels_for(s, url) {
        Some(pixels) => {
            let image = base.key(key).pixels_fit(pixels, SurfaceFit::Fill).a11y_hidden();
            if blur { image.blur(crate::screens::settings::nsfw::card_blur()) } else { image }
        }
        None => base
            .key(format!("{key}-missing"))
            .fill(Color::WHITE.alpha(0.045))
            .a11y_hidden(),
    }
}

/// The veil over a mature image: an NSFW tag and a Show button that reveals the mod's images.
pub(crate) fn nsfw_veil(u: &Ui, scope: &str, id: u64, compact: bool) -> El {
    col()
        .key(format!("nsfw-veil-{scope}-{id}"))
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .center()
        .gap(if compact { 8.0 } else { 12.0 })
        .fill(Color::BLACK.alpha(0.30))
        .child(nsfw_tag(u))
        .child(
            row()
                .key(format!("nsfw-show-{scope}-{id}"))
                .h(30.0)
                .px(14.0)
                .gap(6.0)
                .center()
                .radius(crate::ui::r(15.0))
                .fill(Color::BLACK.alpha(0.35))
                .border(1.0, Color::WHITE.alpha(0.18))
                .hover(Color::WHITE.alpha(0.08))
                .press(Color::WHITE.alpha(0.12))
                .focusable(true)
                .role("button")
                .label("Show mature preview")
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    crate::screens::settings::nsfw::revealed(st, id);
                })
                // On a darkened image in every theme, so the ink is always white.
                .child(icon(&icons::EYE, 14.0).color(Color::WHITE).a11y_hidden())
                .child(text("Show").font(LABEL).color(Color::WHITE).a11y_hidden()),
        )
}

pub(crate) fn nsfw_tag(u: &Ui) -> El {
    row()
        .h(22.0)
        .px(8.0)
        .center()
        .radius(crate::ui::r(6.0))
        .fill(u.rose.alpha(0.16))
        .border(1.0, u.rose.alpha(0.45))
        .child(text("NSFW").font(MICRO_CAPS).color(u.rose))
}

pub(crate) fn stat(u: &Ui, glyph: &'static Icon, value: u64) -> El {
    row()
        .gap(4.0)
        .center_y()
        .child(icon(glyph, 14.0).color(u.faint).a11y_hidden())
        .child(text(format_count(value)).font(SMALL).color(u.faint))
}

pub(crate) fn veiled(s: &State, id: u64, nsfw: bool) -> bool {
    use crate::screens::settings::nsfw;
    nsfw && s.prefs.mature == crate::prefs::Mature::Blur
        && !s.browse_view.revealed.contains(&id)
        && !nsfw::remembered(id)
        // Settings > Mods > Reveal on hover.
        && !(nsfw::reveal_on_hover() && s.browse_view.hovered == Some(id))
}

/// A card's width less its border and text padding: what the footer row gets.
const FOOTER_INSET: f32 = 24.0;
const FOOTER_GAP: f32 = 12.0;
/// A generous width for one character of the footer's small text and label fonts.
const FOOTER_CHAR: f32 = 7.4;

/// The card button with words: padding, icon, gap, label, and Cancel's X while busy.
fn worded_button_w(b: &super::super::quick::Button) -> f32 {
    let cancel = if b.busy() { 18.0 } else { 0.0 };
    18.0 + 14.0 + 6.0 + b.short().chars().count() as f32 * FOOTER_CHAR + cancel
}

/// The card button without words: a square, or the percentage while it downloads.
fn icon_button_w(b: &super::super::quick::Button) -> f32 {
    use super::super::quick::Button;
    let cancel = if b.busy() { 18.0 } else { 0.0 };
    match b {
        Button::Downloading(Some(_)) | Button::Paused(Some(_)) => {
            18.0 + b.short().chars().count() as f32 * FOOTER_CHAR + cancel
        }
        _ => 28.0,
    }
}

/// How many leading counts fit beside the actions in a footer `room` points wide.
fn stats_that_fit(room: f32, actions: f32, labels: &[String]) -> usize {
    let mut used = actions;
    let mut keep = 0;
    for label in labels {
        used += FOOTER_GAP + 18.0 + label.chars().count() as f32 * FOOTER_CHAR;
        if used > room {
            break;
        }
        keep += 1;
    }
    keep
}

/// One mod: a 16:9 image flush with the top, the title, the author and when it changed, and
/// a footer with likes, views and Install.
pub(crate) fn card_el(s: &State, u: &Ui, card: &BrowseCard, card_w: f32) -> El {
    let inner = (crate::ui::r(CARD_RADIUS) - 1.0).max(0.0);
    let id = card.id;
    let veil = veiled(s, id, card.nsfw);
    let age = gamebanana::relative_age(card.updated_ts, gamebanana::now_unix());

    let mut frame = el()
        .w_full()
        .aspect_ratio(16.0 / 9.0)
        .clip_corners([inner, inner, 0.0, 0.0])
        .child(image_fill(
            s,
            format!("thumb-{id}"),
            card.thumbnail_url.as_ref(),
            [inner, inner, 0.0, 0.0],
            veil,
        ));
    if veil {
        frame = frame.child(nsfw_veil(u, "card", id, true));
    } else if card.nsfw && crate::screens::settings::nsfw::always_tag() {
        // Settings > Mods > Always show the 18+ tag.
        frame = frame.child(el().pin(Some(8.0), Some(8.0), None, None).child(nsfw_tag(u)));
    }

    let byline = row()
        .w_full()
        .gap(6.0)
        .center_y()
        .child(
            text(if card.author.is_empty() {
                "Unknown author".to_string()
            } else {
                format!("by {}", card.author)
            })
            .font(SMALL)
            .color(u.muted)
            .truncate()
            .min_w(0.0)
            .shrink(),
        )
        .child(spacer())
        .child(
            text(age)
                .font(SMALL)
                .color(if outdated(card) { u.amber } else { u.faint }),
        );

    let is_mod = card.model.eq_ignore_ascii_case("mod");
    // Each card reads its own mod's state, so one install never changes another card. Sounds
    // and WiPs install by file link: the details list their files.
    let state = if is_mod {
        super::super::quick::button(s, id)
    } else {
        super::super::quick::Button::Install
    };
    let room = card_w - FOOTER_INSET;
    let size = if card_w >= CARD_LABEL_MIN && worded_button_w(&state) <= room {
        super::super::quick::Size::Card
    } else {
        super::super::quick::Size::Icon
    };
    let button_w = if size == super::super::quick::Size::Card { worded_button_w(&state) } else { icon_button_w(&state) };
    let install = super::super::quick::button_el(u, &state, format!("install-{id}"), id, &card.name, size);
    let install = if is_mod {
        install
    } else {
        install.on_press(move |st: &mut State| open_details(st, id))
    };
    // While this mod installs, or after it failed, its own state takes the byline's place.
    let byline = match is_mod.then(|| super::super::quick::status_line(s, u, id)).flatten() {
        Some(line) => line,
        None => byline,
    };
    if is_mod {
        if let Some(bar) = super::super::quick::image_bar(s, u, id) {
            frame = frame.child(bar);
        }
        if !veil && let Some(badge) = super::super::quick::installed_badge(s, id) {
            frame = frame.child(badge);
        }
    }

    let saved = is_favorite(s, &card.model, id);
    let favorite = Favorite::from_card(card);
    let star = el()
        .key(format!("star-{id}"))
        .pin(Some(8.0), None, Some(8.0), None)
        .size(28.0, 28.0)
        .center()
        .radius(crate::ui::r(14.0))
        .fill(Color::BLACK.alpha(0.45))
        .hover(Color::BLACK.alpha(0.7))
        .focusable(true)
        .role("button")
        .label(if saved { "Remove from favorites" } else { "Save to favorites" })
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| toggle_favorite(st, favorite.clone()))
        // On a darkened image in every theme, so the ink is always white.
        .child(
            icon(if saved { &STAR_FILLED } else { &STAR }, 14.0)
                .color(Color::WHITE)
                .a11y_hidden(),
        );
    let hovered = s.browse_view.hovered == Some(id);
    if !veil && (saved || hovered) {
        frame = frame.child(star);
    }

    let mut footer = row().w_full().h(28.0).mt(8.0).gap(FOOTER_GAP).center_y();
    let playing = s.browse_view.audio.playing == Some(id);
    let sample = |size: f32, big: bool| {
        let url = card.audio_url.clone().unwrap_or_default();
        let mut button = if big {
            el().key(format!("sample-big-{id}"))
                .size(size, size)
                .center()
                .radius(size / 2.0)
                .fill(u.over.alpha(0.08))
                .border(1.0, u.hair_strong)
        } else {
            el().key(format!("sample-{id}"))
                .size(size, size)
                .center()
                .radius(crate::ui::r(8.0))
        };
        button = button
            .hover(u.hover)
            .focusable(true)
            .role("button")
            .label(if playing { "Stop the sample" } else { "Play the sample" })
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| {
                st.browse_view.audio.toggle(id, url.clone());
            })
            .child(
                icon(if playing { &icons::SQUARE_STOP } else { &icons::PLAY }, if big { 20.0 } else { 14.0 })
                    .color(if big { u.ink } else { u.muted })
                    .a11y_hidden(),
            );
        button
    };
    // A sound without art gets one large play button in the image slot instead.
    let bare_sound = card.thumbnail_url.is_none() && card.audio_url.is_some();
    if bare_sound && !veil {
        frame = frame.child(
            col()
                .absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .center()
                .gap(8.0)
                .child(sample(48.0, true))
                .child(text(card.category.clone()).font(SMALL).color(u.faint).truncate()),
        );
    }
    // The action row never outgrows the card: the button and sample keep their size, and the
    // counts give way from the least important one. The clip covers a font wider than the
    // estimate.
    let small_sample = card.audio_url.is_some() && !bare_sound;
    let mut counts: Vec<(&'static Icon, u64)> = vec![(&icons::THUMBS_UP, card.likes), (&icons::EYE, card.views)];
    if let Some(downloads) = s.browse_view.download_counts.get(&id) {
        counts.push((&icons::DOWNLOAD, *downloads));
    }
    let labels: Vec<String> = counts.iter().map(|(_, value)| format_count(*value)).collect();
    let actions = button_w + if small_sample { 28.0 + FOOTER_GAP } else { 0.0 };
    let keep = stats_that_fit(room, actions, &labels);
    let mut stats = row().grow(1.0).shrink().min_w(0.0).h_full().gap(FOOTER_GAP).center_y().clip_rounded(0.0);
    for (glyph, value) in counts.into_iter().take(keep) {
        stats = stats.child(stat(u, glyph, value));
    }
    footer = footer.child(stats);
    if small_sample {
        footer = footer.child(sample(28.0, false));
    }
    footer = footer.child(install);

    let mut card_el = col()
        .key(format!("browse-card-{id}"))
        .min_w(0.0)
        .padding(1.0)
        .radius(crate::ui::r(CARD_RADIUS))
        .on_hover_change(move |st: &mut State, over: bool, _tx: &mut Tx| {
            if over {
                st.browse_view.hovered = Some(id);
                st.browse_view.hover_since = Some((id, Instant::now()));
            } else if st.browse_view.hovered == Some(id) {
                st.browse_view.hovered = None;
                st.browse_view.hover_since = None;
            }
        })
        .on_secondary_press(move |st: &mut State, _press, _tx: &mut Tx| {
            st.browse_view.context = Some(id);
            st.browse_view.menu = None;
        })
        .fill(u.well)
        .border(1.0, u.hair)
        .hover(Color::WHITE.alpha(0.025))
        .focusable(true)
        .role("button")
        .label(format!("{} by {}", card.name, card.author))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| open_details(st, id))
        .child(frame)
        .child(
            col()
                .w_full()
                .px(11.0)
                .pt(10.0)
                .pb(9.0)
                .gap(2.0)
                .child(text(card.name.clone()).font(H2).color(u.ink).truncate())
                .child(byline)
                .child(footer),
        );
    if playing && s.browse_view.audio.error.is_none() {
        card_el = card_el.border(1.0, u.accent.alpha(0.35));
    }
    card_el
}

pub(crate) fn skeleton(u: &Ui, w: f32, h: f32) -> El {
    el().h(h).w(w).radius(crate::ui::r(4.0)).fill(u.hover)
}

pub(crate) fn loading_grid(u: &Ui, cols: usize) -> El {
    let mut grid = el().w_full().equal_columns(cols).gap(GRID_GAP);
    for _ in 0..cols * 2 {
        grid = grid.child(
            col()
                .min_w(0.0)
                .padding(1.0)
                .radius(crate::ui::r(CARD_RADIUS))
                .fill(u.well)
                .border(1.0, u.hair)
                .child(
                    el().w_full()
                        .aspect_ratio(16.0 / 9.0)
                        .corners([(crate::ui::r(CARD_RADIUS) - 1.0).max(0.0), (crate::ui::r(CARD_RADIUS) - 1.0).max(0.0), 0.0, 0.0])
                        .fill(u.hover),
                )
                .child(
                    col()
                        .px(11.0)
                        .pt(14.0)
                        .pb(16.0)
                        .gap(10.0)
                        .child(skeleton(u, 140.0, 10.0))
                        .child(skeleton(u, 80.0, 8.0))
                        .child(el().h(24.0)),
                ),
        );
    }
    grid
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(list: &[&str]) -> Vec<String> {
        list.iter().map(|l| l.to_string()).collect()
    }

    #[test]
    fn the_counts_give_way_before_the_action_row_leaves_the_card() {
        let big = labels(&["1.2m", "987.7m", "45.7m"]);
        for width in (120..=420).step_by(4) {
            let room = width as f32 - FOOTER_INSET;
            let actions = 90.0;
            let keep = stats_that_fit(room, actions, &big);
            let used: f32 = actions
                + big[..keep]
                    .iter()
                    .map(|l| FOOTER_GAP + 18.0 + l.chars().count() as f32 * FOOTER_CHAR)
                    .sum::<f32>();
            assert!(keep == 0 || used <= room, "{width}: {used} > {room}");
        }
        assert_eq!(stats_that_fit(400.0, 90.0, &big), 3);
        assert_eq!(stats_that_fit(60.0, 90.0, &big), 0);
    }

    #[test]
    fn small_counts_all_stay() {
        assert_eq!(stats_that_fit(196.0, 80.0, &labels(&["12", "340"])), 2);
    }
}
