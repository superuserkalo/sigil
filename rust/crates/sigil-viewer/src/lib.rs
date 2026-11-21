//! Sigil's 3D viewer renderer: a player's hero as the game draws it, with their mods.
//!
//! An offscreen wgpu renderer on its own device. It loads the `.glb` heroes vpkmerge's morphic
//! exports (skeleton, skinned LOD0 meshes, PBR materials with the `extras.morphic` Source 2
//! payload, the model's clips), skins them on the GPU, plays a clip by name with looping and
//! seek, and lights them with Deadlock's Citadel NPR model as Grimoire's viewer ports it
//! (stepped wrapped diffuse, exposure-normalized ambient bands, the silhouette rim with its
//! depth test, inverted-hull outlines, the hero inspect post curve). `render` returns sRGB
//! RGBA8 rows; the caller owns presenting them. It never picks and never touches the UI.
//!
//! `Viewer::render` is meant for a dedicated thread: it blocks on the GPU. Model parsing
//! (`model::load_path`) is plain CPU work and can run anywhere.

pub mod camera;
pub mod capture;
pub mod clearance;
pub mod cloth;
pub mod fixture;
pub mod fixture_room;
pub mod gpu;
pub mod hideout;
pub mod material;
pub mod math;
pub mod model;
pub mod particles;
pub mod scene;
pub mod shaders;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub use camera::OrbitCamera;
pub use gpu::{GpuError, Timings, Usage};
pub use model::Model;
pub use scene::{Playmode, SceneDesc, Sidecar};

use gpu::{FloorGpu, FrameGpu, FrameInputs, FrameResources, Gpu, GpuMesh, LightGpu, Pipelines, PostGpu, Shown, Targets};
use math::{Aabb, Mat4, Vec3, normalize};

#[derive(Debug, Clone, PartialEq)]
pub enum ViewerError {
    /// No GPU adapter: show a still image instead.
    NoGpu(String),
    Device(String),
    Load(String),
    /// The caller cancelled the load.
    Cancelled,
}

impl std::fmt::Display for ViewerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ViewerError::NoGpu(e) => write!(f, "no GPU adapter: {e}"),
            ViewerError::Device(e) => write!(f, "GPU device: {e}"),
            ViewerError::Load(e) => f.write_str(e),
            ViewerError::Cancelled => f.write_str("cancelled"),
        }
    }
}

impl std::error::Error for ViewerError {}

impl From<GpuError> for ViewerError {
    fn from(e: GpuError) -> Self {
        match e {
            GpuError::NoAdapter(s) => ViewerError::NoGpu(s),
            GpuError::Device(s) => ViewerError::Device(s),
        }
    }
}

/// Which model shows: the modded hero, the stock one, or both split at a fraction of the
/// width (stock on the left of the line, modded on the right).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Compare {
    After,
    Before,
    Split(f32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// Leave the backdrop out (alpha), for a capture to paste elsewhere.
    pub transparent: bool,
    pub bloom: bool,
    pub floor: bool,
    /// The key light casts a shadow map (studio key, or the Hideout's nearest realtime
    /// directional or spot light).
    pub shadows: bool,
    /// What fills the frame behind the hero (a scene's sky always wins).
    pub backdrop: Backdrop,
    /// Grimoire's lighting scenes for the studio (a Hideout brings its own).
    pub lighting: Lighting,
    /// Simulate the hero's cloth (Grimoire's Cloth switch, on by default).
    pub cloth: bool,
    /// Play the hero's and the map's particle effects (Grimoire's Particles switch).
    pub particles: bool,
    /// March the Hideout's volumetric fog at half resolution and upsample it by depth
    /// (Grimoire's default); off marches it per surface fragment.
    pub half_res_fog: bool,
}

impl Default for Look {
    fn default() -> Self {
        Look { transparent: false, bloom: true, floor: true, shadows: true, backdrop: Backdrop::Studio, lighting: Lighting::Studio, cloth: true, particles: true, half_res_fog: true }
    }
}

/// The studio backdrop.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Backdrop {
    /// Sigil's charcoal with an ember glow behind the hero.
    #[default]
    Studio,
    /// One of `GRADIENTS`.
    Gradient(usize),
    /// Grimoire's None: the flat viewer grey.
    Plain,
    /// A picture the player chose (`Viewer::set_backdrop_image`), cover cropped.
    Image,
}

/// Grimoire's lighting scenes (HeroPoseViewer.tsx): the studio rig, or Midtown dusk's warm
/// key, brighter environment and higher exposure.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Lighting {
    #[default]
    Studio,
    Midtown,
}

/// A backdrop gradient: sRGB stops at 0, `mid` and 1, radial from `center` over
/// `spread` times the frame's diagonal (or vertical when `center` is None), then a vignette.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gradient {
    pub name: &'static str,
    pub center: Option<[f32; 2]>,
    pub spread: f32,
    pub stops: [[f32; 3]; 3],
    pub mid: f32,
    pub vignette: f32,
}

const fn hex(rgb: u32) -> [f32; 3] {
    [((rgb >> 16) & 255) as f32 / 255.0, ((rgb >> 8) & 255) as f32 / 255.0, (rgb & 255) as f32 / 255.0]
}

const fn radial(name: &'static str, cx: f32, cy: f32, spread: f32, stops: [u32; 3], mid: f32) -> Gradient {
    Gradient { name, center: Some([cx, cy]), spread, stops: [hex(stops[0]), hex(stops[1]), hex(stops[2])], mid, vignette: 0.5 }
}

/// Deadlock Mod Manager's studio gradient, then Grimoire's ten soul backdrops
/// (soulBackdrops.ts), in their order.
pub const GRADIENTS: [Gradient; 11] = [
    Gradient { name: "Studio gradient", center: None, spread: 1.0, stops: [hex(0x16171b), hex(0x101114), hex(0x0a0b0d)], mid: 0.5, vignette: 0.0 },
    radial("Charcoal spotlight", 0.5, 0.42, 0.78, [0x34343a, 0x18181b, 0x0a0a0b], 0.5),
    radial("Warm ember", 0.5, 0.88, 0.85, [0x5e3717, 0x2a160a, 0x080605], 0.4),
    radial("Cool abyss blue", 0.5, 0.4, 0.78, [0x1f3a58, 0x0d1a2a, 0x05080d], 0.5),
    radial("Violet soul", 0.5, 0.42, 0.78, [0x3d2160, 0x1c0f30, 0x080510], 0.5),
    radial("Deep forest", 0.5, 0.46, 0.78, [0x17402e, 0x0b1f17, 0x050b08], 0.5),
    radial("Crimson dusk", 0.5, 0.46, 0.78, [0x5e1a22, 0x2a0d12, 0x0c0608], 0.45),
    radial("Teal nebula", 0.42, 0.38, 0.78, [0x0f5050, 0x0a2626, 0x04090a], 0.45),
    radial("Amber halo", 0.5, 0.5, 0.78, [0x7a4410, 0x3a2208, 0x0b0805], 0.42),
    Gradient { name: "Indigo-to-rose dawn", center: None, spread: 1.0, stops: [hex(0x241a3a), hex(0x1a0f1e), hex(0x08060a)], mid: 0.55, vignette: 0.4 },
    radial("Ice steel", 0.5, 0.36, 0.78, [0x2c3a44, 0x141a1f, 0x070a0c], 0.5),
];

/// Grimoire's turntable: the hero turns at a quarter radian a second.
pub const SPIN_SPEED: f32 = 0.25;

/// A directional light's shadow camera: an orthographic box around a sphere of `radius` at
/// `center`, seen from `to_light`.
pub fn directional_shadow(to_light: Vec3, center: Vec3, radius: f32) -> Mat4 {
    let d = normalize(to_light);
    let up = if d[1].abs() > 0.95 { [0.0, 0.0, 1.0] } else { [0.0, 1.0, 0.0] };
    let eye = math::add(center, math::scale(d, radius * 2.0));
    let view = Mat4::look_at(eye, center, up);
    Mat4::orthographic(radius, radius, 0.0, radius * 4.0).mul(&view)
}

/// A spot light's shadow camera: its cone from its position, out to its range.
pub fn spot_shadow(position: Vec3, shine: Vec3, outer: f32, range: f32) -> Mat4 {
    let d = normalize(shine);
    let up = if d[1].abs() > 0.95 { [0.0, 0.0, 1.0] } else { [0.0, 1.0, 0.0] };
    let view = Mat4::look_at(position, math::add(position, d), up);
    let fov = (outer * 2.0 + 0.05).clamp(0.2, 2.8);
    Mat4::perspective(fov, 1.0, 0.05, range.max(1.0)).mul(&view)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Playback {
    pub playing: bool,
    pub speed: f32,
    /// Seconds into the clip.
    pub time: f32,
}

impl Default for Playback {
    fn default() -> Self {
        Playback { playing: true, speed: 1.0, time: 0.0 }
    }
}

/// What a loaded hero offers the controls.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeroInfo {
    pub clips: Vec<(String, f32)>,
    pub clip: Option<String>,
    pub triangles: u32,
    pub bounds: Option<Aabb>,
}

