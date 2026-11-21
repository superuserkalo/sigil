//! What one file looks like in the explorer: text, decoded Source 2 data, a Panorama layout,
//! a text DMX summary, a texture, a sound, or a hex dump. The same decoding feeds content
//! search, comparison and the editable export.

use morphic::resource::{Resource, read_rerl};
use serde::Serialize;

use super::{dmx, extension, kv3_text};
use crate::game_assets::{AssetThumbnail, decode_thumbnail};

/// Preview text past this many bytes is cut, with a note.
pub const PREVIEW_TEXT_MAX: usize = 512 * 1024;
/// A hex dump shows this many bytes.
const HEX_BYTES: usize = 4096;
/// Textures preview at this size on the longer edge.
pub const PREVIEW_IMAGE_EDGE: u32 = 1024;
/// A file this large is not decoded to text for search or export.
pub const DECODE_MAX_BYTES: usize = 16 * 1024 * 1024;

/// Source files read as plain text. DMM's text list plus the Panorama and DMX sources.
pub const TEXT_EXTENSIONS: &[&str] = &[
    "txt", "json", "xml", "cfg", "kv3", "lst", "lua", "nut", "js", "ts", "csv", "ini", "vdf",
    "vmt", "vtex", "vmat", "vmdl", "vpcf", "vdata", "vsndevts", "vxml", "vcss", "vjs", "css",
    "html", "gi", "res", "vsnap", "vpost", "vnmskel", "vnmclip", "vnmgraph", "vcompmat", "md",
    "vfx", "hlsl", "fgd", "dmx", "vseq", "kv", "vmap_txt", "vrman", "vsmart", "lst",
];

/// Compiled Panorama resources: the DATA block holds the source text.
pub const PANORAMA_COMPILED: &[&str] = &["vxml_c", "vcss_c", "vjs_c", "vts_c", "vpdi_c"];

/// Compiled files content search skips: pixels, sound and vertex data hold no text.
pub const NO_TEXT_COMPILED: &[&str] = &["vtex_c", "vsnd_c", "vwrld_c", "vwnod_c", "vrman_c"];

/// Images a desktop previewer decodes itself.
pub const RASTER_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp", "tga"];

