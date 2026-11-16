use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;

fn temp_workspace(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    let path = env::temp_dir().join(format!(
        "dah-character-{label}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn cleanup(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

/// Build a PNG with a centred opaque disc on a transparent background. The
/// disc is antialiased at the rim so the edge-quality heuristic has something
/// real to measure.
fn png_with_disc(width: u32, height: u32) -> Vec<u8> {
    let mut raw = Vec::with_capacity((height * (1 + width * 4)) as usize);
    let cx = width as f64 / 2.0;
    let cy = height as f64 / 2.0;
    let radius = width.min(height) as f64 * 0.35;
    let softness = 2.0f64;

    for y in 0..height {
        raw.push(0u8);
        for x in 0..width {
            let dx = x as f64 + 0.5 - cx;
            let dy = y as f64 + 0.5 - cy;
            let distance = (dx * dx + dy * dy).sqrt();
            let alpha = if distance <= radius - softness {
                255.0
            } else if distance >= radius {
                0.0
            } else {
                ((radius - distance) / softness * 255.0).clamp(0.0, 255.0)
            };
            raw.extend_from_slice(&[200u8, 120, 90, alpha as u8]);
        }
    }

    let mut encoder =
        flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&raw).unwrap();
    let compressed = encoder.finish().unwrap();

    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    push_png_chunk(&mut png, b"IHDR", &ihdr);
    push_png_chunk(&mut png, b"IDAT", &compressed);
    push_png_chunk(&mut png, b"IEND", &[]);
    png
}

fn push_png_chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    png.extend_from_slice(&(data.len() as u32).to_be_bytes());
    png.extend_from_slice(kind);
    png.extend_from_slice(data);
    let mut body = Vec::with_capacity(4 + data.len());
    body.extend_from_slice(kind);
    body.extend_from_slice(data);
    png.extend_from_slice(&crc32(&body).to_be_bytes());
}

/// A hand-built GLB with one triangle, positions, normals and UVs.
fn glb_with_triangle() -> Vec<u8> {
    let positions: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let normals: [[f32; 3]; 3] = [[0.0, 0.0, 1.0]; 3];
    let uvs: [[f32; 2]; 3] = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
    let indices: [u16; 3] = [0, 1, 2];

    let mut bin = Vec::new();
    for position in positions {
        for value in position {
            bin.extend_from_slice(&value.to_le_bytes());
        }
    }
    let position_bytes = bin.len();
    for normal in normals {
        for value in normal {
            bin.extend_from_slice(&value.to_le_bytes());
        }
    }
    let normal_bytes = bin.len() - position_bytes;
    for uv in uvs {
        for value in uv {
            bin.extend_from_slice(&value.to_le_bytes());
        }
    }
    let uv_bytes = bin.len() - position_bytes - normal_bytes;
    for index in indices {
        bin.extend_from_slice(&index.to_le_bytes());
    }
    let index_bytes = bin.len() - position_bytes - normal_bytes - uv_bytes;
    while bin.len() % 4 != 0 {
        bin.push(0);
    }

    let json = json!({
        "asset": {"version": "2.0", "generator": "dah-test"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": [{
            "attributes": {"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2},
            "indices": 3,
            "mode": 4
        }]}],
        "buffers": [{"byteLength": bin.len()}],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": position_bytes, "target": 34962},
            {"buffer": 0, "byteOffset": position_bytes, "byteLength": normal_bytes, "target": 34962},
            {"buffer": 0, "byteOffset": position_bytes + normal_bytes, "byteLength": uv_bytes, "target": 34962},
            {"buffer": 0, "byteOffset": position_bytes + normal_bytes + uv_bytes, "byteLength": index_bytes, "target": 34963}
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
             "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 0.0]},
            {"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3"},
            {"bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC2"},
            {"bufferView": 3, "componentType": 5123, "count": 3, "type": "SCALAR"}
        ],
        "materials": [{"pbrMetallicRoughness": {"metallicFactor": 0.0}}]
    });

    let mut json_bytes = serde_json::to_vec(&json).unwrap();
    while json_bytes.len() % 4 != 0 {
        json_bytes.push(b' ');
    }

    let total = 12 + 8 + json_bytes.len() + 8 + bin.len();
    let mut output = Vec::with_capacity(total);
    output.extend_from_slice(b"glTF");
    output.extend_from_slice(&2u32.to_le_bytes());
    output.extend_from_slice(&(total as u32).to_le_bytes());
    output.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    output.extend_from_slice(&GLB_JSON_CHUNK.to_le_bytes());
    output.extend_from_slice(&json_bytes);
    output.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    output.extend_from_slice(&GLB_BIN_CHUNK.to_le_bytes());
    output.extend_from_slice(&bin);
    output
}

