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
pub mod fixture;
pub mod gpu;
pub mod material;
pub mod math;
pub mod model;
pub mod scene;

use std::path::Path;

pub use camera::OrbitCamera;
pub use gpu::{GpuError, Timings};
pub use model::Model;
pub use scene::{Playmode, SceneDesc, Sidecar};

use gpu::{FloorGpu, FrameGpu, FrameInputs, FrameResources, Gpu, GpuModel, LightGpu, Pipelines, PostGpu, Shown, Targets};
use math::{Aabb, Mat4, Vec3, normalize};

#[derive(Debug, Clone, PartialEq)]
pub enum ViewerError {
    /// No GPU adapter: show a still image instead.
    NoGpu(String),
    Device(String),
    Load(String),
}

impl std::fmt::Display for ViewerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ViewerError::NoGpu(e) => write!(f, "no GPU adapter: {e}"),
            ViewerError::Device(e) => write!(f, "GPU device: {e}"),
            ViewerError::Load(e) => f.write_str(e),
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
}

impl Default for Look {
    fn default() -> Self {
        Look { transparent: false, bloom: true, floor: true }
    }
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
    model: Model,
    gpu: GpuModel,
    placement: Mat4,
    clip: Option<usize>,
    palette: Vec<Mat4>,
    bases: Vec<u32>,
    unit: f32,
}

impl Slot {
    fn new(gpu: &Gpu, p: &Pipelines, model: Model, placement: Mat4, clip: Option<usize>, tint: [f32; 4], only: Option<usize>) -> Slot {
        let locals = model::sample_pose(&model, clip.and_then(|c| model.clips.get(c)), 0.0);
        let globals = model::globals(&model, &locals);
        let mut palette = Vec::new();
        let bases = model::palettes(&model, &globals, &placement, &mut palette);
        let unit = model::game_unit(&model, &globals) * placement.scale_factor();
        let g = GpuModel::new(gpu, p, &model, palette.len(), &bases, tint, only);
        Slot { model, gpu: g, placement, clip, palette, bases, unit }
    }

    fn duration(&self) -> f32 {
        self.clip.and_then(|c| self.model.clips.get(c)).map_or(0.0, |c| c.duration)
    }

    fn pose(&mut self, gpu: &Gpu, time: f32) {
        let clip = self.clip.and_then(|c| self.model.clips.get(c));
        let locals = model::sample_pose(&self.model, clip, time);
        let globals = model::globals(&self.model, &locals);
        self.bases = model::palettes(&self.model, &globals, &self.placement, &mut self.palette);
        self.gpu.write_palette(gpu, &self.palette);
    }

    fn bounds(&self) -> Aabb {
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
    }
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

/// Everything a Hideout needs, read on the CPU: the description, its props (a failed one is
/// `None`), the sky and the reflection probe, and what failed to load.
pub struct SceneAssets {
    pub desc: SceneDesc,
    pub models: Vec<Option<Model>>,
    pub sky: Option<CubeData>,
    pub reflection: Option<CubeData>,
    pub errors: Vec<String>,
}

/// Reads a `scene.json` and everything it names (any thread). The hero is not loaded here.
pub fn load_scene(path: &Path) -> Result<SceneAssets, String> {
    let desc = SceneDesc::read(path)?;
    let (models, mut errors) = load_scene_models(&desc);
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
    Ok(SceneAssets { desc, models, sky, reflection, errors })
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
}

impl Viewer {
    /// Opens the viewer's own GPU device. `ViewerError::NoGpu` when the machine has none.
    pub fn new() -> Result<Viewer, ViewerError> {
        let gpu = Gpu::new()?;
        let pipes = Pipelines::new(&gpu);
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
        })
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
        let slot = Slot::new(&self.gpu, &self.pipes, model, self.hero_placement(), clip, [1.0; 4], None);
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
            Slot::new(&self.gpu, &self.pipes, m, self.hero_placement(), clip, [1.0; 4], None)
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
    pub fn set_scene(&mut self, assets: SceneAssets) {
        let SceneAssets { desc, models, sky, reflection, .. } = assets;
        self.env = desc
            .models
            .iter()
            .zip(models)
            .filter_map(|(m, model)| {
                let model = model?;
                let clip = m.animation.as_deref().and_then(|a| model.clips.iter().position(|c| c.name == a));
                Some(Slot::new(&self.gpu, &self.pipes, model, m.transform, clip, m.tint, m.primitive))
            })
            .collect();
        let upload = |gpu: &Gpu, d: CubeData| GpuCube { view: gpu::upload_cube(gpu, &d.levels), top_mip: (d.levels.len() - 1) as f32, data: d };
        self.sky = sky.map(|d| upload(&self.gpu, d));
        self.reflection = reflection.map(|d| upload(&self.gpu, d));
        self.scene = Some(desc);
        let placement = self.hero_placement();
        for slot in [&mut self.hero, &mut self.before].into_iter().flatten() {
            slot.placement = placement;
        }
        self.home = None;
    }

