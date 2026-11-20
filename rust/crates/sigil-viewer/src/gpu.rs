//! The wgpu side: one device of the viewer's own (never Rune's), the pipelines, a model's
//! buffers and bind groups, the size-dependent targets and the frame: hero depth for rim
//! occlusion, a 4x MSAA HDR pass per shown model, bloom, then post into an sRGB target that is
//! read back as RGBA8.

use std::borrow::Cow;
use std::collections::HashMap;
use std::time::Instant;

use crate::material::{AlphaMode, Resolved};
use crate::math::{Mat4, Vec3};
use crate::model::{Model, Vertex};

pub const HDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
pub const OUTPUT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
/// Textures larger than this are downsampled on load (a hero's 4K maps fill no viewport).
pub const MAX_TEXTURE_EDGE: u32 = 2048;
const BLOOM_LEVELS: usize = 5;
/// What one viewer may put on the GPU in textures and in vertex and index buffers. Past it a
/// texture is downscaled (then dropped) and a mesh is left out, with the reason recorded: a whole
/// room on a 4 GB card must open, not run the driver out of memory. Integrated and software
/// adapters share system memory and get half.
pub const TEXTURE_BUDGET: u64 = 1 << 30;
pub const BUFFER_BUDGET: u64 = 1536 << 20;
/// The longest a frame may take on the GPU before the viewer gives up on it (a lost or hung
/// device) instead of waiting forever.
pub const FRAME_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
/// The most "left out" lines kept; the rest are counted.
const SKIPPED_LINES: usize = 64;

/// What one viewer has on the GPU and what it left out to stay inside the device's limits and
/// the budgets above.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Usage {
    pub textures: usize,
    pub texture_bytes: u64,
    pub buffer_bytes: u64,
    /// Textures uploaded smaller than the file to fit the budget.
    pub downscaled: usize,
    /// Textures, meshes and palettes left out, and why (the first `SKIPPED_LINES`).
    pub skipped: Vec<String>,
    pub skipped_count: usize,
}

impl Usage {
    pub fn skip(&mut self, why: String) {
        self.skipped_count += 1;
        if self.skipped.len() < SKIPPED_LINES {
            self.skipped.push(why);
        }
    }
}

fn mb(bytes: u64) -> u64 {
    bytes.div_ceil(1 << 20)
}

#[derive(Debug, Clone)]
pub enum GpuError {
    /// No adapter on this machine (no driver, a VM, a headless box). The UI shows a still.
    NoAdapter(String),
    Device(String),
}

impl std::fmt::Display for GpuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GpuError::NoAdapter(e) => write!(f, "no GPU adapter: {e}"),
            GpuError::Device(e) => write!(f, "GPU device: {e}"),
        }
    }
}

pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub info: wgpu::AdapterInfo,
    pub msaa: u32,
    /// The first GPU error since the last [`Gpu::take_error`]: validation, out of memory or a lost
    /// device. wgpu's default handler panics on these, which killed the viewer's thread (and on
    /// some drivers the app) with the view stuck on "Loading"; the viewer reports them instead.
    errors: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    /// The device's limits: every buffer and texture is checked against them before it is made.
    pub limits: wgpu::Limits,
    pub texture_budget: u64,
    pub buffer_budget: u64,
    usage: std::sync::Mutex<Usage>,
}

fn instance() -> wgpu::Instance {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    #[cfg(target_os = "windows")]
    {
        // D3D12 alone with FXC, as Rune does: a Vulkan instance loads every GPU's driver, and
        // FXC ships with Windows where dxcompiler.dll may not.
        descriptor.backends = wgpu::Backends::DX12;
        descriptor.backend_options.dx12.shader_compiler = wgpu::Dx12Compiler::Fxc;
    }
    #[cfg(not(target_os = "windows"))]
    {
        descriptor.backends = wgpu::Backends::PRIMARY;
    }
    descriptor.flags.remove(wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL);
    wgpu::Instance::new(descriptor)
}

impl Gpu {
    /// The viewer's own device on the high-performance adapter (a hero at 60 fps wants the
    /// discrete GPU where there is one). A software adapter is accepted.
    pub fn new() -> Result<Gpu, GpuError> {
        Gpu::on_adapter(false)
    }

    /// The software adapter (WARP on Windows), for a machine whose driver cannot build the
    /// pipelines. `NoAdapter` where the platform has none.
    pub fn software() -> Result<Gpu, GpuError> {
        Gpu::on_adapter(true)
    }

