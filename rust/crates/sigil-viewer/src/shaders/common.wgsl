// Shared by scene.wgsl and floor.wgsl: the frame uniform and the hero depth for rim occlusion.

struct Light {
    // xyz: direction toward the light (w = 0) or its position (w = 1 point, 2 spot).
    pos_dir: vec4<f32>,
    // rgb: irradiance; w: range for a point light (0 = no falloff window).
    color: vec4<f32>,
    // A spot's shine direction and the cosines of its outer and inner half-angles.
    spot: vec4<f32>,
    spot_inner: vec4<f32>,
};

struct Frame {
    view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    proj: mat4x4<f32>,
    camera_pos: vec4<f32>,   // xyz, near
    camera_back: vec4<f32>,  // camera +Z axis in world (toward the viewer), far
    key_dir: vec4<f32>,      // primary light direction, exposure
    ambient: array<vec4<f32>, 6>,
    ambient_fit: vec4<f32>,
    env_top: vec4<f32>,      // rgb, environment intensity
    env_bottom: vec4<f32>,   // rgb, world units per game unit
    viewport: vec4<f32>,     // width, height, rim depth on, time
    fog_color: vec4<f32>,    // rgb, fog on
    fog_params: vec4<f32>,   // start, end, density, max amount
    counts: vec4<f32>,       // light count, npr enabled, reflection cube on, its top mip
    env_scale: vec4<f32>,    // reflection cube scale (rgb)
    lights: array<Light, 8>,
    // The shadow-casting light's view projection (world to its clip space).
    shadow_matrix: mat4x4<f32>,
    shadow: vec4<f32>,       // on, light index, texel size (uv), normal offset (world units)
};

struct Material {
    base_factor: vec4<f32>,
    emissive: vec4<f32>,      // rgb factor, strength
    tint_r0: vec4<f32>,       // tint matrix row 0, tint masked
    tint_r1: vec4<f32>,
    tint_r2: vec4<f32>,
    tint_offset: vec4<f32>,   // rgb, has tint mask
    csb: vec4<f32>,           // contrast, saturation, brightness, on
    reflectivity: vec4<f32>,  // rgb pivot, alpha cutoff
    pbr: vec4<f32>,           // metallic, roughness, occlusion strength, normal scale
    flags: vec4<f32>,         // npr, hemi-oct normals, unlit, alpha mode (0 opaque 1 mask 2 blend 3 additive)
    has: vec4<f32>,           // base, normal, metal-rough, occlusion
    illum: vec4<f32>,         // has emissive, self-illum on, scale, albedo factor
    illum_tint: vec4<f32>,    // rgb, outline thickness in game units
    extra: vec4<f32>,         // has outline mask, vertex color, transmission, unused
};

struct Draw {
    palette_base: vec4<u32>,
    // A scene placement's linear tint and opacity.
    tint: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var rim_depth: texture_depth_2d;
// A scene's reflection probe (linear HDR, glTF cube order); 1x1 black without one.
@group(0) @binding(2) var env_cube: texture_cube<f32>;
@group(0) @binding(3) var s_env: sampler;
// The key light's shadow map (one directional or spot light; 1x1 lit without one).
@group(0) @binding(4) var shadow_map: texture_depth_2d;
@group(0) @binding(5) var s_shadow: sampler_comparison;

// 1 lit, 0 in shadow: a 3x3 percentage-closer filter over the key light's depth, the surface
// pushed out along its normal by a texel or so against acne.
fn shadow_at(world: vec3<f32>, n: vec3<f32>) -> f32 {
    if (frame.shadow.x < 0.5) {
        return 1.0;
    }
    let p = frame.shadow_matrix * vec4<f32>(world + n * frame.shadow.w, 1.0);
    if (p.w <= 0.0) {
        return 1.0;
    }
    let ndc = p.xyz / p.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if (uv.x < 0.0 || uv.y < 0.0 || uv.x > 1.0 || uv.y > 1.0 || ndc.z > 1.0 || ndc.z < 0.0) {
        return 1.0;
    }
    let texel = frame.shadow.z;
    var sum = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            sum = sum + textureSampleCompareLevel(shadow_map, s_shadow, uv + vec2<f32>(f32(x), f32(y)) * texel, ndc.z);
        }
    }
    return sum / 9.0;
}