struct Slot {
    /// Shared by every placement of the same file.
    model: Arc<Model>,
    gpu: gpu::GpuModel,
    placement: Mat4,
    clip: Option<usize>,
    palette: Vec<Mat4>,
    bases: Vec<u32>,
    unit: f32,
    /// A scene prop's bounds, measured once: props never move, and measuring hundreds of them
    /// every frame (CPU skinning of every vertex) froze the Hideout.
    fixed_bounds: Option<Aabb>,
    /// A still prop is posed once.
    still: bool,
    posed: bool,
    /// The node matrices of the last pose (the glb's frame), for attachments.
    globals: Vec<Mat4>,
    /// The hero's cloth (Grimoire's solver), stepped from the clip's own clock; off when the
    /// player switches cloth off.
    cloth: Option<cloth::Cloth>,
    cloth_time: f32,
    cloth_on: bool,
}

impl Slot {
    fn new(gpu: &Gpu, p: &Pipelines, model: Arc<Model>, placement: Mat4, clip: Option<usize>, tint: [f32; 4], only: Option<usize>) -> Slot {
        let mesh = GpuMesh::new(gpu, p, &model, "the hero");
        Slot::placed(gpu, p, model, &mesh, placement, clip, tint, only)
    }

    #[allow(clippy::too_many_arguments)]
    fn placed(gpu: &Gpu, p: &Pipelines, model: Arc<Model>, mesh: &GpuMesh, placement: Mat4, clip: Option<usize>, tint: [f32; 4], only: Option<usize>) -> Slot {
        let locals = model::sample_pose(&model, clip.and_then(|c| model.clips.get(c)), 0.0);
        let globals = model::globals(&model, &locals);
        let mut palette = Vec::new();
        let bases = model::palettes(&model, &globals, &placement, &mut palette);
        let unit = model::game_unit(&model, &globals) * placement.scale_factor();
        let g = mesh.instance(gpu, p, palette.len(), &bases, tint, only);
        Slot { model, gpu: g, placement, clip, palette, bases, unit, fixed_bounds: None, still: false, posed: false, globals, cloth: None, cloth_time: 0.0, cloth_on: true }
    }

    /// A scene prop: bounds measured now (only its fragment when `only`), posed once when still.
    fn prop(mut self, only: Option<usize>) -> Slot {
        let visible: Vec<bool> = self.model.primitives.iter().map(|p| !p.authored_shell && only.is_none_or(|i| i == p.index_in_mesh)).collect();
        self.fixed_bounds = Some(model::posed_bounds(&self.model, &self.palette, &self.bases, |i| visible[i]));
        self.still = self.clip.is_none();
        self
    }

    fn duration(&self) -> f32 {
        self.clip.and_then(|c| self.model.clips.get(c)).map_or(0.0, |c| c.duration)
    }

    fn pose(&mut self, gpu: &Gpu, time: f32) {
        if self.still && self.posed {
            return;
        }
        self.posed = true;
        let clip = self.clip.and_then(|c| self.model.clips.get(c));
        self.globals = match self.cloth.as_mut().filter(|_| self.cloth_on) {
            Some(cloth) => {
                // The animation advances inside the solver's substeps (Grimoire's driver),
                // by the clip time that passed, wrapped at the loop.
                let d = clip.map_or(0.0, |c| c.duration);
                let mut dt = time - self.cloth_time;
                if d > 0.0 && dt < 0.0 {
                    dt += d;
                }
                let (model, mut t) = (&self.model, self.cloth_time);
                cloth.set_placement(&self.placement);
                cloth.step(dt.min(0.1), &mut |sub| {
                    t += sub;
                    model::sample_pose(model, clip, if d > 0.0 { t.rem_euclid(d) } else { t })
                });
                self.cloth_time = time;
                cloth.pose()
            }
            None => model::globals(&self.model, &model::sample_pose(&self.model, clip, time)),
        };
        self.bases = model::palettes(&self.model, &self.globals, &self.placement, &mut self.palette);
        self.gpu.write_palette(gpu, &self.palette);
    }

    /// The cloth starts over from the pose at `time` (a clip change or a seek).
    fn reset_cloth(&mut self, time: f32) {
        if let Some(cloth) = &mut self.cloth {
            let clip = self.clip.and_then(|c| self.model.clips.get(c));
            cloth.reset(&model::sample_pose(&self.model, clip, time));
        }
        self.cloth_time = time;
    }

    /// The glb's Source-to-glTF node (`skeleton`) as posed, so Source-space effects can follow.
    fn skeleton(&self) -> Mat4 {
        self.model.nodes.iter().position(|n| n.name == "skeleton").and_then(|i| self.globals.get(i).copied()).unwrap_or_else(fixture::source_to_gltf)
    }

    fn bounds(&self) -> Aabb {
        if let Some(b) = self.fixed_bounds {
            return b;
        }
        let visible: Vec<bool> = self.model.primitives.iter().map(|p| !p.authored_shell).collect();
        model::posed_bounds(&self.model, &self.palette, &self.bases, |i| visible[i])
    }
}

/// The preview rig, Grimoire's studio: a white key from the upper front left, a cool fill
/// from behind right, both scaled by its calibration anchor over the exposure.
pub const STUDIO_EXPOSURE: f32 = 1.3;
const RIG_EXPOSURE: f32 = 0.284_933_4;
const KEY: Vec3 = [-3.0, 4.0, 3.0];
const FILL: Vec3 = [3.0, 2.0, -3.0];
const SKY: Vec3 = [96.0 / 255.0, 135.0 / 255.0, 183.0 / 255.0];
const GROUND: Vec3 = [0.32, 0.25, 0.2];
const ENV_TOP: Vec3 = [0.92, 0.92, 0.95];
const ENV_BOTTOM: Vec3 = [0.16, 0.145, 0.135];