    fn on_adapter(software: bool) -> Result<Gpu, GpuError> {
        let instance = instance();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: software,
            compatible_surface: None,
            apply_limit_buckets: false,
        }))
        .map_err(|e| GpuError::NoAdapter(e.to_string()))?;
        let info = adapter.get_info();
        let available = adapter.limits();
        let limits = wgpu::Limits::downlevel_defaults().using_resolution(available.clone()).using_alignment(available.clone());
        // A whole room (the Hideout) needs more than the 256 MB downlevel buffer cap; ask for what
        // the adapter has rather than failing validation mid-load.
        let limits = wgpu::Limits { max_buffer_size: available.max_buffer_size, ..limits };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("sigil-viewer"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits {
                max_storage_buffers_per_shader_stage: 4,
                max_storage_buffer_binding_size: (64 << 20u64).min(available.max_storage_buffer_binding_size),
                // The Hideout's lightmap, probe, fog and cookie textures join the material's
                // seven: more sampled textures than the downlevel floor of 16.
                max_sampled_textures_per_shader_stage: available.max_sampled_textures_per_shader_stage.min(32),
                ..limits
            },
            experimental_features: Default::default(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: Default::default(),
        }))
        .map_err(|e| GpuError::Device(e.to_string()))?;
        let flags = adapter.get_texture_format_features(HDR).flags;
        let msaa = if flags.sample_count_supported(4) && adapter.get_texture_format_features(DEPTH).flags.sample_count_supported(4) { 4 } else { 1 };
        let errors = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
        let sink = errors.clone();
        device.on_uncaptured_error(std::sync::Arc::new(move |error: wgpu::Error| {
            if let Ok(mut slot) = sink.lock() {
                slot.get_or_insert_with(|| error.to_string());
            }
        }));
        let sink = errors.clone();
        device.set_device_lost_callback(move |_reason, message| {
            if let Ok(mut slot) = sink.lock() {
                slot.get_or_insert_with(|| format!("The graphics device was lost: {message}"));
            }
        });
        let limits = device.limits();
        let shared = matches!(info.device_type, wgpu::DeviceType::IntegratedGpu | wgpu::DeviceType::Cpu);
        let (texture_budget, buffer_budget) = if shared { (TEXTURE_BUDGET / 2, BUFFER_BUDGET / 2) } else { (TEXTURE_BUDGET, BUFFER_BUDGET) };
        Ok(Gpu { device, queue, info, msaa, errors, limits, texture_budget, buffer_budget, usage: Default::default() })
    }

    /// The first GPU error since the last call, if any.
    pub fn take_error(&self) -> Option<String> {
        self.errors.lock().ok().and_then(|mut slot| slot.take())
    }

    /// Records an error the way the device's own handler does (the first one wins).
    pub fn report(&self, error: String) {
        if let Ok(mut slot) = self.errors.lock() {
            slot.get_or_insert(error);
        }
    }

    /// What is on the GPU and what was left out.
    pub fn usage(&self) -> Usage {
        self.usage.lock().map(|u| u.clone()).unwrap_or_default()
    }

    fn with_usage<T>(&self, f: impl FnOnce(&mut Usage) -> T) -> T {
        let mut guard = self.usage.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut guard)
    }

    /// Reserves `bytes` of buffer memory for `what`. False (and recorded) when one buffer would
    /// pass the device's `max_buffer_size` or the total would pass the budget.
    pub fn reserve_buffers(&self, sizes: &[u64], what: &str) -> bool {
        let total: u64 = sizes.iter().sum();
        let limit = self.limits.max_buffer_size;
        let budget = self.buffer_budget;
        self.with_usage(|u| {
            if let Some(big) = sizes.iter().find(|s| **s > limit) {
                u.skip(format!("{what}: a {} MB buffer is over this GPU's {} MB limit", mb(*big), mb(limit)));
                false
            } else if u.buffer_bytes + total > budget {
                u.skip(format!("{what}: over the {} MB geometry budget", mb(budget)));
                false
            } else {
                u.buffer_bytes += total;
                true
            }
        })
    }

    /// The largest render target edge this device takes (and the viewer allows).
    pub fn max_edge(&self) -> u32 {
        self.limits.max_texture_dimension_2d.clamp(1, 8192)
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LightGpu {
    pub pos_dir: [f32; 4],
    pub color: [f32; 4],
    pub spot: [f32; 4],
    pub spot_inner: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FrameGpu {
    pub view_proj: [f32; 16],
    pub view: [f32; 16],
    pub proj: [f32; 16],
    pub camera_pos: [f32; 4],
    pub camera_back: [f32; 4],
    pub key_dir: [f32; 4],
    pub ambient: [[f32; 4]; 6],
    pub ambient_fit: [f32; 4],
    pub env_top: [f32; 4],
    pub env_bottom: [f32; 4],
    pub viewport: [f32; 4],
    pub fog_color: [f32; 4],
    pub fog_params: [f32; 4],
    pub counts: [f32; 4],
    pub env_scale: [f32; 4],
    pub lights: [LightGpu; 8],
    /// The shadow-casting light's view projection, and on, light index, texel, normal offset.
    pub shadow_matrix: [f32; 16],
    pub shadow: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MaterialGpu {
    base_factor: [f32; 4],
    emissive: [f32; 4],
    tint_r0: [f32; 4],
    tint_r1: [f32; 4],
    tint_r2: [f32; 4],
    tint_offset: [f32; 4],
    csb: [f32; 4],
    reflectivity: [f32; 4],
    pbr: [f32; 4],
    flags: [f32; 4],
    has: [f32; 4],
    illum: [f32; 4],
    illum_tint: [f32; 4],
    extra: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PostGpu {
    pub exposure: [f32; 4],
    pub curve_a: [f32; 4],
    pub curve_b: [f32; 4],
    pub split: [f32; 4],
    pub top: [f32; 4],
    pub bottom: [f32; 4],
    pub glow: [f32; 4],
    pub glow_at: [f32; 4],
    pub inv_view_proj: [f32; 16],
    pub sky: [f32; 4],
    pub bloom2: [f32; 4],
    pub lut: [f32; 4],
    pub bloom_w: [[f32; 4]; 5],
    /// Backdrop: mode (0 studio, 1 radial, 2 vertical, 3 plain, 4 image), spread, vignette,
    /// middle stop; centre, frame aspect, image aspect; three sRGB stops.
    pub bg: [f32; 4],
    pub bg_at: [f32; 4],
    pub bg0: [f32; 4],
    pub bg1: [f32; 4],
    pub bg2: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FloorGpu {
    pub center: [f32; 4],
    pub shadow: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Blend {
    Opaque,
    Alpha,
    Additive,
}

pub struct Pipelines {
    pub(crate) frame_layout: wgpu::BindGroupLayout,
    frame_only_layout: wgpu::BindGroupLayout,
    material_layout: wgpu::BindGroupLayout,
    draw_layout: wgpu::BindGroupLayout,
    floor_layout: wgpu::BindGroupLayout,
    post_layout: wgpu::BindGroupLayout,
    main: HashMap<(Blend, bool), wgpu::RenderPipeline>,
    outline: wgpu::RenderPipeline,
    depth: wgpu::RenderPipeline,
    floor: wgpu::RenderPipeline,
    post: wgpu::RenderPipeline,
    threshold: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
    shadow: wgpu::RenderPipeline,
    sampler_repeat: wgpu::Sampler,
    sampler_clamp: wgpu::Sampler,
    white: wgpu::TextureView,
    black: wgpu::TextureView,
    flat_normal: wgpu::TextureView,
    black_hdr: wgpu::TextureView,
    black_cube: wgpu::TextureView,
    pub(crate) hideout_none: crate::hideout::HideoutDefaults,
}

fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    }
}

fn texture_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
        count: None,
    }
}

fn cube_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::Cube, multisampled: false },
        count: None,
    }
}

fn texture3d_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false },
        count: None,
    }
}

fn sampler_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None }
}

const VERTEX_ATTRS: [wgpu::VertexAttribute; 9] = wgpu::vertex_attr_array![
    0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32x2, 4 => Float32x2, 5 => Float32x4, 6 => Uint32x4, 7 => Float32x4, 8 => Float32x2
];

fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &VERTEX_ATTRS }
}

fn solid(device: &wgpu::Device, queue: &wgpu::Queue, rgba: [u8; 4], format: wgpu::TextureFormat) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("viewer solid"),
        size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let bytes: Vec<u8> = if format == HDR { [0u16; 4].iter().flat_map(|v| v.to_le_bytes()).collect() } else { rgba.to_vec() };
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        &bytes,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bytes.len() as u32), rows_per_image: Some(1) },
        wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
    );
    texture.create_view(&Default::default())
}

impl Pipelines {
    /// Compiles the shaders and builds every pipeline. An error (a shader the driver's compiler
    /// rejects, FXC on D3D12 among them) is returned with its message instead of reaching the
    /// device's handler, so the viewer can say why or try another adapter.
    pub fn new(gpu: &Gpu) -> Result<Pipelines, String> {
        let validation = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let internal = gpu.device.push_error_scope(wgpu::ErrorFilter::Internal);
        let memory = gpu.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let pipes = Pipelines::build(gpu);
        let errors = [pollster::block_on(memory.pop()), pollster::block_on(internal.pop()), pollster::block_on(validation.pop())];
        match errors.into_iter().flatten().next() {
            Some(error) => Err(error.to_string()),
            None => Ok(pipes),
        }
    }

