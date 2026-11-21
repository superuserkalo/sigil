//! Settings > Appearance > Personalize: preset and custom themes with a builder, a background
//! image, the ambient glow, art behind the active tab and the launch buttons, and corner
//! radius. Off by default: with nothing chosen the shell paints exactly the house look.
//!
//! The shell calls `apply`, `backdrop`, `sidebar_fill`, `tab_art` and `launch_art` from
//! `app.rs`; each returns the house look unchanged while its feature is off. Images are copied
//! into `<data>/personalize` on a worker (scaled to a bounded size) and decoded by Rune's image
//! cache, never on the UI thread. Data, files and share codes live in `ui::personalize`.

use std::path::PathBuf;
use std::sync::OnceLock;

use rune::{
    Color, Drag, El, FileDialogOptions, Key, KeyEvent, Phase, Pixels, SurfaceFit, TextEdit, Tx,
    col, el, icon, row, spacer, text,
};

use super::i18n::{fill, tr};
use super::{button, detail, section, segmented, setting_row, stacked_row, toggle_row};
use crate::app::State;
use crate::icons;
use crate::jobs::Job;
use crate::prefs;
use crate::ui::personalize::{
    self as model, ArtSource, BLUR_MAX, Density, Framing, GLOWS, Personal, Radius, Surface,
    ThemePick, ZOOM_MAX,
};
use crate::ui::{
    ActionKind, BODY, BODY_MED, LABEL, MONO, PRESETS, Palette, SIDEBAR_W, SMALL, TITLEBAR_H,
    TOKENS, Ui, hex6,
};

const SLIDER_W: f32 = 220.0;
/// The sidebar's nav rows and launch buttons are this wide (the sidebar less its 8 point
/// gutters).
const SIDE_ITEM_W: f32 = SIDEBAR_W - 16.0;

/// A theme being built or edited. While it is open the whole app paints with it.
pub struct Draft {
    /// The saved theme this edits; None makes a new one.
    pub editing: Option<String>,
    pub name: TextEdit,
    pub palette: Palette,
    /// The token the colour editor works on.
    pub token: usize,
    pub hex: TextEdit,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Target {
    Background,
    Art(Surface),
}

#[derive(Default)]
pub struct View {
    pub draft: Option<Draft>,
    /// A pasted share code or theme JSON.
    pub paste: TextEdit,
    pub surface: Surface,
    pub job: Job<(Target, String)>,
    pub glow_from: TextEdit,
    pub glow_to: TextEdit,
    /// Decoded images last poll (see `ready_count`).
    pub ready: usize,
}

// --- the shell ---------------------------------------------------------------------

/// Hand the palette and radius to `Ui::new`. The builder's draft wins while it is open.
pub fn apply(s: &State) {
    let personal = &s.prefs.personal;
    let palette = s
        .settings
        .personalize
        .draft
        .as_ref()
        .map(|d| d.palette)
        .or_else(|| personal.palette());
    crate::ui::set_personal(palette, personal.radius.scale());
    crate::ui::set_density(personal.density.scale());
}

/// Copied images. Headless runs (scenes, snapshots) keep theirs in a temporary folder.
fn images_dir() -> PathBuf {
    if rune::is_headless() {
        return std::env::temp_dir().join("sigil-headless").join(model::DIR);
    }
    model::images_dir(&crate::data::data_dir())
}

fn heroes() -> &'static [(String, PathBuf, Option<PathBuf>)] {
    static HEROES: OnceLock<Vec<(String, PathBuf, Option<PathBuf>)>> = OnceLock::new();
    HEROES.get_or_init(model::hero_renders)
}

fn shipped(surface: Surface) -> Option<&'static str> {
    static ART: OnceLock<[Option<String>; Surface::ALL.len()]> = OnceLock::new();
    let art = ART.get_or_init(|| {
        Surface::ALL.map(|s| {
            s.default_file()
                .and_then(model::shipped_art)
                .map(|p| p.display().to_string())
        })
    });
    let index = Surface::ALL.iter().position(|s| *s == surface).unwrap_or(0);
    art[index].as_deref()
}

/// The image file a source paints, as the image cache key. The tab's default is a glow,
/// not an image.
fn art_path(surface: Surface, source: &ArtSource) -> Option<String> {
    match source {
        ArtSource::Off => None,
        ArtSource::Default => shipped(surface).map(str::to_string),
        ArtSource::Hero(name) => heroes()
            .iter()
            .find(|(n, _, _)| n == name)
            .map(|(_, p, _)| p.display().to_string()),
        ArtSource::Custom(file) => Some(images_dir().join(file).display().to_string()),
    }
}

fn background_path(p: &Personal) -> Option<String> {
    p.background
        .file
        .as_ref()
        .map(|f| images_dir().join(f).display().to_string())
}

/// The images Personalize paints on every page (background, launch and tab art). Pages that
/// free their images on leave keep these.
pub fn in_use(p: &Personal) -> Vec<String> {
    let mut want = Vec::new();
    if let Some(path) = background_path(p) {
        want.push(path);
    }
    for surface in Surface::ALL {
        if let Some(path) = art_path(surface, &p.art.slot(surface).source) {
            want.push(path);
        }
    }
    want
}

/// Start decoding every image Personalize shows. The cache decodes on its own worker.
fn ensure_images(st: &mut State) {
    let want = in_use(&st.prefs.personal);
    for path in want {
        if st.thumbnails.contains_key(&path) || st.pending_thumbs.iter().any(|(k, _)| *k == path) {
            continue;
        }
        match st.images.as_ref() {
            Some(cache) => {
                let image = cache.file(&path, 1.0, 1.0);
                st.thumbnails.insert(path, image.pixels().clone());
            }
            // A headless scene runs before the cache exists; setup decodes these.
            None => {
                if let Ok(bytes) = std::fs::read(&path) {
                    st.pending_thumbs.push((path, bytes));
                }
            }
        }
    }
}

/// The image for a path once its decode has started. It may still be empty; Rune draws the
/// pixels when they land.
fn pixels<'a>(s: &'a State, path: Option<String>) -> Option<&'a Pixels> {
    s.thumbnails.get(&path?)
}

/// How many of the images Personalize shows have decoded. The view re-runs when it changes,
/// so framing that needs an image's size lands as soon as the decode does.
fn ready_count(st: &State) -> usize {
    let p = &st.prefs.personal;
    std::iter::once(background_path(p))
        .chain(Surface::ALL.map(|s| art_path(s, &p.art.slot(s).source)))
        .flatten()
        .filter(|path| st.thumbnails.get(path).is_some_and(|px| px.size().0 > 0))
        .count()
}

/// The background image and the glow, painted under the sidebar and the pages. None while
/// both are off.
/// The sidebar's width now: narrower when it is collapsed to icons (wave 4).
fn side_w(s: &State) -> f32 {
    crate::shell_w4::sidebar_width(s, 0.0)
}

pub fn backdrop(s: &State, u: &Ui, _window: (f32, f32)) -> Option<El> {
    let p = &s.prefs.personal;
    let image = if p.background.on {
        // A worker blurs the image once (crate::shell); the draw applies what is left.
        crate::shell::background_pixels(s, background_path(p), p.background.blur)
    } else {
        None
    };
    if image.is_none() && !p.glow.on {
        return None;
    }
    let mut layer = el()
        .key("personal-backdrop")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .paint_order(-1)
        .a11y_hidden();
    if let Some((px, blur)) = image {
        let left = if p.background.sidebar { 0.0 } else { side_w(s) };
        let top = if p.background.topbar { 0.0 } else { TITLEBAR_H };
        // The status bar keeps the plain window colour unless the image is asked under it.
        let bottom = if s.prefs.status_bar && !p.background.statusbar { crate::statusbar::BAR_H } else { 0.0 };
        let bleed = p.background.blur * 2.0;
        let picture = el()
            .key("personal-bg-image")
            .pin(Some(-bleed), Some(-bleed), Some(-bleed), Some(-bleed))
            .pixels_fit(px, SurfaceFit::Fill)
            .blur(blur)
            .opacity(p.background.opacity);
        layer = layer.child(
            el().key("personal-bg")
                .pin(Some(left), Some(top), Some(0.0), Some(bottom))
                .clip_rounded(0.0)
                .child(picture),
        );
    }
    if p.glow.on {
        // The glow starts where the page does unless the sidebar lets it through.
        let open_sidebar = s.prefs.transparent_sidebar
            || (p.background.sidebar && image.is_some());
        let left = if open_sidebar { 0.0 } else { side_w(s) };
        layer = layer.child(
            el().key("personal-glow")
                .pin(Some(left), Some(0.0), Some(0.0), Some(0.0))
                .child(glow_layer(&p.glow, u, false)),
        );
    }
    Some(layer)
}

