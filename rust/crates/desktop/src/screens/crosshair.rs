//! Crosshair: the reticle the game reads, drawn at true scale over a game scene.
//!
//! The settings and the command text come from `deadlock_core::crosshair`, which
//! also owns the HUD geometry. Applying writes a managed block into
//! `autoexec.cfg`, so it sits behind a confirm step and the same backup path as
//! the Autoexec screen. The two scene captures are Grimoire's (MIT), recompressed
//! to WebP under `assets/crosshair/`.
//!
//! Presets sit under the stage as a grid of live mini reticles: the built-in ones from
//! `deadlock_core::crosshair::builtin_presets` and the player's own, saved as JSON in the app
//! data directory. A preset only loads into the editor; writing to the game stays the Apply
//! step with its confirm. Import reads a pasted share code or console commands.

use std::time::{Duration, Instant};

use deadlock_core::crosshair::{Crosshair, Import, Preset};
use rune::{
    Color, Drag, El, Key, KeyEvent, MotionIntent, Phase, Pixels, ScrollSpec, Spring, TextEdit, Tx,
    col, el, icon, row, spacer, text,
};

use crate::app::State;
use crate::model::Screen;
use crate::icons;
use crate::screens::settings::i18n::fill;

#[path = "crosshair_de.rs"]
mod de;
#[path = "crosshair_game.rs"]
pub mod game;

/// The UI text in the player's language: this page's German table first, then Sigil's.
fn tr(text: &str) -> &str {
    use crate::screens::settings::i18n::{self, Lang};
    if i18n::current() == Lang::German
        && let Some(de) = de::get(text)
    {
        return de;
    }
    i18n::tr(text)
}
use crate::jobs::Job;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, H2, LABEL, MONO, PAGE_TOP, PAGE_X, SMALL, Tone, Ui};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Gap,
    Height,
    Width,
    Opacity,
    OutlineBorder,
    OutlineGap,
    OutlineOpacity,
    DotSize,
    DotOpacity,
    DotBorder,
    DotGap,
    DotOutlineOpacity,
    Red,
    Green,
    Blue,
    OutlineRed,
    OutlineGreen,
    OutlineBlue,
}

/// The option group whose controls show under the picker.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Group {
    #[default]
    Lines,
    Dot,
    Colour,
    Game,
}

impl Group {
    const ALL: [Group; 4] = [Group::Lines, Group::Dot, Group::Colour, Group::Game];

    fn label(self) -> &'static str {
        match self {
            Group::Lines => tr("Lines"),
            Group::Dot => tr("Dot"),
            Group::Colour => tr("Color"),
            Group::Game => tr("Game"),
        }
    }
}

/// What the preview stage shows behind the reticle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scene {
    #[default]
    Street,
    Sandbox,
    Plain,
}

impl Scene {
    const ALL: [Scene; 3] = [Scene::Street, Scene::Sandbox, Scene::Plain];

    fn label(self) -> &'static str {
        match self {
            Scene::Street => "Street scene",
            Scene::Sandbox => "Sandbox scene",
            Scene::Plain => "Plain background",
        }
    }
}

/// What the Copy buttons last put on the clipboard, shown for a moment as "Copied".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Copied {
    Code,
    Console,
}

/// The preset the editor last loaded. A saved one can be renamed or deleted from the
/// section head; tiles ring on their own whenever the editor matches them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Chosen {
    Builtin(&'static str),
    Saved(String),
}

/// The preset and import dialogs. The Apply confirm keeps its own `pending` flag.
pub enum PresetDialog {
    /// Save the editor as a new preset, or rename the saved preset with this id.
    Name {
        editing: Option<String>,
        name: TextEdit,
        error: Option<String>,
    },
    /// Delete the saved preset with this id.
    Delete(String),
    /// Paste a share code or console commands.
    Import { input: TextEdit, error: Option<String> },
}

/// The decoded scene captures and their thumbnails.
pub struct SceneArt {
    street: Pixels,
    sandbox: Pixels,
    street_thumb: Pixels,
    sandbox_thumb: Pixels,
}

pub struct View {
    /// Wave 4: what writing to the hero settings would change (files changed, files), read only.
    pub w4_plan: Option<Result<(usize, usize), String>>,
    /// Wave 4: the hero settings write or restore on its worker (`game_apply`), and whether it
    /// was a write (true) or a restore.
    pub w4_job: Job<(bool, usize)>,
    pub settings: deadlock_core::crosshair::Crosshair,
    pub source: String,
    pub log: Option<String>,
    pub tone: Tone,
    pub pending: bool,
    pub group: Group,
    pub scene: Scene,
    /// The simulated display height in pixels: 1080, 1440 or 2160.
    pub resolution: u32,
    /// Index into `ZOOM_STEPS`.
    pub zoom: usize,
    pub art: Option<SceneArt>,
    pub share: Job<String>,
    pub copied: Option<(Copied, Instant)>,
    /// The player's saved presets, read once from the app data directory.
    pub presets: Vec<Preset>,
    pub presets_loaded: bool,
    pub chosen: Option<Chosen>,
    pub preset_dialog: Option<PresetDialog>,
    /// A pasted share code being decoded.
    pub decode: Job<Crosshair>,
    /// Bound when a text field outlives its dialog for a frame.
    pub blank: TextEdit,
    /// The "Remove from autoexec" confirm is open.
    pub confirm_remove: bool,
}

impl Default for View {
    fn default() -> Self {
        View {
            w4_plan: None,
            w4_job: Job::default(),
            settings: deadlock_core::crosshair::Crosshair::default(),
            source: "editor defaults".to_string(),
            log: None,
            tone: Tone::Quiet,
            pending: false,
            group: Group::default(),
            scene: Scene::default(),
            resolution: 1440,
            zoom: ZOOM_ONE,
            art: None,
            share: Job::default(),
            copied: None,
            presets: Vec::new(),
            presets_loaded: false,
            chosen: None,
            preset_dialog: None,
            decode: Job::default(),
            blank: TextEdit::new(""),
            confirm_remove: false,
        }
    }
}

/// The scene captures are 2560 by 1440. At 1x one capture pixel is one point, so the stage
/// is a window onto a 1440 pixel tall screen and the reticle is drawn at its in-game size.
const CAPTURE_W: f32 = 2560.0;
const CAPTURE_H: f32 = 1440.0;
const STREET: &[u8] = include_bytes!("../../assets/crosshair/street.webp");
const SANDBOX: &[u8] = include_bytes!("../../assets/crosshair/sandbox.webp");
const STREET_THUMB: &[u8] = include_bytes!("../../assets/crosshair/street-thumb.webp");
const SANDBOX_THUMB: &[u8] = include_bytes!("../../assets/crosshair/sandbox-thumb.webp");

/// Tall enough that the largest reticle the game accepts (a 171 point reach) fits at 1x
/// with `FIT_MARGIN` to spare, so 1x is always true scale.
const STAGE_H: f32 = 400.0;
const ZOOM_STEPS: [f32; 6] = [0.5, 0.75, 1.0, 1.5, 2.0, 3.0];
const ZOOM_ONE: usize = 2;
/// The reticle is scaled to fit the stage up to this zoom and clipped beyond it.
const FIT_UP_TO: f32 = 1.5;
/// Clear space kept between the fitted reticle and the stage edge.
const FIT_MARGIN: f32 = 24.0;
const RESOLUTIONS: [(u32, &str); 3] = [(1080, "1080p"), (1440, "1440p"), (2160, "4K")];
const SLIDER_W: f32 = 280.0;
const VALUE_W: f32 = 40.0;
const COPIED_FOR: Duration = Duration::from_secs(2);
/// The picker's selected card: a quick spring with a small settle past the target.
const SLIDE: Spring = Spring::new(0.3, 0.12);
const DESCRIPTION: &str = "Tune your crosshair and read it against a real scene.";

thread_local! {
    /// The main window, handed over by the shell when Rune creates it. UI thread only.
    static WINDOW: std::cell::RefCell<Option<rune::WindowHandle>> =
        const { std::cell::RefCell::new(None) };
}

/// Hook for `main.rs`: `.on_window(|w| screens::crosshair::remember_window(w))`. Rune calls it
/// on the UI thread each time it creates the window, so a recreated window keeps the pin.
pub fn remember_window(window: &rune::WindowHandle) {
    WINDOW.with(|slot| *slot.borrow_mut() = Some(window.clone()));
}

/// Keep the window above other windows, or let it go back to normal. Grimoire's Pin window:
/// tune the crosshair with the game in view. The shell owns the window state.
fn toggle_pin(st: &mut State) {
    let on = !crate::shell::always_on_top(st);
    crate::shell::set_always_on_top(st, on);
}