    fn build(gpu: &Gpu) -> Pipelines {
        let device = &gpu.device;
        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewer frame"),
            entries: &[
                uniform_entry(0, vf),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                cube_entry(2, wgpu::ShaderStages::FRAGMENT),
                sampler_entry(3, wgpu::ShaderStages::FRAGMENT),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 5, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison), count: None },
                uniform_entry(6, wgpu::ShaderStages::FRAGMENT),
                texture_entry(7, wgpu::ShaderStages::FRAGMENT),
                texture_entry(8, wgpu::ShaderStages::FRAGMENT),
                texture_entry(9, wgpu::ShaderStages::FRAGMENT),
                texture3d_entry(10, wgpu::ShaderStages::FRAGMENT),
                texture3d_entry(11, wgpu::ShaderStages::FRAGMENT),
                texture3d_entry(12, wgpu::ShaderStages::FRAGMENT),
                texture_entry(13, wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let frame_only_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("viewer frame only"), entries: &[uniform_entry(0, vf)] });
        let mut material_entries = vec![uniform_entry(0, vf)];
        for b in 1..=7 {
            material_entries.push(texture_entry(b, vf));
        }
        material_entries.push(sampler_entry(8, vf));
        let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("viewer material"), entries: &material_entries });
        let draw_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewer draw"),
            entries: &[
                uniform_entry(0, vf),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let floor_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("viewer floor"), entries: &[uniform_entry(0, vf)] });
        let post_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewer post"),
            entries: &[
                uniform_entry(0, vf),
                texture_entry(1, wgpu::ShaderStages::FRAGMENT),
                texture_entry(2, wgpu::ShaderStages::FRAGMENT),
                texture_entry(3, wgpu::ShaderStages::FRAGMENT),
                sampler_entry(4, wgpu::ShaderStages::FRAGMENT),
                cube_entry(5, wgpu::ShaderStages::FRAGMENT),
                texture3d_entry(6, wgpu::ShaderStages::FRAGMENT),
                texture_entry(7, wgpu::ShaderStages::FRAGMENT),
            ],
        });

        let common = include_str!("shaders/common.wgsl");
        let scene_src = format!("{common}\n{}", include_str!("shaders/scene.wgsl"));
        let floor_src = format!("{common}\n{}", include_str!("shaders/floor.wgsl"));
        let scene = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("viewer scene"), source: wgpu::ShaderSource::Wgsl(Cow::Owned(scene_src)) });
        let floor_module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("viewer floor"), source: wgpu::ShaderSource::Wgsl(Cow::Owned(floor_src)) });
        let post_module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("viewer post"), source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("shaders/post.wgsl"))) });

        let main_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewer main"),
            bind_group_layouts: &[Some(&frame_layout), Some(&material_layout), Some(&draw_layout)],
            immediate_size: 0,
        });
        let depth_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewer depth"),
            bind_group_layouts: &[Some(&frame_only_layout), None, Some(&draw_layout)],
            immediate_size: 0,
        });
        let floor_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewer floor"),
            bind_group_layouts: &[Some(&frame_layout), Some(&floor_layout)],
            immediate_size: 0,
        });
        let post_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("viewer post"), bind_group_layouts: &[Some(&post_layout)], immediate_size: 0 });

        let premultiplied = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
        };
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
        };
        let msaa = wgpu::MultisampleState { count: gpu.msaa, mask: !0, alpha_to_coverage_enabled: false };
        let make = |label: &str, vs: &str, fs: &str, blend: Option<wgpu::BlendState>, cull: Option<wgpu::Face>, write: bool, compare: wgpu::CompareFunction| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&main_layout),
                vertex: wgpu::VertexState { module: &scene, entry_point: Some(vs), compilation_options: Default::default(), buffers: &[Some(vertex_layout())] },
                primitive: wgpu::PrimitiveState { cull_mode: cull, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH, depth_write_enabled: Some(write), depth_compare: Some(compare), stencil: Default::default(), bias: Default::default() }),
                multisample: msaa,
                fragment: Some(wgpu::FragmentState {
                    module: &scene,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: HDR, blend, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let mut main = HashMap::new();
        for (blend, state, write) in [(Blend::Opaque, None, true), (Blend::Alpha, Some(premultiplied), false), (Blend::Additive, Some(additive), false)] {
            for double in [false, true] {
                let cull = if double { None } else { Some(wgpu::Face::Back) };
                main.insert((blend, double), make("viewer main", "vs_main", "fs_main", state, cull, write, wgpu::CompareFunction::LessEqual));
            }
        }
        let outline = make("viewer outline", "vs_outline", "fs_outline", None, Some(wgpu::Face::Front), true, wgpu::CompareFunction::LessEqual);
        let depth = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("viewer rim depth"),
            layout: Some(&depth_layout),
            vertex: wgpu::VertexState { module: &scene, entry_point: Some("vs_depth"), compilation_options: Default::default(), buffers: &[Some(vertex_layout())] },
            primitive: wgpu::PrimitiveState { cull_mode: Some(wgpu::Face::Back), ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Less), stencil: Default::default(), bias: Default::default() }),
            multisample: Default::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });
        // The key light's shadow map: depth only, both faces (cloth and capes are thin), with a
        // slope-scaled bias against acne.
        let shadow = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("viewer shadow"),
            layout: Some(&depth_layout),
            vertex: wgpu::VertexState { module: &scene, entry_point: Some("vs_depth"), compilation_options: Default::default(), buffers: &[Some(vertex_layout())] },
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 },
            }),
            multisample: Default::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });
        let floor = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("viewer floor"),
            layout: Some(&floor_pl),
            vertex: wgpu::VertexState { module: &floor_module, entry_point: Some("vs_floor"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH, depth_write_enabled: Some(false), depth_compare: Some(wgpu::CompareFunction::LessEqual), stencil: Default::default(), bias: Default::default() }),
            multisample: msaa,
            fragment: Some(wgpu::FragmentState {
                module: &floor_module,
                entry_point: Some("fs_floor"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: HDR, blend: Some(premultiplied), write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let full = |label: &str, fs: &str, format: wgpu::TextureFormat, blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&post_pl),
                vertex: wgpu::VertexState { module: &post_module, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &post_module,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let post = full("viewer post", "fs_post", OUTPUT, None);
        let threshold = full("viewer bloom threshold", "fs_threshold", HDR, None);
        let down = full("viewer bloom down", "fs_down", HDR, None);
        let weighted = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::Constant, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
        };
        let up = full("viewer bloom up", "fs_up", HDR, Some(weighted));
        let sampler = |mode: wgpu::AddressMode| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("viewer sampler"),
                address_mode_u: mode,
                address_mode_v: mode,
                address_mode_w: mode,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                anisotropy_clamp: 8,
                ..Default::default()
            })
        };
        let sampler_repeat = sampler(wgpu::AddressMode::Repeat);
        let sampler_clamp = sampler(wgpu::AddressMode::ClampToEdge);
        let q = &gpu.queue;
        Pipelines {
            white: solid(device, q, [255; 4], wgpu::TextureFormat::Rgba8Unorm),
            black: solid(device, q, [0, 0, 0, 255], wgpu::TextureFormat::Rgba8Unorm),
            flat_normal: solid(device, q, [128, 128, 255, 255], wgpu::TextureFormat::Rgba8Unorm),
            black_hdr: solid(device, q, [0; 4], HDR),
            black_cube: upload_cube(gpu, &[(1, vec![[0.0f32; 4]; 6])]),
            hideout_none: crate::hideout::HideoutDefaults::new(gpu),
            frame_layout,
            frame_only_layout,
            material_layout,
            draw_layout,
            floor_layout,
            post_layout,
            main,
            outline,
            depth,
            floor,
            post,
            threshold,
            down,
            up,
            shadow,
            sampler_repeat,
            sampler_clamp,
        }
    }
}

