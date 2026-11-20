//! A glTF 2.0 model on the CPU: node tree, skins, triangle primitives, materials, decoded
//! images and animation clips. `load_glb` reads a `.glb` (or `.gltf` with its files) the way
//! vpkmerge's morphic writes heroes: bones under a `skeleton` node that carries Source to glTF
//! (inches to meters, Z-up to Y-up), skinned LOD0 meshes, `extras.morphic` on materials and
//! `extras.morphic_shell` on authored outline primitives.

use std::path::Path;

use crate::material::Material;
use crate::math::{Aabb, Mat4, Quat, Vec3, normalize, quat_normalize, slerp};

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub tangent: [f32; 4],
    pub uv0: [f32; 2],
    pub uv1: [f32; 2],
    pub color: [f32; 4],
    pub joints: [u32; 4],
    pub weights: [f32; 4],
}

#[derive(Clone, Debug)]
pub struct Node {
    pub name: String,
    pub parent: Option<usize>,
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    /// A node given as a matrix keeps it; animation never targets such a node.
    pub matrix: Option<Mat4>,
}

impl Node {
    pub fn local(&self) -> Mat4 {
        self.matrix.unwrap_or_else(|| Mat4::from_trs(self.translation, self.rotation, self.scale))
    }
}

#[derive(Clone, Debug)]
pub struct Skin {
    pub joints: Vec<usize>,
    pub inverse_bind: Vec<Mat4>,
}

#[derive(Clone, Debug)]
pub struct Primitive {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub material: Option<usize>,
    /// `extras.morphic_shell`: an authored outline or jitter part. Hidden unless it is a plain
    /// opaque NPR shell that asks for no outline growth (Grimoire's rule).
    pub authored_shell: bool,
    pub bounds: Aabb,
    /// The primitive's index within its glTF mesh (a scene fragment draws one of them).
    pub index_in_mesh: usize,
}

/// A mesh placed by a node, optionally skinned.
#[derive(Clone, Debug)]
pub struct MeshInstance {
    pub node: usize,
    pub skin: Option<usize>,
    pub primitives: Vec<usize>,
}

