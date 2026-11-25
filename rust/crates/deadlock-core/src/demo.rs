//! Local Deadlock demo import and the merged API plus demo match summary.
//!
//! This module is the depth half of the stats product. It reads a Source 2
//! `.dem` container, stores it under `<workspace>/matches/<match-id>/` and
//! builds a deterministic summary from the fields a parser actually produced.
//!
//! What is real today:
//!
//! - The Source 2 outer framing: the `PBDEMS2\0` magic, the two little-endian
//!   32 bit offsets at byte 8, the command stream at byte 16, varint command
//!   framing, the Snappy compression bit (64) and a Snappy block decoder.
//! - `CDemoFileHeader`, `CDemoFileInfo`, `CDemoStringTables`,
//!   `CDemoFullPacket`, `CDemoPacket`, `CDemoCustomData` and
//!   `CDemoCustomDataCallbacks` protobuf payloads, using the field numbers in
//!   the Source 2 `demo.proto` used by `source2-demo`.
//! - The packet framing inside `CDemoPacket`, so the importer can inventory
//!   the user message ids a demo contains without decoding the messages.
//!
//! What is not real today:
//!
//! - Entity state, player positions, kills, deaths, assists, objectives and
//!   item purchases. Those need a serializer aware backend: entity state is
//!   delta encoded against per-class baselines from `DEM_SendTables` and
//!   `DEM_ClassInfo`, and Citadel match metadata lives in user message 316
//!   (`k_EUserMsg_PostMatchDetails`). The container extractor does not guess.
//! - A heatmap without position samples. `summary.heatmap` says absent and
//!   carries the reason instead of drawing an empty map.
//!
//! The extractor seam is [`DemoExtractor`]. [`ContainerExtractor`] is shipped;
//! a serializer aware backend can implement the same trait and return events
//! and positions without changing the store or the summary merge.
//!
//! Format references are recorded in `docs/research/demo-import.md`.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufReader, ErrorKind, Read};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::util::{now_iso, write_atomic};

/// Source 2 demo magic. Every Deadlock `.dem` starts with these eight bytes.
pub const DEMO_MAGIC: [u8; 8] = *b"PBDEMS2\0";
/// Name of the importer recorded in provenance.
pub const IMPORTER_NAME: &str = "dah demo importer";
/// Version of the importer recorded in provenance.
pub const IMPORTER_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Directory under the workspace that holds imported matches.
pub const MATCHES_DIR_NAME: &str = "matches";
/// Stored demo file name inside a match folder.
pub const MATCH_DEMO_FILE_NAME: &str = "match.dem";
/// Stored fixture file name. Fixtures are test and UI development input only.
pub const MATCH_FIXTURE_FILE_NAME: &str = "fixture.json";
/// Compact match metadata written by every import.
pub const MATCH_META_FILE_NAME: &str = "meta.json";
/// One JSON object per extracted event.
pub const MATCH_EVENTS_FILE_NAME: &str = "events.jsonl";
/// Merged API plus demo summary.
pub const MATCH_SUMMARY_FILE_NAME: &str = "summary.json";
/// Importer provenance, hashes and the fields that were not extracted.
pub const MATCH_PROVENANCE_FILE_NAME: &str = "provenance.json";
/// Cached raw deadlock-api response, when the user or the app put one here.
pub const MATCH_API_FILE_NAME: &str = "api.json";
/// Optional coaching review. Imports never delete it.
pub const MATCH_REVIEW_FILE_NAME: &str = "review.md";

/// Version of the `summary.json` shape.
pub const SUMMARY_SCHEMA: &str = "dah.match.summary/1";
/// Version of the `provenance.json` shape.
pub const PROVENANCE_SCHEMA: &str = "dah.match.provenance/1";
/// Version of the `meta.json` shape.
pub const META_SCHEMA: &str = "dah.match.meta/1";
/// Heatmap cell size in map world units. The map asset reports a radius of
/// 10752, so a 512 unit cell is 84 cells across the widest axis.
pub const HEATMAP_CELL_SIZE: f64 = 512.0;
/// Verified Citadel user message id carried in `CDemoPacket`.
///
/// `k_EUserMsg_PostMatchDetails = 316` in the Deadlock proto used by
/// `source2-demo`. The importer counts the message and does not decode it.
pub const CITADEL_POST_MATCH_DETAILS_ID: i32 = 316;

const MAX_PAYLOAD_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DECODE_BYTES: usize = 16 * 1024 * 1024;
const MAX_EVENTS: usize = 20_000;
const MAX_POSITIONS: usize = 200_000;
const MAX_CUSTOM_DATA: usize = 512;
const MAX_CALLBACKS: usize = 256;
const MAX_STRING_TABLES: usize = 256;
const MAX_STRING_TABLE_SAMPLES: usize = 16;
const MAX_PACKET_MESSAGES: u64 = 1_000_000;

// ---------------------------------------------------------------------------
// Container model
// ---------------------------------------------------------------------------

/// Decoded `CDemoFileHeader`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DemoHeader {
    /// `demo_file_stamp`. The required first field, normally `PBDEMS2`.
    pub demo_file_stamp: String,
    /// `patch_version`. The Source 2 demo protocol version. Source 1 called
    /// this the demo protocol number.
    pub patch_version: Option<i32>,
    pub server_name: Option<String>,
    pub client_name: Option<String>,
    pub map_name: Option<String>,
    pub game_directory: Option<String>,
    pub fullpackets_version: Option<i32>,
    pub allow_clientside_entities: Option<bool>,
    pub allow_clientside_particles: Option<bool>,
    pub addons: Option<String>,
    pub demo_version_name: Option<String>,
    pub demo_version_guid: Option<String>,
    pub build_num: Option<i32>,
    pub game: Option<String>,
    pub server_start_tick: Option<i32>,
}

/// Decoded `CDemoFileInfo`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DemoFileInfo {
    /// `playback_time` in seconds. This is the server's own duration value.
    pub playback_time_seconds: Option<f32>,
    pub playback_ticks: Option<u32>,
    pub playback_frames: Option<u32>,
    /// Bytes of the nested `CGameInfo` message. The importer does not decode
    /// game specific player arrays because Deadlock does not use the Dota or
    /// CS branches of that message.
    pub game_info_bytes: u64,
}

/// One string table in the demo. Only an inventory is kept: names, counts and
/// a short sample of keys. Full table contents are not needed for a match card.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DemoStringTable {
    pub name: String,
    pub items: u64,
    pub clientside_items: u64,
    pub data_bytes: u64,
    pub table_flags: Option<i32>,
    pub sample_keys: Vec<String>,
    pub truncated: bool,
}

/// One embedded user or net message counted inside `CDemoPacket`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DemoUserMessageStat {
    /// Raw message id from the packet stream.
    pub id: i32,
    /// Verified name when the importer knows the id. Most ids stay unnamed.
    pub name: Option<String>,
    pub count: u64,
    pub payload_bytes: u64,
}

/// Aggregate packet counts.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DemoPacketInventory {
    pub packets: u64,
    pub bytes: u64,
    pub messages: u64,
    pub truncated_packets: u64,
}

/// One `CDemoCustomData` payload. The data itself is not kept in the store.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DemoCustomDataEntry {
    pub callback_index: Option<i32>,
    pub bytes: u64,
    /// First printable ASCII run, useful to recognise a custom data block.
    pub preview: Option<String>,
}

/// Counts and offsets from the outer framing.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DemoFraming {
    pub commands_total: u64,
    /// Commands whose payload was read and, when compressed, decompressed.
    pub commands_decoded: u64,
    pub compressed_commands: u64,
    pub compressed_decoded: u64,
    pub unknown_commands: u64,
    /// `CDemoFileInfo` offset from byte 8 of the file.
    pub file_info_offset: Option<u32>,
    /// Second little-endian offset at byte 12. Its meaning is not documented
    /// in the sources used here, so it is recorded and not interpreted.
    pub offset_2: u32,
    pub stop_seen: bool,
    pub trailing_bytes: u64,
    pub bytes_consumed: u64,
    pub command_counts: BTreeMap<String, u64>,
}

/// One parsed demo container.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DemoFile {
    pub source_path: PathBuf,
    pub byte_len: u64,
    pub crc32: u32,
    pub header: Option<DemoHeader>,
    pub file_info: Option<DemoFileInfo>,
    pub string_tables: Vec<DemoStringTable>,
    pub user_messages: Vec<DemoUserMessageStat>,
    pub packets: DemoPacketInventory,
    pub custom_data: Vec<DemoCustomDataEntry>,
    pub custom_data_callbacks: Vec<String>,
    pub framing: DemoFraming,
    pub notes: Vec<String>,
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

struct DemoReader<R: Read> {
    inner: R,
    crc: crc32fast::Hasher,
    consumed: u64,
}

impl<R: Read> DemoReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            crc: crc32fast::Hasher::new(),
            consumed: 0,
        }
    }

    fn read_byte(&mut self) -> Result<Option<u8>> {
        let mut buffer = [0_u8; 1];
        loop {
            match self.inner.read(&mut buffer) {
                Ok(0) => return Ok(None),
                Ok(_) => {
                    self.crc.update(&buffer);
                    self.consumed += 1;
                    return Ok(Some(buffer[0]));
                }
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) => return Err(Error::Io(error)),
            }
        }
    }

    fn read_exact(&mut self, count: usize) -> Result<Vec<u8>> {
        let mut output = vec![0_u8; count];
        let mut filled = 0;
        while filled < count {
            match self.inner.read(&mut output[filled..]) {
                Ok(0) => {
                    return Err(Error::msg(format!(
                        "Unexpected end of demo at byte {}: expected {count} more bytes.",
                        self.consumed
                    )));
                }
                Ok(read) => {
                    self.crc.update(&output[filled..filled + read]);
                    self.consumed += read as u64;
                    filled += read;
                }
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) => return Err(Error::Io(error)),
            }
        }
        Ok(output)
    }

    fn read_varint(&mut self) -> Result<Option<u64>> {
        let mut result = 0_u64;
        let mut shift = 0_u32;

        loop {
            match self.read_byte()? {
                None => {
                    if shift == 0 {
                        return Ok(None);
                    }
                    return Err(Error::msg(format!(
                        "Truncated varint at byte {}: the last byte had its continuation bit set.",
                        self.consumed
                    )));
                }
                Some(byte) => {
                    result |= u64::from(byte & 0x7f) << shift;
                    if byte & 0x80 == 0 {
                        return Ok(Some(result));
                    }
                    shift += 7;
                    if shift >= 64 {
                        return Err(Error::msg(format!(
                            "Varint at byte {} is longer than ten bytes.",
                            self.consumed
                        )));
                    }
                }
            }
        }
    }

    fn skip(&mut self, count: u64) -> Result<()> {
        let mut remaining = count;
        let mut buffer = [0_u8; 8192];

        while remaining > 0 {
            let want = remaining.min(buffer.len() as u64) as usize;
            let mut filled = 0;
            while filled < want {
                match self.inner.read(&mut buffer[filled..want]) {
                    Ok(0) => {
                        return Err(Error::msg(format!(
                            "Unexpected end of demo at byte {}: {remaining} bytes were skipped past the end.",
                            self.consumed
                        )));
                    }
                    Ok(read) => {
                        self.crc.update(&buffer[filled..filled + read]);
                        self.consumed += read as u64;
                        filled += read;
                    }
                    Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                    Err(error) => return Err(Error::Io(error)),
                }
            }
            remaining -= want as u64;
        }

        Ok(())
    }

    fn drain(&mut self) -> Result<u64> {
        let mut total = 0_u64;
        let mut buffer = [0_u8; 8192];

        loop {
            match self.inner.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    self.crc.update(&buffer[..read]);
                    self.consumed += read as u64;
                    total += read as u64;
                }
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) => return Err(Error::Io(error)),
            }
        }

        Ok(total)
    }

    fn finish(&mut self) -> (u64, u32) {
        let crc = std::mem::replace(&mut self.crc, crc32fast::Hasher::new()).finalize();
        (self.consumed, crc)
    }
}

/// Read and parse a Source 2 demo file.
///
/// The file is checked for the `PBDEMS2\0` magic before any folder is created,
/// so a wrong file cannot produce a partial match store.
pub fn parse_demo(path: &Path) -> Result<DemoFile> {
    let metadata = fs::metadata(path).map_err(|error| {
        Error::msg(format!(
            "Could not read demo {}: {error}. Pass a .dem file recorded by Deadlock.",
            path.display()
        ))
    })?;

    if !metadata.is_file() {
        return Err(Error::msg(format!(
            "{} is not a file. Pass a .dem file recorded by Deadlock.",
            path.display()
        )));
    }

    let byte_len = metadata.len();
    if byte_len < 16 {
        let found = fs::read(path).unwrap_or_default();
        return Err(non_demo_error(path, &found, byte_len));
    }

    let file = fs::File::open(path)
        .map_err(|error| Error::msg(format!("Could not open demo {}: {error}", path.display())))?;
    let mut reader = DemoReader::new(BufReader::with_capacity(64 * 1024, file));

    parse_stream(&mut reader, path.to_path_buf(), byte_len)
}

fn non_demo_error(path: &Path, found: &[u8], byte_len: u64) -> Error {
    let sample: Vec<u8> = found.iter().copied().take(16).collect();
    let hex = sample
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    let ascii: String = sample
        .iter()
        .map(|byte| {
            if byte.is_ascii_graphic() || *byte == b' ' {
                *byte as char
            } else {
                '.'
            }
        })
        .collect();
    let source1 = if found.starts_with(b"HL2DEMO") {
        " That is a Source 1 demo. Deadlock records Source 2 demos."
    } else {
        ""
    };

    Error::msg(format!(
        "{} is not a Source 2 demo (the file is {byte_len} bytes). Expected the magic \
         PBDEMS2\\0 (50 42 44 45 4d 53 32 00) at byte 0, found {hex} ({ascii:?}).{source1} \
         A Deadlock match recording is a Source 2 .dem written by the game.",
        path.display()
    ))
}

