//! A small VP8 encoder (RFC 6386) for the viewer's WebM recordings.
//!
//! Original code written from the format description in RFC 6386; only the normative tables in
//! `vp8_tables.rs` come from the RFC. It codes a subset of the format that every VP8 decoder
//! reads:
//!
//! - key frames with whole-macroblock intra prediction (DC, V, H or TM for luma and chroma,
//!   picked by the smallest sum of absolute differences), so the Y2 block is always present;
//! - inter frames that predict each macroblock from the last frame with a motion vector in
//!   whole pixels (zero, nearest, near or a new vector from a small diamond search, kept on even
//!   pixels so chroma needs no subpixel filter either), falling back to intra prediction;
//! - skipped macroblocks, per-frame coefficient probability updates, one token partition.
//!
//! Left out on purpose: B_PRED, SPLITMV, golden and altref frames, segmentation and the loop
//! filter (level 0, so the reconstruction is prediction plus residue and the encoder keeps an
//! exact copy of what the decoder sees).
//!
//! Input is sRGB RGBA8 (rows top first, tightly packed), composited onto black and converted to
//! I420 with BT.601 limited range, which is what VP8's `color_space` 0 means.

#![allow(clippy::needless_range_loop)]

use super::vp8_tables::{
    AC_Q, COEFF_BANDS, COEFF_UPDATE_PROBS, DC_Q, DEFAULT_COEFF_PROBS, DEFAULT_MV_PROBS, MODE_CONTEXTS,
    MV_UPDATE_PROBS, PCAT3, PCAT4, PCAT5, PCAT6, ZIGZAG,
};

/// Quantizer index for recordings: near transparent at 480p to 720p.
pub const DEFAULT_Q_INDEX: u8 = 16;
/// A key frame every three seconds at 30 fps, so players can seek.
pub const DEFAULT_KEYFRAME_INTERVAL: u32 = 90;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Vp8Options {
    /// Quantizer index, 0 (finest) to 127 (coarsest).
    pub q_index: u8,
    /// A key frame every this many frames; 0 for only the first.
    pub keyframe_interval: u32,
    /// Search for motion vectors; off codes every inter macroblock with a zero vector.
    pub motion_search: bool,
}

impl Default for Vp8Options {
    fn default() -> Self {
        Vp8Options { q_index: DEFAULT_Q_INDEX, keyframe_interval: DEFAULT_KEYFRAME_INTERVAL, motion_search: true }
    }
}

/// One compressed frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vp8Frame {
    pub data: Vec<u8>,
    pub key: bool,
}

type Probs = [[[[u8; 11]; 3]; 8]; 4];
type Counts = [[[[[u32; 2]; 11]; 3]; 8]; 4];

const DC_PRED: u8 = 0;
const V_PRED: u8 = 1;
const H_PRED: u8 = 2;
const TM_PRED: u8 = 3;

/// The boolean entropy coder of RFC 6386 section 7, with byte-wise output and carry
/// propagation into bytes already written.
#[derive(Debug, Clone)]
struct BoolEncoder {
    buf: Vec<u8>,
    low: u32,
    range: u32,
    count: i32,
}

impl BoolEncoder {
    fn new() -> Self {
        BoolEncoder { buf: Vec::new(), low: 0, range: 255, count: -24 }
    }

    /// Write `bit`, where `prob / 256` is the chance of a zero.
    #[inline]
    fn put(&mut self, bit: bool, prob: u8) {
        let split = 1 + (((self.range - 1) * prob as u32) >> 8);
        if bit {
            self.low = self.low.wrapping_add(split);
            self.range -= split;
        } else {
            self.range = split;
        }
        let mut shift = self.range.leading_zeros() as i32 - 24;
        self.range <<= shift;
        self.count += shift;
        if self.count >= 0 {
            let offset = shift - self.count;
            if (self.low << (offset - 1)) & 0x8000_0000 != 0 {
                for b in self.buf.iter_mut().rev() {
                    if *b == 0xff {
                        *b = 0;
                    } else {
                        *b += 1;
                        break;
                    }
                }
            }
            self.buf.push((self.low >> (24 - offset)) as u8);
            self.low <<= offset;
            shift = self.count;
            self.low &= 0xff_ffff;
            self.count -= 8;
        }
        self.low <<= shift;
    }

    /// An unsigned `bits`-wide value, high bit first, each bit at even odds.
    fn literal(&mut self, value: u32, bits: u32) {
        for i in (0..bits).rev() {
            self.put((value >> i) & 1 == 1, 128);
        }
    }

    fn finish(mut self) -> Vec<u8> {
        for _ in 0..32 {
            self.put(false, 128);
        }
        self.buf
    }
}

/// Bits to code `bit` at `prob` (the chance of a zero, out of 256).
fn bit_cost(bit: bool, prob: u8) -> f64 {
    let p = if bit { 256 - prob as u32 } else { prob as u32 } as f64 / 256.0;
    -p.log2()
}

/// `prob` for a bool that was zero `zeros` times out of `total`.
fn prob_of_zero(zeros: usize, total: usize) -> u8 {
    if total == 0 {
        return 128;
    }
    ((zeros * 256 + total / 2) / total).clamp(1, 255) as u8
}

/// The forward 4x4 DCT that pairs with the decoder's inverse (section 14.4): `out[0]` is eight
/// times the mean, so the inverse's `(x + 4) >> 3` returns the residue.
fn fdct(input: &[i32; 16], out: &mut [i32; 16]) {
    let mut tmp = [0i32; 16];
    for r in 0..4 {
        let i = &input[r * 4..r * 4 + 4];
        let a = (i[0] + i[3]) * 8;
        let b = (i[1] + i[2]) * 8;
        let c = (i[1] - i[2]) * 8;
        let d = (i[0] - i[3]) * 8;
        tmp[r * 4] = a + b;
        tmp[r * 4 + 2] = a - b;
        tmp[r * 4 + 1] = (c * 2217 + d * 5352 + 14500) >> 12;
        tmp[r * 4 + 3] = (d * 2217 - c * 5352 + 7500) >> 12;
    }
    for c in 0..4 {
        let a = tmp[c] + tmp[12 + c];
        let b = tmp[4 + c] + tmp[8 + c];
        let cc = tmp[4 + c] - tmp[8 + c];
        let d = tmp[c] - tmp[12 + c];
        out[c] = (a + b + 7) >> 4;
        out[8 + c] = (a - b + 7) >> 4;
        out[4 + c] = ((cc * 2217 + d * 5352 + 12000) >> 16) + (d != 0) as i32;
        out[12 + c] = (d * 2217 - cc * 5352 + 51000) >> 16;
    }
}

/// The decoder's inverse DCT (section 14.4), bit for bit, 16-bit intermediates included.
fn idct(input: &[i32; 16], out: &mut [i32; 16]) {
    const C: i32 = 20091;
    const S: i32 = 35468;
    let mut tmp = [0i16; 16];
    for i in 0..4 {
        let (i0, i4, i8, i12) = (input[i], input[4 + i], input[8 + i], input[12 + i]);
        let a = i0 + i8;
        let b = i0 - i8;
        let c = ((i4 * S) >> 16) - (i12 + ((i12 * C) >> 16));
        let d = (i4 + ((i4 * C) >> 16)) + ((i12 * S) >> 16);
        tmp[i] = (a + d) as i16;
        tmp[12 + i] = (a - d) as i16;
        tmp[4 + i] = (b + c) as i16;
        tmp[8 + i] = (b - c) as i16;
    }
    for r in 0..4 {
        let t = &tmp[r * 4..r * 4 + 4];
        let (t0, t1, t2, t3) = (t[0] as i32, t[1] as i32, t[2] as i32, t[3] as i32);
        let a = t0 + t2;
        let b = t0 - t2;
        let c = ((t1 * S) >> 16) - (t3 + ((t3 * C) >> 16));
        let d = (t1 + ((t1 * C) >> 16)) + ((t3 * S) >> 16);
        out[r * 4] = ((a + d + 4) >> 3) as i16 as i32;
        out[r * 4 + 3] = ((a - d + 4) >> 3) as i16 as i32;
        out[r * 4 + 1] = ((b + c + 4) >> 3) as i16 as i32;
        out[r * 4 + 2] = ((b - c + 4) >> 3) as i16 as i32;
    }
}

