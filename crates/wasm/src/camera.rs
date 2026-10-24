//! A 3D orthographic orbit camera over the G-code volume.
//!
//! The view orbits a target point with yaw and pitch, pans in the screen plane,
//! and zooms by changing pixels per millimetre. Projection is orthographic and
//! depth-tested: view-space Z maps to the clip range through a fixed focus
//! point and radius, so panning the target never pushes the scene out of the
//! depth range. The math is pure so it can be tested on the host.

use std::f64::consts::{FRAC_PI_2, TAU};

/// Smallest and largest allowed zoom, in pixels per millimetre.
const MIN_SCALE: f64 = 1e-4;
const MAX_SCALE: f64 = 1e5;
/// Pitch stays just short of vertical so the view basis never degenerates.
const MAX_PITCH: f64 = FRAC_PI_2 - 1e-3;
/// Default yaw; the eye sits on the -Y side, so world +Y is screen up.
const DEFAULT_YAW: f64 = -FRAC_PI_2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Camera {
    /// Orbit and pan center.
    target: [f64; 3],
    /// Scene center for the depth range. Panning moves only the target, so the
    /// scene always stays inside the clip volume.
    focus: [f64; 3],
    /// Bounding-sphere radius of the scene.
    radius: f64,
    yaw: f64,
    pitch: f64,
    scale: f64,
    viewport: [f64; 2],
}

impl Camera {
    pub(crate) fn new() -> Self {
        Self {
            target: [0.0; 3],
            focus: [0.0; 3],
            radius: 1.0,
            yaw: DEFAULT_YAW,
            pitch: MAX_PITCH,
            scale: 1.0,
            viewport: [1.0, 1.0],
        }
    }

    pub(crate) fn set_viewport(&mut self, width: f64, height: f64) {
        self.viewport = [width.max(1.0), height.max(1.0)];
    }

    /// Frames `min`..`max` with `margin` device pixels of padding and returns
    /// the view to its default orientation.
    pub(crate) fn fit(&mut self, min: [f64; 3], max: [f64; 3], margin: f64) {
        self.target = [
            (min[0] + max[0]) / 2.0,
            (min[1] + max[1]) / 2.0,
            (min[2] + max[2]) / 2.0,
        ];
        self.focus = self.target;
        self.radius = (0..3)
            .map(|axis| ((max[axis] - min[axis]) / 2.0).powi(2))
            .sum::<f64>()
            .sqrt()
            .max(1e-6);
        self.yaw = DEFAULT_YAW;
        self.pitch = MAX_PITCH;

        // Fit the bounding box at the default orientation: its support along
        // each camera axis is the half-extent dot that axis.
        let half = [
            (max[0] - min[0]) / 2.0,
            (max[1] - min[1]) / 2.0,
            (max[2] - min[2]) / 2.0,
        ];
        let (x, y, _) = self.basis();
        let extent = [
            half[0] * x[0].abs() + half[1] * x[1].abs() + half[2] * x[2].abs(),
            half[0] * y[0].abs() + half[1] * y[1].abs() + half[2] * y[2].abs(),
        ];
        let usable = [
            (self.viewport[0] - 2.0 * margin).max(1.0),
            (self.viewport[1] - 2.0 * margin).max(1.0),
        ];

        self.scale = (usable[0] / (2.0 * extent[0].max(1e-9)))
            .min(usable[1] / (2.0 * extent[1].max(1e-9)))
            .clamp(MIN_SCALE, MAX_SCALE);
    }

    /// Orbits around the target. Pitch is clamped just short of the poles.
    pub(crate) fn orbit(&mut self, delta_yaw: f64, delta_pitch: f64) {
        self.yaw = (self.yaw + delta_yaw).rem_euclid(TAU);
        self.pitch = (self.pitch + delta_pitch).clamp(-MAX_PITCH, MAX_PITCH);
    }

    /// Drags the view by a pointer delta in device pixels.
    pub(crate) fn pan(&mut self, dx: f64, dy: f64) {
        let (x, y, _) = self.basis();
        let per_pixel = 1.0 / self.scale;

        for axis in 0..3 {
            self.target[axis] += (-x[axis] * dx + y[axis] * dy) * per_pixel;
        }
    }

    /// Scales by `factor` while keeping the world point under `cursor` fixed.
    pub(crate) fn zoom_at(&mut self, factor: f64, cursor: [f64; 2]) {
        let before = self.world_at(cursor);
        self.scale = (self.scale * factor).clamp(MIN_SCALE, MAX_SCALE);
        let after = self.world_at(cursor);

        for axis in 0..3 {
            self.target[axis] += before[axis] - after[axis];
        }
    }

