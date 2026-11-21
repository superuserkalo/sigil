//! Game art: the hero's own textures from the game files, as thumbnails (asset catalog
//! previews).
//!
//! - Grimoire: Foundry's Library (ability icons, item icons, hero images as a thumbnail grid with
//!   a category and hero filter and a search) and Texture browse (hero model and effect textures
//!   as a text list); a lightbox with the native size and Save PNG.
//! - Deadlock Mod Manager: no catalog of the game; its Foundry opens one skin's files with
//!   groups, a filter, a preview and a properties panel.
//! - Sigil: a Game art section on every hero page. One grid for all of the hero's textures with
//!   kind chips (cards and portraits, ability icons, skin textures, ability effects, other) and a
//!   search over name and path; thumbnails for every kind, a page at a time. The lightbox shows the
//!   path, the native size, Copy path, Save PNG, and for a card "Use for an in-game card".

use rune::{El, FileDialogOptions, SurfaceFit, Tx, col, el, row, spacer, text};

use super::assets::{self, Load, Tex, TexKind, thumb_key};
use super::{HeroCard, View};
use crate::app::State;
use crate::icons;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{ActionKind, BODY, BODY_MED, H1, MONO, SMALL, SMALL_NUM, Ui};

/// Tiles per page of the grid.
pub const PAGE: usize = 48;
const COLS: usize = 4;
const GAP: f32 = 8.0;
/// The pane's content width (the pane is 480 with 24 padding).
const CONTENT_W: f32 = 432.0;

fn tile_w() -> f32 {
    ((CONTENT_W - GAP * (COLS as f32 - 1.0)) / COLS as f32).floor()
}

/// The textures the grid shows now, after the kind chip and the search.
pub fn matches(v: &View) -> Vec<&Tex> {
    let a = &v.assets;
    let Some(list) = a.textures.ready() else { return Vec::new() };
    let query = a.tex_query.text.trim().to_ascii_lowercase();
    list.iter()
        .filter(|t| a.tex_kind.is_none_or(|kind| t.kind == kind))
        .filter(|t| {
            query.is_empty() || t.label.to_ascii_lowercase().contains(&query) || t.path.to_ascii_lowercase().contains(&query)
        })
        .collect()
}

/// The texture paths on screen (the shown page and the lightbox), for the thumbnail cap.
pub fn shown_paths(st: &State) -> Vec<String> {
    let v = &st.locker;
    if v.hero_section != super::HeroSection::GameArt {
        return Vec::new();
    }
    let mut out: Vec<String> = matches(v).into_iter().take(v.assets.tex_shown.max(PAGE)).map(|t| t.path.clone()).collect();
    out.extend(v.assets.lightbox.clone());
    out
}

/// The pane body.
pub fn section(s: &State, u: &Ui, v: &View, hero: &HeroCard) -> El {
    let a = &v.assets;
    let total = a.textures.ready().map_or(0, Vec::len);
    let head = super::pane_head(
        u,
        tr("Game art"),
        if a.textures.ready().is_some() { fill(tr("{} textures"), &[&total.to_string()]) } else { String::new() },
        None,
    );
    let mut c = col().w_full().gap(12.0).child(head);
    match &a.textures {
        Load::Idle | Load::Loading => {
            return c
                .child(
                    text(tr("Reading the game's textures..."))
                        .font(BODY)
                        .color(u.muted)
                        .wrap(),
                )
                .child(skeleton(u));
        }
        Load::Failed(error) => {
            let retry = u
                .action(tr("Try again"), ActionKind::Secondary)
                .key("locker-art-retry")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| {
                    st.locker.assets.textures = Load::Idle;
                    assets::ensure_loaded(st);
                });
            return c
                .child(text(tr("The game's textures could not be read")).font(BODY_MED).color(u.ink).wrap())
                .child(text(error.clone()).font(SMALL).color(u.muted).wrap())
                .child(row().child(retry));
        }
        Load::Ready(_) => {}
    }
    if total == 0 {
        return c.child(
            text(fill(tr("The game files have no textures for {}."), &[&hero.display_name]))
                .font(BODY)
                .color(u.muted)
                .wrap(),
        );
    }

    c = c.child(row().w_full().h(32.0).gap(8.0).center_y().child(search(u, v)).child(kind_chips(u, v)));
    let list = matches(v);
    if list.is_empty() {
        return c.child(
            col()
                .w_full()
                .gap(4.0)
                .pt(16.0)
                .child(text(tr("No textures match")).font(BODY_MED).color(u.ink))
                .child(text(tr("Try another kind or a shorter search.")).font(SMALL).color(u.muted)),
        );
    }
    let shown = a.tex_shown.max(PAGE).min(list.len());
    let mut grid = col().w_full().gap(GAP);
    for chunk in list[..shown].chunks(COLS) {
        let mut r = row().w_full().gap(GAP);
        for tex in chunk {
            r = r.child(tile(s, u, tex));
        }
        grid = grid.child(r);
    }
    c = c.child(grid);
    if shown < list.len() {
        c = c.child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(
                    text(fill(tr("Showing {} of {}"), &[&shown.to_string(), &list.len().to_string()]))
                        .font(SMALL)
                        .color(u.faint)
                        .grow(1.0),
                )
                .child(
                    u.action(tr("Show more"), ActionKind::Secondary)
                        .key("locker-art-more")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| st.locker.assets.tex_shown = st.locker.assets.tex_shown.max(PAGE) + PAGE),
                ),
        );
    }
    c
}

