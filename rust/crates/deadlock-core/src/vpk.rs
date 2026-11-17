//! VPK v1/v2 container reader.
//!
//! Layout facts live in `docs/research/vpk-format.md`. The directory tree holds
//! entry metadata. Entry bytes live either in the directory file itself or in
//! numbered chunk files next to it.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

pub const VPK_MAGIC: u32 = 0x55aa_1234;
pub const VPK_DIRECTORY_ARCHIVE: u16 = 0x7fff;

const HEADER_V1_SIZE: u32 = 12;
const HEADER_V2_SIZE: u32 = 28;
const ENTRY_METADATA_SIZE: usize = 18;
const ENTRY_TERMINATOR: u16 = 0xffff;
const MAX_TREE_BYTES: u32 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VpkHeader {
    pub version: u32,
    pub header_size: u32,
    pub tree_size: u32,
    pub file_data_section_size: u32,
    pub archive_md5_section_size: u32,
    pub other_md5_section_size: u32,
    pub signature_section_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VpkEntry {
    /// Full game path with forward slashes, for example `models/heroes_wip/billy/billy.vmdl_c`.
    pub path: String,
    pub crc32: u32,
    /// Bytes stored in the directory tree ahead of the file data.
    #[serde(skip)]
    pub preload: Vec<u8>,
    pub preload_bytes: u16,
    /// `VPK_DIRECTORY_ARCHIVE` when the data is in the directory file.
    pub archive_index: u16,
    /// Data offset. Relative to the file data section for the directory file, else the chunk start.
    pub offset: u32,
    /// Bytes in the archive, excluding preload bytes.
    pub length: u32,
    pub total_length: u32,
}

#[derive(Debug, Clone)]
pub struct VpkDirectory {
    pub path: PathBuf,
    pub header: VpkHeader,
    pub entries: Vec<VpkEntry>,
    /// Entries keyed by the lowercased path.
    pub by_path: HashMap<String, VpkEntry>,
    pub skipped: u64,
    pub duplicates: u64,
}

#[derive(Debug, Clone)]
pub struct ReadVpkOptions {
    /// Keep preload bytes in memory. Indexing passes false; extraction needs them.
    pub keep_preload: bool,
}

impl Default for ReadVpkOptions {
    fn default() -> Self {
        Self { keep_preload: true }
    }
}

pub fn read_vpk_directory(path: &Path) -> Result<VpkDirectory> {
    read_vpk_directory_with(path, &ReadVpkOptions::default())
}

pub fn read_vpk_directory_with(path: &Path, options: &ReadVpkOptions) -> Result<VpkDirectory> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();

    if size < u64::from(HEADER_V1_SIZE) {
        return Err(Error::msg(format!(
            "{} is too small to be a VPK file.",
            path.display()
        )));
    }

    let header = read_header(&mut file, size, path)?;

    if header.tree_size > MAX_TREE_BYTES {
        return Err(Error::msg(format!(
            "{} has a {} byte tree, above the {} byte limit.",
            path.display(),
            header.tree_size,
            MAX_TREE_BYTES
        )));
    }

    let tree_start = u64::from(header.header_size);
    let tree_end = tree_start + u64::from(header.tree_size);

    if tree_end > size {
        return Err(Error::msg(format!(
            "{} claims a {} byte tree but only {} bytes remain.",
            path.display(),
            header.tree_size,
            size.saturating_sub(tree_start)
        )));
    }

    let mut tree = vec![0u8; header.tree_size as usize];
    read_exact_at(&mut file, &mut tree, tree_start, path)?;

    let parsed = parse_tree(&tree, options.keep_preload)?;

    Ok(VpkDirectory {
        path: path.to_path_buf(),
        header,
        entries: parsed.entries,
        by_path: parsed.by_path,
        skipped: parsed.skipped,
        duplicates: parsed.duplicates,
    })
}