fn parse_stream<R: Read>(
    reader: &mut DemoReader<R>,
    source_path: PathBuf,
    byte_len: u64,
) -> Result<DemoFile> {
    let magic = reader.read_exact(8)?;
    if magic != DEMO_MAGIC {
        return Err(non_demo_error(&source_path, &magic, byte_len));
    }

    let offsets = reader.read_exact(8)?;
    let file_info_offset = u32::from_le_bytes(offsets[0..4].try_into().unwrap_or([0; 4]));
    let offset_2 = u32::from_le_bytes(offsets[4..8].try_into().unwrap_or([0; 4]));

    let mut notes = Vec::new();
    let mut framing = DemoFraming {
        offset_2,
        ..DemoFraming::default()
    };

    if file_info_offset < 16 || u64::from(file_info_offset) >= byte_len {
        notes.push(format!(
            "The CDemoFileInfo offset {file_info_offset} is outside the file; the command stream is still parsed in order."
        ));
    } else {
        framing.file_info_offset = Some(file_info_offset);
    }

    if offset_2 != 0 && (offset_2 < 16 || u64::from(offset_2) >= byte_len) {
        notes.push(format!(
            "The second header offset {offset_2} is outside the file. Its meaning is undocumented, so it is only recorded."
        ));
    }

    let mut header: Option<DemoHeader> = None;
    let mut file_info: Option<DemoFileInfo> = None;
    let mut string_tables: Vec<DemoStringTable> = Vec::new();
    let mut user_message_counts: BTreeMap<i32, (u64, u64)> = BTreeMap::new();
    let mut packets = DemoPacketInventory::default();
    let mut custom_data: Vec<DemoCustomDataEntry> = Vec::new();
    let mut callbacks: Vec<String> = Vec::new();
    let mut command_index = 0_u64;

    loop {
        let Some(command_raw) = reader.read_varint()? else {
            break;
        };
        command_index += 1;

        let Some(tick) = reader.read_varint()? else {
            notes.push(format!(
                "Command {command_index} ends after its command id; the tick and size are missing."
            ));
            break;
        };
        let Some(size) = reader.read_varint()? else {
            notes.push(format!(
                "Command {command_index} at tick {tick} ends after its size; the payload is missing."
            ));
            break;
        };

        if size > MAX_PAYLOAD_BYTES {
            return Err(Error::msg(format!(
                "Demo command {command_index} at tick {tick} declares {size} bytes, above the {MAX_PAYLOAD_BYTES} byte cap. The file is corrupt or not a Source 2 demo."
            )));
        }

        let compressed = command_raw & 64 != 0;
        let raw = (command_raw & !64) as u32;
        let known = is_known_command(raw);
        let name = command_name(raw);

        framing.commands_total += 1;
        if compressed {
            framing.compressed_commands += 1;
        }
        if !known {
            framing.unknown_commands += 1;
            *framing
                .command_counts
                .entry(format!("unknown:{raw}"))
                .or_insert(0) += 1;
        } else {
            *framing.command_counts.entry(name.to_string()).or_insert(0) += 1;
        }

        let wants_payload = matches!(raw, 1 | 2 | 6 | 7 | 8 | 10 | 11 | 13 | 18);
        let mut payload: Option<Vec<u8>> = None;

        if wants_payload && size <= MAX_DECODE_BYTES as u64 {
            let bytes = reader.read_exact(size as usize)?;
            if compressed {
                match snappy_decompress(&bytes) {
                    Ok(decoded) => {
                        framing.compressed_decoded += 1;
                        payload = Some(decoded);
                    }
                    Err(error) => notes.push(format!(
                        "Command {command_index} ({name}) is Snappy compressed and could not be decoded: {error}"
                    )),
                }
            } else {
                payload = Some(bytes);
            }
        } else {
            reader.skip(size)?;
            if wants_payload {
                notes.push(format!(
                    "Command {command_index} ({name}) has a {size} byte payload, above the {MAX_DECODE_BYTES} byte decode cap; it was skipped."
                ));
            }
        }

        if payload.is_some() {
            framing.commands_decoded += 1;
        }

        match raw {
            0 => {
                framing.stop_seen = true;
                break;
            }
            1 => {
                if let Some(bytes) = &payload {
                    match decode_file_header(bytes) {
                        Ok(value) => header = Some(value),
                        Err(error) => notes.push(format!(
                            "CDemoFileHeader at command {command_index} did not decode: {error}"
                        )),
                    }
                }
            }
            2 => {
                if let Some(bytes) = &payload {
                    match decode_file_info(bytes) {
                        Ok(value) => file_info = Some(value),
                        Err(error) => notes.push(format!(
                            "CDemoFileInfo at command {command_index} did not decode: {error}"
                        )),
                    }
                }
            }
            6 => {
                if let Some(bytes) = &payload {
                    match decode_string_tables(bytes) {
                        Ok(mut tables) => {
                            string_tables.append(&mut tables);
                            if string_tables.len() > MAX_STRING_TABLES {
                                string_tables.truncate(MAX_STRING_TABLES);
                                notes.push(format!(
                                    "String table inventory was capped at {MAX_STRING_TABLES} tables."
                                ));
                            }
                        }
                        Err(error) => notes.push(format!(
                            "CDemoStringTables at command {command_index} did not decode: {error}"
                        )),
                    }
                }
            }
            7 | 8 => {
                if let Some(bytes) = &payload {
                    match decode_packet(bytes) {
                        Ok(data) => {
                            packets.packets += 1;
                            packets.bytes += data.len() as u64;
                            scan_packet(&data, &mut user_message_counts, &mut packets, &mut notes);
                        }
                        Err(error) => notes.push(format!(
                            "CDemoPacket at command {command_index} did not decode: {error}"
                        )),
                    }
                }
            }
            10 => {
                if let Some(bytes) = &payload {
                    if custom_data.len() >= MAX_CUSTOM_DATA {
                        notes.push(format!(
                            "Custom data entries were capped at {MAX_CUSTOM_DATA}."
                        ));
                    } else {
                        match decode_custom_data(bytes) {
                            Ok(entry) => custom_data.push(entry),
                            Err(error) => notes.push(format!(
                                "CDemoCustomData at command {command_index} did not decode: {error}"
                            )),
                        }
                    }
                }
            }
            11 => {
                if let Some(bytes) = &payload {
                    match decode_custom_data_callbacks(bytes) {
                        Ok(mut ids) => {
                            for id in ids.drain(..) {
                                if callbacks.len() < MAX_CALLBACKS {
                                    callbacks.push(id);
                                }
                            }
                        }
                        Err(error) => notes.push(format!(
                            "CDemoCustomDataCallbacks at command {command_index} did not decode: {error}"
                        )),
                    }
                }
            }
            13 => {
                if let Some(bytes) = &payload {
                    match decode_full_packet(bytes) {
                        Ok((mut tables, packet)) => {
                            string_tables.append(&mut tables);
                            if string_tables.len() > MAX_STRING_TABLES {
                                string_tables.truncate(MAX_STRING_TABLES);
                            }
                            if let Some(data) = packet {
                                packets.packets += 1;
                                packets.bytes += data.len() as u64;
                                scan_packet(
                                    &data,
                                    &mut user_message_counts,
                                    &mut packets,
                                    &mut notes,
                                );
                            }
                        }
                        Err(error) => notes.push(format!(
                            "CDemoFullPacket at command {command_index} did not decode: {error}"
                        )),
                    }
                }
            }
            _ => {}
        }
    }

    let trailing_bytes = reader.drain()?;
    framing.trailing_bytes = trailing_bytes;
    let (bytes_consumed, crc32) = reader.finish();
    framing.bytes_consumed = bytes_consumed;

    if !framing.stop_seen {
        notes.push(
            "The command stream ended without DEM_Stop. The import used every command it could read."
                .to_string(),
        );
    }
    if trailing_bytes > 0 {
        notes.push(format!(
            "{trailing_bytes} trailing byte(s) follow DEM_Stop and were hashed but not parsed."
        ));
    }
    if framing.compressed_commands > framing.compressed_decoded {
        notes.push(format!(
            "{} compressed command(s) were not decoded; their payloads are counted but not inspected.",
            framing.compressed_commands - framing.compressed_decoded
        ));
    }

    let mut user_messages: Vec<DemoUserMessageStat> = user_message_counts
        .into_iter()
        .map(|(id, (count, payload_bytes))| DemoUserMessageStat {
            id,
            name: user_message_name(id).map(str::to_string),
            count,
            payload_bytes,
        })
        .collect();
    user_messages.sort_by(|left, right| right.count.cmp(&left.count).then(left.id.cmp(&right.id)));

    Ok(DemoFile {
        source_path,
        byte_len,
        crc32,
        header,
        file_info,
        string_tables,
        user_messages,
        packets,
        custom_data,
        custom_data_callbacks: callbacks,
        framing,
        notes: dedupe_notes(notes),
    })
}

fn dedupe_notes(notes: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    notes
        .into_iter()
        .filter(|note| seen.insert(note.clone()))
        .collect()
}

fn is_known_command(raw: u32) -> bool {
    raw <= 18
}

fn command_name(raw: u32) -> &'static str {
    match raw {
        0 => "stop",
        1 => "fileHeader",
        2 => "fileInfo",
        3 => "syncTick",
        4 => "sendTables",
        5 => "classInfo",
        6 => "stringTables",
        7 => "packet",
        8 => "signonPacket",
        9 => "consoleCmd",
        10 => "customData",
        11 => "customDataCallbacks",
        12 => "userCmd",
        13 => "fullPacket",
        14 => "saveGame",
        15 => "spawnGroups",
        16 => "animationData",
        17 => "animationHeader",
        18 => "recovery",
        _ => "unknown",
    }
}

fn user_message_name(id: i32) -> Option<&'static str> {
    match id {
        CITADEL_POST_MATCH_DETAILS_ID => Some("citadel.post_match_details"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Snappy block decoder
// ---------------------------------------------------------------------------

/// Decode the raw Snappy block format used for compressed demo commands.
///
/// Snappy is implemented here instead of adding a dependency so the importer
/// stays buildable offline. The format is a varint uncompressed length followed
/// by literal and copy tags. See `docs/research/demo-import.md`.
fn snappy_decompress(input: &[u8]) -> std::result::Result<Vec<u8>, String> {
    let (expected, mut position) = read_snappy_varint(input)?;
    let expected = usize::try_from(expected).map_err(|_| "Snappy length overflow".to_string())?;

    if expected > MAX_DECODE_BYTES {
        return Err(format!(
            "declares {expected} bytes, above the {MAX_DECODE_BYTES} byte decode cap"
        ));
    }

    let mut output: Vec<u8> = Vec::with_capacity(expected.min(1024 * 1024));

    while position < input.len() {
        let tag = input[position];
        position += 1;

        match tag & 0x03 {
            0 => {
                let mut length = (tag >> 2) as usize;
                if length < 60 {
                    length += 1;
                } else {
                    let extra = length - 59;
                    if position + extra > input.len() {
                        return Err("truncated literal length".to_string());
                    }
                    let mut value = 0_usize;
                    for index in 0..extra {
                        value |= (input[position + index] as usize) << (8 * index);
                    }
                    position += extra;
                    length = value
                        .checked_add(1)
                        .ok_or_else(|| "literal length overflow".to_string())?;
                }

                if position + length > input.len() {
                    return Err("truncated literal".to_string());
                }
                if output.len() + length > expected {
                    return Err("literal runs past the declared length".to_string());
                }
                output.extend_from_slice(&input[position..position + length]);
                position += length;
            }
            1 => {
                if position + 1 > input.len() {
                    return Err("truncated one byte copy".to_string());
                }
                let length = (((tag >> 2) & 0x07) as usize) + 4;
                let offset = (((tag >> 5) as usize) << 8) | input[position] as usize;
                position += 1;
                copy_snappy(&mut output, offset, length, expected)?;
            }
            2 => {
                if position + 2 > input.len() {
                    return Err("truncated two byte copy".to_string());
                }
                let length = ((tag >> 2) as usize) + 1;
                let offset = u16::from_le_bytes([input[position], input[position + 1]]) as usize;
                position += 2;
                copy_snappy(&mut output, offset, length, expected)?;
            }
            _ => {
                if position + 4 > input.len() {
                    return Err("truncated four byte copy".to_string());
                }
                let length = ((tag >> 2) as usize) + 1;
                let offset = u32::from_le_bytes([
                    input[position],
                    input[position + 1],
                    input[position + 2],
                    input[position + 3],
                ]) as usize;
                position += 4;
                copy_snappy(&mut output, offset, length, expected)?;
            }
        }
    }

    if output.len() != expected {
        return Err(format!(
            "decoded {} bytes but the block declared {expected}",
            output.len()
        ));
    }

    Ok(output)
}

fn copy_snappy(
    output: &mut Vec<u8>,
    offset: usize,
    length: usize,
    expected: usize,
) -> std::result::Result<(), String> {
    if offset == 0 || offset > output.len() {
        return Err(format!(
            "copy offset {offset} is outside the {} decoded bytes",
            output.len()
        ));
    }
    if output.len() + length > expected {
        return Err("copy runs past the declared length".to_string());
    }

    let start = output.len() - offset;
    for index in 0..length {
        let byte = output[start + index];
        output.push(byte);
    }

    Ok(())
}

fn read_snappy_varint(input: &[u8]) -> std::result::Result<(u64, usize), String> {
    let mut result = 0_u64;
    let mut shift = 0_u32;
    let mut position = 0_usize;

    loop {
        if position >= input.len() {
            return Err("truncated Snappy length".to_string());
        }
        let byte = input[position];
        position += 1;
        result |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok((result, position));
        }
        shift += 7;
        if shift >= 64 {
            return Err("Snappy length varint is too long".to_string());
        }
    }
}

// ---------------------------------------------------------------------------
// Minimal protobuf reader
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum ProtoValue<'a> {
    Varint(u64),
    Fixed32(u32),
    #[allow(dead_code)]
    Fixed64(u64),
    Bytes(&'a [u8]),
}

impl<'a> ProtoValue<'a> {
    fn string(&self) -> std::result::Result<String, String> {
        match self {
            ProtoValue::Bytes(bytes) => Ok(String::from_utf8_lossy(bytes).into_owned()),
            _ => Err("field is not length delimited".to_string()),
        }
    }

    fn bytes(&self) -> Option<&'a [u8]> {
        match self {
            ProtoValue::Bytes(bytes) => Some(bytes),
            _ => None,
        }
    }

    fn u64(&self) -> Option<u64> {
        match self {
            ProtoValue::Varint(value) => Some(*value),
            _ => None,
        }
    }

    fn i32(&self) -> Option<i32> {
        self.u64().map(|value| value as i32)
    }

    fn f32(&self) -> Option<f32> {
        match self {
            ProtoValue::Fixed32(bits) => Some(f32::from_bits(*bits)),
            _ => None,
        }
    }
}

fn read_proto_fields(data: &[u8]) -> std::result::Result<Vec<(u32, ProtoValue<'_>)>, String> {
    let mut fields = Vec::new();
    let mut position = 0_usize;

    while position < data.len() {
        let key = read_proto_varint(data, &mut position)?;
        let number = (key >> 3) as u32;
        if number == 0 {
            return Err("protobuf field number 0 is invalid".to_string());
        }
        let wire = (key & 0x07) as u8;

        let value = match wire {
            0 => ProtoValue::Varint(read_proto_varint(data, &mut position)?),
            1 => {
                if position + 8 > data.len() {
                    return Err("truncated fixed64 field".to_string());
                }
                let bytes: [u8; 8] = data[position..position + 8]
                    .try_into()
                    .map_err(|_| "truncated fixed64 field".to_string())?;
                position += 8;
                ProtoValue::Fixed64(u64::from_le_bytes(bytes))
            }
            2 => {
                let length = usize::try_from(read_proto_varint(data, &mut position)?)
                    .map_err(|_| "length delimited field is too large".to_string())?;
                if position + length > data.len() {
                    return Err("truncated length delimited field".to_string());
                }
                let bytes = &data[position..position + length];
                position += length;
                ProtoValue::Bytes(bytes)
            }
            5 => {
                if position + 4 > data.len() {
                    return Err("truncated fixed32 field".to_string());
                }
                let bytes: [u8; 4] = data[position..position + 4]
                    .try_into()
                    .map_err(|_| "truncated fixed32 field".to_string())?;
                position += 4;
                ProtoValue::Fixed32(u32::from_le_bytes(bytes))
            }
            3 | 4 => {
                return Err("protobuf groups are not used by Source 2 demo messages".to_string());
            }
            other => return Err(format!("unsupported protobuf wire type {other}")),
        };

        fields.push((number, value));
    }

    Ok(fields)
}

fn read_proto_varint(data: &[u8], position: &mut usize) -> std::result::Result<u64, String> {
    let mut result = 0_u64;
    let mut shift = 0_u32;

    loop {
        if *position >= data.len() {
            return Err("truncated protobuf varint".to_string());
        }
        let byte = data[*position];
        *position += 1;
        result |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(result);
        }
        shift += 7;
        if shift >= 64 {
            return Err("protobuf varint is longer than ten bytes".to_string());
        }
    }
}

// ---------------------------------------------------------------------------
// Demo message decoders
// ---------------------------------------------------------------------------

fn decode_file_header(payload: &[u8]) -> std::result::Result<DemoHeader, String> {
    let mut header = DemoHeader::default();

    for (number, value) in read_proto_fields(payload)? {
        match number {
            1 => header.demo_file_stamp = value.string()?,
            2 => header.patch_version = value.i32(),
            3 => header.server_name = non_empty_string(value.string()?),
            4 => header.client_name = non_empty_string(value.string()?),
            5 => header.map_name = non_empty_string(value.string()?),
            6 => header.game_directory = non_empty_string(value.string()?),
            7 => header.fullpackets_version = value.i32(),
            8 => header.allow_clientside_entities = value.u64().map(|raw| raw != 0),
            9 => header.allow_clientside_particles = value.u64().map(|raw| raw != 0),
            10 => header.addons = non_empty_string(value.string()?),
            11 => header.demo_version_name = non_empty_string(value.string()?),
            12 => header.demo_version_guid = non_empty_string(value.string()?),
            13 => header.build_num = value.i32(),
            14 => header.game = non_empty_string(value.string()?),
            15 => header.server_start_tick = value.i32(),
            _ => {}
        }
    }

    Ok(header)
}

fn decode_file_info(payload: &[u8]) -> std::result::Result<DemoFileInfo, String> {
    let mut info = DemoFileInfo::default();

    for (number, value) in read_proto_fields(payload)? {
        match number {
            1 => {
                if let Some(seconds) = value.f32()
                    && seconds.is_finite()
                    && seconds >= 0.0
                {
                    info.playback_time_seconds = Some(seconds);
                }
            }
            2 => info.playback_ticks = value.u64().map(|raw| raw as u32),
            3 => info.playback_frames = value.u64().map(|raw| raw as u32),
            4 => info.game_info_bytes = value.bytes().map(|bytes| bytes.len() as u64).unwrap_or(0),
            _ => {}
        }
    }

    Ok(info)
}

fn decode_string_tables(payload: &[u8]) -> std::result::Result<Vec<DemoStringTable>, String> {
    let mut tables = Vec::new();

    for (number, value) in read_proto_fields(payload)? {
        if number == 1
            && let Some(bytes) = value.bytes()
        {
            tables.push(decode_string_table(bytes)?);
            if tables.len() >= MAX_STRING_TABLES {
                break;
            }
        }
    }

    Ok(tables)
}