/// Called from the app's poll: decodes the scene art the first time the screen shows,
/// takes the share code result and clears the "Copied" flash.
pub fn poll(s: &mut State, tx: &mut Tx) -> bool {
    // The Autoexec page's launch options jobs, until app.rs polls them directly.
    let mut changed = super::autoexec::poll(s);
    if s.screen == Screen::Crosshair
        && s.crosshair.art.is_none()
        && let Some(cache) = s.images.as_ref()
    {
        let load = |bytes: &'static [u8]| {
            let image = cache.load(bytes);
            if rune::is_headless() {
                let _ = image.wait();
            }
            image.pixels().clone()
        };
        s.crosshair.art = Some(SceneArt {
            street: load(STREET),
            sandbox: load(SANDBOX),
            street_thumb: load(STREET_THUMB),
            sandbox_thumb: load(SANDBOX_THUMB),
        });
        changed = true;
    }
    if s.screen == Screen::Crosshair && !s.crosshair.presets_loaded {
        load_presets(s);
        changed = true;
    }
    if let Some(result) = s.crosshair.decode.try_take() {
        match result {
            Ok(crosshair) => {
                s.crosshair.settings = crosshair;
                s.crosshair.source = "share code".to_string();
                s.crosshair.chosen = None;
                s.crosshair.preset_dialog = None;
                s.crosshair.log = Some("Imported the share code.".to_string());
                s.crosshair.tone = Tone::Ok;
            }
            Err(error) => {
                let message = format!("Could not read the share code: {error}");
                match s.crosshair.preset_dialog.as_mut() {
                    Some(PresetDialog::Import { error, .. }) => *error = Some(message),
                    _ => {
                        s.crosshair.log = Some(message);
                        s.crosshair.tone = Tone::Warn;
                    }
                }
            }
        }
        changed = true;
    }
    if let Some(result) = s.crosshair.share.try_take() {
        match result {
            Ok(code) => {
                tx.copy(code);
                flash(s, Copied::Code);
            }
            Err(error) => {
                s.crosshair.log = Some(format!("Could not make a share code: {error}"));
                s.crosshair.tone = Tone::Warn;
            }
        }
        changed = true;
    }
    changed |= poll_hero_write(s);
    if let Some((_, at)) = s.crosshair.copied
        && at.elapsed() >= COPIED_FOR
    {
        s.crosshair.copied = None;
        changed = true;
    }
    changed
}

/// Show "Copied" on a button for two seconds, then wake the loop to clear it.
fn flash(st: &mut State, what: Copied) {
    st.crosshair.copied = Some((what, Instant::now()));
    if let Some(wake) = st.wake.clone() {
        let _ = std::thread::Builder::new()
            .name("dah-crosshair-copied".to_string())
            .spawn(move || {
                std::thread::sleep(COPIED_FOR + Duration::from_millis(30));
                wake();
            });
    }
}

fn copy_share_code(st: &mut State) {
    if st.crosshair.share.is_running() {
        return;
    }
    if !st.prefs.allow_share_codes {
        st.crosshair.log =
            Some("Share codes are turned off in Settings, Privacy.".to_string());
        st.crosshair.tone = Tone::Quiet;
        return;
    }
    if rune::is_headless() {
        st.crosshair.log = Some("Headless run: the share code was not requested.".to_string());
        st.crosshair.tone = Tone::Quiet;
        return;
    }
    st.crosshair.log = None;
    let settings = st.crosshair.settings.clone();
    let wake = st.wake.clone();
    st.crosshair.share.start(wake, move || {
        deadlock_core::crosshair::fetch_share_code(&settings).map_err(|error| error.to_string())
    });
}

fn copy_console(st: &mut State, tx: &mut Tx) {
    tx.copy(st.crosshair.settings.console_line());
    flash(st, Copied::Console);
}

fn copied(v: &View, what: Copied) -> bool {
    matches!(v.copied, Some((shown, _)) if shown == what)
}

pub fn view(s: &State, u: &Ui) -> El {
    let v = &s.crosshair;
    let pinned = crate::shell::always_on_top(s);

    let code_label = if v.share.is_running() {
        "Copying"
    } else if copied(v, Copied::Code) {
        "Copied"
    } else {
        "Copy code"
    };
    let code_icon = if copied(v, Copied::Code) {
        &icons::CHECK
    } else {
        &icons::COPY
    };

    let actions = row()
        .gap(8.0)
        .center_y()
        .child(
            u.icon_action(
                if pinned { &icons::PIN_OFF } else { &icons::PIN },
                tr(if pinned { "Pinned" } else { "Pin window" }),
                ActionKind::Secondary,
            )
            .key("crosshair-pin")
            .w(124.0)
            .fill(if pinned { u.selected } else { Color::TRANSPARENT })
            .label(tr(if pinned { "Unpin the window" } else { "Keep the window above other windows" }))
            .checked(pinned)
            .cursor(rune::Cursor::Pointer)
            .on_press(toggle_pin),
        )
        .child(
            u.icon_action(&icons::ROTATE_CCW, tr("Reset"), ActionKind::Secondary)
                .key("crosshair-reset")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| {
                    st.crosshair.settings = deadlock_core::crosshair::Crosshair::default();
                    st.crosshair.source = "editor defaults".to_string();
                    st.crosshair.log = None;
                    st.crosshair.tone = Tone::Quiet;
                }),
        )
        .child(
            u.icon_action(&icons::DOWNLOAD, tr("Import"), ActionKind::Secondary)
                .key("crosshair-import")
                .cursor(rune::Cursor::Pointer)
                .on_press(open_import),
        )
        .child(
            u.icon_action(code_icon, tr(code_label), ActionKind::Secondary)
                .key("crosshair-copy-code")
                .w(116.0)
                .cursor(rune::Cursor::Pointer)
                .on_press(copy_share_code),
        )
        .child(
            u.action(tr("Apply"), ActionKind::Primary)
                .key("crosshair-apply")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| st.crosshair.pending = true),
        );

    let mut page = col()
        .key("crosshair-page")
        .w_full()
        .px(PAGE_X)
        .pt(PAGE_TOP)
        .pb(32.0)
        .gap(16.0)
        .child(u.page_head("", tr("Crosshair"), tr(DESCRIPTION), Some(actions)));

    if let Some(log) = &v.log {
        page = page.child(log_line(u, log, v.tone));
    }

    let controls = match v.group {
        Group::Lines => lines_section(s, u),
        Group::Dot => dot_section(s, u),
        Group::Colour => colour_section(s, u),
        Group::Game => game_section(s, u),
    };

    page = page
        .child(stage(s, u))
        .child(presets_section(s, u))
        .child(
            col()
                .w_full()
                .pt(8.0)
                .child(picker(u, v.group))
                .child(controls.key(format!("crosshair-group-{:?}", v.group))),
        )
        .child(el().h(8.0))
        .child(commands(s, u));

    col()
        .key("crosshair-scroll")
        .w_full()
        .h_full()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(page)
}

fn log_line(u: &Ui, message: &str, tone: Tone) -> El {
    let bad = matches!(tone, Tone::Warn | Tone::Bad);
    row()
        .key("crosshair-log")
        .w_full()
        .gap(8.0)
        .items_start()
        .child(
            el().pt(3.0).child(
                icon(
                    if bad {
                        &icons::TRIANGLE_ALERT
                    } else {
                        &icons::CHECK
                    },
                    14.0,
                )
                .color(if bad { tone.fg(u) } else { u.muted })
                .a11y_hidden(),
            ),
        )
        .child(
            text(friendly(message))
                .font(BODY)
                .color(u.muted)
                .grow(1.0)
                .min_w(0.0)
                .wrap(),
        )
}

/// The import reports a missing convars file with its full path; a player only needs to know
/// the game has nothing saved yet.
fn friendly(message: &str) -> String {
    if message.contains("machine_convars.vcfg")
        && (message.contains("No such file") || message.contains("never have written"))
    {
        "The game has not saved a crosshair yet, so the defaults are shown.".to_string()
    } else if message.contains("No Deadlock install") && message.contains("machine_convars") {
        "No Deadlock install found, so there is no in-game crosshair to import.".to_string()
    } else {
        message.to_string()
    }
}

// --- stage ------------------------------------------------------------------------------

fn colour(r: i32, g: i32, b: i32, a: f64) -> Color {
    Color {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: a as f32,
    }
}

/// The preview: the chosen scene, the reticle on top at true scale, scene thumbnails top
/// left and the resolution and zoom controls top right.
fn stage(s: &State, u: &Ui) -> El {
    let v = &s.crosshair;
    let chosen = ZOOM_STEPS[v.zoom.min(ZOOM_STEPS.len() - 1)];
    let fit = fit_factor(&v.settings, chosen);
    // Scene and reticle shrink together, so the reticle keeps its size against the scene.
    let zoom = chosen * fit;

    // Everything hangs off a zero sized anchor at the stage centre, so the scene and the
    // reticle stay centred at any width and the stage clips whatever runs past its edge.
    let mut anchor = el().key("crosshair-anchor").size(0.0, 0.0);
    let scene = match (&v.art, v.scene) {
        (Some(art), Scene::Street) => Some(&art.street),
        (Some(art), Scene::Sandbox) => Some(&art.sandbox),
        _ => None,
    };
    if let Some(pixels) = scene {
        let (w, h) = (CAPTURE_W * zoom, CAPTURE_H * zoom);
        anchor = anchor.child(
            el().key(format!("crosshair-scene-{:?}", v.scene))
                .absolute(-w / 2.0, -h / 2.0)
                .size(w, h)
                .pixels(pixels)
                .a11y_hidden(),
        );
    }
    anchor = reticle(&v.settings, v.resolution, zoom, anchor);

    let mut surface = el()
        .key("crosshair-stage")
        .role("img")
        .label(format!("Crosshair preview: {}", v.settings))
        .w_full()
        .h(STAGE_H)
        .radius(crate::ui::r(12.0))
        .clip_rounded(crate::ui::r(12.0))
        .fill(oklab_dark());
    if scene.is_none() {
        surface = surface.radial_gradient(
            [0.5, 0.5],
            [640.0, 320.0],
            [
                (0.0, Color::WHITE.alpha(0.07)),
                (1.0, Color::WHITE.alpha(0.015)),
            ],
        );
    }
    surface
        .child(
            el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .center()
                .child(anchor),
        )
        // The hairline sits above the scene so the image never covers the edge.
        .child(
            el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .radius(crate::ui::r(12.0))
                .border(1.0, u.hair)
                .a11y_hidden(),
        )
        .child(
            scene_bar(s, u)
                .pin(Some(12.0), Some(12.0), None, None),
        )
        .child(view_bar(s, u).pin(None, Some(12.0), Some(12.0), None))
        .children((fit < 1.0).then(|| {
            bar()
                .key("crosshair-fitted")
                .h(28.0)
                .px(10.0)
                .radius(crate::ui::r(8.0))
                .child(
                    text(format!("Scaled to fit, {}", zoom_label(zoom)))
                        .font(LABEL)
                        .color(u.prose),
                )
                .pin(Some(12.0), None, None, Some(12.0))
        }))
}

