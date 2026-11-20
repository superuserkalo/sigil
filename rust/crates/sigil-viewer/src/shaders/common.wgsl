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

// The Hideout's baked and volumetric lighting (hideout.rs `HideoutUniform`).
struct Probe {
    world_to_local: mat4x4<f32>,
    size: vec4<f32>,     // voxels, rgbm
    offset: vec4<f32>,   // atlas offset in voxels
    fade: vec4<f32>,     // size over the edge fade distances
};

struct FogVolume {
    world_to_local: mat4x4<f32>,
    params: vec4<f32>,   // sphere, strength, falloff, indirect strength override (< 0 none)
    tint: vec4<f32>,     // linear tint, has tint
};

struct LightEx {
    a: vec4<f32>,        // Source units, skirt, cookie layer (< 0 none), fog strength
    b: vec4<f32>,        // fog only, baked shadow channel (< 0 none)
    x: vec4<f32>,        // the light's Source forward, left, up in glTF space
    y: vec4<f32>,
    z: vec4<f32>,
};

struct HideoutUniform {
    lightmap: vec4<f32>,        // on, direction on, shadows on
    lightmap_scale: vec4<f32>,  // uv scale
    counts: vec4<f32>,          // probes, probe shadows on, cookie layers
    atlas: vec4<f32>,           // probe atlas texels (w, h, depth / 6)
    probes: array<Probe, 8>,
    fog_irradiance: mat4x4<f32>,
    fog: vec4<f32>,             // irradiance on, indirect strength, strength, draw distance
    fog_fade: vec4<f32>,        // fade in start, end, volume count, steps
    fog_volumes: array<FogVolume, 16>,
    lights: array<LightEx, 8>,
};

@group(0) @binding(6) var<uniform> hideout: HideoutUniform;
@group(0) @binding(7) var t_lightmap: texture_2d<f32>;
@group(0) @binding(8) var t_lm_direction: texture_2d<f32>;
@group(0) @binding(9) var t_lm_shadows: texture_2d<f32>;
@group(0) @binding(10) var t_probes: texture_3d<f32>;
@group(0) @binding(11) var t_probe_shadows: texture_3d<f32>;
@group(0) @binding(12) var t_fog: texture_3d<f32>;
@group(0) @binding(13) var t_cookies: texture_2d<f32>;

// glTF world directions to Source's: Source x is glTF z, y is x, z is y.
fn to_source(d: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(d.z, d.x, d.y);
}