fn decode_string_table(data: &[u8]) -> std::result::Result<DemoStringTable, String> {
    let mut table = DemoStringTable::default();

    for (number, value) in read_proto_fields(data)? {
        match number {
            1 => table.name = value.string()?,
            2 => {
                table.items += 1;
                if let Some(bytes) = value.bytes() {
                    let (key, data_bytes) = decode_string_table_item(bytes)?;
                    table.data_bytes += data_bytes;
                    if table.sample_keys.len() < MAX_STRING_TABLE_SAMPLES && !key.is_empty() {
                        table.sample_keys.push(key);
                    }
                }
            }
            3 => table.clientside_items += 1,
            4 => table.table_flags = value.i32(),
            _ => {}
        }
    }

    table.truncated = table.items as usize > MAX_STRING_TABLE_SAMPLES;
    Ok(table)
}

fn decode_string_table_item(data: &[u8]) -> std::result::Result<(String, u64), String> {
    let mut key = String::new();
    let mut data_bytes = 0_u64;

    for (number, value) in read_proto_fields(data)? {
        match number {
            1 => key = value.string()?,
            2 => data_bytes = value.bytes().map(|bytes| bytes.len() as u64).unwrap_or(0),
            _ => {}
        }
    }

    Ok((key, data_bytes))
}

/// `CDemoPacket.data` is field 3 in the Source 2 proto.
fn decode_packet(payload: &[u8]) -> std::result::Result<Vec<u8>, String> {
    for (number, value) in read_proto_fields(payload)? {
        if number == 3
            && let Some(bytes) = value.bytes()
        {
            return Ok(bytes.to_vec());
        }
    }

    Ok(Vec::new())
}

fn decode_full_packet(
    payload: &[u8],
) -> std::result::Result<(Vec<DemoStringTable>, Option<Vec<u8>>), String> {
    let mut tables = Vec::new();
    let mut packet = None;

    for (number, value) in read_proto_fields(payload)? {
        match number {
            1 => {
                if let Some(bytes) = value.bytes() {
                    tables.extend(decode_string_tables(bytes)?);
                }
            }
            2 => {
                if let Some(bytes) = value.bytes() {
                    packet = Some(decode_packet(bytes)?);
                }
            }
            _ => {}
        }
    }

    Ok((tables, packet))
}

fn decode_custom_data(payload: &[u8]) -> std::result::Result<DemoCustomDataEntry, String> {
    let mut entry = DemoCustomDataEntry::default();

    for (number, value) in read_proto_fields(payload)? {
        match number {
            1 => entry.callback_index = value.i32(),
            2 => {
                if let Some(bytes) = value.bytes() {
                    entry.bytes = bytes.len() as u64;
                    entry.preview = printable_preview(bytes);
                }
            }
            _ => {}
        }
    }

    Ok(entry)
}

fn decode_custom_data_callbacks(data: &[u8]) -> std::result::Result<Vec<String>, String> {
    let mut ids = Vec::new();

    for (number, value) in read_proto_fields(data)? {
        if number == 1 {
            let id = value.string()?;
            if !id.is_empty() {
                ids.push(id);
            }
        }
    }

    Ok(ids)
}

fn printable_preview(bytes: &[u8]) -> Option<String> {
    let text: String = bytes
        .iter()
        .copied()
        .take(120)
        .filter(|byte| byte.is_ascii_graphic() || *byte == b' ')
        .map(|byte| byte as char)
        .collect();

    if text.trim().is_empty() {
        None
    } else {
        Some(text.trim().to_string())
    }
}

fn non_empty_string(value: String) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

// ---------------------------------------------------------------------------
// CDemoPacket bit framing and message inventory
// ---------------------------------------------------------------------------

struct BitCursor<'a> {
    data: &'a [u8],
    bit: usize,
}

impl<'a> BitCursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, bit: 0 }
    }

    fn remaining_bits(&self) -> usize {
        self.data.len() * 8 - self.bit.min(self.data.len() * 8)
    }

    fn read_bits(&mut self, count: usize) -> Option<u32> {
        if count > 32 || self.remaining_bits() < count {
            return None;
        }

        let mut value = 0_u32;
        for index in 0..count {
            let bit_index = self.bit + index;
            let byte = self.data[bit_index / 8];
            let bit = (byte >> (bit_index % 8)) & 1;
            value |= u32::from(bit) << index;
        }
        self.bit += count;

        Some(value)
    }

    fn skip_bits(&mut self, count: usize) -> bool {
        if self.remaining_bits() < count {
            return false;
        }
        self.bit += count;
        true
    }

    fn read_var_u32(&mut self) -> Option<u32> {
        let mut result = 0_u32;
        let mut shift = 0_u32;

        loop {
            let byte = self.read_bits(8)?;
            result |= (byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(result);
            }
            shift += 7;
            if shift >= 35 {
                return None;
            }
        }
    }

    /// Valve's variable length unsigned integer used for packet message ids.
    ///
    /// Six bits carry the base. The `0x10`, `0x20` and `0x30` markers in the
    /// top two base bits select a 4, 8 or 28 bit extension. This mirrors
    /// `ReadUBitVar` in the Source SDK and `read_ubit_var` in `source2-demo`.
    fn read_ubit_var(&mut self) -> Option<u32> {
        let mut value = self.read_bits(6)?;

        match value & 0x30 {
            0x10 => value = (value & 0x0f) | (self.read_bits(4)? << 4),
            0x20 => value = (value & 0x0f) | (self.read_bits(8)? << 4),
            0x30 => value = (value & 0x0f) | (self.read_bits(28)? << 4),
            _ => {}
        }

        Some(value)
    }
}

fn scan_packet(
    data: &[u8],
    counts: &mut BTreeMap<i32, (u64, u64)>,
    inventory: &mut DemoPacketInventory,
    notes: &mut Vec<String>,
) {
    let mut cursor = BitCursor::new(data);
    let mut messages = 0_u64;

    while cursor.remaining_bits() >= 8 {
        if messages >= MAX_PACKET_MESSAGES {
            notes.push(format!(
                "Packet message inventory was capped at {MAX_PACKET_MESSAGES} messages."
            ));
            inventory.messages += messages;
            inventory.truncated_packets += 1;
            return;
        }

        let Some(id) = cursor.read_ubit_var() else {
            inventory.messages += messages;
            inventory.truncated_packets += 1;
            return;
        };
        let Some(size) = cursor.read_var_u32() else {
            inventory.messages += messages;
            inventory.truncated_packets += 1;
            return;
        };
        let size_bits = (size as usize).saturating_mul(8);
        if !cursor.skip_bits(size_bits) {
            inventory.messages += messages;
            inventory.truncated_packets += 1;
            return;
        }

        let entry = counts.entry(id as i32).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += u64::from(size);
        messages += 1;
    }

    inventory.messages += messages;
}

// ---------------------------------------------------------------------------
// Extractor seam
// ---------------------------------------------------------------------------

/// Which source a merged field came from.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FieldSource {
    /// Parsed from the demo itself.
    Demo,
    /// Read from the cached `api.json`.
    Api,
    /// Test or UI development fixture. Never a real capture.
    Fixture,
    /// Computed locally from other values, such as KDA from kills and deaths.
    Derived,
}

impl FieldSource {
    pub fn label(self) -> &'static str {
        match self {
            FieldSource::Demo => "demo",
            FieldSource::Api => "api",
            FieldSource::Fixture => "fixture",
            FieldSource::Derived => "derived",
        }
    }
}

/// How much a merged field should be trusted.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    High,
    Medium,
    Low,
}

/// One merged field with its source and confidence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FieldValue<T> {
    pub value: T,
    pub source: FieldSource,
    pub confidence: Confidence,
}

impl<T> FieldValue<T> {
    pub fn new(value: T, source: FieldSource, confidence: Confidence) -> Self {
        Self {
            value,
            source,
            confidence,
        }
    }
}

/// Match level facts produced by an extractor.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedMatchFacts {
    pub map: Option<String>,
    pub duration_seconds: Option<u64>,
    pub duration_ticks: Option<u64>,
    pub result: Option<String>,
    pub start_time_unix: Option<i64>,
    pub game_mode: Option<String>,
    pub build: Option<String>,
    pub server_name: Option<String>,
    pub rank: Option<String>,
    pub mmr: Option<f64>,
    pub population_percentile: Option<f64>,
}

/// One player produced by an extractor.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedPlayer {
    pub account_id: Option<u64>,
    pub name: Option<String>,
    pub hero: Option<String>,
    pub hero_id: Option<u32>,
    pub team: Option<i32>,
    pub kills: Option<u64>,
    pub deaths: Option<u64>,
    pub assists: Option<u64>,
    pub net_worth: Option<u64>,
    pub badge: Option<u32>,
    pub mmr: Option<f64>,
    pub population_percentile: Option<f64>,
    pub result: Option<String>,
}

/// One event in the match timeline.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DemoEvent {
    /// Position in the normalized timeline, assigned by the importer.
    pub sequence: u64,
    pub tick: Option<u64>,
    pub time_seconds: Option<f64>,
    /// `kill`, `death`, `objective`, `item` or a backend specific label.
    pub kind: String,
    pub actor: Option<String>,
    pub target: Option<String>,
    #[serde(default)]
    pub assists: Vec<String>,
    pub team: Option<i32>,
    pub position_x: Option<f64>,
    pub position_y: Option<f64>,
    pub detail: Option<String>,
}

/// One position sample. Only a position produced by a real parse belongs here.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DemoPosition {
    pub time_seconds: Option<f64>,
    pub account_id: Option<u64>,
    pub player: Option<String>,
    pub team: Option<i32>,
    /// `position`, `death`, `kill` or a backend specific label.
    pub kind: String,
    pub x: f64,
    pub y: f64,
    pub z: Option<f64>,
}

/// KDA counted from resolved kill events.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KdaCounts {
    pub kills: u64,
    pub deaths: u64,
    pub assists: u64,
}

/// KDA derivation and its unresolved sample count.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KdaDerivation {
    /// Keyed by [`player_key`].
    pub counts: BTreeMap<String, KdaCounts>,
    pub resolved_kills: u64,
    pub unresolved_kills: u64,
}

/// Everything an extractor produced for one demo.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DemoExtract {
    pub extractor: String,
    pub source: FieldSource,
    pub match_facts: ExtractedMatchFacts,
    pub players: Vec<ExtractedPlayer>,
    pub events: Vec<DemoEvent>,
    pub positions: Vec<DemoPosition>,
    pub derived_kda: KdaDerivation,
    pub missing_fields: Vec<String>,
    pub notes: Vec<String>,
}

/// A pluggable demo backend.
///
/// A serializer aware backend implements this trait to add entity state,
/// Citadel game events, positions and match metadata. The importer, the store
/// and the summary merge do not change.
pub trait DemoExtractor {
    /// Stable name recorded in provenance.
    fn name(&self) -> &'static str;
    /// Extractor version recorded in provenance.
    fn version(&self) -> &'static str {
        IMPORTER_VERSION
    }
    /// Extract what this backend can prove from the parsed container.
    fn extract(&self, demo: &DemoFile) -> Result<DemoExtract>;
}

/// The shipped extractor: header, file info, string tables, packet inventory.
///
/// It never invents players, events or positions. Those fields are reported as
/// missing with their reasons so the UI can label the match as container only.
#[derive(Debug, Clone, Copy, Default)]
pub struct ContainerExtractor;

impl DemoExtractor for ContainerExtractor {
    fn name(&self) -> &'static str {
        "container"
    }

    fn extract(&self, demo: &DemoFile) -> Result<DemoExtract> {
        let header = demo.header.as_ref();
        let info = demo.file_info.as_ref();

        let map = header.and_then(|value| value.map_name.clone());
        let duration_seconds = info
            .and_then(|value| value.playback_time_seconds)
            .filter(|value| value.is_finite() && *value >= 0.0)
            .map(|value| value.round() as u64);
        let duration_ticks = info
            .and_then(|value| value.playback_ticks)
            .filter(|value| *value > 0)
            .map(u64::from);
        let server_name = header.and_then(|value| value.server_name.clone());
        let build = header
            .and_then(|value| value.build_num)
            .map(|value| value.to_string());

        let match_facts = ExtractedMatchFacts {
            map,
            duration_seconds,
            duration_ticks,
            result: None,
            start_time_unix: None,
            game_mode: None,
            build,
            server_name,
            rank: None,
            mmr: None,
            population_percentile: None,
        };

        let mut missing = Vec::new();
        if demo.header.is_none() {
            missing.push("header".to_string());
        }
        if match_facts.map.is_none() {
            missing.push("map".to_string());
        }
        if match_facts.duration_seconds.is_none() {
            missing.push("durationSeconds".to_string());
        }
        if match_facts.duration_ticks.is_none() {
            missing.push("durationTicks".to_string());
        }
        for field in [
            "result",
            "players",
            "kills",
            "deaths",
            "assists",
            "events",
            "positions",
            "heatmap",
        ] {
            missing.push(field.to_string());
        }
        missing.sort();
        missing.dedup();

        let mut notes = vec![
            "Container extractor decoded the demo framing, header, file info, string tables and packet message ids."
                .to_string(),
            format!(
                "Packets: {} carrying {} embedded message(s). Ids are inventoried, not decoded.",
                demo.packets.packets, demo.packets.messages
            ),
        ];

        if let Some(info) = info {
            notes.push(format!(
                "Playback: {} tick(s), {} frame(s).",
                info.playback_ticks.unwrap_or(0),
                info.playback_frames.unwrap_or(0)
            ));
        }
        if !demo.user_messages.is_empty() {
            let known = demo
                .user_messages
                .iter()
                .filter(|entry| entry.name.is_some())
                .map(|entry| {
                    format!(
                        "{} (id {}, {} message(s))",
                        entry.name.as_deref().unwrap_or("unknown"),
                        entry.id,
                        entry.count
                    )
                })
                .collect::<Vec<_>>();
            if !known.is_empty() {
                notes.push(format!(
                    "Recognised user messages: {}. Payloads stay unparsed.",
                    known.join(", ")
                ));
            }
        }

        Ok(DemoExtract {
            extractor: self.name().to_string(),
            source: FieldSource::Demo,
            match_facts,
            players: Vec::new(),
            events: Vec::new(),
            positions: Vec::new(),
            derived_kda: KdaDerivation::default(),
            missing_fields: missing,
            notes,
        })
    }
}

/// Availability of demo backends. Mirrors the honesty of `toolchain.rs`: an
/// unavailable backend carries the reason and never pretends to work.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DemoBackendInfo {
    pub name: String,
    pub available: bool,
    pub test_only: bool,
    pub provides: Vec<String>,
    pub missing: Vec<String>,
    pub reason: Option<String>,
}

/// Which demo extraction lanes exist on this build.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DemoBackendStatus {
    pub platform: String,
    pub extractors: Vec<DemoBackendInfo>,
    pub event_extraction_available: bool,
    pub position_extraction_available: bool,
    pub heatmap_available: bool,
    pub reason: Option<String>,
    pub notes: Vec<String>,
}

/// Report the demo backend lanes on this build.
pub fn backend_status() -> DemoBackendStatus {
    let position_reason = "No serializer aware demo parser is linked. Source 2 entity state is delta encoded against per-class baselines from DEM_SendTables and DEM_ClassInfo, so the container parser cannot recover positions, kills or objectives.";

    DemoBackendStatus {
        platform: std::env::consts::OS.to_string(),
        extractors: vec![
            DemoBackendInfo {
                name: "container".to_string(),
                available: true,
                test_only: false,
                provides: vec![
                    "magic and framing".to_string(),
                    "CDemoFileHeader".to_string(),
                    "CDemoFileInfo".to_string(),
                    "string table inventory".to_string(),
                    "packet user message inventory".to_string(),
                    "provenance and coverage".to_string(),
                ],
                missing: vec![
                    "players".to_string(),
                    "kills, deaths, assists".to_string(),
                    "events".to_string(),
                    "positions".to_string(),
                    "heatmap".to_string(),
                ],
                reason: None,
            },
            DemoBackendInfo {
                name: "fixture".to_string(),
                available: true,
                test_only: true,
                provides: vec![
                    "documented JSON players, events and positions".to_string(),
                    "UI development data".to_string(),
                ],
                missing: Vec::new(),
                reason: Some(
                    "Tests and UI development only. Every field is recorded with source fixture."
                        .to_string(),
                ),
            },
            DemoBackendInfo {
                name: "source2-demo".to_string(),
                available: false,
                test_only: false,
                provides: vec![
                    "serializers, entities, Citadel game events".to_string(),
                    "positions and match metadata".to_string(),
                ],
                missing: Vec::new(),
                reason: Some(
                    "Not linked. It is a Rust library, so wiring it is a dependency decision, not an install step. The deadlock feature also needs the Citadel proto set kept in sync with the game build."
                        .to_string(),
                ),
            },
            DemoBackendInfo {
                name: "deadlock-api demo analyzer".to_string(),
                available: false,
                test_only: false,
                provides: vec!["remote demo parsing".to_string()],
                missing: Vec::new(),
                reason: Some(
                    "Not used. It uploads the demo to a third party, which the local first rule forbids without an explicit opt in."
                        .to_string(),
                ),
            },
        ],
        event_extraction_available: false,
        position_extraction_available: false,
        heatmap_available: false,
        reason: Some(position_reason.to_string()),
        notes: vec![
            "The container import is useful for match metadata, duration, map and provenance."
                .to_string(),
            "Run `Sigil cli demo backends --json` for machine-readable status.".to_string(),
        ],
    }
}