#[test]
fn provider_debug_redacts_the_key() {
    let provider =
        MeshyProvider::with_base_url("secret-key-value", "https://api.meshy.ai").unwrap();
    let text = format!("{provider:?}");
    assert!(!text.contains("secret-key-value"));
    assert!(text.contains("redacted"));
}

#[test]
fn base64_matches_known_vectors() {
    assert_eq!(base64_encode(b""), "");
    assert_eq!(base64_encode(b"f"), "Zg==");
    assert_eq!(base64_encode(b"fo"), "Zm8=");
    assert_eq!(base64_encode(b"foo"), "Zm9v");
    assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");

    for sample in [b"".as_slice(), b"f", b"fo", b"foo", b"foobar", b"glTF bytes"] {
        assert_eq!(base64_decode(&base64_encode(sample)).unwrap(), sample);
    }
    assert!(base64_decode("A").is_err());
    assert!(base64_decode("****").is_err());
}

#[test]
fn slugify_is_stable() {
    assert_eq!(slugify("A Red Hooded Character"), "a-red-hooded-character");
    assert_eq!(slugify("  --  "), "character");
    assert_eq!(slugify("!!!!"), "character");
}

#[test]
fn stage_graph_is_ordered_and_complete() {
    let graph = stage_graph();
    assert_eq!(graph.len(), 11);
    assert_eq!(graph[0].id, "prepare_reference");
    assert_eq!(graph[1].id, "generate_views");
    assert_eq!(graph[10].id, "export");

    let mut ids: Vec<&str> = graph.iter().map(|stage| stage.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 11, "stage ids must be unique");

    for stage in &graph {
        assert!(
            !stage.evidence.is_empty(),
            "{} carries no evidence id",
            stage.id
        );
    }

    // The deterministic stages are the ones that need no model.
    let native: Vec<&str> = graph
        .iter()
        .filter(|stage| !stage.generative)
        .map(|stage| stage.id)
        .collect();
    assert_eq!(
        native,
        vec![
            "prepare_reference",
            "cleanup",
            "source2_map",
            "export"
        ]
    );
}

#[test]
fn reference_analysis_measures_a_transparent_png() {
    let workspace = temp_workspace("reference");
    let image = workspace.join("reference.png");
    fs::write(&image, png_with_disc(128, 128)).unwrap();

    let report = analyse_reference(&image).unwrap();

    assert_eq!(report.format, "png");
    assert_eq!(report.width, 128);
    assert_eq!(report.height, 128);
    assert!(report.has_alpha_channel);

    let coverage = report.alpha_coverage.expect("coverage is measured");
    assert!(
        (0.25..0.5).contains(&coverage),
        "disc covers about a third of the image, got {coverage}"
    );
    assert_eq!(report.centered, Some(true));

    let box_xywh = report.subject_box.expect("subject box is measured");
    assert!(box_xywh[2] > 80 && box_xywh[2] < 100, "{box_xywh:?}");
    assert!(box_xywh[0] > 10, "the disc is inset from the left edge");

    let quality = report.edge_quality.expect("edge quality is measured");
    assert!(
        (0.4..=1.0).contains(&quality),
        "an antialiased rim should read as soft, got {quality}"
    );
    assert!(report.problems.is_empty(), "{:?}", report.problems);

    cleanup(&workspace);
}

#[test]
fn reference_analysis_rejects_a_tiny_png() {
    let workspace = temp_workspace("tiny");
    let image = workspace.join("reference.png");
    fs::write(&image, png_with_disc(16, 16)).unwrap();

    let report = analyse_reference(&image).unwrap();
    assert!(
        report.problems.iter().any(|problem| problem.contains("fatal")),
        "{:?}",
        report.problems
    );

    cleanup(&workspace);
}