/// Sounds the desktop previewer can describe.
pub const AUDIO_EXTENSIONS: &[&str] = &["vsnd_c", "mp3", "wav", "ogg", "flac"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PreviewKind {
    /// A source or text file, shown as is.
    Text,
    /// A compiled resource's KV3 DATA as KV3 text.
    Data,
    /// A compiled Panorama layout, style or script.
    Panorama,
    /// A text DMX: a summary of models, meshes and joints, then the text.
    Dmx,
    /// A texture (decoded) or an image file (the caller decodes the bytes).
    Image,
    /// A sound.
    Audio,
    /// Anything else: resource blocks or a hex dump.
    Binary,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub kind: PreviewKind,
    /// A short format name: "KV3 data", "Panorama layout", "Text DMX", "Texture 512 x 512".
    pub format: String,
    pub text: String,
    /// A decoded texture. `None` for an image file the caller decodes from the bytes.
    #[serde(skip)]
    pub image: Option<AssetThumbnail>,
    /// External references (RERL) of a compiled resource.
    pub references: Vec<String>,
    pub truncated: bool,
    pub note: Option<String>,
}

impl Preview {
    fn new(kind: PreviewKind, format: impl Into<String>, text: String) -> Self {
        let mut preview = Preview {
            kind,
            format: format.into(),
            text,
            image: None,
            references: Vec::new(),
            truncated: false,
            note: None,
        };
        preview.clip();
        preview
    }

    fn clip(&mut self) {
        if self.text.len() > PREVIEW_TEXT_MAX {
            let mut cut = PREVIEW_TEXT_MAX;
            while cut > 0 && !self.text.is_char_boundary(cut) {
                cut -= 1;
            }
            self.text.truncate(cut);
            self.truncated = true;
        }
    }

    pub fn line_count(&self) -> usize {
        self.text.lines().count()
    }
}

/// Preview one file from its game path and bytes.
pub fn preview(path: &str, bytes: &[u8]) -> Preview {
    let ext = extension(path);
    if RASTER_EXTENSIONS.contains(&ext.as_str()) {
        let mut preview = Preview::new(PreviewKind::Image, format!("{} image", ext.to_uppercase()), String::new());
        preview.note = Some(size_note(bytes.len()));
        return preview;
    }
    if ext == "vtex_c" {
        return texture(bytes);
    }
    if AUDIO_EXTENSIONS.contains(&ext.as_str()) {
        let mut preview = Preview::new(PreviewKind::Audio, sound_format(&ext), String::new());
        preview.note = Some(size_note(bytes.len()));
        if ext == "vsnd_c" {
            preview.references = references(bytes);
        }
        return preview;
    }
    if let Some(text) = decoded_text(path, bytes) {
        let kind = text.kind;
        let mut preview = Preview::new(kind, text.format, text.text);
        preview.references = text.references;
        preview.note = text.note;
        return preview;
    }
    binary(bytes)
}

/// A file decoded to text, with what it was decoded as.
#[derive(Debug, Clone)]
pub struct DecodedText {
    pub kind: PreviewKind,
    pub format: String,
    pub text: String,
    pub references: Vec<String>,
    pub note: Option<String>,
}

/// Text for a file that has some: a text source, a text DMX, a compiled Panorama file or any
/// compiled KV3 resource. `None` for pixels, sound, binary data and anything too large.
pub fn decoded_text(path: &str, bytes: &[u8]) -> Option<DecodedText> {
    if bytes.len() > DECODE_MAX_BYTES {
        return None;
    }
    let ext = extension(path);
    if ext.ends_with("_c") {
        if NO_TEXT_COMPILED.contains(&ext.as_str()) {
            return None;
        }
        if PANORAMA_COMPILED.contains(&ext.as_str())
            && let Some(text) = panorama_text(bytes)
        {
            return Some(DecodedText {
                kind: PreviewKind::Panorama,
                format: panorama_format(&ext).to_string(),
                text,
                references: references(bytes),
                note: None,
            });
        }
        let value = morphic::decode_kv3_resource(bytes).ok()?;
        return Some(DecodedText {
            kind: PreviewKind::Data,
            format: "KV3 data".to_string(),
            text: kv3_text::to_text(&value),
            references: references(bytes),
            note: Some("Compiled data decoded to KV3 text. Read-only.".to_string()),
        });
    }
    let text = source_text(bytes)?;
    if ext == "dmx" || text.starts_with("<!-- dmx") {
        return Some(match dmx::parse(&text) {
            Ok(document) => DecodedText {
                kind: PreviewKind::Dmx,
                format: "Text DMX".to_string(),
                text: format!("{}\n{text}", dmx::summary_text(&document)),
                references: Vec::new(),
                note: None,
            },
            Err(reason) => DecodedText {
                kind: PreviewKind::Text,
                format: "DMX".to_string(),
                text: if dmx::is_binary(bytes) { String::new() } else { text },
                references: Vec::new(),
                note: Some(reason),
            },
        });
    }
    let known = TEXT_EXTENSIONS.contains(&ext.as_str());
    if !known && !mostly_printable(&text) {
        return None;
    }
    Some(DecodedText {
        kind: PreviewKind::Text,
        format: if ext.is_empty() { "Text".to_string() } else { format!("{} text", ext.to_uppercase()) },
        text,
        references: Vec::new(),
        note: None,
    })
}

/// A source file as text: UTF-8 (with or without a BOM) or UTF-16 with a BOM. `None` when the
/// bytes hold a NUL or do not decode.
pub fn source_text(bytes: &[u8]) -> Option<String> {
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, u16::from_le_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, u16::from_be_bytes);
    }
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if body.contains(&0) {
        return None;
    }
    String::from_utf8(body.to_vec()).ok()
}

fn utf16(bytes: &[u8], read: fn([u8; 2]) -> u16) -> Option<String> {
    let units: Vec<u16> = bytes.chunks_exact(2).map(|pair| read([pair[0], pair[1]])).collect();
    String::from_utf16(&units).ok()
}