/// Two soft radial glows from opposite corners (Grimoire's ambient background, MIT). Baked
/// once per setting into a small texture and stretched over the area: one image quad, no
/// per-frame gradient work (two live radial gradients cost about 5 ms a frame on Browse).
fn glow_layer(g: &model::Glow, u: &Ui, tile: bool) -> El {
    let from = model::parse_hex6(&g.from).unwrap_or(u.accent);
    let to = model::parse_hex6(&g.to).unwrap_or(u.accent);
    let (alpha, reach) = if tile {
        (0.75, 1.15)
    } else {
        (0.05 + g.strength * 0.45, 0.3 + g.spread * 0.6)
    };
    el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .pixels_fit(&glow_pixels(from, to, alpha, reach), SurfaceFit::Fill)
        .a11y_hidden()
}

const GLOW_W: u32 = 192;
const GLOW_H: u32 = 120;

/// The glow texture for two colours, a peak alpha and a reach (radius as a share of the
/// area). Kept for the last few settings so tiles and the live layer reuse theirs.
fn glow_pixels(from: Color, to: Color, alpha: f32, reach: f32) -> Pixels {
    type Key = [u32; 10];
    static CACHE: std::sync::Mutex<Vec<(Key, Pixels)>> = std::sync::Mutex::new(Vec::new());
    let key: Key = [
        from.r.to_bits(),
        from.g.to_bits(),
        from.b.to_bits(),
        to.r.to_bits(),
        to.g.to_bits(),
        to.b.to_bits(),
        alpha.to_bits(),
        reach.to_bits(),
        GLOW_W,
        GLOW_H,
    ];
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, px)) = cache.iter().find(|(k, _)| *k == key) {
        return px.clone();
    }
    let px = Pixels::new();
    px.set(GLOW_W, GLOW_H, bake_glow(from, to, alpha, reach));
    cache.push((key, px.clone()));
    if cache.len() > 16 {
        cache.remove(0);
    }
    px
}

/// Straight-alpha sRGB pixels: `from` fading out from the top left, `to` from the bottom
/// right, along the same eased stops the layer used as gradients.
fn bake_glow(from: Color, to: Color, alpha: f32, reach: f32) -> Vec<u8> {
    const STOPS: [(f32, f32); 4] = [(0.0, 1.0), (0.35, 0.55), (0.7, 0.12), (1.0, 0.0)];
    let fade = |d: f32| -> f32 {
        if d >= 1.0 {
            return 0.0;
        }
        for w in STOPS.windows(2) {
            let ((t0, a0), (t1, a1)) = (w[0], w[1]);
            if d <= t1 {
                return a0 + (a1 - a0) * (d - t0) / (t1 - t0);
            }
        }
        0.0
    };
    let reach = reach.max(0.01);
    let mut out = Vec::with_capacity((GLOW_W * GLOW_H * 4) as usize);
    for y in 0..GLOW_H {
        let v = (y as f32 + 0.5) / GLOW_H as f32;
        for x in 0..GLOW_W {
            let u = (x as f32 + 0.5) / GLOW_W as f32;
            let a1 = alpha * fade((u * u + v * v).sqrt() / reach);
            let (du, dv) = (1.0 - u, 1.0 - v);
            let a2 = alpha * fade((du * du + dv * dv).sqrt() / reach);
            let a = a1 + a2 * (1.0 - a1);
            let mix = |c1: f32, c2: f32| {
                if a <= 0.0 {
                    0.0
                } else {
                    (c1 * a1 + c2 * a2 * (1.0 - a1)) / a
                }
            };
            let enc = |c: f32| (rune::color::srgb_encode(c.clamp(0.0, 1.0)) * 255.0).round() as u8;
            out.extend_from_slice(&[
                enc(mix(from.r, to.r)),
                enc(mix(from.g, to.g)),
                enc(mix(from.b, to.b)),
                (a.clamp(0.0, 1.0) * 255.0).round() as u8,
            ]);
        }
    }
    out
}

/// The sidebar's fill. A background image that reaches under the sidebar shows through it.
pub fn sidebar_fill(s: &State, u: &Ui) -> Color {
    let p = &s.prefs.personal;
    let base = if s.prefs.transparent_sidebar { u.bg } else { u.sidebar };
    if p.background.on && p.background.sidebar && pixels(s, background_path(p)).is_some() {
        if s.prefs.transparent_sidebar {
            Color::TRANSPARENT
        } else {
            base.alpha(0.62)
        }
    } else {
        base
    }
}

/// An image framed into a box: covers it, the focus point kept in view.
fn framed(px: &Pixels, boxed: (f32, f32), framing: Framing) -> El {
    let (iw, ih) = px.size();
    if iw == 0 || ih == 0 {
        return el()
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .pixels_fit(px, SurfaceFit::Fill);
    }
    let (w, h, left, top) = model::frame((iw as f32, ih as f32), boxed, framing);
    el().absolute(left, top).size(w, h).pixels(px)
}

/// Art and its scrim under a sidebar item. The scrim keeps the label side readable.
fn under(item: El, px: &Pixels, boxed: (f32, f32), framing: Framing, scrim: [(f32, Color); 3]) -> El {
    item.clip_rounded(crate::ui::r(8.0))
        .child(
            el().key("art")
                .absolute(0.0, 0.0)
                .size(boxed.0, boxed.1)
                .paint_order(-2)
                .a11y_hidden()
                .child(framed(px, boxed, framing)),
        )
        .child(
            el().key("art-scrim")
                .absolute(0.0, 0.0)
                .size(boxed.0, boxed.1)
                .paint_order(-1)
                .linear_gradient(std::f32::consts::FRAC_PI_2, scrim)
                .a11y_hidden(),
        )
}

/// Art behind the active nav item. The default is a soft accent glow from the left.
pub fn tab_art(s: &State, u: &Ui, item: El, on: bool) -> El {
    if !on {
        return item;
    }
    let slot = &s.prefs.personal.art.tab;
    let boxed = (SIDE_ITEM_W, 34.0);
    match &slot.source {
        ArtSource::Off => item,
        ArtSource::Default => item.clip_rounded(crate::ui::r(8.0)).child(
            el().key("art-glow")
                .absolute(0.0, 0.0)
                .size(boxed.0, boxed.1)
                .paint_order(-1)
                .radial_gradient(
                    [0.0, 0.5],
                    [boxed.0 * 0.9, boxed.1 * 1.6],
                    [
                        (0.0, u.accent.alpha(0.30)),
                        (0.5, u.accent.alpha(0.10)),
                        (1.0, u.accent.alpha(0.0)),
                    ],
                )
                .a11y_hidden(),
        ),
        source => match pixels(s, art_path(Surface::Tab, source)) {
            Some(px) => {
                let base = sidebar_base(s, u);
                under(
                    item,
                    px,
                    boxed,
                    slot.framing,
                    [
                        (0.0, base.alpha(0.94)),
                        (0.5, base.alpha(0.55)),
                        (1.0, base.alpha(0.3)),
                    ],
                )
            }
            None => item,
        },
    }
}

fn sidebar_base(s: &State, u: &Ui) -> Color {
    if s.prefs.transparent_sidebar { u.bg } else { u.sidebar }
}

/// Art behind a launch button.
pub fn launch_art(s: &State, u: &Ui, button: El, vanilla: bool) -> El {
    let surface = if vanilla { Surface::Vanilla } else { Surface::Modded };
    let slot = s.prefs.personal.art.slot(surface);
    let Some(px) = pixels(s, art_path(surface, &slot.source)) else {
        return button;
    };
    let framing = if slot.source == ArtSource::Default {
        surface.default_framing()
    } else {
        slot.framing
    };
    let (h, tint) = if vanilla {
        (32.0, sidebar_base(s, u))
    } else {
        (36.0, u.accent)
    };
    let scrim = if vanilla {
        [(0.0, tint.alpha(0.92)), (0.5, tint.alpha(0.55)), (1.0, tint.alpha(0.1))]
    } else {
        [(0.0, tint.alpha(0.96)), (0.5, tint.alpha(0.7)), (1.0, tint.alpha(0.2))]
    };
    under(button, px, (SIDE_ITEM_W, h), framing, scrim)
}

/// The status bar's volume control is this wide while it carries art.
pub const VOLUME_ART_W: f32 = 76.0;

/// Art behind the status bar's volume control (Grimoire's volume bar art). The art sits in its
/// own clipped layer beside the control, so the volume popover anchored to it is never cut.
pub fn volume_art(s: &State, u: &Ui, chip: El) -> El {
    let slot = &s.prefs.personal.art.volume;
    let Some(px) = pixels(s, art_path(Surface::Volume, &slot.source)) else {
        return chip;
    };
    let framing = if slot.source == ArtSource::Default {
        Surface::Volume.default_framing()
    } else {
        slot.framing
    };
    let boxed = (VOLUME_ART_W, 20.0);
    let base = u.bg;
    el().key("statusbar-volume-art")
        .size(boxed.0, boxed.1)
        .child(
            el().absolute(0.0, 0.0)
                .size(boxed.0, boxed.1)
                .clip_rounded(crate::ui::r(5.0))
                .a11y_hidden()
                .child(framed(px, boxed, framing))
                .child(
                    el().absolute(0.0, 0.0)
                        .size(boxed.0, boxed.1)
                        .linear_gradient(
                            std::f32::consts::FRAC_PI_2,
                            [(0.0, base.alpha(0.88)), (0.55, base.alpha(0.45)), (1.0, base.alpha(0.15))],
                        )
                        .a11y_hidden(),
                ),
        )
        .child(chip.w(boxed.0))
}

