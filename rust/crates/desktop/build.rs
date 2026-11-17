//! Embeds the packed Locker art so Sigil ships as one executable.
//!
//! `SIGIL_ART_DIR` names a folder written by `scripts/pack-locker-art.sh` (heroes/, names/,
//! hero_portraits/, global-bg.webp, the launch art). Without it the build looks at
//! `.scratch/locker-art` in the repository. When neither exists the table is empty and the app
//! falls back to the art it finds on disk (the Grimoire checkout during development).

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=SIGIL_ART_DIR");
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let default = manifest.join("../../../.scratch/locker-art");
    let root = env::var_os("SIGIL_ART_DIR").map(PathBuf::from).unwrap_or(default);
    println!("cargo:rerun-if-changed={}", root.display());

    let mut files = Vec::new();
    if root.is_dir() {
        collect(&root, &root, &mut files);
    }
    files.sort();

    let mut hasher = DefaultHasher::new();
    let mut table = String::from("pub static ART: &[(&str, &[u8])] = &[\n");
    for (relative, path) in &files {
        println!("cargo:rerun-if-changed={}", path.display());
        let bytes = fs::read(path).expect("read art");
        relative.hash(&mut hasher);
        bytes.hash(&mut hasher);
        writeln!(table, "    ({relative:?}, include_bytes!({:?})),", path.display().to_string())
            .unwrap();
    }
    table.push_str("];\n");
    writeln!(table, "pub const ART_ID: &str = \"{:016x}\";", hasher.finish()).unwrap();

    let out = PathBuf::from(env::var_os("OUT_DIR").expect("out dir"));
    fs::write(out.join("embedded_art.rs"), table).expect("write art table");

    // Windows: the app icon as resource 1, linked into Sigil.exe (`brand::install_app_icon`).
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let ico = manifest.join("assets/brand/sigil.ico");
        println!("cargo:rerun-if-changed={}", ico.display());
        let res = out.join("sigil-icon.res");
        let bytes = fs::read(&ico).expect("read sigil.ico");
        fs::write(&res, icon_res(&bytes)).expect("write sigil-icon.res");
        println!("cargo:rustc-link-arg-bins={}", res.display());
    }
}

/// A compiled resource file (`.res`) holding an `.ico` as icon group 1: each image an `RT_ICON`
/// (ids 1..n) and the directory an `RT_GROUP_ICON`. Linkers (link.exe, lld-link) take a `.res`
/// as input, so no resource compiler is needed.
fn icon_res(ico: &[u8]) -> Vec<u8> {
    const RT_ICON: u16 = 3;
    const RT_GROUP_ICON: u16 = 14;
    let u16_at = |at: usize| u16::from_le_bytes([ico[at], ico[at + 1]]);
    let u32_at = |at: usize| u32::from_le_bytes(ico[at..at + 4].try_into().unwrap());
    assert!(u16_at(0) == 0 && u16_at(2) == 1, "sigil.ico is not an icon file");
    let count = u16_at(4) as usize;

    fn entry(out: &mut Vec<u8>, kind: u16, id: u16, flags: u16, data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_le_bytes()); // DataSize
        out.extend_from_slice(&32u32.to_le_bytes()); // HeaderSize
        out.extend_from_slice(&[0xff, 0xff]);
        out.extend_from_slice(&kind.to_le_bytes()); // TYPE by ordinal
        out.extend_from_slice(&[0xff, 0xff]);
        out.extend_from_slice(&id.to_le_bytes()); // NAME by ordinal
        out.extend_from_slice(&0u32.to_le_bytes()); // DataVersion
        out.extend_from_slice(&flags.to_le_bytes()); // MemoryFlags
        out.extend_from_slice(&0x0409u16.to_le_bytes()); // LanguageId: en-US
        out.extend_from_slice(&0u32.to_le_bytes()); // Version
        out.extend_from_slice(&0u32.to_le_bytes()); // Characteristics
        out.extend_from_slice(data);
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }

    // The empty first entry every .res starts with.
    let mut out = vec![0u8; 32];
    out[4] = 0x20;
    out[8..12].copy_from_slice(&[0xff, 0xff, 0, 0]);
    out[12..16].copy_from_slice(&[0xff, 0xff, 0, 0]);

    let mut group = Vec::new();
    group.extend_from_slice(&0u16.to_le_bytes());
    group.extend_from_slice(&1u16.to_le_bytes());
    group.extend_from_slice(&(count as u16).to_le_bytes());
    for i in 0..count {
        let at = 6 + i * 16;
        let size = u32_at(at + 8) as usize;
        let offset = u32_at(at + 12) as usize;
        let id = (i + 1) as u16;
        // MOVEABLE | PURE | DISCARDABLE, as rc.exe writes icons.
        entry(&mut out, RT_ICON, id, 0x1030, &ico[offset..offset + size]);
        group.extend_from_slice(&ico[at..at + 12]); // width, height, colors, reserved, planes, bits, bytes
        group.extend_from_slice(&id.to_le_bytes());
    }
    entry(&mut out, RT_GROUP_ICON, 1, 0x1030, &group);
    out
}

/// Every art file under `dir`, as a `/`-separated path relative to `root`.
fn collect(root: &Path, dir: &Path, files: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            println!("cargo:rerun-if-changed={}", path.display());
            collect(root, &path, files);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("webp" | "png" | "jpg" | "jpeg")
        ) {
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            files.push((relative, path));
        }
    }
}
