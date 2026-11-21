//! The stand-in Hideout's baked data, made in code so the headless scenes exercise every
//! Hideout path without Deadlock: a room glb with lightmap UVs (`TEXCOORD_3`), a lightmap
//! baked here from the room's lamp (DC irradiance, the SH2 direction and a stationary light's
//! shadow mask), a light probe volume, a fog volume with its lights' precomputed shadow atlas,
//! a window cookie, a colour grade and the map's tone curve and bloom. `upgrade_hideout` rewrites a scene `fixture::write_hideout`
//! made. Nothing here is a Valve asset.

use std::path::Path;

use crate::math::{Vec3, add, dot, length, normalize, scale, sub};

/// (min, max, material) boxes: the floor, two walls, the desk and its top, a cabinet and the
/// lamp (the same room as `fixture::room_glb`).
const BOXES: [([f32; 3], [f32; 3], usize); 7] = [
    ([-4.0, -0.1, -4.0], [4.0, 0.0, 4.0], 0),
    ([-4.0, 0.0, -4.0], [4.0, 3.2, -3.8], 1),
    ([-4.0, 0.0, -3.8], [-3.8, 3.2, 4.0], 1),
    ([1.4, 0.0, -3.6], [3.4, 0.78, -2.6], 2),
    ([1.5, 0.78, -3.5], [3.3, 0.82, -2.7], 2),
    ([-3.6, 0.0, -3.6], [-2.4, 2.0, -3.1], 2),
    ([2.4, 0.82, -3.3], [2.6, 1.3, -3.1], 3),
];

/// The baked lamp (fog only at run time; its light is in the lightmap).
const LAMP: Vec3 = [2.5, 1.45, -3.0];
const LAMP_COLOR: Vec3 = [1.0, 0.62, 0.3];
/// The stationary spot: drawn live, its static shadow baked into channel 0.
const SPOT: Vec3 = [1.2, 2.9, 1.6];
const SPOT_DIR: Vec3 = [-0.4, -0.85, -0.6];
const SKY_AMBIENT: Vec3 = [0.035, 0.04, 0.06];

const GRID: usize = 7;
const CELL: usize = 32;

struct Quad {
    corners: [Vec3; 4],
    normal: Vec3,
    material: usize,
    cell: usize,
}

fn quads() -> Vec<Quad> {
    let mut out = Vec::new();
    for (min, max, m) in BOXES {
        let faces: [([f32; 3], [usize; 3]); 6] = [
            ([1.0, 0.0, 0.0], [0, 1, 2]),
            ([-1.0, 0.0, 0.0], [0, 1, 2]),
            ([0.0, 1.0, 0.0], [1, 2, 0]),
            ([0.0, -1.0, 0.0], [1, 2, 0]),
            ([0.0, 0.0, 1.0], [2, 0, 1]),
            ([0.0, 0.0, -1.0], [2, 0, 1]),
        ];
        for (n, [a, b, c]) in faces {
            let fixed = if n[a] > 0.0 { max[a] } else { min[a] };
            let corners = [(0.0f32, 0.0f32), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].map(|(u, v)| {
                let mut p = [0.0f32; 3];
                p[a] = fixed;
                p[b] = min[b] + (max[b] - min[b]) * u;
                p[c] = min[c] + (max[c] - min[c]) * v;
                p
            });
            let cell = out.len();
            out.push(Quad { corners, normal: n, material: m, cell });
        }
    }
    out
}

fn bilerp(c: &[Vec3; 4], u: f32, v: f32) -> Vec3 {
    let bottom = add(scale(c[0], 1.0 - u), scale(c[1], u));
    let top = add(scale(c[3], 1.0 - u), scale(c[2], u));
    add(scale(bottom, 1.0 - v), scale(top, v))
}