/// A CPU twin of `environment()` in scene.wgsl, for the ambient cube.
fn environment(dir: Vec3, top: Vec3, bottom: Vec3, key: Vec3, k: f32) -> Vec3 {
    let blur = 1.0f32;
    let smooth = |e0: f32, e1: f32, x: f32| {
        let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let t = smooth(-0.35 - blur, 0.55 + blur, dir[1]);
    let mut c = math::lerp3(bottom, top, t);
    let d = math::dot(dir, key).max(0.0).powf(2.0) * 0.6;
    c = math::add(c, math::scale([1.0, 0.97, 0.92], d));
    math::scale(c, k)
}

/// Diffuse irradiance per world axis (+X, -X, +Y, -Y, +Z, -Z): the cosine-weighted studio plus
/// the hemisphere light, as `viewerAmbientCube` builds it.
fn ambient_cube(top: Vec3, bottom: Vec3, key: Vec3, env_k: f32, hemi_k: f32) -> [Vec3; 6] {
    let axes: [Vec3; 6] = [[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, -1.0]];
    axes.map(|n| {
        let mut sum = [0.0f32; 3];
        let mut wsum = 0.0;
        for i in 0..16 {
            for j in 0..32 {
                let th = (i as f32 + 0.5) / 16.0 * std::f32::consts::PI;
                let ph = (j as f32 + 0.5) / 32.0 * std::f32::consts::TAU;
                let d = [th.sin() * ph.cos(), th.cos(), th.sin() * ph.sin()];
                let w = math::dot(d, n).max(0.0) * th.sin();
                if w > 0.0 {
                    sum = math::add(sum, math::scale(environment(d, top, bottom, key, env_k), w));
                    wsum += w;
                }
            }
        }
        let env = math::scale(sum, 1.0 / wsum.max(1e-6));
        let hemi = math::scale(math::lerp3(GROUND, SKY, n[1] * 0.5 + 0.5), hemi_k);
        math::add(env, hemi)
    })
}

fn luminance_fit(cube: &[Vec3; 6]) -> [f32; 4] {
    let l: Vec<f32> = cube.iter().map(|c| c[0] * 0.2125 + c[1] * 0.7154 + c[2] * 0.0721).collect();
    let mean = l.iter().sum::<f32>() / 6.0;
    [(l[0] - l[1]) / 2.0, (l[2] - l[3]) / 2.0, (l[4] - l[5]) / 2.0, mean]
}

/// The hero inspect post volume (`postprocessing/caldera.vpost`): VRF's default curve with a
/// 3.9996-stop white point and a bloom of 0.888 over a threshold of 4.
/// `postprocessing/basepostprocess_deadlock.vpost`, the curve the Hideout's volume starts from
/// (Grimoire's DEADLOCK_BASE_POST): used for a scene, whose own tone curve is an omission.
fn base_post(exposure: f32) -> PostGpu {
    let (a, b, c, d, e, f) = (0.3538f32, 0.3258, 0.2528, 0.6966, 0.0, 0.7819);
    let curve = |x: f32| (x * (a * x + c * b) + d * e) / (x * (a * x + b) + d * f) - e / f;
    let white = 2f32.powf(3.9996);
    let mut p = hero_post(exposure * 2f32.powf(0.8992));
    p.exposure[2] = 1.0 / curve(white);
    p.curve_a = [a, b, c, d];
    p.curve_b = [e, f, 0.5 * 0.2, 1.0];
    p
}

fn hero_post(exposure: f32) -> PostGpu {
    let (a, b, c, d, e, f) = (0.15f32, 0.5, 0.1, 0.2, 0.02, 0.3);
    let curve = |x: f32| (x * (a * x + c * b) + d * e) / (x * (a * x + b) + d * f) - e / f;
    let white = 2f32.powf(3.9996);
    PostGpu {
        exposure: [exposure, white, 1.0 / curve(white), 1.0],
        curve_a: [a, b, c, d],
        curve_b: [e, f, 0.888 * 0.2, 4.0],
        split: [-1.0, 1.0, 0.0, 0.0],
        // Sigil's studio backdrop: charcoal with an ember glow behind the hero.
        top: [0.026, 0.025, 0.028, 0.0],
        bottom: [0.006, 0.006, 0.007, 0.0],
        glow: [0.075, 0.042, 0.026, 1.0],
        glow_at: [0.5, 0.42, 0.42, 0.5],
        inv_view_proj: Mat4::IDENTITY.to_cols_array(),
        sky: [0.0; 4],
        bloom2: [0.0; 4],
        lut: [0.0; 4],
        bloom_w: [[1.0, 1.0, 1.0, 0.0]; 5],
        bg: [0.0; 4],
        bg_at: [0.5, 0.5, 1.0, 1.0],
        bg0: [0.0; 4],
        bg1: [0.0; 4],
        bg2: [0.0; 4],
    }
}

/// A scene's post: its own tone curve, exposure bias and bloom when the map's post volume
/// gives them (VRF's PostProcessRenderer as Grimoire ports it), else the base Deadlock curve.
fn map_post(exposure: f32, post: &scene::Post) -> (PostGpu, [[f32; 3]; 5]) {
    let mut p = base_post(exposure);
    let mut weights = [[1.0f32; 3]; 5];
    if let Some(t) = post.tonemap {
        let (a, b, c, d, e, f) = (t.shoulder, t.linear_strength, t.linear_angle, t.toe, t.toe_num, t.toe_denom.max(1e-4));
        let curve = |x: f32| (x * (a * x + c * b) + d * e) / (x * (a * x + b) + d * f) - e / f;
        let white = 2f32.powf(t.white_point);
        p.exposure = [exposure * 2f32.powf(t.exposure_bias), white, 1.0 / curve(white).max(1e-4), 1.0];
        p.curve_a = [a, b, c, d];
        p.curve_b[0] = e;
        p.curve_b[1] = f;
    }
    if let Some(bl) = post.bloom {
        let max = bl.add.max(bl.screen).max(bl.blur);
        if max > 0.0 {
            let mode = match bl.mode {
                scene::BloomMode::Add => 0.0,
                scene::BloomMode::Screen => 1.0,
                scene::BloomMode::Blur => 2.0,
            };
            p.curve_b[2] = bl.add / max;
            p.curve_b[3] = bl.threshold;
            p.bloom2 = [mode, bl.screen / max, bl.blur / max, bl.width.max(1e-3)];
            p.lut[2] = max;
            for (i, w) in weights.iter_mut().enumerate() {
                *w = bl.tints[i].map(|t| t * bl.weights[i]);
            }
            for (i, slot) in p.bloom_w.iter_mut().enumerate() {
                *slot = [weights[i][0], weights[i][1], weights[i][2], 0.0];
            }
            p.bloom_w[0][3] = 5.0;
        } else {
            p.curve_b[2] = 0.0;
        }
    }
    (p, weights)
}

/// A scene cube read and resampled on the CPU: square faces in glTF order with box mips, and
/// each face's mean (the ambient it gives that side).
#[derive(Clone, Debug)]
pub struct CubeData {
    pub levels: Vec<(u32, Vec<[f32; 4]>)>,
    pub face_means: [Vec3; 6],
    pub scale: Vec3,
}

const CUBE_EDGE: u32 = 128;

/// Reads six Radiance faces into a `CubeData` (any thread).
pub fn load_cube(cube: &scene::Cube) -> Result<CubeData, String> {
    let mut faces = Vec::with_capacity(6);
    for path in &cube.faces {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Hdr).map_err(|e| format!("{}: {e}", path.display()))?.into_rgb32f();
        let (w, h) = img.dimensions();
        let data = img.into_raw();
        // Nearest resample to a fixed square edge: probes are blurry by nature.
        let edge = CUBE_EDGE.min(w.max(1));
        let mut out = Vec::with_capacity((edge * edge) as usize);
        for y in 0..edge {
            for x in 0..edge {
                let sx = ((x as f32 + 0.5) / edge as f32 * w as f32) as usize;
                let sy = ((y as f32 + 0.5) / edge as f32 * h as f32) as usize;
                let i = (sy.min(h as usize - 1) * w as usize + sx.min(w as usize - 1)) * 3;
                out.push([data[i], data[i + 1], data[i + 2], 1.0]);
            }
        }
        faces.push((edge, out));
    }
    let edge = faces.iter().map(|f| f.0).min().unwrap_or(1);
    if faces.iter().any(|f| f.0 != edge) {
        return Err("cube faces differ in size".into());
    }
    let face_means: [Vec3; 6] = std::array::from_fn(|i| {
        let f = &faces[i].1;
        let n = f.len().max(1) as f32;
        let s = f.iter().fold([0.0f32; 3], |a, t| [a[0] + t[0], a[1] + t[1], a[2] + t[2]]);
        [s[0] / n * cube.scale[0], s[1] / n * cube.scale[1], s[2] / n * cube.scale[2]]
    });
    let mut levels = vec![(edge, faces.iter().flat_map(|f| f.1.iter().copied()).collect::<Vec<_>>())];
    while levels.last().is_some_and(|l| l.0 > 1) {
        let (e, src) = levels.last().unwrap();
        let ne = e / 2;
        let mut next = Vec::with_capacity((6 * ne * ne) as usize);
        for face in 0..6 {
            let base = (face * e * e) as usize;
            for y in 0..ne {
                for x in 0..ne {
                    let mut acc = [0.0f32; 4];
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let t = src[base + ((y * 2 + dy) * e + x * 2 + dx) as usize];
                        for c in 0..4 {
                            acc[c] += t[c] * 0.25;
                        }
                    }
                    next.push(acc);
                }
            }
        }
        levels.push((ne, next));
    }
    Ok(CubeData { levels, face_means, scale: cube.scale })
}

/// Everything a Hideout needs, read on the CPU: the description, its props (one per
/// `desc.models` entry; placements of the same file share one `Model`, a failed or left-out one
/// is `None`), the sky and the reflection probe, and what failed or was left out.
pub struct SceneAssets {
    pub desc: SceneDesc,
    pub models: Vec<Option<Arc<Model>>>,
    pub sky: Option<CubeData>,
    pub reflection: Option<CubeData>,
    /// The lightmap, probe and fog volumes, cookies and colour correction.
    pub hideout: hideout::HideoutData,
    /// The map's particle bundle, when the scene names one.
    pub particles: Option<particles::Bundle>,
    pub errors: Vec<String>,
    /// Files and placements left out to stay inside the bounds below.
    pub skipped: Vec<String>,
    /// Distinct model files read.
    pub files: usize,
}

/// The most distinct model files and placements a scene loads. A real `dl_hideout` has a few
/// hundred files and a few thousand placements; past these the rest are left out.
pub const MAX_SCENE_FILES: usize = 2048;
pub const MAX_SCENE_PLACEMENTS: usize = 20_000;
/// A model file larger than this is left out unread (the whole file is read into memory).
pub const MAX_MODEL_FILE: u64 = 512 << 20;
/// What the scene's decoded models may hold in memory together (vertices, indices, pixels).
pub const SCENE_MEMORY_BUDGET: u64 = 2 << 30;

/// Where a scene load is, for the loading screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadStage {
    /// Reading model files: `done` of `of` distinct files.
    Reading,
    /// The sky and reflection cubes.
    Lighting,
    /// Putting placements on the GPU: `done` of `of`.
    Uploading,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadProgress {
    pub stage: LoadStage,
    pub done: usize,
    pub of: usize,
}

/// Reads a `scene.json` and everything it names (any thread). The hero is not loaded here.
pub fn load_scene(path: &Path) -> Result<SceneAssets, String> {
    load_scene_with(path, &AtomicBool::new(false), &mut |_| {}).map_err(|e| e.to_string())
}