#[derive(Clone, Debug)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
pub struct TextureRef {
    pub image: usize,
    pub repeat: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Path3 {
    Translation,
    Rotation,
    Scale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interp {
    Step,
    Linear,
    Cubic,
}

#[derive(Clone, Debug)]
pub struct Channel {
    pub node: usize,
    pub path: Path3,
    pub interp: Interp,
    pub times: Vec<f32>,
    /// Flat values: 3 or 4 floats per key, times three for cubic (in-tangent, value, out-tangent).
    pub values: Vec<f32>,
}

#[derive(Clone, Debug)]
pub struct Clip {
    pub name: String,
    pub duration: f32,
    pub channels: Vec<Channel>,
}

#[derive(Clone, Debug, Default)]
pub struct Model {
    pub nodes: Vec<Node>,
    pub skins: Vec<Skin>,
    pub meshes: Vec<MeshInstance>,
    pub primitives: Vec<Primitive>,
    pub materials: Vec<Material>,
    pub images: Vec<Image>,
    pub textures: Vec<TextureRef>,
    pub clips: Vec<Clip>,
}

#[derive(Debug)]
pub struct LoadError(pub String);

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LoadError {}

pub fn load_path(path: &Path) -> Result<Model, LoadError> {
    let (doc, buffers, images) = gltf::import(path).map_err(|e| LoadError(format!("{}: {e}", path.display())))?;
    build(&doc, &buffers, images)
}

pub fn load_slice(bytes: &[u8]) -> Result<Model, LoadError> {
    let (doc, buffers, images) = gltf::import_slice(bytes).map_err(|e| LoadError(format!("glTF: {e}")))?;
    build(&doc, &buffers, images)
}

fn build(doc: &gltf::Document, buffers: &[gltf::buffer::Data], images: Vec<gltf::image::Data>) -> Result<Model, LoadError> {
    let mut model = Model::default();
    // Nodes and parents.
    for node in doc.nodes() {
        let (t, r, s) = node.transform().decomposed();
        let matrix = match node.transform() {
            gltf::scene::Transform::Matrix { matrix } => Some(Mat4(matrix)),
            _ => None,
        };
        model.nodes.push(Node {
            name: node.name().unwrap_or_default().to_string(),
            parent: None,
            translation: t,
            rotation: quat_normalize(r),
            scale: s,
            matrix,
        });
    }
    for node in doc.nodes() {
        for child in node.children() {
            model.nodes[child.index()].parent = Some(node.index());
        }
    }
    for skin in doc.skins() {
        let joints: Vec<usize> = skin.joints().map(|j| j.index()).collect();
        let reader = skin.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
        let inverse_bind = match reader.read_inverse_bind_matrices() {
            Some(it) => it.map(Mat4).collect(),
            None => vec![Mat4::IDENTITY; joints.len()],
        };
        model.skins.push(Skin { joints, inverse_bind });
    }
    model.images = images.into_iter().map(to_rgba).collect();
    for texture in doc.textures() {
        let sampler = texture.sampler();
        let repeat = !matches!(sampler.wrap_s(), gltf::texture::WrappingMode::ClampToEdge);
        model.textures.push(TextureRef { image: texture.source().index(), repeat });
    }
    for material in doc.materials() {
        model.materials.push(Material::from_gltf(&material));
    }
    // Meshes: only nodes reachable from the default scene draw.
    let mut placed = Vec::new();
    let scene = doc.default_scene().or_else(|| doc.scenes().next());
    let mut stack: Vec<gltf::Node> = match &scene {
        Some(scene) => scene.nodes().collect(),
        None => doc.nodes().filter(|n| model.nodes[n.index()].parent.is_none()).collect(),
    };
    while let Some(node) = stack.pop() {
        if let Some(mesh) = node.mesh() {
            placed.push((node.index(), node.skin().map(|s| s.index()), mesh));
        }
        stack.extend(node.children());
    }
    placed.sort_by_key(|(n, _, _)| *n);
    for (node, skin, mesh) in placed {
        let mut prims = Vec::new();
        for primitive in mesh.primitives() {
            if primitive.mode() != gltf::mesh::Mode::Triangles {
                continue;
            }
            if let Some(mut p) = read_primitive(&primitive, buffers) {
                p.index_in_mesh = primitive.index();
                prims.push(model.primitives.len());
                model.primitives.push(p);
            }
        }
        model.meshes.push(MeshInstance { node, skin, primitives: prims });
    }
    for animation in doc.animations() {
        let mut channels = Vec::new();
        let mut duration = 0.0f32;
        for channel in animation.channels() {
            let reader = channel.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
            let Some(times) = reader.read_inputs() else { continue };
            let times: Vec<f32> = times.collect();
            let path = match channel.target().property() {
                gltf::animation::Property::Translation => Path3::Translation,
                gltf::animation::Property::Rotation => Path3::Rotation,
                gltf::animation::Property::Scale => Path3::Scale,
                gltf::animation::Property::MorphTargetWeights => continue,
            };
            let values: Vec<f32> = match reader.read_outputs() {
                Some(gltf::animation::util::ReadOutputs::Translations(it)) => it.flatten().collect(),
                Some(gltf::animation::util::ReadOutputs::Scales(it)) => it.flatten().collect(),
                Some(gltf::animation::util::ReadOutputs::Rotations(it)) => it.into_f32().flatten().collect(),
                _ => continue,
            };
            let interp = match channel.sampler().interpolation() {
                gltf::animation::Interpolation::Step => Interp::Step,
                gltf::animation::Interpolation::Linear => Interp::Linear,
                gltf::animation::Interpolation::CubicSpline => Interp::Cubic,
            };
            if let Some(last) = times.last() {
                duration = duration.max(*last);
            }
            channels.push(Channel { node: channel.target().node().index(), path, interp, times, values });
        }
        let name = animation.name().map(str::to_string).unwrap_or_else(|| format!("clip {}", animation.index()));
        model.clips.push(Clip { name, duration, channels });
    }
    if model.primitives.is_empty() {
        return Err(LoadError("the model has no triangle meshes".into()));
    }
    Ok(model)
}

fn to_rgba(img: gltf::image::Data) -> Image {
    use gltf::image::Format;
    let (w, h) = (img.width, img.height);
    let n = (w * h) as usize;
    let p = &img.pixels;
    let mut rgba = Vec::with_capacity(n * 4);
    match img.format {
        Format::R8G8B8A8 => rgba = img.pixels,
        Format::R8G8B8 => p.as_chunks::<3>().0.iter().for_each(|c| rgba.extend_from_slice(&[c[0], c[1], c[2], 255])),
        Format::R8G8 => p.as_chunks::<2>().0.iter().for_each(|c| rgba.extend_from_slice(&[c[0], c[1], 0, 255])),
        Format::R8 => p.iter().for_each(|&c| rgba.extend_from_slice(&[c, c, c, 255])),
        Format::R16 => p.as_chunks::<2>().0.iter().for_each(|c| rgba.extend_from_slice(&[c[1], c[1], c[1], 255])),
        Format::R16G16 => p.as_chunks::<4>().0.iter().for_each(|c| rgba.extend_from_slice(&[c[1], c[3], 0, 255])),
        Format::R16G16B16 => p.as_chunks::<6>().0.iter().for_each(|c| rgba.extend_from_slice(&[c[1], c[3], c[5], 255])),
        Format::R16G16B16A16 => p.as_chunks::<8>().0.iter().for_each(|c| rgba.extend_from_slice(&[c[1], c[3], c[5], c[7]])),
        Format::R32G32B32FLOAT | Format::R32G32B32A32FLOAT => {
            let k = if img.format == Format::R32G32B32FLOAT { 3 } else { 4 };
            for c in p.chunks_exact(4 * k) {
                let f = |i: usize| f32::from_le_bytes([c[i * 4], c[i * 4 + 1], c[i * 4 + 2], c[i * 4 + 3]]);
                let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                rgba.extend_from_slice(&[b(f(0)), b(f(1)), b(f(2)), if k == 4 { b(f(3)) } else { 255 }]);
            }
        }
    }
    if rgba.len() != n * 4 {
        rgba = vec![255; n * 4];
    }
    Image { width: w, height: h, rgba }
}

fn read_primitive(primitive: &gltf::Primitive, buffers: &[gltf::buffer::Data]) -> Option<Primitive> {
    let reader = primitive.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
    let positions: Vec<Vec3> = reader.read_positions()?.collect();
    if positions.is_empty() {
        return None;
    }
    let n = positions.len();
    let indices: Vec<u32> = match reader.read_indices() {
        Some(it) => it.into_u32().collect(),
        None => (0..n as u32).collect(),
    };
    let indices: Vec<u32> = indices.as_chunks::<3>().0.iter().filter(|t| t.iter().all(|&i| (i as usize) < n)).flatten().copied().collect();
    let mut vertices = vec![Vertex { color: [1.0; 4], weights: [1.0, 0.0, 0.0, 0.0], ..Default::default() }; n];
    let mut bounds = Aabb::empty();
    for (v, p) in vertices.iter_mut().zip(&positions) {
        v.position = *p;
        bounds.grow(*p);
    }
    let has_normals = if let Some(it) = reader.read_normals() {
        for (v, nrm) in vertices.iter_mut().zip(it) {
            v.normal = normalize(nrm);
        }
        true
    } else {
        false
    };
    if !has_normals {
        compute_normals(&mut vertices, &indices);
    }
    if let Some(it) = reader.read_tex_coords(0) {
        for (v, uv) in vertices.iter_mut().zip(it.into_f32()) {
            v.uv0 = uv;
        }
    }
    if let Some(it) = reader.read_tex_coords(1) {
        for (v, uv) in vertices.iter_mut().zip(it.into_f32()) {
            v.uv1 = uv;
        }
    } else {
        for v in &mut vertices {
            v.uv1 = v.uv0;
        }
    }
    if let Some(it) = reader.read_colors(0) {
        for (v, c) in vertices.iter_mut().zip(it.into_rgba_f32()) {
            v.color = c;
        }
    }
    if let (Some(joints), Some(weights)) = (reader.read_joints(0), reader.read_weights(0)) {
        for ((v, j), w) in vertices.iter_mut().zip(joints.into_u16()).zip(weights.into_f32()) {
            let sum = w[0] + w[1] + w[2] + w[3];
            v.joints = [j[0] as u32, j[1] as u32, j[2] as u32, j[3] as u32];
            v.weights = if sum > 1e-6 { [w[0] / sum, w[1] / sum, w[2] / sum, w[3] / sum] } else { [1.0, 0.0, 0.0, 0.0] };
        }
    }
    let has_tangents = if let Some(it) = reader.read_tangents() {
        for (v, t) in vertices.iter_mut().zip(it) {
            v.tangent = t;
        }
        true
    } else {
        false
    };
    if !has_tangents {
        compute_tangents(&mut vertices, &indices);
    }
    let authored_shell = primitive.extras().as_ref().is_some_and(|raw| {
        serde_json::from_str::<serde_json::Value>(raw.get()).ok().is_some_and(|v| v.get("morphic_shell").is_some())
    });
    Some(Primitive { vertices, indices, material: primitive.material().index(), authored_shell, bounds, index_in_mesh: primitive.index() })
}

fn compute_normals(vertices: &mut [Vertex], indices: &[u32]) {
    let mut acc = vec![[0.0f32; 3]; vertices.len()];
    for t in indices.as_chunks::<3>().0 {
        let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
        let e1 = crate::math::sub(vertices[b].position, vertices[a].position);
        let e2 = crate::math::sub(vertices[c].position, vertices[a].position);
        let n = crate::math::cross(e1, e2);
        for i in [a, b, c] {
            acc[i] = crate::math::add(acc[i], n);
        }
    }
    for (v, n) in vertices.iter_mut().zip(acc) {
        v.normal = normalize(n);
    }
}

/// Per-triangle UV tangents averaged per vertex, Gram-Schmidt against the normal. Enough for
/// the normal maps a model without exported tangents carries.
fn compute_tangents(vertices: &mut [Vertex], indices: &[u32]) {
    let mut tan = vec![[0.0f32; 3]; vertices.len()];
    let mut bit = vec![[0.0f32; 3]; vertices.len()];
    for t in indices.as_chunks::<3>().0 {
        let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
        let (p0, p1, p2) = (vertices[a].position, vertices[b].position, vertices[c].position);
        let (w0, w1, w2) = (vertices[a].uv0, vertices[b].uv0, vertices[c].uv0);
        let (e1, e2) = (crate::math::sub(p1, p0), crate::math::sub(p2, p0));
        let (du1, dv1, du2, dv2) = (w1[0] - w0[0], w1[1] - w0[1], w2[0] - w0[0], w2[1] - w0[1]);
        let det = du1 * dv2 - du2 * dv1;
        if det.abs() < 1e-12 {
            continue;
        }
        let r = 1.0 / det;
        let sdir = crate::math::scale(crate::math::sub(crate::math::scale(e1, dv2), crate::math::scale(e2, dv1)), r);
        let tdir = crate::math::scale(crate::math::sub(crate::math::scale(e2, du1), crate::math::scale(e1, du2)), r);
        for i in [a, b, c] {
            tan[i] = crate::math::add(tan[i], sdir);
            bit[i] = crate::math::add(bit[i], tdir);
        }
    }
    for (i, v) in vertices.iter_mut().enumerate() {
        let n = v.normal;
        let t = normalize(crate::math::sub(tan[i], crate::math::scale(n, crate::math::dot(n, tan[i]))));
        let t = if crate::math::length(t) < 0.5 { normalize(crate::math::cross(n, if n[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] })) } else { t };
        let w = if crate::math::dot(crate::math::cross(n, t), bit[i]) < 0.0 { -1.0 } else { 1.0 };
        v.tangent = [t[0], t[1], t[2], w];
    }
}

/// Node locals for one moment of a clip (or the rest pose with `None`).
pub fn sample_pose(model: &Model, clip: Option<&Clip>, time: f32) -> Vec<Mat4> {
    let mut t: Vec<Vec3> = model.nodes.iter().map(|n| n.translation).collect();
    let mut r: Vec<Quat> = model.nodes.iter().map(|n| n.rotation).collect();
    let mut s: Vec<Vec3> = model.nodes.iter().map(|n| n.scale).collect();
    if let Some(clip) = clip {
        for ch in &clip.channels {
            if ch.node >= model.nodes.len() || ch.times.is_empty() {
                continue;
            }
            let width = if ch.path == Path3::Rotation { 4 } else { 3 };
            let v = sample_channel(ch, width, time);
            match ch.path {
                Path3::Translation => t[ch.node] = [v[0], v[1], v[2]],
                Path3::Scale => s[ch.node] = [v[0], v[1], v[2]],
                Path3::Rotation => r[ch.node] = quat_normalize([v[0], v[1], v[2], v[3]]),
            }
        }
    }
    model
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| match (n.matrix, clip) {
            (Some(m), _) => m,
            _ => Mat4::from_trs(t[i], r[i], s[i]),
        })
        .collect()
}

