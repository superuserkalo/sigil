//! A minimal WebM (Matroska) writer for one VP8 video track, after the Matroska and WebM
//! container specs: an EBML header with DocType `webm`, then one Segment holding a SeekHead,
//! Info (millisecond timestamps, the duration), Tracks (`V_VP8` with its size and BT.601
//! colour), one Cluster per key frame of SimpleBlocks, and Cues pointing at each key frame.

const EBML: u32 = 0x1A45_DFA3;
const EBML_VERSION: u32 = 0x4286;
const EBML_READ_VERSION: u32 = 0x42F7;
const EBML_MAX_ID_LENGTH: u32 = 0x42F2;
const EBML_MAX_SIZE_LENGTH: u32 = 0x42F3;
const DOC_TYPE: u32 = 0x4282;
const DOC_TYPE_VERSION: u32 = 0x4287;
const DOC_TYPE_READ_VERSION: u32 = 0x4285;
const SEGMENT: u32 = 0x1853_8067;
const SEEK_HEAD: u32 = 0x114D_9B74;
const SEEK: u32 = 0x4DBB;
const SEEK_ID: u32 = 0x53AB;
const SEEK_POSITION: u32 = 0x53AC;
const INFO: u32 = 0x1549_A966;
const TIMECODE_SCALE: u32 = 0x2A_D7B1;
const DURATION: u32 = 0x4489;
const MUXING_APP: u32 = 0x4D80;
const WRITING_APP: u32 = 0x5741;
const TRACKS: u32 = 0x1654_AE6B;
const TRACK_ENTRY: u32 = 0xAE;
const TRACK_NUMBER: u32 = 0xD7;
const TRACK_UID: u32 = 0x73C5;
const TRACK_TYPE: u32 = 0x83;
const FLAG_LACING: u32 = 0x9C;
const CODEC_ID: u32 = 0x86;
const DEFAULT_DURATION: u32 = 0x23_E383;
const VIDEO: u32 = 0xE0;
const PIXEL_WIDTH: u32 = 0xB0;
const PIXEL_HEIGHT: u32 = 0xBA;
const COLOUR: u32 = 0x55B0;
const MATRIX_COEFFICIENTS: u32 = 0x55B1;
const RANGE: u32 = 0x55B9;
const CLUSTER: u32 = 0x1F43_B675;
const TIMECODE: u32 = 0xE7;
const SIMPLE_BLOCK: u32 = 0xA3;
const CUES: u32 = 0x1C53_BB6B;
const CUE_POINT: u32 = 0xBB;
const CUE_TIME: u32 = 0xB3;
const CUE_TRACK_POSITIONS: u32 = 0xB7;
const CUE_TRACK: u32 = 0xF7;
const CUE_CLUSTER_POSITION: u32 = 0xF1;

/// Element IDs keep their length marker, so they are written as their big-endian bytes.
fn put_id(out: &mut Vec<u8>, id: u32) {
    let bytes = id.to_be_bytes();
    let skip = (id.leading_zeros() / 8) as usize;
    out.extend_from_slice(&bytes[skip..]);
}

/// An element data size as the shortest EBML variable-length integer (all-ones is reserved).
fn put_size(out: &mut Vec<u8>, size: u64) {
    let mut n = 1;
    while n < 8 && size >= (1u64 << (7 * n)) - 1 {
        n += 1;
    }
    let v = size | (1u64 << (7 * n));
    out.extend_from_slice(&v.to_be_bytes()[8 - n..]);
}

fn put_el(out: &mut Vec<u8>, id: u32, body: &[u8]) {
    put_id(out, id);
    put_size(out, body.len() as u64);
    out.extend_from_slice(body);
}

fn put_uint(out: &mut Vec<u8>, id: u32, v: u64) {
    let bytes = v.to_be_bytes();
    let skip = ((v.leading_zeros() / 8) as usize).min(7);
    put_el(out, id, &bytes[skip..]);
}

