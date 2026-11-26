//! The shareable result card: a 1200 x 630 PNG drawn here, without the window. Text is Inter,
//! the faces Rune already embeds (taken from Rune, not embedded again), rasterized with `swash`,
//! which Rune already links; the image is written with the `image` crate Sigil already uses.
//! Nothing is uploaded.

use deadlock_core::benchmark::compare::Verdict;
use deadlock_core::benchmark::store::{RunRecord, run_label};
use deadlock_core::benchmark::{Side, metrics};
use image::{ImageEncoder, Rgba, RgbaImage};
use swash::FontRef;
use swash::scale::{Render, ScaleContext, Source};
use swash::zeno::Format;

use super::tr;
use crate::screens::settings::i18n::fill;

pub const W: u32 = 1200;
pub const H: u32 = 630;

/// Rune's own Inter faces (`rune::text::embedded_fonts`: regular, medium, semibold), so the
/// executable carries them once. Missing faces draw no text rather than fail.
fn face(at: usize) -> &'static [u8] {
    rune::text::embedded_fonts().get(at).copied().unwrap_or(&[])
}

fn regular() -> &'static [u8] {
    face(0)
}

fn medium() -> &'static [u8] {
    face(1)
}

fn semibold() -> &'static [u8] {
    face(2)
}

type Rgb = [u8; 3];
const BG: Rgb = [23, 23, 26];
const PANEL: Rgb = [31, 31, 35];
const INK: Rgb = [245, 245, 247];
const MUTED: Rgb = [170, 170, 178];
const FAINT: Rgb = [118, 118, 128];
const EMBER: Rgb = [240, 144, 84];
const TEAL: Rgb = [116, 214, 156];
const ROSE: Rgb = [238, 124, 112];

struct Canvas {
    img: RgbaImage,
    scale: ScaleContext,
}

impl Canvas {
    fn new() -> Self {
        let mut img = RgbaImage::new(W, H);
        for px in img.pixels_mut() {
            *px = Rgba([BG[0], BG[1], BG[2], 255]);
        }
        Canvas { img, scale: ScaleContext::new() }
    }

    fn blend(&mut self, x: i32, y: i32, c: Rgb, a: f32) {
        if x < 0 || y < 0 || x >= W as i32 || y >= H as i32 || a <= 0.0 {
            return;
        }
        let a = a.min(1.0);
        let px = self.img.get_pixel_mut(x as u32, y as u32);
        for (dst, src) in px.0.iter_mut().zip(c.iter()) {
            *dst = (*dst as f32 * (1.0 - a) + *src as f32 * a).round() as u8;
        }
    }

    fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: Rgb, a: f32) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.blend(xx, yy, c, a);
            }
        }
    }

    /// A rounded panel.
    fn panel(&mut self, x: i32, y: i32, w: i32, h: i32, r: f32) {
        for yy in y..y + h {
            for xx in x..x + w {
                let dx = (xx as f32 + 0.5 - x as f32).min(x as f32 + w as f32 - xx as f32 - 0.5);
                let dy = (yy as f32 + 0.5 - y as f32).min(y as f32 + h as f32 - yy as f32 - 0.5);
                let cover = if dx < r && dy < r {
                    let d = ((r - dx).powi(2) + (r - dy).powi(2)).sqrt();
                    (r - d + 0.5).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                self.blend(xx, yy, PANEL, cover);
            }
        }
    }

    fn measure(&mut self, font: &[u8], px: f32, s: &str) -> f32 {
        let Some(font) = FontRef::from_index(font, 0) else { return 0.0 };
        let metrics = font.glyph_metrics(&[]).scale(px);
        let map = font.charmap();
        s.chars().map(|c| metrics.advance_width(map.map(c))).sum()
    }

    /// Draw `s` with its baseline at `y`. Returns the width.
    #[allow(clippy::too_many_arguments)]
    fn text(&mut self, font: &[u8], px: f32, x: f32, y: f32, s: &str, c: Rgb, alpha: f32) -> f32 {
        let Some(font) = FontRef::from_index(font, 0) else { return 0.0 };
        let metrics = font.glyph_metrics(&[]).scale(px);
        let map = font.charmap();
        let mut pen = x;
        let mut glyphs = Vec::new();
        {
            let mut scaler = self.scale.builder(font).size(px).hint(false).build();
            for ch in s.chars() {
                let id = map.map(ch);
                if let Some(image) = Render::new(&[Source::Outline]).format(Format::Alpha).render(&mut scaler, id) {
                    glyphs.push((pen, image));
                }
                pen += metrics.advance_width(id);
            }
        }
        for (at, image) in glyphs {
            let p = image.placement;
            let left = at.round() as i32 + p.left;
            let top = y.round() as i32 - p.top;
            for row in 0..p.height as i32 {
                for col in 0..p.width as i32 {
                    let a = image.data[(row * p.width as i32 + col) as usize] as f32 / 255.0;
                    self.blend(left + col, top + row, c, a * alpha);
                }
            }
        }
        pen - x
    }

    /// An anti-aliased polyline, drawn through a coverage mask so joins do not double up.
    fn polyline(&mut self, points: &[[f32; 2]], width: f32, c: Rgb, alpha: f32) {
        let mut mask = vec![0f32; (W * H) as usize];
        let r = width / 2.0;
        for pair in points.windows(2) {
            let ([x0, y0], [x1, y1]) = (pair[0], pair[1]);
            let (minx, maxx) = (x0.min(x1) - r - 1.0, x0.max(x1) + r + 1.0);
            let (miny, maxy) = (y0.min(y1) - r - 1.0, y0.max(y1) + r + 1.0);
            let (dx, dy) = (x1 - x0, y1 - y0);
            let len2 = (dx * dx + dy * dy).max(1e-6);
            for yy in miny.max(0.0) as i32..=maxy.min(H as f32 - 1.0) as i32 {
                for xx in minx.max(0.0) as i32..=maxx.min(W as f32 - 1.0) as i32 {
                    let (px, py) = (xx as f32 + 0.5, yy as f32 + 0.5);
                    let t = (((px - x0) * dx + (py - y0) * dy) / len2).clamp(0.0, 1.0);
                    let d = ((px - x0 - t * dx).powi(2) + (py - y0 - t * dy).powi(2)).sqrt();
                    let cover = (r + 0.5 - d).clamp(0.0, 1.0);
                    let at = (yy as u32 * W + xx as u32) as usize;
                    mask[at] = mask[at].max(cover);
                }
            }
        }
        for (i, cover) in mask.iter().enumerate() {
            if *cover > 0.0 {
                self.blend((i as u32 % W) as i32, (i as u32 / W) as i32, c, cover * alpha);
            }
        }
    }

    /// The area under a line, fading towards `floor`.
    fn area(&mut self, points: &[[f32; 2]], floor: f32, c: Rgb, top_alpha: f32) {
        for pair in points.windows(2) {
            let ([x0, y0], [x1, y1]) = (pair[0], pair[1]);
            for xx in x0.round() as i32..x1.round() as i32 {
                let t = if x1 > x0 { (xx as f32 - x0) / (x1 - x0) } else { 0.0 };
                let y = y0 + t * (y1 - y0);
                for yy in y.round() as i32..floor as i32 {
                    let fade = 1.0 - (yy as f32 - y) / (floor - y).max(1.0);
                    self.blend(xx, yy, c, top_alpha * fade.clamp(0.0, 1.0));
                }
            }
        }
    }

    fn png(&self) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(self.img.as_raw(), W, H, image::ExtendedColorType::Rgba8)
            .map_err(|e| e.to_string())?;
        Ok(out)
    }
}

fn number(v: f64) -> String {
    if v >= 100.0 { format!("{v:.0}") } else { format!("{v:.1}") }
}

