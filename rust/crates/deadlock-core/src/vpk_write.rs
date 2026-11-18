//! VPK v2 writer: pack a staging tree into a directory file plus chunk files.
//!
//! Layout facts live in `docs/research/vpk-format.md` and the matching reader in
//! [`crate::vpk`]. The writer always stores entry bytes in chunk files
//! (`<base>_000.vpk` and up), never inline in the directory file, so the header
//! file data section stays empty. That stays the default.
//!
//! [`pack_vpk_single`] writes one self-contained `<base>_dir.vpk` instead: every entry inline
//! in the file data section, archive index `0x7fff`, no `_000` chunk. [`combine_chunks`]
//! rewrites an existing split archive that way, which is what a compatibility repair needs
//! (vpkmerge and Grimoire heal single-file archives only).

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::util::{temp_path, write_atomic};
use crate::vpk::{
    ReadVpkOptions, VPK_DIRECTORY_ARCHIVE, VPK_MAGIC, is_safe_vpk_path, read_vpk_directory_with,
    read_vpk_entry_data, try_chunk_path_for,
};

/// Chunk cap used when [`PackOptions::chunk_size`] is zero.
pub const DEFAULT_CHUNK_SIZE: u64 = 1024 * 1024 * 1024;

const HEADER_V2_SIZE: u32 = 28;
const ENTRY_TERMINATOR: u16 = 0xffff;
/// Indices `0..1000` keep the `<base>_NNN.vpk` three digit chunk name.
const MAX_CHUNKS: usize = 1000;
const READ_BUFFER_SIZE: usize = 64 * 1024;
const ROOT_DIRECTORY_MARKER: &str = " ";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PackOptions {
    pub staging_dir: PathBuf,
    pub output_dir: PathBuf,
    /// File name stem for the output. A trailing `.vpk` or `_dir.vpk` is removed.
    pub base_name: String,
    /// Chunk size cap in bytes. Never splits a file. Zero means
    /// [`DEFAULT_CHUNK_SIZE`].
    pub chunk_size: u64,
}