/// The forward Walsh-Hadamard transform of the sixteen luma DCs, the inverse of `iwht`.
fn fwht(input: &[i32; 16], out: &mut [i32; 16]) {
    let mut tmp = [0i32; 16];
    for r in 0..4 {
        let i = &input[r * 4..r * 4 + 4];
        let a = (i[0] + i[2]) * 4;
        let d = (i[1] + i[3]) * 4;
        let c = (i[1] - i[3]) * 4;
        let b = (i[0] - i[2]) * 4;
        tmp[r * 4] = a + d + (a != 0) as i32;
        tmp[r * 4 + 1] = b + c;
        tmp[r * 4 + 2] = b - c;
        tmp[r * 4 + 3] = a - d;
    }
    for c in 0..4 {
        let a = tmp[c] + tmp[8 + c];
        let d = tmp[4 + c] + tmp[12 + c];
        let cc = tmp[4 + c] - tmp[12 + c];
        let b = tmp[c] - tmp[8 + c];
        let v = [a + d, b + cc, b - cc, a - d];
        for (k, &x) in v.iter().enumerate() {
            out[k * 4 + c] = (x + (x < 0) as i32 + 3) >> 3;
        }
    }
}

/// The decoder's inverse WHT (section 14.3), bit for bit.
fn iwht(input: &[i32; 16], out: &mut [i32; 16]) {
    let mut tmp = [0i16; 16];
    for i in 0..4 {
        let a = input[i] + input[12 + i];
        let b = input[4 + i] + input[8 + i];
        let c = input[4 + i] - input[8 + i];
        let d = input[i] - input[12 + i];
        tmp[i] = (a + b) as i16;
        tmp[4 + i] = (c + d) as i16;
        tmp[8 + i] = (a - b) as i16;
        tmp[12 + i] = (d - c) as i16;
    }
    for r in 0..4 {
        let t = &tmp[r * 4..r * 4 + 4];
        let a = t[0] as i32 + t[3] as i32;
        let b = t[1] as i32 + t[2] as i32;
        let c = t[1] as i32 - t[2] as i32;
        let d = t[0] as i32 - t[3] as i32;
        out[r * 4] = ((a + b + 3) >> 3) as i16 as i32;
        out[r * 4 + 1] = ((c + d + 3) >> 3) as i16 as i32;
        out[r * 4 + 2] = ((a - b + 3) >> 3) as i16 as i32;
        out[r * 4 + 3] = ((d - c + 3) >> 3) as i16 as i32;
    }
}

/// Quantize with a rounding offset in 1/16 steps, clamped to the token range.
#[inline]
fn quantize(c: i32, step: i32, round16: i32) -> i16 {
    let level = ((c.abs() * 16 + step * round16) / (step * 16)).min(2048);
    (if c < 0 { -level } else { level }) as i16
}

/// The six dequantization factors (DC, AC) of a quantizer index, as `dequant_init` derives them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Quant {
    y1: [i32; 2],
    y2: [i32; 2],
    uv: [i32; 2],
}

impl Quant {
    fn new(q: u8) -> Self {
        let q = q.min(127) as usize;
        let (dc, ac) = (DC_Q[q] as i32, AC_Q[q] as i32);
        Quant { y1: [dc, ac], y2: [dc * 2, (ac * 155 / 100).max(8)], uv: [dc.min(132), ac] }
    }
}

/// An I420 frame padded to whole macroblocks.
#[derive(Debug, Clone)]
struct Planes {
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
}

impl Planes {
    fn new(mbw: usize, mbh: usize) -> Self {
        Planes { y: vec![0; mbw * mbh * 256], u: vec![0; mbw * mbh * 64], v: vec![0; mbw * mbh * 64] }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MvMode {
    Zero,
    Nearest,
    Near,
    New,
}

/// What the encoder chose for one macroblock. Motion vectors are row, column in eighth luma
/// pixels (twice the coded value), as decoders keep them.
#[derive(Debug, Clone, Copy)]
struct MbInfo {
    inter: bool,
    mv: [i32; 2],
    mv_mode: MvMode,
    /// The coded NEWMV offset from the best reference vector, in quarter pixels.
    mv_delta: [i32; 2],
    /// The inter mode tree probabilities from the neighbour census.
    mode_probs: [u8; 4],
    ymode: u8,
    uvmode: u8,
    skip: bool,
}

impl Default for MbInfo {
    fn default() -> Self {
        MbInfo {
            inter: false,
            mv: [0; 2],
            mv_mode: MvMode::Zero,
            mv_delta: [0; 2],
            mode_probs: [128; 4],
            ymode: DC_PRED,
            uvmode: DC_PRED,
            skip: false,
        }
    }
}

/// Where a token tree decision goes: counted for probability updates or written.
trait Sink {
    fn node(&mut self, t: usize, band: usize, ctx: usize, node: usize, bit: bool);
    fn raw(&mut self, bit: bool, prob: u8);
}

struct Count<'a>(&'a mut Counts);

impl Sink for Count<'_> {
    #[inline]
    fn node(&mut self, t: usize, band: usize, ctx: usize, node: usize, bit: bool) {
        self.0[t][band][ctx][node][bit as usize] += 1;
    }

    #[inline]
    fn raw(&mut self, _bit: bool, _prob: u8) {}
}

struct Put<'a> {
    e: &'a mut BoolEncoder,
    p: &'a Probs,
}

impl Sink for Put<'_> {
    #[inline]
    fn node(&mut self, t: usize, band: usize, ctx: usize, node: usize, bit: bool) {
        self.e.put(bit, self.p[t][band][ctx][node]);
    }

    #[inline]
    fn raw(&mut self, bit: bool, prob: u8) {
        self.e.put(bit, prob);
    }
}

/// Code one block's coefficients (raster order) of plane type `t` from position `first`, with
/// the above plus left context `ctx`. Returns whether any coefficient was coded (the next
/// blocks' context).
fn put_block<S: Sink>(s: &mut S, t: usize, ctx: usize, coef: &[i16], first: usize) -> bool {
    let mut last = None;
    for k in (first..16).rev() {
        if coef[ZIGZAG[k]] != 0 {
            last = Some(k);
            break;
        }
    }
    let Some(last) = last else {
        s.node(t, COEFF_BANDS[first], ctx, 0, false);
        return false;
    };
    let mut ctx = ctx;
    let mut after_zero = false;
    for k in first..=last {
        let band = COEFF_BANDS[k];
        if !after_zero {
            s.node(t, band, ctx, 0, true);
        }
        let v = coef[ZIGZAG[k]] as i32;
        let a = v.unsigned_abs();
        if a == 0 {
            s.node(t, band, ctx, 1, false);
            ctx = 0;
            after_zero = true;
            continue;
        }
        s.node(t, band, ctx, 1, true);
        if a == 1 {
            s.node(t, band, ctx, 2, false);
        } else {
            s.node(t, band, ctx, 2, true);
            if a <= 4 {
                s.node(t, band, ctx, 3, false);
                if a == 2 {
                    s.node(t, band, ctx, 4, false);
                } else {
                    s.node(t, band, ctx, 4, true);
                    s.node(t, band, ctx, 5, a == 4);
                }
            } else {
                s.node(t, band, ctx, 3, true);
                if a <= 10 {
                    s.node(t, band, ctx, 6, false);
                    if a <= 6 {
                        s.node(t, band, ctx, 7, false);
                        s.raw(a == 6, 159);
                    } else {
                        s.node(t, band, ctx, 7, true);
                        let e = a - 7;
                        s.raw(e & 2 != 0, 165);
                        s.raw(e & 1 != 0, 145);
                    }
                } else {
                    s.node(t, band, ctx, 6, true);
                    let (base, probs): (u32, &[u8]) = if a <= 34 {
                        s.node(t, band, ctx, 8, false);
                        if a <= 18 {
                            s.node(t, band, ctx, 9, false);
                            (11, &PCAT3)
                        } else {
                            s.node(t, band, ctx, 9, true);
                            (19, &PCAT4)
                        }
                    } else {
                        s.node(t, band, ctx, 8, true);
                        if a <= 66 {
                            s.node(t, band, ctx, 10, false);
                            (35, &PCAT5)
                        } else {
                            s.node(t, band, ctx, 10, true);
                            (67, &PCAT6)
                        }
                    };
                    let e = a - base;
                    let n = probs.len();
                    for (i, &p) in probs.iter().enumerate() {
                        s.raw((e >> (n - 1 - i)) & 1 == 1, p);
                    }
                }
            }
        }
        s.raw(v < 0, 128);
        ctx = if a == 1 { 1 } else { 2 };
        after_zero = false;
    }
    if last < 15 {
        s.node(t, COEFF_BANDS[last + 1], ctx, 0, false);
    }
    true
}