/// How much the view shrinks so the whole reticle stays on the stage. A large gap and
/// length at 1.5x would run past the edge; past that the player asked for a close look,
/// so the stage clips instead.
fn fit_factor(c: &deadlock_core::crosshair::Crosshair, zoom: f32) -> f32 {
    if zoom > FIT_UP_TO {
        return 1.0;
    }
    // `reach` is in 1080p units; the stage shows a 1440 pixel tall screen at 1x.
    let reach = c.layout(1.0).reach * CAPTURE_H / 1080.0 * zoom;
    let room = STAGE_H / 2.0 - FIT_MARGIN;
    if reach > room { room / reach } else { 1.0 }
}

fn oklab_dark() -> Color {
    rune::oklch(0.15, 0.002, 286.0)
}

/// The reticle as the HUD rasterizes it: whole device pixels of the chosen resolution, each
/// drawn `1440 * zoom / resolution` points wide, so 1440p at 1x is one pixel per point.
fn reticle(c: &deadlock_core::crosshair::Crosshair, resolution: u32, zoom: f32, mut anchor: El) -> El {
    let scale = resolution as f32 / 1080.0;
    let k = CAPTURE_H * zoom / resolution as f32;
    let layout = c.layout(scale);
    let fill = colour(c.color_r, c.color_g, c.color_b, c.pip_opacity);
    let outline = |a: f64| colour(c.outline_color_r, c.outline_color_g, c.outline_color_b, a);
    let dot = colour(c.color_r, c.color_g, c.color_b, c.dot_opacity);

    let place = |e: El, r: &deadlock_core::crosshair::Rect| {
        e.absolute(r.x * k, r.y * k).size(r.w * k, r.h * k)
    };

    // An outline is a ring: its border is the outline, the inside stays clear so an
    // outline gap shows the scene, as it does in game.
    if layout.pip_outline_border > 0.0 && c.pip_outline_opacity > 0.0 {
        for (index, rect) in layout.pip_outlines.iter().enumerate() {
            if rect.w <= 0.0 || rect.h <= 0.0 {
                continue;
            }
            anchor = anchor.child(
                place(el().key(format!("outline-{index}")), rect)
                    .border(layout.pip_outline_border * k, outline(c.pip_outline_opacity)),
            );
        }
    }
    if c.pip_opacity > 0.0 {
        for (index, rect) in layout.pips.iter().enumerate() {
            if rect.w <= 0.0 || rect.h <= 0.0 {
                continue;
            }
            anchor = anchor.child(place(el().key(format!("pip-{index}")), rect).fill(fill));
        }
    }
    if c.dot_opacity > 0.0 && layout.dot.w > 0.0 {
        if layout.dot_outline_border > 0.0 && c.dot_outline_opacity > 0.0 {
            anchor = anchor.child(
                place(el().key("dot-outline"), &layout.dot_outline)
                    .radius(layout.dot_outline.w * k / 2.0)
                    .border(layout.dot_outline_border * k, outline(c.dot_outline_opacity)),
            );
        }
        anchor = anchor.child(
            place(el().key("dot"), &layout.dot)
                .radius(layout.dot.w * k / 2.0)
                .fill(dot),
        );
    }
    anchor
}

/// A dark glass strip over the scene for the stage controls.
fn bar() -> El {
    row()
        .h(40.0)
        .padding(4.0)
        .gap(4.0)
        .center_y()
        .radius(crate::ui::r(10.0))
        .fill(Color::BLACK.alpha(0.55))
        .border(1.0, crate::ui::Ui::new().hair)
}

fn scene_bar(s: &State, u: &Ui) -> El {
    let v = &s.crosshair;
    let mut strip = bar()
        .key("crosshair-scenes")
        .role("radiogroup")
        .label("Scene");
    for scene in Scene::ALL {
        let on = v.scene == scene;
        let thumb = match (&v.art, scene) {
            (Some(art), Scene::Street) => Some(&art.street_thumb),
            (Some(art), Scene::Sandbox) => Some(&art.sandbox_thumb),
            _ => None,
        };
        let mut chip = el()
            .key(format!("crosshair-scene-pick-{scene:?}"))
            .size(52.0, 32.0)
            .radius(crate::ui::r(6.0))
            .clip_rounded(crate::ui::r(6.0))
            .fill(Color::WHITE.alpha(0.06))
            .focusable(true)
            .role("radio")
            .checked(on)
            .label(scene.label())
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| st.crosshair.scene = scene);
        chip = match thumb {
            Some(pixels) => chip.child(
                el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                    .pixels_fit(pixels, rune::SurfaceFit::Fill)
                    .a11y_hidden(),
            ),
            None => chip.center().child(
                text("Plain")
                    .font(LABEL)
                    .color(if on { u.ink } else { u.muted })
                    .a11y_hidden(),
            ),
        };
        // The ring sits on top of the image so the selection reads on any scene.
        chip = chip.child(
            el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .radius(crate::ui::r(6.0))
                .border(
                    if on { 1.5 } else { 1.0 },
                    if on { u.ink } else { Color::WHITE.alpha(0.10) },
                )
                .hover(Color::WHITE.alpha(0.08))
                .a11y_hidden(),
        );
        strip = strip.child(chip);
    }
    strip
}

fn view_bar(s: &State, u: &Ui) -> El {
    let v = &s.crosshair;
    let selected = RESOLUTIONS
        .iter()
        .position(|(height, _)| *height == v.resolution)
        .unwrap_or(1);
    let labels: Vec<&str> = RESOLUTIONS.iter().map(|(_, label)| *label).collect();
    let zoom = ZOOM_STEPS[v.zoom.min(ZOOM_STEPS.len() - 1)];
    let at_min = v.zoom == 0;
    let at_max = v.zoom + 1 >= ZOOM_STEPS.len();

    let step = |glyph: &'static rune::Icon, label: &str, off: bool, delta: i32| {
        let mut b = el()
            .key(format!("crosshair-zoom-{label}"))
            .size(32.0, 32.0)
            .center()
            .radius(crate::ui::r(6.0))
            .role("button")
            .label(label.to_string())
            .child(
                icon(glyph, 16.0)
                    .color(if off { u.faint } else { u.prose })
                    .a11y_hidden(),
            );
        if !off {
            b = b
                .focusable(true)
                .hover(Color::WHITE.alpha(0.08))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    let next = st.crosshair.zoom as i32 + delta;
                    st.crosshair.zoom = next.clamp(0, ZOOM_STEPS.len() as i32 - 1) as usize;
                });
        }
        b
    };

    bar()
        .key("crosshair-view")
        .child(segmented(
            u,
            "crosshair-resolution",
            "Resolution",
            &labels,
            selected,
            true,
            |st: &mut State, index| st.crosshair.resolution = RESOLUTIONS[index].0,
        ))
        .child(el().w(1.0).h(20.0).mx(4.0).fill(Color::WHITE.alpha(0.10)))
        .child(step(&icons::MINUS, "Zoom out", at_min, -1))
        .child(
            el().key("crosshair-zoom-reset")
                .w(44.0)
                .h(32.0)
                .center()
                .radius(crate::ui::r(6.0))
                .focusable(true)
                .role("button")
                .label("Reset zoom")
                .hover(Color::WHITE.alpha(0.08))
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| st.crosshair.zoom = ZOOM_ONE)
                .child(
                    text(zoom_label(zoom))
                        .font(MONO)
                        .color(u.ink)
                        .a11y_hidden(),
                ),
        )
        .child(step(&icons::PLUS, "Zoom in", at_max, 1))
}

fn zoom_label(zoom: f32) -> String {
    let mut label = format!("{zoom:.2}");
    while label.ends_with('0') {
        label.pop();
    }
    if label.ends_with('.') {
        label.pop();
    }
    format!("{label}x")
}

// --- option picker ----------------------------------------------------------------------

/// Lines, Dot, Color and Game: one card holding four option cards. The selected card's
/// fill is one element that springs from cell to cell, so a pick slides rather than cuts.
fn picker(u: &Ui, group: Group) -> El {
    let labels: Vec<&str> = Group::ALL.iter().map(|g| g.label()).collect();
    let selected = Group::ALL.iter().position(|g| *g == group).unwrap_or(0);
    segmented(
        u,
        "crosshair-groups",
        "Crosshair options",
        &labels,
        selected,
        false,
        |st: &mut State, index| st.crosshair.group = Group::ALL[index],
    )
    .w_full()
}

/// A segmented control built from cards in equal columns. The highlight shares the selected
/// cell and carries the spring, so a new pick moves it across instead of swapping fills.
fn segmented(
    u: &Ui,
    key: &str,
    name: &str,
    labels: &[&str],
    selected: usize,
    compact: bool,
    pick: fn(&mut State, usize),
) -> El {
    let (h, pad, radius) = if compact { (32.0, 0.0, 6.0) } else { (36.0, 4.0, 8.0) };
    let mut grid = row()
        .key(key.to_string())
        .equal_columns(labels.len().max(1))
        .gap(4.0)
        .role("radiogroup")
        .label(name.to_string());
    if !compact {
        grid = grid
            .padding(pad)
            .radius(radius + pad)
            .fill(u.well)
            .border(1.0, u.hair);
    }
    grid = grid.child(
        el().key(format!("{key}-indicator"))
            .grid_cell(selected)
            .h(h)
            .radius(radius)
            .fill(if compact {
                Color::WHITE.alpha(0.14)
            } else {
                Color::WHITE.alpha(0.075)
            })
            .border(1.0, if compact { Color::TRANSPARENT } else { u.hair_strong })
            .motion(SLIDE)
            .motion_intent(MotionIntent::OVERSHOOT)
            .a11y_hidden(),
    );
    for (index, label) in labels.iter().enumerate() {
        let on = index == selected;
        let mut cell = row()
            .key(format!("{key}-{index}"))
            .grid_cell(index)
            .h(h)
            .px(if compact { 10.0 } else { 12.0 })
            .center()
            .radius(radius)
            .focusable(true)
            .role("radio")
            .checked(on)
            .label(label.to_string())
            .child(
                text(label.to_string())
                    .font(if compact { LABEL } else { BODY_MED })
                    .color(if on { u.ink } else { u.muted })
                    .truncate()
                    .a11y_hidden(),
            );
        if !compact {
            // Every option is a card of its own; the sliding highlight is the selected one.
            cell = cell.border(1.0, u.hair);
        }
        if !on {
            cell = cell
                .hover(Color::WHITE.alpha(0.04))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| pick(st, index));
        }
        grid = grid.child(cell);
    }
    grid
}

