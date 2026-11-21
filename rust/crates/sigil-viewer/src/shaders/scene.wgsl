// Sigil's hero and Hideout shader. The Citadel NPR terms are ported from Grimoire's viewer
// (MIT, src/lib/citadelNprLighting.ts and the citadel branch of source2NprMaterial.ts), which
// follows Deadlock's pbr.vfx forward pass with the scene NPR settings of
// materials/dev/npr_dummy.vmat as ValveResourceFormat's contributors reverse-engineered them.
//
// Units: light colors carry no 1/PI (the game's convention), so a light of irradiance E
// shades as E / PI. Output is linear HDR; post.wgsl exposes and tonemaps it.

@group(1) @binding(0) var<uniform> mat: Material;
@group(1) @binding(1) var t_base: texture_2d<f32>;
@group(1) @binding(2) var t_normal: texture_2d<f32>;
@group(1) @binding(3) var t_mr: texture_2d<f32>;
@group(1) @binding(4) var t_occlusion: texture_2d<f32>;
@group(1) @binding(5) var t_emissive: texture_2d<f32>;
@group(1) @binding(6) var t_tint: texture_2d<f32>;
@group(1) @binding(7) var t_outline: texture_2d<f32>;
@group(1) @binding(8) var s_material: sampler;

@group(2) @binding(0) var<uniform> draw: Draw;
@group(2) @binding(1) var<storage, read> palette: array<mat4x4<f32>>;

const PI: f32 = 3.14159265;
const STEP_SHARPNESS: f32 = 0.9;
const WRAP: f32 = 0.8;
const PBR_BLEND: f32 = 0.25;
const TARGETS: vec3<f32> = vec3<f32>(1.0, 0.5, 0.1);
const CAMERA_WEIGHT: f32 = 0.35;
const RIM_WRAP: f32 = 0.7;
const RIM_FALLOFF: f32 = 0.7;
const RIM_STRENGTH: f32 = 2.0;
const RIM_UP: f32 = 0.2;
const LUMA: vec3<f32> = vec3<f32>(0.2125, 0.7154, 0.0721);
const RIM_OCCLUSION: vec3<f32> = vec3<f32>(0.5, 2.5, 0.5);
// Lambert's sphere integral over the wrapped stepped response's (directLightNormalization).
const OO_NORMALIZATION: f32 = 0.625;
const OUTLINE_COLOR: vec4<f32> = vec4<f32>(0.039216, 0.003922, 0.003922, 0.74902);
const OUTLINE_LINEAR: vec3<f32> = vec3<f32>(0.003035, 0.000304, 0.000304);

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tangent: vec4<f32>,
    @location(3) uv0: vec2<f32>,
    @location(4) uv1: vec2<f32>,
    @location(5) color: vec4<f32>,
    @location(6) joints: vec4<u32>,
    @location(7) weights: vec4<f32>,
    @location(8) uv3: vec2<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tangent: vec4<f32>,
    @location(3) uv0: vec2<f32>,
    @location(4) uv1: vec2<f32>,
    @location(5) color: vec4<f32>,
    @location(6) uv3: vec2<f32>,
};

fn skin_matrix(joints: vec4<u32>, weights: vec4<f32>) -> mat4x4<f32> {
    let b = draw.palette_base.x;
    return palette[b + joints.x] * weights.x + palette[b + joints.y] * weights.y
        + palette[b + joints.z] * weights.z + palette[b + joints.w] * weights.w;
}

fn skin(v: VsIn) -> VsOut {
    let m = skin_matrix(v.joints, v.weights);
    var out: VsOut;
    let world = m * vec4<f32>(v.position, 1.0);
    let n3 = mat3x3<f32>(m[0].xyz, m[1].xyz, m[2].xyz);
    out.world = world.xyz;
    out.normal = normalize(n3 * v.normal);
    out.tangent = vec4<f32>(normalize(n3 * v.tangent.xyz), v.tangent.w);
    out.uv0 = v.uv0;
    out.uv1 = v.uv1;
    out.uv3 = v.uv3;
    out.color = v.color;
    out.clip = frame.view_proj * vec4<f32>(out.world, 1.0);
    return out;
}

@vertex
fn vs_main(v: VsIn) -> VsOut {
    return skin(v);
}

