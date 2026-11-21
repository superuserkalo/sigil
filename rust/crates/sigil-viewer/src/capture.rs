//! Recordings of the 3D viewer: an animated GIF and a short video.
//!
//! Pure CPU encoders over the frames `Viewer::render` returns (sRGB RGBA8, rows top first,
//! tightly packed). Nothing here touches the GPU, the UI or the disk: callers push frames and
//! get bytes back to save.
//!
//! The GIF follows Grimoire's viewer recorder (`viewerGif.ts`, MIT): 12 fps for at most 4
//! seconds, the long edge scaled to 480, a 24 MiB cap, and a fresh palette per frame. The
//! palette comes from a histogram of opaque pixels on 5-bit keys, cut by weighted median into
//! at most 512 boxes, which are then reduced to 256 colors (255 plus a transparent index 0 when
//! the frame has see-through pixels). Grimoire reduces with gifenc's pairwise nearest neighbor
//! quantizer on rgb565 bins; this port runs the same pairwise nearest neighbor merge (cost
//! `n1 * n2 / (n1 + n2) * distance^2`) on those weighted colors. Pixels get a 4x4 Bayer offset
//! before mapping, and the nearest palette entry is cached per rgb565 key, as gifenc does.
//!
//! The video matches Deadlock Mod Manager's recorder (30 fps, at most 10 seconds, WebM): frames
//! go through the VP8 encoder in `capture/vp8.rs` (pure Rust, written here from RFC 6386) into
//! the WebM writer in `capture/webm.rs`, which plays in browsers, Discord, VLC and most players.
//! Motion JPEG in a QuickTime `.mov` or a RIFF `.avi` stays available as a fallback format.

use std::ops::Range;
use std::time::Duration;

mod vp8;
mod vp8_tables;
mod webm;

pub use vp8::{DEFAULT_KEYFRAME_INTERVAL, DEFAULT_Q_INDEX, Vp8Encoder, Vp8Frame, Vp8Options};
pub use webm::mux_webm;

/// Grimoire's recording limits.
pub const GIF_MAX_SECONDS: f32 = 4.0;
pub const GIF_FPS: f32 = 12.0;
pub const GIF_MAX_EDGE: u32 = 480;
pub const GIF_MAX_BYTES: usize = 24 * 1024 * 1024;

/// Deadlock Mod Manager's recording limits.
pub const VIDEO_FPS: u32 = 30;
pub const VIDEO_MAX_SECONDS: u32 = 10;
/// Video quality (1 to 100): the Motion JPEG quality, and the VP8 quantizer through
/// `vp8_q_index` (85 gives `DEFAULT_Q_INDEX`).
pub const VIDEO_JPEG_QUALITY: u8 = 85;

/// The VP8 quantizer index for a 1 to 100 quality: 100 is the finest (0), 85 the default.
pub fn vp8_q_index(quality: u8) -> u8 {
    let q = 100 - quality.clamp(1, 100) as u32;
    ((q * DEFAULT_Q_INDEX as u32 + 7) / 15).min(127) as u8
}

const MAX_BOXES: usize = 512;
const PALETTE_SAMPLES: f64 = 8192.0;
const BAYER: [i32; 16] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GifOptions {
    pub fps: f32,
    pub max_seconds: f32,
    pub max_edge: u32,
    pub max_bytes: usize,
}

impl Default for GifOptions {
    fn default() -> Self {
        GifOptions { fps: GIF_FPS, max_seconds: GIF_MAX_SECONDS, max_edge: GIF_MAX_EDGE, max_bytes: GIF_MAX_BYTES }
    }
}

impl GifOptions {
    /// How many frames a recording keeps: `fps * max_seconds`, at least one.
    pub fn max_frames(&self) -> usize {
        ((self.fps * self.max_seconds).round() as usize).max(1)
    }

    /// Time between captured frames.
    pub fn frame_interval(&self) -> Duration {
        Duration::from_secs_f32(1.0 / self.fps.max(0.001))
    }

    /// GIF delay in hundredths of a second (8 at 12 fps, as gifenc rounds 1000 / 12 ms).
    pub fn delay_centis(&self) -> u16 {
        (100.0 / self.fps.max(0.001)).round().clamp(1.0, u16::MAX as f32) as u16
    }
}

/// The GIF size for a `w` x `h` canvas: the long edge scaled down to 480, never up.
pub fn gif_dimensions(w: u32, h: u32) -> (u32, u32) {
    fit_dimensions(w, h, GIF_MAX_EDGE)
}

/// `w` x `h` scaled so the long edge is at most `max_edge` (never up, never below 1).
pub fn fit_dimensions(w: u32, h: u32, max_edge: u32) -> (u32, u32) {
    let (w, h) = (w.max(1), h.max(1));
    let scale = (max_edge.max(1) as f64 / w.max(h) as f64).min(1.0);
    (((w as f64 * scale).round() as u32).max(1), ((h as f64 * scale).round() as u32).max(1))
}

/// Grimoire's trim clamp: a non-empty range inside `0..count` (count must be at least 1).
pub fn gif_trim_bounds(count: usize, range: Range<usize>) -> Range<usize> {
    let first = range.start.min(count.saturating_sub(1));
    first..range.end.min(count).max(first + 1)
}

/// Box filter resize of RGBA8 with premultiplied alpha, so transparent pixels do not darken
/// edges. Each destination pixel averages the source pixels its footprint covers.
pub fn downscale(rgba: &[u8], w: u32, h: u32, nw: u32, nh: u32) -> Vec<u8> {
    let (w, h, nw, nh) = (w as usize, h as usize, nw.max(1) as usize, nh.max(1) as usize);
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return vec![0; nw * nh * 4];
    }
    if w == nw && h == nh {
        return rgba[..w * h * 4].to_vec();
    }
    let span = |i: usize, src: usize, dst: usize| {
        let a = i * src / dst;
        let b = ((i + 1) * src / dst).max(a + 1).min(src);
        (a, b)
    };
    let xs: Vec<(usize, usize)> = (0..nw).map(|x| span(x, w, nw)).collect();
    let mut out = vec![0u8; nw * nh * 4];
    let mut acc = vec![[0u64; 4]; nw];
    for y in 0..nh {
        let (y0, y1) = span(y, h, nh);
        acc.iter_mut().for_each(|a| *a = [0; 4]);
        for sy in y0..y1 {
            let row = &rgba[sy * w * 4..(sy + 1) * w * 4];
            for (a, &(x0, x1)) in acc.iter_mut().zip(&xs) {
                for p in row[x0 * 4..x1 * 4].as_chunks::<4>().0 {
                    let al = p[3] as u64;
                    a[0] += p[0] as u64 * al;
                    a[1] += p[1] as u64 * al;
                    a[2] += p[2] as u64 * al;
                    a[3] += al;
                }
            }
        }
        let rows = (y1 - y0) as u64;
        for (x, (a, &(x0, x1))) in acc.iter().zip(&xs).enumerate() {
            let o = &mut out[(y * nw + x) * 4..(y * nw + x) * 4 + 4];
            if a[3] == 0 {
                continue;
            }
            let area = rows * (x1 - x0) as u64;
            for c in 0..3 {
                o[c] = ((a[c] + a[3] / 2) / a[3]).min(255) as u8;
            }
            o[3] = ((a[3] + area / 2) / area).min(255) as u8;
        }
    }
    out
}

/// One quantized GIF frame.
struct IndexedFrame {
    palette: Vec<u8>,
    indices: Vec<u8>,
    transparent: Option<u8>,
}

#[derive(Clone, Copy)]
struct Bin {
    rgb: [f32; 3],
    count: u32,
}

struct CutBox {
    start: usize,
    end: usize,
    axis: usize,
    count: u64,
    score: f64,
}

