//! A bundled stand-in hero, built in code: a stylised figure in a long coat and hat with a
//! skeleton, rigid skinning, an idle clip and `pbr.vfx` NPR materials shaped like vpkmerge's
//! export (bones in Source units under a `skeleton` node that converts inches Z-up to meters
//! Y-up, `extras.morphic` schema 4, a tint and rim mask, an outline mask, self-illuminated
//! eyes). No Valve asset is involved; it exists so tests, the demo scene and a machine without
//! Deadlock can exercise the whole renderer. `Variant::Modded` recolors the coat through the
//! tint and the glow, as a skin mod would.

use std::f32::consts::{PI, TAU};

use crate::math::{Mat4, Quat, Vec3, quat_axis_angle, quat_mul};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    Stock,
    Modded,
}

struct Bone {
    name: &'static str,
    parent: Option<usize>,
    /// Bind position in Source space (inches, Z up, facing +X), global.
    head: Vec3,
}

const BONES: &[Bone] = &[
    Bone { name: "pelvis", parent: None, head: [0.0, 0.0, 38.0] },
    Bone { name: "spine_0", parent: Some(0), head: [0.0, 0.0, 44.0] },
    Bone { name: "spine_1", parent: Some(1), head: [0.0, 0.0, 51.0] },
    Bone { name: "neck_0", parent: Some(2), head: [0.0, 0.0, 57.0] },
    Bone { name: "head", parent: Some(3), head: [0.0, 0.0, 61.0] },
    Bone { name: "arm_upper_L", parent: Some(2), head: [0.0, 9.5, 55.0] },
    Bone { name: "arm_lower_L", parent: Some(5), head: [0.0, 10.5, 42.0] },
    Bone { name: "hand_L", parent: Some(6), head: [1.5, 10.8, 31.0] },
    Bone { name: "arm_upper_R", parent: Some(2), head: [0.0, -9.5, 55.0] },
    Bone { name: "arm_lower_R", parent: Some(8), head: [0.0, -10.5, 42.0] },
    Bone { name: "hand_R", parent: Some(9), head: [1.5, -10.8, 31.0] },
    Bone { name: "leg_upper_L", parent: Some(0), head: [0.0, 4.5, 37.0] },
    Bone { name: "leg_lower_L", parent: Some(11), head: [0.0, 4.5, 20.0] },
    Bone { name: "ankle_L", parent: Some(12), head: [0.0, 4.8, 3.0] },
    Bone { name: "leg_upper_R", parent: Some(0), head: [0.0, -4.5, 37.0] },
    Bone { name: "leg_lower_R", parent: Some(14), head: [0.0, -4.5, 20.0] },
    Bone { name: "ankle_R", parent: Some(15), head: [0.0, -4.8, 3.0] },
];

/// Atlas rows (8 swatches, 32 px each).
#[derive(Clone, Copy)]
enum Swatch {
    Skin = 0,
    Coat = 1,
    Trousers = 2,
    Boots = 3,
    Shirt = 4,
    Hat = 5,
    Brass = 6,
    Gloves = 7,
}

#[derive(Default)]
struct Mesh {
    pos: Vec<Vec3>,
    nrm: Vec<Vec3>,
    uv: Vec<[f32; 2]>,
    joint: Vec<u16>,
    idx: Vec<u32>,
}