// Inverted hull: the posed surface grown along its world normal by the outline mask's red
// times the thickness in game units (pbr's NprLitOutline; no pixel-width term).
@vertex
fn vs_outline(v: VsIn) -> VsOut {
    var out = skin(v);
    let mask = textureSampleLevel(t_outline, s_material, v.uv0, 0.0).r;
    let grow = mask * mat.illum_tint.w * frame.env_bottom.w;
    out.world = out.world + out.normal * grow;
    out.clip = frame.view_proj * vec4<f32>(out.world, 1.0);
    out.color = vec4<f32>(mask);
    return out;
}

@vertex
fn vs_depth(v: VsIn) -> @builtin(position) vec4<f32> {
    let m = skin_matrix(v.joints, v.weights);
    return frame.view_proj * (m * vec4<f32>(v.position, 1.0));
}

fn power_step(x: f32, sharpness: f32) -> f32 {
    let k = 1.0 / (1.0 - sharpness);
    let hi = x > 0.5;
    let m = select(x, 1.0 - x, hi);
    let edge = pow(0.5, 1.0 - k) * pow(max(m, 0.0), k);
    return select(edge, 1.0 - edge, hi);
}

fn direct_diffuse(n_dot_l: f32) -> f32 {
    let lambert = clamp(n_dot_l, 0.0, 1.0);
    let w = clamp(2.0 * (lambert + WRAP) - 1.5, 0.0, 1.0);
    return mix(OO_NORMALIZATION * power_step(w, STEP_SHARPNESS), lambert, PBR_BLEND);
}

fn direct_specular(n: vec3<f32>, l: vec3<f32>, v: vec3<f32>, roughness: f32, f0: vec3<f32>) -> vec3<f32> {
    let n_dot_l = clamp(dot(n, l), 0.0, 1.0);
    let n_dot_v = clamp(dot(n, v), 0.0, 1.0);
    let h = normalize(l + v);
    let n_dot_h = dot(h, n);
    let a = max(roughness * roughness, 0.002);
    let a2 = a * a;
    let d = (n_dot_h * a2 - n_dot_h) * n_dot_h + 1.0;
    let vis = 0.5 / max(1e-5, n_dot_l * (n_dot_v * (1.0 - a) + a) + n_dot_v * (n_dot_l * (1.0 - a) + a));
    let lobe = a2 / max(1e-8, d * d) * vis * n_dot_l;
    let fresnel = f0 + (1.0 - f0) * pow(max(0.0, 1.0 - clamp(dot(l, h), 0.0, 1.0)), 5.0);
    return fresnel * lobe * (1.0 + f0 * (2.0 * a2 * n_dot_v));
}

fn cube(d: vec3<f32>) -> vec3<f32> {
    let w = d * d;
    return w.x * select(frame.ambient[1].rgb, frame.ambient[0].rgb, d.x >= 0.0)
        + w.y * select(frame.ambient[3].rgb, frame.ambient[2].rgb, d.y >= 0.0)
        + w.z * select(frame.ambient[5].rgb, frame.ambient[4].rgb, d.z >= 0.0);
}

fn xy_y(c: vec3<f32>) -> vec3<f32> {
    let xyz = vec3<f32>(dot(c, vec3<f32>(0.4124, 0.3576, 0.1805)), dot(c, vec3<f32>(0.2126, 0.7152, 0.0722)), dot(c, vec3<f32>(0.0193, 0.1192, 0.9505)));
    let sum = xyz.x + xyz.y + xyz.z;
    if (sum > 0.0) {
        return vec3<f32>(xyz.xy / sum, xyz.y);
    }
    return vec3<f32>(0.0);
}

// The sample's hue (pulled toward the cube's average hue where it is darker than average)
// at the target luminance.
fn expose_ambient(sampled: vec3<f32>, average: vec3<f32>, target_y: f32) -> vec3<f32> {
    let s = xy_y(sampled);
    let xy = mix(average.xy, s.xy, clamp(s.z / max(average.z, 1e-8), 0.0, 1.0));
    if (xy.y <= 0.0 || target_y <= 0.0) {
        return vec3<f32>(0.0);
    }
    let xyz = vec3<f32>(xy.x, xy.y, 1.0 - xy.x - xy.y) * (target_y / xy.y);
    return vec3<f32>(dot(xyz, vec3<f32>(3.2406, -1.5372, -0.4986)), dot(xyz, vec3<f32>(-0.9689, 1.8758, 0.0415)), dot(xyz, vec3<f32>(0.0557, -0.2040, 1.0570)));
}