// --- worker results ----------------------------------------------------------------

/// Called from Settings' poll: a finished image copy, and decodes for what is on.
pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.settings.personalize.job.try_take() {
        match result {
            Ok((target, name)) => {
                let old = match target {
                    Target::Background => background_path(&st.prefs.personal),
                    Target::Art(surface) => {
                        art_path(surface, &st.prefs.personal.art.slot(surface).source)
                    }
                };
                prefs::edit(st, |p| match target {
                    Target::Background => {
                        p.personal.background.file = Some(name);
                        p.personal.background.on = true;
                    }
                    Target::Art(surface) => {
                        let slot = p.personal.art.slot_mut(surface);
                        slot.source = ArtSource::Custom(name);
                        slot.framing = Framing::default();
                    }
                });
                if let Some(old) = old
                    && old.starts_with(&images_dir().display().to_string())
                {
                    st.thumbnails.remove(&old);
                }
                if !rune::is_headless() {
                    model::prune_images(&images_dir(), &st.prefs.personal.files());
                }
            }
            Err(error) => say(st, &error, true),
        }
        changed = true;
    }
    let before = st.thumbnails.len();
    ensure_images(st);
    let ready = ready_count(st);
    if ready != st.settings.personalize.ready {
        st.settings.personalize.ready = ready;
        changed = true;
    }
    changed | (st.thumbnails.len() != before)
}

fn say(st: &mut State, message: &str, error: bool) {
    st.settings.message = Some((tr(message).to_string(), error));
}

/// Change a Personalize setting, save, and start any decode it needs.
fn edit(st: &mut State, change: impl FnOnce(&mut Personal)) {
    prefs::edit(st, |p| change(&mut p.personal));
    ensure_images(st);
}

fn choose_image(st: &mut State, tx: &mut Tx, target: Target) {
    if st.settings.personalize.job.is_running() {
        return;
    }
    let options = FileDialogOptions {
        title: tr("Choose an image").into(),
        ..Default::default()
    }
    .filter(tr("Images"), &["png", "jpg", "jpeg", "webp"]);
    tx.open_file(options, move |st: &mut State, result, _tx| match result {
        Ok(Some(path)) => {
            let dir = images_dir();
            let (prefix, edge) = match target {
                Target::Background => ("background".to_string(), model::BACKGROUND_EDGE),
                Target::Art(surface) => (format!("art-{}", surface.id()), model::ART_EDGE),
            };
            let wake = st.wake.clone();
            st.settings.personalize.job.start(wake, move || {
                model::import_image(&path, &dir, &prefix, edge).map(|name| (target, name))
            });
            say(st, "Copying the image.", false);
        }
        Ok(None) => {}
        Err(error) => {
            let line = fill(tr("The file picker could not open: {}"), &[&error.to_string()]);
            st.settings.message = Some((line, true));
        }
    });
}

// --- themes ------------------------------------------------------------------------

fn current_name(s: &State) -> String {
    match &s.prefs.personal.theme {
        Some(ThemePick::Preset(id)) => crate::ui::preset(id)
            .map(|p| tr(p.name).to_string())
            .unwrap_or_default(),
        Some(ThemePick::Mine(id)) => s
            .prefs
            .personal
            .saved(id)
            .map(|t| t.name.clone())
            .unwrap_or_default(),
        None => tr(s.prefs.theme.label()).to_string(),
    }
}

fn open_draft(st: &mut State, editing: Option<String>, name: String, palette: Palette) {
    st.settings.personalize.draft = Some(Draft {
        editing,
        name: TextEdit::new(name),
        palette,
        token: 7,
        hex: TextEdit::new(hex6(palette.accent)),
    });
}

/// A new theme from what is on screen now.
fn new_theme(st: &mut State) {
    let palette = Palette::from_ui(&Ui::new());
    let name = st.prefs.personal.copy_name(&current_name(st));
    open_draft(st, None, name, palette);
}

fn save_draft(st: &mut State) {
    let Some(draft) = st.settings.personalize.draft.take() else {
        return;
    };
    let name = draft.name.text.clone();
    prefs::edit(st, |p| {
        let id = p.personal.save_theme(draft.editing.clone(), &name, draft.palette);
        p.personal.theme = Some(ThemePick::Mine(id));
    });
    say(st, "Theme saved.", false);
}

fn import_theme(st: &mut State, name: String, palette: Palette) {
    let name = st
        .prefs
        .personal
        .themes
        .iter()
        .any(|t| t.name == name)
        .then(|| st.prefs.personal.copy_name(&name))
        .unwrap_or(name);
    prefs::edit(st, |p| {
        let id = p.personal.save_theme(None, &name, palette);
        p.personal.theme = Some(ThemePick::Mine(id));
    });
    let line = fill(tr("Imported {}."), &[&name]);
    st.settings.message = Some((line, false));
}

fn import_pasted(st: &mut State) {
    match model::parse_theme_text(&st.settings.personalize.paste.text) {
        Ok((name, palette)) => {
            st.settings.personalize.paste = TextEdit::new("");
            import_theme(st, name, palette);
        }
        Err(error) => say(st, &error, true),
    }
}

fn import_file(_st: &mut State, tx: &mut Tx) {
    let options = FileDialogOptions {
        title: tr("Import a theme").into(),
        ..Default::default()
    }
    .filter(tr("Sigil theme"), &["json"]);
    tx.open_file(options, |st: &mut State, result, _tx| {
        let Ok(Some(path)) = result else {
            return;
        };
        let read = std::fs::metadata(&path)
            .map_err(|e| e.to_string())
            .and_then(|m| {
                if m.len() as usize > model::THEME_FILE_CAP {
                    Err(tr("That theme is too large. A theme file is under 4 KB.").to_string())
                } else {
                    std::fs::read_to_string(&path).map_err(|e| e.to_string())
                }
            })
            .and_then(|text| model::parse_theme_json(&text));
        match read {
            Ok((name, palette)) => import_theme(st, name, palette),
            Err(error) => say(st, &error, true),
        }
    });
}

fn export_file(st: &mut State, tx: &mut Tx, id: String) {
    let Some(theme) = st.prefs.personal.saved(&id).cloned() else {
        return;
    };
    let options = FileDialogOptions {
        title: tr("Export the theme").into(),
        file_name: Some(model::export_file_name(&theme.name)),
        ..Default::default()
    }
    .filter(tr("Sigil theme"), &["json"]);
    tx.save_file(options, move |st: &mut State, result, _tx| {
        let Ok(Some(path)) = result else {
            return;
        };
        match std::fs::write(&path, model::theme_json(&theme.name, &theme.palette)) {
            Ok(()) => {
                let line = fill(tr("Saved {}."), &[&path.display().to_string()]);
                st.settings.message = Some((line, false));
            }
            Err(error) => {
                let line = fill(tr("The theme could not be saved: {}"), &[&error.to_string()]);
                st.settings.message = Some((line, true));
            }
        }
    });
}

// --- the view ----------------------------------------------------------------------

/// The Personalize sections under Appearance: the theme builder while drafting, then the
/// background image, glow and sidebar art.
pub fn view(s: &State, u: &Ui) -> El {
    let mut c = col().key("settings-personalize").w_full().gap(super::LIST_GAP_PUB);
    if let Some(draft) = &s.settings.personalize.draft {
        c = c.child(builder(s, u, draft));
    }
    c.child(background_section(s, u))
        .child(glow_section(s, u))
        .child(art_section(s, u))
}

/// Density and corners, for Appearance's Interface section.
pub fn interface_rows(s: &State, u: &Ui) -> Vec<El> {
    vec![
                setting_row(
                    u,
                    "settings-radius",
                    "Corners",
                    detail(u, "Rounding on buttons, fields, panels and dialogs."),
                    segmented(
                        u,
                        "settings-radius-pick",
                        &Radius::ALL.map(|r| (r, r.label())),
                        s.prefs.personal.radius,
                        |st, radius| edit(st, |p| p.radius = radius),
                    ),
                ),
                setting_row(
                    u,
                    "settings-density",
                    "Density",
                    detail(u, "Sizes for text, rows and controls. The zoom shortcuts fine-tune it."),
                    segmented(
                        u,
                        "settings-density-pick",
                        &Density::ALL.map(|d| (d, d.label())),
                        s.prefs.personal.density,
                        |st, density| edit(st, |p| p.density = density),
                    ),
                ),
    ]
}