impl Mesh {
    /// A tapered tube from `a` to `b` with domed ends, its UVs inside `swatch`'s row.
    #[allow(clippy::too_many_arguments)]
    fn capsule(&mut self, a: Vec3, b: Vec3, ra: f32, rb: f32, bone: usize, swatch: Swatch, squash: [f32; 2], caps: bool) {
        let axis = crate::math::sub(b, a);
        let len = crate::math::length(axis);
        let w = crate::math::normalize(axis);
        let up = if w[2].abs() > 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 0.0, 1.0] };
        let u = crate::math::normalize(crate::math::cross(up, w));
        let v = crate::math::cross(w, u);
        let (seg, rings) = (24usize, 14usize);
        let cap_rings = if caps { 5 } else { 0 };
        let start = self.pos.len() as u32;
        let total = rings + 2 * cap_rings;
        let row = swatch as u32 as f32;
        for i in 0..=total {
            // Parameter along the profile: caps are hemispheres scaled by the end radius.
            let (t, r, nz) = if i < cap_rings {
                let ang = (i as f32 / cap_rings as f32) * PI * 0.5;
                (-(ra * (PI * 0.5 - ang).sin().max(0.0)) / len.max(1e-3) * 0.6, ra * ang.sin(), -(PI * 0.5 - ang).sin())
            } else if i > rings + cap_rings {
                let k = (i - rings - cap_rings) as f32 / cap_rings as f32;
                let ang = k * PI * 0.5;
                (1.0 + (rb * ang.sin()) / len.max(1e-3) * 0.6, rb * ang.cos(), ang.sin())
            } else {
                let t = (i - cap_rings) as f32 / rings as f32;
                (t, ra + (rb - ra) * t, (ra - rb) / len.max(1e-3))
            };
            let center = crate::math::add(a, crate::math::scale(axis, t));
            for j in 0..=seg {
                let ang = j as f32 / seg as f32 * TAU;
                let (s, c) = ang.sin_cos();
                let dir = crate::math::add(crate::math::scale(u, c * squash[0]), crate::math::scale(v, s * squash[1]));
                let p = crate::math::add(center, crate::math::scale(dir, r));
                let radial = crate::math::normalize(crate::math::add(crate::math::scale(u, c / squash[0]), crate::math::scale(v, s / squash[1])));
                let n = crate::math::normalize(crate::math::add(crate::math::scale(radial, (1.0 - nz * nz).max(0.0).sqrt()), crate::math::scale(w, nz)));
                self.pos.push(p);
                self.nrm.push(n);
                let vv = (i as f32 / total as f32).clamp(0.0, 1.0);
                self.uv.push([j as f32 / seg as f32, (row + 0.06 + vv * 0.88) / 8.0]);
                self.joint.push(bone as u16);
            }
        }
        let stride = seg as u32 + 1;
        for i in 0..total as u32 {
            for j in 0..seg as u32 {
                let a0 = start + i * stride + j;
                let b0 = a0 + stride;
                self.idx.extend_from_slice(&[a0, a0 + 1, b0, a0 + 1, b0 + 1, b0]);
            }
        }
    }

    fn sphere(&mut self, c: Vec3, r: Vec3, bone: usize, swatch: Swatch) {
        let (seg, rings) = (24usize, 16usize);
        let start = self.pos.len() as u32;
        let row = swatch as u32 as f32;
        for i in 0..=rings {
            let th = i as f32 / rings as f32 * PI;
            for j in 0..=seg {
                let ph = j as f32 / seg as f32 * TAU;
                let d = [th.sin() * ph.cos(), th.sin() * ph.sin(), th.cos()];
                self.pos.push([c[0] + d[0] * r[0], c[1] + d[1] * r[1], c[2] + d[2] * r[2]]);
                self.nrm.push(crate::math::normalize([d[0] / r[0], d[1] / r[1], d[2] / r[2]]));
                self.uv.push([j as f32 / seg as f32, (row + 0.06 + (i as f32 / rings as f32) * 0.88) / 8.0]);
                self.joint.push(bone as u16);
            }
        }
        let stride = seg as u32 + 1;
        for i in 0..rings as u32 {
            for j in 0..seg as u32 {
                let a0 = start + i * stride + j;
                let b0 = a0 + stride;
                self.idx.extend_from_slice(&[a0, b0, a0 + 1, a0 + 1, b0, b0 + 1]);
            }
        }
    }
}

