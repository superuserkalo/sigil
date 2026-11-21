use std::fs;
use std::path::PathBuf;

use morphic::kv3::Value;

use super::compare::{Change, LineOp, compare_trees, diff_lines};
use super::export::{ExportMode, editable_path, export};
use super::preview::{PreviewKind, decoded_text, hex_dump, panorama_text, preview};
use super::search::{SearchControl, SearchOptions, search};
use super::{dmx, kv3_text, open};
use crate::vpk_write::{PackOptions, pack_vpk_single};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sigil-explorer-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(root: &std::path::Path, path: &str, bytes: &[u8]) {
    let target = root.join(path);
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(target, bytes).unwrap();
}

/// A minimal compiled resource with one DATA block holding `data`.
fn resource(data: &[u8]) -> Vec<u8> {
    let mut file = vec![0u8; 16 + 12];
    file[4..6].copy_from_slice(&12u16.to_le_bytes());
    file[8..12].copy_from_slice(&8u32.to_le_bytes());
    file[12..16].copy_from_slice(&1u32.to_le_bytes());
    file[16..20].copy_from_slice(b"DATA");
    // Offset is relative to the offset field (byte 20): data starts at 28.
    file[20..24].copy_from_slice(&8u32.to_le_bytes());
    file[24..28].copy_from_slice(&(data.len() as u32).to_le_bytes());
    file.extend_from_slice(data);
    let size = file.len() as u32;
    file[0..4].copy_from_slice(&size.to_le_bytes());
    file
}

fn kv3_resource(value: &Value) -> Vec<u8> {
    resource(&morphic::kv3::encode(value, &morphic::kv3::Format([0; 16])))
}

fn sample_value() -> Value {
    Value::Object(vec![
        ("generic_data_type".to_string(), Value::String("hero_data".to_string())),
        ("m_flSpeed".to_string(), Value::Double(6.5)),
        ("m_nCount".to_string(), Value::Int(3)),
        ("m_list".to_string(), Value::Array(vec![Value::Int(1), Value::Int(2)])),
    ])
}

#[test]
fn folder_listing_groups_children_and_reads() {
    let root = scratch("folder");
    write(&root, "panorama/images/hud/a.png", b"png");
    write(&root, "panorama/images/hud/b.png", b"png2");
    write(&root, "models/x.vmdl_c", b"model");
    let listing = open(&root).unwrap();
    assert_eq!(listing.files.len(), 3);
    let top = listing.children("");
    assert_eq!(top.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(), ["models", "panorama"]);
    assert_eq!(top[1].files, 2);
    assert_eq!(top[1].size, 7);
    let hud = listing.children("panorama/images/hud");
    assert_eq!(hud.len(), 2);
    assert!(!hud[0].folder);
    assert_eq!(listing.read("models/x.vmdl_c").unwrap(), b"model");
    assert!(listing.read("../etc/passwd").is_err());
    assert_eq!(listing.files_under("panorama").len(), 2);
}

#[test]
fn vpk_listing_reads_entries() {
    let root = scratch("vpk");
    let staging = root.join("staging");
    write(&staging, "scripts/a.txt", b"hello world\nsecond line");
    let packed = pack_vpk_single(&PackOptions::new(staging.clone(), root.clone(), "pak01")).unwrap();
    let listing = open(&packed.dir_path).unwrap();
    assert_eq!(listing.kind, super::SourceKind::Vpk);
    assert!(listing.files[0].crc32.is_some());
    assert_eq!(listing.read("scripts/a.txt").unwrap(), b"hello world\nsecond line");
}

#[test]
fn kv3_text_writes_a_readable_tree() {
    let text = kv3_text::to_text(&sample_value());
    assert!(text.starts_with("<!-- kv3 encoding:text"));
    assert!(text.contains("generic_data_type = \"hero_data\""));
    assert!(text.contains("m_flSpeed = 6.5"));
    assert!(text.contains("m_list = [ 1, 2 ]"));
}

#[test]
fn compiled_data_decodes_to_kv3_text() {
    let bytes = kv3_resource(&sample_value());
    let shown = preview("scripts/heroes.vdata_c", &bytes);
    assert_eq!(shown.kind, PreviewKind::Data);
    assert!(shown.text.contains("m_nCount = 3"), "{}", shown.text);
    assert_eq!(editable_path("scripts/heroes.vdata_c").as_deref(), Some("scripts/heroes.vdata"));
    assert_eq!(editable_path("materials/x.vmat_c").as_deref(), Some("materials/x.vmat.kv3"));
    assert_eq!(editable_path("materials/x.vtex_c"), None);
}