/// Box-filtered mips of an RGBA8 image, largest first.
pub fn mip_chain(width: u32, height: u32, rgba: &[u8]) -> Vec<(u32, u32, Vec<u8>)> {
    let mut levels = vec![(width, height, rgba.to_vec())];
    while let Some((w, h, data)) = levels.last() {
        if *w == 1 && *h == 1 {
            break;
        }
        let (nw, nh) = ((*w / 2).max(1), (*h / 2).max(1));
        let mut out = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..4 {
                    let mut sum = 0u32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let sx = (x * 2 + dx).min(*w - 1);
                        let sy = (y * 2 + dy).min(*h - 1);
                        sum += data[((sy * *w + sx) * 4 + c) as usize] as u32;
                    }
                    out[((y * nw + x) * 4 + c) as usize] = ((sum + 2) / 4) as u8;
                }
            }
        }
        levels.push((nw, nh, out));
    }
    levels
}

/// Uploads an image with its mips, inside the device's edge limit and the texture budget:
/// past the budget the largest mips go first (down to 64 pixels), then the texture is left out
/// (`None`, and the material falls back to a flat one).
fn upload_image(gpu: &Gpu, image: &crate::model::Image, srgb: bool) -> Option<wgpu::TextureView> {
    let (iw, ih) = (image.width, image.height);
    if iw == 0 || ih == 0 || (image.rgba.len() as u64) < u64::from(iw) * u64::from(ih) * 4 {
        gpu.with_usage(|u| u.skip(format!("a {iw}x{ih} texture with missing pixels")));
        return None;
    }
    let mut levels = mip_chain(iw, ih, &image.rgba);
    let max_edge = MAX_TEXTURE_EDGE.min(gpu.limits.max_texture_dimension_2d).max(1);
    // Drop levels above the edge cap.
    while levels.len() > 1 && levels[0].0.max(levels[0].1) > max_edge {
        levels.remove(0);
    }
    let bytes = |levels: &[(u32, u32, Vec<u8>)]| levels.iter().map(|l| l.2.len() as u64).sum::<u64>();
    let budget = gpu.texture_budget;
    let fits = gpu.with_usage(|u| {
        let mut downscaled = false;
        while levels.len() > 1 && levels[0].0.max(levels[0].1) > 64 && u.texture_bytes + bytes(&levels) > budget {
            levels.remove(0);
            downscaled = true;
        }
        let size = bytes(&levels);
        if u.texture_bytes + size > budget {
            u.skip(format!("a {iw}x{ih} texture: over the {} MB texture budget", mb(budget)));
            return false;
        }
        u.texture_bytes += size;
        u.textures += 1;
        u.downscaled += downscaled as usize;
        true
    });
    if !fits {
        return None;
    }
    let (w, h) = (levels[0].0, levels[0].1);
    let format = if srgb { wgpu::TextureFormat::Rgba8UnormSrgb } else { wgpu::TextureFormat::Rgba8Unorm };
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("viewer texture"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (i, (lw, lh, data)) in levels.iter().enumerate() {
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: i as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(lw * 4), rows_per_image: Some(*lh) },
            wgpu::Extent3d { width: *lw, height: *lh, depth_or_array_layers: 1 },
        );
    }
    Some(texture.create_view(&Default::default()))
}

#[derive(Clone)]
pub struct GpuPrim {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
    material: usize,
    mesh: usize,
    blend: Blend,
    double: bool,
    outline: bool,
    /// Alpha-tested (hair cards, foliage): left out of the shadow map rather than cast as quads.
    masked: bool,
    pub visible: bool,
    pub center: Vec3,
    /// The material's own blend, before an instance's tint turns it translucent.
    own_blend: Blend,
    /// Shown by the shell rule, before an instance draws only one primitive.
    shown: bool,
    index_in_mesh: usize,
}

/// A model's textures, materials and vertex and index buffers, uploaded once and shared by
/// every placement of it (a room places one aggregate glb hundreds of times, one fragment each).
pub struct GpuMesh {
    prims: Vec<GpuPrim>,
    materials: Vec<wgpu::BindGroup>,
}

/// A placed model: the shared mesh plus its own palette, draw uniforms and visibility.
pub struct GpuModel {
    prims: Vec<GpuPrim>,
    materials: Vec<wgpu::BindGroup>,
    draws: Vec<wgpu::BindGroup>,
    palette: wgpu::Buffer,
    palette_len: usize,
    _draw_buffers: Vec<wgpu::Buffer>,
}