fn mostly_printable(text: &str) -> bool {
    let sample: Vec<char> = text.chars().take(4096).collect();
    if sample.is_empty() {
        return true;
    }
    let odd = sample
        .iter()
        .filter(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        .count();
    odd * 50 < sample.len()
}

/// The source text inside a compiled Panorama file. Tries, in order: a KV3 DATA block with a
/// source string, the legacy Panorama block (CRC, name table, then the text), and the longest
/// run of text in the block.
pub fn panorama_text(bytes: &[u8]) -> Option<String> {
    let resource = Resource::parse(bytes).ok()?;
    let data = resource.data_block().ok()?;
    if let Ok(value) = morphic::kv3::decode(data) {
        if let Some(source) = longest_string(&value) {
            return Some(source);
        }
        return Some(kv3_text::to_text(&value));
    }
    if let Some(text) = legacy_panorama(data) {
        return Some(text);
    }
    longest_text_run(data)
}

/// The legacy Panorama DATA layout: u32 CRC, u16 name count, each name NUL-terminated and
/// followed by a u32, then the source text (NUL-terminated).
fn legacy_panorama(data: &[u8]) -> Option<String> {
    if data.len() < 6 {
        return None;
    }
    let crc = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    let count = u16::from_le_bytes([data[4], data[5]]) as usize;
    let mut at = 6;
    for _ in 0..count {
        let end = data[at..].iter().position(|b| *b == 0)? + at;
        at = end + 1 + 4;
        if at > data.len() {
            return None;
        }
    }
    let body = &data[at..];
    let body = match body.iter().rposition(|b| *b != 0) {
        Some(last) => &body[..=last],
        None => return None,
    };
    let text = std::str::from_utf8(body).ok()?;
    let crc_ok = crate::vpk::crc32(body) == crc || crate::vpk::crc32(&data[at..]) == crc;
    (crc_ok || mostly_printable(text)).then(|| text.to_string())
}

fn longest_string(value: &morphic::kv3::Value) -> Option<String> {
    use morphic::kv3::Value;
    let mut best: Option<&str> = None;
    let mut stack = vec![value];
    while let Some(node) = stack.pop() {
        match node {
            Value::String(text) if text.len() > 64 && text.contains('\n') => {
                if best.is_none_or(|b| text.len() > b.len()) {
                    best = Some(text);
                }
            }
            Value::Array(items) => stack.extend(items.iter()),
            Value::Object(pairs) => stack.extend(pairs.iter().map(|(_, v)| v)),
            _ => {}
        }
    }
    best.map(str::to_string)
}

fn longest_text_run(data: &[u8]) -> Option<String> {
    let mut best = (0, 0);
    let mut start = 0;
    for (i, byte) in data.iter().enumerate() {
        let ok = matches!(byte, b'\n' | b'\r' | b'\t') || (0x20..0x7F).contains(byte) || *byte >= 0x80;
        if !ok {
            if i - start > best.1 - best.0 {
                best = (start, i);
            }
            start = i + 1;
        }
    }
    if data.len() - start > best.1 - best.0 {
        best = (start, data.len());
    }
    (best.1 - best.0 >= 32).then(|| String::from_utf8_lossy(&data[best.0..best.1]).into_owned())
}

fn panorama_format(ext: &str) -> &'static str {
    match ext {
        "vxml_c" => "Panorama layout",
        "vcss_c" => "Panorama style",
        "vjs_c" | "vts_c" => "Panorama script",
        _ => "Panorama data",
    }
}

/// The external references (RERL) a compiled resource lists.
pub fn references(bytes: &[u8]) -> Vec<String> {
    let Ok(resource) = Resource::parse(bytes) else {
        return Vec::new();
    };
    let Some(block) = resource.find_block(*b"RERL") else {
        return Vec::new();
    };
    read_rerl(block)
        .map(|refs| refs.into_iter().map(|(_, name)| name).collect())
        .unwrap_or_default()
}

fn texture(bytes: &[u8]) -> Preview {
    match decode_thumbnail(bytes, PREVIEW_IMAGE_EDGE) {
        Ok(image) => {
            let mut preview = Preview::new(
                PreviewKind::Image,
                format!("Texture {} x {}", image.source_width, image.source_height),
                String::new(),
            );
            preview.image = Some(image);
            preview.references = references(bytes);
            preview
        }
        Err(error) => {
            let mut preview = binary(bytes);
            preview.note = Some(format!("The texture did not decode: {error}"));
            preview
        }
    }
}

fn sound_format(ext: &str) -> String {
    match ext {
        "vsnd_c" => "Compiled sound".to_string(),
        other => format!("{} sound", other.to_uppercase()),
    }
}

fn size_note(bytes: usize) -> String {
    format!("{} bytes", bytes)
}

/// A compiled resource's block table, or a hex dump of the first bytes.
fn binary(bytes: &[u8]) -> Preview {
    if let Ok(resource) = Resource::parse(bytes) {
        let mut text = String::from("Block  Offset      Size\n");
        for block in resource.blocks() {
            text.push_str(&format!(
                "{}   {:>10}  {:>10}\n",
                String::from_utf8_lossy(&block.kind),
                block.offset,
                block.size
            ));
        }
        let mut preview = Preview::new(PreviewKind::Binary, "Compiled resource", text);
        preview.references = references(bytes);
        preview.note = Some("This resource has no text data. Its blocks are listed.".to_string());
        return preview;
    }
    let mut preview = Preview::new(PreviewKind::Binary, "Binary", hex_dump(&bytes[..bytes.len().min(HEX_BYTES)]));
    preview.truncated = bytes.len() > HEX_BYTES;
    preview
}

/// Sixteen bytes per line: offset, hex, then the printable characters.
pub fn hex_dump(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 4 + 64);
    for (line, chunk) in bytes.chunks(16).enumerate() {
        out.push_str(&format!("{:08x}  ", line * 16));
        for i in 0..16 {
            match chunk.get(i) {
                Some(byte) => out.push_str(&format!("{byte:02x} ")),
                None => out.push_str("   "),
            }
        }
        out.push(' ');
        for byte in chunk {
            out.push(if (0x20..0x7F).contains(byte) { *byte as char } else { '.' });
        }
        out.push('\n');
    }
    out
}