/// Human-readable backend report.
pub fn format_backend_status(status: &DemoBackendStatus) -> String {
    let mut lines = vec![format!("Demo backends on {}", status.platform)];

    for extractor in &status.extractors {
        let state = if extractor.available {
            if extractor.test_only {
                "available (tests and UI development only)".to_string()
            } else {
                "available".to_string()
            }
        } else {
            "unavailable".to_string()
        };
        lines.push(format!("  {}: {state}", extractor.name));

        if !extractor.provides.is_empty() {
            lines.push(format!("    Provides: {}", extractor.provides.join(", ")));
        }
        if !extractor.missing.is_empty() {
            lines.push(format!("    Missing: {}", extractor.missing.join(", ")));
        }
        if let Some(reason) = &extractor.reason {
            lines.push(format!("    Reason: {reason}"));
        }
    }

    lines.push(format!(
        "Event extraction: {}",
        available_label(status.event_extraction_available)
    ));
    lines.push(format!(
        "Position extraction: {}",
        available_label(status.position_extraction_available)
    ));
    lines.push(format!(
        "Heatmap: {}",
        available_label(status.heatmap_available)
    ));
    if let Some(reason) = &status.reason {
        lines.push(format!("  {reason}"));
    }

    lines.join("\n")
}

fn available_label(available: bool) -> &'static str {
    if available {
        "available"
    } else {
        "unavailable"
    }
}

// ---------------------------------------------------------------------------
// Summary model
// ---------------------------------------------------------------------------

/// Present, partial or absent for one summary field.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CoverageState {
    Present,
    Partial,
    Absent,
}

/// Per-field coverage. The UI labels matches with this instead of implying
/// that a missing number is zero.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Coverage {
    pub fields: BTreeMap<String, CoverageState>,
    pub notes: Vec<String>,
}

/// How the API record was joined to the demo.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MatchJoin {
    pub key: String,
    pub inferred: bool,
    pub note: Option<String>,
}

/// A conflict between the demo and the API. Conflicts are recorded, never
/// resolved silently.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Disagreement {
    pub field: String,
    pub demo: Option<Value>,
    pub api: Option<Value>,
    /// Which value the merged summary kept.
    pub resolution: String,
    pub note: Option<String>,
}

/// Match level merged fields.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MergedMatchInfo {
    pub map: Option<FieldValue<String>>,
    pub duration_seconds: Option<FieldValue<u64>>,
    pub duration_ticks: Option<u64>,
    pub result: Option<FieldValue<String>>,
    pub start_time_unix: Option<FieldValue<i64>>,
    pub game_mode: Option<FieldValue<String>>,
    pub build: Option<FieldValue<String>>,
    pub server_name: Option<FieldValue<String>>,
    pub rank: Option<FieldValue<String>>,
    pub mmr: Option<FieldValue<f64>>,
    pub population_percentile: Option<FieldValue<f64>>,
}

/// One merged player row.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MergedPlayer {
    pub key: String,
    pub account_id: Option<FieldValue<u64>>,
    pub name: Option<FieldValue<String>>,
    pub hero: Option<FieldValue<String>>,
    pub hero_id: Option<FieldValue<u32>>,
    pub team: Option<FieldValue<i32>>,
    pub kills: Option<FieldValue<u64>>,
    pub deaths: Option<FieldValue<u64>>,
    pub assists: Option<FieldValue<u64>>,
    pub kda: Option<FieldValue<f64>>,
    pub net_worth: Option<FieldValue<u64>>,
    pub badge: Option<FieldValue<u32>>,
    pub mmr: Option<FieldValue<f64>>,
    pub population_percentile: Option<FieldValue<f64>>,
    pub result: Option<FieldValue<String>>,
    pub evidence: PlayerEvidence,
    /// True when the demo row was joined to the API row by name instead of by
    /// account id.
    pub join_inferred: bool,
}

/// Sample counts behind a player row.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlayerEvidence {
    pub event_samples: u64,
    pub position_samples: u64,
}

/// One heatmap cell. The coordinate is the south west corner of the cell.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HeatCell {
    pub x: i32,
    pub y: i32,
    pub samples: u64,
}

/// Heatmap aggregate or an explicit absence.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Heatmap {
    pub available: bool,
    /// `deaths` when death positions were used, else `positions`.
    pub basis: Option<String>,
    pub cell_size: f64,
    pub samples: u64,
    pub cells: Vec<HeatCell>,
    pub source: Option<FieldSource>,
    pub reason: Option<String>,
}

/// Merged API plus demo match summary written as `summary.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MatchSummaryDoc {
    pub schema: String,
    pub match_id: String,
    pub generated_at: String,
    /// `demo`, `fixture`, `demo+api` or `fixture+api`.
    pub source: String,
    pub join: MatchJoin,
    pub match_info: MergedMatchInfo,
    pub players: Vec<MergedPlayer>,
    pub timeline: Vec<DemoEvent>,
    pub heatmap: Heatmap,
    pub coverage: Coverage,
    pub disagreements: Vec<Disagreement>,
    pub notes: Vec<String>,
}

/// Compact metadata for match lists.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MatchMeta {
    pub schema: String,
    pub match_id: String,
    pub source: String,
    pub imported_at: String,
    pub updated_at: String,
    pub revision: u64,
    pub map: Option<String>,
    pub duration_seconds: Option<u64>,
    pub duration_ticks: Option<u64>,
    pub result: Option<String>,
    pub player_count: usize,
    pub event_count: usize,
    pub has_demo: bool,
    pub has_fixture: bool,
    pub has_api: bool,
    pub position_samples: usize,
    pub heatmap_cells: usize,
    pub missing_fields: Vec<String>,
}

/// Source facts recorded in provenance.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProvenanceSource {
    /// `demo` or `fixture`.
    pub kind: String,
    pub path: String,
    pub byte_len: u64,
    /// Lower-case hex CRC32 of the stored source bytes.
    pub crc32: String,
    /// `requested`, `fileStem`, `fixture` or `crc32`.
    pub match_id_source: String,
}

/// Demo specific provenance.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DemoProvenance {
    pub magic: String,
    pub patch_version: Option<i32>,
    pub demo_version_name: Option<String>,
    pub fullpackets_version: Option<i32>,
    pub commands_total: u64,
    pub compressed_commands: u64,
    pub compressed_decoded: u64,
    pub unknown_commands: u64,
    pub file_info_offset: Option<u32>,
    pub stop_seen: bool,
    pub playback_ticks: Option<u32>,
    pub playback_time_seconds: Option<f32>,
    pub string_tables: usize,
    pub user_messages: usize,
    pub custom_data_entries: usize,
    pub notes: Vec<String>,
}

/// Cached API provenance.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ApiProvenance {
    pub path: String,
    pub byte_len: u64,
    pub fetched_at: Option<String>,
    pub match_id: Option<u64>,
    pub merged: bool,
    pub fields_present: Vec<String>,
    pub parse_error: Option<String>,
}

/// `provenance.json`. This is the record that answers "where did this number
/// come from" and "what was not extracted".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
    pub schema: String,
    pub importer: String,
    pub importer_version: String,
    pub extractor: String,
    pub source: ProvenanceSource,
    pub imported_at: String,
    pub updated_at: String,
    /// Timestamp of the parse that produced this record.
    #[serde(default)]
    pub parsed_at: String,
    pub revision: u64,
    pub byte_len: u64,
    pub crc32: String,
    pub demo: Option<DemoProvenance>,
    pub api: Option<ApiProvenance>,
    pub missing_fields: Vec<String>,
    pub unsupported: Vec<String>,
    pub prior_review_preserved: bool,
    pub notes: Vec<String>,
}

/// Result of one import or re-import.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub match_id: String,
    pub directory: PathBuf,
    /// True when the match folder already existed.
    pub existed: bool,
    pub review_preserved: bool,
    pub meta: MatchMeta,
    pub summary: MatchSummaryDoc,
    pub provenance: Provenance,
}

/// One row in `dah demo list`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MatchListItem {
    pub match_id: String,
    pub directory: PathBuf,
    pub source: String,
    pub map: Option<String>,
    pub duration_seconds: Option<u64>,
    pub result: Option<String>,
    pub player_count: usize,
    pub event_count: usize,
    pub heatmap_cells: usize,
    pub has_api: bool,
    pub revision: u64,
    pub updated_at: String,
    pub missing_fields: Vec<String>,
}

/// `dah demo list` payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MatchList {
    pub workspace: PathBuf,
    pub matches: Vec<MatchListItem>,
    /// Folders under `matches/` that had no readable `meta.json` or
    /// `summary.json`.
    pub skipped: usize,
}

// ---------------------------------------------------------------------------
// Summary merge
// ---------------------------------------------------------------------------

fn merge_field<T>(
    field: &str,
    demo: Option<FieldValue<T>>,
    api: Option<FieldValue<T>>,
    disagreements: &mut Vec<Disagreement>,
    note: Option<&str>,
) -> Option<FieldValue<T>>
where
    T: PartialEq + Serialize,
{
    match (demo, api) {
        (Some(demo_value), Some(api_value)) => {
            if demo_value.value != api_value.value {
                disagreements.push(Disagreement {
                    field: field.to_string(),
                    demo: serde_json::to_value(&demo_value.value).ok(),
                    api: serde_json::to_value(&api_value.value).ok(),
                    resolution: demo_value.source.label().to_string(),
                    note: note.map(str::to_string),
                });
            }
            Some(demo_value)
        }
        (Some(demo_value), None) => Some(demo_value),
        (None, Some(api_value)) => Some(api_value),
        (None, None) => None,
    }
}

fn build_heatmap(extract: &DemoExtract, notes: &mut Vec<String>) -> Heatmap {
    let mut usable: Vec<&DemoPosition> = extract
        .positions
        .iter()
        .filter(|position| position.x.is_finite() && position.y.is_finite())
        .collect();

    if usable.len() > MAX_POSITIONS {
        notes.push(format!(
            "Heatmap used the first {MAX_POSITIONS} of {} position samples.",
            usable.len()
        ));
        usable.truncate(MAX_POSITIONS);
    }

    if usable.is_empty() {
        return Heatmap {
            available: false,
            basis: None,
            cell_size: HEATMAP_CELL_SIZE,
            samples: 0,
            cells: Vec::new(),
            source: None,
            reason: Some(
                "No position samples were produced by the extractor. The container backend cannot recover entity state, so no heatmap is drawn."
                    .to_string(),
            ),
        };
    }

    let deaths: Vec<&DemoPosition> = usable
        .iter()
        .copied()
        .filter(|position| position.kind.eq_ignore_ascii_case("death"))
        .collect();
    let (basis, points) = if deaths.is_empty() {
        ("positions", usable)
    } else {
        ("deaths", deaths)
    };

    let mut cells: BTreeMap<(i32, i32), u64> = BTreeMap::new();
    for position in &points {
        let cell_x = (position.x / HEATMAP_CELL_SIZE).floor() as i32;
        let cell_y = (position.y / HEATMAP_CELL_SIZE).floor() as i32;
        *cells.entry((cell_x, cell_y)).or_insert(0) += 1;
    }

    let mut cells: Vec<HeatCell> = cells
        .into_iter()
        .map(|((cell_x, cell_y), samples)| HeatCell {
            x: (cell_x as f64 * HEATMAP_CELL_SIZE) as i32,
            y: (cell_y as f64 * HEATMAP_CELL_SIZE) as i32,
            samples,
        })
        .collect();
    cells.sort_by(|left, right| {
        right
            .samples
            .cmp(&left.samples)
            .then(left.x.cmp(&right.x))
            .then(left.y.cmp(&right.y))
    });

    Heatmap {
        available: true,
        basis: Some(basis.to_string()),
        cell_size: HEATMAP_CELL_SIZE,
        samples: points.len() as u64,
        cells,
        source: Some(extract.source),
        reason: None,
    }
}