/// A small picture of a palette: its sidebar, a page with lines, and the accent.
fn palette_picture(p: &Palette, h: f32) -> El {
    let t = p.to_ui();
    row()
        .w_full()
        .h(h)
        .padding(1.0)
        .radius(crate::ui::r(8.0))
        .fill(t.bg)
        .border(1.0, t.hair_strong)
        .child(
            col()
                .w(22.0)
                .h_full()
                .corners([7.0, 0.0, 0.0, 7.0])
                .fill(t.sidebar)
                .pt(8.0)
                .px(5.0)
                .gap(4.0)
                .child(el().w(12.0).h(3.0).radius(crate::ui::r(1.5)).fill(t.ink.alpha(0.6)))
                .child(el().w(10.0).h(3.0).radius(crate::ui::r(1.5)).fill(t.muted.alpha(0.5))),
        )
        .child(
            col()
                .grow(1.0)
                .h_full()
                .pt(8.0)
                .px(7.0)
                .gap(4.0)
                .child(el().w(28.0).h(4.0).radius(crate::ui::r(2.0)).fill(t.ink.alpha(0.85)))
                .child(el().w_full().h(3.0).radius(crate::ui::r(1.5)).fill(t.muted.alpha(0.45)))
                .child(spacer())
                .child(
                    row()
                        .w_full()
                        .pb(7.0)
                        .child(spacer())
                        .child(el().w(18.0).h(8.0).radius(crate::ui::r(3.0)).fill(t.accent)),
                ),
        )
}

fn preset_tile(s: &State, u: &Ui, index: usize) -> El {
    let preset = &PRESETS[index];
    let on = s.prefs.personal.theme == Some(ThemePick::Preset(preset.id));
    let id = preset.id;
    col()
        .key(format!("settings-preset-{id}"))
        .w(92.0)
        .gap(6.0)
        .focusable(true)
        .role("radio")
        .checked(on)
        .label(tr(preset.name).to_string())
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            st.settings.personalize.draft = None;
            edit(st, |p| p.theme = Some(ThemePick::Preset(id)))
        })
        .child(
            col()
                .w_full()
                .padding(2.0)
                .radius(crate::ui::r(8.0))
                .border(1.0, if on { u.ink } else { Color::TRANSPARENT })
                .child(palette_picture(&preset.palette(), 64.0)),
        )
        .child(
            text(tr(preset.name).to_string())
                .font(SMALL)
                .color(if on { u.ink } else { u.muted })
                .a11y_hidden(),
        )
}

/// The rows of Appearance's one Theme section after the built-in tiles: presets, your
/// themes and import.
pub fn theme_rows(s: &State, u: &Ui) -> Vec<El> {
    let p = &s.prefs.personal;
    let mut tiles = row()
        .key("settings-presets")
        .w_full()
        .flex_wrap()
        .gap(12.0);
    for index in 0..PRESETS.len() {
        tiles = tiles.child(preset_tile(s, u, index));
    }
    let mut actions = row()
        .gap(8.0)
        .center_y()
        .child(
            button(u, "settings-theme-new", "New theme", ActionKind::Secondary)
                .on_press(new_theme),
        );
    if p.theme.is_some() {
        actions = actions.child(
            button(u, "settings-theme-stop", "Use the theme above", ActionKind::Ghost)
                .on_press(|st: &mut State| {
                    st.settings.personalize.draft = None;
                    edit(st, |p| p.theme = None)
                }),
        );
    }
    let mut rows = vec![stacked_row(
        u,
        "settings-presets-row",
        "Presets",
        "Picking one replaces the theme above until you choose a theme there.",
        col().w_full().gap(16.0).child(tiles).child(actions),
    )];

    if !p.themes.is_empty() {
        let mut list = col().key("settings-my-themes").w_full();
        for theme in &p.themes {
            list = list.child(saved_row(s, u, theme));
        }
        rows.push(stacked_row(u, "settings-my-themes-row", "Your themes", "", list));
    }

    let paste = rune::text_input(&s.settings.personalize.paste, MONO)
        .key("settings-theme-paste-input")
        .label(tr("Theme code or JSON"))
        .single_line()
        .grow(1.0)
        .color(u.ink)
        .placeholder("sigil-theme:...", u.faint)
        .caret(u.accent, u.accent.alpha(0.22))
        .on_edit(|st: &mut State| &mut st.settings.personalize.paste);
    let empty = s.settings.personalize.paste.text.trim().is_empty();
    rows.push(stacked_row(
        u,
        "settings-theme-import",
        "Import a theme",
        "Paste a theme code someone shared, or open a theme file.",
        row()
            .w_full()
            .gap(8.0)
            .center_y()
            .child(
                row()
                    .grow(1.0)
                    .min_w(0.0)
                    .h(32.0)
                    .px(10.0)
                    .center_y()
                    .radius(crate::ui::r(8.0))
                    .fill(u.well)
                    .border(1.0, u.hair)
                    .child(paste),
            )
            .child(super::disabled(
                button(u, "settings-theme-import-go", "Import", ActionKind::Secondary)
                    .on_press(import_pasted),
                empty,
            ))
            .child(
                button(u, "settings-theme-import-file", "Open file", ActionKind::Ghost)
                    .on_press_tx(|st: &mut State, tx: &mut Tx| import_file(st, tx)),
            ),
    ));
    rows
}

fn saved_row(s: &State, u: &Ui, theme: &model::SavedTheme) -> El {
    let on = s.prefs.personal.theme == Some(ThemePick::Mine(theme.id.clone()));
    let mut dots = row().gap(4.0).center_y();
    for i in [0usize, 1, 2, 5, 7] {
        dots = dots.child(
            el().size(12.0, 12.0)
                .radius(crate::ui::r(6.0))
                .fill(theme.palette.get(i))
                .border(1.0, u.hair_strong)
                .a11y_hidden(),
        );
    }
    let id = theme.id.clone();
    let (edit_id, dup_id, copy_id, export_id, delete_id) =
        (id.clone(), id.clone(), id.clone(), id.clone(), id.clone());
    let icon_btn = |glyph, label: &str, key: &str| {
        u.icon_button(glyph, tr(label))
            .key(format!("settings-my-theme-{key}-{}", theme.id))
            .cursor(rune::Cursor::Pointer)
    };
    row()
        .key(format!("settings-my-theme-{}", theme.id))
        .w_full()
        .h(48.0)
        .gap(12.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(dots)
        .child(
            text(theme.name.clone())
                .font(BODY_MED)
                .color(u.ink)
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        )
        .child(if on {
            text(tr("In use").to_string()).font(SMALL).color(u.muted)
        } else {
            button(u, &format!("settings-my-theme-use-{}", theme.id), "Use", ActionKind::Secondary)
                .on_press(move |st: &mut State| {
                    let id = id.clone();
                    st.settings.personalize.draft = None;
                    edit(st, |p| p.theme = Some(ThemePick::Mine(id)))
                })
        })
        .child(
            icon_btn(&icons::SQUARE_PEN, "Edit", "edit").on_press(move |st: &mut State| {
                if let Some(t) = st.prefs.personal.saved(&edit_id).cloned() {
                    open_draft(st, Some(t.id), t.name, t.palette);
                }
            }),
        )
        .child(
            icon_btn(&icons::LAYERS, "Duplicate", "dup").on_press(move |st: &mut State| {
                if let Some(t) = st.prefs.personal.saved(&dup_id).cloned() {
                    let name = st.prefs.personal.copy_name(&t.name);
                    prefs::edit(st, |p| {
                        p.personal.save_theme(None, &name, t.palette);
                    });
                }
            }),
        )
        .child(
            icon_btn(&icons::COPY, "Copy theme code", "copy").on_press_tx(
                move |st: &mut State, tx: &mut Tx| {
                    if let Some(t) = st.prefs.personal.saved(&copy_id) {
                        tx.copy(model::share_code(&t.name, &t.palette));
                        say(st, "Theme code copied.", false);
                    }
                },
            ),
        )
        .child(
            icon_btn(&icons::DOWNLOAD, "Export file", "export").on_press_tx(
                move |st: &mut State, tx: &mut Tx| export_file(st, tx, export_id.clone()),
            ),
        )
        .child(
            icon_btn(&icons::TRASH_2, "Delete", "delete").on_press(move |st: &mut State| {
                let id = delete_id.clone();
                if st
                    .settings
                    .personalize
                    .draft
                    .as_ref()
                    .is_some_and(|d| d.editing.as_deref() == Some(id.as_str()))
                {
                    st.settings.personalize.draft = None;
                }
                prefs::edit(st, |p| p.personal.delete_theme(&id));
            }),
        )
}

// --- the builder -------------------------------------------------------------------

fn draft_mut(st: &mut State) -> Option<&mut Draft> {
    st.settings.personalize.draft.as_mut()
}

fn builder(s: &State, u: &Ui, draft: &Draft) -> El {
    let warnings = draft.palette.contrast_warnings();
    let name = rune::text_input(&draft.name, BODY)
        .key("settings-builder-name-input")
        .label(tr("Theme name"))
        .single_line()
        .grow(1.0)
        .color(u.ink)
        .placeholder(tr("Theme name"), u.faint)
        .caret(u.accent, u.accent.alpha(0.22))
        .on_edit(|st: &mut State| &mut st.settings.personalize.draft.as_mut().expect("draft").name);
    let head = row()
        .w_full()
        .gap(16.0)
        .items_start()
        .pt(16.0)
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(10.0)
                .child(
                    row()
                        .w_full()
                        .h(32.0)
                        .px(10.0)
                        .center_y()
                        .radius(crate::ui::r(8.0))
                        .fill(u.well)
                        .border(1.0, u.hair)
                        .child(name),
                )
                .child(
                    text(tr("Sigil paints with this theme while you edit it.").to_string())
                        .font(SMALL)
                        .color(u.muted)
                        .wrap(),
                ),
        )
        .child(col().w(140.0).child(palette_picture(&draft.palette, 72.0)));

    let mut tokens = col().key("settings-builder-tokens").w_full();
    for (index, token) in TOKENS.iter().enumerate() {
        tokens = tokens.child(token_row(u, draft, index, token, &warnings));
    }

    let mut c = col()
        .w_full()
        .gap(16.0)
        .child(head)
        .child(color_editor(u, draft))
        .child(tokens);
    if !warnings.is_empty() {
        let mut list = col().key("settings-builder-contrast").w_full().gap(4.0);
        for (fg, bg, ratio) in &warnings {
            list = list.child(
                row()
                    .gap(8.0)
                    .center_y()
                    .child(icon(&icons::TRIANGLE_ALERT, 14.0).color(u.amber).a11y_hidden())
                    .child(
                        text(fill(
                            tr("{} on {} is {} to 1, under the 4.5 that body text needs."),
                            &[
                                tr(TOKENS[*fg].label),
                                &tr(TOKENS[*bg].label).to_lowercase(),
                                &format!("{ratio:.1}"),
                            ],
                        ))
                        .font(SMALL)
                        .color(u.amber)
                        .wrap(),
                    ),
            );
        }
        c = c.child(list);
    }
    let title = if draft.editing.is_some() {
        "Edit theme"
    } else {
        "New theme"
    };
    c = c.child(
        row()
            .w_full()
            .gap(8.0)
            .center_y()
            .child(spacer())
            .child(
                button(u, "settings-builder-cancel", "Cancel", ActionKind::Ghost)
                    .on_press(|st: &mut State| st.settings.personalize.draft = None),
            )
            .child(
                button(u, "settings-builder-save", "Save theme", ActionKind::Primary)
                    .on_press(save_draft),
            ),
    );
    let _ = s;
    section(
        u,
        title,
        vec![col().key("settings-builder").w_full().pb(16.0).child(c)],
    )
}