// NPR ambient: a stepped half-Lambert toward the primary light picks a direction between
// straight down, the horizontal direction to the camera and straight up; the cube is read
// there and forced to a fixed luminance. Returns rgb and the band (0.25 shadow .. 0.75 lit).
fn npr_ambient(n: vec3<f32>, to_camera: vec3<f32>, key: vec3<f32>, ao: f32, faces: array<vec3<f32>, 6>) -> vec4<f32> {
    let exposure = frame.key_dir.w;
    let primary = normalize(mix(key, frame.camera_back.xyz, CAMERA_WEIGHT));
    let h = 0.5 + 0.5 * dot(primary, n);
    let x = clamp(h * ao * 2.0 - 0.5, 0.0, 1.0);
    let fx = fract(x);
    let stepped = (x - fx) + power_step(fx, STEP_SHARPNESS);
    let band = (stepped + 0.5) * 0.5;
    var horizontal = vec3<f32>(to_camera.x, 0.0, to_camera.z);
    if (dot(horizontal, horizontal) > 1e-8) {
        horizontal = normalize(horizontal);
    } else {
        horizontal = vec3<f32>(0.0, 0.0, 1.0);
    }
    var dir: vec3<f32>;
    if (band < 0.5) {
        dir = mix(vec3<f32>(0.0, -1.0, 0.0), horizontal, stepped + 0.5);
    } else {
        dir = mix(horizontal, vec3<f32>(0.0, 1.0, 0.0), stepped - 0.5);
    }
    dir = normalize(dir);
    let average = xy_y((faces[0] + faces[1] + faces[2] + faces[3] + faces[4] + faces[5]) / 6.0);
    let d2 = dir * dir;
    let target_y = dot(vec3<f32>(TARGETS.y, select(TARGETS.z, TARGETS.x, dir.y > 0.0), TARGETS.y), d2) / exposure;
    return vec4<f32>(expose_ambient(probe_cube(faces, dir), average, target_y), band);
}

// The ambient cube at a point: the Hideout's probe volumes where they cover it, blended into
// the frame's cube (the scene's probe or the studio) for the rest.
fn faces_at(world: vec3<f32>) -> ProbeSample {
    var p = probe_at(world);
    let rest = clamp(1.0 - p.coverage, 0.0, 1.0);
    // Unrolled: FXC cannot write `p.faces` through a dynamic index.
    p.faces[0] = p.faces[0] + frame.ambient[0].rgb * rest;
    p.faces[1] = p.faces[1] + frame.ambient[1].rgb * rest;
    p.faces[2] = p.faces[2] + frame.ambient[2].rgb * rest;
    p.faces[3] = p.faces[3] + frame.ambient[3].rgb * rest;
    p.faces[4] = p.faces[4] + frame.ambient[4].rgb * rest;
    p.faces[5] = p.faces[5] + frame.ambient[5].rgb * rest;
    return p;
}

fn faces_fit(faces: array<vec3<f32>, 6>) -> vec4<f32> {
    let l = array<f32, 6>(dot(faces[0], LUMA), dot(faces[1], LUMA), dot(faces[2], LUMA), dot(faces[3], LUMA), dot(faces[4], LUMA), dot(faces[5], LUMA));
    return vec4<f32>((l[0] - l[1]) * 0.5, (l[2] - l[3]) * 0.5, (l[4] - l[5]) * 0.5, (l[0] + l[1] + l[2] + l[3] + l[4] + l[5]) / 6.0);
}

// A stationary light's baked visibility: the lightmap's shadow mask on lightmapped surfaces,
// the probe shadow volume elsewhere, squared for point and spot lights (Grimoire).
fn baked_visibility(i: u32, s: Surface, probe: ProbeSample) -> f32 {
    let channel = hideout.lights[i].b.y;
    if (channel < 0.0) {
        return 1.0;
    }
    var v = 1.0;
    if (lightmapped(s)) {
        v = lightmap_visibility(s.uv3, channel);
    } else if (hideout.counts.y > 0.5 && probe.coverage > 0.0) {
        v = clamp(1.0 - dot(probe.shadow, channel_mask(channel)), 0.0, 1.0);
    }
    if (frame.lights[i].pos_dir.w > 0.5) {
        v = v * v;
    }
    return v;
}

fn lightmapped(s: Surface) -> bool {
    return hideout.lightmap.x > 0.5 && s.uv3.x > -4.0;
}

fn scene_depth(px: vec2<i32>) -> f32 {
    let size = vec2<i32>(textureDimensions(rim_depth));
    let p = clamp(px, vec2<i32>(0), size - vec2<i32>(1));
    let d = textureLoad(rim_depth, p, 0);
    let near = frame.camera_pos.w;
    let far = frame.camera_back.w;
    return (near * far) / (far - (far - near) * d);
}