#[test]
fn reference_analysis_reads_jpeg_headers_without_decoding() {
    // SOI, APP0, SOF0 with 64x48 and 3 components.
    let bytes: Vec<u8> = vec![
        0xFF, 0xD8, // SOI
        0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00, // APP0, empty
        0xFF, 0xC0, 0x00, 0x11, 0x08, 0x00, 0x30, 0x00, 0x40, 0x03, 0x01, 0x11, 0x00, 0x02, 0x11,
        0x01, 0x03, 0x11, 0x01,
    ];

    let header = read_image_header(&bytes).unwrap();
    assert_eq!(header.format, "jpeg");
    assert_eq!(header.width, 64);
    assert_eq!(header.height, 48);
    assert!(!header.has_alpha_channel);
}

#[test]
fn glb_parser_reads_a_real_triangle() {
    let report = parse_glb(&glb_with_triangle(), "glb").unwrap();

    assert_eq!(report.glb_version, 2);
    assert_eq!(report.generator.as_deref(), Some("dah-test"));
    assert_eq!(report.meshes, 1);
    assert_eq!(report.primitives, 1);
    assert_eq!(report.triangles, 1);
    assert_eq!(report.vertices, 3);
    assert_eq!(report.materials, 1);
    assert!(report.has_normals);
    assert!(report.has_uvs);
    assert!(!report.has_tangents);
    assert!(!report.has_skin);
    assert_eq!(report.bounds_min, Some([0.0, 0.0, 0.0]));
    assert_eq!(report.bounds_max, Some([1.0, 1.0, 0.0]));
    assert_eq!(report.accessors_checked, 4);
    assert_eq!(report.bounds_size(), Some([1.0, 1.0, 0.0]));
    assert_eq!(report.height(), Some(1.0));
    assert!(report.problems.is_empty(), "{:?}", report.problems);
    assert!(!report.over_budget(60_000));
    assert_eq!(report.fitting_lods(), vec![0, 1, 2, 3]);
}

#[test]
fn glb_parser_reports_an_accessor_past_the_buffer_view() {
    let mut bytes = glb_with_triangle();
    let needle = b"\"count\":3,\"type\":\"VEC3\"";
    let position = bytes
        .windows(needle.len())
        .position(|window| window == needle)
        .expect("the test GLB contains the accessor");

    // Grow the element count far past the 36 byte view without growing the BIN
    // chunk: the parser must notice.
    let replacement = b"\"count\":9,\"type\":\"VEC3\"";
    bytes[position..position + needle.len()].copy_from_slice(replacement);

    let report = parse_glb(&bytes, "glb").unwrap();
    assert!(
        report
            .problems
            .iter()
            .any(|problem| problem.contains("bufferView")),
        "{:?}",
        report.problems
    );
}

#[test]
fn glb_parser_rejects_a_broken_chunk_table() {
    let mut bytes = glb_with_triangle();
    // Corrupt the declared total length in the header.
    bytes[8] = bytes[8].wrapping_add(0x40);

    assert!(parse_glb(&bytes, "glb").is_err());

    let mut wrong_magic = glb_with_triangle();
    wrong_magic[0] = b'X';
    assert!(parse_glb(&wrong_magic, "glb").is_err());

    let mut short = glb_with_triangle();
    short.truncate(8);
    assert!(parse_glb(&short, "glb").is_err());
}

#[test]
fn minimal_glb_passes_validation_and_parses_empty() {
    let bytes = minimal_glb_bytes();
    let version = validate_glb_bytes(&bytes).unwrap();
    assert_eq!(version, 2);

    let report = parse_glb(&bytes, "glb").unwrap();
    assert_eq!(report.meshes, 0);
    assert_eq!(report.triangles, 0);
    assert!(report.problems.iter().any(|problem| problem.contains("no meshes")));
}

#[test]
fn glb_validation_rejects_html_and_truncated_files() {
    assert!(validate_glb_bytes(b"<html>not a model</html>").is_err());
    assert!(validate_glb_bytes(b"glTF").is_err());

    let mut wrong_length = minimal_glb_bytes();
    wrong_length.truncate(wrong_length.len() - 4);
    assert!(validate_glb_bytes(&wrong_length).is_err());
}

