//! The particle renderer on the GPU: one pipeline per blend and depth mode, one bind group
//! per renderer (its constants, texture, sampler), one shared instance buffer drawn in
//! ranges inside the scene's HDR MSAA pass.

use std::collections::HashMap;

use super::bundle::{Bundle, Sheet};
use super::host::{Effects, TOTAL_CAP};
use super::render::{BLEND_LIGHTEN, BLEND_MOD2X, Instance, View};
use crate::math::{Mat4, Vec3, length};

/// VRF's viewer sun (Grimoire's `VRF_VIEWER_SUN_COLOR`): particle lighting is `1 + sun`.
pub const VIEWER_SUN: [f32; 3] = [2.5, 2.42, 2.3];

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    light: [f32; 4],
    shade: [f32; 4],
    alpha: [f32; 4],
    up: [f32; 4],
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PipeKey {
    blend: u8,
    depth: u8,
}

struct Draw {
    pipe: PipeKey,
    group: wgpu::BindGroup,
    params: wgpu::Buffer,
    blend_mode: i32,
    diffuse: f32,
    blend_frames: bool,
    range: (u32, u32),
}

/// What one bundle's particle textures may take on the GPU, mips included.
pub const PARTICLE_TEXTURE_BUDGET: u64 = 256 * 1024 * 1024;

pub struct ParticleGpu {
    /// Bytes and count of the textures this bundle put on the GPU (the viewer charges them to
    /// its texture budget).
    texture_bytes: u64,
    texture_count: usize,
    effects: Effects,
    sheets: HashMap<String, Sheet>,
    pipes: HashMap<PipeKey, wgpu::RenderPipeline>,
    draws: Vec<Draw>,
    instances: Vec<Instance>,
    ranges: Vec<(u32, u32)>,
    shading: Vec<[f32; 5]>,
    params: Vec<Params>,
    buffer: wgpu::Buffer,
    capacity: usize,
    device: wgpu::Device,
    sun: [f32; 3],
    up_world: Vec3,
    _textures: Vec<wgpu::Texture>,
}

/// A soft round dot, the stand-in for `base_sprite` and any texture the bundle lacks.
fn soft_dot(size: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    let c = (size as f32 - 1.0) * 0.5;
    for y in 0..size {
        for x in 0..size {
            let d = (((x as f32 - c) / c).powi(2) + ((y as f32 - c) / c).powi(2)).sqrt();
            let a = (1.0 - d).clamp(0.0, 1.0).powf(1.5);
            out.extend_from_slice(&[255, 255, 255, (a * 255.0) as u8]);
        }
    }
    out
}

fn blend_state(blend: u8) -> wgpu::BlendState {
    let c = |src, dst, operation| wgpu::BlendComponent { src_factor: src, dst_factor: dst, operation };
    use wgpu::{BlendFactor as F, BlendOperation as O};
    match blend {
        1 => wgpu::BlendState { color: c(F::Dst, F::Src, O::Add), alpha: c(F::Zero, F::One, O::Add) },
        2 => wgpu::BlendState { color: c(F::One, F::One, O::Max), alpha: c(F::One, F::One, O::Max) },
        _ => wgpu::BlendState { color: c(F::One, F::OneMinusSrcAlpha, O::Add), alpha: c(F::One, F::OneMinusSrcAlpha, O::Add) },
    }
}