fn build_players(
    extract: &DemoExtract,
    api: Option<&ApiFacts>,
    disagreements: &mut Vec<Disagreement>,
) -> Vec<MergedPlayer> {
    let demo_source = extract.source;
    let mut demo_by_key: BTreeMap<String, &ExtractedPlayer> = BTreeMap::new();
    let mut demo_by_name: BTreeMap<String, String> = BTreeMap::new();

    for player in &extract.players {
        let key = player_key(
            player.account_id,
            player.name.as_deref(),
            player.hero.as_deref(),
            player.team,
        );
        if let Some(name) = player.name.as_deref() {
            demo_by_name.insert(normalize_name(name), key.clone());
        }
        demo_by_key.insert(key, player);
    }

    let mut api_by_key: BTreeMap<String, &ApiPlayerFacts> = BTreeMap::new();
    let mut api_by_name: BTreeMap<String, String> = BTreeMap::new();
    if let Some(api) = api {
        for player in &api.players {
            let key = player_key(
                player.account_id,
                player.name.as_deref(),
                player.hero.as_deref(),
                player.team,
            );
            if let Some(name) = player.name.as_deref() {
                api_by_name.insert(normalize_name(name), key.clone());
            }
            api_by_key.insert(key, player);
        }
    }

    // When the demo row has no account id, fall back to a normalized name join
    // and mark the row as inferred. This is the documented join fallback.
    let mut aliases: BTreeMap<String, String> = BTreeMap::new();
    let mut consumed_api_keys: BTreeSet<String> = BTreeSet::new();
    for (demo_key, player) in &demo_by_key {
        if player.account_id.is_some() {
            continue;
        }
        let Some(name) = player.name.as_deref() else {
            continue;
        };
        if let Some(api_key) = api_by_name.get(&normalize_name(name))
            && api_key != demo_key
        {
            aliases.insert(demo_key.clone(), api_key.clone());
            consumed_api_keys.insert(api_key.clone());
        }
    }

    let mut keys: BTreeSet<String> = demo_by_key.keys().cloned().collect();
    for key in api_by_key.keys() {
        if !consumed_api_keys.contains(key) {
            keys.insert(key.clone());
        }
    }

    let mut players = Vec::new();

    for key in keys {
        let demo_player = demo_by_key.get(&key).copied();
        let api_key = aliases.get(&key).cloned().unwrap_or_else(|| key.clone());
        let api_player = api_by_key.get(&api_key).copied();
        let join_inferred = aliases.contains_key(&key);
        let prefix = format!("players.{key}");

        macro_rules! merged {
            ($field:literal, $demo:expr, $api:expr) => {{
                let field_name = format!("{prefix}.{}", $field);
                merge_field(&field_name, $demo, $api, disagreements, None)
            }};
        }

        let account_id = merged!(
            "accountId",
            demo_player
                .and_then(|player| player.account_id)
                .map(|value| FieldValue::new(value, demo_source, Confidence::High)),
            api_player
                .and_then(|player| player.account_id)
                .map(|value| FieldValue::new(value, FieldSource::Api, Confidence::Medium))
        );
        let name = merged!(
            "name",
            demo_player
                .and_then(|player| player.name.clone())
                .map(|value| FieldValue::new(value, demo_source, Confidence::High)),
            api_player
                .and_then(|player| player.name.clone())
                .map(|value| FieldValue::new(value, FieldSource::Api, Confidence::Medium))
        );
        let hero = merged!(
            "hero",
            demo_player
                .and_then(|player| player.hero.clone())
                .map(|value| FieldValue::new(value, demo_source, Confidence::High)),
            api_player
                .and_then(|player| player.hero.clone())
                .map(|value| FieldValue::new(value, FieldSource::Api, Confidence::Medium))
        );
        let hero_id = merged!(
            "heroId",
            demo_player
                .and_then(|player| player.hero_id)
                .map(|value| FieldValue::new(value, demo_source, Confidence::High)),
            api_player
                .and_then(|player| player.hero_id)
                .map(|value| FieldValue::new(value, FieldSource::Api, Confidence::Medium))
        );
        let team = merged!(
            "team",
            demo_player
                .and_then(|player| player.team)
                .map(|value| FieldValue::new(value, demo_source, Confidence::High)),
            api_player
                .and_then(|player| player.team)
                .map(|value| FieldValue::new(value, FieldSource::Api, Confidence::Medium))
        );

        let derived = extract.derived_kda.counts.get(&key).copied();
        let use_derived = extract.derived_kda.resolved_kills > 0 && derived.is_some();
        let count_confidence = if use_derived {
            Confidence::Medium
        } else {
            Confidence::High
        };

        let kills = merged!(
            "kills",
            demo_player
                .and_then(|player| {
                    if use_derived {
                        derived.map(|counts| counts.kills)
                    } else {
                        player.kills
                    }
                })
                .map(|value| FieldValue::new(value, demo_source, count_confidence)),
            api_player
                .and_then(|player| player.kills)
                .map(|value| FieldValue::new(value, FieldSource::Api, Confidence::Medium))
        );
        let deaths = merged!(
            "deaths",
            demo_player
                .and_then(|player| {
                    if use_derived {
                        derived.map(|counts| counts.deaths)
                    } else {
                        player.deaths
                    }
                })
                .map(|value| FieldValue::new(value, demo_source, count_confidence)),
            api_player
                .and_then(|player| player.deaths)
                .map(|value| FieldValue::new(value, FieldSource::Api, Confidence::Medium))
        );
        let assists = merged!(
            "assists",
            demo_player
                .and_then(|player| {
                    if use_derived {
                        derived.map(|counts| counts.assists)
                    } else {
                        player.assists
                    }
                })
                .map(|value| FieldValue::new(value, demo_source, count_confidence)),
            api_player
                .and_then(|player| player.assists)
                .map(|value| FieldValue::new(value, FieldSource::Api, Confidence::Medium))
        );

        let kda = match (&kills, &deaths, &assists) {
            (Some(kills), Some(deaths), Some(assists)) => Some(FieldValue::new(
                round_two((kills.value + assists.value) as f64 / deaths.value.max(1) as f64),
                FieldSource::Derived,
                count_confidence,
            )),
            _ => None,
        };

        let net_worth = merged!(
            "netWorth",
            demo_player
                .and_then(|player| player.net_worth)
                .map(|value| FieldValue::new(value, demo_source, Confidence::High)),
            api_player
                .and_then(|player| player.net_worth)
                .map(|value| FieldValue::new(value, FieldSource::Api, Confidence::Medium))
        );
        let badge = merged!(
            "badge",
            demo_player
                .and_then(|player| player.badge)
                .map(|value| FieldValue::new(value, demo_source, Confidence::High)),
            api_player
                .and_then(|player| player.badge)
                .map(|value| FieldValue::new(value, FieldSource::Api, Confidence::High))
        );
        let mmr = merged!(
            "mmr",
            demo_player
                .and_then(|player| player.mmr)
                .map(|value| FieldValue::new(value, demo_source, Confidence::High)),
            api_player
                .and_then(|player| player.mmr)
                .map(|value| FieldValue::new(value, FieldSource::Api, Confidence::High))
        );
        let population_percentile = merged!(
            "populationPercentile",
            demo_player
                .and_then(|player| player.population_percentile)
                .map(|value| FieldValue::new(value, demo_source, Confidence::Medium)),
            api_player
                .and_then(|player| player.population_percentile)
                .map(|value| FieldValue::new(value, FieldSource::Api, Confidence::High))
        );
        let result = merged!(
            "result",
            demo_player
                .and_then(|player| player.result.clone())
                .map(|value| FieldValue::new(value, demo_source, Confidence::High)),
            api_player
                .and_then(|player| player.result.clone())
                .map(|value| FieldValue::new(value, FieldSource::Api, Confidence::Medium))
        );

        let player_name = name
            .as_ref()
            .map(|value| value.value.clone())
            .or_else(|| demo_player.and_then(|player| player.name.clone()));
        let account = account_id.as_ref().map(|value| value.value);
        let event_samples = count_player_events(extract, player_name.as_deref(), account);
        let position_samples = count_player_positions(extract, player_name.as_deref(), account);

        players.push(MergedPlayer {
            key: key.clone(),
            account_id,
            name,
            hero,
            hero_id,
            team,
            kills,
            deaths,
            assists,
            kda,
            net_worth,
            badge,
            mmr,
            population_percentile,
            result,
            evidence: PlayerEvidence {
                event_samples,
                position_samples,
            },
            join_inferred,
        });
    }

    players
}

fn build_summary(
    match_id: &str,
    extract: &DemoExtract,
    api: Option<&ApiFacts>,
    api_merged: bool,
    api_id_mismatch: Option<(String, u64)>,
    join: MatchJoin,
    generated_at: &str,
    extra_notes: &[String],
) -> MatchSummaryDoc {
    let mut disagreements = Vec::new();
    let mut notes: Vec<String> = extract.notes.clone();
    notes.extend(extra_notes.iter().cloned());

    if let Some((demo_id, api_id)) = api_id_mismatch {
        disagreements.push(Disagreement {
            field: "match.id".to_string(),
            demo: Some(json!(demo_id)),
            api: Some(json!(api_id)),
            resolution: "demo".to_string(),
            note: Some(
                "api.json was not merged because its match id differs from the demo store id."
                    .to_string(),
            ),
        });
    }

    let source = extract.source;
    let confidence = Confidence::High;
    let api_confidence = Confidence::Medium;

    let map = merge_field(
        "match.map",
        extract
            .match_facts
            .map
            .clone()
            .map(|value| FieldValue::new(value, source, confidence)),
        api.and_then(|facts| facts.map.clone())
            .map(|value| FieldValue::new(value, FieldSource::Api, api_confidence)),
        &mut disagreements,
        None,
    );
    let duration_seconds = merge_field(
        "match.durationSeconds",
        extract
            .match_facts
            .duration_seconds
            .map(|value| FieldValue::new(value, source, confidence)),
        api.and_then(|facts| facts.duration_seconds)
            .map(|value| FieldValue::new(value, FieldSource::Api, api_confidence)),
        &mut disagreements,
        Some(
            "The demo playback time and the server match duration can differ by a few seconds. The demo value is kept.",
        ),
    );
    let result = merge_field(
        "match.result",
        extract
            .match_facts
            .result
            .clone()
            .map(|value| FieldValue::new(value, source, confidence)),
        api.and_then(|facts| facts.result.clone())
            .map(|value| FieldValue::new(value, FieldSource::Api, api_confidence)),
        &mut disagreements,
        None,
    );
    let start_time_unix = merge_field(
        "match.startTimeUnix",
        extract
            .match_facts
            .start_time_unix
            .map(|value| FieldValue::new(value, source, confidence)),
        api.and_then(|facts| facts.start_time_unix)
            .map(|value| FieldValue::new(value, FieldSource::Api, api_confidence)),
        &mut disagreements,
        None,
    );
    let game_mode = merge_field(
        "match.gameMode",
        extract
            .match_facts
            .game_mode
            .clone()
            .map(|value| FieldValue::new(value, source, confidence)),
        api.and_then(|facts| facts.game_mode.clone())
            .map(|value| FieldValue::new(value, FieldSource::Api, api_confidence)),
        &mut disagreements,
        None,
    );
    let build = merge_field(
        "match.build",
        extract
            .match_facts
            .build
            .clone()
            .map(|value| FieldValue::new(value, source, confidence)),
        api.and_then(|facts| facts.build.clone())
            .map(|value| FieldValue::new(value, FieldSource::Api, api_confidence)),
        &mut disagreements,
        None,
    );
    let server_name = extract
        .match_facts
        .server_name
        .clone()
        .map(|value| FieldValue::new(value, source, confidence));

    // Rank, MMR and population context are API wins by the join rules.
    let rank = api
        .and_then(|facts| facts.rank.clone())
        .map(|value| FieldValue::new(value, FieldSource::Api, confidence));
    let mmr = api
        .and_then(|facts| facts.mmr)
        .map(|value| FieldValue::new(value, FieldSource::Api, confidence));
    let population_percentile = api
        .and_then(|facts| facts.population_percentile)
        .map(|value| FieldValue::new(value, FieldSource::Api, confidence));

    let players = build_players(extract, api, &mut disagreements);

    let (timeline, truncated) = normalize_events(extract.events.clone());
    if truncated {
        notes.push(format!(
            "The event timeline was capped at {MAX_EVENTS} entries."
        ));
    }

    let heatmap = build_heatmap(extract, &mut notes);
    if heatmap.available {
        notes.push(format!(
            "Heatmap uses {} {} sample(s) in {} cell(s) of {} world units.",
            heatmap.samples,
            heatmap.basis.as_deref().unwrap_or("position"),
            heatmap.cells.len(),
            heatmap.cell_size
        ));
    }

    let mut coverage = Coverage::default();
    coverage
        .fields
        .insert("match.id".to_string(), CoverageState::Present);
    set_optional_coverage(&mut coverage, "match.map", map.is_some());
    if duration_seconds.is_some() {
        coverage
            .fields
            .insert("match.durationSeconds".to_string(), CoverageState::Present);
    } else if extract.match_facts.duration_ticks.is_some() {
        coverage
            .fields
            .insert("match.durationSeconds".to_string(), CoverageState::Partial);
        coverage.notes.push(
            "match.durationSeconds: partial. The demo reports playback ticks but no playback time, and the tick rate is not assumed."
                .to_string(),
        );
    } else {
        coverage
            .fields
            .insert("match.durationSeconds".to_string(), CoverageState::Absent);
    }
    set_optional_coverage(&mut coverage, "match.result", result.is_some());
    set_optional_coverage(
        &mut coverage,
        "match.startTimeUnix",
        start_time_unix.is_some(),
    );
    set_optional_coverage(&mut coverage, "match.gameMode", game_mode.is_some());
    set_optional_coverage(&mut coverage, "match.build", build.is_some());
    set_optional_coverage(&mut coverage, "match.serverName", server_name.is_some());
    set_optional_coverage(&mut coverage, "match.rank", rank.is_some());
    set_optional_coverage(&mut coverage, "match.mmr", mmr.is_some());
    set_optional_coverage(
        &mut coverage,
        "match.populationPercentile",
        population_percentile.is_some(),
    );

    coverage.fields.insert(
        "players".to_string(),
        if players.is_empty() {
            CoverageState::Absent
        } else {
            CoverageState::Present
        },
    );

    let kda_fields = players.iter().filter(|player| player.kda.is_some()).count();
    if kda_fields == 0 {
        coverage
            .fields
            .insert("players.kda".to_string(), CoverageState::Absent);
    } else if kda_fields < players.len() {
        coverage
            .fields
            .insert("players.kda".to_string(), CoverageState::Partial);
        coverage.notes.push(format!(
            "players.kda: partial. KDA is present for {kda_fields} of {} player row(s).",
            players.len()
        ));
    } else if extract.derived_kda.unresolved_kills > 0 {
        coverage
            .fields
            .insert("players.kda".to_string(), CoverageState::Partial);
        coverage.notes.push(format!(
            "players.kda: partial. {} kill event(s) could not be attributed to a player.",
            extract.derived_kda.unresolved_kills
        ));
    } else if extract.derived_kda.resolved_kills > 0 {
        coverage
            .fields
            .insert("players.kda".to_string(), CoverageState::Present);
        coverage.notes.push(format!(
            "players.kda: present. Counted from {} resolved kill event(s).",
            extract.derived_kda.resolved_kills
        ));
    } else {
        coverage
            .fields
            .insert("players.kda".to_string(), CoverageState::Present);
        coverage.notes.push(
            "players.kda: present. KDA was computed from the available kills, deaths and assists, which may come from the demo, a fixture or the API."
                .to_string(),
        );
    }

    coverage.fields.insert(
        "events.timeline".to_string(),
        if timeline.is_empty() {
            CoverageState::Absent
        } else {
            CoverageState::Present
        },
    );
    coverage.fields.insert(
        "positions".to_string(),
        if extract.positions.is_empty() {
            CoverageState::Absent
        } else {
            CoverageState::Present
        },
    );
    coverage.fields.insert(
        "heatmap".to_string(),
        if heatmap.available {
            CoverageState::Present
        } else {
            CoverageState::Absent
        },
    );

    if players.is_empty() {
        coverage.notes.push(
            "players: absent. The container extractor does not decode entity state or Citadel match metadata."
                .to_string(),
        );
    }
    if timeline.is_empty() {
        coverage
            .notes
            .push("events.timeline: absent. No event extractor ran on this demo.".to_string());
    }
    if extract.positions.is_empty() {
        coverage.notes.push(
            "positions: absent. No position samples were produced by the extractor.".to_string(),
        );
    }
    if !heatmap.available {
        coverage.notes.push(
            "heatmap: absent. A heatmap is only drawn from real position samples.".to_string(),
        );
    }

    let source_label = match (extract.source, api_merged) {
        (FieldSource::Demo, true) => "demo+api",
        (FieldSource::Fixture, true) => "fixture+api",
        (FieldSource::Demo, false) => "demo",
        (FieldSource::Fixture, false) => "fixture",
        (FieldSource::Api, _) => "api",
        (FieldSource::Derived, _) => "demo",
    }
    .to_string();

    MatchSummaryDoc {
        schema: SUMMARY_SCHEMA.to_string(),
        match_id: match_id.to_string(),
        generated_at: generated_at.to_string(),
        source: source_label,
        join,
        match_info: MergedMatchInfo {
            map,
            duration_seconds,
            duration_ticks: extract.match_facts.duration_ticks,
            result,
            start_time_unix,
            game_mode,
            build,
            server_name,
            rank,
            mmr,
            population_percentile,
        },
        players,
        timeline,
        heatmap,
        coverage,
        disagreements,
        notes: dedupe_notes(notes),
    }
}

fn set_optional_coverage(coverage: &mut Coverage, field: &str, present: bool) {
    coverage.fields.insert(
        field.to_string(),
        if present {
            CoverageState::Present
        } else {
            CoverageState::Absent
        },
    );
}

fn normalize_events(mut events: Vec<DemoEvent>) -> (Vec<DemoEvent>, bool) {
    events.sort_by(|left, right| {
        let left_time = left.time_seconds.unwrap_or(0.0);
        let right_time = right.time_seconds.unwrap_or(0.0);
        left_time
            .partial_cmp(&right_time)
            .unwrap_or(Ordering::Equal)
            .then(left.tick.unwrap_or(0).cmp(&right.tick.unwrap_or(0)))
            .then(left.sequence.cmp(&right.sequence))
    });

    let truncated = events.len() > MAX_EVENTS;
    events.truncate(MAX_EVENTS);

    for (index, event) in events.iter_mut().enumerate() {
        event.sequence = index as u64;
    }

    (events, truncated)
}

fn round_two(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn normalize_name(name: &str) -> String {
    name.trim().to_lowercase()
}

fn player_key(
    account_id: Option<u64>,
    name: Option<&str>,
    hero: Option<&str>,
    team: Option<i32>,
) -> String {
    if let Some(id) = account_id {
        return format!("acct:{id}");
    }
    if let Some(name) = name.map(str::trim).filter(|name| !name.is_empty()) {
        return format!("name:{}", name.to_lowercase());
    }
    format!(
        "hero:{}#{}",
        hero.unwrap_or("unknown").trim().to_lowercase(),
        team.map(|value| value.to_string())
            .unwrap_or_else(|| "?".to_string())
    )
}

fn count_player_events(extract: &DemoExtract, name: Option<&str>, account_id: Option<u64>) -> u64 {
    let normalized = name.map(normalize_name);
    extract
        .events
        .iter()
        .filter(|event| {
            matches_player(event.actor.as_deref(), normalized.as_deref(), account_id)
                || matches_player(event.target.as_deref(), normalized.as_deref(), account_id)
                || event
                    .assists
                    .iter()
                    .any(|assist| matches_player(Some(assist), normalized.as_deref(), account_id))
        })
        .count() as u64
}

fn count_player_positions(
    extract: &DemoExtract,
    name: Option<&str>,
    account_id: Option<u64>,
) -> u64 {
    let normalized = name.map(normalize_name);
    extract
        .positions
        .iter()
        .filter(|position| {
            if account_id.is_some() && position.account_id == account_id {
                return true;
            }
            matches_player(
                position.player.as_deref(),
                normalized.as_deref(),
                account_id,
            )
        })
        .count() as u64
}

fn matches_player(
    value: Option<&str>,
    normalized_name: Option<&str>,
    account_id: Option<u64>,
) -> bool {
    let Some(value) = value else {
        return false;
    };

    if let Some(id) = account_id
        && value.trim().parse::<u64>().ok() == Some(id)
    {
        return true;
    }

    match normalized_name {
        Some(name) => normalize_name(value) == name,
        None => false,
    }
}

fn derive_kda(players: &[ExtractedPlayer], events: &[DemoEvent]) -> KdaDerivation {
    let mut by_name: BTreeMap<String, String> = BTreeMap::new();
    let mut by_account: BTreeMap<u64, String> = BTreeMap::new();
    let mut counts: BTreeMap<String, KdaCounts> = BTreeMap::new();

    for player in players {
        let key = player_key(
            player.account_id,
            player.name.as_deref(),
            player.hero.as_deref(),
            player.team,
        );
        if let Some(name) = player.name.as_deref() {
            by_name.insert(normalize_name(name), key.clone());
        }
        if let Some(account_id) = player.account_id {
            by_account.insert(account_id, key.clone());
        }
        counts.entry(key).or_default();
    }

    let mut resolved_kills = 0_u64;
    let mut unresolved_kills = 0_u64;

    for event in events {
        if !event.kind.eq_ignore_ascii_case("kill") {
            continue;
        }

        let actor = resolve_player_key(event.actor.as_deref(), &by_name, &by_account);
        let target = resolve_player_key(event.target.as_deref(), &by_name, &by_account);

        match (actor, target) {
            (Some(actor), Some(target)) => {
                counts.entry(actor).or_default().kills += 1;
                counts.entry(target).or_default().deaths += 1;
                for assist in &event.assists {
                    if let Some(key) = resolve_player_key(Some(assist), &by_name, &by_account) {
                        counts.entry(key).or_default().assists += 1;
                    }
                }
                resolved_kills += 1;
            }
            _ => unresolved_kills += 1,
        }
    }

    KdaDerivation {
        counts,
        resolved_kills,
        unresolved_kills,
    }
}

fn resolve_player_key(
    value: Option<&str>,
    by_name: &BTreeMap<String, String>,
    by_account: &BTreeMap<u64, String>,
) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(account_id) = value.parse::<u64>()
        && let Some(key) = by_account.get(&account_id)
    {
        return Some(key.clone());
    }
    by_name.get(&normalize_name(value)).cloned()
}