#[test]
fn create_without_a_backend_records_a_real_stage_graph() {
    let workspace = temp_workspace("no-backend");
    let image = workspace.join("reference.png");
    fs::write(&image, png_with_disc(96, 128)).unwrap();

    let mut request = CharacterRequest::new("", CharacterProviderKind::Auto);
    request.reference_image = Some(image);
    request.backend = Some("none".to_string());
    request.target_hero = Some("billy".to_string());

    let record = create_character(&workspace, &request).unwrap();

    let stages = record.stages();
    let state = |stage: CharacterStage| {
        stages
            .iter()
            .find(|record| record.id == stage)
            .map(|record| record.state)
            .unwrap()
    };

    assert_eq!(state(CharacterStage::PrepareReference), StageState::Succeeded);
    assert_eq!(state(CharacterStage::GenerateViews), StageState::Unsupported);
    assert_eq!(state(CharacterStage::Reconstruct), StageState::Skipped);
    assert_eq!(state(CharacterStage::Source2Map), StageState::Skipped);
    assert_eq!(state(CharacterStage::Export), StageState::Skipped);

    // The refusal names the fix.
    let generate = stages
        .iter()
        .find(|record| record.id == CharacterStage::GenerateViews)
        .unwrap();
    assert!(
        generate.notes[0].contains(MESHGEN_COMMAND_ENV),
        "{:?}",
        generate.notes
    );

    assert_eq!(record.task.status, CharacterStatus::Blocked);
    assert_eq!(record.task.stage, STAGE_PIPELINE);
    assert!(record.task.error.as_deref().unwrap().contains("generate_views"));

    // The reference stage did real work.
    let reference = record.reference.expect("reference report");
    assert_eq!(reference.width, 96);
    assert_eq!(reference.height, 128);
    assert!(reference.alpha_coverage.is_some());
    assert_eq!(reference.centered, Some(true));

    // Nothing claims a game-ready asset.
    assert!(record.mesh.is_none());
    assert!(!record.rights.publish_allowed);
    assert!(!model_path(&workspace, &record.slug).exists());
    assert!(!character_manifest_path(&workspace, &record.slug).exists());

    // Import refuses to pretend.
    let error = import_character(&workspace, &record.slug).unwrap_err();
    assert!(error.to_string().contains("BLOCKED"), "{error}");

    // The record round-trips through task.json with the stage graph intact.
    let reread = read_task_record(&workspace, &record.slug).unwrap();
    assert_eq!(reread.stages().len(), 11);
    assert_eq!(
        reread
            .stage(CharacterStage::PrepareReference)
            .map(|stage| stage.state),
        Some(StageState::Succeeded)
    );

    cleanup(&workspace);
}