// --- groups -----------------------------------------------------------------------------

fn lines_section(s: &State, u: &Ui) -> El {
    let c = &s.crosshair.settings;
    rows()
        .child(slider_row(u, tr("Gap"), c, Field::Gap))
        .child(slider_row(u, tr("Length"), c, Field::Height))
        .child(slider_row(u, tr("Thickness"), c, Field::Width))
        .child(slider_row(u, tr("Opacity"), c, Field::Opacity))
        .child(slider_row(u, tr("Outline"), c, Field::OutlineBorder))
        .child(slider_row(u, tr("Outline gap"), c, Field::OutlineGap))
        .child(slider_row(u, tr("Outline opacity"), c, Field::OutlineOpacity))
}

fn dot_section(s: &State, u: &Ui) -> El {
    let c = &s.crosshair.settings;
    rows()
        .child(slider_row(u, tr("Size"), c, Field::DotSize))
        .child(slider_row(u, tr("Opacity"), c, Field::DotOpacity))
        .child(slider_row(u, tr("Outline"), c, Field::DotBorder))
        .child(slider_row(u, tr("Outline gap"), c, Field::DotGap))
        .child(slider_row(u, tr("Outline opacity"), c, Field::DotOutlineOpacity))
}

fn colour_section(s: &State, u: &Ui) -> El {
    let c = &s.crosshair.settings;
    let hex = |r: i32, g: i32, b: i32| {
        text(format!("#{r:02X}{g:02X}{b:02X}"))
            .font(MONO)
            .color(u.muted)
            .w(64.0)
            .align(rune::Align::End)
    };
    rows()
        .child(
            setting_row(u, tr("Lines and dot"))
                .child(swatches(u, c))
                .child(hex(c.color_r, c.color_g, c.color_b)),
        )
        .child(slider_row(u, tr("Red"), c, Field::Red))
        .child(slider_row(u, tr("Green"), c, Field::Green))
        .child(slider_row(u, tr("Blue"), c, Field::Blue))
        .child(
            setting_row(u, tr("Outline color"))
                .key("crosshair-outline-colour")
                .child(
                    el().size(20.0, 20.0)
                        .radius(5.0)
                        .border(1.0, u.hair)
                        .fill(colour(
                            c.outline_color_r,
                            c.outline_color_g,
                            c.outline_color_b,
                            1.0,
                        ))
                        .a11y_hidden(),
                )
                .child(hex(c.outline_color_r, c.outline_color_g, c.outline_color_b)),
        )
        .child(slider_row(u, tr("Outline red"), c, Field::OutlineRed))
        .child(slider_row(u, tr("Outline green"), c, Field::OutlineGreen))
        .child(slider_row(u, tr("Outline blue"), c, Field::OutlineBlue))
}

fn game_section(s: &State, u: &Ui) -> El {
    let c = &s.crosshair.settings;
    let on = crate::screens::settings::options::get().crosshair_hero_settings;
    // Wave 4: DMM's "write the crosshair to the game's hero settings", through the owner's
    // `game_apply` step with a backup; off restores.
    let hero_line = if s.crosshair.w4_job.is_running() {
        tr("Writing the hero settings").to_string()
    } else if on {
        match &s.crosshair.w4_plan {
            Some(Ok((changed, files))) => fill(
                tr("On: {} of {} hero settings files take the crosshair. Apply writes it again; turning it off restores your previous settings."),
                &[&changed.to_string(), &files.to_string()],
            ),
            Some(Err(error)) => error.clone(),
            None => tr("On: the crosshair is the game's default, hero overrides stay. Turning it off restores your previous settings.").to_string(),
        }
    } else {
        tr("Applies the crosshair as the game's default and keeps hero overrides. Turning it off restores your previous settings. Close Deadlock first.").to_string()
    };
    rows()
        .child(
            col()
                .key("crosshair-hero-settings")
                .w_full()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .pb(8.0)
                .child(setting_row(u, tr("Write to the game's hero settings")).border_sides([0.0; 4], u.hair).child(
                    u.toggle(on)
                        .label(tr("Write to the game's hero settings").to_string())
                        .cursor(rune::Cursor::Pointer)
                        .on_press(toggle_hero_settings),
                ))
                .child(text(hero_line).font(SMALL).color(u.muted).wrap()),
        )
        .child(switch_row(
            u,
            tr("Static gap"),
            c.pip_gap_static,
            |st: &mut State| {
                st.crosshair.settings.pip_gap_static = !st.crosshair.settings.pip_gap_static;
                st.crosshair.source = "edited".to_string();
            },
        ))
        .child(switch_row(
            u,
            tr("Disable hero crosshair"),
            c.disable_hero_specific_crosshairs,
            |st: &mut State| {
                st.crosshair.settings.disable_hero_specific_crosshairs =
                    !st.crosshair.settings.disable_hero_specific_crosshairs;
                st.crosshair.source = "edited".to_string();
            },
        ))
}

/// Turn the hero settings write on or off (DMM): on writes the current crosshair into the
/// game's and the Steam account's `citadel_hero_settings.lst` with a backup; off restores what
/// was there. Both run on a worker through the owner's `game_apply` step, with the game closed.
fn toggle_hero_settings(st: &mut State) {
    if crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    let on = !crate::screens::settings::options::get().crosshair_hero_settings;
    crate::screens::settings::options::edit(st, |o| o.crosshair_hero_settings = on);
    st.crosshair.w4_plan = if on { Some(hero_settings_plan(st)) } else { None };
    start_hero_write(st, on);
}

/// After a crosshair is applied with the switch on, the hero settings get it too.
pub fn after_apply(st: &mut State) {
    if crate::screens::settings::options::get().crosshair_hero_settings {
        start_hero_write(st, true);
    }
}

/// The game's and the Steam accounts' hero settings files.
fn hero_settings_paths(st: &State) -> Option<Vec<std::path::PathBuf>> {
    let root = st.game_root.clone()?;
    let roots = deadlock_core::discovery::default_steam_roots();
    let accounts: Vec<u32> = deadlock_core::steam_users::detect(&roots).iter().map(|u| u.account_id).collect();
    Some(deadlock_core::game_plans::hero_settings_paths(&root, roots.first().map(|p| p.as_path()), &accounts))
}

fn hero_backup_path() -> std::path::PathBuf {
    crate::data::data_dir().join("crosshair-hero-settings-backup.json")
}

/// GAME WRITE, user triggered: the owner's `game_apply::apply_crosshair_hero_settings` with
/// approve, off the UI thread.
fn start_hero_write(st: &mut State, on: bool) {
    if st.demo || rune::is_headless() || st.crosshair.w4_job.is_running() {
        return;
    }
    let Some(paths) = hero_settings_paths(st) else {
        st.crosshair.log = Some(tr("Find the game folder first.").to_string());
        st.crosshair.tone = Tone::Warn;
        return;
    };
    let crosshair = on.then(|| st.crosshair.settings.clone());
    let backup = hero_backup_path();
    let wake = st.wake.clone();
    st.crosshair.w4_job.start(wake, move || {
        deadlock_core::game_apply::apply_crosshair_hero_settings(&paths, &backup, crosshair.as_ref(), true)
            .map(|count| (on, count))
            .map_err(|e| e.to_string())
    });
}

/// Take the worker's result: a toast either way (DMM's wording).
fn poll_hero_write(st: &mut State) -> bool {
    let Some(result) = st.crosshair.w4_job.try_take() else {
        return false;
    };
    match result {
        Ok((true, _)) => crate::notify::push(
            st,
            crate::notify::Notice::success(tr("Crosshair written to the hero settings")).detail(tr("Launch Deadlock to use it.")),
        ),
        Ok((false, _)) => crate::notify::push(
            st,
            crate::notify::Notice::success(tr("Hero settings restored")).detail(tr("Your previous crosshair settings are back.")),
        ),
        Err(error) => crate::notify::push(
            st,
            crate::notify::Notice::warning(tr("The hero settings were not changed")).detail(error),
        ),
    }
    st.crosshair.w4_plan = None;
    true
}

/// (settings that would change, files) for the current crosshair, read only.
fn hero_settings_plan(st: &State) -> Result<(usize, usize), String> {
    use deadlock_core::game_plans;
    let paths = hero_settings_paths(st).ok_or_else(|| tr("Find the game folder first.").to_string())?;
    let backup = game_plans::read_crosshair_backup(&hero_backup_path()).map_err(|e| e.to_string())?;
    let plan = game_plans::plan_crosshair_write(&paths, &backup, Some(&st.crosshair.settings)).map_err(|e| e.to_string())?;
    let changed = plan
        .edits
        .iter()
        .filter(|e| e.original.as_deref() != Some(e.updated.as_str()))
        .count();
    Ok((changed, plan.edits.len()))
}