// 1 where the hero depth a little up and out from the surface is clearly farther away (a
// silhouette), 0 where more of the hero is there.
fn rim_occlusion(world: vec3<f32>, n: vec3<f32>, frag: vec2<f32>) -> f32 {
    if (frame.viewport.z < 0.5) {
        return 1.0;
    }
    let unit = frame.env_bottom.w;
    let offset = normalize(mix(n, vec3<f32>(0.0, 1.0, 0.0), 0.5)) * (RIM_OCCLUSION.x * unit);
    let clip = frame.view_proj * vec4<f32>(world + offset, 1.0);
    let ndc = clip.xy / clip.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    let there = scene_depth(vec2<i32>(uv * frame.viewport.xy));
    let here = scene_depth(vec2<i32>(frag));
    return smoothstep(0.0, 1.0, clamp((there - (RIM_OCCLUSION.y * unit + here)) / (RIM_OCCLUSION.z * unit), 0.0, 1.0));
}

fn npr_rim(n: vec3<f32>, to_camera: vec3<f32>, ao: f32, mask: f32) -> f32 {
    let width = 1.0 + RIM_WRAP;
    let grazing = pow(clamp((RIM_WRAP - dot(n, to_camera)) / (width * width), 0.0, 1.0), RIM_FALLOFF);
    return grazing * step(RIM_UP, n.y) * RIM_STRENGTH * ao * mask;
}

fn multi_bounce(ao: f32, albedo: vec3<f32>) -> vec3<f32> {
    let a = 2.0404 * albedo - 0.3324;
    let b = -4.7951 * albedo + 0.6417;
    let c = 2.7552 * albedo + 0.6903;
    return ((ao * a + b) * ao + c) * ao;
}

fn env_brdf(roughness: f32, n_dot_v: f32) -> vec2<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = roughness * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * n_dot_v)) * r.x + r.y;
    return vec2<f32>(-1.04, 1.04) * a004 + r.zw;
}

// The studio the preview reflects: a soft gradient room with a broad key softbox and a
// cooler strip opposite, blurred analytically by roughness.
fn environment(dir: vec3<f32>, roughness: f32) -> vec3<f32> {
    if (frame.counts.z > 0.5) {
        return textureSampleLevel(env_cube, s_env, dir, roughness * frame.counts.w).rgb * frame.env_scale.rgb;
    }
    let k = frame.env_top.w;
    let blur = roughness * roughness;
    let t = smoothstep(-0.35 - blur, 0.55 + blur, dir.y);
    var c = mix(frame.env_bottom.rgb, frame.env_top.rgb, t);
    let key = normalize(frame.key_dir.xyz);
    let lobe = mix(48.0, 2.0, clamp(blur * 1.5, 0.0, 1.0));
    let peak = mix(5.0, 0.6, clamp(blur * 1.5, 0.0, 1.0));
    c = c + vec3<f32>(1.0, 0.97, 0.92) * pow(max(dot(dir, key), 0.0), lobe) * peak;
    let strip = normalize(vec3<f32>(-key.x, 0.25, -key.z));
    c = c + vec3<f32>(0.62, 0.74, 1.0) * pow(max(dot(dir, strip), 0.0), lobe * 0.7) * peak * 0.45;
    return c * k;
}

fn light_at(i: u32, world: vec3<f32>) -> vec4<f32> {
    // xyz: direction to the light, w: attenuation.
    let l = frame.lights[i];
    let ex = hideout.lights[i];
    let source = ex.a.x > 0.5;
    if (l.pos_dir.w < 0.5) {
        // Source units carry no 1/PI; the shading divides by PI, so put it back.
        return vec4<f32>(normalize(l.pos_dir.xyz), select(1.0, 3.14159265, source));
    }
    let d = l.pos_dir.xyz - world;
    let dist2 = max(dot(d, d), 1e-4);
    let dist = sqrt(dist2);
    let to_light = d / dist;
    var att: f32;
    if (source) {
        // 10000 / d^2 in inches is 6.4516 / d^2 in meters (source2DynamicLights.ts), with the
        // skirt fading the last part of the range.
        att = 6.4516 / max(dist2, 0.00064516) * 3.14159265;
        if (l.color.w > 0.0) {
            att = att * smoothstep(0.0, 1.0, clamp((l.color.w - dist) / (l.color.w * ex.a.y), 0.0, 1.0));
        }
    } else {
        att = 1.0 / dist2;
        if (l.color.w > 0.0) {
            let r = clamp(1.0 - pow(max(dist / l.color.w, 0.0), 4.0), 0.0, 1.0);
            att = att * r * r;
        }
    }
    if (l.pos_dir.w > 1.5) {
        let c = dot(-to_light, normalize(l.spot.xyz));
        att = att * smoothstep(l.spot.w, max(l.spot_inner.x, l.spot.w + 1e-4), c);
    }
    if (ex.a.z >= 0.0) {
        att = att * cookie(i, -to_light);
    }
    return vec4<f32>(to_light, att);
}