/// `load_scene` that reports progress and stops early (`ViewerError::Cancelled`) when `cancel`
/// is set.
pub fn load_scene_with(path: &Path, cancel: &AtomicBool, progress: &mut dyn FnMut(LoadProgress)) -> Result<SceneAssets, ViewerError> {
    let desc = SceneDesc::read(path).map_err(ViewerError::Load)?;
    let loaded = load_scene_models(&desc, cancel, progress)?;
    let mut errors = loaded.errors;
    progress(LoadProgress { stage: LoadStage::Lighting, done: 0, of: 2 });
    let mut cube = |c: &Option<scene::Cube>| {
        c.as_ref().and_then(|c| match load_cube(c) {
            Ok(d) => Some(d),
            Err(e) => {
                errors.push(e);
                None
            }
        })
    };
    let sky = cube(&desc.sky);
    let reflection = cube(&desc.reflection);
    let hideout = hideout::HideoutData::load(&desc);
    errors.extend(hideout.errors.iter().cloned());
    let particles = desc.particles.as_deref().and_then(|p| match particles::Bundle::read(p) {
        Ok(b) => Some(b),
        Err(e) => {
            errors.push(e);
            None
        }
    });
    if cancel.load(Ordering::Relaxed) {
        return Err(ViewerError::Cancelled);
    }
    Ok(SceneAssets { desc, models: loaded.models, sky, reflection, hideout, particles, errors, skipped: loaded.skipped, files: loaded.files })
}

/// Bytes a decoded model holds (vertices, indices, pixels, clips), for the memory budget.
pub fn model_bytes(m: &Model) -> u64 {
    let prims: u64 = m.primitives.iter().map(|p| (p.vertices.len() * std::mem::size_of::<model::Vertex>() + p.indices.len() * 4) as u64).sum();
    let images: u64 = m.images.iter().map(|i| i.rgba.len() as u64).sum();
    let clips: u64 = m.clips.iter().flat_map(|c| &c.channels).map(|ch| ((ch.times.len() + ch.values.len()) * 4) as u64).sum();
    prims + images + clips
}

/// What `load_scene_models` read.
pub struct LoadedModels {
    pub models: Vec<Option<Arc<Model>>>,
    pub errors: Vec<String>,
    pub skipped: Vec<String>,
    pub files: usize,
}

/// Loads every prop a scene lists, each distinct file once (a room places one aggregate glb
/// hundreds of times, one fragment each: reading it per placement ran the app out of memory).
/// A file that fails is an error; one past the bounds above is left out.
pub fn load_scene_models(desc: &SceneDesc, cancel: &AtomicBool, progress: &mut dyn FnMut(LoadProgress)) -> Result<LoadedModels, ViewerError> {
    let mut skipped = Vec::new();
    let placements = desc.models.len().min(MAX_SCENE_PLACEMENTS);
    if desc.models.len() > placements {
        skipped.push(format!("{} placements past the first {MAX_SCENE_PLACEMENTS}", desc.models.len() - placements));
    }
    let mut order: Vec<&PathBuf> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for m in &desc.models[..placements] {
        if seen.insert(&m.glb) {
            order.push(&m.glb);
        }
    }
    if order.len() > MAX_SCENE_FILES {
        skipped.push(format!("{} model files past the first {MAX_SCENE_FILES}", order.len() - MAX_SCENE_FILES));
        order.truncate(MAX_SCENE_FILES);
    }
    let mut errors = Vec::new();
    let mut by_path: HashMap<&PathBuf, Arc<Model>> = HashMap::new();
    let mut used = 0u64;
    for (i, path) in order.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(ViewerError::Cancelled);
        }
        progress(LoadProgress { stage: LoadStage::Reading, done: i, of: order.len() });
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        match std::fs::metadata(path) {
            Ok(meta) if meta.len() > MAX_MODEL_FILE => {
                skipped.push(format!("{name}: a {} MB file", meta.len() >> 20));
                continue;
            }
            Err(e) => {
                errors.push(format!("{}: {e}", path.display()));
                continue;
            }
            Ok(_) => {}
        }
        match model::load_path(path) {
            Ok(model) => {
                let bytes = model_bytes(&model);
                if used + bytes > SCENE_MEMORY_BUDGET {
                    skipped.push(format!("{name}: over the {} MB memory budget", SCENE_MEMORY_BUDGET >> 20));
                    continue;
                }
                used += bytes;
                by_path.insert(*path, Arc::new(model));
            }
            Err(e) => errors.push(e.0),
        }
    }
    progress(LoadProgress { stage: LoadStage::Reading, done: order.len(), of: order.len() });
    let files = by_path.len();
    let models = desc.models.iter().enumerate().map(|(i, m)| if i < placements { by_path.get(&m.glb).cloned() } else { None }).collect();
    Ok(LoadedModels { models, errors, skipped, files })
}

/// What `set_scene` put up.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneReport {
    /// Distinct model files uploaded.
    pub files: usize,
    pub placements: usize,
    pub triangles: u64,
    pub cancelled: bool,
    /// Placements past 65 m from the spawn the room hides (`clearance`), left out.
    pub culled: usize,
}

struct GpuCube {
    view: wgpu::TextureView,
    top_mip: f32,
    data: CubeData,
}

pub struct Viewer {
    gpu: Gpu,
    pipes: Pipelines,
    res: FrameResources,
    targets: Option<Targets>,
    hero: Option<Slot>,
    before: Option<Slot>,
    env: Vec<Slot>,
    scene: Option<SceneDesc>,
    pub camera: OrbitCamera,
    home: Option<OrbitCamera>,
    framed_aspect: f32,
    pub playback: Playback,
    pub compare: Compare,
    pub look: Look,
    frame: Vec<u8>,
    last: Timings,
    clock: f32,
    playmode: Playmode,
    sidecar: Sidecar,
    skip_read: bool,
    sky: Option<GpuCube>,
    reflection: Option<GpuCube>,
    /// The props' largest extent, for the depth range (measured once per scene).
    env_extent: f32,
    /// How the device came up: the software adapter after the driver failed, how long the
    /// pipelines took. For the app's log.
    notes: Vec<String>,
    hideout: Option<hideout::HideoutGpu>,
    /// The hero's effects (its bundle for the attachments) and the map's.
    hero_particles: Option<(particles::Bundle, particles::ParticleGpu)>,
    scene_particles: Option<particles::ParticleGpu>,
    /// Effects get a second of simulation before their first frame, so a still shows them.
    particles_warm: bool,
    /// The turntable (auto-rotate): on, and the hero's turn so far in radians.
    pub auto_rotate: bool,
    turn: f32,
    backdrop_image: Option<(wgpu::TextureView, f32)>,
}

impl Viewer {
    /// The first GPU error since the last call (validation, out of memory, lost device). The
    /// caller stops drawing and says why instead of waiting on a frame that never comes.
    pub fn gpu_error(&self) -> Option<String> {
        self.gpu.take_error()
    }

    /// Opens the viewer's own GPU device. `ViewerError::NoGpu` when the machine has none.
    /// The pipelines are built inside error scopes; when the driver rejects them (a shader its
    /// compiler fails on) the software adapter is tried before giving up with the reason.
    pub fn new() -> Result<Viewer, ViewerError> {
        let gpu = Gpu::new()?;
        let started = std::time::Instant::now();
        let mut notes = Vec::new();
        let (gpu, pipes) = match Pipelines::new(&gpu) {
            Ok(pipes) => {
                notes.push(format!("pipelines built in {} ms", started.elapsed().as_millis()));
                (gpu, pipes)
            }
            Err(error) => {
                let failed = format!("{} ({:?})", gpu.info.name, gpu.info.backend);
                notes.push(format!("{failed} could not build the pipelines: {error}"));
                drop(gpu);
                // The report names the software try too: WARP compiles with the same FXC, so a
                // shader FXC rejects fails on both, and the feedback should say it was tried.
                let soft = Gpu::software().map_err(|_| "no software adapter to try".to_string()).and_then(|soft| {
                    let name = format!("{} ({:?})", soft.info.name, soft.info.backend);
                    Pipelines::new(&soft).map(|p| (soft, p)).map_err(|_| format!("the software adapter {name} failed too"))
                });
                match soft {
                    Ok((soft, pipes)) => {
                        notes.push(format!("using the software adapter {} instead", soft.info.name));
                        (soft, pipes)
                    }
                    Err(soft) => return Err(ViewerError::Device(format!("{failed} could not build the 3D view's shaders ({soft}): {error}"))),
                }
            }
        };
        let res = FrameResources::new(&gpu, &pipes);
        Ok(Viewer {
            gpu,
            pipes,
            res,
            targets: None,
            hero: None,
            before: None,
            env: Vec::new(),
            scene: None,
            camera: OrbitCamera::default(),
            home: None,
            framed_aspect: 0.0,
            playback: Playback::default(),
            compare: Compare::After,
            look: Look::default(),
            frame: Vec::new(),
            last: Timings::default(),
            clock: 0.0,
            playmode: Playmode::Loop,
            sidecar: Sidecar::default(),
            skip_read: false,
            sky: None,
            reflection: None,
            env_extent: 3.0,
            notes,
            hideout: None,
            hero_particles: None,
            scene_particles: None,
            particles_warm: false,
            auto_rotate: false,
            turn: 0.0,
            backdrop_image: None,
        })
    }

