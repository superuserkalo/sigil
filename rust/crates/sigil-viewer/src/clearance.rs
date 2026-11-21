//! Grimoire's distant-instance culling for the Hideout (MIT, `src/lib/hideoutClearance.ts` and
//! `HideoutScene.tsx`): props whose bounds lie more than 65 m from the spawn are drawn only when
//! the room around the hero has an opening toward them.
//!
//! The room's free distance around the hero is measured once per scene: six 90 degree views of
//! the near room's opaque surfaces, 32 by 32 each, from the hero (`Viewer::set_scene_with`
//! renders them with the shadow pipeline and reads the depths back). A distant prop stays when
//! any point of a 5 by 5 by 5 grid over its box is at least as close as the free distance in its
//! direction (99 %), or the free distance there reaches the far limit: an open window or door.

use crate::math::{Aabb, Mat4, Vec3, add, length, scale, sub};

/// Texels per cube face edge.
pub const EDGE: u32 = 32;
/// Props whose bounds are farther than this from the spawn (meters) are tested.
pub const NEAR: f32 = 65.0;
/// The measurement's far limit (meters): the free distance where nothing is hit.
pub const REACH: f32 = 1000.0;
/// The views look along +X, -X, +Y, -Y, +Z, -Z.
const AXES: [Vec3; 6] = [[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, -1.0]];
const UPS: [Vec3; 6] = [[0.0, 1.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [0.0, 1.0, 0.0]];

/// The six views' projections from `origin`.
pub fn faces(origin: Vec3) -> [Mat4; 6] {
    std::array::from_fn(|i| Mat4::perspective(std::f32::consts::FRAC_PI_2, 1.0, REACH * 0.001, REACH).mul(&Mat4::look_at(origin, add(origin, AXES[i]), UPS[i])))
}

fn project(m: &Mat4, p: Vec3) -> [f32; 4] {
    let c = m.0;
    let row = |r: usize| c[0][r] * p[0] + c[1][r] * p[1] + c[2][r] * p[2] + c[3][r];
    [row(0), row(1), row(2), row(3)]
}

/// The distance from `origin` to the nearest point of `b` (0 inside).
pub fn box_distance(b: &Aabb, p: Vec3) -> f32 {
    let d: Vec3 = std::array::from_fn(|i| (b.min[i] - p[i]).max(0.0).max(p[i] - b.max[i]));
    length(d)
}

pub struct Clearance {
    pub origin: Vec3,
    views: [Mat4; 6],
    /// Free distance per texel, face after face, rows top first.
    free: Vec<f32>,
}

impl Clearance {
    /// From the six views' depth buffers (`EDGE * EDGE` each, face order as `faces`, 1 where
    /// nothing was drawn).
    pub fn from_depths(origin: Vec3, depths: &[f32]) -> Clearance {
        let views = faces(origin);
        let n = (EDGE * EDGE) as usize;
        let mut free = vec![REACH; 6 * n];
        for (face, view) in views.iter().enumerate() {
            let inv = view.inverse();
            for y in 0..EDGE {
                for x in 0..EDGE {
                    let i = face * n + (y * EDGE + x) as usize;
                    let d = depths.get(i).copied().unwrap_or(1.0);
                    if d >= 1.0 {
                        continue;
                    }
                    let ndc = [(x as f32 + 0.5) / EDGE as f32 * 2.0 - 1.0, 1.0 - (y as f32 + 0.5) / EDGE as f32 * 2.0, d];
                    let h = project(&inv, ndc);
                    if h[3].abs() < 1e-12 {
                        continue;
                    }
                    let world = scale([h[0], h[1], h[2]], 1.0 / h[3]);
                    free[i] = length(sub(world, origin)).min(REACH);
                }
            }
        }
        Clearance { origin, views, free }
    }

    /// The free distance along the unit direction `dir`, bilinear between texels.
    pub fn distance(&self, dir: Vec3) -> f32 {
        let a = dir.map(f32::abs);
        let face = if a[0] >= a[1] && a[0] >= a[2] {
            if dir[0] >= 0.0 { 0 } else { 1 }
        } else if a[1] >= a[2] {
            if dir[1] >= 0.0 { 2 } else { 3 }
        } else if dir[2] >= 0.0 {
            4
        } else {
            5
        };
        let c = project(&self.views[face], add(self.origin, dir));
        if c[3] <= 1e-9 {
            return REACH;
        }
        let e = EDGE as f32;
        let tx = ((c[0] / c[3]) * 0.5 + 0.5) * e - 0.5;
        let ty = (0.5 - (c[1] / c[3]) * 0.5) * e - 0.5;
        let (x0, y0) = (tx.floor(), ty.floor());
        let (fx, fy) = (tx - x0, ty - y0);
        let at = |x: f32, y: f32| {
            let (x, y) = (x.clamp(0.0, e - 1.0) as usize, y.clamp(0.0, e - 1.0) as usize);
            self.free[face * (EDGE * EDGE) as usize + y * EDGE as usize + x]
        };
        let top = at(x0, y0) * (1.0 - fx) + at(x0 + 1.0, y0) * fx;
        let bottom = at(x0, y0 + 1.0) * (1.0 - fx) + at(x0 + 1.0, y0 + 1.0) * fx;
        top * (1.0 - fy) + bottom * fy
    }

    /// Grimoire's `hideoutSeenThroughRoom`: some point of a 5 by 5 by 5 grid over `b` is in
    /// front of the room's surfaces, or looks out through an opening.
    pub fn sees(&self, b: &Aabb) -> bool {
        for i in 0..5 {
            for j in 0..5 {
                for k in 0..5 {
                    let t = [i as f32 / 4.0, j as f32 / 4.0, k as f32 / 4.0];
                    let p: Vec3 = std::array::from_fn(|a| b.min[a] + (b.max[a] - b.min[a]) * t[a]);
                    let d = sub(p, self.origin);
                    let dist = length(d);
                    if dist < 1e-4 {
                        return true;
                    }
                    let free = self.distance(scale(d, 1.0 / dist));
                    if free >= REACH * 0.999 || free >= dist * 0.99 {
                        return true;
                    }
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Depths of a closed box room of half size `h` around the origin, rendered by projecting
    /// each texel's ray onto the walls (what the GPU pass draws), with `hole` texels left open.
    fn room(origin: Vec3, h: f32, hole: Option<(usize, u32, u32)>) -> Vec<f32> {
        let views = faces(origin);
        let n = (EDGE * EDGE) as usize;
        let mut out = vec![1.0; 6 * n];
        for (face, view) in views.iter().enumerate() {
            let inv = view.inverse();
            for y in 0..EDGE {
                for x in 0..EDGE {
                    if hole.is_some_and(|(f, hx, hy)| f == face && x.abs_diff(hx) <= 2 && y.abs_diff(hy) <= 2) {
                        continue;
                    }
                    let ndc = [(x as f32 + 0.5) / EDGE as f32 * 2.0 - 1.0, 1.0 - (y as f32 + 0.5) / EDGE as f32 * 2.0, 0.5];
                    let q = project(&inv, ndc);
                    let dir = crate::math::normalize(sub(scale([q[0], q[1], q[2]], 1.0 / q[3]), origin));
                    let t = (0..3).map(|a| h / dir[a].abs().max(1e-6)).fold(f32::MAX, f32::min);
                    let hit = add(origin, scale(dir, t));
                    let c = project(view, hit);
                    out[face * n + (y * EDGE + x) as usize] = c[2] / c[3];
                }
            }
        }
        out
    }

    #[test]
    fn a_closed_room_hides_what_is_outside_and_shows_what_is_in_it() {
        let c = Clearance::from_depths([0.0; 3], &room([0.0; 3], 5.0, None));
        // The free distance straight at a wall is the wall's distance.
        assert!((c.distance([1.0, 0.0, 0.0]) - 5.0).abs() < 0.05, "{}", c.distance([1.0, 0.0, 0.0]));
        assert!((c.distance(crate::math::normalize([1.0, 1.0, 0.0])) - 5.0 * 2f32.sqrt()).abs() < 0.3);
        let outside = Aabb { min: [80.0, -1.0, -1.0], max: [82.0, 1.0, 1.0] };
        assert!(!c.sees(&outside));
        let inside = Aabb { min: [2.0, -1.0, -1.0], max: [3.0, 1.0, 1.0] };
        assert!(c.sees(&inside));
    }

    #[test]
    fn an_opening_lets_the_far_prop_through() {
        // A hole in the +X wall's middle: the view along +X is face 0, its centre texel.
        let c = Clearance::from_depths([0.0; 3], &room([0.0; 3], 5.0, Some((0, 16, 16))));
        assert_eq!(c.distance([1.0, 0.0, 0.0]), REACH);
        let through = Aabb { min: [80.0, -1.0, -1.0], max: [82.0, 1.0, 1.0] };
        assert!(c.sees(&through));
        let behind = Aabb { min: [-82.0, -1.0, -1.0], max: [-80.0, 1.0, 1.0] };
        assert!(!c.sees(&behind));
    }

    #[test]
    fn box_distance_is_zero_inside() {
        let b = Aabb { min: [0.0; 3], max: [1.0; 3] };
        assert_eq!(box_distance(&b, [0.5; 3]), 0.0);
        assert!((box_distance(&b, [4.0, 0.5, 0.5]) - 3.0).abs() < 1e-6);
    }
}