fn token_row(
    u: &Ui,
    draft: &Draft,
    index: usize,
    token: &crate::ui::Token,
    warnings: &[(usize, usize, f32)],
) -> El {
    let on = draft.token == index;
    let color = draft.palette.get(index);
    let warn = warnings.iter().any(|(fg, bg, _)| *fg == index || *bg == index);
    let mut r = row()
        .key(format!("settings-token-{}", token.id))
        .w_full()
        .h(40.0)
        .px(8.0)
        .gap(12.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .fill(if on { u.selected } else { Color::TRANSPARENT })
        .hover(if on { u.selected } else { u.hover })
        .focusable(true)
        .role("radio")
        .checked(on)
        .label(tr(token.label).to_string())
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            if let Some(d) = draft_mut(st) {
                d.token = index;
                d.hex = TextEdit::new(hex6(d.palette.get(index)));
            }
        })
        .child(
            el().size(20.0, 20.0)
                .radius(crate::ui::r(6.0))
                .fill(color)
                .border(1.0, u.hair_strong)
                .a11y_hidden(),
        )
        .child(
            text(tr(token.label).to_string())
                .font(BODY_MED)
                .color(u.ink)
                .w(112.0)
                .a11y_hidden(),
        )
        .child(
            text(tr(token.line).to_string())
                .font(SMALL)
                .color(u.muted)
                .grow(1.0)
                .min_w(0.0)
                .truncate()
                .a11y_hidden(),
        );
    if warn {
        r = r.child(icon(&icons::TRIANGLE_ALERT, 14.0).color(u.amber).a11y_hidden());
    }
    r.child(
        text(hex6(color))
            .font(MONO)
            .color(u.muted)
            .w(64.0)
            .a11y_hidden(),
    )
}

/// The colour editor for the chosen token: a hex field and lightness, chroma and hue.
fn color_editor(u: &Ui, draft: &Draft) -> El {
    let color = draft.palette.get(draft.token);
    let [l, a, b, _] = color.to_oklab();
    let chroma = (a * a + b * b).sqrt();
    let hue = b.atan2(a).to_degrees().rem_euclid(360.0);
    let hex = rune::text_input(&draft.hex, MONO)
        .key("settings-builder-hex-input")
        .label(tr("Colour, hex"))
        .single_line()
        .grow(1.0)
        .color(u.ink)
        .caret(u.accent, u.accent.alpha(0.22))
        .on_edit(|st: &mut State| &mut st.settings.personalize.draft.as_mut().expect("draft").hex)
        .on_key(|st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Enter {
                use_hex(st);
                true
            } else {
                false
            }
        });
    let token = &TOKENS[draft.token];
    let set_lch = |which: usize| -> fn(&mut State, f32) {
        match which {
            0 => |st, v| set_oklch(st, Some(v), None, None),
            1 => |st, v| set_oklch(st, None, Some(v * 0.37), None),
            _ => |st, v| set_oklch(st, None, None, Some(v * 360.0)),
        }
    };
    col()
        .key("settings-builder-editor")
        .w_full()
        .gap(8.0)
        .padding(16.0)
        .radius(crate::ui::r(10.0))
        .border(1.0, u.hair)
        .child(
            row()
                .w_full()
                .gap(12.0)
                .center_y()
                .child(
                    el().size(32.0, 32.0)
                        .radius(crate::ui::r(8.0))
                        .fill(color)
                        .border(1.0, u.hair_strong)
                        .a11y_hidden(),
                )
                .child(
                    text(tr(token.label).to_string())
                        .font(BODY_MED)
                        .color(u.ink)
                        .grow(1.0),
                )
                .child(
                    row()
                        .w(120.0)
                        .h(32.0)
                        .px(10.0)
                        .center_y()
                        .radius(crate::ui::r(8.0))
                        .fill(u.well)
                        .border(1.0, u.hair)
                        .child(hex),
                )
                .child(
                    button(u, "settings-builder-hex-use", "Use", ActionKind::Secondary)
                        .on_press(use_hex),
                ),
        )
        .child(slider_row(u, "builder-l", "Lightness", l.clamp(0.0, 1.0), format!("{:.0}%", l * 100.0), set_lch(0), false))
        .child(slider_row(u, "builder-c", "Colourfulness", (chroma / 0.37).clamp(0.0, 1.0), format!("{chroma:.2}"), set_lch(1), false))
        .child(slider_row(u, "builder-h", "Hue", hue / 360.0, format!("{hue:.0}"), set_lch(2), false))
}

fn use_hex(st: &mut State) {
    let Some(d) = draft_mut(st) else {
        return;
    };
    match model::parse_hex6(&d.hex.text) {
        Some(c) => {
            let index = d.token;
            d.palette.set(index, c);
            d.hex = TextEdit::new(hex6(c));
        }
        None => say(st, "Type a colour as #rgb or #rrggbb.", true),
    }
}

fn set_oklch(st: &mut State, l: Option<f32>, c: Option<f32>, h: Option<f32>) {
    let Some(d) = draft_mut(st) else {
        return;
    };
    let color = d.palette.get(d.token);
    let [l0, a0, b0, _] = color.to_oklab();
    let c0 = (a0 * a0 + b0 * b0).sqrt();
    let h0 = b0.atan2(a0).to_degrees();
    let next = rune::oklch(l.unwrap_or(l0), c.unwrap_or(c0), h.unwrap_or(h0));
    let index = d.token;
    d.palette.set(index, next);
    d.hex = TextEdit::new(hex6(next));
}

// --- sliders -----------------------------------------------------------------------

/// A setting row with a flat slider and its value. `set` takes 0 to 1. With `save` the
/// value is written to settings.json when the drag ends; a draft only changes in memory.
fn slider_row(
    u: &Ui,
    key: &str,
    label: &str,
    frac: f32,
    shown: String,
    set: fn(&mut State, f32),
    save: bool,
) -> El {
    row()
        .key(format!("settings-{key}"))
        .w_full()
        .h(40.0)
        .gap(16.0)
        .center_y()
        .child(
            text(tr(label).to_string())
                .font(BODY)
                .color(u.prose)
                .grow(1.0)
                .min_w(0.0),
        )
        .child(slider(u, key, label, frac.clamp(0.0, 1.0), set, save))
        .child(
            text(shown)
                .font(MONO)
                .color(u.muted)
                .w(48.0)
                .align(rune::Align::End),
        )
}