    /// Caps what later loads may put on the GPU (bytes of textures, of vertex and index
    /// buffers). For tests, and for a caller that knows the machine is short of memory.
    pub fn set_budgets(&mut self, texture: u64, buffer: u64) {
        self.gpu.texture_budget = texture;
        self.gpu.buffer_budget = buffer;
    }

    /// The largest frame edge this device renders; `render` clamps to it.
    pub fn max_edge(&self) -> u32 {
        self.gpu.max_edge()
    }

    /// How the device came up (see `notes` on the struct).
    pub fn notes(&self) -> &[String] {
        &self.notes
    }

    /// What is on the GPU and what was left out to fit it.
    pub fn usage(&self) -> Usage {
        self.gpu.usage()
    }

    /// The adapter, for diagnostics ("Apple M2 (Metal)").
    pub fn adapter(&self) -> String {
        format!("{} ({:?})", self.gpu.info.name, self.gpu.info.backend)
    }

    fn hero_placement(&self) -> Mat4 {
        let scene = self.scene.as_ref();
        scene.and_then(|s| s.hero.as_ref().map(|h| h.transform)).or_else(|| scene.and_then(|s| s.hero_anchor)).unwrap_or(Mat4::IDENTITY)
    }

    fn pick_clip(model: &Model, sidecar: &Sidecar) -> Option<usize> {
        let by_name = |name: &str| model.clips.iter().position(|c| c.name == name).or_else(|| model.clips.iter().position(|c| c.name.eq_ignore_ascii_case(name)));
        sidecar.idle.as_deref().and_then(by_name).or_else(|| model::default_idle(model).and_then(|c| by_name(&c.name)))
    }

    /// Shows `model` as the hero (the "after", with the player's mods). Resets playback to
    /// the idle the sidecar names and frames the camera on the next render.
    pub fn set_hero(&mut self, model: Model, sidecar: &Sidecar) -> HeroInfo {
        let clip = Self::pick_clip(&model, sidecar);
        let slot = Slot::new(&self.gpu, &self.pipes, Arc::new(model), self.hero_placement(), clip, [1.0; 4], None);
        self.playmode = clip.map(|c| sidecar.playmode(&slot.model.clips[c].name)).unwrap_or_default();
        self.sidecar = sidecar.clone();
        let info = HeroInfo {
            clips: slot.model.clips.iter().map(|c| (c.name.clone(), c.duration)).collect(),
            clip: clip.map(|c| slot.model.clips[c].name.clone()),
            triangles: slot.gpu.triangle_count(),
            bounds: Some(slot.bounds()),
        };
        self.hero = Some(slot);
        self.playback.time = 0.0;
        self.home = None;
        info
    }

    /// The unmodded hero to compare against, or `None` to drop it.
    pub fn set_before(&mut self, model: Option<Model>, sidecar: &Sidecar) {
        self.before = model.map(|m| {
            let clip = Self::pick_clip(&m, sidecar);
            Slot::new(&self.gpu, &self.pipes, Arc::new(m), self.hero_placement(), clip, [1.0; 4], None)
        });
        if self.before.is_none() {
            self.compare = Compare::After;
        }
    }

    pub fn has_before(&self) -> bool {
        self.before.is_some()
    }

    /// A Hideout scene (see `load_scene`): its props, lights, fog, sky, reflection probe and
    /// camera. The hero moves to the scene's anchor.
    pub fn set_scene(&mut self, assets: SceneAssets) -> SceneReport {
        self.set_scene_with(assets, &AtomicBool::new(false), &mut |_| {})
    }

    /// `set_scene` that reports progress and stops early when `cancel` is set (`cancelled` in
    /// the report; what was placed so far stays). Each distinct file is uploaded once and its
    /// placements share it; what does not fit the device is left out (`usage`).
    pub fn set_scene_with(&mut self, assets: SceneAssets, cancel: &AtomicBool, progress: &mut dyn FnMut(LoadProgress)) -> SceneReport {
        let SceneAssets { desc, models, sky, reflection, hideout: baked, particles: map_particles, .. } = assets;
        self.scene_particles = map_particles.map(|b| {
            let mut pg = particles::ParticleGpu::new(&self.gpu.device, &self.gpu.queue, &self.pipes.frame_layout, self.gpu.msaa, gpu::HDR, gpu::DEPTH, &b, self.gpu.texture_room());
            self.gpu.charge_textures(pg.textures());
            // The map's sun lights its particles (Grimoire's HideoutParticleLayer).
            if let Some(sun) = desc.lights.iter().find(|l| l.kind == scene::LightKind::Directional && !l.fog_only) {
                pg.set_sun(sun.color);
            }
            pg
        });
        self.particles_warm = false;
        self.hideout = Some(hideout::HideoutGpu::new(&self.gpu, &desc, &baked, &self.pipes.hideout_none));
        let mut report = SceneReport::default();
        let mut meshes: HashMap<*const Model, GpuMesh> = HashMap::new();
        self.env.clear();
        let of = models.len();
        for (i, (m, model)) in desc.models.iter().zip(models).enumerate() {
            if cancel.load(Ordering::Relaxed) {
                report.cancelled = true;
                break;
            }
            if i % 16 == 0 {
                progress(LoadProgress { stage: LoadStage::Uploading, done: i, of });
            }
            let Some(model) = model else { continue };
            let what = m.glb.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let mesh = meshes.entry(Arc::as_ptr(&model)).or_insert_with(|| GpuMesh::new(&self.gpu, &self.pipes, &model, &what));
            let clip = m.animation.as_deref().and_then(|a| model.clips.iter().position(|c| c.name == a));
            let slot = Slot::placed(&self.gpu, &self.pipes, model, mesh, m.transform, clip, m.tint, m.primitive).prop(m.primitive);
            report.triangles += u64::from(slot.gpu.triangle_count());
            self.env.push(slot);
        }
        progress(LoadProgress { stage: LoadStage::Uploading, done: of, of });
        report.files = meshes.len();
        let anchor = desc.hero.as_ref().map(|h| h.transform).or(desc.hero_anchor).map(|m| m.point([0.0; 3]));
        report.culled = anchor.map_or(0, |a| self.cull_distant(a));
        report.placements = self.env.len();
        self.env_extent = self
            .env
            .iter()
            .map(|s| math::length(s.bounds().size()))
            .filter(|e| e.is_finite())
            .fold(3.0, f32::max);
        // Cube edges are fixed small (`CUBE_EDGE`); a device below that has no 3D anyway.
        let upload = |gpu: &Gpu, d: CubeData| GpuCube { view: gpu::upload_cube(gpu, &d.levels), top_mip: (d.levels.len() - 1) as f32, data: d };
        self.sky = sky.map(|d| upload(&self.gpu, d));
        self.reflection = reflection.map(|d| upload(&self.gpu, d));
        self.scene = Some(desc);
        let placement = self.hero_placement();
        for slot in [&mut self.hero, &mut self.before].into_iter().flatten() {
            slot.placement = placement;
        }
        self.home = None;
        report
    }

    /// Grimoire's distant-instance culling (`clearance`): props more than 65 m from the spawn
    /// stay only when the room around the hero opens toward them. Returns how many went.
    fn cull_distant(&mut self, anchor: Vec3) -> usize {
        let near = |s: &Slot| clearance::box_distance(&s.bounds(), anchor) <= clearance::NEAR;
        if self.env.iter().all(near) {
            return 0;
        }
        // From the hero's chest, so the floor under its feet does not close every view down.
        let origin = math::add(anchor, [0.0, 1.0, 0.0]);
        // The palettes are first written when a frame poses the props; the views need them now.
        for s in self.env.iter().filter(|s| near(s)) {
            s.gpu.write_palette(&self.gpu, &s.palette);
        }
        let room: Vec<&gpu::GpuModel> = self.env.iter().filter(|s| near(s)).map(|s| &s.gpu).collect();
        let depths = gpu::clearance_depths(&self.gpu, &self.pipes, &bytemuck::Zeroable::zeroed(), origin, &room);
        if depths.len() != (clearance::EDGE * clearance::EDGE * 6) as usize {
            return 0;
        }
        let seen = clearance::Clearance::from_depths(origin, &depths);
        let before = self.env.len();
        self.env.retain(|s| near(s) || seen.sees(&s.bounds()));
        before - self.env.len()
    }

    pub fn clear_scene(&mut self) {
        self.env.clear();
        self.env_extent = 3.0;
        self.scene = None;
        self.scene_particles = None;
        self.hideout = None;
        self.sky = None;
        self.reflection = None;
        let placement = self.hero_placement();
        for slot in [&mut self.hero, &mut self.before].into_iter().flatten() {
            slot.placement = placement;
        }
        self.home = None;
    }