impl GpuMesh {
    /// Uploads `model`. A primitive whose buffers pass the device's limit or the geometry budget
    /// is left out (recorded in `Gpu::usage`); `what` names the model there.
    pub fn new(gpu: &Gpu, p: &Pipelines, model: &Model, what: &str) -> GpuMesh {
        use wgpu::util::DeviceExt;
        let device = &gpu.device;
        let mut cache: HashMap<(usize, bool), Option<wgpu::TextureView>> = HashMap::new();
        let mut view_for = |slot: Option<crate::material::Slot>, srgb: bool| -> Option<wgpu::TextureView> {
            let slot = slot?;
            let tex = model.textures.get(slot.texture)?;
            let image = model.images.get(tex.image)?;
            cache.entry((tex.image, srgb)).or_insert_with(|| upload_image(gpu, image, srgb)).clone()
        };
        let resolved: Vec<Resolved> = model.materials.iter().map(|m| m.resolve()).collect();
        let default_material = crate::material::Material::default();
        let mut materials = Vec::new();
        let material_count = model.materials.len().max(1);
        for i in 0..material_count {
            let m = model.materials.get(i).unwrap_or(&default_material);
            let r = resolved.get(i).cloned().unwrap_or_else(|| m.resolve());
            let base = view_for(m.base_tex, true);
            let normal = view_for(m.normal_tex, false);
            let mr = view_for(m.mr_tex, false);
            let occ = view_for(m.occlusion_tex, false);
            let si_mask = r.self_illum.as_ref().and_then(|s| s.mask);
            let emissive = view_for(si_mask.or(m.emissive_tex), r.self_illum.is_none());
            let tint = view_for(r.tint_mask, false);
            let outline = view_for(r.outline_mask, false);
            let alpha_mode = match m.alpha {
                AlphaMode::Opaque => 0.0,
                AlphaMode::Mask(_) => 1.0,
                AlphaMode::Blend => 2.0,
                AlphaMode::Additive => 3.0,
            };
            let cutoff = if let AlphaMode::Mask(c) = m.alpha { c } else { 0.5 };
            let si = r.self_illum.clone();
            let uniform = MaterialGpu {
                base_factor: r.base_factor,
                emissive: [m.emissive[0], m.emissive[1], m.emissive[2], m.emissive_strength],
                tint_r0: [r.tint_matrix[0][0], r.tint_matrix[0][1], r.tint_matrix[0][2], if r.tint_masked { 1.0 } else { 0.0 }],
                tint_r1: [r.tint_matrix[1][0], r.tint_matrix[1][1], r.tint_matrix[1][2], 0.0],
                tint_r2: [r.tint_matrix[2][0], r.tint_matrix[2][1], r.tint_matrix[2][2], 0.0],
                tint_offset: [r.tint_offset[0], r.tint_offset[1], r.tint_offset[2], if tint.is_some() { 1.0 } else { 0.0 }],
                csb: match r.csb {
                    Some(c) => [c[0], c[1], c[2], 1.0],
                    None => [1.0, 1.0, 1.0, 0.0],
                },
                reflectivity: [r.reflectivity[0], r.reflectivity[1], r.reflectivity[2], cutoff],
                pbr: [m.metallic, m.roughness, m.occlusion_strength, m.normal_scale],
                flags: [if r.npr { 1.0 } else { 0.0 }, if r.hemi_oct_normals { 1.0 } else { 0.0 }, if m.unlit { 1.0 } else { 0.0 }, alpha_mode],
                has: [base.is_some() as u8 as f32, normal.is_some() as u8 as f32, mr.is_some() as u8 as f32, occ.is_some() as u8 as f32],
                illum: [emissive.is_some() as u8 as f32, si.is_some() as u8 as f32, si.as_ref().map_or(0.0, |s| s.scale), si.as_ref().map_or(0.0, |s| s.albedo_factor)],
                illum_tint: {
                    let t = si.as_ref().map_or([1.0; 3], |s| s.tint);
                    [t[0], t[1], t[2], r.outline_thickness]
                },
                extra: [outline.is_some() as u8 as f32, if m.morphic.is_none() { 1.0 } else { 0.0 }, m.transmission, 0.0],
            };
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("viewer material"), contents: bytemuck::bytes_of(&uniform), usage: wgpu::BufferUsages::UNIFORM });
            let repeat = m.base_tex.and_then(|s| model.textures.get(s.texture)).is_none_or(|t| t.repeat);
            let bind = |v: &Option<wgpu::TextureView>, fallback: &wgpu::TextureView| v.clone().unwrap_or_else(|| fallback.clone());
            let views = [bind(&base, &p.white), bind(&normal, &p.flat_normal), bind(&mr, &p.white), bind(&occ, &p.white), bind(&emissive, &p.white), bind(&tint, &p.white), bind(&outline, &p.black)];
            let mut entries = vec![wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }];
            for (i, v) in views.iter().enumerate() {
                entries.push(wgpu::BindGroupEntry { binding: i as u32 + 1, resource: wgpu::BindingResource::TextureView(v) });
            }
            entries.push(wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::Sampler(if repeat { &p.sampler_repeat } else { &p.sampler_clamp }) });
            materials.push(device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("viewer material"), layout: &p.material_layout, entries: &entries }));
        }
        let mut prims = Vec::new();
        for (mi, mesh) in model.meshes.iter().enumerate() {
            for &pi in &mesh.primitives {
                let Some(prim) = model.primitives.get(pi) else { continue };
                if prim.indices.is_empty() || prim.vertices.is_empty() {
                    continue;
                }
                let sizes = [(prim.vertices.len() * std::mem::size_of::<Vertex>()) as u64, (prim.indices.len() * 4) as u64];
                if !gpu.reserve_buffers(&sizes, what) {
                    continue;
                }
                let material = prim.material.unwrap_or(0).min(material_count - 1);
                let m = model.materials.get(material).unwrap_or(&default_material);
                let r = &resolved.get(material).cloned().unwrap_or_else(|| m.resolve());
                let shown = !prim.authored_shell || (m.shell_visible(r) && shell_wants_no_growth(model, m, r));
                let blend = match m.alpha {
                    AlphaMode::Opaque | AlphaMode::Mask(_) => Blend::Opaque,
                    AlphaMode::Blend => Blend::Alpha,
                    AlphaMode::Additive => Blend::Additive,
                };
                let blend = if m.transmission > 0.0 && blend == Blend::Opaque { Blend::Alpha } else { blend };
                prims.push(GpuPrim {
                    vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("viewer vertices"), contents: bytemuck::cast_slice(&prim.vertices), usage: wgpu::BufferUsages::VERTEX }),
                    indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("viewer indices"), contents: bytemuck::cast_slice(&prim.indices), usage: wgpu::BufferUsages::INDEX }),
                    count: prim.indices.len() as u32,
                    material,
                    mesh: mi,
                    blend,
                    double: m.double_sided || m.morphic.as_ref().is_some_and(|x| x.flag("F_RENDER_BACKFACES")),
                    outline: r.outline_thickness > 0.0 && r.outline_mask.is_some() && !prim.authored_shell,
                    masked: matches!(m.alpha, AlphaMode::Mask(_)),
                    visible: shown,
                    center: prim.bounds.center(),
                    own_blend: blend,
                    shown,
                    index_in_mesh: prim.index_in_mesh,
                });
            }
        }
        GpuMesh { prims, materials }
    }

    /// One placement: its own palette of `palette_len` matrices, draw uniforms per mesh
    /// (`bases`), tint, and only primitive `only` of each mesh when given (an aggregate
    /// fragment). The textures and buffers are the mesh's.
    pub fn instance(&self, gpu: &Gpu, p: &Pipelines, palette_len: usize, bases: &[u32], tint: [f32; 4], only: Option<usize>) -> GpuModel {
        use wgpu::util::DeviceExt;
        let device = &gpu.device;
        // A palette past the storage binding limit is cut (the bones past it read zero).
        let max_len = (gpu.limits.max_storage_buffer_binding_size / 64).max(1) as usize;
        let palette_len = if palette_len > max_len {
            gpu.with_usage(|u| u.skip(format!("a skeleton of {palette_len} bones: over this GPU's {max_len}")));
            max_len
        } else {
            palette_len
        };
        let palette = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewer palette"),
            size: (palette_len.max(1) * 64) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut draws = Vec::new();
        let mut draw_buffers = Vec::new();
        for &base in bases {
            let mut words: [u32; 8] = [base, 0, 0, 0, 0, 0, 0, 0];
            for (w, t) in words[4..].iter_mut().zip(tint) {
                *w = t.to_bits();
            }
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("viewer draw"), contents: bytemuck::cast_slice(&words), usage: wgpu::BufferUsages::UNIFORM });
            draws.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("viewer draw"),
                layout: &p.draw_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: palette.as_entire_binding() }],
            }));
            draw_buffers.push(buffer);
        }
        // A fragment keeps only its primitive: the frame never walks the others.
        let prims = self
            .prims
            .iter()
            .filter(|q| only.is_none_or(|i| i == q.index_in_mesh) && q.mesh < draws.len())
            .map(|q| {
                let mut q = q.clone();
                q.blend = if tint[3] < 0.999 && q.own_blend == Blend::Opaque { Blend::Alpha } else { q.own_blend };
                q.visible = q.shown;
                q
            })
            .collect();
        GpuModel { prims, materials: self.materials.clone(), draws, palette, palette_len, _draw_buffers: draw_buffers }
    }
}