/// The rows of one group, straight under the picker.
fn rows() -> El {
    col().w_full().pt(8.0)
}

/// A 44 point row: label on the left, the control on the right, a hairline under it.
fn setting_row(u: &Ui, label: &str) -> El {
    row()
        .w_full()
        .h(44.0)
        .gap(16.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(
            text(label.to_string())
                .font(BODY)
                .color(u.prose)
                .grow(1.0)
                .min_w(0.0),
        )
}

fn slider_row(u: &Ui, label: &str, c: &deadlock_core::crosshair::Crosshair, field: Field) -> El {
    let value = get(c, field);
    let (low, high) = range(field);
    let frac = ((value - low) / (high - low)).clamp(0.0, 1.0) as f32;
    let shown = if is_float(field) {
        format!("{value:.2}")
    } else {
        format!("{}", value as i32)
    };
    // The Color group's values line up with its hex readouts.
    let value_w = if range(field).1 > 64.0 { 64.0 } else { VALUE_W };
    setting_row(u, label)
        .child(slider(u, label, field, frac))
        .child(
            text(shown)
                .font(MONO)
                .color(u.ink)
                .w(value_w)
                .align(rune::Align::End),
        )
}

/// A flat slider: a 4 point track, a neutral fill and a white knob. Drag anywhere on it, or
/// use the arrow keys when it has focus.
fn slider(u: &Ui, label: &str, field: Field, frac: f32) -> El {
    let knob = 14.0;
    let track_y = 10.0;
    el().key(format!("slider-{field:?}"))
        .w(SLIDER_W)
        .h(24.0)
        .focusable(true)
        .role("slider")
        .label(label.to_string())
        .cursor(rune::Cursor::Grab)
        .child(
            el().absolute(0.0, track_y)
                .w(SLIDER_W)
                .h(4.0)
                .radius(2.0)
                .fill(Color::WHITE.alpha(0.10))
                .a11y_hidden(),
        )
        .child(
            el().absolute(0.0, track_y)
                .w((SLIDER_W * frac).max(0.0))
                .h(4.0)
                .radius(2.0)
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
            set_fraction(st, field, (d.x / width).clamp(0.0, 1.0) as f64);
        })
        .on_key(move |st: &mut State, ev: &KeyEvent, _| match ev.key {
            Key::Left | Key::Down => {
                nudge(st, field, -step(field));
                true
            }
            Key::Right | Key::Up => {
                nudge(st, field, step(field));
                true
            }
            _ => false,
        })
}

fn is_float(field: Field) -> bool {
    matches!(
        field,
        Field::Opacity | Field::OutlineOpacity | Field::DotOpacity | Field::DotOutlineOpacity
    )
}

/// The keyboard step: one unit, or 0.05 for an opacity (in hundredths, as `nudge` takes it).
fn step(field: Field) -> i32 {
    if is_float(field) { 5 } else { 1 }
}

fn range(field: Field) -> (f64, f64) {
    match field {
        Field::Red
        | Field::Green
        | Field::Blue
        | Field::OutlineRed
        | Field::OutlineGreen
        | Field::OutlineBlue => (0.0, 255.0),
        Field::Gap | Field::Height | Field::Width | Field::DotSize => (0.0, 64.0),
        Field::OutlineBorder | Field::OutlineGap | Field::DotBorder | Field::DotGap => (0.0, 16.0),
        _ => (0.0, 1.0),
    }
}

fn get(c: &deadlock_core::crosshair::Crosshair, field: Field) -> f64 {
    match field {
        Field::Gap => c.pip_gap as f64,
        Field::Height => c.pip_height as f64,
        Field::Width => c.pip_width as f64,
        Field::Opacity => c.pip_opacity,
        Field::OutlineBorder => c.pip_outline_border as f64,
        Field::OutlineGap => c.pip_outline_gap as f64,
        Field::OutlineOpacity => c.pip_outline_opacity,
        Field::DotSize => c.dot_size as f64,
        Field::DotOpacity => c.dot_opacity,
        Field::DotBorder => c.dot_outline_border as f64,
        Field::DotGap => c.dot_outline_gap as f64,
        Field::DotOutlineOpacity => c.dot_outline_opacity,
        Field::Red => c.color_r as f64,
        Field::Green => c.color_g as f64,
        Field::Blue => c.color_b as f64,
        Field::OutlineRed => c.outline_color_r as f64,
        Field::OutlineGreen => c.outline_color_g as f64,
        Field::OutlineBlue => c.outline_color_b as f64,
    }
}

/// Set a field from a 0..1 position along its slider.
fn set_fraction(st: &mut State, field: Field, frac: f64) {
    let (low, high) = range(field);
    let value = low + frac * (high - low);
    let current = get(&st.crosshair.settings, field);
    let delta = if is_float(field) {
        ((value - current) * 100.0).round() as i32
    } else {
        (value.round() - current) as i32
    };
    if delta != 0 {
        nudge(st, field, delta);
    }
}

pub fn nudge(st: &mut State, field: Field, delta: i32) {
    let crosshair = &mut st.crosshair.settings;
    match field {
        Field::Gap => crosshair.pip_gap = clamp_int(crosshair.pip_gap + delta, 0, 64),
        Field::Height => crosshair.pip_height = clamp_int(crosshair.pip_height + delta, 0, 64),
        Field::Width => crosshair.pip_width = clamp_int(crosshair.pip_width + delta, 0, 64),
        Field::OutlineBorder => {
            crosshair.pip_outline_border = clamp_int(crosshair.pip_outline_border + delta, 0, 16)
        }
        Field::OutlineGap => {
            crosshair.pip_outline_gap = clamp_int(crosshair.pip_outline_gap + delta, 0, 16)
        }
        Field::DotSize => crosshair.dot_size = clamp_int(crosshair.dot_size + delta, 0, 64),
        Field::DotBorder => {
            crosshair.dot_outline_border = clamp_int(crosshair.dot_outline_border + delta, 0, 16)
        }
        Field::DotGap => {
            crosshair.dot_outline_gap = clamp_int(crosshair.dot_outline_gap + delta, 0, 16)
        }
        Field::Opacity => {
            crosshair.pip_opacity = clamp_float(crosshair.pip_opacity + delta as f64 / 100.0)
        }
        Field::OutlineOpacity => {
            crosshair.pip_outline_opacity =
                clamp_float(crosshair.pip_outline_opacity + delta as f64 / 100.0)
        }
        Field::DotOpacity => {
            crosshair.dot_opacity = clamp_float(crosshair.dot_opacity + delta as f64 / 100.0)
        }
        Field::DotOutlineOpacity => {
            crosshair.dot_outline_opacity =
                clamp_float(crosshair.dot_outline_opacity + delta as f64 / 100.0)
        }
        Field::Red => crosshair.color_r = clamp_int(crosshair.color_r + delta, 0, 255),
        Field::Green => crosshair.color_g = clamp_int(crosshair.color_g + delta, 0, 255),
        Field::Blue => crosshair.color_b = clamp_int(crosshair.color_b + delta, 0, 255),
        Field::OutlineRed => {
            crosshair.outline_color_r = clamp_int(crosshair.outline_color_r + delta, 0, 255)
        }
        Field::OutlineGreen => {
            crosshair.outline_color_g = clamp_int(crosshair.outline_color_g + delta, 0, 255)
        }
        Field::OutlineBlue => {
            crosshair.outline_color_b = clamp_int(crosshair.outline_color_b + delta, 0, 255)
        }
    }
    st.crosshair.source = "edited".to_string();
}

fn clamp_int(value: i32, low: i32, high: i32) -> i32 {
    value.clamp(low, high)
}

fn clamp_float(value: f64) -> f64 {
    ((value * 100.0).round() / 100.0).clamp(0.0, 1.0)
}

fn swatches(u: &Ui, crosshair: &deadlock_core::crosshair::Crosshair) -> El {
    let colours = [
        ("green", 0, 255, 0),
        ("cyan", 0, 255, 255),
        ("white", 255, 255, 255),
        ("red", 255, 60, 60),
        ("amber", 255, 190, 0),
        ("magenta", 255, 60, 220),
    ];
    let mut row_el = row().gap(8.0).center_y();
    for (name, r, g, b) in colours {
        let selected = crosshair.color_r == r && crosshair.color_g == g && crosshair.color_b == b;
        // The ring sits outside the swatch so a selected colour keeps its full size.
        row_el = row_el.child(
            el().key(format!("swatch-{name}"))
                .size(28.0, 28.0)
                .radius(crate::ui::r(8.0))
                .border(1.5, if selected { u.ink } else { Color::TRANSPARENT })
                .center()
                .focusable(true)
                .role("button")
                .label(format!("{name} crosshair colour"))
                .checked(selected)
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    st.crosshair.settings.color_r = r;
                    st.crosshair.settings.color_g = g;
                    st.crosshair.settings.color_b = b;
                    st.crosshair.source = "edited".to_string();
                })
                .child(
                    el().size(20.0, 20.0)
                        .radius(5.0)
                        .fill(Color {
                            r: r as f32 / 255.0,
                            g: g as f32 / 255.0,
                            b: b as f32 / 255.0,
                            a: 1.0,
                        })
                        .a11y_hidden(),
                ),
        );
    }
    row_el
}

fn switch_row(u: &Ui, label: &str, on: bool, press: fn(&mut State)) -> El {
    setting_row(u, label).child(
        u.toggle(on)
            .key(format!("crosshair-switch-{label}"))
            .label(label.to_string())
            .cursor(rune::Cursor::Pointer)
            .on_press(press),
    )
}