fn slider(u: &Ui, key: &str, label: &str, frac: f32, set: fn(&mut State, f32), save: bool) -> El {
    let knob = 14.0;
    let track_y = 10.0;
    el().key(format!("slider-{key}"))
        .w(SLIDER_W)
        .h(24.0)
        .focusable(true)
        .role("slider")
        .label(tr(label).to_string())
        .cursor(rune::Cursor::Grab)
        .child(
            el().absolute(0.0, track_y)
                .w(SLIDER_W)
                .h(4.0)
                .radius(crate::ui::r(2.0))
                .fill(u.over.alpha(0.10))
                .a11y_hidden(),
        )
        .child(
            el().absolute(0.0, track_y)
                .w((SLIDER_W * frac).max(0.0))
                .h(4.0)
                .radius(crate::ui::r(2.0))
                .fill(u.muted)
                .a11y_hidden(),
        )
        .child(
            el().absolute((SLIDER_W - knob) * frac, (24.0 - knob) / 2.0)
                .size(knob, knob)
                .radius(knob / 2.0)
                .fill(Color::WHITE)
                .shadow(1.0, 3.0, Color::BLACK.alpha(0.4))
                .a11y_hidden(),
        )
        .on_drag(move |st: &mut State, d: &mut Drag, _tx: &mut Tx| {
            if d.phase == Phase::Up && d.cancelled() {
                return;
            }
            let width = if d.w > 0.0 { d.w } else { SLIDER_W };
            set(st, (d.x / width).clamp(0.0, 1.0));
            if save && d.phase == Phase::Up {
                prefs::edit(st, |_| {});
            }
        })
        .on_key(move |st: &mut State, ev: &KeyEvent, _| {
            let step = match ev.key {
                Key::Left | Key::Down => -0.05,
                Key::Right | Key::Up => 0.05,
                _ => return false,
            };
            set(st, (frac + step).clamp(0.0, 1.0));
            if save {
                prefs::edit(st, |_| {});
            }
            true
        })
}

// --- background --------------------------------------------------------------------

/// A little window: sidebar, top bar and page, with the image where it reaches.
fn coverage_picture(s: &State, u: &Ui) -> El {
    let p = &s.prefs.personal.background;
    let image = pixels(s, background_path(&s.prefs.personal));
    let (w, h) = (176.0, 110.0);
    let side = 44.0;
    let bar = 12.0;
    let mut pic = el()
        .key("settings-bg-preview")
        .size(w, h)
        .radius(crate::ui::r(8.0))
        .clip_rounded(crate::ui::r(8.0))
        .fill(u.bg)
        .border(1.0, u.hair_strong);
    let foot = 8.0;
    if let Some(px) = image {
        let left = if p.sidebar { 0.0 } else { side };
        let top = if p.topbar { 0.0 } else { bar };
        let bottom = if p.statusbar { 0.0 } else { foot };
        pic = pic.child(
            el().absolute(left, top)
                .size(w - left, h - top - bottom)
                .clip_rounded(0.0)
                .child(
                    el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                        .pixels_fit(px, SurfaceFit::Fill)
                        .blur(p.blur * 0.25)
                        .opacity(if p.on { p.opacity.max(0.15) } else { 0.15 }),
                ),
        );
    }
    pic.child(
        el().absolute(0.0, 0.0)
            .size(side, h)
            .fill(if p.sidebar && image.is_some() {
                u.sidebar.alpha(0.62)
            } else {
                u.sidebar
            })
            .border_sides([0.0, 1.0, 0.0, 0.0], u.hair)
            .a11y_hidden(),
    )
    .child(
        el().absolute(side, 0.0)
            .size(w - side, bar)
            .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
            .a11y_hidden(),
    )
    .child(
        el().absolute(side, h - foot)
            .size(w - side, foot)
            .border_sides([1.0, 0.0, 0.0, 0.0], u.hair)
            .a11y_hidden(),
    )
    .child(
        el().absolute(side + 10.0, bar + 10.0)
            .size(48.0, 5.0)
            .radius(crate::ui::r(2.5))
            .fill(u.ink.alpha(0.7))
            .a11y_hidden(),
    )
    .a11y_hidden()
}

fn background_section(s: &State, u: &Ui) -> El {
    let p = &s.prefs.personal.background;
    let busy = s.settings.personalize.job.is_running();
    let chosen = p.file.is_some();
    let mut actions = row().gap(8.0).center_y().child(super::disabled(
        button(
            u,
            "settings-bg-choose",
            if chosen { "Change image" } else { "Choose image" },
            ActionKind::Secondary,
        )
        .on_press_tx(|st: &mut State, tx: &mut Tx| choose_image(st, tx, Target::Background)),
        busy,
    ));
    if chosen {
        actions = actions.child(
            button(u, "settings-bg-remove", "Remove", ActionKind::Ghost).on_press(
                |st: &mut State| {
                    if let Some(old) = background_path(&st.prefs.personal) {
                        st.thumbnails.remove(&old);
                    }
                    edit(st, |p| {
                        p.background.file = None;
                        p.background.on = false;
                    });
                    if !rune::is_headless() {
                        model::prune_images(&images_dir(), &st.prefs.personal.files());
                    }
                },
            ),
        );
    }
    let top = row()
        .w_full()
        .gap(16.0)
        .items_start()
        .child(coverage_picture(s, u))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(10.0)
                .child(
                    text(tr("A picture of your own behind the pages. Sigil keeps a scaled copy, so the original can move.").to_string())
                        .font(SMALL)
                        .color(u.muted)
                        .wrap(),
                )
                .child(actions),
        );
    let mut rows = vec![col().key("settings-bg-top").w_full().pt(16.0).pb(16.0).border_sides([0.0, 0.0, 1.0, 0.0], u.hair).child(top)];
    if chosen {
        rows.push(toggle_row(
            u,
            "settings-bg-on",
            "Show the image",
            "Turn it off to keep the image for later.",
            p.on,
            |st| edit(st, |p| p.background.on = !p.background.on),
        ));
        rows.push(
            col()
                .w_full()
                .py(8.0)
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(slider_row(
                    u,
                    "bg-opacity",
                    "Opacity",
                    p.opacity,
                    format!("{:.0}%", p.opacity * 100.0),
                    |st, v| st.prefs.personal.background.opacity = (v * 100.0).round() / 100.0,
                    true,
                ))
                .child(slider_row(
                    u,
                    "bg-blur",
                    "Blur",
                    p.blur / BLUR_MAX,
                    format!("{:.0}", p.blur),
                    |st, v| st.prefs.personal.background.blur = (v * BLUR_MAX).round(),
                    true,
                )),
        );
        rows.push(toggle_row(
            u,
            "settings-bg-sidebar",
            "Behind the sidebar",
            "The sidebar turns see-through over the image.",
            p.sidebar,
            |st| edit(st, |p| p.background.sidebar = !p.background.sidebar),
        ));
        rows.push(toggle_row(
            u,
            "settings-bg-topbar",
            "Behind the top bar",
            "The image reaches the top edge of the window.",
            p.topbar,
            |st| edit(st, |p| p.background.topbar = !p.background.topbar),
        ));
        rows.push(super::smalls::statusbar_row(s, u, |st| edit(st, |p| p.background.statusbar = !p.background.statusbar)));
    }
    section(u, "Background image", rows)
}

// --- glow --------------------------------------------------------------------------

fn glow_tile(u: &Ui, key: String, label: &str, from: &str, to: &str, on: bool) -> El {
    let g = model::Glow {
        from: from.to_string(),
        to: to.to_string(),
        ..Default::default()
    };
    let (f, t) = (from.to_string(), to.to_string());
    el().key(key)
        .size(36.0, 36.0)
        .radius(crate::ui::r(8.0))
        .clip_rounded(crate::ui::r(8.0))
        .fill(u.bg)
        .border(if on { 2.0 } else { 1.0 }, if on { u.accent } else { u.hair_strong })
        .focusable(true)
        .role("radio")
        .checked(on)
        .label(tr(label).to_string())
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            let (f, t) = (f.clone(), t.clone());
            st.settings.personalize.glow_from = TextEdit::new(f.clone());
            st.settings.personalize.glow_to = TextEdit::new(t.clone());
            edit(st, |p| {
                p.glow.from = f;
                p.glow.to = t;
                p.glow.on = true;
            })
        })
        .child(glow_layer(&g, u, true))
}