impl GpuModel {
    pub fn new(gpu: &Gpu, p: &Pipelines, model: &Model, palette_len: usize, bases: &[u32], tint: [f32; 4], only_primitive: Option<usize>) -> GpuModel {
        GpuMesh::new(gpu, p, model, "a model").instance(gpu, p, palette_len, bases, tint, only_primitive)
    }

    pub fn write_palette(&self, gpu: &Gpu, palette: &[Mat4]) {
        let n = palette.len().min(self.palette_len.max(1));
        let flat: Vec<[f32; 16]> = palette[..n].iter().map(|m| m.to_cols_array()).collect();
        gpu.queue.write_buffer(&self.palette, 0, bytemuck::cast_slice(&flat));
    }

    pub fn triangle_count(&self) -> u32 {
        self.prims.iter().filter(|p| p.visible).map(|p| p.count / 3).sum()
    }
}

/// Grimoire reveals an authored shell only when its outline mask asks for no growth: the
/// mask is a tiny black constant, or outlines are off for the material.
fn shell_wants_no_growth(model: &Model, m: &crate::material::Material, r: &Resolved) -> bool {
    if m.morphic.as_ref().is_some_and(|x| x.flag("F_DISABLE_NPR_OUTLINE")) {
        return true;
    }
    let Some(slot) = m.morphic.as_ref().and_then(|x| x.textures.get("g_tNprOutlineMask")) else { return false };
    let _ = r;
    let Some(image) = model.textures.get(*slot).and_then(|t| model.images.get(t.image)) else { return false };
    image.width <= 4 && image.height <= 4 && image.rgba.as_chunks::<4>().0.iter().all(|p| p[0] == 0)
}

/// Size-dependent render targets.
pub struct Targets {
    pub width: u32,
    pub height: u32,
    msaa_color: Option<wgpu::TextureView>,
    msaa_depth: wgpu::TextureView,
    resolved: [wgpu::TextureView; 2],
    rim_depth: wgpu::TextureView,
    bloom: Vec<(u32, u32, wgpu::TextureView)>,
    output: wgpu::Texture,
    output_view: wgpu::TextureView,
    readback: wgpu::Buffer,
    padded_row: u32,
}

fn target(device: &wgpu::Device, w: u32, h: u32, format: wgpu::TextureFormat, samples: u32, usage: wgpu::TextureUsages) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("viewer target"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: samples,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

impl Targets {
    pub fn new(gpu: &Gpu, width: u32, height: u32) -> Targets {
        let device = &gpu.device;
        let rt = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let sampled = wgpu::TextureUsages::TEXTURE_BINDING;
        let msaa_color = (gpu.msaa > 1).then(|| target(device, width, height, HDR, gpu.msaa, rt).create_view(&Default::default()));
        let msaa_depth = target(device, width, height, DEPTH, gpu.msaa, rt).create_view(&Default::default());
        let resolved = [target(device, width, height, HDR, 1, rt | sampled).create_view(&Default::default()), target(device, width, height, HDR, 1, rt | sampled).create_view(&Default::default())];
        let rim_depth = target(device, width, height, DEPTH, 1, rt | sampled).create_view(&Default::default());
        let mut bloom = Vec::new();
        let (mut bw, mut bh) = (width, height);
        for _ in 0..BLOOM_LEVELS {
            bw = (bw / 2).max(1);
            bh = (bh / 2).max(1);
            bloom.push((bw, bh, target(device, bw, bh, HDR, 1, rt | sampled).create_view(&Default::default())));
        }
        let output = target(device, width, height, OUTPUT, 1, rt | wgpu::TextureUsages::COPY_SRC);
        let output_view = output.create_view(&Default::default());
        let padded_row = (width * 4).div_ceil(256) * 256;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewer readback"),
            size: (padded_row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Targets { width, height, msaa_color, msaa_depth, resolved, rim_depth, bloom, output, output_view, readback, padded_row }
    }
}

/// One shown model in a frame: its buffers and the draw order for alpha.
pub struct Shown<'a> {
    pub model: &'a GpuModel,
    pub hero: bool,
    /// World matrix applied after the model's own (scene placement); used for sorting only.
    pub eye_distance: &'a dyn Fn(Vec3, usize) -> f32,
}

/// What the frame cost, in milliseconds, measured on the calling thread.
#[derive(Clone, Copy, Debug, Default)]
pub struct Timings {
    /// Encoding and submitting the passes.
    pub encode_ms: f32,
    /// Waiting for the GPU to finish and the readback to map.
    pub gpu_wait_ms: f32,
    /// Copying the mapped rows out, without row padding.
    pub unpad_ms: f32,
}

pub struct FrameInputs<'a> {
    pub frame: FrameGpu,
    pub post: PostGpu,
    pub floor: Option<FloorGpu>,
    /// The "after" layer, then the optional "before" layer for a split.
    pub layers: [Option<&'a [Shown<'a>]>; 2],
    pub read: bool,
    /// The reflection probe and the sky, when the scene has them.
    pub env_cube: Option<&'a wgpu::TextureView>,
    pub sky_cube: Option<&'a wgpu::TextureView>,
    /// The player's backdrop picture.
    pub backdrop: Option<&'a wgpu::TextureView>,
    /// The key light's view projection when it casts a shadow this frame.
    pub shadow: Option<[f32; 16]>,
    /// Only the hero casts into the shadow map: the light's static shadow is baked (a
    /// stationary light's lightmap shadow mask), so the room must not shadow itself twice.
    pub shadow_hero_only: bool,
    /// The Hideout's baked data and this frame's uniform for it (lights in the frame's order).
    pub hideout: Option<(&'a crate::hideout::HideoutGpu, crate::hideout::HideoutUniform)>,
    /// Per bloom level, the weight times tint the map's post gives it (level 0 first).
    pub bloom_weights: [[f32; 3]; 5],
    /// Particle effects, drawn in the "after" layer once its surfaces are down.
    pub particles: &'a [&'a crate::particles::ParticleGpu],
}

pub struct FrameResources {
    frame_buf: wgpu::Buffer,
    post_buf: wgpu::Buffer,
    floor_buf: wgpu::Buffer,
    frame_only: wgpu::BindGroup,
    floor_group: wgpu::BindGroup,
    /// The light's camera for the shadow pass (only `view_proj` is read there).
    shadow_buf: wgpu::Buffer,
    shadow_only: wgpu::BindGroup,
    shadow_map: wgpu::TextureView,
    shadow_sampler: wgpu::Sampler,
}

/// The shadow map's edge in texels.
pub const SHADOW_SIZE: u32 = 2048;