fn cut_box(bins: &[Bin], start: usize, end: usize) -> CutBox {
    let entries = &bins[start..end];
    let mut lo = [255f32; 3];
    let mut hi = [0f32; 3];
    let mut count = 0u64;
    for b in entries {
        for c in 0..3 {
            lo[c] = lo[c].min(b.rgb[c]);
            hi[c] = hi[c].max(b.rgb[c]);
        }
        count += b.count as u64;
    }
    let ranges = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
    let mut axis = 0;
    for c in 1..3 {
        if ranges[c] > ranges[axis] {
            axis = c;
        }
    }
    let score = if entries.len() > 1 { ranges[axis] as f64 * (count as f64).sqrt() } else { 0.0 };
    CutBox { start, end, axis, count, score }
}

/// Grimoire's `paletteSamples`: weighted colors (rgb, sample count) summarizing the opaque
/// pixels in at most 512 boxes, weights summing to about 8192.
fn palette_samples(rgba: &[u8]) -> Vec<([u8; 3], u32)> {
    let mut hist = vec![[0u32; 4]; 1 << 15];
    for p in rgba.as_chunks::<4>().0 {
        if p[3] < 128 {
            continue;
        }
        let key = ((p[0] as usize >> 3) << 10) | ((p[1] as usize >> 3) << 5) | (p[2] as usize >> 3);
        let h = &mut hist[key];
        h[0] += p[0] as u32;
        h[1] += p[1] as u32;
        h[2] += p[2] as u32;
        h[3] += 1;
    }
    let mut bins: Vec<Bin> = hist
        .iter()
        .filter(|h| h[3] > 0)
        .map(|h| {
            let n = h[3] as f32;
            Bin { rgb: [h[0] as f32 / n, h[1] as f32 / n, h[2] as f32 / n], count: h[3] }
        })
        .collect();
    if bins.is_empty() {
        return Vec::new();
    }
    let total: u64 = bins.iter().map(|b| b.count as u64).sum();
    let mut boxes = vec![cut_box(&bins, 0, bins.len())];
    while boxes.len() < MAX_BOXES {
        let mut selected: Option<usize> = None;
        for (i, b) in boxes.iter().enumerate() {
            if b.score > selected.map_or(0.0, |s| boxes[s].score) {
                selected = Some(i);
            }
        }
        let Some(sel) = selected else { break };
        let (start, end, axis, count) = (boxes[sel].start, boxes[sel].end, boxes[sel].axis, boxes[sel].count);
        bins[start..end].sort_by(|a, b| a.rgb[axis].total_cmp(&b.rgb[axis]));
        let len = end - start;
        let mut split = 1;
        let mut acc = bins[start].count as u64;
        while split < len - 1 && (acc as f64) < count as f64 / 2.0 {
            acc += bins[start + split].count as u64;
            split += 1;
        }
        let left = cut_box(&bins, start, start + split);
        let right = cut_box(&bins, start + split, end);
        boxes[sel] = left;
        boxes.insert(sel + 1, right);
    }
    boxes
        .iter()
        .map(|b| {
            let mut sum = [0f64; 3];
            for bin in &bins[b.start..b.end] {
                for (s, v) in sum.iter_mut().zip(bin.rgb) {
                    *s += v as f64 * bin.count as f64;
                }
            }
            let rgb = sum.map(|s| (s / b.count as f64).round().clamp(0.0, 255.0) as u8);
            let n = ((b.count as f64 / total as f64 * PALETTE_SAMPLES).round() as u32).max(1);
            (rgb, n)
        })
        .collect()
}

fn rgb565(r: u8, g: u8, b: u8) -> usize {
    (((r as usize) << 8) & 0xf800) | (((g as usize) << 3) & 0x07e0) | (b as usize >> 3)
}

/// Pairwise nearest neighbor reduction of weighted colors to at most `max_colors`, after
/// merging colors that share an rgb565 key (gifenc's `quantize` with `format: 'rgb565'`).
fn pnn_quantize(samples: &[([u8; 3], u32)], max_colors: usize) -> Vec<[u8; 3]> {
    struct Node {
        rgb: [f64; 3],
        n: f64,
        nn: usize,
        cost: f64,
        live: bool,
    }
    let mut by_key: Vec<(usize, [f64; 3], f64)> = Vec::new();
    {
        let mut slot = std::collections::HashMap::new();
        for &(rgb, n) in samples {
            let key = rgb565(rgb[0], rgb[1], rgb[2]);
            let i = *slot.entry(key).or_insert_with(|| {
                by_key.push((key, [0.0; 3], 0.0));
                by_key.len() - 1
            });
            for (s, v) in by_key[i].1.iter_mut().zip(rgb) {
                *s += v as f64 * n as f64;
            }
            by_key[i].2 += n as f64;
        }
    }
    let mut nodes: Vec<Node> = by_key
        .iter()
        .map(|&(_, s, n)| Node { rgb: [s[0] / n, s[1] / n, s[2] / n], n, nn: 0, cost: f64::INFINITY, live: true })
        .collect();
    let cost = |a: &Node, b: &Node| {
        let d = (a.rgb[0] - b.rgb[0]).powi(2) + (a.rgb[1] - b.rgb[1]).powi(2) + (a.rgb[2] - b.rgb[2]).powi(2);
        a.n * b.n / (a.n + b.n) * d
    };
    let find_nn = |nodes: &mut Vec<Node>, i: usize| {
        let mut best = (usize::MAX, f64::INFINITY);
        for j in 0..nodes.len() {
            if j != i && nodes[j].live {
                let c = cost(&nodes[i], &nodes[j]);
                if c < best.1 {
                    best = (j, c);
                }
            }
        }
        nodes[i].nn = best.0;
        nodes[i].cost = best.1;
    };
    let mut live = nodes.len();
    if live > max_colors {
        for i in 0..nodes.len() {
            find_nn(&mut nodes, i);
        }
    }
    while live > max_colors.max(1) {
        let mut a = usize::MAX;
        for i in 0..nodes.len() {
            if nodes[i].live && (a == usize::MAX || nodes[i].cost < nodes[a].cost) {
                a = i;
            }
        }
        let b = nodes[a].nn;
        let (na, nb) = (nodes[a].n, nodes[b].n);
        let d = 1.0 / (na + nb);
        for c in 0..3 {
            nodes[a].rgb[c] = d * (na * nodes[a].rgb[c] + nb * nodes[b].rgb[c]);
        }
        nodes[a].n = na + nb;
        nodes[b].live = false;
        live -= 1;
        if live <= max_colors {
            break;
        }
        find_nn(&mut nodes, a);
        for i in 0..nodes.len() {
            if i == a || !nodes[i].live {
                continue;
            }
            if nodes[i].nn == a || nodes[i].nn == b {
                find_nn(&mut nodes, i);
            } else {
                let c = cost(&nodes[i], &nodes[a]);
                if c < nodes[i].cost {
                    nodes[i].nn = a;
                    nodes[i].cost = c;
                }
            }
        }
    }
    nodes
        .iter()
        .filter(|n| n.live)
        .map(|n| n.rgb.map(|v| v.round().clamp(0.0, 255.0) as u8))
        .collect()
}