#[test]
fn mock_pipeline_records_task_then_imports() {
    let workspace = temp_workspace("mock-pipeline");
    let mut request =
        CharacterRequest::new("a slim pilot in a green coat", CharacterProviderKind::Mock);
    request.target_hero = Some("billy".to_string());

    let record = create_character(&workspace, &request).unwrap();
    assert_eq!(record.task.status, CharacterStatus::Pending);
    assert!(task_path(&workspace, &record.slug).is_file());
    assert!(!model_path(&workspace, &record.slug).exists());

    let first = refresh_character(&workspace, &record.slug).unwrap();
    assert_eq!(first.task.status, CharacterStatus::InProgress);
    assert_eq!(first.task.progress, 50);

    let second = refresh_character(&workspace, &record.slug).unwrap();
    assert_eq!(second.task.status, CharacterStatus::Succeeded);
    assert!(model_path(&workspace, &record.slug).is_file());
    assert!(second.asset.is_some());
    assert!(second.mesh.is_some());

    let import = import_character(&workspace, &record.slug).unwrap();
    assert_eq!(import.source2_compile, SOURCE2_COMPILE_REQUIRED);
    assert!(!import.in_game_ready);
    assert_eq!(import.target_hero.as_deref(), Some("billy"));
    assert_eq!(import.provider, "mock");
    assert_eq!(import.backend, "mock");
    assert!(!import.rights.publish_allowed);
    assert!(import.notes.iter().any(|note| note.contains("untextured")));
    assert!(character_manifest_path(&workspace, &record.slug).is_file());
    assert_eq!(import.stages.len(), 11);

    let plan = import.source2_plan.expect("source2 plan");
    assert!(!plan.in_game_ready);
    assert!(!plan.publish_allowed);
    assert_eq!(plan.source2_compile, SOURCE2_COMPILE_REQUIRED);
    assert!(plan.artifacts.iter().any(|item| item.kind == "vmdl"));
    assert!(plan.artifacts.iter().any(|item| item.kind == "vmat"));
    assert!(plan.artifacts.iter().any(|item| item.kind == "vtex"));
    assert!(
        plan.artifacts
            .iter()
            .filter(|item| item.kind == "vnmskel_c")
            .all(|item| !item.shipped_by_mod)
    );

    let status = character_status(&workspace).unwrap();
    assert_eq!(status.len(), 1);
    assert_eq!(status[0].slug, record.slug);

    // A deleted GLB is restored from the recorded task instead of leaving a
    // succeeded task without an asset.
    fs::remove_file(model_path(&workspace, &record.slug)).unwrap();
    let restored = refresh_character(&workspace, &record.slug).unwrap();
    assert!(restored.asset.is_some());
    assert!(model_path(&workspace, &record.slug).is_file());

    cleanup(&workspace);
}

#[test]
fn import_refuses_an_unfinished_task() {
    let workspace = temp_workspace("unfinished");
    let request = CharacterRequest::new("a test character", CharacterProviderKind::Mock);
    let record = create_character(&workspace, &request).unwrap();

    let error = import_character(&workspace, &record.slug).unwrap_err();
    assert!(error.to_string().contains("PENDING"));

    cleanup(&workspace);
}

#[test]
fn reference_image_needs_consent_for_a_remote_provider() {
    let workspace = temp_workspace("consent");
    let image = workspace.join("reference.png");
    fs::write(&image, png_with_disc(64, 64)).unwrap();

    let mut request = CharacterRequest::new("", CharacterProviderKind::Meshy);
    request.reference_image = Some(image);

    let provider = MeshyProvider::with_base_url("test-key", "http://127.0.0.1:1").unwrap();
    let error = create_character_with_provider(&workspace, &request, &provider).unwrap_err();
    assert!(error.to_string().contains("consent"));

    cleanup(&workspace);
}

#[test]
fn meshy_flow_against_a_local_scripted_server() {
    let workspace = temp_workspace("http-meshy");
    let image = workspace.join("reference.png");
    fs::write(&image, png_with_disc(64, 64)).unwrap();

    let server = ScriptedMeshyServer::start();

    let mut request = CharacterRequest::new(
        "a tall mechanic with a red scarf",
        CharacterProviderKind::Meshy,
    );
    request.style = "stylized painted game character".to_string();
    request.reference_image = Some(image);
    request.allow_reference_upload = true;

    let provider = MeshyProvider::with_base_url("test-key", server.base_url()).unwrap();

    let record = create_character_with_provider(&workspace, &request, &provider).unwrap();
    assert_eq!(record.task.status, CharacterStatus::Pending);
    assert_eq!(record.task.id, "img-task-1");
    assert_eq!(record.task.stage, STAGE_IMAGE_TO_3D);
    assert!(record.rights.reference_uploaded);

    let refreshed =
        refresh_character_with_provider(&workspace, &record.slug, &provider).unwrap();
    assert_eq!(refreshed.task.status, CharacterStatus::Succeeded);
    assert_eq!(refreshed.task.progress, 100);
    assert_eq!(refreshed.task.retry_after_seconds, Some(2));
    assert!(refreshed.asset.is_some());

    let import = import_character(&workspace, &record.slug).unwrap();
    assert_eq!(import.source2_compile, SOURCE2_COMPILE_REQUIRED);
    assert!(!import.in_game_ready);
    assert_eq!(import.task_id, "img-task-1");
    assert_eq!(import.asset.glb_version, 2);
    assert_eq!(import.asset.format, "glb");
    assert!(import.asset.bytes > 0);
    assert!(!import.rights.publish_allowed);
    let mesh = import.mesh.expect("mesh report");
    assert_eq!(mesh.meshes, 1);
    assert_eq!(mesh.triangles, 1);

    server.join();
    cleanup(&workspace);
}

