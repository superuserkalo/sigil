//! Deadlock Mod Manager's axis gizmo (`foundry-model-viewer.tsx`): the world axes as the camera
//! sees them, in the stage's top right corner. Drawn into the preview frame after the
//! recorders took it, so pictures, GIFs and videos never show it.

/// DMM's gizmo palette: X, Y, Z.
const COLORS: [[u8; 3]; 3] = [[0xd9, 0x8d, 0x6a], [0xe7, 0xd5, 0xa3], [0x8f, 0xb0, 0xc9]];
/// 3 by 5 letters, one row a byte (low three bits, the leftmost column highest).
const LETTERS: [[u8; 5]; 3] = [[0b101, 0b101, 0b010, 0b101, 0b101], [0b101, 0b101, 0b010, 0b010, 0b010], [0b111, 0b001, 0b010, 0b100, 0b111]];
/// Points from the frame's corner to the gizmo's centre, the axis length and the handle radius.
const INSET: f32 = 58.0;
const REACH: f32 = 30.0;
const HANDLE: f32 = 8.0;
/// A narrower stage keeps its title and leaves the gizmo out.
const MIN_POINTS: f32 = 480.0;

/// One end of an axis on screen: right and up from the centre (-1 to 1), and toward the eye.
#[derive(Clone, Copy, Debug, PartialEq)]
struct End {
    axis: usize,
    positive: bool,
    x: f32,
    y: f32,
    depth: f32,
}

/// The six axis ends for an orbit camera at `yaw` and `pitch`, farthest first.
fn ends(yaw: f32, pitch: f32) -> [End; 6] {
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    let right = [cy, 0.0, -sy];
    let up = [-sy * sp, cp, -cy * sp];
    let eye = [sy * cp, sp, cy * cp];
    let end = |i: usize| {
        let (axis, sign) = (i % 3, if i < 3 { 1.0 } else { -1.0 });
        End { axis, positive: i < 3, x: right[axis] * sign, y: up[axis] * sign, depth: eye[axis] * sign }
    };
    let mut out = [end(0), end(1), end(2), end(3), end(4), end(5)];
    out.sort_by(|a, b| a.depth.total_cmp(&b.depth));
    out
}

/// A canvas over the frame's RGBA bytes.
struct Canvas<'a> {
    rgba: &'a mut [u8],
    w: usize,
    h: usize,
}

impl Canvas<'_> {
    fn blend(&mut self, x: usize, y: usize, color: [u8; 3], cover: f32) {
        let at = (y * self.w + x) * 4;
        let Some(px) = self.rgba.get_mut(at..at + 4) else { return };
        let cover = cover.clamp(0.0, 1.0);
        for (p, c) in px.iter_mut().zip(color) {
            *p = (f32::from(*p) + (f32::from(c) - f32::from(*p)) * cover).round() as u8;
        }
        px[3] = (f32::from(px[3]) + (255.0 - f32::from(px[3])) * cover).round() as u8;
    }

    /// Runs `f` over the pixels of a box, clipped to the canvas, with the pixel's centre.
    fn each(&mut self, min: [f32; 2], max: [f32; 2], mut f: impl FnMut(&mut Self, usize, usize, f32, f32)) {
        let x0 = min[0].floor().max(0.0) as usize;
        let y0 = min[1].floor().max(0.0) as usize;
        let x1 = (max[0].ceil().max(0.0) as usize).min(self.w);
        let y1 = (max[1].ceil().max(0.0) as usize).min(self.h);
        for y in y0..y1 {
            for x in x0..x1 {
                f(self, x, y, x as f32 + 0.5, y as f32 + 0.5);
            }
        }
    }

    fn line(&mut self, a: [f32; 2], b: [f32; 2], width: f32, color: [u8; 3]) {
        let pad = width + 1.0;
        let min = [a[0].min(b[0]) - pad, a[1].min(b[1]) - pad];
        let max = [a[0].max(b[0]) + pad, a[1].max(b[1]) + pad];
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len2 = (dx * dx + dy * dy).max(1e-4);
        self.each(min, max, |c, x, y, px, py| {
            let t = (((px - a[0]) * dx + (py - a[1]) * dy) / len2).clamp(0.0, 1.0);
            let d = (px - a[0] - dx * t).hypot(py - a[1] - dy * t);
            c.blend(x, y, color, width / 2.0 + 0.5 - d);
        });
    }

    /// A disc, or a ring when `ring` is the stroke width.
    fn disc(&mut self, at: [f32; 2], radius: f32, ring: Option<f32>, color: [u8; 3], alpha: f32) {
        let pad = radius + 1.0;
        self.each([at[0] - pad, at[1] - pad], [at[0] + pad, at[1] + pad], |c, x, y, px, py| {
            let d = (px - at[0]).hypot(py - at[1]);
            let outer = radius + 0.5 - d;
            let cover = ring.map_or(outer, |w| outer.min(d - (radius - w) + 0.5));
            if cover > 0.0 {
                c.blend(x, y, color, cover * alpha);
            }
        });
    }

    fn letter(&mut self, at: [f32; 2], axis: usize, unit: usize) {
        let (left, top) = ((at[0] - 1.5 * unit as f32).round() as isize, (at[1] - 2.5 * unit as f32).round() as isize);
        for (row, bits) in LETTERS[axis].iter().enumerate() {
            for column in 0..3 {
                if bits & (0b100 >> column) == 0 {
                    continue;
                }
                let (x, y) = (left + (column * unit) as isize, top + (row * unit) as isize);
                if x < 0 || y < 0 {
                    continue;
                }
                let (x, y) = (x as f32, y as f32);
                let size = unit as f32;
                self.each([x, y], [x + size, y + size], |c, px, py, _, _| c.blend(px, py, [0x14, 0x14, 0x16], 1.0));
            }
        }
    }
}