/// The console commands the Apply step writes, selectable, with a one line copy for the
/// in-game console.
fn commands(s: &State, u: &Ui) -> El {
    let v = &s.crosshair;
    let done = copied(v, Copied::Console);
    col()
        .key("crosshair-commands")
        .w_full()
        .gap(8.0)
        .child(
            row()
                .w_full()
                .h(32.0)
                .center_y()
                .child(text(tr("Console")).font(H2).color(u.ink))
                .child(spacer())
                .child(if s.game_detected && s.game_root.is_some() {
                    u.icon_action(&icons::TRASH_2, tr("Remove from autoexec"), ActionKind::Ghost)
                        .key("crosshair-remove")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| st.crosshair.confirm_remove = true)
                } else {
                    el()
                })
                .child(
                    u.icon_action(
                        if done { &icons::CHECK } else { &icons::TERMINAL },
                        tr(if done { "Copied" } else { "Copy commands" }),
                        ActionKind::Secondary,
                    )
                    .key("crosshair-copy-console")
                    .cursor(rune::Cursor::Pointer)
                    .on_press_tx(|st: &mut State, tx: &mut Tx| copy_console(st, tx)),
                ),
        )
        .child(
            col()
                .w_full()
                .padding(12.0)
                .radius(crate::ui::r(8.0))
                .fill(u.well)
                .border(1.0, u.hair)
                .child(
                    text(v.settings.command_text())
                        .font(MONO)
                        .color(u.prose)
                        .wrap_anywhere()
                        .selectable(true),
                ),
        )
}

/// Mounted at the root so the apply approval overlays the shell.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    if s.crosshair.confirm_remove {
        return Some(remove_dialog(u));
    }
    if !s.crosshair.pending {
        return preset_dialog(s, u);
    }
    let crosshair = &s.crosshair.settings;
    let close = |st: &mut State| st.crosshair.pending = false;
    Some(
        u.dialog_shell()
            .key("crosshair-apply-dialog")
            .role("dialog")
            .label(tr("Apply crosshair"))
            .w(480.0)
            .padding(24.0)
            .gap(16.0)
            .modal(true, close)
            .on_key(move |st: &mut State, ev: &KeyEvent, _| {
                if ev.key == Key::Escape {
                    close(st);
                    true
                } else {
                    false
                }
            })
            .child(
                col()
                    .w_full()
                    .gap(6.0)
                    .child(text(tr("Apply crosshair")).font(H1).color(u.ink))
                    .child(
                        text(tr("Writes these settings to autoexec.cfg. The current file is backed up first."))
                            .font(BODY)
                            .color(u.muted)
                            .wrap(),
                    ),
            )
            .child(
                col()
                    .w_full()
                    .child(u.kv(tr("Lines"), &lines_summary(crosshair), u.ink))
                    .child(u.kv(tr("Dot"), &dot_summary(crosshair), u.ink))
                    .child(u.kv(
                        tr("Color"),
                        &format!(
                            "#{:02X}{:02X}{:02X}",
                            crosshair.color_r, crosshair.color_g, crosshair.color_b
                        ),
                        u.ink,
                    )),
            )
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .pt(8.0)
                    .child(spacer())
                    .child(u.action(tr("Cancel"), ActionKind::Ghost).cursor(rune::Cursor::Pointer)
        .on_press(close))
                    .child(
                        u.action(tr("Apply"), ActionKind::Primary)
                            .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| crate::data::apply_crosshair(st)),
                    ),
            ),
    )
}

fn lines_summary(c: &deadlock_core::crosshair::Crosshair) -> String {
    if c.pip_opacity <= 0.0 || c.pip_width <= 0 || c.pip_height <= 0 {
        return "Off".to_string();
    }
    format!(
        "Length {}, thickness {}, gap {}",
        c.pip_height, c.pip_width, c.pip_gap
    )
}

fn dot_summary(c: &deadlock_core::crosshair::Crosshair) -> String {
    if c.dot_opacity <= 0.0 || c.dot_size <= 0 {
        "Off".to_string()
    } else {
        format!("Size {}", c.dot_size)
    }
}

// --- presets ----------------------------------------------------------------------------

/// The side of a preset tile's art.
const TILE: f32 = 80.0;
/// Clear space kept between a mini reticle and its tile edge.
const TILE_MARGIN: f32 = 10.0;

fn presets_path() -> std::path::PathBuf {
    crate::data::data_dir().join(deadlock_core::crosshair::PRESETS_FILE)
}

/// Read the saved presets once. Headless and demo runs keep an empty list so snapshots do
/// not depend on the machine.
fn load_presets(st: &mut State) {
    st.crosshair.presets_loaded = true;
    if st.demo || rune::is_headless() {
        return;
    }
    match deadlock_core::crosshair::load_presets(&presets_path()) {
        Ok(presets) => st.crosshair.presets = presets,
        Err(error) => {
            st.crosshair.log = Some(format!("Could not read your saved presets: {error}"));
            st.crosshair.tone = Tone::Warn;
        }
    }
}

/// Write `next` and keep it only when the write worked, so the grid never shows a preset
/// that is not on disk. Headless and demo runs keep it in memory.
fn commit(st: &mut State, next: Vec<Preset>) -> Result<(), String> {
    if !st.demo && !rune::is_headless() {
        deadlock_core::crosshair::save_presets(&presets_path(), &next)
            .map_err(|error| format!("Could not save presets: {error}"))?;
    }
    st.crosshair.presets = next;
    Ok(())
}

/// Built-in presets leave the hero toggle as it is: it is a game behaviour, not a look.
fn load_builtin(st: &mut State, id: &'static str) {
    let Some(preset) = deadlock_core::crosshair::builtin_presets()
        .into_iter()
        .find(|preset| preset.id == id)
    else {
        return;
    };
    let keep = st.crosshair.settings.disable_hero_specific_crosshairs;
    st.crosshair.settings = Crosshair {
        disable_hero_specific_crosshairs: keep,
        ..preset.settings
    };
    st.crosshair.source = preset.name.to_string();
    st.crosshair.chosen = Some(Chosen::Builtin(id));
    st.crosshair.log = None;
    st.crosshair.tone = Tone::Quiet;
}

fn load_saved(st: &mut State, id: &str) {
    let Some(preset) = st.crosshair.presets.iter().find(|preset| preset.id == id) else {
        return;
    };
    st.crosshair.settings = preset.settings.clone();
    st.crosshair.source = preset.name.clone();
    st.crosshair.chosen = Some(Chosen::Saved(id.to_string()));
    st.crosshair.log = None;
    st.crosshair.tone = Tone::Quiet;
}

fn chosen_saved(v: &View) -> Option<&Preset> {
    match &v.chosen {
        Some(Chosen::Saved(id)) => v.presets.iter().find(|preset| &preset.id == id),
        _ => None,
    }
}

/// The section under the stage: built-in tiles, then a hairline and the player's own. Rename
/// and Delete act on the saved preset last loaded.
fn presets_section(s: &State, u: &Ui) -> El {
    let v = &s.crosshair;
    let mut head = row()
        .w_full()
        .h(32.0)
        .gap(8.0)
        .center_y()
        .child(text(tr("Presets")).font(H2).color(u.ink));
    head = head.child(spacer());
    if let Some(preset) = chosen_saved(v) {
        let rename_id = preset.id.clone();
        let delete_id = preset.id.clone();
        head = head
            .child(
                text(preset.name.clone())
                    .font(SMALL)
                    .color(u.muted)
                    .max_w(240.0)
                    .truncate(),
            )
            .child(
                u.icon_button(&icons::SQUARE_PEN, "Rename preset")
                    .key("crosshair-preset-rename")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| open_rename(st, &rename_id)),
            )
            .child(
                u.icon_button(&icons::TRASH_2, "Delete preset")
                    .key("crosshair-preset-delete")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        st.crosshair.preset_dialog = Some(PresetDialog::Delete(delete_id.clone()))
                    }),
            )
            .child(el().w(1.0).h(20.0).mx(4.0).fill(u.hair).a11y_hidden());
    }
    head = head.child(
        u.icon_action(&icons::PLUS, tr("Save current"), ActionKind::Secondary)
            .key("crosshair-preset-save")
            .label("Save the current crosshair as a preset")
            .cursor(rune::Cursor::Pointer)
            .on_press(open_save),
    );

    let mut grid = row()
        .key("crosshair-presets")
        .w_full()
        .flex_wrap()
        .gap(12.0)
        .role("list")
        .label("Crosshair presets");
    for preset in deadlock_core::crosshair::builtin_presets() {
        let id = preset.id;
        // Ringed whenever the editor matches it, so a reset or a pasted default shows too.
        let on = Crosshair {
                disable_hero_specific_crosshairs: v.settings.disable_hero_specific_crosshairs,
                ..preset.settings.clone()
            } == v.settings;
        grid = grid.child(
            tile(u, format!("crosshair-preset-{id}"), preset.name, &preset.settings, on)
                .on_press(move |st: &mut State| load_builtin(st, id)),
        );
    }
    if !v.presets.is_empty() {
        grid = grid.child(el().w(1.0).h(TILE).mx(4.0).fill(u.hair).a11y_hidden());
    }
    for preset in &v.presets {
        let id = preset.id.clone();
        let on = preset.settings == v.settings;
        grid = grid.child(
            tile(u, format!("crosshair-saved-{id}"), &preset.name, &preset.settings, on)
                .on_press(move |st: &mut State| load_saved(st, &id)),
        );
    }

    col().w_full().gap(12.0).pt(8.0).child(head).child(grid)
}