    pub fn clips(&self) -> Vec<(String, f32)> {
        self.hero.as_ref().map(|s| s.model.clips.iter().map(|c| (c.name.clone(), c.duration)).collect()).unwrap_or_default()
    }

    /// Plays `name` from its start. False when the hero has no such clip.
    pub fn set_clip(&mut self, name: &str) -> bool {
        let Some(hero) = &mut self.hero else { return false };
        let Some(i) = hero.model.clips.iter().position(|c| c.name == name) else { return false };
        hero.clip = Some(i);
        self.playmode = self.sidecar.playmode(name);
        hero.reset_cloth(0.0);
        if let Some(before) = &mut self.before
            && let Some(j) = before.model.clips.iter().position(|c| c.name == name)
        {
            before.clip = Some(j);
        }
        self.playback.time = 0.0;
        true
    }

    /// Every clip with its picker label and group, in the sidecar's order.
    pub fn clip_labels(&self) -> Vec<(String, String, String)> {
        self.clips().into_iter().map(|(name, _)| (self.sidecar.label(&name), self.sidecar.group(&name), name)).map(|(l, g, n)| (n, l, g)).collect()
    }

    pub fn clip(&self) -> Option<String> {
        let hero = self.hero.as_ref()?;
        hero.clip.map(|c| hero.model.clips[c].name.clone())
    }

    pub fn duration(&self) -> f32 {
        self.hero.as_ref().map_or(0.0, |h| h.duration())
    }

    /// Whether a frame would differ from the last one without input (a clip is playing).
    pub fn animating(&self) -> bool {
        let d = self.duration();
        let held = self.playmode == Playmode::Hold && self.playback.time >= d;
        (self.playback.playing && d > 0.0 && !held) || self.env.iter().any(|e| e.clip.is_some()) || (self.auto_rotate && self.hero.is_some())
    }

    /// The hero's cloth: the `<codename>.cloth.json` text beside its sidecar (`PHYS.m_pFeModel`),
    /// or none. A model the solver cannot fit plays without cloth.
    pub fn set_hero_cloth(&mut self, json: Option<&str>) -> Option<cloth::ClothStats> {
        let hero = self.hero.as_mut()?;
        hero.cloth = json.and_then(|j| cloth::Cloth::new(&hero.model, j).ok().flatten());
        let time = self.playback.time;
        hero.reset_cloth(time);
        hero.cloth.as_ref().map(|c| c.stats())
    }

    /// The hero's cloth solver, for its statistics.
    pub fn cloth_stats(&self) -> Option<cloth::ClothStats> {
        self.hero.as_ref()?.cloth.as_ref().map(|c| c.stats())
    }

    /// The hero's particle effects (`particles/bundle.json` beside its sidecar), or none.
    pub fn set_hero_particles(&mut self, bundle: Option<particles::Bundle>) {
        self.hero_particles = bundle.map(|b| {
            let gpu = particles::ParticleGpu::new(&self.gpu.device, &self.gpu.queue, &self.pipes.frame_layout, self.gpu.msaa, gpu::HDR, gpu::DEPTH, &b, self.gpu.texture_room());
            self.gpu.charge_textures(gpu.textures());
            (b, gpu)
        });
        self.particles_warm = false;
    }

    /// What the particle port skipped (unknown classes and inputs), hero then map.
    pub fn particle_diagnostics(&self) -> Vec<String> {
        let mut out: Vec<String> = self.hero_particles.iter().flat_map(|(_, g)| g.diagnostics().iter().cloned()).collect();
        out.extend(self.scene_particles.iter().flat_map(|g| g.diagnostics().iter().cloned()));
        out
    }

    /// Live particles, hero and map together.
    pub fn particle_count(&self) -> usize {
        self.hero_particles.as_ref().map_or(0, |(_, g)| g.particle_count()) + self.scene_particles.as_ref().map_or(0, |g| g.particle_count())
    }

    /// Steps the effects by `dt` and puts their cards on the GPU.
    fn step_particles(&mut self, dt: f32) {
        if !self.look.particles || (self.hero_particles.is_none() && self.scene_particles.is_none()) {
            return;
        }
        let camera = self.camera.eye();
        let (steps, step) = if self.particles_warm { (1, dt) } else { (30, 1.0 / 30.0) };
        self.particles_warm = true;
        if let (Some((bundle, pg)), Some(hero)) = (&mut self.hero_particles, &self.hero) {
            let skeleton = hero.skeleton();
            let to_world = hero.placement.mul(&skeleton);
            let back = skeleton.inverse();
            let bone = |name: &str| hero.model.nodes.iter().position(|n| n.name == name).and_then(|i| hero.globals.get(i)).map(|g| back.mul(g));
            let attach = |name: &str| bundle.attachment_frame(name, &bone);
            // The playing clip drives the animation-event effects.
            let clip = hero.clip.and_then(|c| hero.model.clips.get(c)).map(|c| (c.name.as_str(), if c.duration > 0.0 { self.playback.time / c.duration } else { 0.0 }));
            pg.set_clip(clip);
            pg.set_fov(self.camera.fov_y);
            for _ in 0..steps {
                pg.step(step, &to_world, &attach, camera);
            }
            pg.upload(&self.gpu.queue);
        }
        if let Some(pg) = &mut self.scene_particles {
            let to_world = fixture::source_to_gltf();
            for _ in 0..steps {
                pg.step(step, &to_world, &|_| None, camera);
            }
            pg.upload(&self.gpu.queue);
        }
    }

    /// A picture behind the hero (`Backdrop::Image`): straight sRGB RGBA8, cover cropped. `None`
    /// drops it (the backdrop falls back to the studio).
    pub fn set_backdrop_image(&mut self, image: Option<(u32, u32, &[u8])>) {
        self.backdrop_image = image.filter(|(w, h, rgba)| *w > 0 && *h > 0 && rgba.len() >= (*w as usize * *h as usize * 4)).and_then(|(w, h, rgba)| {
            let view = gpu::upload_srgb(&self.gpu, w, h, rgba)?;
            Some((view, w as f32 / h as f32))
        });
        if self.backdrop_image.is_none() && self.look.backdrop == Backdrop::Image {
            self.look.backdrop = Backdrop::Studio;
        }
    }

    /// Loop, or play once and hold the last frame (the sidecar's `playback`).
    pub fn playmode(&self) -> Playmode {
        self.playmode
    }

    /// Moves the clock by `dt` seconds of wall time, looping.
    pub fn advance(&mut self, dt: f32) {
        self.clock += dt;
        if self.playback.playing {
            let d = self.duration();
            // The animation graph's own rate for the clip (a shop pose at 0.69) under the player's.
            let graph = self.clip().map_or(1.0, |c| self.sidecar.speed(&c));
            self.playback.time += dt.max(0.0) * self.playback.speed * graph;
            if d > 0.0 {
                self.playback.time = match self.playmode {
                    Playmode::Loop => self.playback.time.rem_euclid(d),
                    Playmode::Hold => self.playback.time.clamp(0.0, d),
                };
            }
        }
    }

    /// Jumps to `time` seconds into the clip (looped into range).
    pub fn seek(&mut self, time: f32) {
        let d = self.duration();
        self.playback.time = match (d > 0.0, self.playmode) {
            (false, _) => 0.0,
            (true, Playmode::Loop) => time.rem_euclid(d),
            (true, Playmode::Hold) => time.clamp(0.0, d),
        };
        let t = self.playback.time;
        if let Some(h) = &mut self.hero {
            h.reset_cloth(t);
        }
    }

    pub fn reset_camera(&mut self) {
        self.home = None;
        self.turn = 0.0;
    }

    pub fn timings(&self) -> Timings {
        self.last
    }

    fn frame_camera(&mut self, aspect: f32) {
        if let Some((eye, target, fov)) = self.scene.as_ref().and_then(|s| s.camera) {
            let d = math::sub(eye, target);
            let dist = math::length(d).max(0.1);
            self.camera = OrbitCamera {
                yaw: d[0].atan2(d[2]),
                pitch: (d[1] / dist).asin(),
                distance: dist,
                target,
                fov_y: fov.to_radians(),
                min_distance: dist * 0.1,
                max_distance: dist * 4.0,
            };
        } else {
            let b = self.hero.as_ref().map(|h| h.bounds()).unwrap_or(Aabb { min: [-0.5, 0.0, -0.5], max: [0.5, 2.0, 0.5] });
            self.camera = OrbitCamera::framing(&b, aspect);
        }
        self.home = Some(self.camera);
        self.framed_aspect = aspect;
    }

