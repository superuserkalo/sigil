// Spritecards (VRF's spritecard shader through Grimoire's port, MIT): one instanced quad per
// particle, the card basis, sheet frames and colour already worked out on the CPU. Drawn in
// the scene's HDR pass with its depth, no depth write, premultiplied, modulate-2x or lighten
// blending.

struct Light {
    pos_dir: vec4<f32>,
    color: vec4<f32>,
    spot: vec4<f32>,
    spot_inner: vec4<f32>,
};

// common.wgsl's Frame, field for field (it may only gain fields at its end).
struct Frame {
    view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    camera_back: vec4<f32>,
    key_dir: vec4<f32>,
    ambient: array<vec4<f32>, 6>,
    ambient_fit: vec4<f32>,
    env_top: vec4<f32>,
    env_bottom: vec4<f32>,
    viewport: vec4<f32>,
    fog_color: vec4<f32>,
    fog_params: vec4<f32>,
    counts: vec4<f32>,
    env_scale: vec4<f32>,
    lights: array<Light, 8>,
    shadow_matrix: mat4x4<f32>,
    shadow: vec4<f32>,
};

struct Params {
    // Sun colour (VRF's particle lighting without probes is 1 + sun), diffuse amount.
    light: vec4<f32>,
    // Overbright, add-self multiplier (1 + amount), self-illum amount, output blend mode.
    shade: vec4<f32>,
    // Alpha remap scale and offset, frames blended (1), unused.
    alpha: vec4<f32>,
    // Source +Z in world space, for the screen-Z orientation.
    up: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var<uniform> params: Params;
@group(1) @binding(1) var t_sprite: texture_2d<f32>;
@group(1) @binding(2) var s_sprite: sampler;

struct Inst {
    @location(0) pos_radius: vec4<f32>,
    @location(1) axis_u: vec4<f32>,
    @location(2) axis_v: vec4<f32>,
    @location(3) color: vec4<f32>,
    @location(4) uv_a: vec4<f32>,
    @location(5) uv_b: vec4<f32>,
    @location(6) misc: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    // Centroid sampling keeps MSAA edge samples inside the frame's tile.
    @location(1) @interpolate(perspective, centroid) uv_a: vec2<f32>,
    @location(2) @interpolate(perspective, centroid) uv_b: vec2<f32>,
    @location(3) @interpolate(flat) blend: f32,
};

fn rotate_about(v: vec3<f32>, axis: vec3<f32>, angle: f32) -> vec3<f32> {
    let s = sin(angle);
    let c = cos(angle);
    return v * c + cross(axis, v) * s + axis * dot(axis, v) * (1.0 - c);
}

@vertex
fn vs_sprite(@builtin(vertex_index) vi: u32, inst: Inst) -> VsOut {
    // Corner ids 0..3 as two triangles (0, 1, 2) and (2, 1, 3).
    var ids = array<u32, 6>(0u, 1u, 2u, 2u, 1u, 3u);
    let id = ids[vi];
    let corner = vec2<f32>(f32(id / 2u) * 2.0 - 1.0, 1.0 - f32(id % 2u) * 2.0);
    let card_uv = vec2<f32>((corner.x + 1.0) * 0.5, (1.0 - corner.y) * 0.5);

    var u = inst.axis_u.xyz;
    var v = inst.axis_v.xyz;
    let mode = inst.axis_u.w;
    if (mode < 1.5) {
        var r = vec3<f32>(frame.view[0][0], frame.view[1][0], frame.view[2][0]);
        var up = vec3<f32>(frame.view[0][1], frame.view[1][1], frame.view[2][1]);
        if (mode > 0.5) {
            // Screen Z aligned: right = Z x camera forward (unnormalized, as VRF), up = Z.
            let forward = -frame.camera_back.xyz;
            r = cross(params.up.xyz, forward);
            up = params.up.xyz;
        }
        let roll = inst.axis_v.w;
        let c = cos(roll);
        let s = sin(roll);
        var right = r * c + up * s;
        let rolled_up = up * c - r * s;
        let yaw = inst.misc.y;
        if (yaw != 0.0 && dot(rolled_up, rolled_up) > 1e-12) {
            right = rotate_about(right, normalize(rolled_up), yaw);
        }
        u = right;
        v = rolled_up;
    }
    let half_u = inst.pos_radius.w;
    let half_v = select(half_u, inst.misc.z, inst.misc.z > 0.0);
    var world = inst.pos_radius.xyz + u * (corner.x * half_u) + v * (corner.y * half_v);
    var color = inst.color;
    var uv_a = mix(inst.uv_a.xy, inst.uv_a.zw, card_uv);
    var uv_b = mix(inst.uv_b.xy, inst.uv_b.zw, card_uv);
    var blend = inst.misc.x;
    if (mode > 2.5) {
        // A rope's strip quad (Grimoire's renderRopes.ts): the top corners at the start
        // sample, the bottom ones at the end, each on its own width axis and half-width;
        // the end's colour rides in uv_b.
        let at_start = corner.y > 0.0;
        let centre = select(inst.axis_v.xyz, inst.pos_radius.xyz, at_start);
        let across = select(inst.misc.xyz, inst.axis_u.xyz, at_start);
        let half = select(inst.axis_v.w, inst.pos_radius.w, at_start);
        world = centre + across * (corner.x * half);
        let uv = vec2<f32>(select(inst.uv_a.z, inst.uv_a.x, corner.x < 0.0), select(inst.uv_a.w, inst.uv_a.y, at_start));
        uv_a = uv;
        uv_b = uv;
        color = select(inst.uv_b, inst.color, at_start);
        blend = 0.0;
    }

    var out: VsOut;
    out.clip = frame.view_proj * vec4<f32>(world, 1.0);
    out.color = color;
    out.uv_a = uv_a;
    out.uv_b = uv_b;
    out.blend = blend;
    return out;
}

@fragment
fn fs_sprite(in: VsOut) -> @location(0) vec4<f32> {
    let a_tex = textureSample(t_sprite, s_sprite, in.uv_a);
    let b_tex = textureSample(t_sprite, s_sprite, in.uv_b);
    let tex = mix(a_tex, b_tex, in.blend);
    var final_color = in.color.rgb * tex.rgb;
    var alpha = tex.a * params.alpha.x + params.alpha.y;
    if (params.alpha.z > 0.5) {
        alpha = clamp(alpha, 0.0, 1.0);
    }
    alpha = alpha * in.color.a;
    let mode = i32(params.shade.w + 0.5);
    let overbright = params.shade.x;
    if (mode == 5) {
        // Modulate 2x: 0.5 leaves the destination untouched.
        let m = mix(vec3<f32>(0.5), mix(vec3<f32>(0.5), final_color * overbright, in.color.rgb), vec3<f32>(alpha));
        return vec4<f32>(clamp(m, vec3<f32>(0.0), vec3<f32>(1.0)), alpha);
    }
    var lit = final_color * params.shade.z;
    if (params.light.w != 0.0) {
        lit = lit + final_color * (vec3<f32>(1.0) + params.light.xyz) * params.light.w;
    }
    final_color = overbright * lit * params.shade.y;
    // Deadlock's branching spritecard: unsigned blend alpha, no saturate before blending.
    var factors = array<f32, 7>(1.0, 0.0, -1.0, 0.5, -0.5, 1.0, 1.0);
    let f = factors[clamp(mode, 0, 6)];
    return vec4<f32>(final_color * mix(alpha, 1.0, clamp(-f, 0.0, 1.0)), alpha * abs(f));
}