/// One preset: the reticle on the stage's dark ground, fitted to the tile, and its name.
fn tile(u: &Ui, key: String, name: &str, c: &Crosshair, on: bool) -> El {
    let art = el()
        .size(TILE, TILE)
        .radius(crate::ui::r(8.0))
        .clip_rounded(crate::ui::r(8.0))
        .fill(oklab_dark())
        .center()
        .child(mini_reticle(c))
        .child(
            el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .radius(crate::ui::r(8.0))
                .border(
                    if on { 1.5 } else { 1.0 },
                    if on { u.ink } else { u.hair },
                )
                .hover(Color::WHITE.alpha(0.05))
                .a11y_hidden(),
        );
    col()
        .key(key)
        .w(TILE)
        .gap(6.0)
        .focusable(true)
        .role("button")
        .label(format!("Load the {name} preset"))
        .checked(on)
        .cursor(rune::Cursor::Pointer)
        .child(art)
        .child(
            text(name.to_string())
                .font(SMALL)
                .color(if on { u.ink } else { u.muted })
                .w(TILE)
                .align(rune::Align::Center)
                .truncate()
                .a11y_hidden(),
        )
}

/// The reticle at its in-game size on a 1440p screen, shrunk only when it would not fit.
fn mini_reticle(c: &Crosshair) -> El {
    let reach = c.layout(1.0).reach * CAPTURE_H / 1080.0;
    let room = TILE / 2.0 - TILE_MARGIN;
    let zoom = if reach > room { room / reach } else { 1.0 };
    reticle(c, 1440, zoom, el().size(0.0, 0.0))
}

fn open_save(st: &mut State) {
    st.crosshair.preset_dialog = Some(PresetDialog::Name {
        editing: None,
        name: TextEdit::new(""),
        error: None,
    });
}

fn open_rename(st: &mut State, id: &str) {
    let Some(preset) = st.crosshair.presets.iter().find(|preset| preset.id == id) else {
        return;
    };
    st.crosshair.preset_dialog = Some(PresetDialog::Name {
        editing: Some(id.to_string()),
        name: TextEdit::new(&preset.name),
        error: None,
    });
}

fn open_import(st: &mut State) {
    st.crosshair.preset_dialog = Some(PresetDialog::Import {
        input: TextEdit::new(""),
        error: None,
    });
}

fn close_preset_dialog(st: &mut State) {
    st.crosshair.preset_dialog = None;
}

fn set_dialog_error(st: &mut State, message: String) {
    match st.crosshair.preset_dialog.as_mut() {
        Some(PresetDialog::Name { error, .. }) | Some(PresetDialog::Import { error, .. }) => {
            *error = Some(message)
        }
        _ => {}
    }
}

fn submit_name(st: &mut State) {
    let Some(PresetDialog::Name { editing, name, .. }) = st.crosshair.preset_dialog.as_ref() else {
        return;
    };
    let editing = editing.clone();
    let name = name.text.clone();
    let mut next = st.crosshair.presets.clone();
    let result = match &editing {
        Some(id) => deadlock_core::crosshair::rename_preset(&mut next, id, &name).map(|_| id.clone()),
        None => deadlock_core::crosshair::add_preset(&mut next, &name, &st.crosshair.settings),
    };
    let id = match result {
        Ok(id) => id,
        Err(error) => return set_dialog_error(st, error.to_string()),
    };
    if let Err(error) = commit(st, next) {
        return set_dialog_error(st, error);
    }
    if editing.is_none() {
        st.crosshair.chosen = Some(Chosen::Saved(id.clone()));
        if let Some(preset) = st.crosshair.presets.iter().find(|preset| preset.id == id) {
            st.crosshair.source = preset.name.clone();
        }
    }
    st.crosshair.preset_dialog = None;
}

fn confirm_delete(st: &mut State) {
    let Some(PresetDialog::Delete(id)) = st.crosshair.preset_dialog.as_ref() else {
        return;
    };
    let id = id.clone();
    let mut next = st.crosshair.presets.clone();
    deadlock_core::crosshair::remove_preset(&mut next, &id);
    st.crosshair.preset_dialog = None;
    match commit(st, next) {
        Ok(()) => {
            if st.crosshair.chosen == Some(Chosen::Saved(id)) {
                st.crosshair.chosen = None;
            }
        }
        Err(error) => {
            st.crosshair.log = Some(error);
            st.crosshair.tone = Tone::Bad;
        }
    }
}

fn submit_import(st: &mut State) {
    if st.crosshair.decode.is_running() {
        return;
    }
    let Some(PresetDialog::Import { input, .. }) = st.crosshair.preset_dialog.as_ref() else {
        return;
    };
    let pasted = input.text.clone();
    match deadlock_core::crosshair::parse_import(&pasted, &st.crosshair.settings) {
        Ok(Import::Settings(crosshair)) => {
            st.crosshair.settings = crosshair;
            st.crosshair.source = "pasted commands".to_string();
            st.crosshair.chosen = None;
            st.crosshair.preset_dialog = None;
            st.crosshair.log = Some("Imported the pasted commands.".to_string());
            st.crosshair.tone = Tone::Ok;
        }
        Ok(Import::Code(code)) => {
            if !st.prefs.allow_share_codes {
                return set_dialog_error(
                    st,
                    "Share codes are turned off in Settings, Privacy.".to_string(),
                );
            }
            if rune::is_headless() {
                return set_dialog_error(
                    st,
                    "Headless run: the share code was not decoded.".to_string(),
                );
            }
            if let Some(PresetDialog::Import { error, .. }) = st.crosshair.preset_dialog.as_mut() {
                *error = None;
            }
            let current = st.crosshair.settings.clone();
            let wake = st.wake.clone();
            st.crosshair.decode.start(wake, move || {
                deadlock_core::crosshair::decode_share_code(&code, &current)
                    .map_err(|error| error.to_string())
            });
        }
        Err(error) => set_dialog_error(st, error.to_string()),
    }
}

fn import_from_game(st: &mut State) {
    st.crosshair.preset_dialog = None;
    st.crosshair.chosen = None;
    game::import(st);
}

fn preset_dialog(s: &State, u: &Ui) -> Option<El> {
    match s.crosshair.preset_dialog.as_ref()? {
        PresetDialog::Name {
            editing,
            name,
            error,
        } => Some(name_dialog(u, editing.is_some(), name, error.as_deref())),
        PresetDialog::Delete(id) => {
            let name = s
                .crosshair
                .presets
                .iter()
                .find(|preset| &preset.id == id)
                .map(|preset| preset.name.clone())
                .unwrap_or_default();
            Some(delete_dialog(u, &name))
        }
        PresetDialog::Import { input, error } => Some(import_dialog(
            s,
            u,
            input,
            error.as_deref(),
        )),
    }
}

fn preset_shell(u: &Ui, key: &str, title: &str, body: El) -> El {
    u.dialog_shell()
        .key(key.to_string())
        .role("dialog")
        .label(title.to_string())
        .w(480.0)
        .padding(24.0)
        .gap(16.0)
        .modal(true, close_preset_dialog)
        .on_key(|st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                close_preset_dialog(st);
                true
            } else {
                false
            }
        })
        .child(text(title.to_string()).font(H1).color(u.ink))
        .child(body)
}

fn buttons(u: &Ui, leading: Option<El>, confirm: El) -> El {
    let mut r = row().w_full().gap(8.0).pt(8.0).center_y();
    if let Some(leading) = leading {
        r = r.child(leading);
    }
    r.child(spacer())
        .child(
            u.action("Cancel", ActionKind::Ghost)
                .key("crosshair-dialog-cancel")
                .cursor(rune::Cursor::Pointer)
                .on_press(close_preset_dialog),
        )
        .child(confirm)
}

fn bind_name(st: &mut State) -> &mut TextEdit {
    match st.crosshair.preset_dialog.as_mut() {
        Some(PresetDialog::Name { name, .. }) => name,
        _ => &mut st.crosshair.blank,
    }
}

fn bind_import(st: &mut State) -> &mut TextEdit {
    match st.crosshair.preset_dialog.as_mut() {
        Some(PresetDialog::Import { input, .. }) => input,
        _ => &mut st.crosshair.blank,
    }
}

fn name_dialog(u: &Ui, renaming: bool, name: &TextEdit, error: Option<&str>) -> El {
    let title = if renaming { "Rename preset" } else { "Save preset" };
    let ready = !name.text.trim().is_empty();
    let mut body = col().w_full().gap(8.0).child(
        row()
            .w_full()
            .h(32.0)
            .px(10.0)
            .center_y()
            .radius(crate::ui::r(8.0))
            .fill(u.well)
            .border(1.0, u.hair)
            .child(
                rune::text_input(name, BODY)
                    .key("crosshair-preset-name")
                    .label("Preset name")
                    .single_line()
                    .autofocus()
                    .grow(1.0)
                    .min_w(0.0)
                    .color(u.ink)
                    .placeholder("Name".to_string(), u.faint)
                    .caret(u.accent, u.accent.alpha(0.20))
                    .on_edit(bind_name)
                    .on_submit(|st: &mut State, _| submit_name(st)),
            ),
    );
    if let Some(error) = error {
        body = body.child(crate::notify::feedback::error_line(u, "crosshair-name-error", error));
    }
    let confirm = u
        .action(if renaming { "Rename" } else { "Save" }, ActionKind::Primary)
        .key("crosshair-preset-name-save")
        .disabled(!ready)
        .opacity(if ready { 1.0 } else { 0.45 })
        .cursor(rune::Cursor::Pointer)
        .on_press(submit_name);
    preset_shell(
        u,
        "crosshair-preset-name-dialog",
        title,
        col().w_full().gap(16.0).child(body).child(buttons(u, None, confirm)),
    )
}

fn delete_dialog(u: &Ui, name: &str) -> El {
    let confirm = u
        .action("Delete", ActionKind::Danger)
        .key("crosshair-preset-delete-confirm")
        .cursor(rune::Cursor::Pointer)
        .on_press(confirm_delete);
    preset_shell(
        u,
        "crosshair-preset-delete-dialog",
        "Delete preset",
        col()
            .w_full()
            .gap(16.0)
            .child(
                text(format!("{name} is removed from your presets."))
                    .font(BODY)
                    .color(u.muted)
                    .wrap(),
            )
            .child(buttons(u, None, confirm)),
    )
}