fn body() -> (Mesh, Mesh) {
    let mut m = Mesh::default();
    for (side, s) in [(1.0f32, 0usize), (-1.0, 3)] {
        let y = 4.5 * side;
        m.capsule([0.0, y, 37.0], [0.0, y, 20.5], 3.7, 2.9, 11 + s, Swatch::Trousers, [1.0, 1.0], true);
        m.capsule([0.0, y, 20.5], [0.0, y * 1.05, 4.5], 2.9, 2.4, 12 + s, Swatch::Trousers, [1.0, 1.0], true);
        m.capsule([-0.8, y * 1.06, 6.0], [-0.5, y * 1.06, 1.6], 2.7, 2.6, 13 + s, Swatch::Boots, [1.0, 1.0], true);
        m.capsule([-1.0, y * 1.06, 1.6], [6.2, y * 1.06, 1.3], 2.4, 1.9, 13 + s, Swatch::Boots, [1.0, 0.8], true);
        let ay = 9.6 * side;
        m.sphere([0.0, ay, 55.2], [3.6, 3.6, 3.0], 2, Swatch::Brass);
        m.capsule([0.0, ay, 55.0], [0.3, ay * 1.08, 42.0], 2.9, 2.4, 5 + s, Swatch::Coat, [1.0, 1.0], true);
        m.capsule([0.3, ay * 1.08, 42.0], [1.6, ay * 1.12, 31.5], 2.4, 2.0, 6 + s, Swatch::Coat, [1.0, 1.0], true);
        m.capsule([1.6, ay * 1.12, 32.8], [1.7, ay * 1.12, 31.0], 2.3, 2.3, 6 + s, Swatch::Brass, [1.0, 1.0], false);
        m.sphere([2.0, ay * 1.13, 28.6], [2.1, 1.7, 2.6], 7 + s, Swatch::Gloves);
    }
    m.sphere([0.0, 0.0, 38.5], [5.2, 7.6, 5.0], 0, Swatch::Trousers);
    m.capsule([0.0, 0.0, 40.0], [0.0, 0.0, 51.0], 6.6, 7.6, 1, Swatch::Shirt, [0.9, 1.0], true);
    m.capsule([0.0, 0.0, 51.0], [0.0, 0.0, 56.5], 7.6, 5.0, 2, Swatch::Coat, [0.95, 1.1], true);
    // The coat: an open cone from the chest to below the knees, and its lapels.
    m.capsule([0.0, 0.0, 50.0], [-0.5, 0.0, 15.0], 7.9, 11.5, 0, Swatch::Coat, [0.95, 1.05], false);
    m.capsule([0.0, 0.0, 41.0], [0.0, 0.0, 42.5], 7.8, 7.8, 1, Swatch::Boots, [0.92, 1.0], false);
    m.sphere([7.2, 0.0, 41.8], [0.6, 1.4, 1.0], 1, Swatch::Brass);
    m.capsule([0.0, 0.0, 56.5], [0.4, 0.0, 61.0], 2.6, 2.4, 3, Swatch::Skin, [1.0, 1.0], false);
    m.sphere([0.9, 0.0, 65.5], [4.8, 4.4, 5.4], 4, Swatch::Skin);
    m.sphere([3.9, 0.0, 64.6], [1.3, 0.9, 1.4], 4, Swatch::Skin);
    m.sphere([-0.4, 0.0, 66.5], [4.9, 4.6, 4.6], 4, Swatch::Gloves);
    m.capsule([0.4, 0.0, 69.0], [0.3, 0.0, 75.0], 5.0, 4.6, 4, Swatch::Hat, [1.05, 1.0], true);
    m.capsule([0.4, 0.0, 69.3], [0.4, 0.0, 70.6], 5.15, 5.1, 4, Swatch::Brass, [1.05, 1.0], false);
    m.sphere([0.4, 0.0, 69.0], [9.6, 9.0, 0.6], 4, Swatch::Hat);
    let mut glow = Mesh::default();
    for side in [1.0f32, -1.0] {
        glow.sphere([4.75, 1.75 * side, 66.6], [0.75, 0.8, 0.55], 4, Swatch::Shirt);
    }
    glow.sphere([7.6, 2.8, 52.5], [0.5, 1.0, 1.0], 2, Swatch::Shirt);
    (m, glow)
}

fn noise(x: u32, y: u32, seed: u32) -> f32 {
    let mut h = x.wrapping_mul(374_761_393) ^ y.wrapping_mul(668_265_263) ^ seed.wrapping_mul(2_246_822_519);
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    ((h ^ (h >> 16)) & 0xffff) as f32 / 65535.0
}

fn srgb(c: f32) -> u8 {
    (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// Albedo, tint+rim mask and ORM atlases, 256 by 256.
fn atlases() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    const N: u32 = 256;
    // sRGB swatch colours, roughness, metalness, tintable.
    let swatches: [([f32; 3], f32, f32, bool); 8] = [
        ([0.80, 0.60, 0.48], 0.62, 0.0, false),
        ([0.86, 0.84, 0.80], 0.78, 0.0, true),
        ([0.20, 0.22, 0.30], 0.82, 0.0, false),
        ([0.30, 0.20, 0.14], 0.48, 0.0, false),
        ([0.88, 0.86, 0.80], 0.75, 0.0, false),
        ([0.16, 0.15, 0.16], 0.70, 0.0, false),
        ([0.86, 0.66, 0.34], 0.32, 1.0, false),
        ([0.09, 0.08, 0.08], 0.55, 0.0, false),
    ];
    let mut albedo = vec![0u8; (N * N * 4) as usize];
    let mut mask = vec![0u8; (N * N * 4) as usize];
    let mut orm = vec![0u8; (N * N * 4) as usize];
    for y in 0..N {
        let row = (y / 32) as usize;
        let (c, rough, metal, tintable) = swatches[row];
        for x in 0..N {
            let i = ((y * N + x) * 4) as usize;
            let fine = noise(x, y, 7) * 0.06 - 0.03;
            let coarse = noise(x / 8, y / 8, 3) * 0.08 - 0.04;
            // Cloth weave on the coat and trousers, seams every eighth of the circumference.
            let weave = if row == 1 || row == 2 { (((x + y) % 4) as f32 / 4.0 - 0.5) * 0.04 } else { 0.0 };
            let seam = if (row == 1 || row == 2 || row == 4) && x % 32 == 0 { -0.12 } else { 0.0 };
            let k = 1.0 + fine + coarse + weave + seam;
            albedo[i] = srgb(c[0] * k);
            albedo[i + 1] = srgb(c[1] * k);
            albedo[i + 2] = srgb(c[2] * k);
            albedo[i + 3] = 255;
            mask[i] = if tintable { 255 } else { 0 };
            mask[i + 1] = 255;
            mask[i + 2] = 0;
            mask[i + 3] = 255;
            orm[i] = 255;
            orm[i + 1] = srgb(rough + (noise(x, y, 11) - 0.5) * 0.06);
            orm[i + 2] = srgb(metal);
            orm[i + 3] = 255;
        }
    }
    (albedo, mask, orm)
}

fn png(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    use image::ImageEncoder;
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out).write_image(rgba, w, h, image::ExtendedColorType::Rgba8).expect("encode png");
    out
}

