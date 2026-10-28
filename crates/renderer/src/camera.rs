//! A 3D orthographic orbit camera over the G-code volume.
//!
//! The view orbits a target point with yaw and pitch, pans in the screen plane,
//! and zooms by changing pixels per millimetre. Projection is orthographic and
//! depth-tested: view-space Z maps to the clip range through a fixed focus
//! point and radius, so panning the target never pushes the scene out of the
//! depth range. The math is pure so it can be tested on the host.

use std::f32::consts::{FRAC_PI_2, TAU};

use glam::{Mat3, Mat4, Quat, Vec2, Vec3};

/// Smallest and largest allowed zoom, in pixels per millimetre.
const MIN_SCALE: f32 = 1e-4;
const MAX_SCALE: f32 = 1e5;
/// Pitch stays just short of vertical so the view basis never degenerates.
const MAX_PITCH: f32 = FRAC_PI_2 - 1e-3;
/// Default yaw; the eye sits on the -Y side, so world +Y is screen up.
const DEFAULT_YAW: f32 = -FRAC_PI_2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Camera {
    /// Orbit and pan center.
    target: Vec3,
    /// Scene center for the depth range. Panning moves only the target, so the
    /// scene always stays inside the clip volume.
    focus: Vec3,
    /// Bounding-sphere radius of the scene.
    radius: f32,
    yaw: f32,
    pitch: f32,
    scale: f32,
    viewport: Vec2,
}

impl Camera {
    pub(crate) fn new() -> Self {
        Self {
            target: Vec3::ZERO,
            focus: Vec3::ZERO,
            radius: 1.0,
            yaw: DEFAULT_YAW,
            pitch: MAX_PITCH,
            scale: 1.0,
            viewport: Vec2::ONE,
        }
    }

    pub(crate) fn set_viewport(&mut self, size: Vec2) {
        self.viewport = size.max(Vec2::ONE);
    }

    /// Frames `min`..`max` with `margin` device pixels of padding and returns
    /// the view to its default orientation.
    pub(crate) fn fit(&mut self, min: Vec3, max: Vec3, margin: f32) {
        self.target = (min + max) * 0.5;
        self.focus = self.target;
        self.radius = ((max - min) * 0.5).length().max(1e-6);
        self.yaw = DEFAULT_YAW;
        self.pitch = MAX_PITCH;

        // Fit the bounding box at the default orientation: its support along
        // each camera axis is the half-extent dot that axis.
        let half = (max - min) * 0.5;
        let (x, y, _) = self.basis();
        let extent = Vec2::new(half.dot(x.abs()), half.dot(y.abs()));
        let usable = (self.viewport - Vec2::splat(2.0 * margin)).max(Vec2::ONE);

        self.scale = (usable / (2.0 * extent.max(Vec2::splat(1e-9))))
            .min_element()
            .clamp(MIN_SCALE, MAX_SCALE);
    }

    /// Orbits around the target. Pitch is clamped just short of the poles.
    pub(crate) fn orbit(&mut self, delta_yaw: f32, delta_pitch: f32) {
        self.yaw = (self.yaw + delta_yaw).rem_euclid(TAU);
        self.pitch = (self.pitch + delta_pitch).clamp(-MAX_PITCH, MAX_PITCH);
    }

    /// Drags the view by a pointer delta in device pixels.
    pub(crate) fn pan(&mut self, delta: Vec2) {
        let (x, y, _) = self.basis();
        self.target += (x * -delta.x + y * delta.y) / self.scale;
    }

    /// Scales by `factor` while keeping the world point under `cursor` fixed.
    pub(crate) fn zoom_at(&mut self, factor: f32, cursor: Vec2) {
        let before = self.world_at(cursor);
        self.scale = (self.scale * factor).clamp(MIN_SCALE, MAX_SCALE);
        self.target += before - self.world_at(cursor);
    }

    /// The world point on the target plane under a device-pixel position.
    pub(crate) fn world_at(&self, pixel: Vec2) -> Vec3 {
        let (x, y, _) = self.basis();
        let view = (pixel - self.viewport * 0.5) / self.scale * Vec2::new(1.0, -1.0);
        self.target + x * view.x + y * view.y
    }