impl FrameResources {
    pub fn new(gpu: &Gpu, p: &Pipelines) -> FrameResources {
        let device = &gpu.device;
        let buf = |size: usize, label: &str| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let frame_buf = buf(std::mem::size_of::<FrameGpu>(), "viewer frame");
        let post_buf = buf(std::mem::size_of::<PostGpu>(), "viewer post");
        let floor_buf = buf(std::mem::size_of::<FloorGpu>(), "viewer floor");
        let frame_only = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("viewer frame only"), layout: &p.frame_only_layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: frame_buf.as_entire_binding() }] });
        let floor_group = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("viewer floor"), layout: &p.floor_layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: floor_buf.as_entire_binding() }] });
        let shadow_buf = buf(std::mem::size_of::<FrameGpu>(), "viewer shadow camera");
        let shadow_only = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("viewer shadow camera"), layout: &p.frame_only_layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: shadow_buf.as_entire_binding() }] });
        let shadow_map = target(device, SHADOW_SIZE, SHADOW_SIZE, DEPTH, 1, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING).create_view(&Default::default());
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("viewer shadow"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        FrameResources { frame_buf, post_buf, floor_buf, frame_only, floor_group, shadow_buf, shadow_only, shadow_map, shadow_sampler }
    }
}

#[allow(clippy::too_many_arguments)]
fn post_group(gpu: &Gpu, p: &Pipelines, r: &FrameResources, a: &wgpu::TextureView, b: &wgpu::TextureView, bloom: &wgpu::TextureView, sky: Option<&wgpu::TextureView>, lut: Option<&wgpu::TextureView>, backdrop: Option<&wgpu::TextureView>) -> wgpu::BindGroup {
    gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("viewer post"),
        layout: &p.post_layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: r.post_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(a) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(b) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(bloom) },
            wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&p.sampler_clamp) },
            wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(sky.unwrap_or(&p.black_cube)) },
            wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(lut.unwrap_or(&p.hideout_none.d3)) },
            wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(backdrop.unwrap_or(&p.black)) },
        ],
    })
}

fn color_attachment<'a>(t: &'a Targets, layer: usize) -> wgpu::RenderPassColorAttachment<'a> {
    let (view, resolve) = match &t.msaa_color {
        Some(m) => (m, Some(&t.resolved[layer])),
        None => (&t.resolved[layer], None),
    };
    wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: resolve,
        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: if resolve.is_some() { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store } },
    }
}

/// Encodes, submits and (with `inputs.read`) reads back one frame into `out` as tightly
/// packed sRGB RGBA8.
pub fn render_frame(gpu: &Gpu, p: &Pipelines, r: &FrameResources, t: &Targets, inputs: &FrameInputs, out: &mut Vec<u8>) -> Timings {
    let started = Instant::now();
    let device = &gpu.device;
    gpu.queue.write_buffer(&r.frame_buf, 0, bytemuck::bytes_of(&inputs.frame));
    gpu.queue.write_buffer(&r.post_buf, 0, bytemuck::bytes_of(&inputs.post));
    if let Some(floor) = &inputs.floor {
        gpu.queue.write_buffer(&r.floor_buf, 0, bytemuck::bytes_of(floor));
    }
    let frame_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("viewer frame"),
        layout: &p.frame_layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: r.frame_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&t.rim_depth) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(inputs.env_cube.unwrap_or(&p.black_cube)) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&p.sampler_clamp) },
            wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&r.shadow_map) },
            wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(&r.shadow_sampler) },
            wgpu::BindGroupEntry { binding: 6, resource: match &inputs.hideout {
                Some((h, _)) => h.buffer.as_entire_binding(),
                None => p.hideout_none.buffer.as_entire_binding(),
            } },
            wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(inputs.hideout.as_ref().map_or(&p.hideout_none.d2_hdr, |h| &h.0.lightmap)) },
            wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(inputs.hideout.as_ref().map_or(&p.hideout_none.d2, |h| &h.0.direction)) },
            wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(inputs.hideout.as_ref().map_or(&p.hideout_none.d2, |h| &h.0.shadows)) },
            wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(inputs.hideout.as_ref().map_or(&p.hideout_none.d3, |h| &h.0.probes)) },
            wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::TextureView(inputs.hideout.as_ref().map_or(&p.hideout_none.d3, |h| &h.0.probe_shadows)) },
            wgpu::BindGroupEntry { binding: 12, resource: wgpu::BindingResource::TextureView(inputs.hideout.as_ref().map_or(&p.hideout_none.d3, |h| &h.0.fog)) },
            wgpu::BindGroupEntry { binding: 13, resource: wgpu::BindingResource::TextureView(inputs.hideout.as_ref().map_or(&p.hideout_none.d2, |h| &h.0.cookies)) },
        ],
    });
    if let Some((h, u)) = &inputs.hideout {
        gpu.queue.write_buffer(&h.buffer, 0, bytemuck::bytes_of(u));
    }
    if let Some(m) = inputs.shadow {
        let mut light = inputs.frame;
        light.view_proj = m;
        gpu.queue.write_buffer(&r.shadow_buf, 0, bytemuck::bytes_of(&light));
    }
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("viewer frame") });
    for (layer, shown) in inputs.layers.iter().enumerate() {
        let Some(shown) = shown else { continue };
        // The key light's shadow map, from this layer's models (the before side casts its own).
        if inputs.shadow.is_some() {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewer shadow"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &r.shadow_map, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }), stencil_ops: None }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&p.shadow);
            pass.set_bind_group(0, &r.shadow_only, &[]);
            for s in shown.iter().filter(|s| s.hero || !inputs.shadow_hero_only) {
                for prim in s.model.prims.iter().filter(|q| q.visible && q.blend == Blend::Opaque && !q.masked) {
                    pass.set_bind_group(2, &s.model.draws[prim.mesh], &[]);
                    pass.set_vertex_buffer(0, prim.vertices.slice(..));
                    pass.set_index_buffer(prim.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..prim.count, 0, 0..1);
                }
            }
        }
        // Hero depth for the rim occlusion test.
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewer rim depth"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &t.rim_depth, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }), stencil_ops: None }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&p.depth);
            pass.set_bind_group(0, &r.frame_only, &[]);
            for s in shown.iter().filter(|s| s.hero) {
                for prim in s.model.prims.iter().filter(|q| q.visible && q.blend == Blend::Opaque) {
                    pass.set_bind_group(2, &s.model.draws[prim.mesh], &[]);
                    pass.set_vertex_buffer(0, prim.vertices.slice(..));
                    pass.set_index_buffer(prim.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..prim.count, 0, 0..1);
                }
            }
        }
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("viewer scene"),
            color_attachments: &[Some(color_attachment(t, layer))],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &t.msaa_depth, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }), stencil_ops: None }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, &frame_group, &[]);
        // Opaque, then the outline hulls, the floor, then blended back to front.
        for s in shown.iter() {
            for prim in s.model.prims.iter().filter(|q| q.visible && q.blend == Blend::Opaque) {
                pass.set_pipeline(&p.main[&(Blend::Opaque, prim.double)]);
                pass.set_bind_group(1, &s.model.materials[prim.material], &[]);
                pass.set_bind_group(2, &s.model.draws[prim.mesh], &[]);
                pass.set_vertex_buffer(0, prim.vertices.slice(..));
                pass.set_index_buffer(prim.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..prim.count, 0, 0..1);
            }
        }
        pass.set_pipeline(&p.outline);
        for s in shown.iter() {
            for prim in s.model.prims.iter().filter(|q| q.visible && q.outline) {
                pass.set_bind_group(1, &s.model.materials[prim.material], &[]);
                pass.set_bind_group(2, &s.model.draws[prim.mesh], &[]);
                pass.set_vertex_buffer(0, prim.vertices.slice(..));
                pass.set_index_buffer(prim.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..prim.count, 0, 0..1);
            }
        }
        if inputs.floor.is_some() {
            pass.set_pipeline(&p.floor);
            pass.set_bind_group(1, &r.floor_group, &[]);
            pass.draw(0..6, 0..1);
        }
        let mut blended: Vec<(f32, &Shown, &GpuPrim)> = Vec::new();
        for s in shown.iter() {
            for prim in s.model.prims.iter().filter(|q| q.visible && q.blend != Blend::Opaque) {
                blended.push(((s.eye_distance)(prim.center, prim.mesh), s, prim));
            }
        }
        blended.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (_, s, prim) in blended {
            pass.set_pipeline(&p.main[&(prim.blend, prim.double)]);
            pass.set_bind_group(1, &s.model.materials[prim.material], &[]);
            pass.set_bind_group(2, &s.model.draws[prim.mesh], &[]);
            pass.set_vertex_buffer(0, prim.vertices.slice(..));
            pass.set_index_buffer(prim.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..prim.count, 0, 0..1);
        }
        // Particles last: depth tested against the scene, never writing it.
        if layer == 0 {
            for pg in inputs.particles {
                pg.draw(&mut pass);
            }
        }
    }
    // Bloom from the "after" layer: threshold to half size, down the chain, back up.
    let bloom_on = inputs.post.curve_b[2] > 0.0;
    if bloom_on {
        let full = |enc: &mut wgpu::CommandEncoder, pipeline: &wgpu::RenderPipeline, src: &wgpu::TextureView, dst: &wgpu::TextureView, clear: bool, weight: Option<([f32; 3], u32)>| {
            let group = post_group(gpu, p, r, src, &p.black_hdr, &p.black_hdr, None, None, None);
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewer bloom"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: dst,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: if clear { wgpu::LoadOp::Clear(wgpu::Color::BLACK) } else { wgpu::LoadOp::Load }, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[]);
            let level = match weight {
                Some((w, level)) => {
                    pass.set_blend_constant(wgpu::Color { r: w[0] as f64, g: w[1] as f64, b: w[2] as f64, a: 1.0 });
                    level
                }
                None => 0,
            };
            // The instance index carries the source level to fs_up (the coarsest scales itself).
            pass.draw(0..3, level..level + 1);
        };
        full(&mut enc, &p.threshold, &t.resolved[0], &t.bloom[0].2, true, None);
        for i in 1..t.bloom.len() {
            full(&mut enc, &p.down, &t.bloom[i - 1].2, &t.bloom[i].2, true, None);
        }
        // Each level ends up weighted once: the destination by the blend constant as the
        // coarser sum is added, the coarsest by itself in fs_up.
        for i in (1..t.bloom.len()).rev() {
            full(&mut enc, &p.up, &t.bloom[i].2, &t.bloom[i - 1].2, false, Some((inputs.bloom_weights[i - 1], i as u32)));
        }
    }
    {
        let before = if inputs.layers[1].is_some() { &t.resolved[1] } else { &p.black_hdr };
        let bloom = if bloom_on { &t.bloom[0].2 } else { &p.black_hdr };
        let group = post_group(gpu, p, r, &t.resolved[0], before, bloom, inputs.sky_cube, inputs.hideout.as_ref().and_then(|h| h.0.lut.as_ref().map(|l| &l.0)), inputs.backdrop);
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("viewer post"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &t.output_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&p.post);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
    if inputs.read {
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &t.output, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &t.readback, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(t.padded_row), rows_per_image: Some(t.height) } },
            wgpu::Extent3d { width: t.width, height: t.height, depth_or_array_layers: 1 },
        );
    }
    gpu.queue.submit([enc.finish()]);
    let encoded = Instant::now();
    let mut timings = Timings { encode_ms: (encoded - started).as_secs_f32() * 1000.0, ..Default::default() };
    // A bounded wait: a hung or lost device becomes an error the caller reports, not a thread
    // stuck here with the view on its last frame.
    let wait = || {
        if let Err(error) = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(FRAME_TIMEOUT) }) {
            gpu.report(format!("the frame did not finish on the GPU: {error}"));
        }
    };
    if !inputs.read {
        wait();
        timings.gpu_wait_ms = encoded.elapsed().as_secs_f32() * 1000.0;
        return timings;
    }
    let slice = t.readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    wait();
    let mapped = Instant::now();
    timings.gpu_wait_ms = (mapped - encoded).as_secs_f32() * 1000.0;
    {
        let row = (t.width * 4) as usize;
        out.resize(row * t.height as usize, 0);
        if let Ok(data) = slice.get_mapped_range() {
            for y in 0..t.height as usize {
                let src = y * t.padded_row as usize;
                out[y * row..(y + 1) * row].copy_from_slice(&data[src..src + row]);
            }
        }
    }
    t.readback.unmap();
    timings.unpad_ms = mapped.elapsed().as_secs_f32() * 1000.0;
    timings
}

