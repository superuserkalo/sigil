// Linear HDR to the display: exposure, Source 2's filmic curve (VRF's TonemapColor with the
// 2.8 pre-scale, as Grimoire's sourcePostProcess.ts ports it, MIT), the brightness gamma, then
// the backdrop under the uncovered pixels. The target is sRGB, so the hardware encodes.

struct Post {
    // exposure (scalar times 2^bias), white point (linear), 1 / curve(white), gamma / 2.2
    exposure: vec4<f32>,
    // shoulder, linear strength, linear angle, toe strength
    curve_a: vec4<f32>,
    // toe numerator, toe denominator, bloom strength, bloom threshold
    curve_b: vec4<f32>,
    // split x in pixels (< 0: off), line half width, width, height
    split: vec4<f32>,
    top: vec4<f32>,      // backdrop top (linear), transparent backdrop
    bottom: vec4<f32>,   // backdrop bottom (linear)
    glow: vec4<f32>,     // glow colour (linear), glow strength
    glow_at: vec4<f32>,  // glow centre (0..1), radius x, radius y
    inv_view_proj: mat4x4<f32>,
    sky: vec4<f32>,      // linear scale (rgb), sky cube on
};

@group(0) @binding(0) var<uniform> post: Post;
@group(0) @binding(1) var t_after: texture_2d<f32>;
@group(0) @binding(2) var t_before: texture_2d<f32>;
@group(0) @binding(3) var t_bloom: texture_2d<f32>;
@group(0) @binding(4) var s_linear: sampler;
@group(0) @binding(5) var t_sky: texture_cube<f32>;

struct FsIn {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> FsIn {
    var out: FsIn;
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    out.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);
    return out;
}

fn curve(x: vec3<f32>) -> vec3<f32> {
    let a = post.curve_a.x;
    let b = post.curve_a.y;
    let c = post.curve_a.z;
    let d = post.curve_a.w;
    let e = post.curve_b.x;
    let f = post.curve_b.y;
    return (x * (a * x + c * b) + d * e) / (x * (a * x + b) + d * f) - e / f;
}

// `x` is already exposed; the ADD bloom joins after the exposure, as VRF's post does.
fn tonemap(x: vec3<f32>) -> vec3<f32> {
    let exposed = min(x * 2.8, vec3<f32>(post.exposure.y));
    let mapped = max(curve(exposed) * post.exposure.z, vec3<f32>(0.0));
    // The game's brightness setting acts on the encoded value; 2.2 is identity.
    if (abs(post.exposure.w - 1.0) > 1e-4) {
        let enc = pow(mapped, vec3<f32>(1.0 / 2.2));
        return pow(pow(enc, vec3<f32>(post.exposure.w)), vec3<f32>(2.2));
    }
    return mapped;
}

fn backdrop(uv: vec2<f32>) -> vec3<f32> {
    if (post.sky.w > 0.5) {
        // The scene's sky along this pixel's view ray, exposed and tonemapped like the scene.
        let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
        let far = post.inv_view_proj * vec4<f32>(ndc, 1.0, 1.0);
        let near = post.inv_view_proj * vec4<f32>(ndc, 0.0, 1.0);
        let dir = normalize(far.xyz / far.w - near.xyz / near.w);
        let sky = textureSampleLevel(t_sky, s_linear, dir, 0.0).rgb * post.sky.rgb;
        return tonemap(sky * post.exposure.x);
    }
    var c = mix(post.top.rgb, post.bottom.rgb, smoothstep(0.0, 1.0, uv.y));
    let d = (uv - post.glow_at.xy) / max(post.glow_at.zw, vec2<f32>(1e-3));
    c = c + post.glow.rgb * post.glow.w * exp(-dot(d, d) * 1.6);
    // A faint vignette keeps the eye on the hero.
    let v = uv - vec2<f32>(0.5);
    c = c * (1.0 - 0.35 * dot(v, v));
    return c;
}