// A light's cookie along `ray` (from the light toward the point): a barn frames it in its
// cone, an omni wraps it around (source2DynamicLights.ts).
fn cookie(i: u32, ray: vec3<f32>) -> f32 {
    let ex = hideout.lights[i];
    let l = frame.lights[i];
    let local = vec3<f32>(dot(ray, ex.x.xyz), dot(ray, ex.y.xyz), dot(ray, ex.z.xyz));
    var uv: vec2<f32>;
    if (l.pos_dir.w > 1.5) {
        if (local.x <= 1e-4) {
            return 0.0;
        }
        let tan_outer = tan(acos(clamp(l.spot.w, -0.999, 0.999)));
        let ndc = vec2<f32>(local.y, local.z) / (local.x * max(tan_outer, 1e-3));
        let half_texel = 0.5 / 256.0;
        uv = clamp(ndc * -0.5 + 0.5, vec2<f32>(half_texel), vec2<f32>(1.0 - half_texel));
    } else {
        let r = normalize(local);
        uv = vec2<f32>(fract(atan2(r.y, -r.x) / 6.2831853), acos(clamp(r.z, -1.0, 1.0)) / 3.14159265);
    }
    let layers = max(hideout.counts.z, 1.0);
    let c = textureSampleLevel(t_cookies, s_env, vec2<f32>((ex.a.z + uv.x) / layers, uv.y), 0.0).rgb;
    return dot(c, vec3<f32>(0.2125, 0.7154, 0.0721));
}

fn decode_normal(tex: vec4<f32>, hemi_oct: bool) -> vec3<f32> {
    if (hemi_oct) {
        let x = tex.r + tex.g - 1.003922;
        let y = tex.r - tex.g;
        let n = normalize(vec3<f32>(x, y, 1.0 - abs(x) - abs(y)));
        return vec3<f32>(n.x, -n.y, n.z);
    }
    return tex.rgb * 2.0 - 1.0;
}

fn color_correct(c: vec3<f32>) -> vec3<f32> {
    let csb = mat.csb.xyz;
    let pivot = mat.reflectivity.rgb;
    let r = ((c - pivot) * csb.x + pivot) * csb.z;
    let w = vec3<f32>(0.0804, 0.9103, 0.0093); // squared, normalized Rec. 709
    let gray = dot(r, w);
    return r * csb.y + gray * (1.0 - csb.y);
}

// Grimoire's volumetric fog (source2Atmosphere.ts), marched along the view ray to the
// surface: density from the fog volumes, light from the irradiance volume and the fog lights,
// in-scattered and extinguished at 0.01 per inch (0.3937 per meter).
fn fog_density(p: vec3<f32>) -> vec4<f32> {
    // xyz unused here; returns density, indirect strength, tint index (as float), 0.
    var best = 0.0;
    var indirect = hideout.fog.y;
    var tint_from = -1.0;
    let n = u32(hideout.fog_fade.z);
    for (var i = 0u; i < 16u; i = i + 1u) {
        if (i >= n) {
            break;
        }
        let v = hideout.fog_volumes[i];
        let q = (v.world_to_local * vec4<f32>(p, 1.0)).xyz * 2.0 - 1.0;
        var edge: f32;
        if (v.params.x > 0.5) {
            edge = 1.0 - length(q);
        } else {
            edge = 1.0 - max(abs(q.x), max(abs(q.y), abs(q.z)));
        }
        if (edge <= 0.0) {
            continue;
        }
        let fall = pow(clamp(edge, 0.0, 1.0), v.params.z);
        let d = clamp(fall * v.params.y, 0.0, 1.0);
        if (d > best) {
            best = d;
            if (v.params.w >= 0.0) {
                indirect = mix(hideout.fog.y, v.params.w, fall);
            }
            if (v.tint.w > 0.5) {
                tint_from = f32(i);
            }
        }
    }
    return vec4<f32>(best, indirect, tint_from, 0.0);
}