/// Grimoire's `gifFramePalette`: palette, Bayer-dithered indices and the transparent index.
fn quantize_frame(rgba: &[u8], width: usize) -> IndexedFrame {
    let has_alpha = rgba.as_chunks::<4>().0.iter().any(|p| p[3] < 128);
    let samples = palette_samples(rgba);
    let colors =
        if samples.is_empty() { vec![[0u8; 3]] } else { pnn_quantize(&samples, if has_alpha { 255 } else { 256 }) };
    let pal: Vec<[i32; 3]> = colors.iter().map(|c| c.map(|v| v as i32)).collect();
    // `(b - 7.5) * 0.75` never lands on .5, so rounding the offset equals rounding the sum.
    let offsets: [i32; 16] = BAYER.map(|b| ((b as f32 - 7.5) * 0.75).round() as i32);
    let mut cache = vec![u16::MAX; 1 << 16];
    let mut indices = Vec::with_capacity(rgba.len() / 4);
    let base = u8::from(has_alpha);
    for (i, p) in rgba.as_chunks::<4>().0.iter().enumerate() {
        if has_alpha && p[3] < 128 {
            indices.push(0);
            continue;
        }
        let off = offsets[((i / width) % 4) * 4 + (i % width) % 4];
        let r = (p[0] as i32 + off).clamp(0, 255);
        let g = (p[1] as i32 + off).clamp(0, 255);
        let b = (p[2] as i32 + off).clamp(0, 255);
        let key = rgb565(r as u8, g as u8, b as u8);
        let mut idx = cache[key];
        if idx == u16::MAX {
            let mut best = (0usize, i32::MAX);
            for (j, c) in pal.iter().enumerate() {
                let d = (c[0] - r).pow(2) + (c[1] - g).pow(2) + (c[2] - b).pow(2);
                if d < best.1 {
                    best = (j, d);
                }
            }
            idx = best.0 as u16;
            cache[key] = idx;
        }
        indices.push(idx as u8 + base);
    }
    let mut palette = Vec::with_capacity((colors.len() + 1) * 3);
    if has_alpha {
        palette.extend_from_slice(&[0, 0, 0]);
    }
    for c in &colors {
        palette.extend_from_slice(c);
    }
    IndexedFrame { palette, indices, transparent: has_alpha.then_some(0) }
}

/// Encode `frames` (each `w` x `h` RGBA8) as a looping GIF, one palette per frame, disposed to
/// background. Fails past `opts.max_bytes`. Frames are quantized in parallel batches.
pub fn encode_gif(frames: &[&[u8]], w: u32, h: u32, opts: &GifOptions) -> Result<Vec<u8>, String> {
    if frames.is_empty() {
        return Err("Empty recording".into());
    }
    if w == 0 || h == 0 || w > u16::MAX as u32 || h > u16::MAX as u32 {
        return Err(format!("Invalid GIF size {w}x{h}"));
    }
    let px = w as usize * h as usize * 4;
    if let Some(i) = frames.iter().position(|f| f.len() < px) {
        return Err(format!("GIF frame {i} is smaller than {w}x{h}"));
    }
    let mut encoder = gif::Encoder::new(Vec::new(), w as u16, h as u16, &[]).map_err(|e| e.to_string())?;
    encoder.set_repeat(gif::Repeat::Infinite).map_err(|e| e.to_string())?;
    let delay = opts.delay_centis();
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 16);
    for batch in frames.chunks(threads) {
        let indexed: Vec<IndexedFrame> = if batch.len() == 1 {
            vec![quantize_frame(&batch[0][..px], w as usize)]
        } else {
            std::thread::scope(|s| {
                let handles: Vec<_> =
                    batch.iter().map(|f| s.spawn(move || quantize_frame(&f[..px], w as usize))).collect();
                handles.into_iter().map(|h| h.join().expect("GIF quantizer thread")).collect()
            })
        };
        for f in indexed {
            let frame = gif::Frame {
                delay,
                dispose: gif::DisposalMethod::Background,
                transparent: f.transparent,
                width: w as u16,
                height: h as u16,
                palette: Some(f.palette),
                buffer: std::borrow::Cow::Owned(f.indices),
                ..gif::Frame::default()
            };
            encoder.write_frame(&frame).map_err(|e| e.to_string())?;
            if encoder.get_ref().len() > opts.max_bytes {
                return Err("Recording size limit exceeded".into());
            }
        }
    }
    let out = encoder.into_inner().map_err(|e| e.to_string())?;
    if out.len() > opts.max_bytes {
        return Err("Recording size limit exceeded".into());
    }
    Ok(out)
}

/// Collects viewer frames for a GIF: downscaled on push to the recording size, capped at
/// `fps * max_seconds` frames. The caller paces pushes (`frame_interval`), shows frames for
/// review with `frame`, and saves any trimmed range with `finish`.
#[derive(Debug, Clone)]
pub struct GifRecorder {
    opts: GifOptions,
    width: u32,
    height: u32,
    frames: Vec<Vec<u8>>,
}

impl Default for GifRecorder {
    fn default() -> Self {
        GifRecorder::new(GifOptions::default())
    }
}

impl GifRecorder {
    pub fn new(opts: GifOptions) -> Self {
        GifRecorder { opts, width: 0, height: 0, frames: Vec::new() }
    }

    pub fn options(&self) -> &GifOptions {
        &self.opts
    }

    pub fn max_frames(&self) -> usize {
        self.opts.max_frames()
    }

    pub fn frame_interval(&self) -> Duration {
        self.opts.frame_interval()
    }

    /// Add a full-size frame. The first frame fixes the recording size; later frames of another
    /// size are scaled to it. `Ok(false)` once the recording is full (the frame is dropped).
    pub fn push(&mut self, rgba: &[u8], w: u32, h: u32) -> Result<bool, String> {
        if w == 0 || h == 0 || rgba.len() < w as usize * h as usize * 4 {
            return Err(format!("frame buffer does not hold {w}x{h} RGBA"));
        }
        if self.is_full() {
            return Ok(false);
        }
        if self.frames.is_empty() {
            (self.width, self.height) = fit_dimensions(w, h, self.opts.max_edge);
        }
        self.frames.push(downscale(rgba, w, h, self.width, self.height));
        Ok(true)
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.frames.len() >= self.max_frames()
    }

    /// The recording size (0 x 0 before the first frame).
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// A recorded frame at the recording size, for review thumbnails and preview.
    pub fn frame(&self, i: usize) -> Option<(&[u8], u32, u32)> {
        self.frames.get(i).map(|f| (f.as_slice(), self.width, self.height))
    }

    pub fn clear(&mut self) {
        self.frames.clear();
        (self.width, self.height) = (0, 0);
    }

    /// Encode frames `range` (clamped like Grimoire's trim, never empty) as a GIF.
    pub fn finish(&self, range: Range<usize>) -> Result<Vec<u8>, String> {
        if self.frames.is_empty() {
            return Err("Empty recording".into());
        }
        let range = gif_trim_bounds(self.frames.len(), range);
        let frames: Vec<&[u8]> = self.frames[range].iter().map(Vec::as_slice).collect();
        encode_gif(&frames, self.width, self.height, &self.opts)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoFormat {
    /// WebM with VP8: browsers, Discord, VLC and most players, on every platform.
    WebM,
    /// QuickTime with Photo JPEG samples: QuickTime Player, macOS and VLC.
    Mov,
    /// RIFF AVI with MJPG: Windows' players and VLC.
    Avi,
}

impl VideoFormat {
    pub fn extension(self) -> &'static str {
        match self {
            VideoFormat::WebM => "webm",
            VideoFormat::Mov => "mov",
            VideoFormat::Avi => "avi",
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            VideoFormat::WebM => "video/webm",
            VideoFormat::Mov => "video/quicktime",
            VideoFormat::Avi => "video/x-msvideo",
        }
    }
}

/// The recording format: WebM (VP8) everywhere, as Deadlock Mod Manager records.
pub fn native_video_format() -> VideoFormat {
    VideoFormat::WebM
}

/// The longest edge a video recording keeps; larger viewports are scaled down before encoding
/// so one VP8 encoder thread keeps up with 30 fps.
pub const VIDEO_MAX_EDGE: u32 = 1280;

/// Records viewer frames. Each push encodes its frame right away (VP8 for WebM, a JPEG for
/// `.mov` and `.avi`), so memory holds compressed frames only. Odd sizes are cropped to even
/// (at least 2).
#[derive(Debug, Clone)]
pub struct VideoRecorder {
    format: VideoFormat,
    src_w: u32,
    src_h: u32,
    width: u32,
    height: u32,
    fps: u32,
    quality: u8,
    frames: Vec<Vec<u8>>,
    /// Key frame flags of WebM frames.
    keys: Vec<bool>,
    vp8: Option<Box<Vp8Encoder>>,
    rgb: Vec<u8>,
}

impl VideoRecorder {
    pub fn new(w: u32, h: u32, fps: u32, format: VideoFormat) -> Self {
        VideoRecorder {
            format,
            src_w: w,
            src_h: h,
            width: (w & !1).max(2),
            height: (h & !1).max(2),
            fps: fps.max(1),
            quality: VIDEO_JPEG_QUALITY,
            frames: Vec::new(),
            keys: Vec::new(),
            vp8: None,
            rgb: Vec::new(),
        }
    }

