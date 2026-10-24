//! Planar point arithmetic shared by the geometry fits.
//!
//! Arc circumcenters, spline control points, and fit validation all work in
//! the XY plane, so they reduce to the same vector operations and the same
//! snapping to the emitter's control precision.

use std::ops::{Add, Mul, Sub};

/// A point in the XY plane, in millimetres.
///
/// Carries the vector arithmetic (`Add`, `Sub`, scalar `Mul`) and the
/// dot/cross/norm products the fitting math is written in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Point(pub(crate) f64, pub(crate) f64);

impl Add for Point {
    type Output = Self;

    fn add(self, b: Self) -> Self {
        Self(self.0 + b.0, self.1 + b.1)
    }
}

impl Sub for Point {
    type Output = Self;

    fn sub(self, b: Self) -> Self {
        Self(self.0 - b.0, self.1 - b.1)
    }
}

impl Mul<f64> for Point {
    type Output = Self;

    fn mul(self, b: f64) -> Self {
        Self(self.0 * b, self.1 * b)
    }
}

impl Point {
    fn round(v: f64, step: f64) -> f64 {
        (v / step).round() * step
    }

    #[inline(always)]
    pub(crate) fn dot(self, b: Self) -> f64 {
        self.0 * b.0 + self.1 * b.1
    }

    #[inline(always)]
    pub(crate) fn cross(self, b: Self) -> f64 {
        self.0 * b.1 - self.1 * b.0
    }

    #[inline(always)]
    pub(crate) fn norm(self) -> f64 {
        self.0.hypot(self.1)
    }

    /// Snaps both coordinates to a multiple of `step`. Geometry code uses this
    /// to build fitted control points at the precision the emitter prints, so
    /// validation and output describe the same curve.
    #[inline(always)]
    pub(crate) fn snapped(self, step: f64) -> Self {
        Self(Self::round(self.0, step), Self::round(self.1, step))
    }

    #[inline(always)]
    pub(crate) fn interp(a: Self, b: Self, t: f64) -> Self {
        a * (1.0 - t) + b * t
    }
}