fn volumetric_fog(c: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    let eye = frame.camera_pos.xyz;
    let ray = world - eye;
    let full = length(ray);
    let len = min(full, max(hideout.fog.w, 0.1));
    if (len <= 1e-3) {
        return c;
    }
    let dir = ray / full;
    let steps = max(hideout.fog_fade.w, 4.0);
    let stride = len / steps;
    var optical = 0.0;
    var scatter = vec3<f32>(0.0);
    let count = u32(frame.counts.x);
    for (var k = 0.0; k < steps; k = k + 1.0) {
        let t = (k + 0.5) * stride;
        let p = eye + dir * t;
        let d = fog_density(p);
        if (d.x <= 0.0) {
            continue;
        }
        var rgb = vec3<f32>(0.0);
        if (hideout.fog.x > 0.5) {
            let q = (hideout.fog_irradiance * vec4<f32>(p, 1.0)).xyz;
            rgb = textureSampleLevel(t_fog, s_env, clamp(q, vec3<f32>(0.0), vec3<f32>(1.0)), 0.0).rgb * d.y;
        }
        for (var i = 0u; i < 8u; i = i + 1u) {
            if (i >= count) {
                break;
            }
            let strength = hideout.lights[i].a.w;
            if (strength <= 0.0) {
                continue;
            }
            let l = light_at(i, p);
            rgb = rgb + frame.lights[i].color.rgb * l.w / PI * strength;
        }
        if (d.z >= 0.0) {
            rgb = rgb * hideout.fog_volumes[u32(d.z)].tint.rgb;
        }
        let fade = clamp((t - hideout.fog_fade.x) / max(hideout.fog_fade.y - hideout.fog_fade.x, 1e-3), 0.0, 1.0);
        let ext = d.x * 0.3937 * hideout.fog.z * stride * fade;
        scatter = scatter + rgb * ext * exp(-optical);
        optical = optical + ext;
    }
    return c * exp(-optical) + scatter;
}

fn apply_fog(c: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    if (hideout.fog_fade.z > 0.5) {
        return volumetric_fog(c, world);
    }
    if (frame.fog_color.w < 0.5) {
        return c;
    }
    let dist = length(world - frame.camera_pos.xyz);
    var f = clamp((dist - frame.fog_params.x) / max(frame.fog_params.y - frame.fog_params.x, 1e-3), 0.0, 1.0);
    if (frame.fog_params.z > 0.0) {
        f = max(f, 1.0 - exp(-frame.fog_params.z * dist));
    }
    return mix(c, frame.fog_color.rgb, f * frame.fog_params.w);
}

struct Surface {
    base: vec4<f32>,
    n: vec3<f32>,
    metal: f32,
    rough: f32,
    ao: f32,
    rim_mask: f32,
    emissive: vec3<f32>,
    si_mask: f32,
    uv3: vec2<f32>,
};

fn surface(in: VsOut, front: bool) -> Surface {
    let base_t = textureSample(t_base, s_material, in.uv0);
    let normal_t = textureSample(t_normal, s_material, in.uv0);
    let mr_t = textureSample(t_mr, s_material, in.uv0);
    let occ_t = textureSample(t_occlusion, s_material, in.uv0);
    let emis_t = textureSample(t_emissive, s_material, in.uv0);
    let tint_t = textureSample(t_tint, s_material, in.uv0);
    var s: Surface;
    var base = mat.base_factor;
    if (mat.has.x > 0.5) {
        base = base * base_t;
    }
    if (mat.extra.y > 0.5) {
        base = base * in.color;
    }
    base = base * draw.tint;
    if (mat.csb.w > 0.5) {
        base = vec4<f32>(color_correct(base.rgb), base.a);
    }
    let has_tint = mat.tint_offset.w > 0.5;
    let tint_amount = select(1.0, tint_t.r, has_tint && mat.tint_r0.w > 0.5);
    let tinted = vec3<f32>(dot(mat.tint_r0.xyz, base.rgb), dot(mat.tint_r1.xyz, base.rgb), dot(mat.tint_r2.xyz, base.rgb)) + mat.tint_offset.rgb;
    base = vec4<f32>(mix(base.rgb, tinted, tint_amount), base.a);
    s.base = base;
    s.rim_mask = select(1.0, tint_t.g, has_tint);

    var n = normalize(in.normal);
    if (!front) {
        n = -n;
    }
    if (mat.has.y > 0.5) {
        let tn = decode_normal(normal_t, mat.flags.y > 0.5);
        let t = normalize(in.tangent.xyz - n * dot(n, in.tangent.xyz));
        let b = cross(n, t) * select(-1.0, 1.0, in.tangent.w >= 0.0);
        let scaled = vec3<f32>(tn.xy * mat.pbr.w, tn.z);
        let mapped = normalize(t * scaled.x + b * scaled.y + n * scaled.z);
        if (all(mapped == mapped)) {
            n = mapped;
        }
    }
    s.n = n;
    s.metal = mat.pbr.x;
    s.rough = mat.pbr.y;
    if (mat.has.z > 0.5) {
        s.metal = s.metal * mr_t.b;
        s.rough = s.rough * mr_t.g;
    }
    s.rough = clamp(s.rough, 0.03, 1.0);
    s.metal = clamp(s.metal, 0.0, 1.0);
    s.ao = 1.0;
    if (mat.has.w > 0.5) {
        s.ao = clamp(1.0 + mat.pbr.z * (occ_t.r - 1.0), 0.0, 1.0);
    }
    s.emissive = vec3<f32>(0.0);
    if (mat.illum.x > 0.5) {
        s.emissive = emis_t.rgb * mat.emissive.rgb * mat.emissive.w;
    } else {
        s.emissive = mat.emissive.rgb * mat.emissive.w;
    }
    s.si_mask = select(1.0, emis_t.r, mat.illum.x > 0.5);
    s.uv3 = in.uv3;
    return s;
}