    #[allow(clippy::type_complexity)]
    fn frame_uniforms(&self, width: u32, height: u32) -> (FrameGpu, PostGpu, Option<FloorGpu>, Option<Mat4>, Option<hideout::HideoutUniform>, [[f32; 3]; 5], bool) {
        let aspect = width as f32 / height.max(1) as f32;
        let extent = self.env_extent;
        let (near, far) = self.camera.near_far(extent);
        let view = self.camera.view();
        let proj = Mat4::perspective(self.camera.fov_y, aspect, near, far);
        let scene = self.scene.as_ref();
        let midtown = scene.is_none() && self.look.lighting == Lighting::Midtown;
        let exposure = scene.and_then(|s| s.exposure).unwrap_or(if midtown { 1.6 } else { STUDIO_EXPOSURE });
        let light_scale = RIG_EXPOSURE / exposure;
        let mut lights = [LightGpu::zeroed_light(); 8];
        let mut count = 0usize;
        let key_dir;
        // The light that casts the shadow map, its index in `lights`, and its camera.
        let mut shadow: Option<(usize, Mat4)> = None;
        let mut shadow_baked = false;
        let mut chosen: Vec<&scene::SceneLight> = Vec::new();
        let hero_bounds = self.hero.as_ref().map(|h| h.bounds()).unwrap_or(Aabb { min: [-0.5, 0.0, -0.5], max: [0.5, 2.0, 0.5] });
        let hero_radius = math::length(hero_bounds.size()) * 0.5 * 1.05;
        match scene.filter(|s| !s.lights.is_empty()) {
            Some(s) => {
                // Directional lights first, then the nearest point and spot lights to the hero.
                let anchor = self.hero_placement().point([0.0; 3]);
                let mut order: Vec<&scene::SceneLight> = s.lights.iter().collect();
                // Fog-only lights come after every light that reaches a surface.
                order.sort_by(|a, b| {
                    let rank = |l: &scene::SceneLight| (if l.fog_only { 1e6 } else { 0.0 }) + if l.kind == scene::LightKind::Directional { 0.0 } else { 1.0 + math::length(math::sub(l.position, anchor)) };
                    rank(a).total_cmp(&rank(b))
                });
                for l in order.into_iter().take(8) {
                    chosen.push(l);
                    let d = normalize(l.direction);
                    if shadow.is_none() && l.realtime && !l.fog_only && self.look.shadows {
                        shadow_baked = l.shadow_channel.is_some() && self.hideout.as_ref().is_some_and(|h| h.base.lightmap[2] > 0.5);
                        match l.kind {
                            scene::LightKind::Directional => shadow = Some((count, directional_shadow(d, hero_bounds.center(), hero_radius.max(3.0)))),
                            scene::LightKind::Spot => shadow = Some((count, spot_shadow(l.position, d, l.cone.map_or(0.8, |c| c.1), l.range))),
                            scene::LightKind::Point => {}
                        }
                    }
                    let (cos_outer, cos_inner) = l.cone.map_or((-1.0, -1.0), |(i, o)| (o.cos(), i.cos()));
                    lights[count] = LightGpu {
                        pos_dir: match l.kind {
                            scene::LightKind::Directional => [d[0], d[1], d[2], 0.0],
                            scene::LightKind::Point => [l.position[0], l.position[1], l.position[2], 1.0],
                            scene::LightKind::Spot => [l.position[0], l.position[1], l.position[2], 2.0],
                        },
                        color: [l.color[0], l.color[1], l.color[2], l.range],
                        spot: [d[0], d[1], d[2], cos_outer],
                        spot_inner: [cos_inner, 0.0, 0.0, 0.0],
                    };
                    count += 1;
                }
                key_dir = s
                    .lights
                    .iter()
                    .find(|l| l.kind == scene::LightKind::Directional)
                    .map(|l| normalize(l.direction))
                    .or_else(|| s.lights.iter().min_by(|a, b| math::length(math::sub(a.position, anchor)).total_cmp(&math::length(math::sub(b.position, anchor)))).map(|l| normalize(math::sub(l.position, anchor))))
                    .unwrap_or(normalize(KEY));
            }
            None => {
                let k = normalize(KEY);
                let f = normalize(FILL);
                // Midtown dusk: a warm (254, 153, 91) key at 2.2 instead of the white 1.4.
                let key = if midtown { [254.0 / 255.0 * 2.2, 153.0 / 255.0 * 2.2, 91.0 / 255.0 * 2.2] } else { [1.4; 3] };
                lights[0] = LightGpu { pos_dir: [k[0], k[1], k[2], 0.0], color: [key[0] * light_scale, key[1] * light_scale, key[2] * light_scale, 0.0], ..LightGpu::zeroed_light() };
                lights[1] = LightGpu { pos_dir: [f[0], f[1], f[2], 0.0], color: [0.65 * 1.2 * light_scale, 0.78 * 1.2 * light_scale, 1.2 * light_scale, 0.0], ..LightGpu::zeroed_light() };
                count = 2;
                key_dir = k;
                if self.look.shadows {
                    shadow = Some((0, directional_shadow(k, hero_bounds.center(), hero_radius)));
                }
            }
        }
        let (top, bottom) = (ENV_TOP, ENV_BOTTOM);
        let env_k = if midtown { 1.6 } else { 1.1 } * light_scale;
        // A scene's probe (or sky) gives the ambient per side; the studio integrates its room.
        let probe = self.reflection.as_ref().or(self.sky.as_ref());
        let cube = match probe {
            Some(c) => c.data.face_means,
            None => ambient_cube(top, bottom, key_dir, env_k, 1.5 * light_scale),
        };
        let fit = luminance_fit(&cube);
        let eye = self.camera.eye();
        let back = self.camera.back();
        let unit = self.hero.as_ref().map_or(0.0254, |h| h.unit);
        let fog = scene.and_then(|s| s.fog.as_ref());
        let frame = FrameGpu {
            view_proj: proj.mul(&view).to_cols_array(),
            view: view.to_cols_array(),
            proj: proj.to_cols_array(),
            camera_pos: [eye[0], eye[1], eye[2], near],
            camera_back: [back[0], back[1], back[2], far],
            key_dir: [key_dir[0], key_dir[1], key_dir[2], exposure],
            ambient: cube.map(|c| [c[0], c[1], c[2], 0.0]),
            ambient_fit: fit,
            env_top: [top[0], top[1], top[2], env_k],
            env_bottom: [bottom[0], bottom[1], bottom[2], unit],
            viewport: [width as f32, height as f32, 1.0, self.clock],
            fog_color: match fog {
                Some(f) => [f.color[0], f.color[1], f.color[2], 1.0],
                None => [0.0; 4],
            },
            fog_params: match fog {
                Some(f) => [f.start, f.end, 0.0, f.strength],
                None => [0.0; 4],
            },
            counts: [count as f32, 1.0, if self.reflection.is_some() { 1.0 } else { 0.0 }, self.reflection.as_ref().map_or(0.0, |c| c.top_mip)],
            env_scale: match &self.reflection {
                Some(c) => [c.data.scale[0], c.data.scale[1], c.data.scale[2], 0.0],
                None => [1.0; 4],
            },
            lights,
            shadow_matrix: shadow.map_or(Mat4::IDENTITY, |s| s.1).to_cols_array(),
            shadow: match shadow {
                // Normal offset: about a texel and a half of the light's box.
                Some((i, _)) => [1.0, i as f32, 1.0 / gpu::SHADOW_SIZE as f32, (hero_radius * 2.0 / gpu::SHADOW_SIZE as f32 * 1.5).max(0.002)],
                None => [0.0; 4],
            },
            inv_view_proj: proj.mul(&view).inverse().to_cols_array(),
            // `render_frame` turns it on when it marches the fog at half resolution.
            fog_half: [0.0; 4],
        };
        let (mut post, bloom_weights) = match scene {
            Some(s) => map_post(exposure, &s.post),
            None => (hero_post(exposure), [[1.0; 3]; 5]),
        };
        post.top[3] = if self.look.transparent { 1.0 } else { 0.0 };
        match self.look.backdrop {
            Backdrop::Studio => {}
            Backdrop::Plain => {
                post.bg = [3.0, 0.0, 0.0, 0.0];
                post.bg0 = [26.0 / 255.0, 26.0 / 255.0, 26.0 / 255.0, 0.0];
            }
            Backdrop::Gradient(i) => {
                let g = GRADIENTS[i.min(GRADIENTS.len() - 1)];
                let center = g.center.unwrap_or([0.5, 0.0]);
                post.bg = [if g.center.is_some() { 1.0 } else { 2.0 }, g.spread, g.vignette, g.mid];
                post.bg_at = [center[0], center[1], width as f32 / height.max(1) as f32, 0.0];
                post.bg0 = [g.stops[0][0], g.stops[0][1], g.stops[0][2], 0.0];
                post.bg1 = [g.stops[1][0], g.stops[1][1], g.stops[1][2], 0.0];
                post.bg2 = [g.stops[2][0], g.stops[2][1], g.stops[2][2], 0.0];
            }
            Backdrop::Image => {
                if let Some((_, image_aspect)) = &self.backdrop_image {
                    post.bg = [4.0, 0.0, 0.0, 0.0];
                    post.bg_at = [0.5, 0.5, width as f32 / height.max(1) as f32, *image_aspect];
                }
            }
        }
        if !self.look.bloom {
            post.curve_b[2] = 0.0;
        }
        post.inv_view_proj = proj.mul(&view).inverse().to_cols_array();
        if let Some(sky) = &self.sky {
            post.sky = [sky.data.scale[0], sky.data.scale[1], sky.data.scale[2], 1.0];
        } else if scene.is_some() {
            // No sky in the export: a flat dark room colour, no studio glow.
            post.glow[3] = 0.0;
        }
        if let Compare::Split(f) = self.compare
            && self.before.is_some()
        {
            post.split = [f.clamp(0.0, 1.0) * width as f32, (height as f32 / 800.0).max(1.0), width as f32, height as f32];
        }
        let floor = (self.look.floor && self.scene.is_none()).then(|| {
            let b = self.hero.as_ref().map(|h| h.bounds()).unwrap_or(Aabb { min: [-0.5, 0.0, -0.5], max: [0.5, 2.0, 0.5] });
            let c = b.center();
            let s = b.size();
            let radius = (s[0].max(s[2]) * 0.9).max(s[1] * 0.32);
            // Grimoire's ground: the hero's origin unless the pose lifts it all above, lowered by
            // the playing clip's floor offset (feet a UI clip sinks stay on the floor).
            let ground = self.hero.as_ref().map_or(b.min[1], |h| h.placement.point([0.0; 3])[1].max(b.min[1]) + self.clip().map_or(0.0, |n| self.sidecar.floor(&n)) * h.placement.scale_factor());
            FloorGpu { center: [c[0], ground + 0.002, c[2], radius], shadow: [-key_dir[0] * 0.35, -key_dir[2] * 0.35, 0.85, 0.0] }
        });
        let baked = match (&self.hideout, scene) {
            (Some(h), Some(s)) => {
                let mut u = h.base;
                hideout::light_extras(&mut u, &chosen, &h.cookie_layers, &s.lights);
                Some(u)
            }
            _ => None,
        };
        (frame, post, floor, shadow.map(|s| s.1), baked, bloom_weights, shadow_baked)
    }