@fragment
fn fs_post(in: FsIn) -> @location(0) vec4<f32> {
    let px = vec2<i32>(in.pos.xy);
    var hdr = textureLoad(t_after, px, 0);
    let split = post.split.x;
    if (split >= 0.0 && in.pos.x < split) {
        hdr = textureLoad(t_before, px, 0);
    }
    let bloom = textureSample(t_bloom, s_linear, in.uv).rgb * post.curve_b.z;
    let a = clamp(hdr.a, 0.0, 1.0);
    var scene = vec3<f32>(0.0);
    if (a > 1e-4) {
        scene = tonemap(hdr.rgb / a * post.exposure.x + bloom);
    }
    let glow = tonemap(bloom) * (1.0 - a);
    var out: vec3<f32>;
    var alpha = 1.0;
    if (post.top.w > 0.5) {
        out = scene * a + glow;
        alpha = max(a, max(glow.r, max(glow.g, glow.b)));
        if (alpha > 1e-4) {
            out = out / alpha;
        }
    } else {
        out = mix(backdrop(in.uv), scene, a) + glow;
    }
    if (split >= 0.0 && abs(in.pos.x - split) < post.split.y) {
        out = mix(out, vec3<f32>(0.85, 0.82, 0.78), 0.85);
        alpha = 1.0;
    }
    return vec4<f32>(out, alpha);
}

// Bloom: bright parts of the unexposed scene above the threshold (Deadlock blooms the
// unexposed scene), then a downsample chain and a tent upsample back.
@fragment
fn fs_threshold(in: FsIn) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(t_after));
    let texel = 1.0 / size;
    var c = vec3<f32>(0.0);
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let s = textureSampleLevel(t_after, s_linear, in.uv + vec2<f32>(f32(x), f32(y)) * texel * 1.5, 0.0);
            c = c + s.rgb;
        }
    }
    c = c / 9.0;
    let peak = max(c.r, max(c.g, c.b));
    let t = post.curve_b.w;
    let k = clamp((peak - t) / max(peak, 1e-4), 0.0, 1.0);
    return vec4<f32>(c * k, 1.0);
}

@fragment
fn fs_down(in: FsIn) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(t_after));
    let o = 1.0 / size;
    var c = textureSampleLevel(t_after, s_linear, in.uv, 0.0).rgb * 4.0;
    c = c + textureSampleLevel(t_after, s_linear, in.uv + vec2<f32>(o.x, o.y), 0.0).rgb;
    c = c + textureSampleLevel(t_after, s_linear, in.uv + vec2<f32>(-o.x, o.y), 0.0).rgb;
    c = c + textureSampleLevel(t_after, s_linear, in.uv + vec2<f32>(o.x, -o.y), 0.0).rgb;
    c = c + textureSampleLevel(t_after, s_linear, in.uv + vec2<f32>(-o.x, -o.y), 0.0).rgb;
    return vec4<f32>(c / 8.0, 1.0);
}

@fragment
fn fs_up(in: FsIn) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(t_after));
    let o = 1.0 / size;
    var c = textureSampleLevel(t_after, s_linear, in.uv, 0.0).rgb * 4.0;
    c = c + textureSampleLevel(t_after, s_linear, in.uv + vec2<f32>(o.x, 0.0), 0.0).rgb * 2.0;
    c = c + textureSampleLevel(t_after, s_linear, in.uv - vec2<f32>(o.x, 0.0), 0.0).rgb * 2.0;
    c = c + textureSampleLevel(t_after, s_linear, in.uv + vec2<f32>(0.0, o.y), 0.0).rgb * 2.0;
    c = c + textureSampleLevel(t_after, s_linear, in.uv - vec2<f32>(0.0, o.y), 0.0).rgb * 2.0;
    c = c + textureSampleLevel(t_after, s_linear, in.uv + o, 0.0).rgb;
    c = c + textureSampleLevel(t_after, s_linear, in.uv - o, 0.0).rgb;
    c = c + textureSampleLevel(t_after, s_linear, in.uv + vec2<f32>(o.x, -o.y), 0.0).rgb;
    c = c + textureSampleLevel(t_after, s_linear, in.uv + vec2<f32>(-o.x, o.y), 0.0).rgb;
    return vec4<f32>(c / 16.0, 1.0);
}