fn shade_npr(s: Surface, world: vec3<f32>, frag: vec2<f32>) -> vec3<f32> {
    let v = normalize(frame.camera_pos.xyz - world);
    let n = s.n;
    var base = s.base.rgb;
    var si_scale = 0.0;
    var si_mask = 0.0;
    if (mat.illum.y > 0.5) {
        si_mask = s.si_mask;
        si_scale = mat.illum.z;
        let si_color = mix(mat.illum_tint.rgb, mat.illum_tint.rgb * base, mat.illum.w);
        base = mix(base, si_color, si_mask * clamp(si_scale, 0.0, 1.0));
    }
    let f0 = mix(vec3<f32>(0.04), base, s.metal) * clamp(max(max(base.r, base.g), base.b) * 25.0, 0.0, 1.0);
    var direct = vec3<f32>(0.0);
    var spec = vec3<f32>(0.0);
    let count = u32(frame.counts.x);
    let shadowed = u32(frame.shadow.y);
    let lit = shadow_at(world, n);
    let probe = faces_at(world);
    for (var i = 0u; i < 8u; i = i + 1u) {
        if (i >= count) {
            break;
        }
        if (hideout.lights[i].b.x > 0.5) {
            continue;
        }
        let l = light_at(i, world);
        let n_dot_l = dot(n, l.xyz);
        if (n_dot_l > 0.0) {
            let c = frame.lights[i].color.rgb * l.w * select(1.0, lit, i == shadowed) * baked_visibility(i, s, probe) / PI;
            direct = direct + c * direct_diffuse(n_dot_l);
            spec = spec + c * direct_specular(n, l.xyz, v, s.rough, f0);
        }
    }
    let amb = npr_ambient(n, v, normalize(frame.key_dir.xyz), s.ao, probe.faces);
    let bounce = mix(vec3<f32>(1.0), multi_bounce(s.ao, base), PBR_BLEND);
    let diffuse = (direct + amb.rgb * bounce) * base * (1.0 - s.metal);
    let rim = (direct + amb.rgb) * npr_rim(n, v, s.ao, s.rim_mask) * rim_occlusion(world, n, frag);
    let emission = base * (si_mask * si_scale);
    // Reflections, rescaled to the NPR ambient level and occluded at grazing roughness.
    let a = s.rough * s.rough;
    let bend = clamp(1.0 - a, 0.0, 1.0);
    let r = normalize(mix(n, reflect(-v, n), bend * (sqrt(bend) + a)));
    let env = environment(r, s.rough);
    let n_dot_v = clamp(dot(n, v), 0.0, 1.0);
    let fab = env_brdf(s.rough, n_dot_v);
    let fit = select(frame.ambient_fit, faces_fit(probe.faces), probe.coverage > 0.0);
    let probe_l = dot(vec4<f32>(n, 1.0), fit);
    let ratio = select(1.0, min(dot(amb.rgb, LUMA) / probe_l, 32.0), probe_l > 0.0);
    let spec_occ = clamp(pow(max(n_dot_v + s.ao, 0.0), a) - 1.0 + s.ao, 0.0, 1.0);
    let ibl = mix(vec3<f32>(fab.y), vec3<f32>(fab.x + fab.y), f0) * env * ratio * spec_occ;
    return diffuse + rim + emission + spec + ibl;
}