pub fn read_vpk_entry_data(directory: &VpkDirectory, entry: &VpkEntry) -> Result<Vec<u8>> {
    if entry.preload.len() != usize::from(entry.preload_bytes) {
        return Err(Error::msg(format!(
            "{}: preload bytes were not kept. Re-read the VPK with preload enabled.",
            entry.path
        )));
    }

    let total = usize::try_from(entry.total_length)
        .map_err(|_| Error::msg(format!("{}: entry is too large to read.", entry.path)))?;
    let mut output = vec![0u8; total];
    output[..entry.preload.len()].copy_from_slice(&entry.preload);

    if entry.length == 0 {
        return Ok(output);
    }

    if entry.archive_index > VPK_DIRECTORY_ARCHIVE {
        return Err(Error::msg(format!(
            "{}: archive index {} is not a chunk file.",
            entry.path, entry.archive_index
        )));
    }

    let (source, offset) = if entry.archive_index == VPK_DIRECTORY_ARCHIVE {
        (
            directory.path.clone(),
            u64::from(directory.header.header_size)
                + u64::from(directory.header.tree_size)
                + u64::from(entry.offset),
        )
    } else {
        (
            chunk_path_for(&directory.path, entry.archive_index),
            u64::from(entry.offset),
        )
    };

    let mut file = File::open(&source)?;
    let source_size = file.metadata()?.len();
    let end = offset + u64::from(entry.length);

    if end > source_size {
        return Err(Error::msg(format!(
            "{}: data range {}..{} is past the end of {}.",
            entry.path,
            offset,
            end,
            source.display()
        )));
    }

    let start = entry.preload.len();
    let end = start + entry.length as usize;
    read_exact_at(&mut file, &mut output[start..end], offset, &source)?;

    Ok(output)
}

/// Chunk path for an archive index. The caller must reject `VPK_DIRECTORY_ARCHIVE`.
pub fn chunk_path_for(directory_path: &Path, archive_index: u16) -> PathBuf {
    let raw = directory_path.to_string_lossy();
    // A slot that is switched off is `pakNN_dir.vpk.disabled`; its chunks keep the plain name.
    let visible = if raw.to_ascii_lowercase().ends_with(".disabled") {
        &raw[..raw.len() - ".disabled".len()]
    } else {
        raw.as_ref()
    };
    let lowered = visible.to_ascii_lowercase();
    let stem = if lowered.ends_with("_dir.vpk") {
        &visible[..visible.len() - "_dir.vpk".len()]
    } else if lowered.ends_with(".vpk") {
        &visible[..visible.len() - ".vpk".len()]
    } else {
        visible
    };

    PathBuf::from(format!("{stem}_{archive_index:03}.vpk"))
}

/// Validating variant of [`chunk_path_for`]. Rejects the directory archive marker.
pub fn try_chunk_path_for(directory_path: &Path, archive_index: u16) -> Result<PathBuf> {
    if archive_index >= VPK_DIRECTORY_ARCHIVE {
        return Err(Error::msg(format!(
            "Invalid VPK archive index {archive_index}"
        )));
    }

    Ok(chunk_path_for(directory_path, archive_index))
}

pub fn crc32(data: &[u8]) -> u32 {
    crc32fast::hash(data)
}

pub fn is_safe_vpk_path(path: &str) -> bool {
    if path.is_empty() || path.contains('\0') || path.contains('\\') {
        return false;
    }

    if path.starts_with('/') {
        return false;
    }

    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return false;
    }

    path.split('/')
        .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

struct ParsedTree {
    entries: Vec<VpkEntry>,
    by_path: HashMap<String, VpkEntry>,
    skipped: u64,
    duplicates: u64,
}

