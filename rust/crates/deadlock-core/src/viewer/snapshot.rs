//! Compiled particle snapshots (`.vsnap_c`) for the particle bundle: the DATA block names the
//! attributes, the block compressed SNAP block holds their rows, one attribute after another.
//!
//! Ported from Grimoire's `src/lib/particles/snapshotResource.ts` (MIT), itself a port of
//! ValveResourceFormat's `ParticleSnapshot.cs` and `BlockCompress.cs` (MIT). Attributes are read
//! in order from the start of the decompressed block, as VRF reads them.

use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

use super::kv::{array, num, text};

/// The most rows a snapshot is read with. Grimoire allows 1 << 20; a hero's snapshots hold a
/// few thousand points, and a cap this low keeps a crafted file from asking for a huge table.
const MAX_ROWS: usize = 65_536;
/// The most bytes a SNAP block may inflate to, and the most decoded output one snapshot may
/// produce (bytes of rows read plus the string table).
pub(crate) const SNAPSHOT_BUDGET: usize = 16 * 1024 * 1024;
/// What every snapshot of one bundle may produce together.
pub(crate) const BUNDLE_SNAPSHOT_BUDGET: usize = 32 * 1024 * 1024;
/// The string table's limits: entries and bytes per entry (joint and hitbox names).
const MAX_STRINGS: usize = 4096;
const MAX_STRING_LEN: usize = 256;
/// Trailing bytes a SNAP block may carry past its attributes (alignment padding).
const MAX_PADDING: usize = 16;

/// One decoded snapshot, as the bundle stores it. `strings` is the DATA block's string list
/// (joint and hitbox names); `string` columns and `skinning` joints are indices into it (-1
/// for an index outside it), never copies, so a small file cannot decode into a large one.
/// `values` is numbers for `float`, `int` and the three components of each `vector` row in
/// turn, string indices for `string`/`bone`, and `{ "joints": [4 per row], "weights": [4 per
/// row] }` for `skinning`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    pub num_particles: usize,
    pub strings: Vec<String>,
    pub columns: Vec<Column>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Column {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    /// `vector`, `float`, `int`, `string` or `skinning`.
    pub data: String,
    pub values: Json,
}

/// The block of `kind` in a compiled resource.
fn resource_block<'a>(bytes: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    let resource = morphic::resource::Resource::parse(bytes).ok()?;
    let block = resource
        .blocks()
        .iter()
        .find(|block| block.kind == *kind)
        .copied()?;
    let start = block.offset as usize;
    bytes.get(start..start.checked_add(block.size as usize)?)
}