impl PackOptions {
    pub fn new(staging_dir: PathBuf, output_dir: PathBuf, base_name: impl Into<String>) -> Self {
        Self {
            staging_dir,
            output_dir,
            base_name: base_name.into(),
            chunk_size: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PackedVpk {
    pub dir_path: PathBuf,
    pub chunk_paths: Vec<PathBuf>,
    pub entry_count: usize,
    pub total_bytes: u64,
}

/// Recursively collect staging files as forward slash relative paths.
///
/// Files come back sorted. Empty folders are skipped, and anything that is not a
/// safe relative VPK path, contains a backslash, or is a symbolic link is an
/// error.
pub fn collect_staging_entries(staging: &Path) -> Result<Vec<String>> {
    if !staging.is_dir() {
        return Err(Error::msg(format!(
            "Staging folder {} does not exist or is not a folder.",
            staging.display()
        )));
    }

    let mut entries = Vec::new();
    walk_staging(staging, "", &mut entries)?;
    entries.sort_unstable();
    Ok(entries)
}

/// Pack a staging tree into `<base>_dir.vpk` plus `<base>_NNN.vpk` chunks.
pub fn pack_vpk(options: &PackOptions) -> Result<PackedVpk> {
    let paths = collect_staging_entries(&options.staging_dir)?;

    if paths.is_empty() {
        return Err(Error::msg(format!(
            "Staging folder {} has no files to pack.",
            options.staging_dir.display()
        )));
    }

    let base_name = sanitize_base_name(&options.base_name)?;
    let chunk_size = if options.chunk_size == 0 {
        DEFAULT_CHUNK_SIZE
    } else {
        options.chunk_size
    };

    fs::create_dir_all(&options.output_dir)?;

    let dir_path = options.output_dir.join(format!("{base_name}_dir.vpk"));

    let mut entries = Vec::with_capacity(paths.len());
    for path in paths {
        entries.push(PackedEntry::new(&options.staging_dir, path)?);
    }

    assign_chunks(&mut entries, chunk_size)?;
    let chunk_paths = write_chunks(&mut entries, &dir_path)?;

    let file = directory_bytes(&entries, 0)?;
    write_atomic(&dir_path, &file)?;

    let total_bytes = entries.iter().map(|entry| u64::from(entry.length)).sum();

    Ok(PackedVpk {
        dir_path,
        chunk_paths,
        entry_count: entries.len(),
        total_bytes,
    })
}

/// Header plus directory tree for `entries`, with `data_size` bytes of inline file data to follow.
fn directory_bytes(entries: &[PackedEntry], data_size: u32) -> Result<Vec<u8>> {
    let tree = build_tree(entries);
    let tree_size = u32::try_from(tree.len())
        .map_err(|_| Error::msg("VPK directory tree is larger than 4 GiB."))?;

    let mut file = Vec::with_capacity(HEADER_V2_SIZE as usize + tree.len());
    file.extend_from_slice(&VPK_MAGIC.to_le_bytes());
    file.extend_from_slice(&2_u32.to_le_bytes());
    file.extend_from_slice(&tree_size.to_le_bytes());
    file.extend_from_slice(&data_size.to_le_bytes()); // file data section
    file.extend_from_slice(&0_u32.to_le_bytes()); // archive MD5 section
    file.extend_from_slice(&0_u32.to_le_bytes()); // other MD5 section
    file.extend_from_slice(&0_u32.to_le_bytes()); // signature section
    file.extend_from_slice(&tree);
    Ok(file)
}

/// Pack a staging tree into one self-contained `<base>_dir.vpk`: every entry is stored inline
/// after the directory tree, so no `<base>_000.vpk` is written. `chunk_size` is ignored. The
/// payload must fit the 4 GiB offset limit of one VPK file.
pub fn pack_vpk_single(options: &PackOptions) -> Result<PackedVpk> {
    let paths = collect_staging_entries(&options.staging_dir)?;

    if paths.is_empty() {
        return Err(Error::msg(format!(
            "Staging folder {} has no files to pack.",
            options.staging_dir.display()
        )));
    }

    let base_name = sanitize_base_name(&options.base_name)?;
    fs::create_dir_all(&options.output_dir)?;
    let dir_path = options.output_dir.join(format!("{base_name}_dir.vpk"));

    let mut entries = Vec::with_capacity(paths.len());
    for path in paths {
        entries.push(PackedEntry::new(&options.staging_dir, path)?);
    }

    write_single_file(&dir_path, &mut entries, |entry, sink| {
        let mut source = File::open(&entry.source)?;
        let mut buffer = vec![0u8; READ_BUFFER_SIZE];
        loop {
            let count = source.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            sink(&buffer[..count])?;
        }
        Ok(())
    })?;

    let total_bytes = entries.iter().map(|entry| u64::from(entry.length)).sum();
    Ok(PackedVpk {
        dir_path,
        chunk_paths: Vec::new(),
        entry_count: entries.len(),
        total_bytes,
    })
}

/// Rewrite the archive behind `dir_vpk` (a split `<name>_dir.vpk` with `_NNN` chunks, or one
/// that is already single-file) as one self-contained VPK at `out_vpk`. Preload bytes are
/// folded into the data, so every entry reads back byte for byte the same with the same CRC.
/// `out_vpk` must not exist yet and must not be `dir_vpk`. Nothing beside `dir_vpk` changes.
pub fn combine_chunks(dir_vpk: &Path, out_vpk: &Path) -> Result<PackedVpk> {
    if out_vpk.exists() {
        return Err(Error::msg(format!(
            "{} already exists. Pick a new path for the combined archive.",
            out_vpk.display()
        )));
    }
    let directory = read_vpk_directory_with(dir_vpk, &ReadVpkOptions { keep_preload: true })?;
    if directory.entries.is_empty() {
        return Err(Error::msg(format!("{} has no entries to combine.", dir_vpk.display())));
    }

    let mut entries = Vec::with_capacity(directory.entries.len());
    for entry in &directory.entries {
        let mut packed = PackedEntry::named(entry.path.clone(), PathBuf::new())?;
        packed.length = entry.total_length;
        entries.push(packed);
    }
    let by_path: BTreeMap<&str, &crate::vpk::VpkEntry> =
        directory.entries.iter().map(|entry| (entry.path.as_str(), entry)).collect();

    if let Some(parent) = out_vpk.parent() {
        fs::create_dir_all(parent)?;
    }
    write_single_file(out_vpk, &mut entries, |packed, sink| {
        let entry = by_path
            .get(packed.path.as_str())
            .ok_or_else(|| Error::msg(format!("{} vanished while combining.", packed.path)))?;
        let data = read_vpk_entry_data(&directory, entry)?;
        sink(&data)
    })?;

    let total_bytes = entries.iter().map(|entry| u64::from(entry.length)).sum();
    Ok(PackedVpk {
        dir_path: out_vpk.to_path_buf(),
        chunk_paths: Vec::new(),
        entry_count: entries.len(),
        total_bytes,
    })
}

/// True when any entry of `dir_vpk` keeps its bytes in a `_NNN` chunk file. Reads the directory
/// only: chunk files left beside a single-file archive do not count.
pub fn is_split(dir_vpk: &Path) -> Result<bool> {
    let directory = read_vpk_directory_with(dir_vpk, &ReadVpkOptions { keep_preload: false })?;
    Ok(directory
        .entries
        .iter()
        .any(|entry| entry.length > 0 && entry.archive_index != VPK_DIRECTORY_ARCHIVE))
}

/// Write `entries` inline into one VPK at `path`. `read` streams each entry's bytes into the
/// sink it is given; lengths are checked and CRCs computed on the way. The data goes to a
/// temporary sibling first, then header, tree and data land in place with one rename.
fn write_single_file(
    path: &Path,
    entries: &mut [PackedEntry],
    mut read: impl FnMut(&PackedEntry, &mut dyn FnMut(&[u8]) -> Result<()>) -> Result<()>,
) -> Result<()> {
    let data_path = temp_path(&path.with_extension("data"));
    let result = (|| -> Result<()> {
        let mut writer = BufWriter::new(File::create(&data_path)?);
        let mut offset: u64 = 0;
        for entry in entries.iter_mut() {
            let mut hasher = crc32fast::Hasher::new();
            let mut written: u64 = 0;
            read(entry, &mut |bytes: &[u8]| {
                hasher.update(bytes);
                writer.write_all(bytes)?;
                written += bytes.len() as u64;
                Ok(())
            })?;
            if written != u64::from(entry.length) {
                return Err(Error::msg(format!(
                    "{} changed size while packing: expected {} bytes, read {}.",
                    entry.path, entry.length, written
                )));
            }
            entry.crc32 = hasher.finalize();
            entry.archive_index = VPK_DIRECTORY_ARCHIVE;
            entry.offset = u32::try_from(offset).map_err(|_| {
                Error::msg("A single-file VPK must stay under 4 GiB. Pack it in chunks instead.")
            })?;
            offset += written;
        }
        writer.flush()?;
        drop(writer);
        let data_size = u32::try_from(offset).map_err(|_| {
            Error::msg("A single-file VPK must stay under 4 GiB. Pack it in chunks instead.")
        })?;

        let head = directory_bytes(entries, data_size)?;
        let partial = temp_path(path);
        let written = (|| -> Result<()> {
            let mut out = BufWriter::new(File::create(&partial)?);
            out.write_all(&head)?;
            std::io::copy(&mut File::open(&data_path)?, &mut out)?;
            out.flush()?;
            Ok(())
        })();
        if let Err(error) = written.and_then(|()| fs::rename(&partial, path).map_err(Error::from)) {
            let _ = fs::remove_file(&partial);
            return Err(error);
        }
        Ok(())
    })();
    let _ = fs::remove_file(&data_path);
    result
}

struct PackedEntry {
    path: String,
    source: PathBuf,
    directory: String,
    name: String,
    extension: String,
    length: u32,
    crc32: u32,
    archive_index: u16,
    offset: u32,
}

impl PackedEntry {
    fn new(staging: &Path, path: String) -> Result<Self> {
        let source = staging.join(&path);
        let metadata = fs::metadata(&source)?;

        if !metadata.is_file() {
            return Err(Error::msg(format!("Staging entry {path} is not a file.")));
        }

        let length = u32::try_from(metadata.len()).map_err(|_| {
            Error::msg(format!(
                "Staging entry {path} is {} bytes, above the 4 GiB VPK entry limit.",
                metadata.len()
            ))
        })?;

        let mut entry = Self::named(path, source)?;
        entry.length = length;
        Ok(entry)
    }

    /// An entry for `path` with its name split the VPK way, length zero.
    fn named(path: String, source: PathBuf) -> Result<Self> {
        let (directory, file_name) = match path.rsplit_once('/') {
            Some((directory, file_name)) => (directory.to_string(), file_name),
            None => (String::new(), path.as_str()),
        };

        let (name, extension) = match split_file_name(file_name) {
            Some((name, extension)) => (name.to_string(), extension.to_string()),
            None => {
                return Err(Error::msg(format!(
                    "Staging entry {path} needs a file name before its extension."
                )));
            }
        };

        Ok(Self {
            source,
            directory,
            name,
            extension,
            length: 0,
            crc32: 0,
            archive_index: 0,
            offset: 0,
            path,
        })
    }
}

fn walk_staging(directory: &Path, prefix: &str, entries: &mut Vec<String>) -> Result<()> {
    for child in fs::read_dir(directory)? {
        let child = child?;
        let file_type = child.file_type()?;
        let raw_name = child.file_name();
        let Some(name) = raw_name.to_str() else {
            return Err(Error::msg(format!(
                "Staging entry {} has a name that is not valid UTF-8.",
                child.path().display()
            )));
        };

        let relative = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}/{name}")
        };

        if file_type.is_symlink() {
            return Err(Error::msg(format!(
                "Staging entry {relative} is a symbolic link. Pack real files only."
            )));
        }

        if file_type.is_dir() {
            walk_staging(&child.path(), &relative, entries)?;
            continue;
        }

        if !file_type.is_file() {
            continue;
        }

        entries.push(normalize_entry_path(&relative)?);
    }

    Ok(())
}

fn normalize_entry_path(relative: &str) -> Result<String> {
    if relative.contains('\\') {
        return Err(Error::msg(format!(
            "Staging entry {relative} contains a backslash. Rename it so the name uses forward slash path segments."
        )));
    }

    if !is_safe_vpk_path(relative) {
        return Err(Error::msg(format!(
            "Staging entry {relative} is not a safe VPK path. Use forward slashes, keep the path relative and avoid empty, current or parent segments."
        )));
    }

    Ok(relative.to_string())
}

/// Split a file name into stem and extension. No extension becomes `" "`, the
/// VPK marker for a name without a dot.
fn split_file_name(file_name: &str) -> Option<(&str, &str)> {
    match file_name.rfind('.') {
        Some(index) if index > 0 && index + 1 < file_name.len() => {
            Some((&file_name[..index], &file_name[index + 1..]))
        }
        Some(_) => None,
        None => Some((file_name, ROOT_DIRECTORY_MARKER)),
    }
}

fn sanitize_base_name(base_name: &str) -> Result<String> {
    let trimmed = base_name.trim();

    if trimmed.is_empty() {
        return Err(Error::msg(
            "A VPK base name is required, for example pak01.",
        ));
    }

    let lowered = trimmed.to_ascii_lowercase();
    let stem = if lowered.ends_with("_dir.vpk") {
        &trimmed[..trimmed.len() - "_dir.vpk".len()]
    } else if lowered.ends_with(".vpk") {
        &trimmed[..trimmed.len() - ".vpk".len()]
    } else {
        trimmed
    };

    if stem.is_empty() {
        return Err(Error::msg(format!(
            "VPK base name {base_name} is empty after removing the .vpk suffix."
        )));
    }

    if stem.contains('/')
        || stem.contains('\\')
        || stem.contains("..")
        || stem.chars().any(|character| {
            character == '\0' || matches!(character, ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
    {
        return Err(Error::msg(format!(
            "VPK base name {base_name} must be a plain file name, not a path."
        )));
    }

    Ok(stem.to_string())
}

fn assign_chunks(entries: &mut [PackedEntry], chunk_size: u64) -> Result<()> {
    let mut chunk_index: usize = 0;
    let mut offset: u64 = 0;

    for entry in entries.iter_mut() {
        let length = u64::from(entry.length);

        if offset > 0 && offset + length > chunk_size {
            chunk_index += 1;
            offset = 0;
        }

        if chunk_index >= MAX_CHUNKS {
            return Err(Error::msg(format!(
                "Staged payload needs more than {MAX_CHUNKS} VPK chunks. Raise the chunk size."
            )));
        }

        if offset + length > u64::from(u32::MAX) {
            return Err(Error::msg(format!(
                "VPK chunk {chunk_index} would pass the 4 GiB entry offset limit. Raise the chunk size."
            )));
        }

        entry.archive_index = u16::try_from(chunk_index)
            .map_err(|_| Error::msg("VPK chunk index does not fit the entry format."))?;
        entry.offset = offset as u32;
        offset += length;
    }

    Ok(())
}

fn write_chunks(entries: &mut [PackedEntry], dir_path: &Path) -> Result<Vec<PathBuf>> {
    let mut groups: BTreeMap<u16, Vec<usize>> = BTreeMap::new();
    for (index, entry) in entries.iter().enumerate() {
        groups.entry(entry.archive_index).or_default().push(index);
    }

    let mut chunk_paths = Vec::with_capacity(groups.len());

    for (archive_index, indices) in groups {
        let chunk_path = try_chunk_path_for(dir_path, archive_index)?;
        let temporary = temp_path(&chunk_path);

        if let Err(error) = write_chunk(&temporary, entries, &indices) {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }

        if let Err(error) = fs::rename(&temporary, &chunk_path) {
            let _ = fs::remove_file(&temporary);
            return Err(error.into());
        }

        chunk_paths.push(chunk_path);
    }

    Ok(chunk_paths)
}

fn write_chunk(path: &Path, entries: &mut [PackedEntry], indices: &[usize]) -> Result<()> {
    let mut writer = BufWriter::new(File::create(path)?);
    let mut buffer = vec![0u8; READ_BUFFER_SIZE];

    for &index in indices {
        let mut source = File::open(&entries[index].source)?;
        let mut hasher = crc32fast::Hasher::new();
        let mut written = 0_u64;

        loop {
            let count = source.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
            writer.write_all(&buffer[..count])?;
            written += count as u64;
        }

        if written != u64::from(entries[index].length) {
            return Err(Error::msg(format!(
                "{} changed size while packing: expected {} bytes, read {}.",
                entries[index].path, entries[index].length, written
            )));
        }

        entries[index].crc32 = hasher.finalize();
    }

    writer.flush()?;
    Ok(())
}

fn build_tree(entries: &[PackedEntry]) -> Vec<u8> {
    let mut extensions: BTreeMap<&str, BTreeMap<&str, Vec<&PackedEntry>>> = BTreeMap::new();

    for entry in entries {
        extensions
            .entry(entry.extension.as_str())
            .or_default()
            .entry(entry.directory.as_str())
            .or_default()
            .push(entry);
    }

    let mut tree = Vec::new();

    for (extension, directories) in extensions {
        write_cstring(&mut tree, extension);

        for (directory, files) in directories {
            write_cstring(
                &mut tree,
                if directory.is_empty() {
                    ROOT_DIRECTORY_MARKER
                } else {
                    directory
                },
            );

            for entry in files {
                write_cstring(&mut tree, &entry.name);
                tree.extend_from_slice(&entry.crc32.to_le_bytes());
                tree.extend_from_slice(&0_u16.to_le_bytes()); // preload bytes
                tree.extend_from_slice(&entry.archive_index.to_le_bytes());
                tree.extend_from_slice(&entry.offset.to_le_bytes());
                tree.extend_from_slice(&entry.length.to_le_bytes());
                tree.extend_from_slice(&ENTRY_TERMINATOR.to_le_bytes());
            }

            write_cstring(&mut tree, "");
        }

        write_cstring(&mut tree, "");
    }

    write_cstring(&mut tree, "");
    tree
}

fn write_cstring(output: &mut Vec<u8>, value: &str) {
    output.extend_from_slice(value.as_bytes());
    output.push(0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vpk::{crc32, read_vpk_directory, read_vpk_entry_data};

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dah-vpk-write-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn packs_a_staging_tree_into_chunks_and_reads_it_back() {
        let root = temp_root("roundtrip");
        let staging = root.join("staging");

        fs::create_dir_all(staging.join("models/heroes_wip/billy")).unwrap();
        fs::create_dir_all(staging.join("materials")).unwrap();
        fs::create_dir_all(staging.join("empty_folder")).unwrap();

        fs::write(
            staging.join("models/heroes_wip/billy/billy.vmdl_c"),
            b"model-bytes",
        )
        .unwrap();
        fs::write(staging.join("materials/hero.vmat_c"), b"material").unwrap();
        fs::write(staging.join("materials/empty.vmat_c"), b"").unwrap();

        let collected = collect_staging_entries(&staging).unwrap();
        assert_eq!(
            collected,
            vec![
                "materials/empty.vmat_c",
                "materials/hero.vmat_c",
                "models/heroes_wip/billy/billy.vmdl_c",
            ]
        );

        let output = root.join("out");
        let packed = pack_vpk(&PackOptions {
            staging_dir: staging.clone(),
            output_dir: output.clone(),
            base_name: "pak01".to_string(),
            chunk_size: 8,
        })
        .unwrap();

        assert_eq!(packed.dir_path, output.join("pak01_dir.vpk"));
        assert_eq!(packed.entry_count, 3);
        assert_eq!(packed.total_bytes, 11 + 8);
        assert_eq!(packed.chunk_paths.len(), 2);
        assert_eq!(packed.chunk_paths[0], output.join("pak01_000.vpk"));
        assert_eq!(packed.chunk_paths[1], output.join("pak01_001.vpk"));
        assert_eq!(
            fs::metadata(&packed.chunk_paths[0]).unwrap().len(),
            8,
            "a file is never split across chunks"
        );
        assert_eq!(fs::metadata(&packed.chunk_paths[1]).unwrap().len(), 11);

        let directory = read_vpk_directory(&packed.dir_path).unwrap();
        assert_eq!(directory.header.version, 2);
        assert!(directory.header.tree_size > 0);
        assert_eq!(directory.header.file_data_section_size, 0);
        assert_eq!(directory.entries.len(), 3);

        for (path, expected) in [
            (
                "models/heroes_wip/billy/billy.vmdl_c",
                b"model-bytes".as_slice(),
            ),
            ("materials/hero.vmat_c", b"material".as_slice()),
            ("materials/empty.vmat_c", b"".as_slice()),
        ] {
            let entry = directory.by_path.get(path).unwrap();
            let data = read_vpk_entry_data(&directory, entry).unwrap();
            assert_eq!(data, expected, "{path}");
            assert_eq!(entry.crc32, crc32(&data), "{path}");
            assert_eq!(entry.preload_bytes, 0, "{path}");
            assert!(entry.archive_index < crate::vpk::VPK_DIRECTORY_ARCHIVE);
        }

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_empty_staging_unsafe_base_names_and_symlinks() {
        let root = temp_root("reject");
        let staging = root.join("staging");
        fs::create_dir_all(&staging).unwrap();

        let empty = pack_vpk(&PackOptions::new(
            staging.clone(),
            root.join("out"),
            "pak01",
        ));
        assert!(empty.is_err(), "empty staging must not pack");

        fs::create_dir_all(staging.join("materials")).unwrap();
        fs::write(staging.join("materials/hero.vmat_c"), b"hero").unwrap();

        let unsafe_name = pack_vpk(&PackOptions {
            staging_dir: staging.clone(),
            output_dir: root.join("out"),
            base_name: "nested/pak01".to_string(),
            chunk_size: 0,
        });
        assert!(
            unsafe_name
                .unwrap_err()
                .to_string()
                .contains("plain file name")
        );

        let usable_base = pack_vpk(&PackOptions {
            staging_dir: staging.clone(),
            output_dir: root.join("out"),
            base_name: "pak01_dir.vpk".to_string(),
            chunk_size: 0,
        });
        assert_eq!(
            usable_base.unwrap().dir_path,
            root.join("out").join("pak01_dir.vpk")
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let backslash = root.join("staging_backslash");
            fs::create_dir_all(backslash.join("materials")).unwrap();
            fs::write(backslash.join("materials/odd\\name.vmat_c"), b"odd").unwrap();
            let rejected = collect_staging_entries(&backslash).unwrap_err();
            assert!(rejected.to_string().contains("backslash"));

            let linked = root.join("staging_link");
            fs::create_dir_all(linked.join("materials")).unwrap();
            fs::write(linked.join("materials/hero.vmat_c"), b"hero").unwrap();
            symlink(
                root.join("outside.bin"),
                linked.join("materials/link.vmat_c"),
            )
            .unwrap();
            let rejected = collect_staging_entries(&linked).unwrap_err();
            assert!(rejected.to_string().contains("symbolic link"));
        }

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn single_file_packs_and_combined_chunks_read_back_the_same() {
        let root = temp_root("single");
        let staging = root.join("staging");
        fs::create_dir_all(staging.join("models/heroes_wip/billy")).unwrap();
        fs::create_dir_all(staging.join("materials")).unwrap();
        fs::write(staging.join("models/heroes_wip/billy/billy.vmdl_c"), b"model-bytes").unwrap();
        fs::write(staging.join("materials/hero.vmat_c"), b"material").unwrap();
        fs::write(staging.join("materials/empty.vmat_c"), b"").unwrap();
        fs::write(staging.join("readme"), b"no extension").unwrap();

        let expected = [
            ("models/heroes_wip/billy/billy.vmdl_c", b"model-bytes".as_slice()),
            ("materials/hero.vmat_c", b"material".as_slice()),
            ("materials/empty.vmat_c", b"".as_slice()),
        ];
        let check = |dir: &Path| {
            let directory = read_vpk_directory(dir).unwrap();
            assert_eq!(directory.entries.len(), 4);
            for (path, bytes) in expected {
                let entry = directory.by_path.get(path).unwrap();
                let data = read_vpk_entry_data(&directory, entry).unwrap();
                assert_eq!(data, bytes, "{path}");
                assert_eq!(entry.crc32, crc32(&data), "{path}");
                assert_eq!(entry.archive_index, VPK_DIRECTORY_ARCHIVE, "{path}");
            }
            assert_eq!(directory.header.file_data_section_size, 11 + 8 + 12);
        };

        let single = pack_vpk_single(&PackOptions::new(staging.clone(), root.join("one"), "pak07"))
            .unwrap();
        assert!(single.chunk_paths.is_empty());
        assert!(!root.join("one").join("pak07_000.vpk").exists());
        assert_eq!(fs::read_dir(root.join("one")).unwrap().count(), 1, "no temporary file is left");
        check(&single.dir_path);
        assert!(!is_split(&single.dir_path).unwrap());

        let split = pack_vpk(&PackOptions {
            staging_dir: staging.clone(),
            output_dir: root.join("split"),
            base_name: "pak07".to_string(),
            chunk_size: 8,
        })
        .unwrap();
        assert!(split.chunk_paths.len() > 1);
        assert!(is_split(&split.dir_path).unwrap());

        let out = root.join("combined").join("pak07_dir.vpk");
        let combined = combine_chunks(&split.dir_path, &out).unwrap();
        assert_eq!(combined.entry_count, 4);
        check(&out);
        assert!(!is_split(&out).unwrap());
        assert!(split.chunk_paths.iter().all(|chunk| chunk.is_file()), "the source is untouched");
        assert!(combine_chunks(&split.dir_path, &out).is_err(), "an existing output is refused");

        let _ = fs::remove_dir_all(&root);
    }
}