fn parse_tree(tree: &[u8], keep_preload: bool) -> Result<ParsedTree> {
    let mut cursor = Cursor {
        bytes: tree,
        position: 0,
    };
    let mut entries = Vec::new();
    let mut by_path: HashMap<String, VpkEntry> = HashMap::new();
    let mut skipped = 0_u64;
    let mut duplicates = 0_u64;

    loop {
        let type_name = cursor.read_cstring()?;
        if type_name.is_empty() {
            break;
        }

        loop {
            let directory_name = cursor.read_cstring()?;
            if directory_name.is_empty() {
                break;
            }

            let directory = if directory_name == " " {
                ""
            } else {
                directory_name.as_str()
            };

            loop {
                let file_name = cursor.read_cstring()?;
                if file_name.is_empty() {
                    break;
                }

                if cursor.position + ENTRY_METADATA_SIZE > tree.len() {
                    return Err(Error::msg(
                        "VPK directory tree ends in the middle of an entry.",
                    ));
                }

                let start = cursor.position;
                let crc32 = read_u32(tree, start);
                let preload_bytes = read_u16(tree, start + 4);
                let archive_index = read_u16(tree, start + 6);
                let offset = read_u32(tree, start + 8);
                let length = read_u32(tree, start + 12);
                let terminator = read_u16(tree, start + 16);
                cursor.position += ENTRY_METADATA_SIZE;

                if terminator != ENTRY_TERMINATOR {
                    return Err(Error::msg(format!(
                        "VPK entry {directory}/{file_name} has terminator 0x{terminator:x}."
                    )));
                }

                let preload_size = usize::from(preload_bytes);
                if cursor.position + preload_size > tree.len() {
                    return Err(Error::msg(
                        "VPK directory tree ends in the middle of preload data.",
                    ));
                }

                let preload = if keep_preload {
                    tree[cursor.position..cursor.position + preload_size].to_vec()
                } else {
                    Vec::new()
                };
                cursor.position += preload_size;

                let Some(path) = join_vpk_path(directory, &file_name, &type_name) else {
                    skipped += 1;
                    continue;
                };

                let total_length = u32::from(preload_bytes).saturating_add(length);
                let entry = VpkEntry {
                    path,
                    crc32,
                    preload,
                    preload_bytes,
                    archive_index,
                    offset,
                    length,
                    total_length,
                };
                let key = entry.path.to_ascii_lowercase();

                if by_path.contains_key(&key) {
                    duplicates += 1;
                }

                entries.push(entry.clone());
                by_path.insert(key, entry);
            }
        }
    }

    Ok(ParsedTree {
        entries,
        by_path,
        skipped,
        duplicates,
    })
}

fn join_vpk_path(directory: &str, file_name: &str, type_name: &str) -> Option<String> {
    if file_name == " " || file_name.is_empty() {
        return None;
    }

    let file = if type_name == " " {
        file_name.to_string()
    } else {
        format!("{file_name}.{type_name}")
    };
    let path = if directory.is_empty() {
        file
    } else {
        format!("{directory}/{file}")
    };

    is_safe_vpk_path(&path).then_some(path)
}

fn read_header(file: &mut File, size: u64, path: &Path) -> Result<VpkHeader> {
    let header_length = usize::try_from(size.min(u64::from(HEADER_V2_SIZE))).unwrap_or(0);
    let mut buffer = vec![0u8; header_length];
    read_exact_at(file, &mut buffer, 0, path)?;

    if buffer.len() < HEADER_V1_SIZE as usize {
        return Err(Error::msg(format!(
            "{} has a truncated VPK header.",
            path.display()
        )));
    }

    let magic = read_u32(&buffer, 0);
    if magic != VPK_MAGIC {
        return Err(Error::msg(format!(
            "{} is not a VPK file (magic 0x{magic:x}).",
            path.display()
        )));
    }

    let version = read_u32(&buffer, 4);
    let tree_size = read_u32(&buffer, 8);

    if version == 1 {
        return Ok(VpkHeader {
            version: 1,
            header_size: HEADER_V1_SIZE,
            tree_size,
            file_data_section_size: 0,
            archive_md5_section_size: 0,
            other_md5_section_size: 0,
            signature_section_size: 0,
        });
    }

    if version != 2 {
        return Err(Error::msg(format!(
            "{} uses unsupported VPK version {version}.",
            path.display()
        )));
    }

    if buffer.len() < HEADER_V2_SIZE as usize {
        return Err(Error::msg(format!(
            "{} has a truncated VPK version 2 header.",
            path.display()
        )));
    }

    Ok(VpkHeader {
        version: 2,
        header_size: HEADER_V2_SIZE,
        tree_size,
        file_data_section_size: read_u32(&buffer, 12),
        archive_md5_section_size: read_u32(&buffer, 16),
        other_md5_section_size: read_u32(&buffer, 20),
        signature_section_size: read_u32(&buffer, 24),
    })
}

fn read_u16(buffer: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([buffer[offset], buffer[offset + 1]])
}

fn read_u32(buffer: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        buffer[offset],
        buffer[offset + 1],
        buffer[offset + 2],
        buffer[offset + 3],
    ])
}