/// One motion vector component (quarter pixels, at most 1023 either way), section 17.
fn put_mv_component(e: &mut BoolEncoder, v: i32, p: &[u8; 19]) {
    let a = v.unsigned_abs();
    if a < 8 {
        e.put(false, p[0]);
        let (b2, b1, b0) = (a & 4 != 0, a & 2 != 0, a & 1 != 0);
        e.put(b2, p[2]);
        if b2 {
            e.put(b1, p[6]);
            e.put(b0, p[7 + b1 as usize]);
        } else {
            e.put(b1, p[3]);
            e.put(b0, p[4 + b1 as usize]);
        }
    } else {
        e.put(true, p[0]);
        for i in 0..3 {
            e.put((a >> i) & 1 == 1, p[9 + i]);
        }
        for i in (4..10).rev() {
            e.put((a >> i) & 1 == 1, p[9 + i]);
        }
        if a > 15 {
            e.put((a >> 3) & 1 == 1, p[9 + 3]);
        }
    }
    if a != 0 {
        e.put(v < 0, p[1]);
    }
}

/// Rough bits of a coded motion vector component, for the search's rate term.
fn mv_bits(v: i32) -> u32 {
    let a = v.unsigned_abs();
    if a < 8 { 4 } else { 6 + 2 * (32 - a.leading_zeros()) }
}

fn sad(a: &[u8], astride: usize, b: &[u8], bstride: usize, n: usize) -> u32 {
    let mut s = 0u32;
    for r in 0..n {
        let ra = &a[r * astride..r * astride + n];
        let rb = &b[r * bstride..r * bstride + n];
        s += ra.iter().zip(rb).map(|(&x, &y)| x.abs_diff(y) as u32).sum::<u32>();
    }
    s
}

/// The intra prediction edges of an `n` x `n` block at (`x`, `y`) of `plane`: the row above,
/// the column to the left and the corner, with the out-of-frame values decoders use (127 above
/// the frame, 129 left of it).
fn edges(plane: &[u8], stride: usize, x: usize, y: usize, n: usize) -> ([u8; 16], [u8; 16], u8) {
    let mut above = [127u8; 16];
    let mut left = [129u8; 16];
    if y > 0 {
        above[..n].copy_from_slice(&plane[(y - 1) * stride + x..(y - 1) * stride + x + n]);
    }
    if x > 0 {
        for i in 0..n {
            left[i] = plane[(y + i) * stride + x - 1];
        }
    }
    let corner = if y == 0 {
        127
    } else if x == 0 {
        129
    } else {
        plane[(y - 1) * stride + x - 1]
    };
    (above, left, corner)
}

/// Whole-block intra prediction (section 12) into `out` (`n` x `n`, stride `n`).
#[allow(clippy::too_many_arguments)]
fn predict(mode: u8, above: &[u8; 16], left: &[u8; 16], corner: u8, has_above: bool, has_left: bool, n: usize, out: &mut [u8]) {
    match mode {
        DC_PRED => {
            let shift = n.trailing_zeros();
            let sa: u32 = above[..n].iter().map(|&v| v as u32).sum();
            let sl: u32 = left[..n].iter().map(|&v| v as u32).sum();
            let dc = match (has_above, has_left) {
                (true, true) => (sa + sl + n as u32) >> (shift + 1),
                (true, false) => (sa + (n as u32 >> 1)) >> shift,
                (false, true) => (sl + (n as u32 >> 1)) >> shift,
                (false, false) => 128,
            } as u8;
            out[..n * n].fill(dc);
        }
        V_PRED => {
            for r in 0..n {
                out[r * n..r * n + n].copy_from_slice(&above[..n]);
            }
        }
        H_PRED => {
            for r in 0..n {
                out[r * n..r * n + n].fill(left[r]);
            }
        }
        _ => {
            for r in 0..n {
                let base = left[r] as i32 - corner as i32;
                for c in 0..n {
                    out[r * n + c] = (base + above[c] as i32).clamp(0, 255) as u8;
                }
            }
        }
    }
}

/// Encodes a sequence of same-size frames: the first is a key frame, later ones are inter frames
/// predicted from the one before, with a key frame every `keyframe_interval` frames.
#[derive(Debug, Clone)]
pub struct Vp8Encoder {
    width: u32,
    height: u32,
    mbw: usize,
    mbh: usize,
    opts: Vp8Options,
    quant: Quant,
    frame: u64,
    src: Planes,
    rec: Planes,
    prev: Planes,
    probs: Box<Probs>,
    mbs: Vec<MbInfo>,
    coeffs: Vec<[i16; 400]>,
}