    /// Advances the clock by `dt` seconds and renders a `width` by `height` frame. Returns
    /// tightly packed sRGB RGBA8 rows, top first, opaque unless `look.transparent`.
    pub fn render(&mut self, width: u32, height: u32, dt: f32) -> &[u8] {
        let edge = self.gpu.max_edge();
        let (width, height) = (width.clamp(1, edge), height.clamp(1, edge));
        self.advance(dt);
        if self.auto_rotate {
            self.turn = (self.turn + dt.clamp(0.0, 0.1) * SPIN_SPEED).rem_euclid(std::f32::consts::TAU);
        }
        let placement = self.hero_placement().mul(&Mat4::from_trs([0.0; 3], math::quat_axis_angle([0.0, 1.0, 0.0], self.turn), [1.0; 3]));
        for slot in [&mut self.hero, &mut self.before].into_iter().flatten() {
            slot.placement = placement;
        }
        let time = self.playback.time;
        let hero_duration = self.duration();
        let cloth_on = self.look.cloth;
        if let Some(h) = &mut self.hero {
            if h.cloth_on != cloth_on {
                h.cloth_on = cloth_on;
                h.reset_cloth(time);
            }
            h.pose(&self.gpu, time);
        }
        if let Some(b) = &mut self.before {
            // The stock clip follows the modded clip's phase, even when its length differs.
            let d = b.duration();
            let t = if hero_duration > 0.0 && d > 0.0 { time / hero_duration * d } else { time };
            b.pose(&self.gpu, t);
        }
        let clock = self.clock;
        for e in &mut self.env {
            let d = e.duration();
            e.pose(&self.gpu, if d > 0.0 { clock.rem_euclid(d) } else { 0.0 });
        }
        self.step_particles(dt);
        let aspect = width as f32 / height as f32;
        if self.home.is_none() || ((aspect - self.framed_aspect).abs() > 0.01 && self.home == Some(self.camera)) {
            self.frame_camera(aspect);
        }
        if self.targets.as_ref().is_none_or(|t| t.width != width || t.height != height) {
            self.targets = Some(Targets::new(&self.gpu, width, height));
        }
        let (frame, post, floor, shadow, baked, bloom_weights, shadow_hero_only) = self.frame_uniforms(width, height);
        let eye = self.camera.eye();
        let dist = |slot: &Slot, c: Vec3, mesh: usize| {
            let m = slot.bases.get(mesh).and_then(|&b| slot.palette.get(b as usize)).copied().unwrap_or_default();
            math::length(math::sub(m.point(c), eye))
        };
        let env_fns: Vec<Box<dyn Fn(Vec3, usize) -> f32 + '_>> = self.env.iter().map(|s| Box::new(move |c, m| dist(s, c, m)) as Box<dyn Fn(Vec3, usize) -> f32>).collect();
        let mut after: Vec<Shown> = self.env.iter().zip(&env_fns).map(|(s, f)| Shown { model: &s.gpu, hero: false, eye_distance: f.as_ref() }).collect();
        let mut before_list: Vec<Shown> = self.env.iter().zip(&env_fns).map(|(s, f)| Shown { model: &s.gpu, hero: false, eye_distance: f.as_ref() }).collect();
        let hero_fn = self.hero.as_ref().map(|s| move |c, m| dist(s, c, m));
        let before_fn = self.before.as_ref().map(|s| move |c, m| dist(s, c, m));
        if let (Some(h), Some(f)) = (&self.hero, &hero_fn) {
            after.push(Shown { model: &h.gpu, hero: true, eye_distance: f });
        }
        if let (Some(b), Some(f)) = (&self.before, &before_fn) {
            before_list.push(Shown { model: &b.gpu, hero: true, eye_distance: f });
        }
        let particle_list: Vec<&particles::ParticleGpu> = if self.look.particles { self.hero_particles.iter().map(|(_, g)| g).chain(self.scene_particles.iter()).collect() } else { Vec::new() };
        let layers: [Option<&[Shown]>; 2] = match (self.compare, self.before.is_some()) {
            (Compare::Before, true) => [Some(&before_list), None],
            (Compare::Split(_), true) => [Some(&after), Some(&before_list)],
            _ => [Some(&after), None],
        };
        let inputs = FrameInputs { frame, post, floor, layers, read: !self.skip_read, env_cube: self.reflection.as_ref().map(|c| &c.view), sky_cube: self.sky.as_ref().map(|c| &c.view), backdrop: self.backdrop_image.as_ref().map(|b| &b.0), shadow: shadow.map(|m| m.to_cols_array()), hideout: self.hideout.as_ref().zip(baked), half_res_fog: self.look.half_res_fog, bloom_weights, shadow_hero_only, particles: &particle_list };
        let targets = self.targets.as_ref().expect("targets");
        let mut out = std::mem::take(&mut self.frame);
        self.last = gpu::render_frame(&self.gpu, &self.pipes, &self.res, targets, &inputs, &mut out);
        // The draw lists borrow the slots; they end before the frame is stored.
        std::mem::drop(env_fns);
        self.frame = out;
        &self.frame
    }

    /// Renders a frame and waits for the GPU without reading it back: what the frame costs
    /// before the copy to the CPU, for measuring the readback.
    pub fn render_without_readback(&mut self, width: u32, height: u32, dt: f32) -> Timings {
        self.skip_read = true;
        let _ = self.render(width, height, dt);
        self.skip_read = false;
        self.last
    }

    /// Renders one frame without advancing the clock and encodes it as PNG.
    pub fn capture_png(&mut self, width: u32, height: u32) -> Result<Vec<u8>, ViewerError> {
        let edge = self.gpu.max_edge();
        let rgba = self.render(width, height, 0.0).to_vec();
        encode_png(width.clamp(1, edge), height.clamp(1, edge), &rgba)
    }

    /// Loads a hero file and its sidecar beside it (same stem, `.json`) or at `sidecar`.
    pub fn load_hero_file(&mut self, glb: &Path, sidecar: Option<&Path>) -> Result<HeroInfo, ViewerError> {
        let model = model::load_path(glb).map_err(|e| ViewerError::Load(e.0))?;
        let side = sidecar.map(Sidecar::read).unwrap_or_else(|| Sidecar::read(&glb.with_extension("json")));
        Ok(self.set_hero(model, &side))
    }
}

pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, ViewerError> {
    use image::ImageEncoder;
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out).write_image(rgba, width, height, image::ExtendedColorType::Rgba8).map_err(|e| ViewerError::Load(e.to_string()))?;
    Ok(out)
}

impl LightGpu {
    fn zeroed_light() -> LightGpu {
        LightGpu { pos_dir: [0.0; 4], color: [0.0; 4], spot: [0.0, -1.0, 0.0, -1.0], spot_inner: [-1.0, 0.0, 0.0, 0.0] }
    }
}