fn read_exact_at(file: &mut File, buffer: &mut [u8], position: u64, source: &Path) -> Result<()> {
    file.seek(SeekFrom::Start(position))?;

    let mut read = 0;
    while read < buffer.len() {
        let count = file.read(&mut buffer[read..])?;
        if count == 0 {
            return Err(Error::msg(format!(
                "{} ended before the expected {} bytes could be read.",
                source.display(),
                buffer.len()
            )));
        }
        read += count;
    }

    Ok(())
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl Cursor<'_> {
    fn read_cstring(&mut self) -> Result<String> {
        let start = self.position;

        while self.position < self.bytes.len() && self.bytes[self.position] != 0 {
            self.position += 1;
        }

        if self.position >= self.bytes.len() {
            return Err(Error::msg(
                "VPK directory tree ends before a string terminator.",
            ));
        }

        let value = String::from_utf8_lossy(&self.bytes[start..self.position]).into_owned();
        self.position += 1;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cstring(value: &str, output: &mut Vec<u8>) {
        output.extend_from_slice(value.as_bytes());
        output.push(0);
    }

    #[test]
    fn reads_a_version_2_directory_with_chunk_and_preload() {
        let preload = b"pre".to_vec();
        let data = b"payload".to_vec();

        let mut tree = Vec::new();
        cstring("vmdl_c", &mut tree);
        cstring("models/heroes_wip/billy", &mut tree);
        cstring("billy", &mut tree);

        let metadata_start = tree.len();
        tree.extend_from_slice(&[0u8; ENTRY_METADATA_SIZE]);
        tree.extend_from_slice(&preload);

        let total = [preload.as_slice(), data.as_slice()].concat();
        let crc = crc32(&total);
        tree[metadata_start..metadata_start + 4].copy_from_slice(&crc.to_le_bytes());
        tree[metadata_start + 4..metadata_start + 6]
            .copy_from_slice(&(preload.len() as u16).to_le_bytes());
        tree[metadata_start + 6..metadata_start + 8].copy_from_slice(&1_u16.to_le_bytes());
        tree[metadata_start + 8..metadata_start + 12].copy_from_slice(&0_u32.to_le_bytes());
        tree[metadata_start + 12..metadata_start + 16]
            .copy_from_slice(&(data.len() as u32).to_le_bytes());
        tree[metadata_start + 16..metadata_start + 18]
            .copy_from_slice(&ENTRY_TERMINATOR.to_le_bytes());

        let inline = b"dir".to_vec();
        cstring("other", &mut tree);
        let inline_start = tree.len();
        tree.extend_from_slice(&[0u8; ENTRY_METADATA_SIZE]);
        tree[inline_start..inline_start + 4].copy_from_slice(&crc32(&inline).to_le_bytes());
        tree[inline_start + 6..inline_start + 8]
            .copy_from_slice(&VPK_DIRECTORY_ARCHIVE.to_le_bytes());
        tree[inline_start + 12..inline_start + 16]
            .copy_from_slice(&(inline.len() as u32).to_le_bytes());
        tree[inline_start + 16..inline_start + 18].copy_from_slice(&ENTRY_TERMINATOR.to_le_bytes());

        cstring("", &mut tree);
        cstring("", &mut tree);
        tree.push(0);

        let mut header = Vec::new();
        header.extend_from_slice(&VPK_MAGIC.to_le_bytes());
        header.extend_from_slice(&2_u32.to_le_bytes());
        header.extend_from_slice(&(tree.len() as u32).to_le_bytes());
        header.extend_from_slice(&(inline.len() as u32).to_le_bytes());
        header.extend_from_slice(&0_u32.to_le_bytes());
        header.extend_from_slice(&0_u32.to_le_bytes());
        header.extend_from_slice(&0_u32.to_le_bytes());

        let directory = std::env::temp_dir().join(format!("dah-vpk-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let directory_path = directory.join("pak01_dir.vpk");
        std::fs::write(&directory_path, [header, tree, inline.clone()].concat()).unwrap();
        std::fs::write(directory.join("pak01_001.vpk"), &data).unwrap();

        let parsed = read_vpk_directory(&directory_path).unwrap();
        assert_eq!(parsed.entries.len(), 2);

        let entry = parsed
            .by_path
            .get("models/heroes_wip/billy/billy.vmdl_c")
            .unwrap();
        assert_eq!(entry.preload, preload);
        assert_eq!(entry.total_length, 10);
        assert_eq!(read_vpk_entry_data(&parsed, entry).unwrap(), total);

        let inline_entry = parsed
            .by_path
            .get("models/heroes_wip/billy/other.vmdl_c")
            .unwrap();
        assert_eq!(read_vpk_entry_data(&parsed, inline_entry).unwrap(), inline);

        assert_eq!(
            chunk_path_for(&directory_path, 7).file_name().unwrap(),
            "pak01_007.vpk"
        );

        std::fs::remove_dir_all(&directory).unwrap();
    }
}
