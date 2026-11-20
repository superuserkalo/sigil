// The studio floor: a soft contact shadow under the hero (Grimoire's 0.25 shadow catcher,
// approximated by a radial falloff toward the key light's far side).
struct FloorOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) world: vec3<f32>,
};

struct FloorDraw {
    center: vec4<f32>,   // xz centre, y, radius
    shadow: vec4<f32>,   // key offset xz, strength, unused
};

@group(1) @binding(0) var<uniform> floor_draw: FloorDraw;

@vertex
fn vs_floor(@builtin(vertex_index) i: u32) -> FloorOut {
    var corners = array<vec2<f32>, 6>(vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0));
    let c = corners[i];
    var out: FloorOut;
    let r = floor_draw.center.w;
    let world = vec3<f32>(floor_draw.center.x + c.x * r, floor_draw.center.y, floor_draw.center.z + c.y * r);
    out.local = c;
    out.world = world;
    out.clip = frame.view_proj * vec4<f32>(world, 1.0);
    return out;
}

@fragment
fn fs_floor(in: FloorOut) -> @location(0) vec4<f32> {
    let core = 1.0 - smoothstep(0.0, 0.42, length(in.local * vec2<f32>(1.0, 1.15)));
    let thrown = 1.0 - smoothstep(0.0, 0.85, length((in.local - floor_draw.shadow.xy) * vec2<f32>(1.0, 1.3)));
    let a = clamp(core * 0.55 + thrown * 0.35, 0.0, 1.0) * floor_draw.shadow.z;
    return vec4<f32>(0.0, 0.0, 0.0, a);
}