/// Whether the segment from `p` to `light` passes through a box other than the floor.
fn blocked(p: Vec3, light: Vec3) -> bool {
    let d = sub(light, p);
    BOXES[3..6].iter().any(|(min, max, _)| {
        let (mut t0, mut t1) = (1e-3f32, 1.0f32);
        for k in 0..3 {
            if d[k].abs() < 1e-6 {
                if p[k] < min[k] || p[k] > max[k] {
                    return false;
                }
                continue;
            }
            let (a, b) = ((min[k] - p[k]) / d[k], (max[k] - p[k]) / d[k]);
            t0 = t0.max(a.min(b));
            t1 = t1.min(a.max(b));
        }
        t0 < t1
    })
}

fn lamp_irradiance(p: Vec3, n: Vec3) -> Vec3 {
    let to = sub(LAMP, p);
    let d = length(to).max(0.05);
    let cos = dot(n, scale(to, 1.0 / d)).max(0.0);
    let k = if blocked(p, LAMP) { 0.0 } else { 1.6 * cos / (d * d) };
    add(scale(LAMP_COLOR, k), SKY_AMBIENT)
}

fn png(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    use image::ImageEncoder;
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out).write_image(rgba, w, h, image::ExtendedColorType::Rgba8).expect("png");
    out
}

fn vtv(w: u32, h: u32, d: u32, texels: &[[f32; 4]]) -> Vec<u8> {
    let mut b = Vec::new();
    for x in [crate::hideout::VTV_MAGIC, w, h, d, 2] {
        b.extend_from_slice(&x.to_le_bytes());
    }
    for t in texels {
        for c in t {
            b.extend_from_slice(&crate::gpu::f16_bits(*c).to_le_bytes());
        }
    }
    b
}

/// The room with lightmap UVs in `TEXCOORD_3`, one atlas cell per face.
pub fn room_glb_lightmapped() -> Vec<u8> {
    let (mut pos, mut nrm, mut uv, mut lm) = (Vec::<f32>::new(), Vec::<f32>::new(), Vec::<f32>::new(), Vec::<f32>::new());
    let mut prims: Vec<Vec<u32>> = vec![Vec::new(); 4];
    let atlas = (GRID * CELL) as f32;
    for q in quads() {
        let start = (pos.len() / 3) as u32;
        let (cx, cy) = ((q.cell % GRID) as f32, (q.cell / GRID) as f32);
        for (i, (u, v)) in [(0.0f32, 0.0f32), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].into_iter().enumerate() {
            pos.extend_from_slice(&q.corners[i]);
            nrm.extend_from_slice(&q.normal);
            let size = sub(q.corners[2], q.corners[0]);
            uv.extend_from_slice(&[u * length(size), v * length(size)]);
            // Texel centres of the cell's inner area (half a texel in from each side).
            let inset = 0.5 / CELL as f32;
            lm.extend_from_slice(&[(cx + inset + u * (1.0 - 2.0 * inset)) * CELL as f32 / atlas, (cy + inset + v * (1.0 - 2.0 * inset)) * CELL as f32 / atlas]);
        }
        prims[q.material].extend_from_slice(&[start, start + 1, start + 2, start, start + 2, start + 3]);
    }
    let mut bin: Vec<u8> = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut push = |bytes: Vec<u8>, count: usize, ty: &str, component: u32, target: u32, minmax: Option<(Vec<f32>, Vec<f32>)>| {
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        views.push(serde_json::json!({ "buffer": 0, "byteOffset": bin.len(), "byteLength": bytes.len(), "target": target }));
        bin.extend_from_slice(&bytes);
        let mut a = serde_json::json!({ "bufferView": views.len() - 1, "componentType": component, "count": count, "type": ty });
        if let Some((min, max)) = minmax {
            a["min"] = min.into();
            a["max"] = max.into();
        }
        accessors.push(a);
        accessors.len() - 1
    };
    let f = |v: &[f32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    let n = pos.len() / 3;
    let pa = push(f(&pos), n, "VEC3", 5126, 34962, Some((vec![-4.0, -0.1, -4.0], vec![4.0, 3.2, 4.0])));
    let na = push(f(&nrm), n, "VEC3", 5126, 34962, None);
    let ua = push(f(&uv), n, "VEC2", 5126, 34962, None);
    let la = push(f(&lm), n, "VEC2", 5126, 34962, None);
    let mut primitives = Vec::new();
    for (m, idx) in prims.iter().enumerate().filter(|(_, i)| !i.is_empty()) {
        let ia = push(idx.iter().flat_map(|i| i.to_le_bytes()).collect(), idx.len(), "SCALAR", 5125, 34963, None);
        primitives.push(serde_json::json!({ "attributes": { "POSITION": pa, "NORMAL": na, "TEXCOORD_0": ua, "TEXCOORD_3": la }, "indices": ia, "material": m }));
    }
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
        "bufferViews": views,
        "accessors": accessors,
        "buffers": [{ "byteLength": bin.len() }]
    });
    crate::fixture::glb(&serde_json::to_vec(&doc).expect("json"), bin)
}