/// Source to glTF (vpkmerge's TRANSFORMSOURCETOGLTF): scale 0.0254 and Z-up to Y-up, so
/// Source +X (forward) becomes glTF +Z and Source +Z becomes +Y.
pub fn source_to_gltf() -> Mat4 {
    let k = 0.0254;
    // Columns: Source X -> glTF Z, Source Y -> glTF X, Source Z -> glTF Y.
    Mat4([[0.0, 0.0, k, 0.0], [k, 0.0, 0.0, 0.0], [0.0, k, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0]])
}

struct Bin {
    data: Vec<u8>,
    views: Vec<serde_json::Value>,
    accessors: Vec<serde_json::Value>,
}

impl Bin {
    fn push(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        while !self.data.len().is_multiple_of(4) {
            self.data.push(0);
        }
        let offset = self.data.len();
        self.data.extend_from_slice(bytes);
        let mut view = serde_json::json!({ "buffer": 0, "byteOffset": offset, "byteLength": bytes.len() });
        if let Some(t) = target {
            view["target"] = t.into();
        }
        self.views.push(view);
        self.views.len() - 1
    }

    fn accessor(&mut self, bytes: &[u8], component: u32, count: usize, ty: &str, target: Option<u32>, minmax: Option<(Vec<f32>, Vec<f32>)>) -> usize {
        let view = self.push(bytes, target);
        let mut a = serde_json::json!({ "bufferView": view, "componentType": component, "count": count, "type": ty });
        if let Some((min, max)) = minmax {
            a["min"] = min.into();
            a["max"] = max.into();
        }
        self.accessors.push(a);
        self.accessors.len() - 1
    }
}