/// An sRGB RGBA8 picture with its mips (a backdrop), capped at 2048 px; `None` past the
/// device's texture budget.
pub fn upload_srgb(gpu: &Gpu, width: u32, height: u32, rgba: &[u8]) -> Option<wgpu::TextureView> {
    upload_image(gpu, &crate::model::Image { width, height, rgba: rgba.to_vec() }, true)
}

/// Half floats from f32 (round to nearest, no NaN payloads), for HDR uploads.
pub fn f16_bits(v: f32) -> u16 {
    let x = v.to_bits();
    let sign = ((x >> 16) & 0x8000) as u16;
    let exp = ((x >> 23) & 0xff) as i32;
    let mant = x & 0x7f_ffff;
    if exp == 255 {
        return sign | 0x7c00 | if mant != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 31 {
        return sign | 0x7c00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let m = (mant | 0x80_0000) >> (1 - e);
        return sign | ((m + 0x1000) >> 13) as u16;
    }
    let half = sign as u32 | ((e as u32) << 10) | (mant >> 13);
    // Round to nearest.
    (half + ((mant >> 12) & 1)) as u16
}

/// A cube texture from square linear faces (glTF order +X, -X, +Y, -Y, +Z, -Z) with box mips;
/// each level is `(edge, 6 * edge * edge texels)`.
pub fn upload_cube(gpu: &Gpu, levels: &[(u32, Vec<[f32; 4]>)]) -> wgpu::TextureView {
    let edge = levels[0].0;
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("viewer cube"),
        size: wgpu::Extent3d { width: edge, height: edge, depth_or_array_layers: 6 },
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: HDR,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (level, (e, texels)) in levels.iter().enumerate() {
        let bytes: Vec<u8> = texels.iter().flat_map(|t| t.iter().flat_map(|c| f16_bits(*c).to_le_bytes())).collect();
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: level as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(e * 8), rows_per_image: Some(*e) },
            wgpu::Extent3d { width: *e, height: *e, depth_or_array_layers: 6 },
        );
    }
    texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::Cube), ..Default::default() })
}