#[test]
fn local_command_backend_runs_a_stage_without_a_shell() {
    let workspace = temp_workspace("local-backend");
    let image = workspace.join("reference.png");
    fs::write(&image, png_with_disc(64, 64)).unwrap();
    let glb = workspace.join("triangle.glb");
    fs::write(&glb, glb_with_triangle()).unwrap();

    let script = workspace.join("backend.sh");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\nset -e\nif [ \"$DAH_STAGE\" = \"generate_views\" ]; then\n  echo \\
             'front view' > \"$DAH_STAGE_DIR/view.txt\"\nfi\nif [ \"$DAH_STAGE\" = \"reconstruct\" \\
             ]; then\n  cp '{}' \"$DAH_CHARACTER_DIR/model.glb\"\nfi\n",
            glb.display()
        ),
    )
    .unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions).unwrap();
    }

    let backend = LocalCommandBackend::new(&script, Vec::new())
        .with_stages(vec![
            CharacterStage::GenerateViews,
            CharacterStage::Reconstruct,
        ])
        .with_timeout(Duration::from_secs(30));

    let request = {
        let mut request = CharacterRequest::new("", CharacterProviderKind::Local);
        request.reference_image = Some(image);
        request.target_hero = Some("billy".to_string());
        request
    };

    let record = create_character_with_backend(&workspace, &request, Some(&backend)).unwrap();

    assert_eq!(record.task.status, CharacterStatus::Succeeded);
    assert_eq!(
        record
            .stage(CharacterStage::GenerateViews)
            .map(|stage| stage.state),
        Some(StageState::Succeeded)
    );
    assert_eq!(
        record
            .stage(CharacterStage::Reconstruct)
            .map(|stage| stage.state),
        Some(StageState::Succeeded)
    );

    let mesh = record.mesh.clone().expect("mesh report");
    assert_eq!(mesh.triangles, 1);
    assert!(mesh.has_normals && mesh.has_uvs);
    assert_eq!(
        record
            .stage(CharacterStage::Cleanup)
            .map(|stage| stage.state),
        Some(StageState::Skipped)
    );
    assert_eq!(
        record
            .stage(CharacterStage::Export)
            .map(|stage| stage.state),
        Some(StageState::Succeeded)
    );
    assert!(export_path(&workspace, &record.slug).is_file());
    assert!(source2_plan_path(&workspace, &record.slug).is_file());

    // The view file was recorded as an output of generate_views.
    let views = record
        .stage(CharacterStage::GenerateViews)
        .expect("views stage");
    assert!(views.outputs.iter().any(|path| path.ends_with("view.txt")));

    cleanup(&workspace);
}

#[test]
fn local_command_backend_reports_a_missing_mesh() {
    let workspace = temp_workspace("local-empty");
    let script = workspace.join("backend.sh");
    fs::write(&script, "#!/bin/sh\nexit 0\n").unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions).unwrap();
    }

    let backend = LocalCommandBackend::new(&script, Vec::new())
        .with_stages(vec![CharacterStage::Reconstruct])
        .with_timeout(Duration::from_secs(30));

    let request = CharacterRequest::new("a test character", CharacterProviderKind::Local);

    let record = create_character_with_backend(&workspace, &request, Some(&backend)).unwrap();
    let reconstruct = record
        .stage(CharacterStage::Reconstruct)
        .expect("reconstruct stage");
    assert_eq!(reconstruct.state, StageState::Skipped, "blocked on views");

    cleanup(&workspace);
}