impl ParticleGpu {
    #[allow(clippy::too_many_arguments)]
    /// `budget`: the texture bytes the viewer has left; this bundle uses at most that and
    /// `PARTICLE_TEXTURE_BUDGET`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, frame_layout: &wgpu::BindGroupLayout, msaa: u32, hdr: wgpu::TextureFormat, depth: wgpu::TextureFormat, bundle: &Bundle, budget: u64) -> ParticleGpu {
        let mut effects = Effects::new(bundle);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("particles"), source: wgpu::ShaderSource::Wgsl(include_str!("sprite.wgsl").into()) });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particles renderer"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("particles"), bind_group_layouts: &[Some(frame_layout), Some(&layout)], immediate_size: 0 });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("particles"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        // Textures, decoded as sRGB colour (the particle vtex formats are), mipped on upload.
        let mut textures = Vec::new();
        let mut views: HashMap<String, usize> = HashMap::new();
        let mut view_list: Vec<wgpu::TextureView> = Vec::new();
        // The viewer's texture edge cap (gpu::upload_image's), the device's own limit, and a
        // budget for all of a bundle's textures: a texture past them is left out with a note.
        let device_edge = device.limits().max_texture_dimension_2d;
        let mut budget = budget.min(PARTICLE_TEXTURE_BUDGET);
        let (mut texture_bytes, mut texture_count) = (0u64, 0usize);
        // `required`: the 64 px fallback dot, which every effect needs, goes up whatever the budget.
        let mut upload = |width: u32, height: u32, rgba: &[u8], required: bool| -> Option<usize> {
            let mut chain = crate::gpu::mip_chain(width, height, rgba);
            while chain.len() > 1 && chain[0].0.max(chain[0].1) > crate::gpu::MAX_TEXTURE_EDGE {
                chain.remove(0);
            }
            let (width, height) = (chain[0].0, chain[0].1);
            let bytes: u64 = chain.iter().map(|l| l.2.len() as u64).sum();
            if !required && (width.max(height) > device_edge || bytes > budget) {
                return None;
            }
            budget = budget.saturating_sub(bytes);
            texture_bytes += bytes;
            texture_count += 1;
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("particle texture"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: chain.len() as u32,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            for (level, (w, h, data)) in chain.iter().enumerate() {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: level as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                    data,
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(*h) },
                    wgpu::Extent3d { width: *w, height: *h, depth_or_array_layers: 1 },
                );
            }
            view_list.push(tex.create_view(&Default::default()));
            textures.push(tex);
            Some(view_list.len() - 1)
        };
        let fallback = upload(64, 64, &soft_dot(64), true).unwrap_or(0);
        let mut sheets = HashMap::new();
        let mut names: Vec<&String> = bundle.textures.keys().collect();
        names.sort();
        for name in names {
            let t = &bundle.textures[name];
            if t.width == 0 || t.height == 0 || t.rgba.len() < (t.width * t.height * 4) as usize {
                continue;
            }
            match upload(t.width, t.height, &t.rgba, false) {
                Some(view) => {
                    views.insert(name.clone(), view);
                }
                None => {
                    effects.push_diagnostic(format!("{name}: a {}x{} texture past the device limit or the particle texture budget, left out", t.width, t.height));
                    continue;
                }
            }
            if let Some(s) = &t.sheet {
                sheets.insert(name.clone(), s.clone());
            }
        }

        let mut pipes = HashMap::new();
        let mut draws = Vec::new();
        let slots = effects.slots.clone();
        for slot in &slots {
            let r = {
                let mut sys = &effects.hosts[slot.host].sys;
                for &k in &slot.path {
                    sys = &sys.children[k];
                }
                sys.renderers[slot.renderer].clone()
            };
            let blend = match r.blend {
                BLEND_MOD2X => 1,
                BLEND_LIGHTEN => 2,
                _ => 0,
            };
            let dmode = if !r.depth_test {
                1
            } else if r.depth_reverse {
                2
            } else {
                0
            };
            let key = PipeKey { blend, depth: dmode };
            pipes.entry(key).or_insert_with(|| {
                let compare = match dmode {
                    1 => wgpu::CompareFunction::Always,
                    2 => wgpu::CompareFunction::Greater,
                    _ => wgpu::CompareFunction::LessEqual,
                };
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("particles"),
                    layout: Some(&pl),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some("vs_sprite"),
                        compilation_options: Default::default(),
                        buffers: &[Some(wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<Instance>() as u64,
                            step_mode: wgpu::VertexStepMode::Instance,
                            attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4],
                        })],
                    },
                    primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
                    depth_stencil: Some(wgpu::DepthStencilState { format: depth, depth_write_enabled: Some(false), depth_compare: Some(compare), stencil: Default::default(), bias: Default::default() }),
                    multisample: wgpu::MultisampleState { count: msaa, mask: !0, alpha_to_coverage_enabled: false },
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some("fs_sprite"),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState { format: hdr, blend: Some(blend_state(blend)), write_mask: wgpu::ColorWrites::ALL })],
                    }),
                    multiview_mask: None,
                    cache: None,
                })
            });
            let tex = match views.get(&r.texture) {
                Some(&i) => i,
                None => {
                    if !r.texture.ends_with("base_sprite.vtex") && !r.texture.ends_with("base_trail.vtex") {
                        effects.push_diagnostic(format!("missing texture {}", r.texture));
                    }
                    fallback
                }
            };
            let params = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("particle renderer"),
                size: std::mem::size_of::<Params>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("particle renderer"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: params.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view_list[tex]) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
                ],
            });
            draws.push(Draw { pipe: key, group, params, blend_mode: r.blend, diffuse: r.diffuse, blend_frames: r.blend_frames, range: (0, 0) });
        }
        let capacity = 256;
        let buffer = Self::instance_buffer(device, capacity);
        ParticleGpu {
            texture_bytes,
            texture_count,
            effects,
            sheets,
            pipes,
            draws,
            instances: Vec::with_capacity(capacity),
            ranges: Vec::new(),
            shading: Vec::new(),
            params: Vec::new(),
            buffer,
            capacity,
            device: device.clone(),
            sun: VIEWER_SUN,
            up_world: [0.0, 1.0, 0.0],
            _textures: textures,
        }
    }

    fn instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particle instances"),
            size: (capacity * std::mem::size_of::<Instance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// The sun colour the particle lighting adds to 1 (VRF's probe-less particle lighting).
    /// The hero viewer default is VRF's viewer sun; a Hideout passes its map sun (linear,
    /// times brightness).
    pub fn set_sun(&mut self, rgb: [f32; 3]) {
        self.sun = rgb;
    }

    /// Simulates by `dt`. `to_world` maps Source space to the glTF world (the hero's placement
    /// times `source_to_gltf`, or the scene's); `attachment` gives an attachment's (or bone's)
    /// Source-space frame this frame; `camera` is the camera position in world space.
    pub fn step(&mut self, dt: f32, to_world: &Mat4, attachment: &dyn Fn(&str) -> Option<Mat4>, camera: [f32; 3]) {
        let inv = to_world.inverse();
        let cam_src = inv.point(camera);
        self.effects.step(dt, attachment, cam_src);
        let up = to_world.vector([0.0, 0.0, 1.0]);
        let l = length(up).max(1e-12);
        self.up_world = [up[0] / l, up[1] / l, up[2] / l];
        let view = View { camera: cam_src, to_world: *to_world, world_scale: to_world.scale_factor() };
        self.effects.build(&view, &self.sheets, &mut self.instances, &mut self.ranges, &mut self.shading);
        self.params.clear();
        for (k, d) in self.draws.iter_mut().enumerate() {
            d.range = self.ranges.get(k).copied().unwrap_or((0, 0));
            let s = self.shading.get(k).copied().unwrap_or([1.0, 1.0, 0.0, 1.0, 0.0]);
            self.params.push(Params {
                light: [self.sun[0], self.sun[1], self.sun[2], d.diffuse],
                shade: [s[0], s[1], s[2], d.blend_mode as f32],
                alpha: [s[3], s[4], if d.blend_frames { 1.0 } else { 0.0 }, 0.0],
                up: [self.up_world[0], self.up_world[1], self.up_world[2], 0.0],
            });
        }
    }

    /// Writes this frame's cards and constants. Grows the instance buffer when a frame needs
    /// more (rarely: it doubles up to the total cap).
    pub fn upload(&mut self, queue: &wgpu::Queue) {
        if self.instances.len() > self.capacity {
            self.capacity = self.instances.len().next_power_of_two().min(TOTAL_CAP.next_power_of_two());
            self.buffer = Self::instance_buffer(&self.device, self.capacity);
        }
        if !self.instances.is_empty() {
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&self.instances));
        }
        for (d, p) in self.draws.iter().zip(&self.params) {
            queue.write_buffer(&d.params, 0, bytemuck::bytes_of(p));
        }
    }

    /// Draws into the scene pass; the caller has set group 0 (the frame) already.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.instances.is_empty() {
            return;
        }
        pass.set_vertex_buffer(0, self.buffer.slice(..));
        let mut current = None;
        for d in &self.draws {
            if d.range.1 <= d.range.0 {
                continue;
            }
            if current != Some(d.pipe) {
                pass.set_pipeline(&self.pipes[&d.pipe]);
                current = Some(d.pipe);
            }
            pass.set_bind_group(1, &d.group, &[]);
            pass.draw(0..6, d.range.0..d.range.1);
        }
    }

    /// Bytes and count of the textures this bundle holds on the GPU.
    pub fn textures(&self) -> (u64, usize) {
        (self.texture_bytes, self.texture_count)
    }

    pub fn particle_count(&self) -> usize {
        self.effects.particle_count()
    }

    /// Cards drawn this frame.
    pub fn instance_count(&self) -> usize {
        self.instances.len()
    }

    /// Unsupported classes and inputs, missing systems and textures.
    pub fn diagnostics(&self) -> &[String] {
        self.effects.diagnostics()
    }
}