fn f32s(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// The idle: breathing, a slow weight shift, the head looking around and the arms swaying.
fn idle_rotations(t: f32) -> Vec<(usize, Quat)> {
    let w = TAU / 3.2;
    let breathe = (t * w).sin();
    let sway = (t * w * 0.5).sin();
    vec![
        (0, quat_axis_angle([1.0, 0.0, 0.0], sway * 0.035)),
        (1, quat_axis_angle([0.0, 1.0, 0.0], breathe * 0.025)),
        (2, quat_mul(quat_axis_angle([0.0, 1.0, 0.0], -breathe * 0.03), quat_axis_angle([0.0, 0.0, 1.0], sway * 0.06))),
        (4, quat_mul(quat_axis_angle([0.0, 0.0, 1.0], (t * w * 0.5 + 0.6).sin() * 0.22), quat_axis_angle([0.0, 1.0, 0.0], breathe * 0.05))),
        (5, quat_axis_angle([1.0, 0.0, 0.0], 0.12 + breathe * 0.04)),
        (6, quat_axis_angle([0.0, 1.0, 0.0], -0.18 - breathe * 0.05)),
        (8, quat_axis_angle([1.0, 0.0, 0.0], -0.12 - breathe * 0.04)),
        (9, quat_axis_angle([0.0, 1.0, 0.0], -0.22 + breathe * 0.04)),
        (11, quat_axis_angle([1.0, 0.0, 0.0], -sway * 0.03)),
        (14, quat_axis_angle([1.0, 0.0, 0.0], -sway * 0.03)),
    ]
}

/// The stand-in hero as `.glb` bytes.
pub fn hero_glb(variant: Variant) -> Vec<u8> {
    let (body, glow) = body();
    let (albedo, mask, orm) = atlases();
    let mut bin = Bin { data: Vec::new(), views: Vec::new(), accessors: Vec::new() };
    let mut meshes = Vec::new();
    for (mesh, material) in [(&body, 0), (&glow, 1)] {
        let n = mesh.pos.len();
        let flat: Vec<f32> = mesh.pos.iter().flatten().copied().collect();
        let mut min = vec![f32::MAX; 3];
        let mut max = vec![f32::MIN; 3];
        for p in &mesh.pos {
            for i in 0..3 {
                min[i] = min[i].min(p[i]);
                max[i] = max[i].max(p[i]);
            }
        }
        let pos = bin.accessor(&f32s(&flat), 5126, n, "VEC3", Some(34962), Some((min, max)));
        let nrm = bin.accessor(&f32s(&mesh.nrm.iter().flatten().copied().collect::<Vec<_>>()), 5126, n, "VEC3", Some(34962), None);
        let uv = bin.accessor(&f32s(&mesh.uv.iter().flatten().copied().collect::<Vec<_>>()), 5126, n, "VEC2", Some(34962), None);
        let joints: Vec<u8> = mesh.joint.iter().flat_map(|&j| [j, 0, 0, 0]).flat_map(|v| v.to_le_bytes()).collect();
        let jo = bin.accessor(&joints, 5123, n, "VEC4", Some(34962), None);
        let weights: Vec<f32> = mesh.joint.iter().flat_map(|_| [1.0f32, 0.0, 0.0, 0.0]).collect();
        let we = bin.accessor(&f32s(&weights), 5126, n, "VEC4", Some(34962), None);
        let idx: Vec<u8> = mesh.idx.iter().flat_map(|i| i.to_le_bytes()).collect();
        let ix = bin.accessor(&idx, 5125, mesh.idx.len(), "SCALAR", Some(34963), None);
        meshes.push(serde_json::json!({
            "name": if material == 0 { "body" } else { "eyes" },
            "primitives": [{ "attributes": { "POSITION": pos, "NORMAL": nrm, "TEXCOORD_0": uv, "JOINTS_0": jo, "WEIGHTS_0": we }, "indices": ix, "material": material }]
        }));
    }
    // Skeleton: bone locals relative to their parent (bind rotations are identity).
    let ibm: Vec<f32> = BONES.iter().flat_map(|b| Mat4::translation([-b.head[0], -b.head[1], -b.head[2]]).to_cols_array()).collect();
    let ibm_acc = bin.accessor(&f32s(&ibm), 5126, BONES.len(), "MAT4", None, None);
    // Idle clip: 3.2 s at 30 keys per second.
    let keys: Vec<f32> = (0..=96).map(|i| i as f32 / 30.0).collect();
    let input = bin.accessor(&f32s(&keys), 5126, keys.len(), "SCALAR", None, Some((vec![0.0], vec![*keys.last().unwrap()])));
    let mut samplers = Vec::new();
    let mut channels = Vec::new();
    let tracks: Vec<(usize, Vec<Quat>)> = {
        let per_key: Vec<Vec<(usize, Quat)>> = keys.iter().map(|&t| idle_rotations(t)).collect();
        (0..per_key[0].len()).map(|k| (per_key[0][k].0, per_key.iter().map(|v| v[k].1).collect())).collect()
    };
    for (bone, rots) in tracks {
        let out = bin.accessor(&f32s(&rots.iter().flatten().copied().collect::<Vec<_>>()), 5126, rots.len(), "VEC4", None, None);
        samplers.push(serde_json::json!({ "input": input, "output": out, "interpolation": "LINEAR" }));
        channels.push(serde_json::json!({ "sampler": samplers.len() - 1, "target": { "node": bone + 1, "path": "rotation" } }));
    }
    // Pelvis bob (translation).
    let bob: Vec<f32> = keys.iter().flat_map(|&t| [0.0, 0.0, 38.0 + (t * TAU / 3.2).sin() * 0.25 - 0.1]).collect();
    let bob_acc = bin.accessor(&f32s(&bob), 5126, keys.len(), "VEC3", None, None);
    samplers.push(serde_json::json!({ "input": input, "output": bob_acc, "interpolation": "LINEAR" }));
    channels.push(serde_json::json!({ "sampler": samplers.len() - 1, "target": { "node": 1, "path": "translation" } }));
    // Images.
    let mut images = Vec::new();
    let white4 = vec![255u8; 4 * 4 * 4];
    for (w, h, data) in [(256, 256, &albedo), (256, 256, &mask), (256, 256, &orm), (4, 4, &white4)] {
        let view = bin.push(&png(w, h, data), None);
        images.push(serde_json::json!({ "bufferView": view, "mimeType": "image/png" }));
    }
    let (coat, glow_tint) = match variant {
        Variant::Stock => ([0.78f32, 0.70, 0.60], [1.0f32, 0.62, 0.22]),
        Variant::Modded => ([0.16f32, 0.42, 0.78], [0.25f32, 0.85, 1.0]),
    };
    let reflectivity = {
        let mut s = [0.0f64; 3];
        for p in albedo.as_chunks::<4>().0 {
            for c in 0..3 {
                s[c] += crate::material::srgb_to_linear(p[c] as f32 / 255.0) as f64;
            }
        }
        let n = (albedo.len() / 4) as f64;
        [s[0] / n, s[1] / n, s[2] / n]
    };
    let morphic_body = serde_json::json!({
        "schema_version": 4, "shader": "pbr.vfx", "blend_mode": "opaque",
        "flags": { "F_USE_NPR_LIGHTING": 1 },
        "ints": { "g_nTextureColorTintMode1": 0, "g_bMaskColorTint1": 1 },
        "floats": {},
        "vectors": { "g_vColorTint1": [coat[0], coat[1], coat[2], 1.0], "g_vAlbedoContrastSaturationBrightness1": [1.08, 1.04, 1.0, 0.0] },
        "textures": { "g_tColor": 0, "g_tTintMaskRimLightMask": 1, "g_tNprOutlineMask": 3 },
        "texture_reflectivity": { "g_tColor": [reflectivity[0], reflectivity[1], reflectivity[2], 0.0] }
    });
    let morphic_glow = serde_json::json!({
        "schema_version": 4, "shader": "pbr.vfx", "blend_mode": "opaque",
        "flags": { "F_USE_NPR_LIGHTING": 1, "F_SELF_ILLUM": 1, "F_DISABLE_NPR_OUTLINE": 1 },
        "floats": { "g_flSelfIllumScale1": 3.0, "g_flSelfIllumAlbedoFactor1": 0.0 },
        "vectors": { "g_vSelfIllumTint1": [glow_tint[0], glow_tint[1], glow_tint[2], 1.0] },
        "textures": { "g_tSelfIllumMask": 3 }
    });
    let mut nodes = vec![serde_json::json!({ "name": "skeleton", "matrix": source_to_gltf().to_cols_array(), "children": [1] })];
    for (i, b) in BONES.iter().enumerate() {
        let local = match b.parent {
            Some(p) => crate::math::sub(b.head, BONES[p].head),
            None => b.head,
        };
        let children: Vec<usize> = BONES.iter().enumerate().filter(|(_, c)| c.parent == Some(i)).map(|(j, _)| j + 1).collect();
        let mut node = serde_json::json!({ "name": b.name, "translation": local, "rotation": [0.0, 0.0, 0.0, 1.0] });
        if !children.is_empty() {
            node["children"] = children.into();
        }
        nodes.push(node);
    }
    let first_mesh_node = nodes.len();
    for m in 0..meshes.len() {
        nodes.push(serde_json::json!({ "name": format!("mesh_{m}"), "mesh": m, "skin": 0, "matrix": source_to_gltf().to_cols_array() }));
    }
    let doc = serde_json::json!({
        "asset": { "version": "2.0", "generator": "sigil-viewer fixture" },
        "scene": 0,
        "scenes": [{ "nodes": [0, first_mesh_node, first_mesh_node + 1] }],
        "nodes": nodes,
        "meshes": meshes,
        "skins": [{ "joints": (1..=BONES.len()).collect::<Vec<_>>(), "inverseBindMatrices": ibm_acc, "skeleton": 0 }],
        "animations": [{ "name": "hero_shop", "samplers": samplers, "channels": channels }],
        "materials": [
            {
                "name": "stand_in_body",
                "pbrMetallicRoughness": { "baseColorFactor": [coat[0], coat[1], coat[2], 1.0], "baseColorTexture": { "index": 0 }, "metallicRoughnessTexture": { "index": 2 }, "metallicFactor": 1.0, "roughnessFactor": 1.0 },
                "extras": { "morphic": morphic_body }
            },
            {
                "name": "stand_in_eyes",
                "pbrMetallicRoughness": { "baseColorFactor": [1.0, 1.0, 1.0, 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.4 },
                "emissiveTexture": { "index": 3 },
                "emissiveFactor": glow_tint,
                "extras": { "morphic": morphic_glow }
            }
        ],
        "textures": [{ "source": 0, "sampler": 0 }, { "source": 1, "sampler": 0 }, { "source": 2, "sampler": 0 }, { "source": 3, "sampler": 0 }],
        "samplers": [{ "magFilter": 9729, "minFilter": 9987, "wrapS": 10497, "wrapT": 10497 }],
        "images": images,
        "bufferViews": bin.views,
        "accessors": bin.accessors,
        "buffers": [{ "byteLength": bin.data.len() }]
    });
    glb(&serde_json::to_vec(&doc).expect("json"), bin.data)
}

/// Frames a JSON chunk and a binary chunk as a GLB.
pub fn glb(json: &[u8], mut bin: Vec<u8>) -> Vec<u8> {
    let mut json = json.to_vec();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let total = 12 + 8 + json.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    out
}

/// The sidecar vpkmerge's pipeline writes beside a hero, for the stand-in.
pub fn sidecar_json() -> String {
    serde_json::json!({ "clips": [{ "name": "hero_shop", "duration": 3.2 }], "idleClip": "hero_shop" }).to_string()
}

/// A stand-in Hideout room: a floor, two walls, a desk and a lamp, as one unskinned glb with
/// plain glTF materials (no `extras.morphic`, so it takes the PBR path like map geometry).
pub fn room_glb() -> Vec<u8> {
    // (min, max, material)
    let boxes: [([f32; 3], [f32; 3], usize); 7] = [
        ([-4.0, -0.1, -4.0], [4.0, 0.0, 4.0], 0),
        ([-4.0, 0.0, -4.0], [4.0, 3.2, -3.8], 1),
        ([-4.0, 0.0, -3.8], [-3.8, 3.2, 4.0], 1),
        ([1.4, 0.0, -3.6], [3.4, 0.78, -2.6], 2),
        ([1.5, 0.78, -3.5], [3.3, 0.82, -2.7], 2),
        ([-3.6, 0.0, -3.6], [-2.4, 2.0, -3.1], 2),
        ([2.4, 0.82, -3.3], [2.6, 1.3, -3.1], 3),
    ];
    let mut pos: Vec<f32> = Vec::new();
    let mut nrm: Vec<f32> = Vec::new();
    let mut uv: Vec<f32> = Vec::new();
    let mut prims: Vec<(usize, Vec<u32>)> = (0..4).map(|m| (m, Vec::new())).collect();
    for (min, max, m) in boxes {
        let faces: [([f32; 3], [usize; 3]); 6] = [
            ([1.0, 0.0, 0.0], [0, 1, 2]),
            ([-1.0, 0.0, 0.0], [0, 1, 2]),
            ([0.0, 1.0, 0.0], [1, 2, 0]),
            ([0.0, -1.0, 0.0], [1, 2, 0]),
            ([0.0, 0.0, 1.0], [2, 0, 1]),
            ([0.0, 0.0, -1.0], [2, 0, 1]),
        ];
        for (n, [a, b, c]) in faces {
            let start = (pos.len() / 3) as u32;
            let fixed = if n[a] > 0.0 { max[a] } else { min[a] };
            for (u, v) in [(0.0f32, 0.0f32), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
                let mut p = [0.0f32; 3];
                p[a] = fixed;
                p[b] = min[b] + (max[b] - min[b]) * u;
                p[c] = min[c] + (max[c] - min[c]) * v;
                pos.extend_from_slice(&p);
                nrm.extend_from_slice(&n);
                uv.extend_from_slice(&[u * (max[b] - min[b]), v * (max[c] - min[c])]);
            }
            // Room materials are double sided, so winding does not matter.
            prims[m].1.extend_from_slice(&[start, start + 1, start + 2, start, start + 2, start + 3]);
        }
    }
    let mut bin = Bin { data: Vec::new(), views: Vec::new(), accessors: Vec::new() };
    let n = pos.len() / 3;
    let mut min = vec![f32::MAX; 3];
    let mut max = vec![f32::MIN; 3];
    for p in pos.as_chunks::<3>().0 {
        for i in 0..3 {
            min[i] = min[i].min(p[i]);
            max[i] = max[i].max(p[i]);
        }
    }
    let pa = bin.accessor(&f32s(&pos), 5126, n, "VEC3", Some(34962), Some((min, max)));
    let na = bin.accessor(&f32s(&nrm), 5126, n, "VEC3", Some(34962), None);
    let ua = bin.accessor(&f32s(&uv), 5126, n, "VEC2", Some(34962), None);
    let primitives: Vec<serde_json::Value> = prims
        .iter()
        .filter(|(_, idx)| !idx.is_empty())
        .map(|(m, idx)| {
            let bytes: Vec<u8> = idx.iter().flat_map(|i| i.to_le_bytes()).collect();
            let ia = bin.accessor(&bytes, 5125, idx.len(), "SCALAR", Some(34963), None);
            serde_json::json!({ "attributes": { "POSITION": pa, "NORMAL": na, "TEXCOORD_0": ua }, "indices": ia, "material": m })
        })
        .collect();
    let doc = serde_json::json!({
        "asset": { "version": "2.0", "generator": "sigil-viewer fixture" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "name": "room", "mesh": 0 }],
        "meshes": [{ "name": "room", "primitives": primitives }],
        "materials": [
            { "name": "floor_boards", "pbrMetallicRoughness": { "baseColorFactor": [0.20, 0.12, 0.07, 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.55 }, "doubleSided": true },
            { "name": "wallpaper", "pbrMetallicRoughness": { "baseColorFactor": [0.18, 0.22, 0.20, 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.85 }, "doubleSided": true },
            { "name": "wood", "pbrMetallicRoughness": { "baseColorFactor": [0.12, 0.06, 0.03, 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.4 }, "doubleSided": true },
            { "name": "lamp", "pbrMetallicRoughness": { "baseColorFactor": [1.0, 0.8, 0.5, 1.0] }, "emissiveFactor": [1.0, 0.7, 0.35], "extensions": { "KHR_materials_emissive_strength": { "emissiveStrength": 6.0 } } }
        ],
        "extensionsUsed": ["KHR_materials_emissive_strength"],
        "bufferViews": bin.views,
        "accessors": bin.accessors,
        "buffers": [{ "byteLength": bin.data.len() }]
    });
    glb(&serde_json::to_vec(&doc).expect("json"), bin.data)
}

/// Writes a stand-in Hideout export into `dir`: `scene.json` (schema 1, the contract's field
/// names), the room glb, the hero glb and sidecar, and six Radiance sky faces. Returns the
/// scene file's path.
pub fn write_hideout(dir: &std::path::Path, variant: Variant) -> std::io::Result<std::path::PathBuf> {
    std::fs::create_dir_all(dir.join("models"))?;
    std::fs::create_dir_all(dir.join("sky"))?;
    std::fs::create_dir_all(dir.join("hero"))?;
    std::fs::write(dir.join("models/room.glb"), room_glb())?;
    std::fs::write(dir.join("hero/stand_in.glb"), hero_glb(variant))?;
    std::fs::write(dir.join("hero/stand_in.json"), sidecar_json())?;
    // Sky faces in Source axes: dusk above, dark below.
    for (name, up) in [("px", 0.0f32), ("nx", 0.0), ("py", 0.0), ("ny", 0.0), ("pz", 1.0), ("nz", -1.0)] {
        let edge = 16u32;
        let mut px = Vec::with_capacity((edge * edge) as usize);
        for y in 0..edge {
            let t = if up != 0.0 { (up + 1.0) * 0.5 } else { 1.0 - y as f32 / (edge - 1) as f32 };
            let c = [0.05 + 0.25 * t, 0.06 + 0.18 * t, 0.10 + 0.25 * t];
            for _ in 0..edge {
                px.push(image::Rgb(c));
            }
        }
        let mut bytes = Vec::new();
        image::codecs::hdr::HdrEncoder::new(&mut bytes).encode(&px, edge as usize, edge as usize).map_err(std::io::Error::other)?;
        std::fs::write(dir.join(format!("sky/{name}.hdr")), bytes)?;
    }
    let anchor = Mat4::from_trs([0.0, 0.0, -0.6], [0.0, 0.0, 0.0, 1.0], [1.0; 3]).to_cols_array();
    let identity = Mat4::IDENTITY.to_cols_array();
    let faces = ["sky/px.hdr", "sky/nx.hdr", "sky/py.hdr", "sky/ny.hdr", "sky/pz.hdr", "sky/nz.hdr"];
    let scene = serde_json::json!({
        "schema": 1,
        "map": "stand_in_hideout",
        "variant": "original",
        "units": "meters",
        "up": "+Y",
        "models": [{ "glb": "models/room.glb", "transform": identity, "tint": [1, 1, 1, 1], "primitive": null, "animation": null }],
        "lights": [
            { "kind": "spot", "color": [1.0, 0.86, 0.68], "intensity": 9.0, "position": [1.2, 2.9, 1.6], "direction": [-0.4, -0.85, -0.6], "range": 9.0, "cone": [25, 50], "realtime": true },
            { "kind": "point", "color": [1.0, 0.72, 0.42], "intensity": 1.2, "position": [2.5, 1.45, -3.0], "range": 4.0, "realtime": true }
        ],
        "fog": { "color": [0.16, 0.15, 0.17], "strength": 0.5, "indirectStrength": 1.0, "drawDistance": 14.0, "fadeIn": [3, 5] },
        "sky": { "cubemap": faces, "exposureBias": 0.0, "tint": [1, 1, 1], "position": null },
        "reflection": { "cubemap": faces, "exposureBias": -1.0, "tint": [1, 1, 1], "position": [0, 1.5, 0] },
        "camera": { "position": [0.9, 1.55, 2.4], "target": [0.0, 1.0, -0.6], "fov": 60.0 },
        "heroAnchor": anchor,
        "hero": { "hero": "stand_in", "glb": "hero/stand_in.glb", "sidecar": "hero/stand_in.json", "transform": anchor },
        "spawn": { "position": [0.0, 0.0, -0.6], "yawDegrees": 0.0 },
        "failures": [],
        "omissions": ["autoExposure", "colorCorrection", "lightProbes", "particles", "tonemap"],
        "pipeline": "sigil-viewer-1"
    });
    let path = dir.join("scene.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&scene).map_err(std::io::Error::other)?)?;
    Ok(path)
}
