//! Fixtures and builders shared by the unit tests.
//!
//! The `#[cfg(test)]` modules across the crate use these so the default
//! [`Options`] and toolpath construction live in one place.
#![allow(dead_code)]

use std::f64::consts::FRAC_PI_2;

use crate::Options;
use crate::gcode::{Move, MoveKind};
use crate::toolpath::Point;

pub(crate) const RAW: &str = "G1 X9 Y9 E9 F1200\n";

pub(crate) fn options(mode: &str) -> Options {
    Options {
        tolerance: 0.02,
        mode: mode.into(),
        extrusion_tolerance: 0.01,
        travel: false,
        max_segments: 256,
    }
}

pub(crate) fn travel_options(mode: &str) -> Options {
    Options {
        travel: true,
        ..options(mode)
    }
}

pub(crate) fn move_between(start: Point, end: Point, raw: &'static str) -> Move<'static> {
    Move {
        start,
        end,
        e0: 0.0,
        e1: 0.0,
        relative_e: false,
        feed: 1200.0,
        kind: MoveKind::Feed,
        raw,
    }
}

/// Consecutive chords through `points`.
pub(crate) fn moves(points: &[Point], raw: &'static str) -> Vec<Move<'static>> {
    points
        .windows(2)
        .map(|w| move_between(w[0], w[1], raw))
        .collect()
}

/// `segments + 1` points on a circle from `from` to `to` radians.
pub(crate) fn arc_points(radius: f64, from: f64, to: f64, segments: usize) -> Vec<Point> {
    (0..=segments)
        .map(|i| {
            let a = from + (to - from) * i as f64 / segments as f64;
            Point(radius * a.cos(), radius * a.sin())
        })
        .collect()
}

/// A counterclockwise quarter turn at `radius`, as chords.
pub(crate) fn quarter_arc_moves(
    radius: f64,
    segments: usize,
    raw: &'static str,
) -> Vec<Move<'static>> {
    moves(&arc_points(radius, 0.0, FRAC_PI_2, segments), raw)
}

pub(crate) fn deposit_uniformly(m: &mut [Move<'static>], per_length: f64) {
    let mut e = 0.0;
    for mv in m.iter_mut() {
        mv.e0 = e;
        e += per_length * (mv.end - mv.start).norm();
        mv.e1 = e;
    }
}