fn sample_channel(ch: &Channel, width: usize, time: f32) -> [f32; 4] {
    let times = &ch.times;
    let stride = if ch.interp == Interp::Cubic { width * 3 } else { width };
    let value_at = |k: usize| -> [f32; 4] {
        let base = k * stride + if ch.interp == Interp::Cubic { width } else { 0 };
        let mut out = [0.0; 4];
        for (i, o) in out.iter_mut().enumerate().take(width) {
            *o = ch.values.get(base + i).copied().unwrap_or(0.0);
        }
        out
    };
    if time <= times[0] || times.len() == 1 {
        return value_at(0);
    }
    let last = times.len() - 1;
    if time >= times[last] {
        return value_at(last);
    }
    let k = times.partition_point(|&x| x <= time).saturating_sub(1).min(last - 1);
    let (t0, t1) = (times[k], times[k + 1]);
    let dt = (t1 - t0).max(1e-9);
    let u = ((time - t0) / dt).clamp(0.0, 1.0);
    match ch.interp {
        Interp::Step => value_at(k),
        Interp::Linear => {
            let (a, b) = (value_at(k), value_at(k + 1));
            if width == 4 {
                slerp(a, b, u)
            } else {
                [a[0] + (b[0] - a[0]) * u, a[1] + (b[1] - a[1]) * u, a[2] + (b[2] - a[2]) * u, 0.0]
            }
        }
        Interp::Cubic => {
            let get = |k: usize, part: usize, i: usize| ch.values.get(k * stride + part * width + i).copied().unwrap_or(0.0);
            let (u2, u3) = (u * u, u * u * u);
            let mut out = [0.0; 4];
            for (i, o) in out.iter_mut().enumerate().take(width) {
                let p0 = get(k, 1, i);
                let m0 = get(k, 2, i) * dt;
                let p1 = get(k + 1, 1, i);
                let m1 = get(k + 1, 0, i) * dt;
                *o = (2.0 * u3 - 3.0 * u2 + 1.0) * p0 + (u3 - 2.0 * u2 + u) * m0 + (-2.0 * u3 + 3.0 * u2) * p1 + (u3 - u2) * m1;
            }
            if width == 4 { quat_normalize(out) } else { out }
        }
    }
}