fn import_dialog(s: &State, u: &Ui, input: &TextEdit, error: Option<&str>) -> El {
    let busy = s.crosshair.decode.is_running();
    let ready = !input.text.trim().is_empty() && !busy;
    let mut body = col()
        .w_full()
        .gap(8.0)
        .child(
            text("Paste a share code or the console commands.")
                .font(BODY)
                .color(u.muted)
                .wrap(),
        )
        .child(
            col()
                .w_full()
                .h(96.0)
                .px(12.0)
                .py(10.0)
                .radius(crate::ui::r(8.0))
                .fill(u.well)
                .border(1.0, u.hair)
                .child(
                    rune::text_input(input, MONO)
                        .key("crosshair-import-input")
                        .label("Share code or console commands")
                        .autofocus()
                        .w_full()
                        .h_full()
                        .color(u.ink)
                        .placeholder("DL.… or citadel_crosshair_pip_gap 5; …".to_string(), u.faint)
                        .caret(u.accent, u.accent.alpha(0.20))
                        .on_edit(bind_import),
                ),
        );
    if let Some(error) = error {
        body = body.child(crate::notify::feedback::error_line(u, "crosshair-import-error", error));
    }
    let from_game = u
        .action("Read from game", ActionKind::Secondary)
        .key("crosshair-import-game")
        .cursor(rune::Cursor::Pointer)
        .on_press(import_from_game);
    let confirm = u
        .action(if busy { "Importing" } else { "Import" }, ActionKind::Primary)
        .key("crosshair-import-confirm")
        .disabled(!ready)
        .opacity(if ready { 1.0 } else { 0.45 })
        .cursor(rune::Cursor::Pointer)
        .on_press(submit_import);
    preset_shell(
        u,
        "crosshair-import-dialog",
        "Import crosshair",
        col()
            .w_full()
            .gap(16.0)
            .child(body)
            .child(buttons(u, Some(from_game), confirm)),
    )
}

// --- scenes ------------------------------------------------------------------------------

fn scene_saved(s: &mut State) {
    s.screen = Screen::Crosshair;
    s.crosshair.presets_loaded = true;
    let mut presets = Vec::new();
    let mut tight = Crosshair::default();
    tight.pip_gap = 1;
    tight.pip_height = 7;
    tight.color_r = 255;
    tight.color_g = 255;
    tight.color_b = 255;
    let mut ring = Crosshair::default();
    ring.pip_opacity = 0.0;
    ring.pip_outline_opacity = 0.0;
    ring.dot_size = 4;
    ring.dot_opacity = 1.0;
    ring.dot_outline_border = 2;
    ring.dot_outline_gap = 4;
    ring.dot_outline_opacity = 1.0;
    ring.color_r = 255;
    ring.color_g = 190;
    ring.color_b = 0;
    ring.outline_color_r = 255;
    ring.outline_color_g = 190;
    ring.outline_color_b = 0;
    for (name, settings) in [("Tight white", tight), ("Ranked", Crosshair::default()), ("Ring", ring)] {
        let _ = deadlock_core::crosshair::add_preset(&mut presets, name, &settings);
    }
    s.crosshair.presets = presets;
}

/// Saved presets beside the built-in ones, one of them loaded.
pub fn scene_presets(s: &mut State) {
    scene_saved(s);
    if let Some(id) = s.crosshair.presets.first().map(|preset| preset.id.clone()) {
        load_saved(s, &id);
    }
}

/// The Save preset dialog with a name typed.
pub fn scene_preset_name(s: &mut State) {
    scene_saved(s);
    load_builtin(s, "wide");
    s.crosshair.preset_dialog = Some(PresetDialog::Name {
        editing: None,
        name: TextEdit::new("Ranked"),
        error: Some("A preset named Ranked already exists.".to_string()),
    });
}

/// The Import dialog with a paste that has a bad value.
pub fn scene_import(s: &mut State) {
    scene_saved(s);
    let pasted = "citadel_crosshair_pip_gap 4; citadel_crosshair_pip_opacity 2";
    let error = deadlock_core::crosshair::parse_import(pasted, &s.crosshair.settings)
        .err()
        .map(|error| error.to_string());
    s.crosshair.preset_dialog = Some(PresetDialog::Import {
        input: TextEdit::new(pasted),
        error,
    });
}

// --- remove from autoexec -----------------------------------------------------------------

/// The "Remove from autoexec" confirm (Grimoire's clear active crosshair).
fn remove_dialog(u: &Ui) -> El {
    let close = |st: &mut State| st.crosshair.confirm_remove = false;
    u.dialog_shell()
        .key("crosshair-remove-dialog")
        .role("dialog")
        .label(tr("Remove the crosshair from autoexec?"))
        .w(460.0)
        .padding(24.0)
        .gap(16.0)
        .modal(true, close)
        .on_key(move |st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                close(st);
                true
            } else {
                false
            }
        })
        .child(
            col()
                .w_full()
                .gap(6.0)
                .child(text(tr("Remove the crosshair from autoexec?")).font(H1).color(u.ink))
                .child(
                    text(tr("Takes Sigil's crosshair out of autoexec.cfg. Every other line stays, and the file is backed up first."))
                        .font(BODY)
                        .color(u.muted)
                        .wrap(),
                ),
        )
        .child(
            row()
                .w_full()
                .gap(8.0)
                .pt(8.0)
                .child(spacer())
                .child(
                    u.action(tr("Cancel"), ActionKind::Ghost)
                        .key("crosshair-remove-cancel")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(close),
                )
                .child(
                    u.action(tr("Remove"), ActionKind::Danger)
                        .key("crosshair-remove-confirm")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(remove_from_autoexec),
                ),
        )
}

/// GAME WRITE, from the confirm: `deadlock_core::crosshair::remove_from_install` takes the
/// block out of autoexec.cfg with a backup. The toast's Undo puts the old file back.
fn remove_from_autoexec(st: &mut State) {
    st.crosshair.confirm_remove = false;
    let Some(game_root) = st.game_root.clone() else {
        st.crosshair.log = Some(tr("No Deadlock install was found. Nothing was written.").to_string());
        st.crosshair.tone = Tone::Bad;
        return;
    };
    if st.demo || rune::is_headless() {
        st.crosshair.log = Some(tr("Removed the crosshair from autoexec.cfg.").to_string());
        st.crosshair.tone = Tone::Ok;
        return;
    }
    match deadlock_core::crosshair::remove_from_install(&game_root) {
        Ok(Some(result)) => {
            st.crosshair.log = Some(tr("Removed the crosshair from autoexec.cfg.").to_string());
            st.crosshair.tone = Tone::Ok;
            st.autoexec.loaded = false;
            let notice = crate::notify::Notice::success(tr("Crosshair removed from autoexec"))
                .detail(tr("The game uses its own crosshair settings on the next start."));
            let notice = match result.backup {
                Some(backup) => notice.undo(move |st: &mut State| undo_remove(st, backup)),
                None => notice,
            };
            crate::notify::push(st, notice);
        }
        Ok(None) => {
            st.crosshair.log = Some(tr("autoexec.cfg has no crosshair from Sigil. Nothing changed.").to_string());
            st.crosshair.tone = Tone::Quiet;
        }
        Err(error) => {
            st.crosshair.log = Some(error.to_string());
            st.crosshair.tone = Tone::Bad;
        }
    }
}

/// GAME WRITE, from the remove toast's Undo: put back the autoexec.cfg the removal copied aside.
fn undo_remove(st: &mut State, backup: std::path::PathBuf) {
    use crate::notify::{self, Notice};
    match deadlock_core::backups::restore(&backup, true) {
        Ok(_) => {
            st.crosshair.log = Some(tr("Put the previous autoexec.cfg back.").to_string());
            st.crosshair.tone = Tone::Ok;
            st.autoexec.loaded = false;
            notify::push(st, Notice::info(tr("Put the previous autoexec.cfg back")));
        }
        Err(error) => notify::push(
            st,
            Notice::error(tr("Could not undo the removal")).detail(error.to_string()),
        ),
    }
}

// --- scenes (wave) ------------------------------------------------------------------------

/// The Crosshair page against a stand-in install, so the install actions show.
fn scene_base(s: &mut State) {
    crate::app::scene_crosshair(s);
    s.game_detected = true;
    s.game_root = Some(std::path::PathBuf::from("/nonexistent/Deadlock"));
    s.crosshair.log = None;
}

/// `crosshair-pinned`: the window pinned above other windows.
pub fn scene_pinned(s: &mut State) {
    scene_base(s);
    crate::shell::set_always_on_top(s, true);
}

/// `crosshair-colour`: the Color group with a custom colour and outline colour.
pub fn scene_colour(s: &mut State) {
    scene_base(s);
    s.crosshair.group = Group::Colour;
    s.crosshair.settings.color_r = 120;
    s.crosshair.settings.color_g = 200;
    s.crosshair.settings.color_b = 255;
    s.crosshair.settings.outline_color_r = 20;
    s.crosshair.settings.outline_color_g = 20;
    s.crosshair.settings.outline_color_b = 60;
    s.crosshair.source = "edited".to_string();
}

/// `crosshair-remove`: the Remove from autoexec confirm.
pub fn scene_remove(s: &mut State) {
    scene_base(s);
    s.crosshair.confirm_remove = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_reticle_fits_its_tile() {
        for preset in deadlock_core::crosshair::builtin_presets() {
            let reach = preset.settings.layout(1.0).reach * CAPTURE_H / 1080.0;
            let room = TILE / 2.0 - TILE_MARGIN;
            let zoom = if reach > room { room / reach } else { 1.0 };
            assert!(reach * zoom <= room + 0.01, "{}", preset.name);
        }
    }
}