fn hex_field(u: &Ui, edit_of: fn(&mut State) -> &mut TextEdit, value: &TextEdit, key: &str, label: &str, placeholder: &str) -> El {
    let typed = model::parse_hex6(&value.text);
    row()
        .w(120.0)
        .h(32.0)
        .px(10.0)
        .gap(8.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .fill(u.well)
        .border(1.0, u.hair)
        .child(
            el().size(14.0, 14.0)
                .radius(7.0)
                .fill(typed.unwrap_or(Color::TRANSPARENT))
                .border(1.0, u.hair_strong)
                .a11y_hidden(),
        )
        .child(
            rune::text_input(value, MONO)
                .key(key.to_string())
                .label(tr(label).to_string())
                .single_line()
                .grow(1.0)
                .color(u.ink)
                .placeholder(placeholder.to_string(), u.faint)
                .caret(u.accent, u.accent.alpha(0.22))
                .on_edit(edit_of),
        )
}

fn glow_section(s: &State, u: &Ui) -> El {
    let g = &s.prefs.personal.glow;
    let mut tiles = row().key("settings-glows").w_full().gap(10.0).center_y().flex_wrap();
    for (id, label, from, to) in GLOWS {
        let on = g.on && g.from == from && g.to == to;
        tiles = tiles.child(glow_tile(u, format!("settings-glow-{id}"), label, from, to, on));
    }
    for (index, (from, to)) in g.mine.iter().enumerate() {
        let on = g.on && g.from == *from && g.to == *to;
        let gone = (from.clone(), to.clone());
        tiles = tiles.child(
            row()
                .gap(2.0)
                .center_y()
                .child(glow_tile(u, format!("settings-glow-mine-{index}"), "Your glow", from, to, on))
                .child(
                    u.icon_button(&icons::X, tr("Remove this glow"))
                        .key(format!("settings-glow-remove-{index}"))
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| {
                            let gone = gone.clone();
                            edit(st, |p| p.glow.mine.retain(|pair| *pair != gone))
                        }),
                ),
        );
    }
    let view = &s.settings.personalize;
    let (from_text, to_text) = (&view.glow_from, &view.glow_to);
    let custom = row()
        .w_full()
        .gap(8.0)
        .center_y()
        .child(hex_field(u, |st| &mut st.settings.personalize.glow_from, from_text, "settings-glow-from-input", "Top left colour", "#8b5cf6"))
        .child(hex_field(u, |st| &mut st.settings.personalize.glow_to, to_text, "settings-glow-to-input", "Bottom right colour", "#06b6d4"))
        .child(
            button(u, "settings-glow-use", "Use", ActionKind::Secondary).on_press(|st: &mut State| {
                let v = &st.settings.personalize;
                let from = if v.glow_from.text.is_empty() { st.prefs.personal.glow.from.clone() } else { v.glow_from.text.trim().to_string() };
                let to = if v.glow_to.text.is_empty() { st.prefs.personal.glow.to.clone() } else { v.glow_to.text.trim().to_string() };
                match (model::parse_hex6(&from), model::parse_hex6(&to)) {
                    (Some(a), Some(z)) => {
                        let (a, z) = (hex6(a), hex6(z));
                        edit(st, |p| {
                            p.remember_glow(&a, &z);
                            p.glow.from = a;
                            p.glow.to = z;
                            p.glow.on = true;
                        })
                    }
                    _ => say(st, "Type a colour as #rgb or #rrggbb.", true),
                }
            }),
        );
    let mut rows = vec![toggle_row(
        u,
        "settings-glow-on",
        "Background glow",
        "Two soft colours that bleed in from the corners of the window.",
        g.on,
        |st| edit(st, |p| p.glow.on = !p.glow.on),
    )];
    rows.push(stacked_row(
        u,
        "settings-glow-colours",
        "Glow colours",
        "Pick a pair, or type two colours to make your own.",
        col().w_full().gap(12.0).child(tiles).child(custom),
    ));
    rows.push(
        col()
            .w_full()
            .py(8.0)
            .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
            .child(slider_row(
                u,
                "glow-strength",
                "Strength",
                g.strength,
                format!("{:.0}%", g.strength * 100.0),
                |st, v| st.prefs.personal.glow.strength = (v * 100.0).round() / 100.0,
                true,
            ))
            .child(slider_row(
                u,
                "glow-spread",
                "Spread",
                g.spread,
                format!("{:.0}%", g.spread * 100.0),
                |st, v| st.prefs.personal.glow.spread = (v * 100.0).round() / 100.0,
                true,
            )),
    );
    section(u, "Glow", rows)
}

// --- art ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Off,
    Default,
    Hero,
    Custom,
}

fn kind_of(source: &ArtSource) -> Kind {
    match source {
        ArtSource::Off => Kind::Off,
        ArtSource::Default => Kind::Default,
        ArtSource::Hero(_) => Kind::Hero,
        ArtSource::Custom(_) => Kind::Custom,
    }
}

fn set_kind(st: &mut State, kind: Kind) {
    let surface = st.settings.personalize.surface;
    let first_hero = heroes().first().map(|(n, _, _)| n.clone());
    edit(st, |p| {
        let slot = p.art.slot_mut(surface);
        slot.source = match kind {
            Kind::Off => ArtSource::Off,
            Kind::Default => ArtSource::Default,
            Kind::Hero => match &slot.source {
                ArtSource::Hero(name) => ArtSource::Hero(name.clone()),
                _ => first_hero.map(ArtSource::Hero).unwrap_or(ArtSource::Off),
            },
            // The image comes from the picker; until then the slot keeps what it had.
            Kind::Custom => slot.source.clone(),
        };
    });
}

/// The real surface at its real size, with the art as it will paint.
fn surface_preview(s: &State, u: &Ui, surface: Surface) -> El {
    let item = match surface {
        Surface::Tab => row()
            .w(SIDE_ITEM_W)
            .h(34.0)
            .px(10.0)
            .gap(10.0)
            .center_y()
            .radius(crate::ui::r(8.0))
            .fill(u.selected)
            .child(icon(&icons::COMPASS, 16.0).color(u.ink).a11y_hidden())
            .child(text(tr("Browse").to_string()).font(crate::ui::NAV).color(u.ink)),
        Surface::Modded => row()
            .w(SIDE_ITEM_W)
            .h(36.0)
            .px(12.0)
            .gap(10.0)
            .center_y()
            .radius(crate::ui::r(8.0))
            .fill(u.accent)
            .child(icon(&icons::PLAY, 14.0).color(u.accent_ink).a11y_hidden())
            .child(text(tr("Launch modded").to_string()).font(BODY_MED).color(u.accent_ink)),
        Surface::Vanilla => row()
            .w(SIDE_ITEM_W)
            .h(32.0)
            .px(12.0)
            .gap(10.0)
            .center_y()
            .radius(crate::ui::r(8.0))
            .child(icon(&icons::PLAY, 14.0).color(u.faint).a11y_hidden())
            .child(text(tr("Launch vanilla").to_string()).font(BODY_MED).color(u.prose)),
        Surface::Volume => super::smalls::volume_preview(s, u),
    };
    let item = match surface {
        Surface::Tab => tab_art(s, u, item, true),
        Surface::Modded => launch_art(s, u, item, false),
        Surface::Vanilla => launch_art(s, u, item, true),
        Surface::Volume => volume_art(s, u, item),
    };
    col()
        .key("settings-art-preview")
        .padding(8.0)
        .radius(crate::ui::r(10.0))
        .fill(sidebar_base(s, u))
        .border(1.0, u.hair)
        .child(item)
}

fn art_section(s: &State, u: &Ui) -> El {
    let view = &s.settings.personalize;
    let surface = view.surface;
    let slot = s.prefs.personal.art.slot(surface);
    let kind = kind_of(&slot.source);
    let surfaces: Vec<(Surface, &str)> = Surface::ALL.iter().map(|s| (*s, s.label())).collect();
    let default_label = if surface == Surface::Tab { "Accent glow" } else { "Sigil's art" };
    let mut kinds = vec![(Kind::Off, "Off"), (Kind::Default, default_label)];
    if !heroes().is_empty() {
        kinds.push((Kind::Hero, "Hero"));
    }
    kinds.push((Kind::Custom, "Your image"));

    let mut body = col()
        .w_full()
        .gap(14.0)
        .child(row().child(segmented(u, "settings-art-surface", &surfaces, surface, |st, surface| {
            st.settings.personalize.surface = surface;
        })))
        .child(row().child(surface_preview(s, u, surface)))
        .child(row().child(segmented(u, "settings-art-kind", &kinds, kind, set_kind)));

    if kind == Kind::Hero {
        let chosen = match &slot.source {
            ArtSource::Hero(name) => name.clone(),
            _ => String::new(),
        };
        let mut grid = el().key("settings-art-heroes").w_full().equal_columns(4).gap(6.0);
        for (name, _, chip) in heroes() {
            let on = *name == chosen;
            let pick = name.clone();
            let mut r = row()
                .key(format!("settings-art-hero-{}", deadlock_core::locker::hero_asset_key(name)))
                .h(32.0)
                .px(8.0)
                .gap(8.0)
                .center_y()
                .radius(crate::ui::r(8.0))
                .fill(if on { u.selected } else { Color::TRANSPARENT })
                .border(1.0, if on { u.accent } else { Color::TRANSPARENT })
                .hover(if on { u.selected } else { u.hover })
                .focusable(true)
                .role("radio")
                .checked(on)
                .label(name.clone())
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    let pick = pick.clone();
                    let surface = st.settings.personalize.surface;
                    edit(st, |p| {
                        let slot = p.art.slot_mut(surface);
                        slot.source = ArtSource::Hero(pick.clone());
                        slot.framing = hero_framing(&pick);
                    })
                });
            if let Some(px) = chip.as_ref().and_then(|c| s.thumbnails.get(&c.display().to_string())) {
                r = r.child(el().size(20.0, 20.0).radius(crate::ui::r(10.0)).pixels_fit(px, SurfaceFit::Fill).a11y_hidden());
            }
            grid = grid.child(
                r.child(
                    text(name.clone())
                        .font(SMALL)
                        .color(if on { u.ink } else { u.prose })
                        .truncate()
                        .min_w(0.0)
                        .a11y_hidden(),
                ),
            );
        }
        body = body.child(grid);
    }
    if kind == Kind::Custom || matches!(slot.source, ArtSource::Custom(_)) {
        let busy = view.job.is_running();
        body = body.child(
            row().gap(8.0).center_y().child(super::disabled(
                button(
                    u,
                    "settings-art-upload",
                    if matches!(slot.source, ArtSource::Custom(_)) { "Change image" } else { "Upload image" },
                    ActionKind::Secondary,
                )
                .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                    choose_image(st, tx, Target::Art(surface))
                }),
                busy,
            )),
        );
    }
    let mut rows = vec![
        col()
            .key("settings-art")
            .w_full()
            .gap(12.0)
            .pt(12.0)
            .pb(16.0)
            .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
            .child(detail(u, "Art behind the active tab and the launch buttons. The label side stays readable.").wrap())
            .children(super::smalls::art_note(u, surface))
            .child(body),
    ];
    if matches!(slot.source, ArtSource::Hero(_) | ArtSource::Custom(_)) {
        let f = slot.framing;
        rows.push(
            col()
                .key("settings-art-framing")
                .w_full()
                .py(8.0)
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(slider_row(u, "art-x", "Across", f.x, format!("{:.0}%", f.x * 100.0), |st, v| frame_set(st, |f| f.x = v), true))
                .child(slider_row(u, "art-y", "Down", f.y, format!("{:.0}%", f.y * 100.0), |st, v| frame_set(st, |f| f.y = v), true))
                .child(slider_row(
                    u,
                    "art-zoom",
                    "Zoom",
                    (f.zoom - 1.0) / (ZOOM_MAX - 1.0),
                    format!("{:.1}x", f.zoom),
                    |st, v| frame_set(st, |f| f.zoom = 1.0 + v * (ZOOM_MAX - 1.0)),
                    true,
                ))
                .child(
                    row().w_full().pt(4.0).child(spacer()).child(
                        button(u, "settings-art-reset", "Reset framing", ActionKind::Ghost).on_press(
                            |st: &mut State| {
                                let surface = st.settings.personalize.surface;
                                edit(st, |p| {
                                    let slot = p.art.slot_mut(surface);
                                    slot.framing = match &slot.source {
                                        ArtSource::Hero(name) => hero_framing(name),
                                        _ => Framing::default(),
                                    };
                                })
                            },
                        ),
                    ),
                ),
        );
    }
    section(u, "Sidebar art", rows)
}