// ---------------------------------------------------------------------------
// API facts
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct ApiPlayerFacts {
    account_id: Option<u64>,
    name: Option<String>,
    hero: Option<String>,
    hero_id: Option<u32>,
    team: Option<i32>,
    kills: Option<u64>,
    deaths: Option<u64>,
    assists: Option<u64>,
    net_worth: Option<u64>,
    badge: Option<u32>,
    mmr: Option<f64>,
    population_percentile: Option<f64>,
    result: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct ApiFacts {
    match_id: Option<u64>,
    map: Option<String>,
    duration_seconds: Option<u64>,
    start_time_unix: Option<i64>,
    game_mode: Option<String>,
    build: Option<String>,
    result: Option<String>,
    rank: Option<String>,
    mmr: Option<f64>,
    population_percentile: Option<f64>,
    players: Vec<ApiPlayerFacts>,
    fields_present: Vec<String>,
    fetched_at: Option<String>,
}

fn parse_api_json(text: &str) -> std::result::Result<ApiFacts, String> {
    let root: Value = serde_json::from_str(text)
        .map_err(|error| format!("api.json is not valid JSON: {error}"))?;
    let root = match root {
        Value::Array(mut rows) => {
            if rows.is_empty() {
                return Err("api.json is an empty array".to_string());
            }
            rows.remove(0)
        }
        other => other,
    };

    if !root.is_object() {
        return Err("api.json is not a JSON object or a one row array".to_string());
    }

    let containers = api_containers(&root);
    let mut facts = ApiFacts::default();

    if let Some(value) = lookup(&containers, &["match_id", "matchId", "matchid"])
        && let Some(id) = value_u64(value)
    {
        facts.match_id = Some(id);
        facts.fields_present.push("matchId".to_string());
    }
    if let Some(value) = lookup(&containers, &["map_name", "mapName"])
        .or_else(|| lookup(&containers, &["map"]).filter(|value| value.is_string()))
        && let Some(map) = value_string(value)
    {
        facts.map = Some(map);
        facts.fields_present.push("map".to_string());
    }
    if let Some(value) = lookup(
        &containers,
        &[
            "duration_s",
            "durationS",
            "duration_seconds",
            "durationSeconds",
            "match_duration_s",
            "matchDurationS",
        ],
    ) && let Some(duration) = value_u64(value)
    {
        facts.duration_seconds = Some(duration);
        facts.fields_present.push("durationSeconds".to_string());
    }
    if let Some(value) = lookup(&containers, &["start_time", "startTime", "start_time_s"])
        && let Some(start) = value_i64(value)
    {
        facts.start_time_unix = Some(start);
        facts.fields_present.push("startTimeUnix".to_string());
    }
    if let Some(value) = lookup(
        &containers,
        &["game_mode", "gameMode", "game_mode_name", "gameModeName"],
    ) && let Some(mode) = value_string(value)
    {
        facts.game_mode = Some(mode);
        facts.fields_present.push("gameMode".to_string());
    }
    if let Some(value) = lookup(
        &containers,
        &["build_number", "buildNumber", "build", "version"],
    ) && let Some(build) = value_string(value)
    {
        facts.build = Some(build);
        facts.fields_present.push("build".to_string());
    }
    if let Some(value) = lookup(
        &containers,
        &["result", "match_result", "matchResult", "outcome"],
    ) && let Some(result) = value_string(value)
    {
        facts.result = Some(normalize_result(&result));
        facts.fields_present.push("result".to_string());
    }

    if let Some(value) = lookup(
        &containers,
        &["average_badge", "averageBadge", "avg_badge", "avgBadge"],
    ) && let Some(badge) = value_u64(value)
    {
        facts.rank = Some(format!("average badge {badge}"));
        facts.fields_present.push("rank".to_string());
    } else if let Some(value) = lookup(&containers, &["tier_name", "tierName", "rank_name"])
        && let Some(name) = value_string(value)
    {
        facts.rank = Some(name);
        facts.fields_present.push("rank".to_string());
    }

    if let Some(value) = lookup(
        &containers,
        &["average_mmr", "averageMmr", "mmr", "match_mmr"],
    ) && let Some(mmr) = value_f64(value)
    {
        facts.mmr = Some(mmr);
        facts.fields_present.push("mmr".to_string());
    }
    if let Some(value) = lookup(
        &containers,
        &[
            "rank_percentile",
            "rankPercentile",
            "population_percentile",
            "percentile",
        ],
    ) && let Some(percentile) = value_f64(value)
    {
        facts.population_percentile = Some(percentile);
        facts
            .fields_present
            .push("populationPercentile".to_string());
    }

    for key in ["fetchedAt", "fetched_at", "cachedAt"] {
        if let Some(value) = root.get(key).and_then(value_string) {
            facts.fetched_at = Some(value);
            break;
        }
    }

    let mut rows: Option<&Vec<Value>> = None;
    for container in &containers {
        for key in ["players", "player_info", "playerInfo"] {
            if let Some(Value::Array(values)) = container.get(key)
                && !values.is_empty()
            {
                rows = Some(values);
                break;
            }
        }
        if rows.is_some() {
            break;
        }
    }

    if let Some(rows) = rows {
        for row in rows {
            if let Some(player) = parse_api_player(row) {
                facts.players.push(player);
            }
        }
        facts.fields_present.push("players".to_string());
    }

    facts.fields_present.sort();
    facts.fields_present.dedup();

    Ok(facts)
}

fn api_containers(root: &Value) -> Vec<&Value> {
    let mut containers = vec![root];

    for key in ["data", "match", "match_info", "matchInfo", "metadata"] {
        if let Some(value) = root.get(key) {
            containers.push(value);
        }
    }

    for key in ["data", "match"] {
        if let Some(value) = root.get(key) {
            for nested in ["match_info", "matchInfo", "metadata"] {
                if let Some(value) = value.get(nested) {
                    containers.push(value);
                }
            }
        }
    }

    containers
}

fn lookup<'a>(containers: &[&'a Value], keys: &[&str]) -> Option<&'a Value> {
    for container in containers {
        for key in keys {
            if let Some(value) = container.get(*key)
                && !value.is_null()
            {
                return Some(value);
            }
        }
    }
    None
}

fn parse_api_player(row: &Value) -> Option<ApiPlayerFacts> {
    if !row.is_object() {
        return None;
    }

    let mut player = ApiPlayerFacts::default();
    player.account_id = row
        .get("account_id")
        .or_else(|| row.get("accountId"))
        .and_then(value_u64);
    player.name = row
        .get("player_name")
        .or_else(|| row.get("playerName"))
        .or_else(|| row.get("personaname"))
        .or_else(|| row.get("persona_name"))
        .or_else(|| row.get("name"))
        .or_else(|| row.get("account_name"))
        .and_then(value_string);
    player.hero = row
        .get("hero_name")
        .or_else(|| row.get("heroName"))
        .and_then(value_string);
    player.hero_id = row
        .get("hero_id")
        .or_else(|| row.get("heroId"))
        .and_then(value_u64)
        .and_then(|value| u32::try_from(value).ok());
    player.team = row
        .get("player_team")
        .or_else(|| row.get("playerTeam"))
        .or_else(|| row.get("team"))
        .or_else(|| row.get("team_number"))
        .and_then(value_i64)
        .and_then(|value| i32::try_from(value).ok());
    player.kills = row
        .get("player_kills")
        .or_else(|| row.get("playerKills"))
        .or_else(|| row.get("kills"))
        .and_then(value_u64);
    player.deaths = row
        .get("player_deaths")
        .or_else(|| row.get("playerDeaths"))
        .or_else(|| row.get("deaths"))
        .and_then(value_u64);
    player.assists = row
        .get("player_assists")
        .or_else(|| row.get("playerAssists"))
        .or_else(|| row.get("assists"))
        .and_then(value_u64);
    player.net_worth = row
        .get("net_worth")
        .or_else(|| row.get("netWorth"))
        .or_else(|| row.get("player_net_worth"))
        .and_then(value_u64);
    player.badge = row
        .get("ranked_badge")
        .or_else(|| row.get("rankedBadge"))
        .or_else(|| row.get("ranked_display_badge"))
        .or_else(|| row.get("badge"))
        .and_then(value_u64)
        .and_then(|value| u32::try_from(value).ok());
    player.mmr = row
        .get("mmr")
        .or_else(|| row.get("ranked_mmr"))
        .or_else(|| row.get("player_mmr"))
        .and_then(value_f64);
    player.population_percentile = row
        .get("rank_percentile")
        .or_else(|| row.get("rankPercentile"))
        .or_else(|| row.get("percentile"))
        .and_then(value_f64);
    player.result = row
        .get("player_match_outcome")
        .or_else(|| row.get("playerMatchOutcome"))
        .and_then(value_i64)
        .and_then(api_outcome_label)
        .map(str::to_string)
        .or_else(|| {
            row.get("result")
                .or_else(|| row.get("outcome"))
                .and_then(value_string)
                .map(|value| normalize_result(&value))
        });

    if player.account_id.is_none()
        && player.name.is_none()
        && player.hero.is_none()
        && player.hero_id.is_none()
    {
        return None;
    }

    Some(player)
}

fn api_outcome_label(value: i64) -> Option<&'static str> {
    match value {
        0 => Some("invalid"),
        1 => Some("win"),
        2 => Some("loss"),
        3 => Some("penalized"),
        4 => Some("penalized party"),
        5 => Some("not scored"),
        _ => None,
    }
}

fn normalize_result(value: &str) -> String {
    let normalized = value.trim().to_lowercase();
    match normalized.as_str() {
        "w" | "won" | "victory" => "win".to_string(),
        "l" | "lost" | "defeat" => "loss".to_string(),
        other => other.to_string(),
    }
}

fn value_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|number| u64::try_from(number).ok()))
        .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
}

fn value_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|number| i64::try_from(number).ok()))
        .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
}

fn value_f64(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
}