    /// The world point on the target plane under a device-pixel position.
    pub(crate) fn world_at(&self, pixel: [f64; 2]) -> [f64; 3] {
        let (x, y, _) = self.basis();
        let u = (pixel[0] - self.viewport[0] / 2.0) / self.scale;
        let v = -(pixel[1] - self.viewport[1] / 2.0) / self.scale;

        [
            self.target[0] + x[0] * u + y[0] * v,
            self.target[1] + x[1] * u + y[1] * v,
            self.target[2] + x[2] * u + y[2] * v,
        ]
    }

    /// The world-to-clip matrix in column-major order.
    ///
    /// X and Y are relative to the target; Z is measured from the focus so the
    /// depth range covers the scene no matter how far the target has panned.
    /// Clip Z grows away from the eye, matching a depth comparison of `less`.
    pub(crate) fn matrix(&self) -> [f32; 16] {
        let (x, y, z) = self.basis();
        let sx = 2.0 * self.scale / self.viewport[0];
        let sy = 2.0 * self.scale / self.viewport[1];
        let depth = 0.5 / self.radius.max(1e-6);

        let tx = dot(x, self.target);
        let ty = dot(y, self.target);
        let fz = dot(z, self.focus);

        [
            (sx * x[0]) as f32,
            (sy * y[0]) as f32,
            (-depth * z[0]) as f32,
            0.0,
            (sx * x[1]) as f32,
            (sy * y[1]) as f32,
            (-depth * z[1]) as f32,
            0.0,
            (sx * x[2]) as f32,
            (sy * y[2]) as f32,
            (-depth * z[2]) as f32,
            0.0,
            (-sx * tx) as f32,
            (-sy * ty) as f32,
            (depth * fz + 0.5) as f32,
            1.0,
        ]
    }

    /// The unit vector from the scene toward the eye, used to shade tubes.
    pub(crate) fn eye_direction(&self) -> [f32; 3] {
        let (_, _, z) = self.basis();
        [z[0] as f32, z[1] as f32, z[2] as f32]
    }