/// The lightmap atlas: DC irradiance (linear), the packed L1 direction toward the lamp, and
/// the spot's baked shadow in channel 0 (1 where the desk or cabinet hides the spot).
pub fn bake() -> (Vec<image::Rgb<f32>>, Vec<u8>, Vec<u8>, u32) {
    let edge = (GRID * CELL) as u32;
    let mut dc = vec![image::Rgb([0.0f32; 3]); (edge * edge) as usize];
    let mut dir = vec![128u8; (edge * edge * 4) as usize];
    let mut shadow = vec![0u8; (edge * edge * 4) as usize];
    for q in quads() {
        let (cx, cy) = (q.cell % GRID, q.cell / GRID);
        for y in 0..CELL {
            for x in 0..CELL {
                let (u, v) = ((x as f32 + 0.5) / CELL as f32, (y as f32 + 0.5) / CELL as f32);
                let p = add(bilerp(&q.corners, u, v), scale(q.normal, 0.01));
                let e = lamp_irradiance(p, q.normal);
                let i = (cy * CELL + y) * edge as usize + cx * CELL + x;
                // Irradiance times PI: the shader divides the DC term by PI.
                dc[i] = image::Rgb(e.map(|c| c * std::f32::consts::PI));
                // L1 toward the lamp, in Source axes, packed the way the shader unpacks it
                // (band = (-z, -x, y) after the 0..1 to -1..1 and sqrt(3) scale).
                let to = normalize(sub(LAMP, p));
                let src = [to[2], to[0], to[1]];
                let band = [-src[1], src[2], -src[0]].map(|c| c * 0.35 / 3f32.sqrt());
                let k = i * 4;
                for c in 0..3 {
                    dir[k + c] = ((band[c] * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0) as u8;
                }
                dir[k + 3] = 255;
                shadow[k] = if blocked(p, SPOT) { 255 } else { 0 };
                shadow[k + 3] = 255;
            }
        }
    }
    (dc, dir, shadow, edge)
}

/// A 4 by 3 by 4 probe volume over the room, six faces per voxel stacked in depth (RGBA16F).
fn probes() -> (Vec<u8>, [u32; 3]) {
    let size = [4u32, 3, 4];
    let faces: [Vec3; 6] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [-1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, -1.0]];
    let mut texels = Vec::new();
    for (f, face) in faces.iter().enumerate() {
        // Source axes to glTF for the face normal.
        let n = [face[1], face[2], face[0]];
        let _ = f;
        for z in 0..size[2] {
            for y in 0..size[1] {
                for x in 0..size[0] {
                    let p = [-4.0 + 8.0 * (x as f32 + 0.5) / size[0] as f32, 3.2 * (y as f32 + 0.5) / size[1] as f32, -4.0 + 8.0 * (z as f32 + 0.5) / size[2] as f32];
                    let e = lamp_irradiance(p, n);
                    texels.push([e[0], e[1], e[2], 1.0]);
                }
            }
        }
    }
    (vtv(size[0], size[1], size[2] * 6, &texels), [size[0], size[1], size[2] * 6])
}