fn shade_pbr(s: Surface, world: vec3<f32>) -> vec3<f32> {
    let v = normalize(frame.camera_pos.xyz - world);
    let n = s.n;
    let base = s.base.rgb;
    let f0 = mix(vec3<f32>(0.04), base, s.metal);
    var direct = vec3<f32>(0.0);
    var spec = vec3<f32>(0.0);
    let count = u32(frame.counts.x);
    let shadowed = u32(frame.shadow.y);
    let lit = shadow_at(world, n);
    let baked = lightmapped(s);
    var probe: ProbeSample;
    if (!baked) {
        probe = faces_at(world);
    }
    for (var i = 0u; i < 8u; i = i + 1u) {
        if (i >= count) {
            break;
        }
        if (hideout.lights[i].b.x > 0.5) {
            continue;
        }
        let l = light_at(i, world);
        let n_dot_l = dot(n, l.xyz);
        if (n_dot_l > 0.0) {
            let c = frame.lights[i].color.rgb * l.w * select(1.0, lit, i == shadowed) * baked_visibility(i, s, probe);
            direct = direct + c * n_dot_l / PI;
            spec = spec + c / PI * direct_specular(n, l.xyz, v, s.rough, f0);
        }
    }
    // Diffuse ambient: the SH2 lightmap where the surface is baked, else the probe volume
    // (or the scene's cube). Both are irradiance over PI already, as the albedo wants it.
    var irradiance: vec3<f32>;
    if (baked) {
        irradiance = lightmap_irradiance(n, s.uv3);
    } else {
        irradiance = probe_cube(probe.faces, n);
    }
    let n_dot_v = clamp(dot(n, v), 0.0, 1.0);
    let fab = env_brdf(s.rough, n_dot_v);
    var env = environment(reflect(-v, n), s.rough);
    if (baked) {
        // Rough baked surfaces reflect the lightmap rather than the far probe.
        let a = s.rough * s.rough;
        let k = 1.0 - a;
        let dom = normalize(mix(n, reflect(-v, n), k * (sqrt(k) + a)));
        let w = clamp(-2.0 + 10.0 * s.rough, 0.0, 1.0) * (1.0 - s.metal);
        env = mix(env, lightmap_irradiance(dom, s.uv3), w);
    }
    let ibl = mix(vec3<f32>(fab.y), vec3<f32>(fab.x + fab.y), f0) * env * s.ao;
    return (direct + irradiance * s.ao) * base * (1.0 - s.metal) + spec + ibl + s.emissive;
}

@fragment
fn fs_main(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let s = surface(in, front);
    let mode = mat.flags.w;
    if (mode > 0.5 && mode < 1.5 && s.base.a < mat.reflectivity.w) {
        discard;
    }
    var color: vec3<f32>;
    if (mat.flags.z > 0.5) {
        color = s.base.rgb + s.emissive;
    } else if (mat.flags.x > 0.5 && frame.counts.y > 0.5) {
        color = shade_npr(s, in.world, in.clip.xy);
    } else {
        color = shade_pbr(s, in.world);
    }
    color = apply_fog(color, in.world);
    var alpha = 1.0;
    if (mode > 1.5) {
        alpha = s.base.a * (1.0 - mat.extra.z * 0.5);
    }
    if (mode > 2.5) {
        // Additive: premultiplied with zero alpha, so it adds light and covers nothing.
        return vec4<f32>(color * s.base.a, 0.0);
    }
    return vec4<f32>(color * alpha, alpha);
}

// The lit hull: the surface's albedo pulled most of the way to the near-black NPR outline
// colour (sRGB in npr_dummy.vmat, decoded here), lit as a surface facing the camera, no rim.
@fragment
fn fs_outline(in: VsOut) -> @location(0) vec4<f32> {
    var s = surface(in, true);
    if (in.color.r <= 0.0) {
        discard;
    }
    s.base = vec4<f32>(mix(s.base.rgb, OUTLINE_LINEAR, OUTLINE_COLOR.a), 1.0);
    s.n = frame.camera_back.xyz;
    s.rim_mask = 0.0;
    var c: vec3<f32>;
    if (frame.counts.y > 0.5) {
        c = shade_npr(s, in.world, in.clip.xy);
    } else {
        c = s.base.rgb * 0.2;
    }
    return vec4<f32>(apply_fog(c, in.world), 1.0);
}

