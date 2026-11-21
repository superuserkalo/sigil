//! Every viewer shader through naga's HLSL backend with the options wgpu's DX12 backend uses
//! (shader model 5.1 for FXC, clamped indexing, bounded loops), checked for what FXC refuses.
//! No FXC runs here (it is a Windows DLL); `.github/workflows/windows-shaders.yml` compiles the
//! real pipelines on Windows. These checks catch the known failures on every platform.
//!
//! Sigil 0.1.20 shipped a scene shader FXC rejected (X3500, X3511): a one-hot mask built with
//! `mask[channel] = 1.0` and probe faces accumulated with `faces[g] = ...` in a loop.

use naga::back::hlsl;

type Stage = (String, String, String);

/// Each (label, WGSL, entry point pairs) module's stages as HLSL: (label, entry point, source).
fn translate(modules: Vec<super::Module<'_>>) -> Vec<Stage> {
    let mut out: Vec<Stage> = Vec::new();
    for (label, source, pairs) in modules {
        let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(&source)));
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::default())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(&source)));
        for (vs, fs) in pairs {
            let stages = std::iter::once((naga::ShaderStage::Vertex, vs)).chain(fs.map(|fs| (naga::ShaderStage::Fragment, fs)));
            for (stage, entry) in stages {
                if out.iter().any(|(l, e, _)| l == label && e == entry) {
                    continue;
                }
                // As wgpu-hal's dx12 `load_shader`: overrides resolved for this entry point, the
                // fragment stage passed with the vertex one, `hlsl::Options::default()` matching
                // its flags (restrict_indexing, force_loop_bounding, zeroed workgroup memory).
                let (module, info) = naga::back::pipeline_constants::process_overrides(&module, &info, Some((stage, entry)), &Default::default()).unwrap();
                let frag = match (stage, fs) {
                    (naga::ShaderStage::Vertex, Some(fs)) => hlsl::FragmentEntryPoint::new(&module, fs),
                    _ => None,
                };
                let options = hlsl::Options { shader_model: hlsl::ShaderModel::V5_1, ..Default::default() };
                let pipeline = hlsl::PipelineOptions { entry_point: Some((stage, entry.to_string())) };
                let mut text = String::new();
                hlsl::Writer::new(&mut text, &options, &pipeline).write(&module, &info, frag.as_ref()).unwrap_or_else(|e| panic!("{label} {entry}: {e:?}"));
                out.push((label.to_string(), entry.to_string(), text));
            }
        }
    }
    out
}

/// The left side of an assignment on this line (`a[i] = x;`, `a.b += y;`), if it is one.
fn assignment_target(line: &str) -> Option<&str> {
    let b = line.as_bytes();
    let mut depth = 0i32;
    for i in 0..b.len() {
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'=' if depth == 0 => {
                let prev = if i > 0 { b[i - 1] } else { b' ' };
                if b.get(i + 1) == Some(&b'=') || matches!(prev, b'=' | b'!' | b'<' | b'>') {
                    continue;
                }
                return Some(&line[..i]);
            }
            _ => {}
        }
    }
    None
}

/// The contents of each outermost `[...]` in `s`.
fn indices(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0, 0);
    for (i, c) in s.char_indices() {
        match c {
            '[' => {
                if depth == 0 {
                    start = i + 1;
                }
                depth += 1;
            }
            ']' => {
                depth -= 1;
                if depth == 0 {
                    out.push(&s[start..i]);
                }
            }
            _ => {}
        }
    }
    out
}