fn search(u: &Ui, v: &View) -> El {
    let field = rune::text_input(&v.assets.tex_query, BODY)
        .key("locker-art-search")
        .label(tr("Search textures"))
        .single_line()
        .color(u.ink)
        .placeholder(tr("Search textures"), u.faint)
        .caret(u.accent, u.accent.alpha(0.20))
        .on_edit(|st: &mut State| {
            st.locker.assets.tex_shown = PAGE;
            &mut st.locker.assets.tex_query
        })
        .on_input_key(|st: &mut State, ev: &rune::KeyEvent, _| {
            if ev.key == rune::Key::Escape && !st.locker.assets.tex_query.text.is_empty() {
                st.locker.assets.tex_query = rune::TextEdit::new("");
                true
            } else {
                false
            }
        });
    u.search_shell(field).grow(1.0).min_w(0.0)
}

/// The kind choice as a 160 point select; its menu (`kind_menu`) lists each kind with a count.
fn kind_chips(u: &Ui, v: &View) -> El {
    let a = &v.assets;
    let now = match a.tex_kind {
        None => tr("All").to_string(),
        Some(kind) => kind.label().to_string(),
    };
    u.select_box(&now, 160.0)
        .key("locker-art-kind")
        .fill(if a.kind_open { u.selected } else { u.well })
        .on_press(|st: &mut State| st.locker.assets.kind_open = !st.locker.assets.kind_open)
}

/// The kind menu, mounted at the root.
pub fn kind_menu(s: &State, u: &Ui) -> Option<El> {
    let a = &s.locker.assets;
    if !a.kind_open {
        return None;
    }
    let list = a.textures.ready().map(Vec::as_slice).unwrap_or(&[]);
    let item = |key: String, label: String, n: usize, on: bool, kind: Option<TexKind>| {
        row()
            .key(key)
            .w_full()
            .h(32.0)
            .px(8.0)
            .gap(8.0)
            .center_y()
            .radius(crate::ui::r(4.0))
            .hover(u.hover)
            .focusable(true)
            .role("option")
            .checked(on)
            .label(format!("{label}, {n}"))
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| {
                st.locker.assets.tex_kind = kind;
                st.locker.assets.tex_shown = PAGE;
                st.locker.assets.kind_open = false;
            })
            .child(text(label).font(BODY).color(if on { u.ink } else { u.prose }).grow(1.0).min_w(0.0).truncate().a11y_hidden())
            .child(text(n.to_string()).font(SMALL_NUM).color(u.muted).a11y_hidden())
    };
    let mut menu = u
        .menu("locker-art-kind-menu", "locker-art-kind", 220.0)
        .role("listbox")
        .child(item("locker-art-kind-all".into(), tr("All").to_string(), list.len(), a.tex_kind.is_none(), None));
    for kind in TexKind::ALL {
        let n = list.iter().filter(|t| t.kind == kind).count();
        if n == 0 {
            continue;
        }
        menu = menu.child(item(format!("locker-art-kind-{kind:?}").to_lowercase(), kind.label().to_string(), n, a.tex_kind == Some(kind), Some(kind)));
    }
    Some(u.popover_layer(
        "locker-art-kind-layer",
        rune::el().on_press(|st: &mut State| st.locker.assets.kind_open = false),
        menu,
    ))
}

/// A square picture of one texture, or a quiet box while it decodes.
fn picture(s: &State, u: &Ui, path: &str, w: f32, h: f32) -> El {
    let base = el().size(w, h).radius(crate::ui::r(8.0)).clip_rounded(crate::ui::r(8.0)).fill(u.well);
    match s.thumbnails.get(&thumb_key(path)) {
        Some(pixels) if pixels.size().0 > 0 => base.child(el().size(w, h).pixels_fit(pixels, SurfaceFit::Fit).a11y_hidden()),
        _ => {
            let failed = s.locker.assets.thumbs.failed.contains(path);
            base.center().child(
                rune::icon(if failed { &icons::TRIANGLE_ALERT } else { &icons::BOX }, 16.0)
                    .color(u.faint.alpha(0.6))
                    .a11y_hidden(),
            )
        }
    }
}