fn value_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.trim().is_empty() => Some(text.trim().to_string()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Fixture input
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FixtureFile {
    #[serde(default)]
    schema: Option<String>,
    #[serde(default)]
    match_id: Option<String>,
    #[serde(default)]
    map: Option<String>,
    #[serde(default)]
    duration_seconds: Option<u64>,
    #[serde(default)]
    duration_ticks: Option<u64>,
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    start_time_unix: Option<i64>,
    #[serde(default)]
    game_mode: Option<String>,
    #[serde(default)]
    build: Option<String>,
    #[serde(default)]
    players: Vec<FixturePlayer>,
    #[serde(default)]
    events: Vec<FixtureEvent>,
    #[serde(default)]
    positions: Vec<FixturePosition>,
    #[serde(default)]
    notes: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FixturePlayer {
    #[serde(default)]
    account_id: Option<u64>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    hero: Option<String>,
    #[serde(default)]
    hero_id: Option<u32>,
    #[serde(default)]
    team: Option<i32>,
    #[serde(default)]
    kills: Option<u64>,
    #[serde(default)]
    deaths: Option<u64>,
    #[serde(default)]
    assists: Option<u64>,
    #[serde(default)]
    net_worth: Option<u64>,
    #[serde(default)]
    badge: Option<u32>,
    #[serde(default)]
    mmr: Option<f64>,
    #[serde(default)]
    population_percentile: Option<f64>,
    #[serde(default)]
    result: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FixtureEvent {
    #[serde(default)]
    tick: u64,
    #[serde(default)]
    time_seconds: Option<f64>,
    kind: String,
    #[serde(default)]
    actor: Option<String>,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    assists: Vec<String>,
    #[serde(default)]
    team: Option<i32>,
    #[serde(default)]
    x: Option<f64>,
    #[serde(default)]
    y: Option<f64>,
    #[serde(default)]
    detail: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FixturePosition {
    #[serde(default)]
    time_seconds: Option<f64>,
    #[serde(default)]
    account_id: Option<u64>,
    #[serde(default)]
    player: Option<String>,
    #[serde(default)]
    team: Option<i32>,
    #[serde(default = "default_position_kind")]
    kind: String,
    x: f64,
    y: f64,
    #[serde(default)]
    z: Option<f64>,
}

fn default_position_kind() -> String {
    "position".to_string()
}

fn fixture_extract(fixture: &FixtureFile) -> Result<DemoExtract> {
    let mut players = Vec::new();
    for (index, player) in fixture.players.iter().enumerate() {
        if player.account_id.is_none() && player.name.is_none() && player.hero.is_none() {
            return Err(Error::msg(format!(
                "Fixture player {index} needs an accountId, a name or a hero."
            )));
        }
        players.push(ExtractedPlayer {
            account_id: player.account_id,
            name: player.name.clone(),
            hero: player.hero.clone(),
            hero_id: player.hero_id,
            team: player.team,
            kills: player.kills,
            deaths: player.deaths,
            assists: player.assists,
            net_worth: player.net_worth,
            badge: player.badge,
            mmr: player.mmr,
            population_percentile: player.population_percentile,
            result: player.result.as_deref().map(normalize_result),
        });
    }

    let mut events = Vec::new();
    for (index, event) in fixture.events.iter().enumerate() {
        if event.kind.trim().is_empty() {
            return Err(Error::msg(format!("Fixture event {index} has no kind.")));
        }
        events.push(DemoEvent {
            sequence: index as u64,
            tick: if event.tick > 0 {
                Some(event.tick)
            } else {
                None
            },
            time_seconds: event.time_seconds.filter(|value| value.is_finite()),
            kind: event.kind.trim().to_lowercase(),
            actor: event.actor.clone(),
            target: event.target.clone(),
            assists: event.assists.clone(),
            team: event.team,
            position_x: event.x.filter(|value| value.is_finite()),
            position_y: event.y.filter(|value| value.is_finite()),
            detail: event.detail.clone(),
        });
    }

    let mut positions = Vec::new();
    for (index, position) in fixture.positions.iter().enumerate() {
        if !position.x.is_finite() || !position.y.is_finite() {
            return Err(Error::msg(format!(
                "Fixture position {index} needs finite x and y values."
            )));
        }
        positions.push(DemoPosition {
            time_seconds: position.time_seconds.filter(|value| value.is_finite()),
            account_id: position.account_id,
            player: position.player.clone(),
            team: position.team,
            kind: if position.kind.trim().is_empty() {
                "position".to_string()
            } else {
                position.kind.trim().to_lowercase()
            },
            x: position.x,
            y: position.y,
            z: position.z,
        });
    }

    if fixture.map.is_none() && players.is_empty() && events.is_empty() && positions.is_empty() {
        return Err(Error::msg(
            "Fixture has no map, players, events or positions. Add at least one so the record is useful.",
        ));
    }

    let derived_kda = derive_kda(&players, &events);
    let mut missing = Vec::new();
    if fixture.map.is_none() {
        missing.push("map".to_string());
    }
    if fixture.duration_seconds.is_none() {
        missing.push("durationSeconds".to_string());
    }
    if players.is_empty() {
        missing.push("players".to_string());
    }
    if events.is_empty() {
        missing.push("events".to_string());
    }
    if positions.is_empty() {
        missing.push("positions".to_string());
        missing.push("heatmap".to_string());
    }
    if fixture.result.is_none() {
        missing.push("result".to_string());
    }
    if derived_kda.resolved_kills == 0 {
        if players.iter().all(|player| player.kills.is_none()) {
            missing.push("kills".to_string());
        }
        if players.iter().all(|player| player.deaths.is_none()) {
            missing.push("deaths".to_string());
        }
        if players.iter().all(|player| player.assists.is_none()) {
            missing.push("assists".to_string());
        }
    }
    missing.sort();
    missing.dedup();

    let mut notes = vec![
        "Fixture extractor. Every field is test or UI development data and is labelled source fixture."
            .to_string(),
    ];
    if let Some(schema) = &fixture.schema {
        notes.push(format!("Fixture schema: {schema}."));
    }
    notes.extend(fixture.notes.iter().cloned());

    Ok(DemoExtract {
        extractor: "fixture".to_string(),
        source: FieldSource::Fixture,
        match_facts: ExtractedMatchFacts {
            map: fixture.map.clone(),
            duration_seconds: fixture.duration_seconds,
            duration_ticks: fixture.duration_ticks,
            result: fixture.result.as_deref().map(normalize_result),
            start_time_unix: fixture.start_time_unix,
            game_mode: fixture.game_mode.clone(),
            build: fixture.build.clone(),
            server_name: None,
            rank: None,
            mmr: None,
            population_percentile: None,
        },
        players,
        events,
        positions,
        derived_kda,
        missing_fields: missing,
        notes,
    })
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/// `matches` directory under a workspace.
pub fn matches_dir(workspace: &Path) -> PathBuf {
    workspace.join(MATCHES_DIR_NAME)
}

/// Validate a match id before it becomes a folder name.
///
/// Allowed: ASCII letters, digits, `.`, `-` and `_`, 1 to 96 characters. The
/// value may not be `.` or `..` and may not contain a path separator.
pub fn validate_match_id(match_id: &str) -> Result<()> {
    let trimmed = match_id.trim();
    if trimmed.is_empty() {
        return Err(Error::msg("Match id must not be empty."));
    }
    if trimmed != match_id {
        return Err(Error::msg(
            "Match id must not have leading or trailing whitespace.",
        ));
    }
    if match_id.len() > 96 {
        return Err(Error::msg("Match id must be 96 characters or fewer."));
    }
    if match_id == "." || match_id == ".." {
        return Err(Error::msg("Match id must not be . or .."));
    }
    if !match_id
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_'))
    {
        return Err(Error::msg(format!(
            "Match id {match_id:?} contains an unsupported character. Use ASCII letters, digits, dot, hyphen or underscore."
        )));
    }
    Ok(())
}

/// Path of one match folder, with the id validated.
pub fn match_dir(workspace: &Path, match_id: &str) -> Result<PathBuf> {
    validate_match_id(match_id)?;
    Ok(matches_dir(workspace).join(match_id))
}

fn sanitize_id(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut last_was_dash = false;

    for character in value.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
            output.push(character);
            last_was_dash = false;
        } else if !last_was_dash {
            output.push('-');
            last_was_dash = true;
        }
    }

    output.trim_matches(['-', '.']).to_string()
}

fn choose_match_id(
    requested: Option<&str>,
    demo_path: &Path,
    crc32: u32,
) -> Result<(String, &'static str)> {
    if let Some(requested) = requested {
        validate_match_id(requested)?;
        return Ok((requested.to_string(), "requested"));
    }

    let stem = demo_path
        .file_stem()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    let sanitized = sanitize_id(&stem);
    let generic = matches!(
        sanitized.to_ascii_lowercase().as_str(),
        "" | "match" | "demo" | "replay" | "fixture" | "download"
    );

    if !generic && sanitized.len() >= 3 {
        return Ok((sanitized, "fileStem"));
    }

    Ok((format!("demo-{crc32:08x}"), "crc32"))
}

/// Import a Source 2 demo into `<workspace>/matches/<match-id>/`.
///
/// The magic is checked before any folder is created. Re-importing the same
/// match updates provenance and the summary; `review.md` and other files in the
/// folder are never removed.
pub fn import_demo(
    workspace: &Path,
    demo_path: &Path,
    requested_id: Option<&str>,
) -> Result<ImportResult> {
    let demo = parse_demo(demo_path)?;
    let extractor = ContainerExtractor;
    let extract = extractor.extract(&demo)?;
    let (match_id, id_source) = choose_match_id(requested_id, demo_path, demo.crc32)?;

    let stamp = SourceStamp {
        path: demo_path.to_path_buf(),
        byte_len: demo.byte_len,
        crc32: demo.crc32,
    };

    import_parsed(
        workspace,
        &match_id,
        id_source,
        "demo",
        &stamp,
        None,
        Some(&demo),
        &extract,
        Vec::new(),
    )
}

/// Import a documented JSON fixture. Tests and UI development only.
///
/// A fixture record is written with `source: "fixture"` and never claims to be
/// a real capture. The CLI `dah demo import` does not accept fixtures; it
/// requires the `PBDEMS2` magic.
pub fn import_fixture(
    workspace: &Path,
    fixture_path: &Path,
    requested_id: Option<&str>,
) -> Result<ImportResult> {
    let bytes = fs::read(fixture_path).map_err(|error| {
        Error::msg(format!(
            "Could not read fixture {}: {error}",
            fixture_path.display()
        ))
    })?;
    let fixture: FixtureFile = serde_json::from_slice(&bytes).map_err(|error| {
        Error::msg(format!(
            "Fixture {} is not valid JSON: {error}. See docs/research/demo-import.md for the shape.",
            fixture_path.display()
        ))
    })?;

    let extract = fixture_extract(&fixture)?;
    let (match_id, id_source) = match requested_id {
        Some(id) => (id.to_string(), "requested"),
        None => match fixture
            .match_id
            .as_deref()
            .filter(|id| !id.trim().is_empty())
        {
            Some(id) => (id.to_string(), "fixture"),
            None => {
                let (id, _) = choose_match_id(None, fixture_path, crc32fast::hash(&bytes))?;
                (id, "fileStem")
            }
        },
    };
    validate_match_id(&match_id)?;

    let stamp = SourceStamp {
        path: fixture_path.to_path_buf(),
        byte_len: bytes.len() as u64,
        crc32: crc32fast::hash(&bytes),
    };

    import_parsed(
        workspace,
        &match_id,
        id_source,
        "fixture",
        &stamp,
        Some(&bytes),
        None,
        &extract,
        Vec::new(),
    )
}

struct SourceStamp {
    path: PathBuf,
    byte_len: u64,
    crc32: u32,
}

fn import_parsed(
    workspace: &Path,
    match_id: &str,
    id_source: &str,
    source_kind: &str,
    stamp: &SourceStamp,
    fixture_bytes: Option<&[u8]>,
    demo: Option<&DemoFile>,
    extract: &DemoExtract,
    extra_notes: Vec<String>,
) -> Result<ImportResult> {
    validate_match_id(match_id)?;
    let directory = match_dir(workspace, match_id)?;
    let existed = directory.exists();
    fs::create_dir_all(&directory)?;

    match source_kind {
        "fixture" => {
            let bytes = fixture_bytes.ok_or_else(|| {
                Error::msg("Internal error: a fixture import needs the fixture bytes.")
            })?;
            write_atomic(&directory.join(MATCH_FIXTURE_FILE_NAME), bytes)?;
        }
        _ => {
            let target = directory.join(MATCH_DEMO_FILE_NAME);
            if stamp.path != target {
                fs::copy(&stamp.path, &target).map_err(|error| {
                    Error::msg(format!(
                        "Could not copy demo {} into the match store at {}: {error}",
                        stamp.path.display(),
                        target.display()
                    ))
                })?;
            }
        }
    }

    let api_path = directory.join(MATCH_API_FILE_NAME);
    let mut notes = extra_notes;
    let api_text = match fs::read_to_string(&api_path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => {
            notes.push(format!("api.json could not be read: {error}"));
            None
        }
    };
    let mut api_facts: Option<ApiFacts> = None;
    let mut api_parse_error: Option<String> = None;

    if let Some(text) = &api_text {
        match parse_api_json(text) {
            Ok(facts) => {
                if facts.players.is_empty() {
                    notes.push(
                        "api.json has no players array. Player rows will be absent where the demo has none."
                            .to_string(),
                    );
                }
                api_facts = Some(facts);
            }
            Err(error) => {
                notes.push(format!("api.json was ignored: {error}"));
                api_parse_error = Some(error);
            }
        }
    }

    let mut api_merged = false;
    let mut api_id_mismatch: Option<(String, u64)> = None;
    let mut join = MatchJoin {
        key: match_id.to_string(),
        inferred: false,
        note: None,
    };

    if api_facts.is_none() {
        join.note =
            Some("No api.json in the match folder; the summary uses the demo only.".to_string());
    }

    if let Some(facts) = &api_facts {
        match facts.match_id {
            Some(api_id) if api_id.to_string() != match_id => {
                api_id_mismatch = Some((match_id.to_string(), api_id));
                join.inferred = true;
                join.note = Some(format!(
                    "api.json belongs to match {api_id}, not {match_id}; its fields were not merged."
                ));
                notes.push(format!(
                    "api.json match id {api_id} does not match the store id {match_id}. API fields were not merged and the conflict is recorded in summary.json."
                ));
            }
            Some(_) => {
                api_merged = true;
                join.inferred = false;
                join.note = Some("Joined on the match id.".to_string());
            }
            None => {
                api_merged = true;
                join.inferred = true;
                join.note = Some(
                    "api.json has no match id; the join used the match folder id.".to_string(),
                );
            }
        }
    }

    let generated_at = now_iso();
    let prior = read_json_optional::<Provenance>(&directory.join(MATCH_PROVENANCE_FILE_NAME));
    let imported_at = prior
        .as_ref()
        .map(|value| value.imported_at.clone())
        .unwrap_or_else(|| generated_at.clone());
    let revision = prior
        .as_ref()
        .map(|value| value.revision.saturating_add(1))
        .unwrap_or(1);
    let review_preserved = directory.join(MATCH_REVIEW_FILE_NAME).is_file();

    let summary = build_summary(
        match_id,
        extract,
        api_facts.as_ref().filter(|_| api_merged),
        api_merged,
        api_id_mismatch,
        join,
        &generated_at,
        &notes,
    );

    write_events(&directory.join(MATCH_EVENTS_FILE_NAME), &summary.timeline)?;

    let crc32 = format!("{:08x}", stamp.crc32);
    let demo_provenance = demo.map(|demo| DemoProvenance {
        magic: "PBDEMS2".to_string(),
        patch_version: demo.header.as_ref().and_then(|value| value.patch_version),
        demo_version_name: demo
            .header
            .as_ref()
            .and_then(|value| value.demo_version_name.clone()),
        fullpackets_version: demo
            .header
            .as_ref()
            .and_then(|value| value.fullpackets_version),
        commands_total: demo.framing.commands_total,
        compressed_commands: demo.framing.compressed_commands,
        compressed_decoded: demo.framing.compressed_decoded,
        unknown_commands: demo.framing.unknown_commands,
        file_info_offset: demo.framing.file_info_offset,
        stop_seen: demo.framing.stop_seen,
        playback_ticks: demo
            .file_info
            .as_ref()
            .and_then(|value| value.playback_ticks),
        playback_time_seconds: demo
            .file_info
            .as_ref()
            .and_then(|value| value.playback_time_seconds),
        string_tables: demo.string_tables.len(),
        user_messages: demo.user_messages.len(),
        custom_data_entries: demo.custom_data.len(),
        notes: demo.notes.clone(),
    });

    let api_provenance = api_text.as_ref().map(|_| ApiProvenance {
        path: MATCH_API_FILE_NAME.to_string(),
        byte_len: api_text.as_ref().map(|text| text.len() as u64).unwrap_or(0),
        fetched_at: api_facts
            .as_ref()
            .and_then(|facts| facts.fetched_at.clone()),
        match_id: api_facts.as_ref().and_then(|facts| facts.match_id),
        merged: api_merged,
        fields_present: api_facts
            .as_ref()
            .map(|facts| facts.fields_present.clone())
            .unwrap_or_default(),
        parse_error: api_parse_error,
    });

    let mut missing_fields = extract.missing_fields.clone();
    if !summary.heatmap.available {
        missing_fields.push("heatmap".to_string());
    }
    missing_fields.sort();
    missing_fields.dedup();

    let provenance = Provenance {
        schema: PROVENANCE_SCHEMA.to_string(),
        importer: IMPORTER_NAME.to_string(),
        importer_version: IMPORTER_VERSION.to_string(),
        extractor: extract.extractor.clone(),
        source: ProvenanceSource {
            kind: source_kind.to_string(),
            path: stamp.path.to_string_lossy().into_owned(),
            byte_len: stamp.byte_len,
            crc32: crc32.clone(),
            match_id_source: id_source.to_string(),
        },
        imported_at: imported_at.clone(),
        updated_at: generated_at.clone(),
        parsed_at: generated_at.clone(),
        revision,
        byte_len: stamp.byte_len,
        crc32,
        demo: demo_provenance,
        api: api_provenance,
        missing_fields: missing_fields.clone(),
        unsupported: if source_kind == "fixture" {
            vec![
                "fixture data is not a real capture".to_string(),
                "only the fields declared in fixture.json are present".to_string(),
            ]
        } else {
            vec![
                "entity state and player positions (needs a serializer aware backend)".to_string(),
                "Citadel game events and post-match match metadata (user message payloads are inventoried, not decoded)".to_string(),
                "net message contents inside CDemoPacket (protocol buffers are inventoried only)".to_string(),
            ]
        },
        prior_review_preserved: review_preserved,
        notes: dedupe_notes(summary.notes.clone()),
    };

    let meta = MatchMeta {
        schema: META_SCHEMA.to_string(),
        match_id: match_id.to_string(),
        source: summary.source.clone(),
        imported_at,
        updated_at: generated_at,
        revision,
        map: summary
            .match_info
            .map
            .as_ref()
            .map(|value| value.value.clone()),
        duration_seconds: summary
            .match_info
            .duration_seconds
            .as_ref()
            .map(|value| value.value),
        duration_ticks: summary.match_info.duration_ticks,
        result: summary
            .match_info
            .result
            .as_ref()
            .map(|value| value.value.clone()),
        player_count: summary.players.len(),
        event_count: summary.timeline.len(),
        has_demo: source_kind == "demo",
        has_fixture: source_kind == "fixture",
        has_api: api_merged,
        position_samples: extract.positions.len(),
        heatmap_cells: summary.heatmap.cells.len(),
        missing_fields,
    };

    write_json(&directory.join(MATCH_META_FILE_NAME), &meta)?;
    write_json(&directory.join(MATCH_SUMMARY_FILE_NAME), &summary)?;
    write_json(&directory.join(MATCH_PROVENANCE_FILE_NAME), &provenance)?;

    Ok(ImportResult {
        match_id: match_id.to_string(),
        directory,
        existed,
        review_preserved,
        meta,
        summary,
        provenance,
    })
}

fn write_events(path: &Path, events: &[DemoEvent]) -> Result<()> {
    let mut text = String::new();
    for event in events {
        text.push_str(&serde_json::to_string(event)?);
        text.push('\n');
    }
    write_atomic(path, text.as_bytes())
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    write_atomic(path, &bytes)
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path)?;
    serde_json::from_slice(&bytes).map_err(Error::from)
}

fn read_json_optional<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Read the merged summary for one match.
pub fn read_summary(workspace: &Path, match_id: &str) -> Result<MatchSummaryDoc> {
    let path = match_dir(workspace, match_id)?.join(MATCH_SUMMARY_FILE_NAME);
    read_json(&path).map_err(|error| {
        Error::msg(format!(
            "Could not read {}: {error}. Run `Sigil cli demo import` for this match first.",
            path.display()
        ))
    })
}

/// Read provenance for one match.
pub fn read_provenance(workspace: &Path, match_id: &str) -> Result<Provenance> {
    let path = match_dir(workspace, match_id)?.join(MATCH_PROVENANCE_FILE_NAME);
    read_json(&path).map_err(|error| {
        Error::msg(format!(
            "Could not read {}: {error}. Run `Sigil cli demo import` for this match first.",
            path.display()
        ))
    })
}

/// Read compact metadata for one match.
pub fn read_meta(workspace: &Path, match_id: &str) -> Result<MatchMeta> {
    let path = match_dir(workspace, match_id)?.join(MATCH_META_FILE_NAME);
    read_json(&path).map_err(|error| {
        Error::msg(format!(
            "Could not read {}: {error}. Run `Sigil cli demo import` for this match first.",
            path.display()
        ))
    })
}

/// List imported matches, newest update first.
pub fn list_matches(workspace: &Path) -> Result<MatchList> {
    let directory = matches_dir(workspace);
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(MatchList {
                workspace: directory,
                matches: Vec::new(),
                skipped: 0,
            });
        }
        Err(error) => return Err(Error::Io(error)),
    };

    let mut matches = Vec::new();
    let mut skipped = 0_usize;

    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let path = entry.path();
        let Some(match_id) = path
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
        else {
            skipped += 1;
            continue;
        };

        if let Some(meta) = read_json_optional::<MatchMeta>(&path.join(MATCH_META_FILE_NAME)) {
            matches.push(MatchListItem {
                match_id,
                directory: path,
                source: meta.source,
                map: meta.map,
                duration_seconds: meta.duration_seconds,
                result: meta.result,
                player_count: meta.player_count,
                event_count: meta.event_count,
                heatmap_cells: meta.heatmap_cells,
                has_api: meta.has_api,
                revision: meta.revision,
                updated_at: meta.updated_at,
                missing_fields: meta.missing_fields,
            });
            continue;
        }

        if let Some(summary) =
            read_json_optional::<MatchSummaryDoc>(&path.join(MATCH_SUMMARY_FILE_NAME))
        {
            let has_api = summary.source.ends_with("+api");
            matches.push(MatchListItem {
                match_id,
                directory: path,
                source: summary.source,
                map: summary
                    .match_info
                    .map
                    .as_ref()
                    .map(|value| value.value.clone()),
                duration_seconds: summary
                    .match_info
                    .duration_seconds
                    .as_ref()
                    .map(|value| value.value),
                result: summary
                    .match_info
                    .result
                    .as_ref()
                    .map(|value| value.value.clone()),
                player_count: summary.players.len(),
                event_count: summary.timeline.len(),
                heatmap_cells: summary.heatmap.cells.len(),
                has_api,
                revision: 0,
                updated_at: summary.generated_at,
                missing_fields: summary
                    .coverage
                    .fields
                    .iter()
                    .filter(|(_, state)| **state != CoverageState::Present)
                    .map(|(field, _)| field.clone())
                    .collect(),
            });
            continue;
        }

        skipped += 1;
    }

    matches.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then(left.match_id.cmp(&right.match_id))
    });

    Ok(MatchList {
        workspace: directory,
        matches,
        skipped,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn pack_bits(values: &[(u32, usize)]) -> Vec<u8> {
        let total: usize = values.iter().map(|(_, count)| count).sum();
        let mut bytes = vec![0_u8; total.div_ceil(8)];
        let mut bit = 0_usize;
        for (value, count) in values {
            for index in 0..*count {
                if (value >> index) & 1 == 1 {
                    bytes[(bit + index) / 8] |= 1 << ((bit + index) % 8);
                }
            }
            bit += count;
        }
        bytes
    }

    fn encode_ubit_var(value: u32) -> Vec<(u32, usize)> {
        if value < 0x10 {
            vec![(value, 6)]
        } else if value < 0x100 {
            vec![((value & 0x0f) | 0x10, 6), (value >> 4, 4)]
        } else if value < 0x1000 {
            vec![((value & 0x0f) | 0x20, 6), (value >> 4, 8)]
        } else {
            vec![((value & 0x0f) | 0x30, 6), (value >> 4, 28)]
        }
    }

    fn write_varint(output: &mut Vec<u8>, mut value: u64) {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            output.push(byte);
            if value == 0 {
                break;
            }
        }
    }

    fn proto_varint(output: &mut Vec<u8>, field: u32, value: u64) {
        write_varint(output, u64::from(field << 3));
        write_varint(output, value);
    }

    fn proto_bytes(output: &mut Vec<u8>, field: u32, bytes: &[u8]) {
        write_varint(output, u64::from((field << 3) | 2));
        write_varint(output, bytes.len() as u64);
        output.extend_from_slice(bytes);
    }

    fn proto_string(output: &mut Vec<u8>, field: u32, value: &str) {
        proto_bytes(output, field, value.as_bytes());
    }

    fn proto_f32(output: &mut Vec<u8>, field: u32, value: f32) {
        write_varint(output, u64::from((field << 3) | 5));
        output.extend_from_slice(&value.to_bits().to_le_bytes());
    }

    fn write_command(output: &mut Vec<u8>, command: u32, tick: u64, payload: &[u8]) {
        write_varint(output, u64::from(command));
        write_varint(output, tick);
        write_varint(output, payload.len() as u64);
        output.extend_from_slice(payload);
    }

    /// Wrap bytes in a single Snappy literal. This is enough to prove the
    /// compression flag path without a compressor dependency.
    fn snappy_literal(bytes: &[u8]) -> Vec<u8> {
        let mut output = Vec::new();
        write_varint(&mut output, bytes.len() as u64);
        assert!(
            bytes.len() <= 60,
            "the test helper only writes one byte literal lengths"
        );
        output.push(((bytes.len() - 1) << 2) as u8);
        output.extend_from_slice(bytes);
        output
    }

    fn demo_prefix() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&DEMO_MAGIC);
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes
    }

    fn file_header_payload(map: &str) -> Vec<u8> {
        let mut payload = Vec::new();
        proto_string(&mut payload, 1, "PBDEMS2");
        proto_varint(&mut payload, 2, 1234);
        proto_string(&mut payload, 3, "test-server");
        proto_string(&mut payload, 4, "test-client");
        proto_string(&mut payload, 5, map);
        proto_string(&mut payload, 6, "citadel");
        proto_varint(&mut payload, 13, 9999);
        payload
    }

    fn build_demo(map: &str, playback_seconds: f32, ticks: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&DEMO_MAGIC);
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());

        write_command(&mut bytes, 1, 0, &file_header_payload(map));

        let mut info = Vec::new();
        proto_f32(&mut info, 1, playback_seconds);
        proto_varint(&mut info, 2, u64::from(ticks));
        proto_varint(&mut info, 3, u64::from(ticks));
        write_command(&mut bytes, 2, 0, &info);

        write_command(&mut bytes, 3, 0, &[]);
        write_command(&mut bytes, 0, 0, &[]);
        bytes
    }

    #[test]
    fn varint_reads_hand_built_bytes() {
        let cases: &[(&[u8], u64)] = &[
            (&[0x00], 0),
            (&[0x7f], 127),
            (&[0x80, 0x01], 128),
            (&[0xac, 0x02], 300),
            (&[0xff, 0xff, 0xff, 0xff, 0x0f], u64::from(u32::MAX)),
        ];

        for (bytes, expected) in cases {
            let mut reader = DemoReader::new(Cursor::new(bytes.to_vec()));
            assert_eq!(
                reader.read_varint().unwrap(),
                Some(*expected),
                "bytes {bytes:?}"
            );
        }
    }

    #[test]
    fn varint_rejects_truncated_input() {
        let mut reader = DemoReader::new(Cursor::new(vec![0x80]));
        assert!(reader.read_varint().is_err());
    }

    #[test]
    fn snappy_decodes_literal_and_copy() {
        // Uncompressed length 10, literal "hello", copy length 5 offset 5.
        let mut compressed = vec![10];
        compressed.push((5 - 1) << 2);
        compressed.extend_from_slice(b"hello");
        compressed.push(((5 - 4) << 2) | 1);
        compressed.push(5);

        assert_eq!(snappy_decompress(&compressed).unwrap(), b"hellohello");
    }

    #[test]
    fn snappy_rejects_bad_offset() {
        let compressed = vec![4, 0, 5];
        assert!(snappy_decompress(&compressed).is_err());
    }

    #[test]
    fn bit_cursor_reads_ubit_var_buckets() {
        for value in [0_u32, 1, 15, 16, 255, 256, 4095, 4096, 70_000] {
            let bits = encode_ubit_var(value);
            let bytes = pack_bits(&bits);
            let mut cursor = BitCursor::new(&bytes);
            assert_eq!(cursor.read_ubit_var(), Some(value), "value {value}");
        }
    }

    #[test]
    fn packet_inventory_counts_post_match_details() {
        let mut bits = encode_ubit_var(316);
        bits.push((3, 8));
        bits.push((1, 8));
        bits.push((2, 8));
        bits.push((3, 8));
        let packet = pack_bits(&bits);

        let mut counts = BTreeMap::new();
        let mut inventory = DemoPacketInventory::default();
        let mut notes = Vec::new();
        scan_packet(&packet, &mut counts, &mut inventory, &mut notes);

        assert_eq!(counts.get(&316), Some(&(1, 3)));
        assert_eq!(inventory.messages, 1);
        assert_eq!(user_message_name(316), Some("citadel.post_match_details"));
    }

    #[test]
    fn file_header_decodes_known_fields() {
        let header = decode_file_header(&file_header_payload("dl_midtown")).unwrap();
        assert_eq!(header.demo_file_stamp, "PBDEMS2");
        assert_eq!(header.patch_version, Some(1234));
        assert_eq!(header.server_name.as_deref(), Some("test-server"));
        assert_eq!(header.client_name.as_deref(), Some("test-client"));
        assert_eq!(header.map_name.as_deref(), Some("dl_midtown"));
        assert_eq!(header.game_directory.as_deref(), Some("citadel"));
        assert_eq!(header.build_num, Some(9999));
    }

    #[test]
    fn file_info_decodes_playback() {
        let mut payload = Vec::new();
        proto_f32(&mut payload, 1, 1234.5);
        proto_varint(&mut payload, 2, 37_035);
        proto_varint(&mut payload, 3, 37_035);

        let info = decode_file_info(&payload).unwrap();
        assert_eq!(info.playback_time_seconds, Some(1234.5));
        assert_eq!(info.playback_ticks, Some(37_035));
        assert_eq!(info.playback_frames, Some(37_035));
    }

    #[test]
    fn string_tables_decode_inventory() {
        let mut item = Vec::new();
        proto_string(&mut item, 1, "player-1");
        proto_bytes(&mut item, 2, &[1, 2, 3, 4]);

        let mut table = Vec::new();
        proto_string(&mut table, 1, "userinfo");
        proto_bytes(&mut table, 2, &item);

        let mut payload = Vec::new();
        proto_bytes(&mut payload, 1, &table);

        let tables = decode_string_tables(&payload).unwrap();
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].name, "userinfo");
        assert_eq!(tables[0].items, 1);
        assert_eq!(tables[0].data_bytes, 4);
        assert_eq!(tables[0].sample_keys, vec!["player-1"]);
    }

    #[test]
    fn parse_stream_reads_synthetic_demo() {
        let bytes = build_demo("dl_midtown", 1234.0, 37_035);
        let mut reader = DemoReader::new(Cursor::new(bytes.clone()));
        let demo = parse_stream(
            &mut reader,
            PathBuf::from("synthetic.dem"),
            bytes.len() as u64,
        )
        .unwrap();

        assert_eq!(demo.header.unwrap().map_name.as_deref(), Some("dl_midtown"));
        assert_eq!(demo.file_info.unwrap().playback_time_seconds, Some(1234.0));
        assert!(demo.framing.stop_seen);
        assert_eq!(demo.framing.commands_total, 4);
        assert_eq!(demo.crc32, crc32fast::hash(&bytes));
    }

    #[test]
    fn parse_rejects_non_demo_bytes() {
        let bytes = b"not a demo at all".to_vec();
        let mut reader = DemoReader::new(Cursor::new(bytes.clone()));
        let error = parse_stream(&mut reader, PathBuf::from("bad.dem"), bytes.len() as u64)
            .unwrap_err()
            .to_string();
        assert!(error.contains("PBDEMS2"));
        assert!(error.contains("Expected the magic"));
    }

    #[test]
    fn compressed_file_info_is_decompressed() {
        let mut info = Vec::new();
        proto_f32(&mut info, 1, 1234.5);
        proto_varint(&mut info, 2, 37_035);

        let mut bytes = demo_prefix();
        write_command(&mut bytes, 2 | 64, 0, &snappy_literal(&info));
        write_command(&mut bytes, 0, 0, &[]);

        let mut reader = DemoReader::new(Cursor::new(bytes.clone()));
        let demo = parse_stream(
            &mut reader,
            PathBuf::from("compressed.dem"),
            bytes.len() as u64,
        )
        .unwrap();

        assert_eq!(demo.file_info.unwrap().playback_time_seconds, Some(1234.5));
        assert_eq!(demo.framing.compressed_commands, 1);
        assert_eq!(demo.framing.compressed_decoded, 1);
    }

    #[test]
    fn unknown_commands_are_counted_not_fatal() {
        let mut bytes = demo_prefix();
        write_command(&mut bytes, 19, 0, &[1, 2, 3]);
        write_command(&mut bytes, 0, 0, &[]);

        let mut reader = DemoReader::new(Cursor::new(bytes.clone()));
        let demo = parse_stream(
            &mut reader,
            PathBuf::from("unknown.dem"),
            bytes.len() as u64,
        )
        .unwrap();

        assert_eq!(demo.framing.unknown_commands, 1);
        assert_eq!(
            demo.framing.command_counts.get("unknown:19").copied(),
            Some(1)
        );
        assert!(demo.framing.stop_seen);
    }

    #[test]
    fn match_id_rejects_traversal() {
        assert!(validate_match_id("../escape").is_err());
        assert!(validate_match_id("a/b").is_err());
        assert!(validate_match_id("").is_err());
        assert!(validate_match_id(".").is_err());
        assert!(validate_match_id("..").is_err());
        assert!(validate_match_id("ok-id_1").is_ok());
    }

    #[test]
    fn container_extractor_does_not_invent_players() {
        let bytes = build_demo("dl_midtown", 1234.0, 37_035);
        let mut reader = DemoReader::new(Cursor::new(bytes.clone()));
        let demo = parse_stream(
            &mut reader,
            PathBuf::from("synthetic.dem"),
            bytes.len() as u64,
        )
        .unwrap();
        let extract = ContainerExtractor.extract(&demo).unwrap();

        assert!(extract.players.is_empty());
        assert!(extract.events.is_empty());
        assert!(extract.positions.is_empty());
        assert_eq!(extract.match_facts.map.as_deref(), Some("dl_midtown"));
        assert!(
            extract
                .missing_fields
                .iter()
                .any(|field| field == "positions")
        );
    }

    #[test]
    fn fixture_kda_uses_resolved_kill_events() {
        let fixture = FixtureFile {
            map: Some("dl_midtown".to_string()),
            players: vec![
                FixturePlayer {
                    account_id: Some(1),
                    name: Some("Alice".to_string()),
                    hero: Some("Haze".to_string()),
                    ..FixturePlayer::default()
                },
                FixturePlayer {
                    account_id: Some(2),
                    name: Some("Bob".to_string()),
                    hero: Some("Mina".to_string()),
                    ..FixturePlayer::default()
                },
            ],
            events: vec![FixtureEvent {
                kind: "kill".to_string(),
                actor: Some("Alice".to_string()),
                target: Some("Bob".to_string()),
                assists: vec!["Alice".to_string()],
                ..FixtureEvent::default()
            }],
            ..FixtureFile::default()
        };

        let extract = fixture_extract(&fixture).unwrap();
        let alice = extract.derived_kda.counts.get("acct:1").unwrap();
        let bob = extract.derived_kda.counts.get("acct:2").unwrap();
        assert_eq!(extract.derived_kda.resolved_kills, 1);
        assert_eq!(alice.kills, 1);
        assert_eq!(alice.assists, 1);
        assert_eq!(bob.deaths, 1);
    }

    #[test]
    fn heatmap_uses_death_positions() {
        let extract = DemoExtract {
            extractor: "fixture".to_string(),
            source: FieldSource::Fixture,
            match_facts: ExtractedMatchFacts::default(),
            players: Vec::new(),
            events: Vec::new(),
            positions: vec![
                DemoPosition {
                    x: 1000.0,
                    y: 2000.0,
                    kind: "death".to_string(),
                    ..DemoPosition::default()
                },
                DemoPosition {
                    x: 1020.0,
                    y: 2020.0,
                    kind: "death".to_string(),
                    ..DemoPosition::default()
                },
                DemoPosition {
                    x: 5000.0,
                    y: 5000.0,
                    kind: "position".to_string(),
                    ..DemoPosition::default()
                },
            ],
            derived_kda: KdaDerivation::default(),
            missing_fields: Vec::new(),
            notes: Vec::new(),
        };
        let mut notes = Vec::new();
        let heatmap = build_heatmap(&extract, &mut notes);

        assert!(heatmap.available);
        assert_eq!(heatmap.basis.as_deref(), Some("deaths"));
        assert_eq!(heatmap.samples, 2);
        assert_eq!(heatmap.cells.len(), 1);
        assert_eq!(heatmap.cells[0].samples, 2);
    }
}