    /// Quality 1 to 100: the JPEG quality of frames pushed after this call, or the VP8
    /// quantizer (`vp8_q_index`) when set before the first WebM frame.
    pub fn set_quality(&mut self, quality: u8) {
        self.quality = quality.clamp(1, 100);
    }

    pub fn format(&self) -> VideoFormat {
        self.format
    }

    pub fn extension(&self) -> &'static str {
        self.format.extension()
    }

    /// The encoded (even) size.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn fps(&self) -> u32 {
        self.fps
    }

    pub fn frame_interval(&self) -> Duration {
        Duration::from_secs_f64(1.0 / self.fps as f64)
    }

    pub fn max_frames(&self) -> usize {
        (self.fps * VIDEO_MAX_SECONDS) as usize
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.frames.len() >= self.max_frames()
    }

    /// Compressed bytes held so far.
    pub fn bytes(&self) -> usize {
        self.frames.iter().map(Vec::len).sum()
    }

    /// Encode one `w` x `h` RGBA frame (the size given to `new`), alpha composited onto black.
    /// `Ok(false)` once the recording is full (the frame is dropped).
    pub fn push(&mut self, rgba: &[u8]) -> Result<bool, String> {
        let (sw, sh) = (self.src_w as usize, self.src_h as usize);
        if rgba.len() < sw * sh * 4 {
            return Err(format!("frame buffer does not hold {sw}x{sh} RGBA"));
        }
        if self.is_full() {
            return Ok(false);
        }
        if self.format == VideoFormat::WebM {
            return self.push_vp8(rgba).map(|_| true);
        }
        let (w, h) = (self.width as usize, self.height as usize);
        self.rgb.clear();
        self.rgb.resize(w * h * 3, 0);
        for y in 0..h.min(sh) {
            let src = &rgba[y * sw * 4..y * sw * 4 + w.min(sw) * 4];
            let dst = &mut self.rgb[y * w * 3..y * w * 3 + w.min(sw) * 3];
            for (d, s) in dst.as_chunks_mut::<3>().0.iter_mut().zip(src.as_chunks::<4>().0) {
                let a = s[3] as u32;
                if a == 255 {
                    d.copy_from_slice(&s[..3]);
                } else {
                    for c in 0..3 {
                        d[c] = ((s[c] as u32 * a + 127) / 255) as u8;
                    }
                }
            }
        }
        self.frames.push(encode_jpeg(&self.rgb, self.width, self.height, self.quality)?);
        Ok(true)
    }

    /// Encode one frame as VP8, cropped to the even size when the source is odd.
    fn push_vp8(&mut self, rgba: &[u8]) -> Result<(), String> {
        if self.vp8.is_none() {
            let opts = Vp8Options { q_index: vp8_q_index(self.quality), ..Vp8Options::default() };
            self.vp8 = Some(Box::new(Vp8Encoder::new(self.width, self.height, opts)?));
        }
        let (sw, w, h) = (self.src_w as usize, self.width as usize, self.height as usize);
        let frame = if sw == w {
            &rgba[..w * h * 4]
        } else {
            self.rgb.clear();
            for y in 0..h {
                self.rgb.extend_from_slice(&rgba[y * sw * 4..y * sw * 4 + w * 4]);
            }
            &self.rgb[..]
        };
        let Some(enc) = self.vp8.as_mut() else { return Err("VP8 encoder missing".into()) };
        let f = enc.encode_rgba(frame)?;
        self.frames.push(f.data);
        self.keys.push(f.key);
        Ok(())
    }

    /// The finished file in this recorder's format.
    pub fn finish(&self) -> Result<Vec<u8>, String> {
        if self.frames.is_empty() {
            return Err("Empty recording".into());
        }
        match self.format {
            VideoFormat::WebM => {
                let frames: Vec<(&[u8], bool)> =
                    self.frames.iter().map(Vec::as_slice).zip(self.keys.iter().copied()).collect();
                mux_webm(&frames, self.width, self.height, self.fps)
            }
            VideoFormat::Mov => mux_mov(&self.frames, self.width, self.height, self.fps),
            VideoFormat::Avi => mux_avi(&self.frames, self.width, self.height, self.fps),
        }
    }
}

fn encode_jpeg(rgb: &[u8], w: u32, h: u32, quality: u8) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(rgb.len() / 8);
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality)
        .encode(rgb, w, h, image::ExtendedColorType::Rgb8)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

fn put16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_be_bytes());
}

fn put32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_be_bytes());
}

fn atom(kind: &[u8; 4], parts: &[&[u8]]) -> Vec<u8> {
    let len: usize = 8 + parts.iter().map(|p| p.len()).sum::<usize>();
    let mut v = Vec::with_capacity(len);
    put32(&mut v, len as u32);
    v.extend_from_slice(kind);
    for p in parts {
        v.extend_from_slice(p);
    }
    v
}

const MATRIX: [u32; 9] = [0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000];

/// A QuickTime counted string: length byte, then the bytes.
fn pascal(v: &mut Vec<u8>, s: &str) {
    v.push(s.len() as u8);
    v.extend_from_slice(s.as_bytes());
}

fn handler(component: &[u8; 4], subtype: &[u8; 4], name: &str) -> Vec<u8> {
    let mut b = Vec::new();
    put32(&mut b, 0);
    b.extend_from_slice(component);
    b.extend_from_slice(subtype);
    put32(&mut b, 0);
    put32(&mut b, 0);
    put32(&mut b, 0);
    pascal(&mut b, name);
    atom(b"hdlr", &[&b])
}