// The SH2 lightmap's irradiance along `n` (VRF's lighting.slang as Grimoire's
// source2Lightmap.ts ports it). Already divided by PI.
fn lightmap_irradiance(n_world: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {
    let lmuv = uv * hideout.lightmap_scale.xy;
    let dc = textureSampleLevel(t_lightmap, s_env, lmuv, 0.0).rgb;
    if (hideout.lightmap.y < 0.5) {
        return dc / 3.14159265;
    }
    let n = to_source(n_world);
    var band = (textureSampleLevel(t_lm_direction, s_env, lmuv, 0.0).rgb * 2.0 - 1.0) * sqrt(3.0);
    band = vec3<f32>(-band.z, -band.x, band.y);
    let lin_r = band * dc.r;
    let lin_g = band * dc.g;
    let lin_b = band * dc.b;
    let axis = normalize(lin_r * 0.2125 + lin_g * 0.7154 + lin_b * 0.0721 + vec3<f32>(1e-7));
    let ratio = vec3<f32>(abs(dot(lin_r, axis)), abs(dot(lin_g, axis)), abs(dot(lin_b, axis))) / max(vec3<f32>(1e-5), dc);
    let zonal = dc * ratio * (0.08 + 0.6 * ratio) * 0.25 * sqrt(5.0 / (16.0 * 3.14159265));
    let linear = (0.886227 * dc + 1.023328 * vec3<f32>(dot(lin_r, n), dot(lin_g, n), dot(lin_b, n))) / 3.14159265;
    let a = dot(axis, n);
    return max(vec3<f32>(0.0), linear + zonal * (3.0 * a * a - 1.0));
}

// The stationary light's visibility from the baked shadow mask: 1 lit.
fn lightmap_visibility(uv: vec2<f32>, channel: f32) -> f32 {
    if (hideout.lightmap.z < 0.5 || channel < 0.0) {
        return 1.0;
    }
    let s = textureSampleLevel(t_lm_shadows, s_env, uv * hideout.lightmap_scale.xy, 0.0);
    var mask = vec4<f32>(0.0);
    mask[u32(channel) & 3u] = 1.0;
    return clamp(1.0 - dot(s, mask), 0.0, 1.0);
}

struct ProbeSample {
    // glTF axis order +X, -X, +Y, -Y, +Z, -Z.
    faces: array<vec3<f32>, 6>,
    shadow: vec4<f32>,
    coverage: f32,
};

fn decode_probe(c: vec4<f32>, rgbm: bool) -> vec3<f32> {
    if (!rgbm) {
        return c.rgb;
    }
    let enc = select(1.055 * pow(c.rgb, vec3<f32>(1.0 / 2.4)) - 0.055, c.rgb * 12.92, c.rgb <= vec3<f32>(0.0031308));
    let v = enc * c.a * 6.0;
    return v * v;
}

// The light probe volumes at `world` (source2ProbeVolume.ts): priority order, each fading at
// its edges, until the point is covered.
fn probe_at(world: vec3<f32>) -> ProbeSample {
    var out: ProbeSample;
    let count = u32(hideout.counts.x);
    let atlas = hideout.atlas.xyz;
    // Source faces +X, +Y, +Z, -X, -Y, -Z land on glTF +Z, +X, +Y, -Z, -X, -Y.
    var to_gltf = array<u32, 6>(4u, 0u, 2u, 5u, 1u, 3u);
    for (var i = 0u; i < 8u; i = i + 1u) {
        if (i >= count || out.coverage > 0.99) {
            break;
        }
        let p = hideout.probes[i];
        let local = (p.world_to_local * vec4<f32>(world, 1.0)).xyz;
        if (any(local < vec3<f32>(0.0)) || any(local > vec3<f32>(1.0))) {
            continue;
        }
        var fade = vec3<f32>(1.0);
        if (p.fade.x > 0.0 || p.fade.y > 0.0 || p.fade.z > 0.0) {
            fade = clamp(min(local, 1.0 - local) * select(vec3<f32>(1e6), p.fade.xyz, p.fade.xyz > vec3<f32>(0.0)), vec3<f32>(0.0), vec3<f32>(1.0));
        }
        let w = smoothstep(0.0, 1.0, min(fade.x, min(fade.y, fade.z))) * (1.0 - out.coverage);
        let size = p.size.xyz;
        let voxel = clamp(local * size, vec3<f32>(0.5), size - vec3<f32>(0.5)) + p.offset.xyz;
        var coord = voxel / atlas;
        coord.z = coord.z / 6.0;
        for (var f = 0u; f < 6u; f = f + 1u) {
            let c = textureSampleLevel(t_probes, s_env, coord + vec3<f32>(0.0, 0.0, f32(f) / 6.0), 0.0);
            let g = to_gltf[f];
            out.faces[g] = out.faces[g] + decode_probe(c, p.size.w > 0.5) * w;
        }
        if (hideout.counts.y > 0.5) {
            let sc = (clamp(local, vec3<f32>(0.0), vec3<f32>(1.0)) * size + p.offset.xyz) / vec3<f32>(atlas.xy, atlas.z * 6.0);
            out.shadow = out.shadow + textureSampleLevel(t_probe_shadows, s_env, sc, 0.0) * w;
        }
        out.coverage = out.coverage + w;
    }
    return out;
}

fn probe_cube(faces: array<vec3<f32>, 6>, d: vec3<f32>) -> vec3<f32> {
    let w = d * d;
    return w.x * select(faces[1], faces[0], d.x >= 0.0) + w.y * select(faces[3], faces[2], d.y >= 0.0) + w.z * select(faces[5], faces[4], d.z >= 0.0);
}