impl Vp8Encoder {
    /// An encoder for `width` x `height` frames (1 to 16383 each way).
    pub fn new(width: u32, height: u32, opts: Vp8Options) -> Result<Self, String> {
        if width == 0 || height == 0 || width > 0x3fff || height > 0x3fff {
            return Err(format!("Invalid VP8 size {width}x{height}"));
        }
        let mbw = width.div_ceil(16) as usize;
        let mbh = height.div_ceil(16) as usize;
        Ok(Vp8Encoder {
            width,
            height,
            mbw,
            mbh,
            opts,
            quant: Quant::new(opts.q_index),
            frame: 0,
            src: Planes::new(mbw, mbh),
            rec: Planes::new(mbw, mbh),
            prev: Planes::new(mbw, mbh),
            probs: Box::new(DEFAULT_COEFF_PROBS),
            mbs: vec![MbInfo::default(); mbw * mbh],
            coeffs: vec![[0; 400]; mbw * mbh],
        })
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn options(&self) -> &Vp8Options {
        &self.opts
    }

    /// Frames encoded so far.
    pub fn frames(&self) -> u64 {
        self.frame
    }

    /// Encode one `width` x `height` RGBA8 frame, alpha composited onto black.
    pub fn encode_rgba(&mut self, rgba: &[u8]) -> Result<Vp8Frame, String> {
        let (w, h) = (self.width as usize, self.height as usize);
        if rgba.len() < w * h * 4 {
            return Err(format!("frame buffer does not hold {w}x{h} RGBA"));
        }
        self.load_rgba(rgba);
        let interval = self.opts.keyframe_interval as u64;
        let key = self.frame == 0 || (interval > 0 && self.frame.is_multiple_of(interval));
        let data = self.encode_frame(key)?;
        std::mem::swap(&mut self.rec, &mut self.prev);
        self.frame += 1;
        Ok(Vp8Frame { data, key })
    }

    /// The last frame as a decoder reconstructs it: Y, U and V planes padded to whole
    /// macroblocks (strides `16 * mb columns` and `8 * mb columns`).
    #[cfg(test)]
    pub(crate) fn reconstruction(&self) -> (&[u8], &[u8], &[u8], usize) {
        (&self.prev.y, &self.prev.u, &self.prev.v, self.mbw)
    }

    /// RGBA to padded I420, BT.601 limited range, chroma averaged over 2x2 pixels.
    fn load_rgba(&mut self, rgba: &[u8]) {
        let (w, h) = (self.width as usize, self.height as usize);
        let ys = self.mbw * 16;
        let cs = self.mbw * 8;
        let premul = |p: &[u8]| -> [i32; 3] {
            let a = p[3] as i32;
            if a == 255 {
                [p[0] as i32, p[1] as i32, p[2] as i32]
            } else {
                [(p[0] as i32 * a + 127) / 255, (p[1] as i32 * a + 127) / 255, (p[2] as i32 * a + 127) / 255]
            }
        };
        for y in 0..h {
            let row = &rgba[y * w * 4..(y + 1) * w * 4];
            let dst = &mut self.src.y[y * ys..(y + 1) * ys];
            for (d, p) in dst.iter_mut().zip(row.as_chunks::<4>().0) {
                let [r, g, b] = premul(p);
                *d = (((66 * r + 129 * g + 25 * b + 128) >> 8) + 16) as u8;
            }
            let last = dst[w - 1];
            dst[w..].fill(last);
        }
        for y in h..self.mbh * 16 {
            self.src.y.copy_within((h - 1) * ys..h * ys, y * ys);
        }
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        for cy in 0..ch {
            let (y0, y1) = (cy * 2, (cy * 2 + 1).min(h - 1));
            for cx in 0..cw {
                let (x0, x1) = (cx * 2, (cx * 2 + 1).min(w - 1));
                let mut s = [0i32; 3];
                for (x, y) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
                    let p = premul(&rgba[(y * w + x) * 4..(y * w + x) * 4 + 4]);
                    for c in 0..3 {
                        s[c] += p[c];
                    }
                }
                let u = ((-38 * s[0] - 74 * s[1] + 112 * s[2] + 512) >> 10) + 128;
                let v = ((112 * s[0] - 94 * s[1] - 18 * s[2] + 512) >> 10) + 128;
                self.src.u[cy * cs + cx] = u.clamp(0, 255) as u8;
                self.src.v[cy * cs + cx] = v.clamp(0, 255) as u8;
            }
            for plane in [&mut self.src.u, &mut self.src.v] {
                let last = plane[cy * cs + cw - 1];
                plane[cy * cs + cw..(cy + 1) * cs].fill(last);
            }
        }
        for y in ch..self.mbh * 8 {
            self.src.u.copy_within((ch - 1) * cs..ch * cs, y * cs);
            self.src.v.copy_within((ch - 1) * cs..ch * cs, y * cs);
        }
    }

    /// The neighbour census of `vp8_find_near_mvs` (section 16.3): the best, nearest and near
    /// vectors (index 0, 1, 2) and the counts that pick the mode probabilities.
    fn near_mvs(&self, mbx: usize, mby: usize) -> ([[i32; 2]; 4], [usize; 4]) {
        let inter_at = |x: usize, y: usize| -> Option<&MbInfo> {
            let m = &self.mbs[y * self.mbw + x];
            m.inter.then_some(m)
        };
        let above = if mby > 0 { inter_at(mbx, mby - 1) } else { None };
        let left = if mbx > 0 { inter_at(mbx - 1, mby) } else { None };
        let above_left = if mbx > 0 && mby > 0 { inter_at(mbx - 1, mby - 1) } else { None };
        let mut near = [[0i32; 2]; 4];
        let mut cnt = [0usize; 4];
        let mut i = 0;
        if let Some(m) = above {
            if m.mv != [0, 0] {
                i += 1;
                near[i] = m.mv;
            }
            cnt[i] += 2;
        }
        for (m, weight) in [(left, 2), (above_left, 1)] {
            let Some(m) = m else { continue };
            if m.mv != [0, 0] {
                if m.mv != near[i] {
                    i += 1;
                    near[i] = m.mv;
                }
                cnt[i] += weight;
            } else {
                cnt[0] += weight;
            }
        }
        if cnt[3] > 0 && near[i] == near[1] {
            cnt[1] += 1;
        }
        // No SPLITMV here, so no neighbour adds to the split count.
        cnt[3] = 0;
        if cnt[2] > cnt[1] {
            cnt.swap(1, 2);
            near.swap(1, 2);
        }
        if cnt[1] >= cnt[0] {
            near[0] = near[1];
        }
        (near, cnt)
    }

    /// `vp8_clamp_mv`: at most one macroblock past the frame edges.
    fn clamp_mv(&self, mv: [i32; 2], mbx: usize, mby: usize) -> [i32; 2] {
        let (mbx, mby) = (mbx as i32, mby as i32);
        let top = -((mby + 1) << 7);
        let bottom = (self.mbh as i32 - mby) << 7;
        let left = -((mbx + 1) << 7);
        let right = (self.mbw as i32 - mbx) << 7;
        [mv[0].clamp(top, bottom), mv[1].clamp(left, right)]
    }

    /// Whether the 16x16 block moved by `mv` stays inside the padded frame, so prediction is a
    /// plain copy with no edge extension.
    fn mv_inside(&self, mv: [i32; 2], mbx: usize, mby: usize) -> bool {
        if mv[0] % 16 != 0 || mv[1] % 16 != 0 {
            return false;
        }
        let x = (mbx * 16) as i32 + mv[1] / 8;
        let y = (mby * 16) as i32 + mv[0] / 8;
        x >= 0 && y >= 0 && x + 16 <= (self.mbw * 16) as i32 && y + 16 <= (self.mbh * 16) as i32
    }

    fn luma_sad(&self, mv: [i32; 2], mbx: usize, mby: usize) -> u32 {
        let ys = self.mbw * 16;
        let x = (mbx * 16) as i32 + mv[1] / 8;
        let y = (mby * 16) as i32 + mv[0] / 8;
        let src = &self.src.y[mby * 16 * ys + mbx * 16..];
        let refp = &self.prev.y[y as usize * ys + x as usize..];
        sad(src, ys, refp, ys, 16)
    }

    /// A small diamond search over even whole pixels around the best of the candidates.
    fn search(&self, mbx: usize, mby: usize, cands: &[([i32; 2], u32)], best_ref: [i32; 2]) -> ([i32; 2], u32) {
        const LAMBDA: u32 = 4;
        const RANGE: i32 = 48 * 8;
        // Rough rate in SAD units: a candidate costs its mode, a new vector its coded offset.
        let rate = |mv: [i32; 2]| match cands.iter().find(|&&(c, _)| c == mv) {
            Some(&(_, r)) => r,
            None => LAMBDA * (2 + mv_bits((mv[0] - best_ref[0]) / 2) + mv_bits((mv[1] - best_ref[1]) / 2)),
        };
        let mut best = ([0, 0], u32::MAX, u32::MAX);
        for &(mv, _) in cands {
            if self.mv_inside(mv, mbx, mby) {
                let s = self.luma_sad(mv, mbx, mby);
                let cost = s + rate(mv);
                if cost < best.2 {
                    best = (mv, s, cost);
                }
            }
        }
        if !self.opts.motion_search || best.1 < 128 {
            return (best.0, best.1);
        }
        for step in [8 * 8, 4 * 8, 2 * 8] {
            for _ in 0..8 {
                let centre = best.0;
                for d in [[-step, 0], [step, 0], [0, -step], [0, step]] {
                    let mv = [centre[0] + d[0], centre[1] + d[1]];
                    if mv[0].abs() > RANGE || mv[1].abs() > RANGE || !self.mv_inside(mv, mbx, mby) {
                        continue;
                    }
                    let s = self.luma_sad(mv, mbx, mby);
                    let cost = s + rate(mv);
                    if cost < best.2 {
                        best = (mv, s, cost);
                    }
                }
                if best.0 == centre {
                    break;
                }
            }
        }
        (best.0, best.1)
    }