/// A warm grade: lifted, teal-leaning shadows and amber highlights (RGBA8, 16^3, x fastest).
fn lut() -> Vec<u8> {
    let dim = 16usize;
    let mut out = Vec::with_capacity(dim * dim * dim * 4);
    for b in 0..dim {
        for g in 0..dim {
            for r in 0..dim {
                let c = [r as f32 / 15.0, g as f32 / 15.0, b as f32 / 15.0];
                let l = c[0] * 0.3 + c[1] * 0.59 + c[2] * 0.11;
                let graded = [c[0] + 0.05 * l - 0.02 * (1.0 - l), c[1] + 0.01, c[2] - 0.04 * l + 0.03 * (1.0 - l)];
                for v in graded {
                    out.push((v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
                }
                out.push(255);
            }
        }
    }
    out
}

/// Window blinds: horizontal bars.
fn cookie() -> Vec<u8> {
    let edge = 128u32;
    let mut px = Vec::with_capacity((edge * edge * 4) as usize);
    for y in 0..edge {
        for x in 0..edge {
            let r = ((x as f32 / edge as f32 - 0.5).powi(2) + (y as f32 / edge as f32 - 0.5).powi(2)).sqrt();
            let bar = if (y / 12) % 2 == 0 { 255u8 } else { 70 };
            let v = if r > 0.5 { 0 } else { bar };
            px.extend_from_slice(&[v, v, v, 255]);
        }
    }
    png(edge, edge, &px)
}

/// The map from a world point to a box's [0,1] coordinates, column major.
/// The nearest box a ray from `o` along `d` hits, as the ray parameter.
fn ray_hit(o: Vec3, d: Vec3) -> Option<f32> {
    let mut best: Option<f32> = None;
    for (min, max, _) in BOXES {
        let (mut t0, mut t1) = (0.0f32, f32::MAX);
        for a in 0..3 {
            if d[a].abs() < 1e-9 {
                if o[a] < min[a] || o[a] > max[a] {
                    t1 = -1.0;
                }
                continue;
            }
            let (u, v) = ((min[a] - o[a]) / d[a], (max[a] - o[a]) / d[a]);
            t0 = t0.max(u.min(v));
            t1 = t1.min(u.max(v));
        }
        if t1 >= t0 && t1 > 0.0 && best.is_none_or(|b| t0 < b) {
            best = Some(t0);
        }
    }
    best
}

/// The barn tile edge and the omni face edge in the fog shadow atlas.
const BARN_TILE: u32 = 128;
const OMNI_TILE: u32 = 64;
const ATLAS_W: u32 = BARN_TILE + 3 * OMNI_TILE;
/// The spot's barn frustum: one inch near plane, its outer cone (`size_params`).
const BARN_NEAR: f32 = 1.0;

fn spot_basis() -> [Vec3; 3] {
    let spot = normalize(SPOT_DIR);
    let up = normalize(sub([0.0, 1.0, 0.0], scale(spot, spot[1])));
    [spot, crate::math::cross(up, spot), up]
}

/// Source's axes in glTF, for the lamp: forward +Z, left +X, up +Y.
const LAMP_BASIS: [Vec3; 3] = [[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];

/// The fog lights' precomputed shadow atlas the game bakes (`precomputed_shadowmap_atlas`):
/// the spot's barn tile and the lamp's six omni faces, each texel `1 - ndc.z` of the nearest
/// surface in the light's own frustum, ray traced here against the room's boxes. Returns the
/// VTV (R16) and each light's tiles as `[w, h, x, y]` in atlas uv.
pub fn fog_shadows() -> (Vec<u8>, Vec<[f32; 4]>, Vec<[f32; 4]>) {
    let (w, h) = (ATLAS_W, BARN_TILE);
    let mut texels = vec![0u16; (w * h) as usize];
    let inch = 0.0254f32;
    let size = (50f32).to_radians().tan() * BARN_NEAR;
    let range = 9.0 / inch;
    let basis = spot_basis();
    let eye = sub(SPOT, scale(basis[0], BARN_NEAR * inch));
    for y in 0..BARN_TILE {
        for x in 0..BARN_TILE {
            let ndc = [1.0 - 2.0 * (x as f32 + 0.5) / BARN_TILE as f32, 1.0 - 2.0 * (y as f32 + 0.5) / BARN_TILE as f32];
            let local = [1.0, ndc[0] * size / BARN_NEAR, ndc[1] * size / BARN_NEAR];
            let d = add(add(scale(basis[0], local[0]), scale(basis[1], local[1])), scale(basis[2], local[2]));
            let Some(s) = ray_hit(eye, d) else { continue };
            let rx = s / inch;
            let far = BARN_NEAR + range;
            let z = far / range - far * BARN_NEAR / (range * rx);
            if z > 0.0 && z < 1.0 {
                texels[(y * w + x) as usize] = ((1.0 - z) * 65535.0) as u16;
            }
        }
    }
    let lamp_range = 4.0 / inch;
    let mut omni = Vec::new();
    for face in 0..6u32 {
        let (ox, oy) = (BARN_TILE + (face % 3) * OMNI_TILE, (face / 3) * OMNI_TILE);
        omni.push([OMNI_TILE as f32 / w as f32, OMNI_TILE as f32 / h as f32, ox as f32 / w as f32, oy as f32 / h as f32]);
        for y in 0..OMNI_TILE {
            for x in 0..OMNI_TILE {
                let v = [1.0 - 2.0 * (x as f32 + 0.5) / OMNI_TILE as f32, 1.0 - 2.0 * (y as f32 + 0.5) / OMNI_TILE as f32, 1.0];
                // Each face's view axes back to the light's local ones (the shader's inverse).
                let l = match face {
                    0 => [v[2], v[0], v[1]],
                    5 => [-v[2], -v[0], v[1]],
                    3 => [-v[0], v[2], v[1]],
                    4 => [v[0], -v[2], v[1]],
                    2 => [-v[1], v[0], v[2]],
                    _ => [v[1], v[0], -v[2]],
                };
                let d = add(add(scale(LAMP_BASIS[0], l[0]), scale(LAMP_BASIS[1], l[1])), scale(LAMP_BASIS[2], l[2]));
                let Some(s) = ray_hit(LAMP, d) else { continue };
                let vz = s / inch;
                let far = 1.0 + lamp_range;
                let z = far / (far - 1.0) - far / ((far - 1.0) * vz);
                if z > 0.0 && z < 1.0 {
                    texels[((oy + y) * w + ox + x) as usize] = ((1.0 - z) * 65535.0) as u16;
                }
            }
        }
    }
    let mut out = Vec::with_capacity(20 + texels.len() * 2);
    for word in [crate::hideout::VTV_MAGIC, w, h, 1, 3] {
        out.extend_from_slice(&word.to_le_bytes());
    }
    out.extend(texels.iter().flat_map(|t| t.to_le_bytes()));
    (out, vec![[BARN_TILE as f32 / w as f32, 1.0, 0.0, 0.0]], omni)
}

fn world_to_box(min: Vec3, max: Vec3) -> [f32; 16] {
    let s = [1.0 / (max[0] - min[0]), 1.0 / (max[1] - min[1]), 1.0 / (max[2] - min[2])];
    [s[0], 0.0, 0.0, 0.0, 0.0, s[1], 0.0, 0.0, 0.0, 0.0, s[2], 0.0, -min[0] * s[0], -min[1] * s[1], -min[2] * s[2], 1.0]
}

/// Rewrites the stand-in Hideout at `scene` (made by `fixture::write_hideout`) with the
/// lightmapped room and every baked part of the contract.
pub fn upgrade_hideout(scene: &Path) -> std::io::Result<()> {
    let dir = scene.parent().unwrap_or(Path::new("."));
    for sub in ["lightmap", "probes", "post", "cookies", "fog"] {
        std::fs::create_dir_all(dir.join(sub))?;
    }
    std::fs::write(dir.join("models/room.glb"), room_glb_lightmapped())?;
    let (dc, direction, shadow, edge) = bake();
    let mut hdr = Vec::new();
    image::codecs::hdr::HdrEncoder::new(&mut hdr).encode(&dc, edge as usize, edge as usize).map_err(std::io::Error::other)?;
    std::fs::write(dir.join("lightmap/irradiance.hdr"), hdr)?;
    std::fs::write(dir.join("lightmap/direction.png"), png(edge, edge, &direction))?;
    std::fs::write(dir.join("lightmap/shadows.png"), png(edge, edge, &shadow))?;
    let (volume, atlas) = probes();
    std::fs::write(dir.join("probes/0.vtv"), volume)?;
    std::fs::write(dir.join("post/color-correction.bin"), lut())?;
    std::fs::write(dir.join("cookies/blinds.png"), cookie())?;
    // The map's candle and dust, and the hero's ember plume beside its sidecar.
    crate::particles::fixture::write_hideout_fixture(&dir.join("particles"))?;
    crate::particles::fixture::write_fixture(&dir.join("hero/particles"))?;
    let side_path = dir.join("hero/stand_in.json");
    if let Ok(text) = std::fs::read_to_string(&side_path)
        && let Ok(mut side) = serde_json::from_str::<serde_json::Value>(&text)
    {
        side["particles"] = "particles/bundle.json".into();
        std::fs::write(&side_path, side.to_string())?;
    }
    let text = std::fs::read_to_string(scene)?;
    let mut v: serde_json::Value = serde_json::from_str(&text).map_err(std::io::Error::other)?;
    let [spot, left, up] = spot_basis();
    let (shadow_atlas, barn, omni) = fog_shadows();
    std::fs::write(dir.join("fog/shadows.vtv"), shadow_atlas)?;
    let barn_size = (50f32).to_radians().tan() * BARN_NEAR;
    v["exposure"] = 1.4.into();
    v["lights"] = serde_json::json!([
        { "kind": "spot", "color": [1.0, 0.86, 0.68], "intensity": 9.0, "brightness": 0.5, "skirt": 0.2, "position": SPOT, "direction": SPOT_DIR,
          "range": 9.0, "cone": [25, 50], "realtime": true, "shadowChannel": 0, "fogStrength": 0.4, "fogOnly": false,
          "cookie": "cookies/blinds.png", "basis": [spot, left, up],
          "fogShadow": { "tiles": barn, "size": [barn_size, barn_size, 1.0 / BARN_NEAR], "shear": [0.0, 0.0] } },
        { "kind": "point", "color": [1.0, 0.72, 0.42], "intensity": 1.2, "brightness": 0.35, "skirt": 0.2, "position": LAMP, "range": 4.0,
          "realtime": false, "fogStrength": 2.0, "fogOnly": true, "cookie": null, "basis": LAMP_BASIS,
          "fogShadow": { "tiles": omni, "size": [0.0, 0.0, 0.0], "shear": [0.0, 0.0] } }
    ]);
    v["lightmap"] = serde_json::json!({ "file": "lightmap/irradiance.hdr", "uvScale": [1.0, 1.0], "uvSet": 3, "direction": "lightmap/direction.png", "shadows": "lightmap/shadows.png" });
    v["probes"] = serde_json::json!([{ "file": "probes/0.vtv", "shadows": null, "worldToLocal": world_to_box([-4.0, 0.0, -4.0], [4.0, 3.2, 4.0]),
        "size": [4, 3, 4], "offset": [0, 0, 0], "atlasSize": atlas, "fade": [0, 0, 0], "priority": 0, "encoding": "linear" }]);
    v["post"] = serde_json::json!({
        "tonemap": { "exposureBias": 0.8992, "shoulderStrength": 0.3538, "linearStrength": 0.3258, "linearAngle": 0.2528, "toeStrength": 0.6966, "toeNum": 0.0, "toeDenom": 0.7819, "whitePoint": 3.9996 },
        "bloom": { "blendMode": "add", "bloomStrength": 0.5, "screenBloomStrength": 0.0, "blurBloomStrength": 0.0, "threshold": 1.0, "thresholdWidth": 0.999,
                   "blurWeights": [0.0, 0.070259, 0.181076, 0.300013, 0.448652], "blurTints": [[1, 1, 1], [1, 1, 1], [1, 1, 1], [1, 1, 1], [1, 1, 1]] },
        "colorCorrection": { "file": "post/color-correction.bin", "dimension": 16 }
    });
    if let Some(fog) = v.get_mut("fog") {
        fog["volumes"] = serde_json::json!([{ "shape": "box", "worldToLocal": world_to_box([-4.0, 0.0, -4.0], [4.0, 1.6, 4.0]), "strength": 0.35, "falloff": 0.6, "indirectStrength": null, "tint": null }]);
        fog["irradiance"] = serde_json::Value::Null;
        fog["drawDistance"] = 14.0.into();
        fog["shadows"] = "fog/shadows.vtv".into();
    }
    v["particles"] = "particles/bundle.json".into();
    v["omissions"] = serde_json::json!(["autoExposure", "staticBundle"]);
    std::fs::write(scene, serde_json::to_vec_pretty(&v).map_err(std::io::Error::other)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lightmapped_room_carries_texcoord_3() {
        let m = crate::model::load_slice(&room_glb_lightmapped()).unwrap();
        assert!(m.primitives.iter().all(|p| p.vertices.iter().all(|v| v.uv3[0] >= 0.0 && v.uv3[0] <= 1.0)));
        let plain = crate::model::load_slice(&crate::fixture::room_glb()).unwrap();
        assert!(plain.primitives[0].vertices.iter().all(|v| v.uv3 == crate::model::NO_LIGHTMAP));
    }

    #[test]
    fn the_bake_lights_the_lamp_corner_and_shadows_behind_the_desk() {
        let (dc, _, shadow, edge) = bake();
        assert_eq!(dc.len(), (edge * edge) as usize);
        assert!(dc.iter().any(|p| p.0[0] > 1.0), "the lamp lights something");
        assert!(shadow.as_chunks::<4>().0.iter().any(|p| p[0] == 255), "the desk hides the spot somewhere");
    }

    #[test]
    fn the_upgraded_scene_reads_back_with_every_part() {
        let dir = std::env::temp_dir().join(format!("sigil-room-{}", std::process::id()));
        let scene = crate::fixture::write_hideout(&dir, crate::fixture::Variant::Stock).unwrap();
        upgrade_hideout(&scene).unwrap();
        let s = crate::scene::SceneDesc::read(&scene).unwrap();
        assert!(s.lightmap.as_ref().is_some_and(|l| l.direction.is_some() && l.shadows.is_some()));
        assert_eq!(s.probes.len(), 1);
        assert!(s.post.tonemap.is_some() && s.post.bloom.is_some() && s.post.lut.is_some());
        assert_eq!(s.fog.as_ref().map(|f| f.volumes.len()), Some(1));
        assert!(s.lights[0].cookie.is_some() && s.lights[0].source_units && !s.lights[0].fog_only);
        assert!(s.lights[1].fog_only);
        assert_eq!(s.lights[0].fog_shadow.as_ref().map(|f| f.tiles.len()), Some(1));
        assert_eq!(s.lights[1].fog_shadow.as_ref().map(|f| f.tiles.len()), Some(6));
        let data = crate::hideout::HideoutData::load(&s);
        assert!(data.fog_shadows.is_some(), "the fog shadow atlas loads");
        assert!(data.errors.is_empty(), "{:?}", data.errors);
        assert!(data.lightmap.is_some() && data.probes.is_some() && data.lut.is_some() && data.cookies.len() == 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
