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
        .h(20.0)
        .px(6.0)
        .center()
        .radius(crate::ui::r(4.0))
        .fill(u.rose.alpha(0.16))
        .border(1.0, u.rose.alpha(0.45))
        .child(text("NSFW").font(MICRO_CAPS).color(u.rose))
}

pub(crate) fn veiled(s: &State, id: u64, nsfw: bool) -> bool {
    use crate::screens::settings::nsfw;
    nsfw && s.prefs.mature == crate::prefs::Mature::Blur
        && !s.browse_view.revealed.contains(&id)
        && !nsfw::remembered(id)
        // Settings > Mods > Reveal on hover.
        && !(nsfw::reveal_on_hover() && s.browse_view.hovered == Some(id))
}

/// A card's count in at most four characters, so three columns fit the narrowest card:
/// 987, 1.2k, 46k, 988m.
pub(crate) fn card_count(n: u64) -> String {
    if n < 1000 {
        return n.to_string();
    }
    for (base, unit) in [(1e3, 'k'), (1e6, 'm'), (1e9, 'b'), (1e12, 't')] {
        let v = n as f64 / base;
        if v < 9.95 {
            return format!("{v:.1}{unit}");
        }
        if v.round() < 1000.0 {
            return format!("{:.0}{unit}", v.round());
        }
    }
    "999t".to_string()
}

/// One column of the counts row. A count nobody has yet reads as a dash, so the column stays.
fn count_cell(u: &Ui, key: String, glyph: &'static Icon, name: &str, value: Option<u64>) -> El {
    let words = value.map_or_else(|| "-".to_string(), card_count);
    let label = format!("{}: {words}", super::super::tree::tr(name));
    row()
        .key(key)
        .min_w(0.0)
        .h_full()
        .gap(4.0)
        .center_y()
        .label(label)
        .child(icon(glyph, 12.0).color(u.faint).a11y_hidden())
        .child(text(words).font(crate::ui::SMALL_NUM).color(u.muted).a11y_hidden())
}

/// One mod: a 16:9 image flush with the top, the title, the author and when it changed, and
/// the same three counts in equal columns, and a full-width action row.
pub(crate) fn card_el(s: &State, u: &Ui, card: &BrowseCard) -> El {
    let inner = (crate::ui::r(CARD_RADIUS) - 1.0).max(0.0);
    let id = card.id;
    let nsfw = card.nsfw;
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
    // Tags sit together at the top left: the 18+ tag, then the installed state.
    let mut tags = row().key(format!("card-tags-{id}")).pin(Some(8.0), Some(8.0), None, None).gap(4.0);
    if veil {
        frame = frame.child(nsfw_veil(u, "card", id, true));
    } else if card.nsfw && crate::screens::settings::nsfw::always_tag() {
        // Settings > Mods > Always show the 18+ tag.
        tags = tags.child(nsfw_tag(u));
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
                .font(crate::ui::SMALL_NUM)
                .color(if outdated(card) { u.amber } else { u.muted }),
        );

    let is_mod = card.model.eq_ignore_ascii_case("mod");
    // Each card reads its own mod's state, so one install never changes another card. Sounds
    // and WiPs install by file link: the details list their files.
    let state = if is_mod {
        super::super::quick::button(s, id)
    } else {
        super::super::quick::Button::Install
    };
    let install = super::super::quick::button_el(u, &state, format!("install-{id}"), id, &card.name, super::super::quick::Size::Fill);
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
            tags = tags.child(badge);
        }
    }

    let saved = is_favorite(s, &card.model, id);
    let favorite = Favorite::from_card(card);
    let star = el()
        .key(format!("star-{id}"))
        .pin(None, Some(8.0), Some(8.0), None)
        .size(24.0, 24.0)
        .center()
        .radius(crate::ui::r(4.0))
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
    // Always drawn, so hovering a card never rebuilds it (`design/browse.md` 5.10).
    if !veil {
        frame = frame.child(tags).child(star);
    }

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
                .border(1.0, u.hair_strong)
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
                .child(sample(40.0, true))
                .child(text(card.category.clone()).font(SMALL).color(u.faint).truncate()),
        );
    }
    // Every card shows the same three counts in the same columns, whatever its state.
    let counts = el()
        .key(format!("card-counts-{id}"))
        .w_full()
        .h(16.0)
        .mt(4.0)
        .equal_columns(3)
        .gap(4.0)
        .child(count_cell(u, format!("card-stat-likes-{id}"), &icons::THUMBS_UP, "Likes", Some(card.likes)))
        .child(count_cell(u, format!("card-stat-views-{id}"), &icons::EYE, "Views", Some(card.views)))
        .child(count_cell(
            u,
            format!("card-stat-downloads-{id}"),
            &icons::DOWNLOAD,
            "Downloads",
            s.browse_view.download_counts.get(&id).copied(),
        ));
    let mut actions = row().key(format!("card-actions-{id}")).w_full().h(28.0).mt(8.0).gap(6.0).center_y();
    if card.audio_url.is_some() && !bare_sound {
        actions = actions.child(sample(28.0, false));
    }
    let actions = actions.child(install);

    let mut card_el = col()
        .key(format!("browse-card-{id}"))
        .min_w(0.0)
        .padding(1.0)
        .radius(crate::ui::r(CARD_RADIUS))
        .on_hover_change(move |st: &mut State, over: bool, tx: &mut Tx| {
            // Only a hidden mature image that reveals on hover changes how the card draws; the
            // hover itself is for the details prefetch, which reads it from the poll.
            if !(nsfw && crate::screens::settings::nsfw::reveal_on_hover()) {
                tx.live_only();
            }
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
        .hover(u.hover)
        .focusable(true)
        .role("button")
        .label(format!("{} by {}", card.name, card.author))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| open_details(st, id))
        .child(frame)
        .child(
            col()
                .w_full()
                .h(CARD_TEXT_H)
                .px(10.0)
                .pt(8.0)
                .pb(8.0)
                .child(row().w_full().h(20.0).center_y().child(text(card.name.clone()).font(H2).color(u.ink).truncate().min_w(0.0).shrink()))
                .child(row().w_full().h(16.0).mt(4.0).center_y().child(byline))
                .child(counts)
                .child(actions),
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
                        .h(CARD_TEXT_H)
                        .px(10.0)
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

    #[test]
    fn card_counts_never_pass_four_characters() {
        for (n, want) in [
            (0, "0"), (999, "999"), (1_000, "1.0k"), (1_234, "1.2k"), (9_949, "9.9k"), (9_950, "10k"),
            (45_678, "46k"), (999_499, "999k"), (999_500, "1.0m"), (987_654_321, "988m"),
            (1_500_000_000, "1.5b"), (u64::MAX, "999t"),
        ] {
            assert_eq!(card_count(n), want);
            assert!(card_count(n).chars().count() <= 4, "{n}");
        }
    }
}