#[test]
fn legacy_panorama_block_yields_source_text() {
    let source = b"<root>\n  <Panel class=\"hud\" />\n</root>";
    let mut data = Vec::new();
    data.extend_from_slice(&crate::vpk::crc32(source).to_le_bytes());
    data.extend_from_slice(&1u16.to_le_bytes());
    data.extend_from_slice(b"file://{resources}/layout/hud.xml\0");
    data.extend_from_slice(&0u32.to_le_bytes());
    data.extend_from_slice(source);
    data.push(0);
    let bytes = resource(&data);
    assert_eq!(panorama_text(&bytes).unwrap(), String::from_utf8_lossy(source));
    let shown = preview("panorama/layout/hud.vxml_c", &bytes);
    assert_eq!(shown.kind, PreviewKind::Panorama);
    assert_eq!(shown.format, "Panorama layout");
}

#[test]
fn text_files_and_binary_files_preview() {
    assert_eq!(preview("cfg/x.cfg", b"bind x y").kind, PreviewKind::Text);
    let utf16: Vec<u8> = [0xFF, 0xFE].into_iter().chain("hi".encode_utf16().flat_map(u16::to_le_bytes)).collect();
    assert_eq!(decoded_text("a.txt", &utf16).unwrap().text, "hi");
    let binary = preview("x.bin", &[0, 1, 2, 3, 255]);
    assert_eq!(binary.kind, PreviewKind::Binary);
    assert!(hex_dump(b"AB").starts_with("00000000  41 42"));
    assert_eq!(preview("a.png", b"\x89PNG").kind, PreviewKind::Image);
}

const DMX: &str = r#"<!-- dmx encoding keyvalues2 1 format model 22 -->
"DmElement"
{
	"id" "elementid" "root-id"
	"name" "string" "root"
	"model" "DmeModel"
	{
		"id" "elementid" "model-id"
		"name" "string" "body"
		"children" "element_array"
		[
			"DmeJoint"
			{
				"id" "elementid" "j1"
				"name" "string" "pelvis"
				"children" "element_array"
				[
					"DmeJoint"
					{
						"id" "elementid" "j2"
						"name" "string" "spine_0"
						"children" "element_array" [ ]
					},
					"DmeDag"
					{
						"id" "elementid" "dag"
						"name" "string" "mesh_dag"
						"shape" "element" "mesh-id"
					}
				]
			}
		]
	}
	"axisSystem" "DmeAxisSystem"
	{
		"id" "elementid" "axis"
		"upAxis" "int" "3"
	}
	"meshes" "element_array"
	[
		"DmeMesh"
		{
			"id" "elementid" "mesh-id"
			"name" "string" "body_mesh"
			"bindState" "DmeVertexData"
			{
				"id" "elementid" "vd"
				"position$0" "vector3_array" [ "0 0 0", "1 0 0", "0 1 0", "1 1 0" ]
			}
			"faceSets" "element_array"
			[
				"DmeFaceSet"
				{
					"id" "elementid" "fs"
					"faces" "int_array" [ "0", "1", "2", "-1", "1", "3", "2", "-1" ]
					"material" "DmeMaterial"
					{
						"id" "elementid" "mat"
						"mtlName" "string" "materials/body.vmat"
					}
				}
			]
		}
	]
}
"#;

#[test]
fn text_dmx_summarizes_meshes_and_joints() {
    let document = dmx::parse(DMX).unwrap();
    let summary = dmx::summarize(&document);
    assert_eq!(summary.format, "model 22");
    assert_eq!(summary.meshes.len(), 1);
    assert_eq!(summary.meshes[0].name, "body_mesh");
    assert_eq!(summary.meshes[0].vertices, 4);
    assert_eq!(summary.meshes[0].faces, 2);
    assert_eq!(summary.meshes[0].materials, ["materials/body.vmat"]);
    assert_eq!(summary.joints, vec![(0, "pelvis".to_string()), (1, "spine_0".to_string())]);
    assert_eq!(summary.up_axis.as_deref(), Some("Z"));
    let shown = preview("models/body.dmx", DMX.as_bytes());
    assert_eq!(shown.kind, PreviewKind::Dmx);
    assert!(shown.text.contains("// Mesh body_mesh: 4 vertices, 2 faces"));
    assert!(dmx::parse("<!-- dmx encoding binary 2 format model 1 -->").is_err());
}