#[test]
fn http_backend_accepts_a_base64_mesh() {
    let workspace = temp_workspace("http-backend");
    let server = ScriptedMeshServer::start(&base64_encode(&glb_with_triangle()));

    let backend = HttpBackend::new(server.base_url())
        .with_stages(vec![
            CharacterStage::GenerateViews,
            CharacterStage::Reconstruct,
        ])
        .with_timeout(Duration::from_secs(30));

    let request = CharacterRequest::new("a test character", CharacterProviderKind::Http);

    let record = create_character_with_backend(&workspace, &request, Some(&backend)).unwrap();

    assert_eq!(
        record
            .stage(CharacterStage::GenerateViews)
            .map(|stage| stage.state),
        Some(StageState::Succeeded)
    );
    let mesh = record.mesh.expect("mesh report");
    assert_eq!(mesh.triangles, 1);
    assert_eq!(record.backend, format!("http:{}", server.base_url()));

    server.join();
    cleanup(&workspace);
}

#[test]
fn schema_versions_are_checked() {
    let workspace = temp_workspace("schema");
    let mut request = CharacterRequest::new("a test character", CharacterProviderKind::Mock);
    request.backend = Some("none".to_string());
    let mut record = create_character(&workspace, &request).unwrap();

    record.schema = 99;
    let mut text = serde_json::to_string_pretty(&record).unwrap();
    text.push('\n');
    fs::write(task_path(&workspace, &record.slug), text).unwrap();

    let error = read_task_record(&workspace, &record.slug).unwrap_err();
    assert!(error.to_string().contains("uses schema 99"));

    cleanup(&workspace);
}

// ---------------------------------------------------------------------------
// Scripted servers
// ---------------------------------------------------------------------------

struct ScriptedMeshyServer {
    base_url: String,
    handle: Option<thread::JoinHandle<()>>,
}

impl ScriptedMeshyServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let base_url = format!("http://127.0.0.1:{port}");

        let server_url = base_url.clone();
        let handle = thread::spawn(move || {
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                let request = read_request(&mut stream);
                let (status, content_type, body) = route(&request, &server_url);
                write_response(&mut stream, status, &content_type, &body);
            }
        });

        Self {
            base_url,
            handle: Some(handle),
        }
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn join(mut self) {
        if let Some(handle) = self.handle.take() {
            handle.join().unwrap();
        }
    }
}

fn route(request: &ScriptedRequest, base_url: &str) -> (u16, String, Vec<u8>) {
    let needs_auth = request.path.starts_with("/openapi/");
    let authorized = !needs_auth
        || request.headers.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("authorization") && value == "Bearer test-key"
        });

    if !authorized {
        return (
            401,
            "application/json".to_string(),
            br#"{"message":"invalid api key"}"#.to_vec(),
        );
    }

    match (request.method.as_str(), request.path.as_str()) {
        ("POST", "/openapi/v1/image-to-3d") => {
            let body = String::from_utf8_lossy(&request.body);
            if !body.contains("data:image/png;base64,") {
                return (
                    400,
                    "application/json".to_string(),
                    br#"{"message":"image_url is missing a data URI"}"#.to_vec(),
                );
            }
            if !body.contains("texture_prompt") {
                return (
                    400,
                    "application/json".to_string(),
                    br#"{"message":"texture_prompt missing"}"#.to_vec(),
                );
            }
            (
                202,
                "application/json".to_string(),
                br#"{"result":"img-task-1"}"#.to_vec(),
            )
        }
        ("GET", "/openapi/v1/image-to-3d/img-task-1") => {
            let glb = format!("{base_url}/files/model.glb");
            let thumbnail = format!("{base_url}/files/preview.png");
            let body = json!({
                "id": "img-task-1",
                "type": "image-to-3d",
                "status": "SUCCEEDED",
                "progress": 100,
                "model_urls": { "glb": glb },
                "thumbnail_url": thumbnail,
                "task_error": null,
                "consumed_credits": 30,
            });
            (
                200,
                "application/json".to_string(),
                body.to_string().into_bytes(),
            )
        }
        ("GET", "/files/model.glb") => (
            200,
            "model/gltf-binary".to_string(),
            glb_with_triangle(),
        ),
        (method, path) => (
            404,
            "application/json".to_string(),
            format!(r#"{{"message":"no route for {method} {path}"}}"#).into_bytes(),
        ),
    }
}

struct ScriptedMeshServer {
    base_url: String,
    handle: Option<thread::JoinHandle<()>>,
}