    /// Pick the prediction for one macroblock, code its residue and reconstruct it.
    fn code_mb(&mut self, mbx: usize, mby: usize, key: bool) {
        let ys = self.mbw * 16;
        let cs = self.mbw * 8;
        let (x0, y0, cx0, cy0) = (mbx * 16, mby * 16, mbx * 8, mby * 8);
        let (has_above, has_left) = (mby > 0, mbx > 0);
        let src_y = &self.src.y[y0 * ys + x0..];
        let src_u = &self.src.u[cy0 * cs + cx0..];
        let src_v = &self.src.v[cy0 * cs + cx0..];

        let (ya, yl, yc) = edges(&self.rec.y, ys, x0, y0, 16);
        let mut pred_y = [0u8; 256];
        let mut buf = [0u8; 256];
        let mut best_y = (u32::MAX, DC_PRED);
        for mode in [DC_PRED, V_PRED, H_PRED, TM_PRED] {
            predict(mode, &ya, &yl, yc, has_above, has_left, 16, &mut buf);
            let s = sad(src_y, ys, &buf, 16, 16);
            if s < best_y.0 {
                best_y = (s, mode);
                pred_y = buf;
            }
        }
        let (ua, ul, uc) = edges(&self.rec.u, cs, cx0, cy0, 8);
        let (va, vl, vc) = edges(&self.rec.v, cs, cx0, cy0, 8);
        let mut pred_u = [0u8; 64];
        let mut pred_v = [0u8; 64];
        let (mut bu, mut bv) = ([0u8; 64], [0u8; 64]);
        let mut best_uv = (u32::MAX, DC_PRED);
        for mode in [DC_PRED, V_PRED, H_PRED, TM_PRED] {
            predict(mode, &ua, &ul, uc, has_above, has_left, 8, &mut bu);
            predict(mode, &va, &vl, vc, has_above, has_left, 8, &mut bv);
            let s = sad(src_u, cs, &bu, 8, 8) + sad(src_v, cs, &bv, 8, 8);
            if s < best_uv.0 {
                best_uv = (s, mode);
                pred_u = bu;
                pred_v = bv;
            }
        }
        let mut info = MbInfo { ymode: best_y.1, uvmode: best_uv.1, ..MbInfo::default() };

        if !key {
            let (near, cnt) = self.near_mvs(mbx, mby);
            let probs = [
                MODE_CONTEXTS[cnt[0]][0],
                MODE_CONTEXTS[cnt[1]][1],
                MODE_CONTEXTS[cnt[2]][2],
                MODE_CONTEXTS[cnt[3]][3],
            ];
            let best_ref = self.clamp_mv(near[0], mbx, mby);
            let nearest = self.clamp_mv(near[1], mbx, mby);
            let near1 = self.clamp_mv(near[2], mbx, mby);
            let cands = [([0, 0], 0), (nearest, 2), (near1, 4)];
            let (mv, sad_y) = self.search(mbx, mby, &cands, best_ref);
            let px = (x0 as i32 + mv[1] / 8) as usize;
            let py = (y0 as i32 + mv[0] / 8) as usize;
            let (pcx, pcy) = ((cx0 as i32 + mv[1] / 16) as usize, (cy0 as i32 + mv[0] / 16) as usize);
            let sad_uv = sad(src_u, cs, &self.prev.u[pcy * cs + pcx..], cs, 8)
                + sad(src_v, cs, &self.prev.v[pcy * cs + pcx..], cs, 8);
            let intra = best_y.0 + best_uv.0;
            if sad_y + sad_uv <= intra + intra / 16 + 64 {
                let mode = if mv == [0, 0] {
                    MvMode::Zero
                } else if mv == nearest {
                    MvMode::Nearest
                } else if mv == near1 {
                    MvMode::Near
                } else {
                    MvMode::New
                };
                info.inter = true;
                info.mv = mv;
                info.mv_mode = mode;
                info.mv_delta = [(mv[0] - best_ref[0]) / 2, (mv[1] - best_ref[1]) / 2];
                for r in 0..16 {
                    pred_y[r * 16..r * 16 + 16].copy_from_slice(&self.prev.y[(py + r) * ys + px..(py + r) * ys + px + 16]);
                }
                for r in 0..8 {
                    pred_u[r * 8..r * 8 + 8].copy_from_slice(&self.prev.u[(pcy + r) * cs + pcx..(pcy + r) * cs + pcx + 8]);
                    pred_v[r * 8..r * 8 + 8].copy_from_slice(&self.prev.v[(pcy + r) * cs + pcx..(pcy + r) * cs + pcx + 8]);
                }
            }
            info.mode_probs = probs;
        }

        let idx = mby * self.mbw + mbx;
        let nonzero = self.residue(idx, mbx, mby, &pred_y, &pred_u, &pred_v, info.inter);
        info.skip = !nonzero;
        self.mbs[idx] = info;
    }

    /// Transform, quantize and reconstruct one macroblock against its prediction. Returns
    /// whether any quantized coefficient is non-zero.
    #[allow(clippy::too_many_arguments)]
    fn residue(&mut self, idx: usize, mbx: usize, mby: usize, pred_y: &[u8; 256], pred_u: &[u8; 64], pred_v: &[u8; 64], inter: bool) -> bool {
        let ys = self.mbw * 16;
        let cs = self.mbw * 8;
        let q = self.quant;
        // Rounding offsets in sixteenths of a step: inter residue gets a wider dead zone.
        let (dc_round, ac_round) = if inter { (8, 5) } else { (8, 6) };
        let coeffs = &mut self.coeffs[idx];
        let mut any = false;
        let mut res = [0i32; 16];
        let mut out = [0i32; 16];
        let mut dcs = [0i32; 16];
        let (x0, y0) = (mbx * 16, mby * 16);
        for b in 0..16 {
            let (bx, by) = ((b % 4) * 4, (b / 4) * 4);
            for r in 0..4 {
                for c in 0..4 {
                    let s = self.src.y[(y0 + by + r) * ys + x0 + bx + c] as i32;
                    res[r * 4 + c] = s - pred_y[(by + r) * 16 + bx + c] as i32;
                }
            }
            fdct(&res, &mut out);
            dcs[b] = out[0];
            let blk = &mut coeffs[b * 16..b * 16 + 16];
            blk[0] = 0;
            for k in 1..16 {
                blk[k] = quantize(out[k], q.y1[1], ac_round);
                any |= blk[k] != 0;
            }
        }
        let mut y2 = [0i32; 16];
        fwht(&dcs, &mut y2);
        let mut y2q = [0i32; 16];
        for k in 0..16 {
            let step = if k == 0 { q.y2[0] } else { q.y2[1] };
            let l = quantize(y2[k], step, if k == 0 { dc_round } else { ac_round + 1 });
            coeffs[384 + k] = l;
            any |= l != 0;
            y2q[k] = l as i32 * step;
        }
        iwht(&y2q, &mut dcs);
        let rec = &mut self.rec;
        for b in 0..16 {
            let (bx, by) = ((b % 4) * 4, (b / 4) * 4);
            let blk = &coeffs[b * 16..b * 16 + 16];
            let mut deq = [0i32; 16];
            deq[0] = dcs[b];
            for k in 1..16 {
                deq[k] = blk[k] as i32 * q.y1[1];
            }
            idct(&deq, &mut out);
            for r in 0..4 {
                for c in 0..4 {
                    let p = pred_y[(by + r) * 16 + bx + c] as i32 + out[r * 4 + c];
                    rec.y[(y0 + by + r) * ys + x0 + bx + c] = p.clamp(0, 255) as u8;
                }
            }
        }
        let (cx0, cy0) = (mbx * 8, mby * 8);
        for (p, (src, dst, pred)) in
            [(&self.src.u, &mut rec.u, pred_u), (&self.src.v, &mut rec.v, pred_v)].into_iter().enumerate()
        {
            for b in 0..4 {
                let (bx, by) = ((b % 2) * 4, (b / 2) * 4);
                for r in 0..4 {
                    for c in 0..4 {
                        let s = src[(cy0 + by + r) * cs + cx0 + bx + c] as i32;
                        res[r * 4 + c] = s - pred[(by + r) * 8 + bx + c] as i32;
                    }
                }
                fdct(&res, &mut out);
                let at = (16 + p * 4 + b) * 16;
                let blk = &mut coeffs[at..at + 16];
                let mut deq = [0i32; 16];
                for k in 0..16 {
                    let (step, round) = if k == 0 { (q.uv[0], dc_round) } else { (q.uv[1], ac_round) };
                    blk[k] = quantize(out[k], step, round);
                    any |= blk[k] != 0;
                    deq[k] = blk[k] as i32 * step;
                }
                idct(&deq, &mut out);
                for r in 0..4 {
                    for c in 0..4 {
                        let v = pred[(by + r) * 8 + bx + c] as i32 + out[r * 4 + c];
                        dst[(cy0 + by + r) * cs + cx0 + bx + c] = v.clamp(0, 255) as u8;
                    }
                }
            }
        }
        any
    }