/// The card for a run. `frames` are the passes the graph draws, A and B.
pub fn png(run: &RunRecord, frames: &[(Side, Vec<f64>)]) -> Result<Vec<u8>, String> {
    let mut c = Canvas::new();
    let x0 = 56.0;
    // Head.
    let w = c.text(semibold(), 22.0, x0, 74.0, "Sigil", EMBER, 1.0);
    c.text(regular(), 22.0, x0 + w + 10.0, 74.0, tr("Benchmark"), MUTED, 1.0);
    let date = crate::prefs::show_stamp_date(&run.spec.created_at).unwrap_or_default();
    let dw = c.measure(regular(), 18.0, &date);
    c.text(regular(), 18.0, W as f32 - x0 - dw, 74.0, &date, FAINT, 1.0);
    let mut sub = vec![run_label(run)];
    if let Some(demo) = &run.spec.demo {
        sub.push(fill(tr("Match {}"), &[&demo.label]));
        if let Some(map) = &demo.map {
            sub.push(map.clone());
        }
        sub.push(tr(run.spec.length.label()).to_string());
    }
    if let Some(build) = &run.settings.build {
        sub.push(fill(tr("build {}"), &[build]));
    }
    c.text(regular(), 18.0, x0, 108.0, &sub.join("  ·  "), MUTED, 1.0);

    // The headline.
    let head = run.headline();
    let summary = head.as_ref().map(|h| h.summary.clone()).unwrap_or_default();
    let big = if summary.avg_fps > 0.0 { format!("{:.0}", summary.avg_fps) } else { "-".into() };
    let bw = c.text(semibold(), 128.0, x0 - 4.0, 252.0, &big, INK, 1.0);
    c.text(medium(), 26.0, x0 + bw + 6.0, 252.0, "FPS", MUTED, 1.0);
    c.text(regular(), 17.0, x0, 286.0, tr("average"), FAINT, 1.0);

    let stats: [(&str, Option<String>); 4] = [
        ("1% low", summary.low_1.map(number)),
        ("0.1% low", summary.low_01.map(number)),
        ("p99 frame", summary.p99_ms.map(|v| format!("{v:.1} ms"))),
        ("Stutters", summary.stutters.map(|n| n.to_string())),
    ];
    let mut sx = 500.0;
    for (label, value) in stats {
        let value = value.unwrap_or_else(|| "-".into());
        c.text(semibold(), 40.0, sx, 236.0, &value, INK, 1.0);
        c.text(regular(), 16.0, sx, 266.0, tr(label), FAINT, 1.0);
        sx += 166.0;
    }

    // The comparison, or the spread.
    let line = match run.comparison() {
        Some(cmp) => {
            let pct = (cmp.gap_share * 100.0).abs();
            let (color, words) = match cmp.verdict {
                Verdict::Faster => (TEAL, fill(tr("{}: {}% faster than {} ({} FPS)"), &[tr(&cmp.b.label), &format!("{pct:.1}"), &super::lower(tr(&cmp.a.label)), &format!("{:+.1}", cmp.gap_fps)])),
                Verdict::Slower => (ROSE, fill(tr("{}: {}% slower than {} ({} FPS)"), &[tr(&cmp.b.label), &format!("{pct:.1}"), &super::lower(tr(&cmp.a.label)), &format!("{:+.1}", cmp.gap_fps)])),
                Verdict::Same => (MUTED, fill(tr("{}: the same as {}, inside the noise"), &[tr(&cmp.b.label), &super::lower(tr(&cmp.a.label))])),
            };
            Some((color, words))
        }
        None => head.as_ref().and_then(|h| h.spread).map(|(lo, hi)| {
            let n = head.as_ref().map(|h| h.passes).unwrap_or(0).to_string();
            (MUTED, fill(tr("{} passes, from {} to {} FPS"), &[&n, &format!("{lo:.0}"), &format!("{hi:.0}")]))
        }),
    };
    if let Some((color, words)) = line {
        c.rect(x0 as i32, 318, 4, 26, color, 1.0);
        c.text(medium(), 21.0, x0 + 16.0, 338.0, &words, color, 1.0);
    }

    // The graph: frame time per bin, A muted behind B in ember.
    let (gx, gy, gw, gh) = (x0 as i32 - 16, 372, (W as f32 - 2.0 * x0) as i32 + 32, 178);
    c.panel(gx, gy, gw, gh, 14.0);
    let series: Vec<(Side, Vec<metrics::Bin>)> = frames.iter().map(|(side, f)| (*side, metrics::graph(f, 240))).collect();
    let top_ms = series
        .iter()
        .flat_map(|(_, bins)| bins.iter().map(|b| 1000.0 / b.fps))
        .fold(0.0f64, f64::max)
        .max(1.0)
        * 1.25;
    let (px0, px1) = (gx as f32 + 20.0, (gx + gw) as f32 - 20.0);
    let (py0, py1) = (gy as f32 + 26.0, (gy + gh) as f32 - 18.0);
    for step in [0.25, 0.5, 0.75] {
        let y = py1 - (py1 - py0) * step as f32;
        c.rect(px0 as i32, y as i32, (px1 - px0) as i32, 1, INK, 0.05);
        let label = format!("{:.0} ms", top_ms * step);
        c.text(regular(), 12.0, px0 + 2.0, y - 4.0, &label, FAINT, 1.0);
    }
    let two = series.len() > 1;
    for (side, bins) in &series {
        if bins.len() < 2 {
            continue;
        }
        let total = bins.last().map(|b| b.at).unwrap_or(1.0).max(1e-6);
        let points: Vec<[f32; 2]> = bins
            .iter()
            .map(|b| {
                let x = px0 + (b.at / total) as f32 * (px1 - px0);
                let y = py1 - ((1000.0 / b.fps) / top_ms) as f32 * (py1 - py0);
                [x, y]
            })
            .collect();
        let color = if two && *side == Side::A { MUTED } else { EMBER };
        if !(two && *side == Side::A) {
            c.area(&points, py1, color, 0.22);
        }
        c.polyline(&points, 2.0, color, if two && *side == Side::A { 0.7 } else { 1.0 });
    }
    if series.is_empty() {
        let words = tr("No per-frame data for this run");
        let ww = c.measure(regular(), 16.0, words);
        c.text(regular(), 16.0, gx as f32 + (gw as f32 - ww) / 2.0, gy as f32 + gh as f32 / 2.0 + 6.0, words, FAINT, 1.0);
    } else {
        let label = tr("Frame time across the section");
        c.text(medium(), 13.0, px0, gy as f32 + 20.0, label, MUTED, 1.0);
        if two {
            let a = run.spec.test.sides().map(|s| tr(s.0)).unwrap_or("A");
            let b = run.spec.test.sides().map(|s| tr(s.1)).unwrap_or("B");
            let bw = c.measure(medium(), 13.0, b);
            let aw = c.measure(medium(), 13.0, a);
            c.rect((px1 - bw - 14.0) as i32, gy + 12, 8, 8, EMBER, 1.0);
            c.text(medium(), 13.0, px1 - bw, gy as f32 + 20.0, b, MUTED, 1.0);
            c.rect((px1 - bw - aw - 40.0) as i32, gy + 12, 8, 8, MUTED, 1.0);
            c.text(medium(), 13.0, px1 - bw - aw - 26.0, gy as f32 + 20.0, a, MUTED, 1.0);
        }
    }

    // The machine.
    let m = &run.machine;
    let mut foot: Vec<String> = Vec::new();
    if let Some(cpu) = &m.cpu {
        foot.push(cpu.split_whitespace().collect::<Vec<_>>().join(" "));
    }
    if let Some(gpu) = &m.gpu {
        foot.push(match &m.driver {
            Some(d) => fill(tr("{} (driver {})"), &[gpu, d]),
            None => gpu.clone(),
        });
    }
    if let Some(res) = run.settings.resolution() {
        foot.push(match run.settings.window_mode() {
            Some(mode) => format!("{res} {}", tr(mode)),
            None => res,
        });
    }
    if let Some(ram) = m.ram_mb {
        foot.push(format!("{} GB", (ram as f64 / 1024.0).round()));
    }
    c.text(regular(), 16.0, x0, 594.0, &foot.join("  ·  "), FAINT, 1.0);
    c.png()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_card_is_a_png_of_the_right_size() {
        let mut s = crate::app::State::default();
        super::super::scenes::stage_result(&mut s);
        let run = s.performance.benchmark.runs[0].clone();
        let frames = super::super::graph_frames(&s, &run);
        let bytes = png(&run, &frames).unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (W, H));
        if std::env::var_os("SIGIL_BENCH_CARD").is_some() {
            std::fs::write(std::env::var("SIGIL_BENCH_CARD").unwrap(), &bytes).unwrap();
        }
    }
}