fn tile(s: &State, u: &Ui, tex: &Tex) -> El {
    let w = tile_w();
    let path = tex.path.clone();
    col()
        .key(format!("locker-art-{}", tex.path))
        .w(w)
        .gap(4.0)
        .focusable(true)
        .role("button")
        .label(fill(tr("Open {}"), &[&tex.label]))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| st.locker.assets.lightbox = Some(path.clone()))
        .child(picture(s, u, &tex.path, w, w).hover(u.hover))
        .child(text(tex.label.clone()).font(SMALL).color(u.muted).truncate().a11y_hidden())
}

fn skeleton(u: &Ui) -> El {
    let w = tile_w();
    let mut grid = col().w_full().gap(GAP).a11y_hidden();
    for _ in 0..2 {
        let mut r = row().w_full().gap(GAP);
        for _ in 0..COLS {
            r = r.child(el().size(w, w).radius(crate::ui::r(8.0)).fill(u.well));
        }
        grid = grid.child(r);
    }
    grid
}

// --- lightbox ---------------------------------------------------------------------

fn close(st: &mut State) {
    st.locker.assets.lightbox = None;
}

fn save_png(st: &mut State, tx: &mut Tx) {
    let Some(path) = st.locker.assets.lightbox.clone() else { return };
    let stem = path.rsplit('/').next().unwrap_or("texture").trim_end_matches(".vtex_c").to_string();
    let options = FileDialogOptions {
        title: tr("Save the preview as PNG").to_string(),
        file_name: Some(format!("{stem}.png")),
        ..FileDialogOptions::default()
    }
    .filter("PNG", &["png"]);
    tx.save_file(options, move |st: &mut State, result, _tx| {
        let Ok(Some(out)) = result else { return };
        let Some(game_root) = st.game_root.clone() else { return };
        let path = path.clone();
        st.locker.assets.save_job.start(st.wake.clone(), move || {
            let bytes = assets::api::preview_png(&game_root, &path)?;
            std::fs::write(&out, bytes).map_err(|error| error.to_string())?;
            Ok(out.display().to_string())
        });
    });
}

/// The lightbox dialog, when a tile is open.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    let v = &s.locker;
    let path = v.assets.lightbox.as_ref()?;
    let tex = v.assets.textures.ready()?.iter().find(|t| &t.path == path)?;
    let hero = match &v.page {
        super::Page::Hero(name) => name.clone(),
        _ => String::new(),
    };
    let mut facts = col()
        .w_full()
        .gap(4.0)
        .child(text(format!("{hero} · {}", tex.kind.label())).font(SMALL).color(u.muted))
        .child(text(tex.path.clone()).font(MONO).color(u.ink).wrap_anywhere().selectable(true));
    if let Some((w, h)) = tex.size.or_else(|| v.assets.thumbs.sizes.get(&tex.path).copied()) {
        facts = facts.child(text(fill(tr("Native size {} × {}"), &[&w.to_string(), &h.to_string()])).font(SMALL).color(u.faint));
    }
    facts = facts.child(
        text(tr("The preview is a copy of at most 256 pixels a side. Save preview writes that copy."))
            .font(SMALL)
            .color(u.faint)
            .wrap(),
    );
    let copy = tex.path.clone();
    let mut buttons = row()
        .w_full()
        .gap(8.0)
        .child(
            u.icon_action(&icons::COPY, tr("Copy path"), ActionKind::Secondary)
                .key("locker-art-copy")
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                    tx.copy(copy.clone());
                    st.status = tr("Path copied").to_string();
                }),
        )
        .child(
            u.icon_action(&icons::DOWNLOAD, tr("Save preview"), ActionKind::Secondary)
                .key("locker-art-save")
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(save_png),
        );
    // Cards and ability icons are 8-bit textures the core can mint from a PNG.
    if matches!(tex.kind, TexKind::Card | TexKind::Ability) {
        let single = (tex.path.clone(), tex.label.clone(), v.assets.thumbs.sizes.get(&tex.path).copied().or(tex.size));
        let card = tex.kind == TexKind::Card;
        buttons = buttons.child(spacer()).child(
            u.action(if card { tr("Replace in game") } else { tr("Replace this icon") }, ActionKind::Primary)
                .key("locker-art-replace")
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    close(st);
                    if card {
                        super::card_art::open_current(st);
                    } else {
                        let (path, label, size) = single.clone();
                        super::card_art::open_single(st, &path, &label, size);
                    }
                }),
        );
    }
    let body = col()
        .w_full()
        .gap(16.0)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(text(tex.label.clone()).font(H1).color(u.ink).grow(1.0).min_w(0.0).truncate())
                .child(
                    u.icon_button(&icons::X, tr("Close"))
                        .key("locker-art-close")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(close),
                ),
        )
        .child(row().w_full().center().child(picture(s, u, &tex.path, 360.0, 360.0)))
        .child(facts)
        .child(buttons);
    Some(
        u.dialog_shell()
            .key("locker-art-lightbox")
            .role("dialog")
            .label(tex.label.clone())
            .w(560.0)
            .max_w(560.0)
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