/// A uint always written in eight bytes, so a SeekHead's size does not depend on its targets.
fn put_uint8(out: &mut Vec<u8>, id: u32, v: u64) {
    put_el(out, id, &v.to_be_bytes());
}

fn seek_head(entries: &[(u32, u64)]) -> Vec<u8> {
    let mut body = Vec::new();
    for &(id, pos) in entries {
        let mut seek = Vec::new();
        let mut idb = Vec::new();
        put_id(&mut idb, id);
        put_el(&mut seek, SEEK_ID, &idb);
        put_uint8(&mut seek, SEEK_POSITION, pos);
        put_el(&mut body, SEEK, &seek);
    }
    let mut out = Vec::new();
    put_el(&mut out, SEEK_HEAD, &body);
    out
}

/// Frame `i`'s timestamp in milliseconds.
fn time_ms(i: usize, fps: u32) -> u64 {
    (i as u64 * 1000 + fps as u64 / 2) / fps as u64
}

/// A WebM file of VP8 `frames` (each with its key frame flag) at a constant `fps`. The first
/// frame must be a key frame.
pub fn mux_webm(frames: &[(&[u8], bool)], w: u32, h: u32, fps: u32) -> Result<Vec<u8>, String> {
    let fps = fps.max(1);
    match frames.first() {
        None => return Err("Empty recording".into()),
        Some((_, false)) => return Err("A WebM recording must start with a key frame".into()),
        _ => {}
    }
    let mut ebml = Vec::new();
    {
        let mut b = Vec::new();
        put_uint(&mut b, EBML_VERSION, 1);
        put_uint(&mut b, EBML_READ_VERSION, 1);
        put_uint(&mut b, EBML_MAX_ID_LENGTH, 4);
        put_uint(&mut b, EBML_MAX_SIZE_LENGTH, 8);
        put_el(&mut b, DOC_TYPE, b"webm");
        put_uint(&mut b, DOC_TYPE_VERSION, 2);
        put_uint(&mut b, DOC_TYPE_READ_VERSION, 2);
        put_el(&mut ebml, EBML, &b);
    }
    let info = {
        let mut b = Vec::new();
        put_uint(&mut b, TIMECODE_SCALE, 1_000_000);
        put_el(&mut b, DURATION, &(frames.len() as f64 * 1000.0 / fps as f64).to_be_bytes());
        put_el(&mut b, MUXING_APP, b"Sigil");
        put_el(&mut b, WRITING_APP, b"Sigil");
        let mut out = Vec::new();
        put_el(&mut out, INFO, &b);
        out
    };
    let tracks = {
        let mut colour = Vec::new();
        put_uint(&mut colour, MATRIX_COEFFICIENTS, 6); // BT.601 (SMPTE 170M)
        put_uint(&mut colour, RANGE, 1); // broadcast (limited) range
        let mut video = Vec::new();
        put_uint(&mut video, PIXEL_WIDTH, w as u64);
        put_uint(&mut video, PIXEL_HEIGHT, h as u64);
        put_el(&mut video, COLOUR, &colour);
        let mut entry = Vec::new();
        put_uint(&mut entry, TRACK_NUMBER, 1);
        put_uint(&mut entry, TRACK_UID, 1);
        put_uint(&mut entry, TRACK_TYPE, 1);
        put_uint(&mut entry, FLAG_LACING, 0);
        put_el(&mut entry, CODEC_ID, b"V_VP8");
        put_uint(&mut entry, DEFAULT_DURATION, 1_000_000_000 / fps as u64);
        put_el(&mut entry, VIDEO, &video);
        let mut b = Vec::new();
        put_el(&mut b, TRACK_ENTRY, &entry);
        let mut out = Vec::new();
        put_el(&mut out, TRACKS, &b);
        out
    };
    // One cluster per key frame; a long run without one is split so block times stay within
    // the signed 16-bit offset from the cluster time.
    let mut clusters: Vec<(u64, bool, Vec<u8>)> = Vec::new();
    let mut body = Vec::new();
    let mut start = (0u64, true);
    for (i, &(data, key)) in frames.iter().enumerate() {
        let t = time_ms(i, fps);
        if i > 0 && (key || t - start.0 > 30_000) {
            clusters.push((start.0, start.1, std::mem::take(&mut body)));
            start = (t, key);
        }
        if body.is_empty() {
            put_uint(&mut body, TIMECODE, t);
        }
        let mut block = Vec::with_capacity(data.len() + 4);
        block.push(0x81); // track 1
        block.extend_from_slice(&((t - start.0) as i16).to_be_bytes());
        block.push(if key { 0x80 } else { 0 });
        block.extend_from_slice(data);
        put_el(&mut body, SIMPLE_BLOCK, &block);
    }
    clusters.push((start.0, start.1, body));
    let clusters: Vec<(u64, bool, Vec<u8>)> = clusters
        .into_iter()
        .map(|(t, key, b)| {
            let mut out = Vec::with_capacity(b.len() + 12);
            put_el(&mut out, CLUSTER, &b);
            (t, key, out)
        })
        .collect();

    // Positions are relative to the start of the Segment's data.
    let head_len = seek_head(&[(INFO, 0), (TRACKS, 0), (CUES, 0)]).len() as u64;
    let info_pos = head_len;
    let tracks_pos = info_pos + info.len() as u64;
    let mut pos = tracks_pos + tracks.len() as u64;
    let mut cue_points = Vec::new();
    for (t, key, c) in &clusters {
        if *key {
            let mut tp = Vec::new();
            put_uint(&mut tp, CUE_TRACK, 1);
            put_uint(&mut tp, CUE_CLUSTER_POSITION, pos);
            let mut cp = Vec::new();
            put_uint(&mut cp, CUE_TIME, *t);
            put_el(&mut cp, CUE_TRACK_POSITIONS, &tp);
            put_el(&mut cue_points, CUE_POINT, &cp);
        }
        pos += c.len() as u64;
    }
    let cues_pos = pos;
    let mut cues = Vec::new();
    put_el(&mut cues, CUES, &cue_points);
    let head = seek_head(&[(INFO, info_pos), (TRACKS, tracks_pos), (CUES, cues_pos)]);
    debug_assert_eq!(head.len() as u64, head_len);

    let segment_len = cues_pos + cues.len() as u64;
    let mut out = Vec::with_capacity(ebml.len() + 12 + segment_len as usize);
    out.extend_from_slice(&ebml);
    put_id(&mut out, SEGMENT);
    put_size(&mut out, segment_len);
    out.extend_from_slice(&head);
    out.extend_from_slice(&info);
    out.extend_from_slice(&tracks);
    for (_, _, c) in &clusters {
        out.extend_from_slice(c);
    }
    out.extend_from_slice(&cues);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_webm_sizes_and_ids_encode() {
        let mut v = Vec::new();
        put_size(&mut v, 0);
        put_size(&mut v, 126);
        put_size(&mut v, 127);
        put_size(&mut v, 16382);
        put_size(&mut v, 16383);
        assert_eq!(v, [0x80, 0xFE, 0x40, 0x7F, 0x7F, 0xFE, 0x20, 0x3F, 0xFF]);
        let mut v = Vec::new();
        put_id(&mut v, EBML);
        put_id(&mut v, TRACK_ENTRY);
        put_id(&mut v, TIMECODE_SCALE);
        assert_eq!(v, [0x1A, 0x45, 0xDF, 0xA3, 0xAE, 0x2A, 0xD7, 0xB1]);
        let mut v = Vec::new();
        put_uint(&mut v, TRACK_NUMBER, 0);
        put_uint(&mut v, TRACK_NUMBER, 300);
        assert_eq!(v, [0xD7, 0x81, 0x00, 0xD7, 0x82, 0x01, 0x2C]);
    }
}