/// `BlockCompress.FastDecompress` over a whole block: a declared size (top bit set: stored
/// raw), then 16-flag masks over literal bytes and 2-byte back references.
pub(crate) fn block_decompress(input: &[u8]) -> Result<Vec<u8>, String> {
    let u16_at = |at: usize| -> Result<u16, String> {
        input
            .get(at..at + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .ok_or_else(|| "block compressed data is truncated".to_string())
    };
    let declared = input
        .get(0..4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or("block compressed data is truncated")?;
    if declared > 0x7fff_ffff {
        let size = (declared & 0x7fff_ffff) as usize;
        if size > SNAPSHOT_BUDGET {
            return Err(format!("a {size} byte snapshot block is past the cap"));
        }
        return input
            .get(4..4 + size)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| "block compressed data is truncated".to_string());
    }
    let size = declared as usize;
    if size > SNAPSHOT_BUDGET {
        return Err(format!("a {size} byte snapshot block is past the cap"));
    }
    let mut out = vec![0u8; size];
    let (mut at, mut pos, mut mask, mut left) = (4usize, 0usize, 0u16, 0);
    while pos < size {
        if left == 0 {
            mask = u16_at(at)?;
            at += 2;
            left = 16;
        }
        if mask & 1 != 0 {
            let packed = u16_at(at)? as usize;
            at += 2;
            let offset = (packed >> 4) + 1;
            let length = (packed & 0xf) + 3;
            if offset > pos || length > size - pos {
                return Err("block compressed back reference is outside of the output".into());
            }
            for j in 0..length {
                out[pos + j] = out[pos - offset + j];
            }
            pos += length;
        } else {
            out[pos] = *input.get(at).ok_or("block compressed data is truncated")?;
            at += 1;
            pos += 1;
        }
        mask >>= 1;
        left -= 1;
    }
    Ok(out)
}

/// Reads a compiled snapshot from its file bytes and its decoded DATA block. `budget` is what
/// the bundle's snapshots may still produce; this one's output is taken from it.
pub(crate) fn read_snapshot(bytes: &[u8], data: &Json, budget: &mut usize) -> Result<Snapshot, String> {
    let block = resource_block(bytes, b"SNAP").ok_or("the snapshot has no SNAP block")?;
    let rows = block_decompress(block)?;
    let snapshot = read_rows(&rows, data)?;
    let produced = rows.len() + snapshot.strings.iter().map(String::len).sum::<usize>();
    *budget = budget
        .checked_sub(produced)
        .ok_or("the bundle's snapshots are past their budget")?;
    Ok(snapshot)
}

/// Bytes one row of an attribute takes, or an error for a type this reader does not know.
fn row_size(kind: &str) -> Result<usize, String> {
    match kind {
        "skinning" => Ok(24),
        "string" | "bone" | "float" | "int" => Ok(4),
        "float3" | "vector" => Ok(12),
        other => Err(format!("unsupported SNAP array type {other}")),
    }
}

/// Reads the decompressed SNAP rows the DATA block describes. The block must hold exactly
/// the rows its attributes ask for (up to a little alignment padding), the row count and
/// the string table are capped, and the output stays within `SNAPSHOT_BUDGET`.
pub(crate) fn read_rows(rows: &[u8], data: &Json) -> Result<Snapshot, String> {
    let n = num(data.get("num_particles")).unwrap_or(0.0);
    if n < 0.0 || n.fract() != 0.0 || n > MAX_ROWS as f64 {
        return Err(format!("unsupported snapshot row count {n}"));
    }
    let n = n as usize;
    let list = array(data.get("string_list"));
    if list.len() > MAX_STRINGS {
        return Err(format!("a snapshot string list of {} entries", list.len()));
    }
    let mut strings = Vec::with_capacity(list.len());
    for entry in list {
        let s = entry.as_str().unwrap_or_default();
        if s.len() > MAX_STRING_LEN {
            return Err(format!("a {} byte snapshot string", s.len()));
        }
        strings.push(s.to_string());
    }
    let attributes: Vec<&Json> = array(data.get("attributes"))
        .iter()
        .filter(|a| a.is_object())
        .collect();
    let mut expected = 0usize;
    for attribute in &attributes {
        let size = row_size(text(attribute.get("type")).unwrap_or_default())?;
        expected = size
            .checked_mul(n)
            .and_then(|bytes| expected.checked_add(bytes))
            .ok_or("the snapshot's attributes overflow")?;
    }
    if expected > SNAPSHOT_BUDGET {
        return Err(format!("a {expected} byte snapshot is past the cap"));
    }
    if rows.len() < expected || rows.len() - expected > MAX_PADDING {
        return Err(format!(
            "the SNAP block holds {} bytes where its attributes take {expected}",
            rows.len()
        ));
    }
    let mut at = 0usize;
    let mut take = |len: usize| -> Result<&[u8], String> {
        let slice = at
            .checked_add(len)
            .and_then(|end| rows.get(at..end))
            .ok_or("unable to read beyond end of stream")?;
        at += len;
        Ok(slice)
    };
    let f32s = |b: &[u8]| -> Vec<Json> {
        b.as_chunks::<4>()
            .0
            .iter()
            .map(|c| json!(f32::from_le_bytes(*c)))
            .collect()
    };
    let index = |i: i64| -> i64 {
        if (0..strings.len() as i64).contains(&i) {
            i
        } else {
            -1
        }
    };
    let mut columns: Vec<Column> = Vec::new();
    for attribute in attributes {
        let name = text(attribute.get("name")).unwrap_or_default().to_string();
        let kind = text(attribute.get("type")).unwrap_or_default().to_string();
        let (data_kind, values) = match kind.as_str() {
            "skinning" => {
                let b = take(n * 24)?;
                let mut joints = Vec::with_capacity(n * 4);
                let mut weights = Vec::with_capacity(n * 4);
                for row in b.as_chunks::<24>().0 {
                    for k in 0..4 {
                        joints.push(json!(index(i64::from(i16::from_le_bytes([row[k * 2], row[k * 2 + 1]])))));
                    }
                    weights.extend(f32s(&row[8..24]));
                }
                ("skinning", json!({ "joints": joints, "weights": weights }))
            }
            "string" | "bone" => {
                let out = take(n * 4)?
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|c| json!(index(i64::from(i32::from_le_bytes(*c)))))
                    .collect();
                ("string", Json::Array(out))
            }
            "float3" | "vector" => ("vector", Json::Array(f32s(take(n * 12)?))),
            "float" => ("float", Json::Array(f32s(take(n * 4)?))),
            "int" => (
                "int",
                Json::Array(
                    take(n * 4)?
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|c| json!(i32::from_le_bytes(*c)))
                        .collect(),
                ),
            ),
            other => return Err(format!("unsupported SNAP array type {other}")),
        };
        if columns.iter().any(|c| c.name == name && c.kind == kind) {
            return Err(format!("the attribute ({name}, {kind}) is stored twice"));
        }
        columns.push(Column {
            name,
            kind,
            data: data_kind.to_string(),
            values,
        });
    }
    Ok(Snapshot {
        num_particles: n,
        strings,
        columns,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_block_decompress_literals_and_back_references() {
        // "abcabcabc": three literals, then a back reference 3 back, 6 long.
        let mut input = 9u32.to_le_bytes().to_vec();
        input.extend_from_slice(&0b1000u16.to_le_bytes());
        input.extend_from_slice(b"abc");
        input.extend_from_slice(&(((3 - 1) << 4) | (6 - 3) as u16).to_le_bytes());
        assert_eq!(block_decompress(&input).unwrap(), b"abcabcabc");
        // Stored raw.
        let mut raw = (0x8000_0000u32 | 3).to_le_bytes().to_vec();
        raw.extend_from_slice(b"xyz");
        assert_eq!(block_decompress(&raw).unwrap(), b"xyz");
        // A reference before the start, and truncation, fail.
        let mut bad = 4u32.to_le_bytes().to_vec();
        bad.extend_from_slice(&1u16.to_le_bytes());
        bad.extend_from_slice(&0u16.to_le_bytes());
        assert!(block_decompress(&bad).is_err());
        assert!(block_decompress(&[5, 0, 0, 0, 0]).is_err());
    }

    #[test]
    fn snapshot_rows_read_every_attribute_type() {
        let data = json!({
            "num_particles": 2,
            "string_list": ["pelvis", "head"],
            "attributes": [
                { "name": "position", "type": "float3" },
                { "name": "skinning", "type": "skinning" },
                { "name": "creation_time", "type": "float" },
                { "name": "hitbox", "type": "bone" },
                { "name": "id", "type": "int" }
            ]
        });
        let mut rows = Vec::new();
        for v in [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0] {
            rows.extend_from_slice(&v.to_le_bytes());
        }
        for joints in [[0i16, 1, 0, 0], [1, 1, 1, 1]] {
            for j in joints {
                rows.extend_from_slice(&j.to_le_bytes());
            }
            for w in [0.75f32, 0.25, 0.0, 0.0] {
                rows.extend_from_slice(&w.to_le_bytes());
            }
        }
        for v in [0.0f32, 0.5] {
            rows.extend_from_slice(&v.to_le_bytes());
        }
        for i in [1i32, 9] {
            rows.extend_from_slice(&i.to_le_bytes());
        }
        for i in [7i32, -1] {
            rows.extend_from_slice(&i.to_le_bytes());
        }
        let s = read_rows(&rows, &data).unwrap();
        assert_eq!(s.num_particles, 2);
        let kinds: Vec<&str> = s.columns.iter().map(|c| c.data.as_str()).collect();
        assert_eq!(kinds, ["vector", "skinning", "float", "string", "int"]);
        assert_eq!(s.columns[0].values, json!([1.0, 2.0, 3.0, 4.0, 5.0, 6.0]));
        assert_eq!(s.strings, ["pelvis", "head"]);
        assert_eq!(s.columns[1].values["joints"], json!([0, 1, 0, 0, 1, 1, 1, 1]));
        assert_eq!(s.columns[1].values["weights"], json!([0.75, 0.25, 0.0, 0.0, 0.75, 0.25, 0.0, 0.0]));
        // String columns are indices into the table; one outside it is -1.
        assert_eq!(s.columns[3].values, json!([1, -1]));
        assert_eq!(s.columns[4].values, json!([7, -1]));
        // Too few rows is an error, as is a block with more than padding left over, and an
        // unknown type.
        assert!(read_rows(&rows[..10], &data).is_err());
        let mut long = rows.clone();
        long.extend_from_slice(&[0; 64]);
        assert!(read_rows(&long, &data).is_err());
        long.truncate(rows.len() + 8);
        assert!(read_rows(&long, &data).is_ok(), "alignment padding is allowed");
        assert!(
            read_rows(
                &[],
                &json!({ "num_particles": 1, "attributes": [{ "name": "x", "type": "matrix" }] })
            )
            .is_err()
        );
    }

    /// A crafted header cannot make a small file decode into a large one: the row count, the
    /// string table, the block size and the bundle's running total are all capped.
    #[test]
    fn snapshot_crafted_headers_are_refused() {
        let skin = |n: usize, strings: Json| json!({ "num_particles": n, "string_list": strings, "attributes": [{ "name": "skinning", "type": "skinning" }] });
        // More rows than the cap.
        assert!(read_rows(&[], &skin(MAX_ROWS + 1, json!([]))).is_err());
        // A long joint name repeated over every row used to be cloned per row: now the table
        // caps its entries and rows hold indices.
        let long = "x".repeat(MAX_STRING_LEN + 1);
        assert!(read_rows(&vec![0; 24], &skin(1, json!([long]))).is_err());
        let many: Vec<Json> = (0..=MAX_STRINGS).map(|i| json!(i.to_string())).collect();
        assert!(read_rows(&vec![0; 24], &skin(1, Json::Array(many))).is_err());
        let rows = vec![0u8; 24 * 1000];
        let s = read_rows(&rows, &skin(1000, json!(["a".repeat(MAX_STRING_LEN)]))).unwrap();
        assert_eq!(s.strings.len(), 1, "one copy of the name, not one per row");
        // Rows the attributes do not account for (a header claiming fewer rows) are refused.
        assert!(read_rows(&rows, &skin(10, json!(["a"]))).is_err());
        // A block declaring more than the budget fails before allocating it.
        let mut big = ((SNAPSHOT_BUDGET + 1) as u32).to_le_bytes().to_vec();
        big.extend_from_slice(&[0; 8]);
        assert!(block_decompress(&big).unwrap_err().contains("cap"));
        let mut raw = ((SNAPSHOT_BUDGET as u32 + 1) | 0x8000_0000).to_le_bytes().to_vec();
        raw.extend_from_slice(&[0; 8]);
        assert!(block_decompress(&raw).is_err());
    }
}