#[test]
fn content_search_finds_lines_in_text_and_compiled_data() {
    let root = scratch("search");
    write(&root, "scripts/a.txt", b"one\nTwo Words here\nthree two");
    write(&root, "scripts/heroes.vdata_c", &kv3_resource(&sample_value()));
    write(&root, "materials/x.vtex_c", b"pixels two");
    let listing = open(&root).unwrap();
    let control = SearchControl::default();
    let report = search(&listing, &SearchOptions::new("two"), &control, &|| {}).unwrap();
    assert_eq!(report.files_matched, 1);
    assert_eq!(report.hits.len(), 2);
    assert_eq!(report.hits[0].line, 2);
    assert_eq!(&report.hits[0].text[report.hits[0].start..report.hits[0].end], "Two");
    assert_eq!(report.files_skipped, 1);
    let report = search(&listing, &SearchOptions::new("hero_data"), &control, &|| {}).unwrap();
    assert_eq!(report.hits[0].path, "scripts/heroes.vdata_c");
    assert_eq!(control.hit_count(), 1);
    let mut capped = SearchOptions::new("o");
    capped.max_hits = 1;
    assert!(search(&listing, &capped, &control, &|| {}).unwrap().capped);
    control.stop();
    assert!(search(&listing, &SearchOptions::new("two"), &control, &|| {}).unwrap().stopped);
}

#[test]
fn tree_and_line_diffs() {
    let left_root = scratch("cmp-left");
    let right_root = scratch("cmp-right");
    write(&left_root, "a.txt", b"same");
    write(&right_root, "a.txt", b"same");
    write(&left_root, "b.txt", b"left");
    write(&right_root, "b.txt", b"rght");
    write(&left_root, "only_left.txt", b"x");
    write(&right_root, "only_right.txt", b"y");
    let diff = compare_trees(&open(&left_root).unwrap(), &open(&right_root).unwrap());
    assert_eq!((diff.same, diff.changed, diff.only_left, diff.only_right), (1, 1, 1, 1));
    assert_eq!(diff.rows[0].change, Change::Changed);

    let lines = diff_lines("a\nb\nc\nd", "a\nc\nd\ne");
    let ops: Vec<LineOp> = lines.iter().map(|l| l.op).collect();
    assert_eq!(ops, [LineOp::Same, LineOp::Removed, LineOp::Same, LineOp::Same, LineOp::Added]);
    assert_eq!(lines[4].right, Some(4));
    let replaced = diff_lines("x\ny", "p\nq\nr");
    assert_eq!(replaced.iter().filter(|l| l.op == LineOp::Removed).count(), 2);
    assert_eq!(replaced.iter().filter(|l| l.op == LineOp::Added).count(), 3);
}

#[test]
fn export_writes_into_a_project_only() {
    let source = scratch("export-src");
    write(&source, "scripts/heroes.vdata_c", &kv3_resource(&sample_value()));
    write(&source, "materials/x.vtex_c", b"pixels");
    let listing = open(&source).unwrap();
    let not_project = scratch("export-none");
    assert!(export(&listing, &["scripts".to_string()], &not_project, ExportMode::Original, false).is_err());

    let project = scratch("export-project");
    crate::project::create_project(&project, "Test", "test").unwrap();
    let report = export(&listing, &["".to_string()], &project, ExportMode::Original, false).unwrap();
    assert_eq!(report.written.len(), 2);
    assert!(project.join("staging/scripts/heroes.vdata_c").is_file());
    assert_eq!(report.new_targets.len(), 2);
    let manifest = crate::project::read_project(&project).unwrap();
    assert!(manifest.targets.contains(&"scripts/heroes.vdata_c".to_string()));
    let again = export(&listing, &["scripts".to_string()], &project, ExportMode::Original, false).unwrap();
    assert_eq!(again.skipped.len(), 1);

    let editable = export(&listing, &["".to_string()], &project, ExportMode::Editable, false).unwrap();
    assert_eq!(editable.written.len(), 1);
    assert_eq!(editable.skipped.len(), 1);
    let text = fs::read_to_string(project.join("sources/scripts/heroes.vdata")).unwrap();
    assert!(text.contains("hero_data"));
}