/// A QuickTime movie of Photo JPEG samples, all in one chunk after the `ftyp`.
pub fn mux_mov(jpegs: &[Vec<u8>], w: u32, h: u32, fps: u32) -> Result<Vec<u8>, String> {
    let n = jpegs.len() as u32;
    let fps = fps.max(1);
    if w > u16::MAX as u32 || h > u16::MAX as u32 {
        return Err(format!("video size {w}x{h} is too large"));
    }
    let data: usize = jpegs.iter().map(Vec::len).sum();
    if data + 8 > u32::MAX as usize - 1024 * 1024 {
        return Err("Recording size limit exceeded".into());
    }
    let ftyp = {
        let mut b = Vec::new();
        b.extend_from_slice(b"qt  ");
        put32(&mut b, 0x2005_0300);
        b.extend_from_slice(b"qt  ");
        atom(b"ftyp", &[&b])
    };
    let chunk_offset = (ftyp.len() + 8) as u32;
    let movie_scale = 1000u32;
    let media_scale = fps * 100;
    let media_duration = n * 100;
    let movie_duration = (n as u64 * 1000 / fps as u64) as u32;

    let mvhd = {
        let mut b = Vec::new();
        put32(&mut b, 0);
        put32(&mut b, 0);
        put32(&mut b, 0);
        put32(&mut b, movie_scale);
        put32(&mut b, movie_duration);
        put32(&mut b, 0x0001_0000);
        put16(&mut b, 0x0100);
        b.extend_from_slice(&[0; 10]);
        MATRIX.iter().for_each(|&m| put32(&mut b, m));
        (0..6).for_each(|_| put32(&mut b, 0));
        put32(&mut b, 2);
        atom(b"mvhd", &[&b])
    };
    let tkhd = {
        let mut b = Vec::new();
        put32(&mut b, 0x0000_000F);
        put32(&mut b, 0);
        put32(&mut b, 0);
        put32(&mut b, 1);
        put32(&mut b, 0);
        put32(&mut b, movie_duration);
        b.extend_from_slice(&[0; 8]);
        put16(&mut b, 0);
        put16(&mut b, 0);
        put16(&mut b, 0);
        put16(&mut b, 0);
        MATRIX.iter().for_each(|&m| put32(&mut b, m));
        put32(&mut b, w << 16);
        put32(&mut b, h << 16);
        atom(b"tkhd", &[&b])
    };
    let mdhd = {
        let mut b = Vec::new();
        put32(&mut b, 0);
        put32(&mut b, 0);
        put32(&mut b, 0);
        put32(&mut b, media_scale);
        put32(&mut b, media_duration);
        put16(&mut b, 0);
        put16(&mut b, 0);
        atom(b"mdhd", &[&b])
    };
    let vmhd = {
        let mut b = Vec::new();
        put32(&mut b, 0x0000_0001);
        put16(&mut b, 0x0040);
        (0..3).for_each(|_| put16(&mut b, 0x8000));
        atom(b"vmhd", &[&b])
    };
    let dinf = {
        let alis = atom(b"alis", &[&[0, 0, 0, 1]]);
        let mut b = Vec::new();
        put32(&mut b, 0);
        put32(&mut b, 1);
        atom(b"dinf", &[&atom(b"dref", &[&b, &alis])])
    };
    let stsd = {
        let mut e = Vec::new();
        e.extend_from_slice(&[0; 6]);
        put16(&mut e, 1);
        put16(&mut e, 0);
        put16(&mut e, 0);
        e.extend_from_slice(b"appl");
        put32(&mut e, 0);
        put32(&mut e, 0x0000_0200);
        put16(&mut e, w as u16);
        put16(&mut e, h as u16);
        put32(&mut e, 72 << 16);
        put32(&mut e, 72 << 16);
        put32(&mut e, 0);
        put16(&mut e, 1);
        let mut name = Vec::new();
        pascal(&mut name, "Photo - JPEG");
        name.resize(32, 0);
        e.extend_from_slice(&name);
        put16(&mut e, 24);
        put16(&mut e, 0xFFFF);
        let entry = atom(b"jpeg", &[&e]);
        let mut b = Vec::new();
        put32(&mut b, 0);
        put32(&mut b, 1);
        atom(b"stsd", &[&b, &entry])
    };
    let stts = {
        let mut b = Vec::new();
        put32(&mut b, 0);
        put32(&mut b, 1);
        put32(&mut b, n);
        put32(&mut b, 100);
        atom(b"stts", &[&b])
    };
    let stsc = {
        let mut b = Vec::new();
        put32(&mut b, 0);
        put32(&mut b, 1);
        put32(&mut b, 1);
        put32(&mut b, n);
        put32(&mut b, 1);
        atom(b"stsc", &[&b])
    };
    let stsz = {
        let mut b = Vec::with_capacity(12 + jpegs.len() * 4);
        put32(&mut b, 0);
        put32(&mut b, 0);
        put32(&mut b, n);
        jpegs.iter().for_each(|j| put32(&mut b, j.len() as u32));
        atom(b"stsz", &[&b])
    };
    let stco = {
        let mut b = Vec::new();
        put32(&mut b, 0);
        put32(&mut b, 1);
        put32(&mut b, chunk_offset);
        atom(b"stco", &[&b])
    };
    let stbl = atom(b"stbl", &[&stsd, &stts, &stsc, &stsz, &stco]);
    let minf = atom(b"minf", &[&vmhd, &handler(b"dhlr", b"alis", "DataHandler"), &dinf, &stbl]);
    let mdia = atom(b"mdia", &[&mdhd, &handler(b"mhlr", b"vide", "VideoHandler"), &minf]);
    let trak = atom(b"trak", &[&tkhd, &mdia]);
    let moov = atom(b"moov", &[&mvhd, &trak]);

    let mut out = Vec::with_capacity(ftyp.len() + 8 + data + moov.len());
    out.extend_from_slice(&ftyp);
    put32(&mut out, (data + 8) as u32);
    out.extend_from_slice(b"mdat");
    jpegs.iter().for_each(|j| out.extend_from_slice(j));
    out.extend_from_slice(&moov);
    Ok(out)
}

fn le32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn le16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn riff_chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(body.len() + 9);
    v.extend_from_slice(id);
    le32(&mut v, body.len() as u32);
    v.extend_from_slice(body);
    if body.len() % 2 == 1 {
        v.push(0);
    }
    v
}

fn riff_list(kind: &[u8; 4], id: &[u8; 4], parts: &[&[u8]]) -> Vec<u8> {
    let len: usize = 4 + parts.iter().map(|p| p.len()).sum::<usize>();
    let mut v = Vec::with_capacity(len + 8);
    v.extend_from_slice(kind);
    le32(&mut v, len as u32);
    v.extend_from_slice(id);
    for p in parts {
        v.extend_from_slice(p);
    }
    v
}