    /// The world-to-clip matrix.
    ///
    /// X and Y are relative to the target; Z is measured from the focus so the
    /// depth range covers the scene no matter how far the target has panned.
    /// Clip Z grows away from the eye, matching a depth comparison of `less`.
    pub(crate) fn view_projection(&self) -> Mat4 {
        let (x, y, z) = self.basis();
        let sx = 2.0 * self.scale / self.viewport.x;
        let sy = 2.0 * self.scale / self.viewport.y;
        let depth = 0.5 / self.radius.max(1e-6);

        let axes = Mat3::from_cols(x * sx, y * sy, z * -depth).transpose();
        let translation = Vec3::new(
            -sx * x.dot(self.target),
            -sy * y.dot(self.target),
            depth * z.dot(self.focus) + 0.5,
        );

        Mat4::from_mat3_translation(axes, translation)
    }

    /// The unit vector from the scene toward the eye, used to shade surfaces.
    pub(crate) fn eye_direction(&self) -> Vec3 {
        self.basis().2
    }

    /// The camera axes as (right, up, toward the eye).
    fn basis(&self) -> (Vec3, Vec3, Vec3) {
        // Yaw about the world Z axis, then pitch toward the horizon. The axes
        // are the rotated frame's (Y, -X, Z), which keeps the right vector
        // horizontal under yaw.
        let yaw_quat = Quat::from_rotation_z(self.yaw);
        let pitch_quat = Quat::from_rotation_y(FRAC_PI_2 - self.pitch);
        let orientation = yaw_quat * pitch_quat;

        (
            orientation * Vec3::Y,
            orientation * Vec3::NEG_X,
            orientation * Vec3::Z,
        )
    }
}

#[cfg(test)]
mod tests {
    use glam::{Vec2, Vec3};
    use rstest::{fixture, rstest};

    use super::*;

    /// An 800x600 viewport, the default for the demo page.
    #[fixture]
    fn camera() -> Camera {
        let mut camera = Camera::new();
        camera.set_viewport(Vec2::new(800.0, 600.0));
        camera
    }

    #[test]
    fn eye_direction_points_from_the_scene_toward_the_eye() {
        let mut camera = camera();
        camera.fit(Vec3::ZERO, Vec3::new(100.0, 100.0, 10.0), 0.0);

        // The default view looks down, so the eye sits above the scene.
        let eye = camera.eye_direction();
        assert!((eye.length() - 1.0).abs() < 1e-6, "{eye:?}");
        assert!((eye.z - 1.0).abs() < 1e-3, "{eye:?}");

        camera.orbit(0.0, -MAX_PITCH);

        // Level with the scene, the eye direction is horizontal.
        assert!(camera.eye_direction().z.abs() < 1e-3);
    }

    #[test]
    fn fit_centers_the_target_and_frames_the_bounds() {
        // given
        let mut camera = camera();

        // when
        camera.fit(Vec3::ZERO, Vec3::new(100.0, 50.0, 10.0), 20.0);

        // then the target is the box center and the width fits the viewport
        assert_eq!(camera.target, Vec3::new(50.0, 25.0, 5.0));
        assert_eq!(camera.focus, camera.target);
        assert!((camera.scale - 7.6).abs() < 1e-3, "{}", camera.scale);
    }

    #[test]
    fn the_default_view_is_north_up_and_looks_down() {
        let mut camera = camera();
        camera.fit(Vec3::ZERO, Vec3::new(100.0, 100.0, 10.0), 0.0);

        let north = camera
            .view_projection()
            .project_point3(Vec3::new(50.0, 60.0, 5.0));

        assert!(north.x.abs() < 1e-6, "{north:?}");
        assert!(north.y > 0.0, "{north:?}");
    }

    #[test]
    fn fit_keeps_a_smaller_bounds_inside_the_viewport() {
        let mut camera = camera();
        camera.fit(Vec3::new(10.0, 10.0, 0.0), Vec3::new(11.0, 10.5, 0.5), 20.0);

        let corners = [
            Vec3::new(10.0, 10.0, 0.0),
            Vec3::new(10.0, 10.5, 0.0),
            Vec3::new(11.0, 10.0, 0.5),
            Vec3::new(11.0, 10.5, 0.5),
        ];
        for corner in corners {
            let clip = camera.view_projection().project_point3(corner);
            assert!(clip.x.abs() <= 1.0 && clip.y.abs() <= 1.0, "{clip:?}");
        }
    }