/// World matrices from node locals, parents first (any order of the node list).
pub fn globals(model: &Model, locals: &[Mat4]) -> Vec<Mat4> {
    let mut out: Vec<Option<Mat4>> = vec![None; model.nodes.len()];
    fn solve(i: usize, model: &Model, locals: &[Mat4], out: &mut Vec<Option<Mat4>>, depth: usize) -> Mat4 {
        if let Some(m) = out[i] {
            return m;
        }
        let m = match model.nodes[i].parent {
            Some(p) if depth < 512 => solve(p, model, locals, out, depth + 1).mul(&locals[i]),
            _ => locals[i],
        };
        out[i] = Some(m);
        m
    }
    for i in 0..model.nodes.len() {
        solve(i, model, locals, &mut out, 0);
    }
    out.into_iter().map(|m| m.unwrap_or_default()).collect()
}

/// The matrix palette of every mesh instance, concatenated, with each instance's base index.
/// A skinned mesh gets `global(joint) * inverse_bind` per joint; an unskinned one its node's
/// world matrix (its vertices weigh joint 0 fully).
pub fn palettes(model: &Model, globals: &[Mat4], root: &Mat4, out: &mut Vec<Mat4>) -> Vec<u32> {
    out.clear();
    let mut bases = Vec::with_capacity(model.meshes.len());
    for mesh in &model.meshes {
        bases.push(out.len() as u32);
        match mesh.skin.and_then(|s| model.skins.get(s)) {
            Some(skin) => {
                for (j, &joint) in skin.joints.iter().enumerate() {
                    let g = globals.get(joint).copied().unwrap_or_default();
                    let ibm = skin.inverse_bind.get(j).copied().unwrap_or_default();
                    out.push(root.mul(&g.mul(&ibm)));
                }
                if skin.joints.is_empty() {
                    out.push(*root);
                }
            }
            None => out.push(root.mul(&globals[mesh.node])),
        }
    }
    bases
}