impl ScriptedMeshServer {
    fn start(glb_base64: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let base_url = format!("http://127.0.0.1:{port}");
        let payload = glb_base64.to_string();

        let handle = thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let request = read_request(&mut stream);
                let stage = serde_json::from_slice::<Value>(&request.body)
                    .ok()
                    .and_then(|value| {
                        value
                            .get("stage")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .unwrap_or_default();
                let body = if stage == "reconstruct" {
                    json!({
                        "state": "succeeded",
                        "outputs": [{"path": "model.glb", "dataBase64": payload}],
                        "notes": ["scripted reconstruct"],
                        "credits": 1,
                    })
                } else {
                    json!({
                        "state": "succeeded",
                        "outputs": [],
                        "notes": ["scripted views"]
                    })
                };

                write_response(
                    &mut stream,
                    200,
                    "application/json",
                    body.to_string().as_bytes(),
                );
            }
        });

        Self {
            base_url,
            handle: Some(handle),
        }
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn join(mut self) {
        if let Some(handle) = self.handle.take() {
            handle.join().unwrap();
        }
    }
}

struct ScriptedRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

fn read_request(stream: &mut TcpStream) -> ScriptedRequest {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];

    let header_end = loop {
        let read = stream.read(&mut chunk).unwrap();
        if read == 0 {
            panic!("client closed before sending headers");
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(index) = find_headers_end(&buffer) {
            break index;
        }
    };

    let header_text = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();

    let mut headers = Vec::new();
    let mut content_length = 0usize;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim().to_string();
            let value = value.trim().to_string();
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.parse().unwrap_or(0);
            }
            headers.push((name, value));
        }
    }

    let body_start = header_end + 4;
    while buffer.len() < body_start + content_length {
        let read = stream.read(&mut chunk).unwrap();
        if read == 0 {
            panic!("client closed before sending the body");
        }
        buffer.extend_from_slice(&chunk[..read]);
    }

    ScriptedRequest {
        method,
        path,
        headers,
        body: buffer[body_start..body_start + content_length].to_vec(),
    }
}

fn find_headers_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn write_response(stream: &mut TcpStream, status: u16, content_type: &str, body: &[u8]) {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "Error",
    };
    let retry = if status == 200 {
        "Retry-After: 2\r\n"
    } else {
        ""
    };
    let headers = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\n{retry}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(headers.as_bytes()).unwrap();
    stream.write_all(body).unwrap();
    stream.flush().unwrap();
}

#[test]
fn backend_options_name_every_lane() {
    let options = backend_options();
    let ids: Vec<&str> = options.iter().map(|option| option.id.as_str()).collect();
    for id in ["blender-code", "local-command", "http", "meshy"] {
        assert!(ids.contains(&id), "missing backend {id}: {ids:?}");
    }
    let blender = options
        .iter()
        .find(|option| option.id == "blender-code")
        .expect("blender option");
    assert!(blender.stages.iter().any(|stage| stage == "reconstruct"));
    assert!(blender.setup.contains("mesh.py"));
    assert!(blender.setup.contains("model.glb"));
    let meshy = options
        .iter()
        .find(|option| option.id == "meshy")
        .expect("meshy option");
    assert!(!meshy.local);
    assert!(meshy.needs_key);
}

#[test]
fn scaffold_writes_the_blender_contract_and_refuses_overwrite() {
    let workspace = temp_workspace("scaffold");
    let path = scaffold_mesh_script(&workspace, "scaffold-test", false).expect("scaffold");
    let text = fs::read_to_string(&path).expect("read scaffold");
    assert!(text.contains("bpy"));
    assert!(text.contains("model.glb"));
    assert!(text.contains("DAH_CHARACTER_DIR"));
    assert!(text.contains("turnaround.png"));
    assert!(
        scaffold_mesh_script(&workspace, "scaffold-test", false).is_err(),
        "a second scaffold must not overwrite an edit"
    );
    assert!(scaffold_mesh_script(&workspace, "scaffold-test", true).is_ok());
    let _ = fs::remove_dir_all(&workspace);
}

#[test]
fn scaffold_rejects_an_unsafe_slug() {
    let workspace = temp_workspace("scaffold-slug");
    assert!(scaffold_mesh_script(&workspace, "../escape", false).is_err());
    let _ = fs::remove_dir_all(&workspace);
}