    #[rstest]
    #[case::right(10.0, 0.0)]
    #[case::down(0.0, 20.0)]
    fn pan_follows_the_pointer(#[case] dx: f32, #[case] dy: f32) {
        // given
        let mut camera = camera();
        camera.fit(Vec3::ZERO, Vec3::new(100.0, 100.0, 10.0), 0.0);
        let pixel = Vec2::new(500.0, 200.0);
        let before = camera.world_at(pixel);

        // when
        camera.pan(Vec2::new(dx, dy));

        // then the world point lands under the moved pointer
        let after = camera.world_at(pixel + Vec2::new(dx, dy));
        assert!((before - after).length() < 1e-4, "{before:?} {after:?}");
    }

    #[test]
    fn pan_follows_the_pointer_in_an_orbited_view() {
        // given
        let mut camera = camera();
        camera.fit(Vec3::ZERO, Vec3::new(100.0, 100.0, 10.0), 0.0);
        camera.orbit(0.9, -0.6);
        let pixel = Vec2::new(500.0, 200.0);
        let before = camera.world_at(pixel);

        // when
        camera.pan(Vec2::new(30.0, -15.0));

        // then
        let after = camera.world_at(pixel + Vec2::new(30.0, -15.0));
        assert!((before - after).length() < 1e-4, "{before:?} {after:?}");
    }

    #[test]
    fn zoom_keeps_the_world_point_under_the_cursor() {
        // given
        let mut camera = camera();
        camera.fit(Vec3::ZERO, Vec3::new(100.0, 100.0, 10.0), 0.0);
        camera.orbit(0.4, -0.3);
        let cursor = Vec2::new(600.0, 150.0);
        let before = camera.world_at(cursor);

        // when
        camera.zoom_at(2.5, cursor);

        // then
        let after = camera.world_at(cursor);
        assert!((before - after).length() < 1e-4, "{before:?} {after:?}");
    }

    #[test]
    fn world_at_inverts_the_projection() {
        let mut camera = camera();
        camera.fit(Vec3::new(-10.0, 5.0, 0.0), Vec3::new(30.0, 25.0, 4.0), 0.0);
        camera.orbit(0.7, -0.5);
        let pixel = Vec2::new(210.0, 430.0);

        let clip = camera
            .view_projection()
            .project_point3(camera.world_at(pixel));

        // The viewport spans clip -1..1, so a pixel maps to
        // `pixel / (viewport / 2) - 1` on X and its negation on Y.
        let expected = Vec2::new(pixel.x / 400.0 - 1.0, 1.0 - pixel.y / 300.0);
        assert!((clip.x - expected.x).abs() < 1e-4, "{clip:?}");
        assert!((clip.y - expected.y).abs() < 1e-4, "{clip:?}");
    }

    #[test]
    fn orbit_keeps_the_target_fixed() {
        let mut camera = camera();
        camera.fit(Vec3::ZERO, Vec3::new(100.0, 100.0, 10.0), 0.0);
        let target = camera.target;

        camera.orbit(1.2, -0.8);

        assert_eq!(camera.target, target);
    }

    #[test]
    fn orbit_clamps_the_pitch_short_of_the_poles() {
        let mut camera = camera();

        camera.orbit(0.0, 10.0);
        assert!((camera.pitch - MAX_PITCH).abs() < 1e-6);

        camera.orbit(0.0, -20.0);
        assert!((camera.pitch + MAX_PITCH).abs() < 1e-6);
    }

    #[test]
    fn the_focus_keeps_the_scene_inside_the_depth_range() {
        let mut camera = camera();
        camera.fit(Vec3::ZERO, Vec3::new(100.0, 100.0, 10.0), 0.0);

        // Pan far away from the scene; the depth range still covers it.
        camera.pan(Vec2::new(10_000.0, -8_000.0));

        let clip = camera.view_projection().project_point3(camera.focus);
        assert!((clip.z - 0.5).abs() < 1e-4, "{clip:?}");
    }
}