    pub fn clear_scene(&mut self) {
        self.env.clear();
        self.scene = None;
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
        if let Some(before) = &mut self.before
            && let Some(j) = before.model.clips.iter().position(|c| c.name == name)
        {
            before.clip = Some(j);
        }
        self.playback.time = 0.0;
        true
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
        (self.playback.playing && d > 0.0 && !held) || self.env.iter().any(|e| e.clip.is_some())
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
            self.playback.time += dt.max(0.0) * self.playback.speed;
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
    }

    pub fn reset_camera(&mut self) {
        self.home = None;
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

    fn frame_uniforms(&self, width: u32, height: u32) -> (FrameGpu, PostGpu, Option<FloorGpu>, f32) {
        let aspect = width as f32 / height.max(1) as f32;
        let extent = self.env.iter().map(|s| math::length(s.bounds().size())).fold(3.0, f32::max);
        let (near, far) = self.camera.near_far(extent);
        let view = self.camera.view();
        let proj = Mat4::perspective(self.camera.fov_y, aspect, near, far);
        let scene = self.scene.as_ref();
        let exposure = scene.and_then(|s| s.exposure).unwrap_or(STUDIO_EXPOSURE);
        let light_scale = RIG_EXPOSURE / exposure;
        let mut lights = [LightGpu::zeroed_light(); 8];
        let mut count = 0usize;
        let key_dir;
        match scene.filter(|s| !s.lights.is_empty()) {
            Some(s) => {
                // Directional lights first, then the nearest point and spot lights to the hero.
                let anchor = self.hero_placement().point([0.0; 3]);
                let mut order: Vec<&scene::SceneLight> = s.lights.iter().collect();
                order.sort_by(|a, b| {
                    let rank = |l: &scene::SceneLight| if l.kind == scene::LightKind::Directional { 0.0 } else { 1.0 + math::length(math::sub(l.position, anchor)) };
                    rank(a).total_cmp(&rank(b))
                });
                for l in order.into_iter().take(8) {
                    let d = normalize(l.direction);
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
                lights[0] = LightGpu { pos_dir: [k[0], k[1], k[2], 0.0], color: [1.4 * light_scale, 1.4 * light_scale, 1.4 * light_scale, 0.0], ..LightGpu::zeroed_light() };
                lights[1] = LightGpu { pos_dir: [f[0], f[1], f[2], 0.0], color: [0.65 * 1.2 * light_scale, 0.78 * 1.2 * light_scale, 1.2 * light_scale, 0.0], ..LightGpu::zeroed_light() };
                count = 2;
                key_dir = k;
            }
        }
        let (top, bottom) = (ENV_TOP, ENV_BOTTOM);
        let env_k = 1.1 * light_scale;
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
        let fog = scene.and_then(|s| s.fog);
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
        };
        let mut post = if scene.is_some() { base_post(exposure) } else { hero_post(exposure) };
        post.top[3] = if self.look.transparent { 1.0 } else { 0.0 };
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
            FloorGpu { center: [c[0], b.min[1] + 0.002, c[2], radius], shadow: [-key_dir[0] * 0.35, -key_dir[2] * 0.35, 0.85, 0.0] }
        });
        (frame, post, floor, aspect)
    }

    /// Advances the clock by `dt` seconds and renders a `width` by `height` frame. Returns
    /// tightly packed sRGB RGBA8 rows, top first, opaque unless `look.transparent`.
    pub fn render(&mut self, width: u32, height: u32, dt: f32) -> &[u8] {
        let (width, height) = (width.clamp(1, 8192), height.clamp(1, 8192));
        self.advance(dt);
        let time = self.playback.time;
        let hero_duration = self.duration();
        if let Some(h) = &mut self.hero {
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
        let aspect = width as f32 / height as f32;
        if self.home.is_none() || ((aspect - self.framed_aspect).abs() > 0.01 && self.home == Some(self.camera)) {
            self.frame_camera(aspect);
        }
        if self.targets.as_ref().is_none_or(|t| t.width != width || t.height != height) {
            self.targets = Some(Targets::new(&self.gpu, width, height));
        }
        let (frame, post, floor, _) = self.frame_uniforms(width, height);
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
        let layers: [Option<&[Shown]>; 2] = match (self.compare, self.before.is_some()) {
            (Compare::Before, true) => [Some(&before_list), None],
            (Compare::Split(_), true) => [Some(&after), Some(&before_list)],
            _ => [Some(&after), None],
        };
        let inputs = FrameInputs { frame, post, floor, layers, read: !self.skip_read, env_cube: self.reflection.as_ref().map(|c| &c.view), sky_cube: self.sky.as_ref().map(|c| &c.view) };
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
        let rgba = self.render(width, height, 0.0).to_vec();
        encode_png(width.clamp(1, 8192), height.clamp(1, 8192), &rgba)
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

/// Loads every prop a scene lists, skipping the ones that fail (their errors are returned).
pub fn load_scene_models(desc: &SceneDesc) -> (Vec<Option<Model>>, Vec<String>) {
    let mut errors = Vec::new();
    let models = desc
        .models
        .iter()
        .map(|m| match model::load_path(&m.glb) {
            Ok(model) => Some(model),
            Err(e) => {
                errors.push(e.0);
                None
            }
        })
        .collect();
    (models, errors)
}