    /// The camera axes as (right, up, toward the eye).
    fn basis(&self) -> ([f64; 3], [f64; 3], [f64; 3]) {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();

        // Toward the eye; pitch keeps it off the world Z axis.
        let z = [cos_pitch * cos_yaw, cos_pitch * sin_yaw, sin_pitch];
        let x = normalize([-z[1], z[0], 0.0]);
        let y = cross(z, x);

        (x, y, z)
    }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: [f64; 3]) -> [f64; 3] {
    let length = dot(v, v).sqrt();

    [v[0] / length, v[1] / length, v[2] / length]
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::{fixture, rstest};

    /// An 800x600 viewport, the default for the demo page.
    #[fixture]
    fn camera() -> Camera {
        let mut camera = Camera::new();
        camera.set_viewport(800.0, 600.0);
        camera
    }

    /// Applies the camera matrix to a world point.
    fn project(camera: &Camera, point: [f64; 3]) -> [f32; 3] {
        let m = camera.matrix();
        let (x, y, z) = (point[0] as f32, point[1] as f32, point[2] as f32);

        [
            m[0] * x + m[4] * y + m[8] * z + m[12],
            m[1] * x + m[5] * y + m[9] * z + m[13],
            m[2] * x + m[6] * y + m[10] * z + m[14],
        ]
    }

    #[test]
    fn eye_direction_points_from_the_scene_toward_the_eye() {
        let mut camera = camera();
        camera.fit([0.0, 0.0, 0.0], [100.0, 100.0, 10.0], 0.0);

        // The default view looks down, so the eye sits above the scene.
        let eye = camera.eye_direction();
        let length = (eye[0] * eye[0] + eye[1] * eye[1] + eye[2] * eye[2]).sqrt();
        assert!((length - 1.0).abs() < 1e-6, "{eye:?}");
        assert!((eye[2] - 1.0).abs() < 1e-3, "{eye:?}");

        camera.orbit(0.0, -MAX_PITCH);

        // Level with the scene, the eye direction is horizontal.
        assert!(camera.eye_direction()[2].abs() < 1e-3);
    }

    #[test]
    fn fit_centers_the_target_and_frames_the_bounds() {
        // given
        let mut camera = camera();

        // when
        camera.fit([0.0, 0.0, 0.0], [100.0, 50.0, 10.0], 20.0);

        // then the target is the box center and the width fits the viewport
        assert_eq!(camera.target, [50.0, 25.0, 5.0]);
        assert_eq!(camera.focus, camera.target);
        assert!((camera.scale - 7.6).abs() < 1e-3, "{}", camera.scale);
    }

    #[test]
    fn the_default_view_is_north_up_and_looks_down() {
        let mut camera = camera();
        camera.fit([0.0, 0.0, 0.0], [100.0, 100.0, 10.0], 0.0);

        let north = project(&camera, [50.0, 60.0, 5.0]);

        assert!(north[0].abs() < 1e-6, "{north:?}");
        assert!(north[1] > 0.0, "{north:?}");
    }

    #[test]
    fn fit_keeps_a_smaller_bounds_inside_the_viewport() {
        let mut camera = camera();
        camera.fit([10.0, 10.0, 0.0], [11.0, 10.5, 0.5], 20.0);

        let corners = [
            [10.0, 10.0, 0.0],
            [10.0, 10.5, 0.0],
            [11.0, 10.0, 0.5],
            [11.0, 10.5, 0.5],
        ];
        for corner in corners {
            let clip = project(&camera, corner);
            assert!(clip[0].abs() <= 1.0 && clip[1].abs() <= 1.0, "{clip:?}");
        }
    }

    #[rstest]
    #[case::right(10.0, 0.0)]
    #[case::down(0.0, 20.0)]
    fn pan_follows_the_pointer(#[case] dx: f64, #[case] dy: f64) {
        // given
        let mut camera = camera();
        camera.fit([0.0, 0.0, 0.0], [100.0, 100.0, 10.0], 0.0);
        let pixel = [500.0, 200.0];
        let before = camera.world_at(pixel);

        // when
        camera.pan(dx, dy);

        // then the world point lands under the moved pointer
        let after = camera.world_at([pixel[0] + dx, pixel[1] + dy]);
        for axis in 0..3 {
            assert!(
                (before[axis] - after[axis]).abs() < 1e-9,
                "{before:?} {after:?}"
            );
        }
    }

    #[test]
    fn pan_follows_the_pointer_in_an_orbited_view() {
        // given
        let mut camera = camera();
        camera.fit([0.0, 0.0, 0.0], [100.0, 100.0, 10.0], 0.0);
        camera.orbit(0.9, -0.6);
        let pixel = [500.0, 200.0];
        let before = camera.world_at(pixel);

        // when
        camera.pan(30.0, -15.0);

        // then
        let after = camera.world_at([pixel[0] + 30.0, pixel[1] - 15.0]);
        for axis in 0..3 {
            assert!(
                (before[axis] - after[axis]).abs() < 1e-9,
                "{before:?} {after:?}"
            );
        }
    }

    #[test]
    fn zoom_keeps_the_world_point_under_the_cursor() {
        // given
        let mut camera = camera();
        camera.fit([0.0, 0.0, 0.0], [100.0, 100.0, 10.0], 0.0);
        camera.orbit(0.4, -0.3);
        let cursor = [600.0, 150.0];
        let before = camera.world_at(cursor);

        // when
        camera.zoom_at(2.5, cursor);

        // then
        let after = camera.world_at(cursor);
        for axis in 0..3 {
            assert!(
                (before[axis] - after[axis]).abs() < 1e-9,
                "{before:?} {after:?}"
            );
        }
    }

    #[test]
    fn world_at_inverts_the_projection() {
        let mut camera = camera();
        camera.fit([-10.0, 5.0, 0.0], [30.0, 25.0, 4.0], 0.0);
        camera.orbit(0.7, -0.5);
        let pixel = [210.0, 430.0];

        let clip = project(&camera, camera.world_at(pixel));

        // The viewport spans clip -1..1, so a pixel maps to
        // `pixel / (viewport / 2) - 1` on X and its negation on Y.
        let expected = [pixel[0] / 400.0 - 1.0, 1.0 - pixel[1] / 300.0];
        assert!((clip[0] as f64 - expected[0]).abs() < 1e-5, "{clip:?}");
        assert!((clip[1] as f64 - expected[1]).abs() < 1e-5, "{clip:?}");
    }

    #[test]
    fn orbit_keeps_the_target_fixed() {
        let mut camera = camera();
        camera.fit([0.0, 0.0, 0.0], [100.0, 100.0, 10.0], 0.0);
        let target = camera.target;

        camera.orbit(1.2, -0.8);

        assert_eq!(camera.target, target);
    }

    #[test]
    fn orbit_clamps_the_pitch_short_of_the_poles() {
        let mut camera = camera();

        camera.orbit(0.0, 10.0);
        assert!((camera.pitch - MAX_PITCH).abs() < 1e-12);

        camera.orbit(0.0, -20.0);
        assert!((camera.pitch + MAX_PITCH).abs() < 1e-12);
    }

    #[test]
    fn the_focus_keeps_the_scene_inside_the_depth_range() {
        let mut camera = camera();
        camera.fit([0.0, 0.0, 0.0], [100.0, 100.0, 10.0], 0.0);

        // Pan far away from the scene; the depth range still covers it.
        camera.pan(10_000.0, -8_000.0);

        let clip = project(&camera, camera.focus);
        assert!((clip[2] - 0.5).abs() < 1e-5, "{clip:?}");
    }
}
