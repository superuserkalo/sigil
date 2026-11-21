//! The orbit camera: yaw and pitch around a target, a distance, and pan in the view plane.
//! `framing` fits a model's posed bounds into a portrait-style view, the way Grimoire's viewer
//! frames a hero (40 degree field of view, a three-quarter turn from the front).

use crate::math::{Aabb, Mat4, Vec3, add, cross, normalize, scale, sub};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrbitCamera {
    /// Radians around +Y; 0 looks at the hero's front (glTF +Z).
    pub yaw: f32,
    /// Radians above the horizon.
    pub pitch: f32,
    pub distance: f32,
    pub target: Vec3,
    pub fov_y: f32,
    /// Distance limits, set by framing.
    pub min_distance: f32,
    pub max_distance: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        OrbitCamera { yaw: 0.0, pitch: 0.0, distance: 3.2, target: [0.0; 3], fov_y: 40f32.to_radians(), min_distance: 0.3, max_distance: 30.0 }
    }
}

/// The three-quarter turn a framed hero starts at.
pub const HOME_YAW: f32 = 0.42;
pub const HOME_PITCH: f32 = 0.06;
pub const PITCH_LIMIT: f32 = 1.45;

impl OrbitCamera {
    pub fn eye(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        add(self.target, scale([sy * cp, sp, cy * cp], self.distance))
    }

    pub fn view(&self) -> Mat4 {
        Mat4::look_at(self.eye(), self.target, [0.0, 1.0, 0.0])
    }

    pub fn near_far(&self, extent: f32) -> (f32, f32) {
        let near = (self.distance * 0.02).max(0.005);
        (near, self.distance + extent * 4.0 + 10.0)
    }

    /// Drag by `dx`, `dy` in points: half a turn across 400 points.
    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx * (std::f32::consts::PI / 400.0);
        self.pitch = (self.pitch + dy * (std::f32::consts::PI / 400.0)).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }

    /// Zoom by a factor (below 1 moves in).
    pub fn zoom(&mut self, factor: f32) {
        if factor.is_finite() && factor > 0.0 {
            self.distance = (self.distance * factor).clamp(self.min_distance, self.max_distance);
        }
    }

    /// Pan by `dx`, `dy` points in a view `height` points tall: the target moves with the
    /// pointer at the target's depth.
    pub fn pan(&mut self, dx: f32, dy: f32, height: f32) {
        let eye = self.eye();
        let forward = normalize(sub(self.target, eye));
        let right = normalize(cross(forward, [0.0, 1.0, 0.0]));
        let up = cross(right, forward);
        let per_point = 2.0 * self.distance * (self.fov_y * 0.5).tan() / height.max(1.0);
        self.target = add(self.target, add(scale(right, -dx * per_point), scale(up, dy * per_point)));
    }

    /// Fits `bounds` at `aspect` (width over height) with a margin, from the home angle.
    pub fn framing(bounds: &Aabb, aspect: f32) -> OrbitCamera {
        let mut cam = OrbitCamera { yaw: HOME_YAW, pitch: HOME_PITCH, ..Default::default() };
        if bounds.is_empty() {
            return cam;
        }
        let size = bounds.size();
        let center = bounds.center();
        let height = size[1].max(1e-3);
        let width = size[0].max(size[2]).max(1e-3);
        let half_v = (cam.fov_y * 0.5).tan();
        let half_h = half_v * aspect.max(0.1);
        let fit_h = height * 0.5 * 1.12 / half_v;
        let fit_w = width * 0.5 * 1.25 / half_h;
        cam.distance = fit_h.max(fit_w) + width * 0.5;
        cam.target = center;
        cam.min_distance = (height * 0.15).max(0.05);
        cam.max_distance = cam.distance * 4.0;
        cam
    }

    /// The camera's +Z axis in world space (from the target toward the eye).
    pub fn back(&self) -> Vec3 {
        normalize(sub(self.eye(), self.target))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framing_puts_the_whole_box_in_view() {
        let b = Aabb { min: [-0.3, 0.0, -0.2], max: [0.3, 1.9, 0.2] };
        let cam = OrbitCamera::framing(&b, 1.6);
        let view = cam.view();
        let proj = Mat4::perspective(cam.fov_y, 1.6, 0.05, 100.0);
        let vp = proj.mul(&view);
        for corner in [[-0.3, 0.0, -0.2], [0.3, 1.9, 0.2], [0.3, 0.0, 0.2], [-0.3, 1.9, -0.2]] {
            let c = vp.0;
            let p: [f32; 4] = std::array::from_fn(|r| c[0][r] * corner[0] + c[1][r] * corner[1] + c[2][r] * corner[2] + c[3][r]);
            assert!((p[0] / p[3]).abs() <= 1.0 && (p[1] / p[3]).abs() <= 1.0, "{corner:?} is outside: {p:?}");
        }
    }

    #[test]
    fn pitch_stops_short_of_the_poles() {
        let mut cam = OrbitCamera::default();
        cam.orbit(0.0, 10_000.0);
        assert!(cam.pitch < std::f32::consts::FRAC_PI_2);
    }

    #[test]
    fn zoom_respects_the_limits() {
        let mut cam = OrbitCamera { min_distance: 1.0, max_distance: 2.0, distance: 1.5, ..Default::default() };
        cam.zoom(0.01);
        assert_eq!(cam.distance, 1.0);
        cam.zoom(100.0);
        assert_eq!(cam.distance, 2.0);
    }
}