/// Draws the gizmo into a `w` by `h` RGBA frame at `scale` device pixels a point.
pub fn draw(rgba: &mut [u8], w: u32, h: u32, yaw: f32, pitch: f32, scale: f32) {
    let scale = if scale.is_finite() { scale.max(1.0) } else { 1.0 };
    let (fw, fh) = (w as f32, h as f32);
    if fw < MIN_POINTS * scale || fh < 2.0 * INSET * scale || rgba.len() < w as usize * h as usize * 4 {
        return;
    }
    let centre = [fw - INSET * scale, INSET * scale];
    let reach = REACH * scale;
    let radius = HANDLE * scale;
    let unit = (1.6 * scale).round().max(1.0) as usize;
    let mut canvas = Canvas { rgba, w: w as usize, h: h as usize };
    for end in ends(yaw, pitch) {
        let at = [centre[0] + end.x * reach, centre[1] - end.y * reach];
        let color = COLORS[end.axis];
        if end.positive {
            canvas.line(centre, at, 2.0 * scale, color);
            canvas.disc(at, radius, None, color, 1.0);
            canvas.letter(at, end.axis, unit);
        } else {
            canvas.disc(at, radius * 0.72, Some(1.25 * scale), color, 0.6);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_axes_face_the_way_the_camera_sees_them() {
        let find = |ends: &[End; 6], axis: usize| *ends.iter().find(|e| e.axis == axis && e.positive).unwrap();
        // From the front: X to the right, Y up, Z at the viewer and drawn last.
        let front = ends(0.0, 0.0);
        let (x, y, z) = (find(&front, 0), find(&front, 1), find(&front, 2));
        assert!((x.x - 1.0).abs() < 1e-5 && x.y.abs() < 1e-5);
        assert!((y.y - 1.0).abs() < 1e-5 && y.x.abs() < 1e-5);
        assert!((z.depth - 1.0).abs() < 1e-5);
        assert_eq!((front[5].axis, front[5].positive), (2, true));
        // A quarter turn to the right of the hero: +X now points at the viewer.
        let side = ends(std::f32::consts::FRAC_PI_2, 0.0);
        assert!((find(&side, 0).depth - 1.0).abs() < 1e-5);
        assert!((find(&side, 2).x + 1.0).abs() < 1e-5);
        // From above, +Y points at the viewer.
        let top = ends(0.0, std::f32::consts::FRAC_PI_2);
        assert!((find(&top, 1).depth - 1.0).abs() < 1e-5);
    }

    #[test]
    fn it_draws_only_in_its_corner_and_skips_a_narrow_stage() {
        let (w, h) = (640u32, 360u32);
        let mut frame = vec![0u8; (w * h * 4) as usize];
        draw(&mut frame, w, h, 0.42, 0.2, 1.0);
        let lit = |x: u32, y: u32| frame[((y * w + x) * 4 + 3) as usize] > 0;
        assert!((w - 116..w).any(|x| (0..116).any(|y| lit(x, y))), "the corner has the gizmo");
        assert!((0..w - 116).all(|x| (0..h).all(|y| !lit(x, y))), "left of it is untouched");
        assert!((w - 116..w).all(|x| (116..h).all(|y| !lit(x, y))), "below it is untouched");

        let mut narrow = vec![0u8; 400 * 300 * 4];
        draw(&mut narrow, 400, 300, 0.0, 0.0, 1.0);
        assert!(narrow.iter().all(|b| *b == 0));
        // A short buffer and a bad scale are ignored, never a panic.
        draw(&mut [0u8; 16], w, h, 0.0, 0.0, 1.0);
        let mut again = vec![0u8; (w * h * 4) as usize];
        draw(&mut again, w, h, 0.0, 0.0, f32::NAN);
    }
}