/// Bounds of the model posed by `palette` (CPU skinning of every vertex: run once per frame
/// the camera frames, not per render).
pub fn posed_bounds(model: &Model, palette: &[Mat4], bases: &[u32], visible: impl Fn(usize) -> bool) -> Aabb {
    let mut b = Aabb::empty();
    for (mi, mesh) in model.meshes.iter().enumerate() {
        let base = bases[mi] as usize;
        for &pi in &mesh.primitives {
            if !visible(pi) {
                continue;
            }
            for v in model.primitives[pi].vertices.iter().step_by(3) {
                let mut p = [0.0f32; 3];
                for k in 0..4 {
                    let w = v.weights[k];
                    if w <= 0.0 {
                        continue;
                    }
                    let m = palette.get(base + v.joints[k] as usize).copied().unwrap_or_default();
                    p = crate::math::add(p, crate::math::scale(m.point(v.position), w));
                }
                b.grow(p);
            }
        }
    }
    b
}

/// World units per Source game unit: the scale of the first mesh's node chain, which holds
/// the exporter's inch to meter conversion (`citadelRimUnit` in Grimoire).
pub fn game_unit(model: &Model, globals: &[Mat4]) -> f32 {
    let Some(mesh) = model.meshes.first() else { return 1.0 };
    // A bone's world matrix carries the skeleton node's conversion; bones themselves are
    // unscaled.
    let m = match mesh.skin.and_then(|s| model.skins.get(s)).and_then(|s| s.joints.first()) {
        Some(&joint) => globals[joint],
        None => globals[mesh.node],
    };
    let k = m.scale_factor();
    if k.is_finite() && k > 0.0 { k } else { 1.0 }
}

pub fn clip_by_name<'a>(model: &'a Model, name: &str) -> Option<&'a Clip> {
    model.clips.iter().find(|c| c.name == name).or_else(|| model.clips.iter().find(|c| c.name.eq_ignore_ascii_case(name)))
}

/// The clip a hero should idle with when the sidecar names none: Grimoire's order (the UI shop
/// state, hero select, then generic idles), then the first clip.
pub fn default_idle(model: &Model) -> Option<&Clip> {
    const PREFERRED: [&str; 6] = ["hero_shop", "hero_select", "ui_idle", "primary_idle", "idle", "stand_idle"];
    for want in PREFERRED {
        if let Some(c) = model.clips.iter().find(|c| c.name.eq_ignore_ascii_case(want)) {
            return Some(c);
        }
    }
    for want in PREFERRED {
        if let Some(c) = model.clips.iter().find(|c| c.name.to_ascii_lowercase().contains(want)) {
            return Some(c);
        }
    }
    model.clips.iter().find(|c| c.duration > 0.0).or(model.clips.first())
}