/// Writes through an index that is not a literal. FXC cannot address a vector's component, a
/// matrix's column or an array inside a struct through a register: it unrolls the loop around
/// the write to make the index constant (X3550) and fails where the index stays dynamic
/// (X3500) or the loop does not unroll (X3511; naga's bounded `while(true)` loops never have a
/// constant trip count). A plain local array would compile, but the rule stays simple: every
/// write to an indexed local uses a literal index.
fn dynamic_index_writes(hlsl: &str) -> Vec<String> {
    let literal = |ix: &str| {
        let ix = ix.trim().trim_end_matches(['u', 'U']);
        !ix.is_empty() && ix.bytes().all(|b| b.is_ascii_digit())
    };
    hlsl.lines()
        .enumerate()
        .filter(|(_, line)| assignment_target(line).is_some_and(|lhs| indices(lhs).iter().any(|ix| !literal(ix))))
        .map(|(n, line)| format!("{}: {}", n + 1, line.trim()))
        .collect()
}

/// HLSL that needs screen-space derivatives: implicit-LOD sampling and the derivative ops.
const GRADIENT_OPS: [&str; 6] = [".Sample(", ".SampleBias(", ".SampleCmp(", "ddx", "ddy", "fwidth("];

/// Whether `line` calls the function `name`.
fn calls(line: &str, name: &str) -> bool {
    let pattern = format!("{name}(");
    line.match_indices(&pattern).any(|(i, _)| !line[..i].ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_' || c == '.'))
}

/// naga's functions: (name, body lines with their line numbers). A function starts at a column
/// 0 signature followed by a `{` line and ends at the next column 0 `}`.
fn functions(hlsl: &str) -> Vec<(String, Vec<(usize, &str)>)> {
    let lines: Vec<&str> = hlsl.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < lines.len() {
        let head = lines[i];
        let starts = !head.starts_with([' ', '\t', '}', '#', '/']) && !head.starts_with("struct ") && head.contains('(') && lines[i + 1] == "{";
        if !starts {
            i += 1;
            continue;
        }
        let name = head[..head.find('(').unwrap()].split_whitespace().last().unwrap_or_default().to_string();
        let mut body = Vec::new();
        let mut j = i + 2;
        while j < lines.len() && lines[j] != "}" {
            body.push((j + 1, lines[j]));
            j += 1;
        }
        out.push((name, body));
        i = j + 1;
    }
    out
}

/// Derivatives inside a loop, directly or through a call. FXC must unroll such a loop (X3570)
/// and fails when it cannot (X3511): sample with an explicit level in loops.
fn gradients_in_loops(hlsl: &str) -> Vec<String> {
    let fns = functions(hlsl);
    let mut gradient: Vec<&str> = Vec::new();
    loop {
        let before = gradient.len();
        for (name, body) in &fns {
            if !gradient.contains(&name.as_str()) && body.iter().any(|(_, l)| GRADIENT_OPS.iter().any(|op| l.contains(op)) || gradient.iter().any(|g| calls(l, g))) {
                gradient.push(name);
            }
        }
        if gradient.len() == before {
            break;
        }
    }
    let mut found = Vec::new();
    for (_, body) in &fns {
        let (mut depth, mut loops) = (0i32, Vec::<i32>::new());
        for (n, line) in body {
            let opens = line.trim_start().starts_with("while(true)") || line.trim_start().starts_with("for(") || line.trim_start().starts_with("do {");
            if !loops.is_empty() && (GRADIENT_OPS.iter().any(|op| line.contains(op)) || gradient.iter().any(|g| calls(line, g))) {
                found.push(format!("{n}: {}", line.trim()));
            }
            if opens {
                loops.push(depth);
            }
            depth += line.matches('{').count() as i32 - line.matches('}').count() as i32;
            while loops.last().is_some_and(|&d| depth <= d) && !opens {
                loops.pop();
            }
        }
    }
    found
}

fn problems(stages: &[Stage]) -> Vec<String> {
    let mut out = Vec::new();
    for (label, entry, text) in stages {
        for p in dynamic_index_writes(text) {
            out.push(format!("{label} {entry}: write through a dynamic index (FXC X3500/X3511): {p}"));
        }
        for p in gradients_in_loops(text) {
            out.push(format!("{label} {entry}: implicit derivatives in a loop (FXC X3570/X3511): {p}"));
        }
    }
    out
}

/// The viewer's own shaders: every pipeline stage, nothing FXC rejects.
#[test]
fn every_viewer_shader_avoids_what_fxc_rejects() {
    let stages = translate(super::modules());
    assert_eq!(stages.len(), 16, "a module or entry point went missing");
    let found = problems(&stages);
    assert!(found.is_empty(), "the HLSL wgpu hands FXC on Windows would not compile:\n{}", found.join("\n"));
}

/// The checks see the patterns in naga's real output: each bad entry point is caught, the
/// clean one is not.
#[test]
fn the_checks_catch_what_broke_0_1_20() {
    let wgsl = r#"
struct Probe { faces: array<vec3<f32>, 6> };
@group(0) @binding(0) var<uniform> u: vec4<f32>;
@group(0) @binding(1) var t: texture_2d<f32>;
@group(0) @binding(2) var s: sampler;

@vertex fn vs() -> @builtin(position) vec4<f32> { return vec4<f32>(0.0); }

@fragment fn fs_vector() -> @location(0) vec4<f32> {
    var mask = vec4<f32>(0.0);
    mask[u32(u.x) & 3u] = 1.0;
    return mask;
}

@fragment fn fs_struct() -> @location(0) vec4<f32> {
    var p: Probe;
    for (var f = 0u; f < 6u; f = f + 1u) {
        p.faces[f] = p.faces[f] + vec3<f32>(u.y);
    }
    return vec4<f32>(p.faces[2], 1.0);
}

fn tap(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(t, s, uv);
}

@fragment fn fs_gradient() -> @location(0) vec4<f32> {
    var c = vec4<f32>(0.0);
    for (var i = 0u; i < u32(u.z); i = i + 1u) {
        c = c + tap(vec2<f32>(f32(i)));
    }
    return c;
}

@fragment fn fs_clean() -> @location(0) vec4<f32> {
    var p: Probe;
    var c = textureSample(t, s, u.xy);
    for (var i = 0u; i < u32(u.z); i = i + 1u) {
        c = c + textureSampleLevel(t, s, vec2<f32>(f32(i)), 0.0);
    }
    p.faces[3] = c.rgb;
    let mask = select(vec4<f32>(0.0), vec4<f32>(1.0), vec4<u32>(0u, 1u, 2u, 3u) == vec4<u32>(u32(u.x) & 3u));
    return vec4<f32>(p.faces[3], 1.0) * mask;
}
"#;
    let entries = vec![("vs", Some("fs_vector")), ("vs", Some("fs_struct")), ("vs", Some("fs_gradient")), ("vs", Some("fs_clean"))];
    let stages = translate(vec![("bad", wgsl.into(), entries)]);
    let of = |entry: &str| {
        let stage: Vec<Stage> = stages.iter().filter(|(_, e, _)| e == entry).cloned().collect();
        problems(&stage)
    };
    assert!(of("fs_vector").iter().any(|p| p.contains("dynamic index") && p.contains("mask[")), "{:?}", of("fs_vector"));
    assert!(of("fs_struct").iter().any(|p| p.contains("dynamic index") && p.contains(".faces[")), "{:?}", of("fs_struct"));
    assert!(of("fs_gradient").iter().any(|p| p.contains("derivatives") && p.contains("tap(")), "{:?}", of("fs_gradient"));
    assert!(of("fs_clean").is_empty(), "{:?}", of("fs_clean"));
    assert!(of("vs").is_empty(), "{:?}", of("vs"));
}

/// `SIGIL_HLSL_DUMP=<dir> cargo test -p sigil-viewer dump_hlsl` writes each stage's HLSL there
/// (FXC's error lines point into it; wgpu's own header adds a few lines on top).
#[test]
fn dump_hlsl() {
    let Some(dir) = std::env::var_os("SIGIL_HLSL_DUMP") else { return };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (label, entry, text) in translate(super::modules()) {
        std::fs::write(dir.join(format!("{}-{entry}.hlsl", label.replace(' ', "-"))), text).unwrap();
    }
}