/// A RIFF AVI with one MJPG video stream and an `idx1` index (offsets from `movi`).
pub fn mux_avi(jpegs: &[Vec<u8>], w: u32, h: u32, fps: u32) -> Result<Vec<u8>, String> {
    let n = jpegs.len() as u32;
    let fps = fps.max(1);
    let data: usize = jpegs.iter().map(|j| j.len() + 9).sum();
    if data > u32::MAX as usize - 1024 * 1024 {
        return Err("Recording size limit exceeded".into());
    }
    if w > i16::MAX as u32 || h > i16::MAX as u32 {
        return Err(format!("video size {w}x{h} is too large"));
    }
    let max_frame = jpegs.iter().map(Vec::len).max().unwrap_or(0) as u32;
    let avih = {
        let mut b = Vec::new();
        le32(&mut b, 1_000_000 / fps);
        le32(&mut b, max_frame.saturating_mul(fps));
        le32(&mut b, 0);
        le32(&mut b, 0x10);
        le32(&mut b, n);
        le32(&mut b, 0);
        le32(&mut b, 1);
        le32(&mut b, max_frame + 8);
        le32(&mut b, w);
        le32(&mut b, h);
        b.extend_from_slice(&[0; 16]);
        riff_chunk(b"avih", &b)
    };
    let strh = {
        let mut b = Vec::new();
        b.extend_from_slice(b"vids");
        b.extend_from_slice(b"MJPG");
        le32(&mut b, 0);
        le16(&mut b, 0);
        le16(&mut b, 0);
        le32(&mut b, 0);
        le32(&mut b, 1);
        le32(&mut b, fps);
        le32(&mut b, 0);
        le32(&mut b, n);
        le32(&mut b, max_frame + 8);
        le32(&mut b, u32::MAX);
        le32(&mut b, 0);
        le16(&mut b, 0);
        le16(&mut b, 0);
        le16(&mut b, w as u16);
        le16(&mut b, h as u16);
        riff_chunk(b"strh", &b)
    };
    let strf = {
        let mut b = Vec::new();
        le32(&mut b, 40);
        le32(&mut b, w);
        le32(&mut b, h);
        le16(&mut b, 1);
        le16(&mut b, 24);
        b.extend_from_slice(b"MJPG");
        le32(&mut b, w * h * 3);
        le32(&mut b, 0);
        le32(&mut b, 0);
        le32(&mut b, 0);
        le32(&mut b, 0);
        riff_chunk(b"strf", &b)
    };
    let strl = riff_list(b"LIST", b"strl", &[&strh, &strf]);
    let hdrl = riff_list(b"LIST", b"hdrl", &[&avih, &strl]);

    let mut movi_body = Vec::with_capacity(data);
    let mut idx = Vec::with_capacity(jpegs.len() * 16);
    for j in jpegs {
        idx.extend_from_slice(b"00dc");
        le32(&mut idx, 0x10);
        le32(&mut idx, 4 + movi_body.len() as u32);
        le32(&mut idx, j.len() as u32);
        movi_body.extend_from_slice(&riff_chunk(b"00dc", j));
    }
    let movi = riff_list(b"LIST", b"movi", &[&movi_body]);
    let idx1 = riff_chunk(b"idx1", &idx);
    Ok(riff_list(b"RIFF", b"AVI ", &[&hdrl, &movi, &idx1]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: u32, h: u32, t: u32, alpha: bool) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let r = (x * 255 / w.max(1)) as u8;
                let g = (y * 255 / h.max(1)) as u8;
                let b = ((x + y + t * 7) % 256) as u8;
                let a = if alpha && x < w / 2 { 0 } else { 255 };
                v.extend_from_slice(&[r, g, b, a]);
            }
        }
        v
    }

    fn decode(bytes: &[u8]) -> (u16, u16, Vec<gif::Frame<'static>>, gif::Repeat) {
        let mut opts = gif::DecodeOptions::new();
        opts.set_color_output(gif::ColorOutput::Indexed);
        let mut d = opts.read_info(bytes).expect("gif header");
        let (w, h) = (d.width(), d.height());
        let mut frames = Vec::new();
        while let Some(f) = d.read_next_frame().expect("gif frame") {
            frames.push(f.clone());
        }
        (w, h, frames, d.repeat())
    }

    #[test]
    fn capture_gif_dimensions_cap_the_long_edge() {
        assert_eq!(gif_dimensions(1920, 1080), (480, 270));
        assert_eq!(gif_dimensions(1000, 3000), (160, 480));
        assert_eq!(gif_dimensions(300, 200), (300, 200));
        assert_eq!(gif_dimensions(0, 0), (1, 1));
        assert_eq!(gif_trim_bounds(10, 12..20), 9..10);
        assert_eq!(gif_trim_bounds(10, 3..3), 3..4);
        assert_eq!(GifOptions::default().max_frames(), 48);
        assert_eq!(GifOptions::default().delay_centis(), 8);
    }

    #[test]
    fn capture_gif_round_trips_frames_loop_and_delay() {
        let (w, h) = (40, 30);
        let frames: Vec<Vec<u8>> = (0..6).map(|t| frame(w, h, t, false)).collect();
        let refs: Vec<&[u8]> = frames.iter().map(Vec::as_slice).collect();
        let bytes = encode_gif(&refs, w, h, &GifOptions::default()).unwrap();
        let (dw, dh, decoded, repeat) = decode(&bytes);
        assert_eq!((dw, dh), (40, 30));
        assert_eq!(decoded.len(), 6);
        assert_eq!(repeat, gif::Repeat::Infinite);
        for (f, src) in decoded.iter().zip(&frames) {
            assert_eq!(f.delay, 8);
            assert_eq!(f.dispose, gif::DisposalMethod::Background);
            assert_eq!(f.transparent, None);
            let pal = f.palette.as_ref().expect("local palette");
            assert!(pal.len() <= 256 * 3);
            // The palette tracks the image: mean error stays small.
            let mut err = 0u64;
            for (i, &ix) in f.buffer.iter().enumerate() {
                for c in 0..3 {
                    err += (pal[ix as usize * 3 + c] as i64 - src[i * 4 + c] as i64).unsigned_abs();
                }
            }
            let mean = err as f64 / (f.buffer.len() * 3) as f64;
            assert!(mean < 8.0, "mean error {mean}");
        }
    }

    #[test]
    fn capture_gif_keeps_index_zero_transparent() {
        let (w, h) = (32, 16);
        let f = frame(w, h, 0, true);
        let bytes = encode_gif(&[&f], w, h, &GifOptions::default()).unwrap();
        let (_, _, decoded, _) = decode(&bytes);
        let d = &decoded[0];
        assert_eq!(d.transparent, Some(0));
        assert!(d.palette.as_ref().unwrap().len() <= 256 * 3);
        for (i, &ix) in d.buffer.iter().enumerate() {
            if f[i * 4 + 3] < 128 {
                assert_eq!(ix, 0);
            } else {
                assert_ne!(ix, 0);
            }
        }
        // Fully transparent frame: just the transparent entry and black.
        let clear = vec![0u8; (w * h * 4) as usize];
        let bytes = encode_gif(&[&clear], w, h, &GifOptions::default()).unwrap();
        let (_, _, decoded, _) = decode(&bytes);
        assert_eq!(decoded[0].transparent, Some(0));
        assert!(decoded[0].buffer.iter().all(|&i| i == 0));
    }

    #[test]
    fn capture_gif_palette_stays_within_256_on_busy_frames() {
        let (w, h) = (64, 64);
        let mut f = Vec::new();
        let mut s = 12345u32;
        for _ in 0..w * h {
            s = s.wrapping_mul(1_103_515_245).wrapping_add(12345);
            f.extend_from_slice(&[(s >> 16) as u8, (s >> 8) as u8, (s >> 24) as u8, 255]);
        }
        let bytes = encode_gif(&[&f], w, h, &GifOptions::default()).unwrap();
        let (_, _, decoded, _) = decode(&bytes);
        let pal = decoded[0].palette.as_ref().unwrap();
        assert!(pal.len() <= 256 * 3);
        assert!(pal.len() >= 200 * 3, "palette uses {} colors", pal.len() / 3);
    }

    #[test]
    fn capture_gif_errors_past_max_bytes() {
        let f = frame(64, 64, 0, false);
        let opts = GifOptions { max_bytes: 100, ..GifOptions::default() };
        assert_eq!(encode_gif(&[&f, &f], 64, 64, &opts).unwrap_err(), "Recording size limit exceeded");
        assert!(encode_gif(&[], 64, 64, &opts).is_err());
    }

    #[test]
    fn capture_gif_recorder_downscales_caps_and_trims() {
        // A small edge keeps the debug-build test quick; the default 480 cap is covered above.
        let opts = GifOptions { max_edge: 96, ..GifOptions::default() };
        let mut rec = GifRecorder::new(opts);
        let big = frame(320, 180, 0, false);
        for _ in 0..60 {
            rec.push(&big, 320, 180).unwrap();
        }
        assert_eq!(rec.len(), 48);
        assert!(rec.is_full());
        assert!(!rec.push(&big, 320, 180).unwrap());
        assert_eq!(rec.size(), (96, 54));
        let (px, fw, fh) = rec.frame(0).unwrap();
        assert_eq!((px.len(), fw, fh), (96 * 54 * 4, 96, 54));
        let bytes = rec.finish(10..20).unwrap();
        let (dw, dh, decoded, _) = decode(&bytes);
        assert_eq!((dw, dh, decoded.len()), (96, 54, 10));
        // Out-of-range trims clamp to the last frame instead of failing.
        let (_, _, decoded, _) = decode(&rec.finish(100..200).unwrap());
        assert_eq!(decoded.len(), 1);
        rec.clear();
        assert!(rec.finish(0..1).is_err());
    }

    #[test]
    fn capture_downscale_averages_with_alpha() {
        // 2x1 to 1x1: a transparent pixel does not darken the opaque one.
        let px = [255, 0, 0, 255, 0, 0, 0, 0];
        assert_eq!(downscale(&px, 2, 1, 1, 1), vec![255, 0, 0, 128]);
        let px = [10, 20, 30, 255, 30, 40, 50, 255];
        assert_eq!(downscale(&px, 2, 1, 1, 1), vec![20, 30, 40, 255]);
    }

    fn be32(b: &[u8], at: usize) -> u32 {
        u32::from_be_bytes(b[at..at + 4].try_into().unwrap())
    }

    /// Walks a QuickTime box tree, collecting (path, body range) for every atom.
    fn walk(b: &[u8], start: usize, end: usize, path: &str, out: &mut Vec<(String, Range<usize>)>) {
        let mut at = start;
        while at + 8 <= end {
            let size = be32(b, at) as usize;
            assert!(size >= 8 && at + size <= end, "bad atom size at {at}");
            let kind = std::str::from_utf8(&b[at + 4..at + 8]).unwrap().to_string();
            let p = format!("{path}/{kind}");
            out.push((p.clone(), at + 8..at + size));
            if matches!(kind.as_str(), "moov" | "trak" | "mdia" | "minf" | "dinf" | "stbl") {
                walk(b, at + 8, at + size, &p, out);
            }
            at += size;
        }
        assert_eq!(at, end);
    }

    fn video(format: VideoFormat, n: u32) -> Vec<u8> {
        let mut rec = VideoRecorder::new(65, 33, VIDEO_FPS, format);
        assert_eq!(rec.size(), (64, 32));
        for t in 0..n {
            assert!(rec.push(&frame(65, 33, t, true)).unwrap());
        }
        rec.finish().unwrap()
    }

    #[test]
    fn capture_mov_is_a_valid_atom_tree() {
        let bytes = video(VideoFormat::Mov, 5);
        let mut atoms = Vec::new();
        walk(&bytes, 0, bytes.len(), "", &mut atoms);
        let find = |p: &str| atoms.iter().find(|(k, _)| k == p).unwrap_or_else(|| panic!("{p}")).1.clone();
        assert_eq!(&bytes[find("/ftyp").start..find("/ftyp").start + 4], b"qt  ");
        let stsz = find("/moov/trak/mdia/minf/stbl/stsz");
        assert_eq!(be32(&bytes, stsz.start + 8), 5);
        let first = be32(&bytes, stsz.start + 12) as usize;
        let stco = find("/moov/trak/mdia/minf/stbl/stco");
        let off = be32(&bytes, stco.start + 8) as usize;
        assert_eq!(off, find("/mdat").start);
        let jpeg = &bytes[off..off + first];
        assert_eq!(&jpeg[..2], &[0xFF, 0xD8]);
        assert_eq!(&jpeg[first - 2..], &[0xFF, 0xD9]);
        let img = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg).unwrap();
        assert_eq!((img.width(), img.height()), (64, 32));
        let stsd = find("/moov/trak/mdia/minf/stbl/stsd");
        assert_eq!(&bytes[stsd.start + 12..stsd.start + 16], b"jpeg");
        assert_eq!(be32(&bytes, stsd.start + 8), 86);
        let hdlr = find("/moov/trak/mdia/hdlr");
        assert_eq!(&bytes[hdlr.start + 8..hdlr.start + 12], b"vide");
        let tkhd = find("/moov/trak/tkhd");
        assert_eq!(be32(&bytes, tkhd.end - 8), 64 << 16);
        assert_eq!(find("/moov/mvhd").len(), 100);
    }

    #[test]
    fn capture_avi_has_movi_frames_and_index() {
        let bytes = video(VideoFormat::Avi, 7);
        let le = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(le(4) + 8, bytes.len());
        assert_eq!(&bytes[8..12], b"AVI ");
        let mut at = 12;
        let (mut movi, mut idx1) = (None, None);
        while at + 8 <= bytes.len() {
            let size = le(at + 4);
            if &bytes[at..at + 4] == b"LIST" && &bytes[at + 8..at + 12] == b"movi" {
                movi = Some((at + 8, at + 8 + size));
            }
            if &bytes[at..at + 4] == b"idx1" {
                idx1 = Some((at + 8, at + 8 + size));
            }
            at += 8 + size + (size & 1);
        }
        assert_eq!(at, bytes.len());
        let (ms, me) = movi.expect("movi");
        let mut chunks = 0;
        let mut p = ms + 4;
        while p < me {
            assert_eq!(&bytes[p..p + 4], b"00dc");
            assert_eq!(&bytes[p + 8..p + 10], &[0xFF, 0xD8]);
            let size = le(p + 4);
            p += 8 + size + (size & 1);
            chunks += 1;
        }
        assert_eq!(chunks, 7);
        let (is, ie) = idx1.expect("idx1");
        assert_eq!((ie - is) / 16, 7);
        for e in (is..ie).step_by(16) {
            assert_eq!(&bytes[e..e + 4], b"00dc");
            assert_eq!(&bytes[ms + le(e + 8)..ms + le(e + 8) + 4], b"00dc");
            assert_eq!(le(ms + le(e + 8) + 4), le(e + 12));
        }
        // avih frame count, strh handler, strf compression.
        assert!(bytes.windows(4).any(|w| w == b"MJPG"));
    }

    #[test]
    fn capture_video_caps_at_ten_seconds() {
        let mut rec = VideoRecorder::new(4, 4, 3, native_video_format());
        let f = frame(4, 4, 0, false);
        for _ in 0..40 {
            rec.push(&f).unwrap();
        }
        assert_eq!(rec.len(), 30);
        assert!(rec.push(&[0; 3]).is_err());
        assert!(VideoRecorder::new(4, 4, 30, VideoFormat::Avi).finish().is_err());
    }

    /// EBML elements in `b[start..end]`: (id, data range).
    fn ebml_children(b: &[u8], start: usize, end: usize) -> Vec<(u32, Range<usize>)> {
        let vint = |at: usize, keep_marker: bool| -> (u64, usize) {
            let n = b[at].leading_zeros() as usize + 1;
            let mut v = if keep_marker { b[at] as u64 } else { (b[at] as u64) & (0xFF >> n) };
            for i in 1..n {
                v = (v << 8) | b[at + i] as u64;
            }
            (v, n)
        };
        let mut out = Vec::new();
        let mut at = start;
        while at < end {
            let (id, idn) = vint(at, true);
            let (size, sn) = vint(at + idn, false);
            let data = at + idn + sn;
            assert!(data + size as usize <= end, "element {id:x} at {at} overruns");
            out.push((id as u32, data..data + size as usize));
            at = data + size as usize;
        }
        assert_eq!(at, end);
        out
    }

    fn ebml_uint(b: &[u8], r: &Range<usize>) -> u64 {
        b[r.clone()].iter().fold(0, |v, &x| (v << 8) | x as u64)
    }

    #[test]
    fn capture_webm_is_a_valid_element_tree() {
        let mut rec = VideoRecorder::new(65, 33, VIDEO_FPS, VideoFormat::WebM);
        assert_eq!(rec.size(), (64, 32));
        assert_eq!(rec.extension(), "webm");
        for t in 0..95 {
            assert!(rec.push(&vp8::test_scene(65, 33, t)).unwrap());
        }
        assert!(rec.bytes() > 0);
        let bytes = rec.finish().unwrap();
        let top = ebml_children(&bytes, 0, bytes.len());
        assert_eq!(top.len(), 2);
        let header = ebml_children(&bytes, top[0].1.start, top[0].1.end);
        let doc = header.iter().find(|(id, _)| *id == 0x4282).unwrap();
        assert_eq!(&bytes[doc.1.clone()], b"webm");
        assert_eq!(top[1].0, 0x1853_8067);
        let seg = top[1].1.clone();
        let kids = ebml_children(&bytes, seg.start, seg.end);
        let ids: Vec<u32> = kids.iter().map(|k| k.0).collect();
        // SeekHead, Info, Tracks, two clusters (key frames at 0 and 90), Cues.
        assert_eq!(ids, [0x114D_9B74, 0x1549_A966, 0x1654_AE6B, 0x1F43_B675, 0x1F43_B675, 0x1C53_BB6B]);
        // Every SeekHead entry points at the element it names.
        for seek in ebml_children(&bytes, kids[0].1.start, kids[0].1.end) {
            let e = ebml_children(&bytes, seek.1.start, seek.1.end);
            let id = ebml_uint(&bytes, &e[0].1) as u32;
            let pos = ebml_uint(&bytes, &e[1].1) as usize;
            let target = kids.iter().find(|k| k.0 == id).unwrap();
            let idlen = 4;
            let sizelen = target.1.start - (seg.start + pos + idlen);
            assert!((1..=8).contains(&sizelen), "seek to {id:x}");
        }
        let track = ebml_children(&bytes, kids[2].1.start, kids[2].1.end);
        let entry = ebml_children(&bytes, track[0].1.start, track[0].1.end);
        let codec = entry.iter().find(|(id, _)| *id == 0x86).unwrap();
        assert_eq!(&bytes[codec.1.clone()], b"V_VP8");
        let mut blocks = 0;
        let mut keys = Vec::new();
        for c in &kids[3..5] {
            for (id, r) in ebml_children(&bytes, c.1.start, c.1.end) {
                if id == 0xA3 {
                    assert_eq!(bytes[r.start], 0x81);
                    keys.push(bytes[r.start + 3] & 0x80 != 0);
                    blocks += 1;
                }
            }
        }
        assert_eq!(blocks, 95);
        assert!(keys[0] && keys[90]);
        assert_eq!(keys.iter().filter(|&&k| k).count(), 2);
        // Cues point at the clusters.
        let cues = ebml_children(&bytes, kids[5].1.start, kids[5].1.end);
        assert_eq!(cues.len(), 2);
        for (cue, cluster) in cues.iter().zip(&kids[3..5]) {
            let cp = ebml_children(&bytes, cue.1.start, cue.1.end);
            let tp = ebml_children(&bytes, cp[1].1.start, cp[1].1.end);
            let pos = ebml_uint(&bytes, &tp[1].1) as usize;
            assert_eq!(&bytes[seg.start + pos..seg.start + pos + 4], &[0x1F, 0x43, 0xB6, 0x75]);
            assert!(seg.start + pos < cluster.1.start);
        }
        assert_eq!(ebml_uint(&bytes, &ebml_children(&bytes, cues[1].1.start, cues[1].1.end)[0].1), 3000);
        assert!(mux_webm(&[(&[0u8; 4], false)], 4, 4, 30).is_err());
        assert!(mux_webm(&[], 4, 4, 30).is_err());
    }

    /// Encode a moving test scene, save `sigil-vp8-sample.webm`, and when ffmpeg is installed
    /// check that its decode matches the encoder's reconstruction bit for bit on every frame.
    /// `SIGIL_CAPTURE_OUT=dir cargo test --release -p sigil-viewer capture_webm_sample -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn capture_webm_sample() {
        let (w, h, n) = (640u32, 400u32, 150u32);
        let out = std::env::var("SIGIL_CAPTURE_OUT").map(std::path::PathBuf::from).unwrap_or_else(|_| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../.scratch/viewer-shots/leftovers")
        });
        std::fs::create_dir_all(&out).unwrap();
        let mut enc = Vp8Encoder::new(w, h, Vp8Options::default()).unwrap();
        let mut frames = Vec::new();
        let mut recon = Vec::new();
        let t = std::time::Instant::now();
        for i in 0..n {
            let f = enc.encode_rgba(&vp8::test_scene(w, h, i)).unwrap();
            let (y, u, v, mbw) = enc.reconstruction();
            let mut planes = Vec::with_capacity((w * h * 3 / 2) as usize);
            for row in 0..h as usize {
                planes.extend_from_slice(&y[row * mbw * 16..row * mbw * 16 + w as usize]);
            }
            for plane in [u, v] {
                for row in 0..(h as usize).div_ceil(2) {
                    planes.extend_from_slice(&plane[row * mbw * 8..row * mbw * 8 + (w as usize).div_ceil(2)]);
                }
            }
            recon.push(planes);
            frames.push(f);
        }
        let elapsed = t.elapsed();
        let refs: Vec<(&[u8], bool)> = frames.iter().map(|f| (f.data.as_slice(), f.key)).collect();
        let bytes = mux_webm(&refs, w, h, VIDEO_FPS).unwrap();
        let path = out.join("sigil-vp8-sample.webm");
        std::fs::write(&path, &bytes).unwrap();
        let secs = n as f64 / VIDEO_FPS as f64;
        eprintln!(
            "{}: {n} frames {w}x{h}, {} bytes ({:.0} bytes/s), {:?} per frame",
            path.display(),
            bytes.len(),
            bytes.len() as f64 / secs,
            elapsed / n
        );
        let ffmpeg = std::path::Path::new("/opt/homebrew/bin/ffmpeg");
        if !ffmpeg.exists() {
            eprintln!("no ffmpeg: skipping the decode check");
            return;
        }
        let yuv = out.join("sigil-vp8-sample.yuv");
        let status = std::process::Command::new(ffmpeg)
            .args(["-v", "error", "-y", "-i"])
            .arg(&path)
            .args(["-f", "rawvideo", "-pix_fmt", "yuv420p"])
            .arg(&yuv)
            .status()
            .unwrap();
        assert!(status.success());
        let decoded = std::fs::read(&yuv).unwrap();
        std::fs::remove_file(&yuv).ok();
        let size = recon[0].len();
        assert_eq!(decoded.len(), size * n as usize, "decoded frame count");
        for (i, r) in recon.iter().enumerate() {
            let d = &decoded[i * size..(i + 1) * size];
            let diff = r.iter().zip(d).filter(|(a, b)| a != b).count();
            assert_eq!(diff, 0, "frame {i} differs from ffmpeg's decode in {diff} samples");
        }
        eprintln!("ffmpeg decode matches the reconstruction on all {n} frames");
    }

    /// `cargo test --release -p sigil-viewer capture_bench -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn capture_bench() {
        let (w, h) = (480u32, 300u32);
        let frames: Vec<Vec<u8>> = (0..48)
            .map(|t| {
                let mut f = frame(w, h, t, false);
                let mut s = t.wrapping_mul(2_654_435_761);
                for p in f.as_chunks_mut::<4>().0 {
                    s = s.wrapping_mul(1_103_515_245).wrapping_add(12345);
                    p[0] = p[0].wrapping_add((s >> 28) as u8);
                }
                f
            })
            .collect();
        let refs: Vec<&[u8]> = frames.iter().map(Vec::as_slice).collect();
        let t = std::time::Instant::now();
        let bytes = encode_gif(&refs, w, h, &GifOptions::default()).unwrap();
        eprintln!("gif: 48 frames {w}x{h} in {:?}, {} bytes", t.elapsed(), bytes.len());
        let t = std::time::Instant::now();
        let one = quantize_frame(refs[0], w as usize);
        eprintln!("gif: one frame quantize {:?} ({} colors)", t.elapsed(), one.palette.len() / 3);

        let big = frame(1920, 1080, 0, false);
        let t = std::time::Instant::now();
        let small = downscale(&big, 1920, 1080, 480, 270);
        eprintln!("downscale 1920x1080 to 480x270 in {:?} ({})", t.elapsed(), small.len());
        for (vw, vh) in [(1280u32, 720u32), (1920, 1080)] {
            let f = frame(vw, vh, 0, false);
            let mut rec = VideoRecorder::new(vw, vh, VIDEO_FPS, VideoFormat::Mov);
            let t = std::time::Instant::now();
            for _ in 0..30 {
                rec.push(&f).unwrap();
            }
            let per = t.elapsed() / 30;
            let mov = rec.finish().unwrap();
            eprintln!("video: {vw}x{vh} jpeg {per:?} per frame, 30 frames {} bytes", mov.len());
        }
        for (vw, vh) in [(640u32, 400u32), (1280, 800), (1920, 1080)] {
            let scenes: Vec<Vec<u8>> = (0..90).map(|t| vp8::test_scene(vw, vh, t)).collect();
            for search in [true, false] {
                let opts = Vp8Options { motion_search: search, ..Vp8Options::default() };
                let mut enc = Vp8Encoder::new(vw, vh, opts).unwrap();
                let (mut key, mut inter, mut bytes) = (Duration::ZERO, Duration::ZERO, 0usize);
                for s in &scenes {
                    let t = std::time::Instant::now();
                    let f = enc.encode_rgba(s).unwrap();
                    if f.key { key += t.elapsed() } else { inter += t.elapsed() }
                    bytes += f.data.len();
                }
                eprintln!(
                    "video: {vw}x{vh} vp8 (search {search}) key frame {key:?}, inter frame {:?} average, {:.0} bytes/s",
                    inter / 89,
                    bytes as f64 / 3.0
                );
            }
        }
    }
}