    /// The coefficient tokens of the whole frame, in macroblock order, into `s`.
    fn put_tokens<S: Sink>(&self, s: &mut S) {
        let mut above = vec![[false; 9]; self.mbw];
        for mby in 0..self.mbh {
            let mut left = [false; 9];
            for mbx in 0..self.mbw {
                let i = mby * self.mbw + mbx;
                let a = &mut above[mbx];
                if self.mbs[i].skip {
                    *a = [false; 9];
                    left = [false; 9];
                    continue;
                }
                let c = &self.coeffs[i];
                let nz = put_block(s, 1, a[8] as usize + left[8] as usize, &c[384..400], 0);
                a[8] = nz;
                left[8] = nz;
                for b in 0..16 {
                    let (bx, by) = (b % 4, b / 4);
                    let nz = put_block(s, 0, a[bx] as usize + left[by] as usize, &c[b * 16..b * 16 + 16], 1);
                    a[bx] = nz;
                    left[by] = nz;
                }
                for b in 0..8 {
                    let (ai, li) = (4 + (b / 4) * 2 + b % 2, 4 + (b / 4) * 2 + (b % 4) / 2);
                    let at = (16 + b) * 16;
                    let nz = put_block(s, 2, a[ai] as usize + left[li] as usize, &c[at..at + 16], 0);
                    a[ai] = nz;
                    left[li] = nz;
                }
            }
        }
    }

    fn encode_frame(&mut self, key: bool) -> Result<Vec<u8>, String> {
        if key {
            *self.probs = DEFAULT_COEFF_PROBS;
        }
        for mby in 0..self.mbh {
            for mbx in 0..self.mbw {
                self.code_mb(mbx, mby, key);
            }
        }
        let total = self.mbs.len();
        let coded = self.mbs.iter().filter(|m| !m.skip).count();
        let intra = self.mbs.iter().filter(|m| !m.inter).count();
        let prob_skip_false = prob_of_zero(coded, total);
        let prob_intra = prob_of_zero(intra, total);

        // Probability updates that pay for themselves on this frame's tokens. They persist
        // (refresh_entropy_probs is set) until the next key frame resets the defaults.
        let mut counts: Box<Counts> = Box::new([[[[[0; 2]; 11]; 3]; 8]; 4]);
        self.put_tokens(&mut Count(&mut counts));
        let mut updates = Box::new([[[[None::<u8>; 11]; 3]; 8]; 4]);
        for t in 0..4 {
            for b in 0..8 {
                for c in 0..3 {
                    for n in 0..11 {
                        let [n0, n1] = counts[t][b][c][n];
                        let all = (n0 + n1) as usize;
                        if all == 0 {
                            continue;
                        }
                        let old = self.probs[t][b][c][n];
                        let new = prob_of_zero(n0 as usize, all);
                        if new == old {
                            continue;
                        }
                        let up = COEFF_UPDATE_PROBS[t][b][c][n];
                        let cost = |p: u8| n0 as f64 * bit_cost(false, p) + n1 as f64 * bit_cost(true, p);
                        let saved = cost(old) - cost(new) - 8.0 - (bit_cost(true, up) - bit_cost(false, up));
                        if saved > 0.0 {
                            updates[t][b][c][n] = Some(new);
                            self.probs[t][b][c][n] = new;
                        }
                    }
                }
            }
        }

        let mut h = BoolEncoder::new();
        if key {
            h.literal(0, 1); // color_space: BT.601
            h.literal(0, 1); // clamping_type: the decoder clamps
        }
        h.literal(0, 1); // segmentation_enabled
        h.literal(0, 1); // filter_type
        h.literal(0, 6); // loop_filter_level
        h.literal(0, 3); // sharpness_level
        h.literal(0, 1); // loop_filter_adj_enable
        h.literal(0, 2); // log2_nbr_of_dct_partitions
        h.literal(self.opts.q_index.min(127) as u32, 7);
        for _ in 0..5 {
            h.literal(0, 1); // no quantizer deltas
        }
        if key {
            h.literal(1, 1); // refresh_entropy_probs
        } else {
            h.literal(0, 1); // refresh_golden_frame
            h.literal(0, 1); // refresh_alternate_frame
            h.literal(0, 2); // copy_buffer_to_golden
            h.literal(0, 2); // copy_buffer_to_alternate
            h.literal(0, 1); // sign_bias_golden
            h.literal(0, 1); // sign_bias_alternate
            h.literal(1, 1); // refresh_entropy_probs
            h.literal(1, 1); // refresh_last
        }
        for t in 0..4 {
            for b in 0..8 {
                for c in 0..3 {
                    for n in 0..11 {
                        let up = updates[t][b][c][n];
                        h.put(up.is_some(), COEFF_UPDATE_PROBS[t][b][c][n]);
                        if let Some(p) = up {
                            h.literal(p as u32, 8);
                        }
                    }
                }
            }
        }
        h.literal(1, 1); // mb_no_skip_coeff
        h.literal(prob_skip_false as u32, 8);
        if !key {
            h.literal(prob_intra as u32, 8);
            h.literal(255, 8); // prob_last: every inter macroblock uses the last frame
            h.literal(128, 8); // prob_gf
            h.literal(0, 1); // intra_16x16_prob_update_flag
            h.literal(0, 1); // intra_chroma_prob_update_flag
            for probs in &MV_UPDATE_PROBS {
                for &p in probs {
                    h.put(false, p);
                }
            }
        }
        for m in &self.mbs {
            h.put(m.skip, prob_skip_false);
            if key {
                // kf_ymode_tree with kf_ymode_prob, then uv_mode_tree with kf_uv_mode_prob.
                h.put(true, 145);
                match m.ymode {
                    DC_PRED | V_PRED => {
                        h.put(false, 156);
                        h.put(m.ymode == V_PRED, 163);
                    }
                    _ => {
                        h.put(true, 156);
                        h.put(m.ymode == TM_PRED, 128);
                    }
                }
                put_uv_mode(&mut h, m.uvmode, [142, 114, 183]);
                continue;
            }
            h.put(m.inter, prob_intra);
            if m.inter {
                h.put(false, 255);
                let p = m.mode_probs;
                h.put(m.mv_mode != MvMode::Zero, p[0]);
                if m.mv_mode != MvMode::Zero {
                    h.put(m.mv_mode != MvMode::Nearest, p[1]);
                    if m.mv_mode != MvMode::Nearest {
                        h.put(m.mv_mode != MvMode::Near, p[2]);
                        if m.mv_mode == MvMode::New {
                            h.put(false, p[3]);
                            put_mv_component(&mut h, m.mv_delta[0], &DEFAULT_MV_PROBS[0]);
                            put_mv_component(&mut h, m.mv_delta[1], &DEFAULT_MV_PROBS[1]);
                        }
                    }
                }
            } else {
                // ymode_tree with the default ymode_prob, then uv_mode_tree.
                h.put(m.ymode != DC_PRED, 112);
                if m.ymode != DC_PRED {
                    h.put(m.ymode == TM_PRED, 86);
                    if m.ymode == TM_PRED {
                        h.put(false, 37);
                    } else {
                        h.put(m.ymode == H_PRED, 140);
                    }
                }
                put_uv_mode(&mut h, m.uvmode, [162, 101, 204]);
            }
        }
        let first = h.finish();
        let mut tokens = BoolEncoder::new();
        self.put_tokens(&mut Put { e: &mut tokens, p: &self.probs });
        let tokens = tokens.finish();

        if first.len() >= 1 << 19 {
            return Err("VP8 frame header partition is too large".into());
        }
        let mut out = Vec::with_capacity(10 + first.len() + tokens.len());
        let tag = (!key as u32) | (1 << 4) | ((first.len() as u32) << 5);
        out.extend_from_slice(&tag.to_le_bytes()[..3]);
        if key {
            out.extend_from_slice(&[0x9d, 0x01, 0x2a]);
            out.extend_from_slice(&(self.width as u16).to_le_bytes());
            out.extend_from_slice(&(self.height as u16).to_le_bytes());
        }
        out.extend_from_slice(&first);
        out.extend_from_slice(&tokens);
        Ok(out)
    }
}

