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