/// A hero's face in view: Grimoire's face position across, the upper third down.
fn hero_framing(name: &str) -> Framing {
    let x = deadlock_core::locker::roster_hero(name)
        .map(|h| h.face_x / 100.0)
        .unwrap_or(0.55);
    Framing { x, y: 0.22, zoom: 1.6 }
}

fn frame_set(st: &mut State, change: impl FnOnce(&mut Framing)) {
    let surface = st.settings.personalize.surface;
    change(&mut st.prefs.personal.art.slot_mut(surface).framing);
}

/// Appearance opened: the glow fields, and decodes for the hero chips and what is on.
pub fn on_open(st: &mut State) {
    let g = &st.prefs.personal.glow;
    st.settings.personalize.glow_from = TextEdit::new(g.from.clone());
    st.settings.personalize.glow_to = TextEdit::new(g.to.clone());
    ensure_images(st);
    let chips: Vec<String> = heroes()
        .iter()
        .filter_map(|(_, _, c)| c.as_ref().map(|c| c.display().to_string()))
        .collect();
    for path in chips {
        if st.thumbnails.contains_key(&path) {
            continue;
        }
        match st.images.as_ref() {
            Some(cache) => {
                let image = cache.file(&path, 1.0, 1.0);
                st.thumbnails.insert(path, image.pixels().clone());
            }
            None => {
                if let Ok(bytes) = std::fs::read(&path) {
                    st.pending_thumbs.push((path, bytes));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contrast_of_the_default_text_pairs_passes() {
        let u = Ui::themed(crate::ui::Theme::Dark, None);
        assert!(crate::ui::contrast(u.ink, u.bg) >= crate::ui::AA_TEXT);
        assert!(crate::ui::contrast(u.muted, u.bg) >= crate::ui::AA_TEXT);
    }
}

// --- scenes ------------------------------------------------------------------------

/// Headless scenes for Personalize.
pub const SCENES: [(&str, fn(&mut State)); 12] = [
    ("personalize", scene_settings),
    ("personalize-builder", scene_builder),
    ("personalize-theme", scene_theme),
    ("personalize-background", scene_background),
    ("personalize-glow", scene_glow),
    ("personalize-art", scene_art),
    ("personalize-all", scene_all),
    ("personalize-all-settings", scene_all_settings),
    ("personalize-statusbar", scene_statusbar),
    ("personalize-statusbar-settings", scene_statusbar_settings),
    ("personalize-volume-art", scene_volume_art),
    ("personalize-volume-art-settings", scene_volume_art_settings),
];

fn scene_settings(s: &mut State) {
    super::scene_tab(s, super::Tab::Appearance);
    on_open(s);
}

fn scene_builder(s: &mut State) {
    scene_settings(s);
    s.prefs.personal.theme = Some(ThemePick::Preset("verdigris"));
    let mut palette = crate::ui::preset("verdigris").map(|p| p.palette()).unwrap_or(PRESETS[0].palette());
    palette.muted = model::parse_hex6("#4a5a55").unwrap_or(palette.muted);
    let name = s.prefs.personal.copy_name(tr("Verdigris"));
    open_draft(s, None, name, palette);
    if let Some(d) = s.settings.personalize.draft.as_mut() {
        d.token = 6;
        d.hex = TextEdit::new("#4a5a55");
    }
}

/// A sample background: Grimoire's General backdrop from the Locker art.
fn sample_background(s: &mut State) {
    let dir = images_dir();
    let path = deadlock_core::locker::LockerAssetPaths::discover().general_backdrop;
    if let Some(path) = path
        && let Ok(name) = model::import_image(&path, &dir, "background", model::BACKGROUND_EDGE)
    {
        s.prefs.personal.background.file = Some(name);
        s.prefs.personal.background.on = true;
    }
}

fn scene_theme(s: &mut State) {
    s.load_demo();
    s.screen = crate::model::Screen::Installed;
    s.prefs.personal.theme = Some(ThemePick::Preset("brass"));
}

fn scene_background(s: &mut State) {
    crate::screens::browse::scene_fixture(s);
    sample_background(s);
    s.prefs.personal.background.opacity = 0.35;
    s.prefs.personal.background.blur = 6.0;
    s.prefs.personal.background.sidebar = true;
    ensure_images(s);
}

fn scene_glow(s: &mut State) {
    crate::screens::browse::scene_fixture(s);
    s.prefs.personal.glow.on = true;
    s.prefs.personal.glow.from = GLOWS[1].2.into();
    s.prefs.personal.glow.to = GLOWS[1].3.into();
}

fn scene_art(s: &mut State) {
    s.load_demo();
    s.screen = crate::model::Screen::Installed;
    let hero = heroes().iter().find(|(n, _, _)| n == "Lady Geist").or(heroes().first()).map(|(n, _, _)| n.clone());
    if let Some(hero) = hero {
        s.prefs.personal.art.tab = model::ArtSlot {
            framing: hero_framing(&hero),
            source: ArtSource::Hero(hero),
        };
    }
    s.prefs.personal.art.modded.source = ArtSource::Default;
    s.prefs.personal.art.vanilla.source = ArtSource::Default;
    ensure_images(s);
}

fn scene_all(s: &mut State) {
    scene_art(s);
    sample_background(s);
    s.prefs.personal.background.sidebar = true;
    s.prefs.personal.background.topbar = true;
    s.prefs.personal.glow.on = true;
    s.prefs.personal.theme = Some(ThemePick::Preset("cobalt"));
    s.prefs.personal.radius = Radius::Round;
    ensure_images(s);
}

/// The background image under the status bar too (DMM's "Include bottom bar").
fn scene_statusbar(s: &mut State) {
    scene_background(s);
    s.prefs.personal.background.opacity = 0.6;
    s.prefs.personal.background.blur = 0.0;
    s.prefs.personal.background.statusbar = true;
}

fn scene_statusbar_settings(s: &mut State) {
    scene_statusbar(s);
    s.settings.tab = super::Tab::Appearance;
    super::open(s);
    on_open(s);
}

/// Sigil's art behind the status bar's volume control (Grimoire's volume bar art).
fn scene_volume_art(s: &mut State) {
    s.load_demo();
    s.screen = crate::model::Screen::Installed;
    s.prefs.personal.art.volume.source = ArtSource::Default;
    ensure_images(s);
}

fn scene_volume_art_settings(s: &mut State) {
    scene_volume_art(s);
    s.settings.tab = super::Tab::Appearance;
    super::open(s);
    s.settings.personalize.surface = Surface::Volume;
    on_open(s);
}

/// Everything on, with Appearance open on the hero art picker.
fn scene_all_settings(s: &mut State) {
    scene_all(s);
    s.settings.tab = super::Tab::Appearance;
    super::open(s);
    s.settings.personalize.surface = Surface::Tab;
    on_open(s);
}