fn put_uv_mode(h: &mut BoolEncoder, mode: u8, p: [u8; 3]) {
    h.put(mode != DC_PRED, p[0]);
    if mode != DC_PRED {
        h.put(mode != V_PRED, p[1]);
        if mode != V_PRED {
            h.put(mode == TM_PRED, p[2]);
        }
    }
}

/// A viewer-like test frame: a static backdrop gradient with a fine grid, and a shaded striped
/// sphere that moves and turns with `t`.
#[cfg(test)]
pub(crate) fn test_scene(w: u32, h: u32, t: u32) -> Vec<u8> {
    let (wf, hf, tf) = (w as f32, h as f32, t as f32);
    let cx = wf * (0.5 + 0.2 * (tf * 0.05).sin());
    let cy = hf * (0.5 + 0.1 * (tf * 0.031).cos());
    let rad = wf.min(hf) * 0.3;
    let mut v = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let (xf, yf) = (x as f32, y as f32);
            let grid = if x % 16 == 0 || y % 16 == 0 { 18.0 } else { 0.0 };
            let mut c = [20.0 + 50.0 * yf / hf + grid, 26.0 + 40.0 * yf / hf + grid, 90.0 - 50.0 * yf / hf + grid];
            let (dx, dy) = ((xf - cx) / rad, (yf - cy) / rad);
            let d2 = dx * dx + dy * dy;
            if d2 < 1.0 {
                let z = (1.0 - d2).sqrt();
                let shade = 0.2 + 0.8 * (-0.4 * dx - 0.5 * dy + 0.75 * z).max(0.0);
                let lon = dx.atan2(z) + tf * 0.08;
                let stripe = if (lon * 5.0).sin() > 0.0 { 1.0 } else { 0.5 };
                c = [235.0 * shade * stripe, 160.0 * shade * stripe, 70.0 * shade];
            }
            v.extend_from_slice(&[c[0].min(255.0) as u8, c[1].min(255.0) as u8, c[2].min(255.0) as u8, 255]);
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decode one key frame through a minimal lossy WebP container (`RIFF`, `WEBP`, `VP8 `).
    fn webp_decode(frame: &[u8]) -> (u32, u32, Vec<u8>) {
        let pad = frame.len() & 1;
        let mut riff = Vec::new();
        riff.extend_from_slice(b"RIFF");
        riff.extend_from_slice(&((12 + frame.len() + pad) as u32).to_le_bytes());
        riff.extend_from_slice(b"WEBPVP8 ");
        riff.extend_from_slice(&(frame.len() as u32).to_le_bytes());
        riff.extend_from_slice(frame);
        riff.resize(riff.len() + pad, 0);
        let mut d = image_webp::WebPDecoder::new(std::io::Cursor::new(riff)).expect("webp header");
        assert!(!d.has_alpha());
        let (w, h) = d.dimensions();
        let mut buf = vec![0; d.output_buffer_size().unwrap()];
        d.read_image(&mut buf).expect("vp8 decode");
        (w, h, buf)
    }

    /// PSNR of decoded RGB against the RGBA source.
    fn psnr(rgba: &[u8], rgb: &[u8]) -> f64 {
        let mut se = 0f64;
        for (s, d) in rgba.as_chunks::<4>().0.iter().zip(rgb.as_chunks::<3>().0) {
            for c in 0..3 {
                se += (s[c] as f64 - d[c] as f64).powi(2);
            }
        }
        let mse = se / (rgb.len() as f64);
        10.0 * (255.0 * 255.0 / mse.max(1e-9)).log10()
    }

    /// Luma PSNR of the encoder's reconstruction against its own I420 source.
    fn luma_psnr(enc: &Vp8Encoder) -> f64 {
        let (ry, _, _, mbw) = enc.reconstruction();
        let (w, h) = (enc.width as usize, enc.height as usize);
        let mut se = 0f64;
        for y in 0..h {
            for x in 0..w {
                se += (ry[y * mbw * 16 + x] as f64 - enc.src.y[y * mbw * 16 + x] as f64).powi(2);
            }
        }
        10.0 * (65025.0 / (se / (w * h) as f64).max(1e-9)).log10()
    }

    #[test]
    fn capture_vp8_key_frames_decode_with_image_webp() {
        // Tiny frames of a sharp, saturated scene lose RGB PSNR to 4:2:0 chroma, not the codec.
        for &(w, h, floor) in &[(16u32, 16u32, 26.0), (33, 17, 26.0), (64, 48, 31.0), (160, 100, 34.0), (320, 200, 36.0)] {
            let mut enc = Vp8Encoder::new(w, h, Vp8Options::default()).unwrap();
            let src = test_scene(w, h, 3);
            let f = enc.encode_rgba(&src).unwrap();
            assert!(f.key);
            assert_eq!(&f.data[3..6], &[0x9d, 0x01, 0x2a]);
            let (dw, dh, rgb) = webp_decode(&f.data);
            assert_eq!((dw, dh), (w, h));
            let p = psnr(&src, &rgb);
            assert!(p > floor, "{w}x{h}: PSNR {p:.1} dB");
            let y = luma_psnr(&enc);
            assert!(y > 38.0, "{w}x{h}: luma PSNR {y:.1} dB");
        }
    }

    #[test]
    fn capture_vp8_quantizer_range_decodes() {
        let (w, h) = (96, 64);
        let src = test_scene(w, h, 0);
        let mut last = (usize::MAX, f64::MAX);
        for (q, floor) in [(0u8, 34.0), (16, 32.0), (60, 27.0), (127, 20.0)] {
            let opts = Vp8Options { q_index: q, ..Vp8Options::default() };
            let mut enc = Vp8Encoder::new(w, h, opts).unwrap();
            let f = enc.encode_rgba(&src).unwrap();
            let (_, _, rgb) = webp_decode(&f.data);
            let p = psnr(&src, &rgb);
            assert!(p > floor, "q {q}: PSNR {p:.1} dB");
            let y = luma_psnr(&enc);
            assert!(f.data.len() < last.0 && y < last.1, "q {q}: {} bytes, luma {y:.1} dB", f.data.len());
            last = (f.data.len(), y);
        }
        assert!(last.1 > 24.0);
        // Flat black and see-through frames are tiny and stay black.
        let clear = vec![0u8; (w * h * 4) as usize];
        let f = Vp8Encoder::new(w, h, Vp8Options::default()).unwrap().encode_rgba(&clear).unwrap();
        let (_, _, rgb) = webp_decode(&f.data);
        assert!(rgb.iter().all(|&v| v <= 2), "black decodes black");
        assert!(f.data.len() < 120, "{} bytes", f.data.len());
    }

    #[test]
    fn capture_vp8_inter_frames_are_smaller_and_key_frames_recur() {
        let (w, h) = (96, 64);
        let opts = Vp8Options { keyframe_interval: 4, ..Vp8Options::default() };
        let mut enc = Vp8Encoder::new(w, h, opts).unwrap();
        let frames: Vec<Vp8Frame> = (0..6).map(|t| enc.encode_rgba(&test_scene(w, h, t)).unwrap()).collect();
        let keys: Vec<bool> = frames.iter().map(|f| f.key).collect();
        assert_eq!(keys, [true, false, false, false, true, false]);
        for f in &frames[1..4] {
            assert_eq!(f.data[0] & 1, 1, "inter frame tag");
            assert!(f.data.len() * 2 < frames[0].data.len(), "{} vs {}", f.data.len(), frames[0].data.len());
        }
        // The recurring key frame decodes on its own.
        let src = test_scene(w, h, 4);
        let (_, _, rgb) = webp_decode(&frames[4].data);
        assert!(psnr(&src, &rgb) > 30.0);
        // A repeated frame is all skipped macroblocks: a few bytes.
        let mut enc = Vp8Encoder::new(w, h, Vp8Options::default()).unwrap();
        let still = test_scene(w, h, 0);
        enc.encode_rgba(&still).unwrap();
        let again = enc.encode_rgba(&still).unwrap();
        assert!(again.data.len() < 60, "{} bytes", again.data.len());
    }

    #[test]
    fn capture_vp8_rejects_bad_sizes() {
        assert!(Vp8Encoder::new(0, 10, Vp8Options::default()).is_err());
        assert!(Vp8Encoder::new(16384, 10, Vp8Options::default()).is_err());
        let mut enc = Vp8Encoder::new(8, 8, Vp8Options::default()).unwrap();
        assert!(enc.encode_rgba(&[0; 10]).is_err());
    }

    /// The RFC's boolean decoder, to check the encoder against.
    struct BoolDecoder<'a> {
        data: &'a [u8],
        pos: usize,
        value: u32,
        range: u32,
        bit_count: i32,
    }

    impl<'a> BoolDecoder<'a> {
        fn new(data: &'a [u8]) -> Self {
            let byte = |i: usize| *data.get(i).unwrap_or(&0) as u32;
            BoolDecoder { data, pos: 2, value: (byte(0) << 8) | byte(1), range: 255, bit_count: 0 }
        }

        fn get(&mut self, prob: u8) -> bool {
            let split = 1 + (((self.range - 1) * prob as u32) >> 8);
            let big = split << 8;
            let bit = if self.value >= big {
                self.range -= split;
                self.value -= big;
                true
            } else {
                self.range = split;
                false
            };
            while self.range < 128 {
                self.value <<= 1;
                self.range <<= 1;
                self.bit_count += 1;
                if self.bit_count == 8 {
                    self.bit_count = 0;
                    self.value |= *self.data.get(self.pos).unwrap_or(&0) as u32;
                    self.pos += 1;
                }
            }
            bit
        }
    }

    #[test]
    fn capture_vp8_bool_coder_round_trips() {
        let mut s = 0x1234_5678u32;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            s
        };
        let bits: Vec<(bool, u8)> = (0..20000)
            .map(|_| {
                let p = (next() % 255 + 1) as u8;
                // Mostly follow the probability, with runs of unlikely bits to force carries.
                let bit = (next() % 256) as u8 >= p;
                (bit, p)
            })
            .collect();
        let mut e = BoolEncoder::new();
        for &(b, p) in &bits {
            e.put(b, p);
        }
        let data = e.finish();
        let mut d = BoolDecoder::new(&data);
        for (i, &(b, p)) in bits.iter().enumerate() {
            assert_eq!(d.get(p), b, "bit {i}");
        }
        // All ones at a near-certain zero probability exercises carries.
        let mut e = BoolEncoder::new();
        for _ in 0..5000 {
            e.put(true, 1);
            e.put(false, 255);
        }
        let data = e.finish();
        let mut d = BoolDecoder::new(&data);
        for _ in 0..5000 {
            assert!(d.get(1));
            assert!(!d.get(255));
        }
    }

    #[test]
    fn capture_vp8_transforms_invert() {
        let mut s = 99u32;
        for _ in 0..2000 {
            let mut res = [0i32; 16];
            for v in &mut res {
                s = s.wrapping_mul(1_103_515_245).wrapping_add(12345);
                *v = ((s >> 16) % 511) as i32 - 255;
            }
            let (mut f, mut back) = ([0i32; 16], [0i32; 16]);
            fdct(&res, &mut f);
            idct(&f, &mut back);
            for k in 0..16 {
                assert!((back[k] - res[k]).abs() <= 1, "dct {res:?} -> {back:?}");
            }
            let mut dcs = [0i32; 16];
            for v in &mut dcs {
                s = s.wrapping_mul(1_103_515_245).wrapping_add(12345);
                *v = ((s >> 16) % 4081) as i32 - 2040;
            }
            fwht(&dcs, &mut f);
            iwht(&f, &mut back);
            for k in 0..16 {
                assert!((back[k] - dcs[k]).abs() <= 1, "wht {dcs:?} -> {back:?}");
            }
        }
    }

    #[test]
    fn capture_vp8_mv_components_decode() {
        // Decode with the RFC's read_mvcomponent.
        fn read(d: &mut BoolDecoder, p: &[u8; 19]) -> i32 {
            let mut a = 0i32;
            if d.get(p[0]) {
                for i in 0..3 {
                    a += (d.get(p[9 + i]) as i32) << i;
                }
                for i in (4..10).rev() {
                    a += (d.get(p[9 + i]) as i32) << i;
                }
                if a & 0xfff0 == 0 || d.get(p[9 + 3]) {
                    a += 8;
                }
            } else {
                let tree: [i32; 14] = [2, 8, 4, 6, 0, -1, -2, -3, 10, 12, -4, -5, -6, -7];
                let mut i = 0usize;
                loop {
                    let n = tree[i + d.get(p[2 + i / 2]) as usize];
                    if n <= 0 {
                        a = -n;
                        break;
                    }
                    i = n as usize;
                }
            }
            if a != 0 && d.get(p[1]) { -a } else { a }
        }
        let values: Vec<i32> = (-1023..=1023).collect();
        let mut e = BoolEncoder::new();
        for &v in &values {
            put_mv_component(&mut e, v, &DEFAULT_MV_PROBS[0]);
        }
        let data = e.finish();
        let mut d = BoolDecoder::new(&data);
        for &v in &values {
            assert_eq!(read(&mut d, &DEFAULT_MV_PROBS[0]), v);
        }
    }
}
