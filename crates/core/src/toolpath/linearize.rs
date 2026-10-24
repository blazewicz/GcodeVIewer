//! Linearizes a curved motion back into a polyline.
//!
//! [`linearize_curve`] is the inverse of the fitting in [`super::arc`] and
//! [`super::spline`]: welding replaces a window of linear moves with an arc or
//! cubic, and linearization samples that curve back into the straight pieces a
//! renderer or measurement pass consumes. Both directions share the same chord
//! deviation bound (`radius * angle^2 / 8`) and the same offset conventions, so
//! they live together and are cross-tested.

use std::f64::consts::{PI, TAU};

use crate::gcode::{CurveKind, MoveKind};

/// Upper bound on segments for one arc.
const MAX_ARC_SEGMENTS: usize = 1024;

/// Linearizes the motion `kind` with optional `curve` into a polyline.
///
/// The first point is `start` and the last is `end`, so consecutive pairs are
/// the straight pieces of the path. Linear motions, curves without offsets, and
/// degenerate shapes fall back to the single `start`-`end` chord. Arc Z follows
/// the sweep linearly, so a helical move stays a helix; spline Z follows the
/// parameter linearly.
///
/// `tolerance` is the target chord deviation in millimetres and
/// `spline_samples` the Bézier sample count.
pub fn linearize_curve(
    kind: MoveKind,
    curve: Option<CurveKind>,
    start: [f64; 3],
    end: [f64; 3],
    tolerance: f64,
    spline_samples: usize,
) -> Vec<[f64; 3]> {
    match (kind, curve) {
        (MoveKind::Arc { clockwise }, Some(CurveKind::Arc { i, j, r })) => {
            linearize_arc(start, end, i, j, r, clockwise, tolerance)
        }

        (MoveKind::Spline, Some(CurveKind::Spline { i, j, p, q })) => {
            linearize_spline(start, end, i, j, p, q, spline_samples)
        }

        _ => vec![start, end],
    }
}

/// Linearizes one arc. The center comes from the `i`/`j` offsets, or from the
/// radius form when `r` is present; a degenerate shape falls back to a chord.
fn linearize_arc(
    start: [f64; 3],
    end: [f64; 3],
    i: f64,
    j: f64,
    r: Option<f64>,
    clockwise: bool,
    tolerance: f64,
) -> Vec<[f64; 3]> {
    let center = match r {
        Some(radius) => arc_center_from_radius(start, end, radius, clockwise),
        None => Some((start[0] + i, start[1] + j)),
    };

    let Some((cx, cy)) = center else {
        return vec![start, end];
    };

    let radius = (start[0] - cx).hypot(start[1] - cy);
    if radius < 1e-12 {
        return vec![start, end];
    }

    let a0 = (start[1] - cy).atan2(start[0] - cx);
    let a1 = (end[1] - cy).atan2(end[0] - cx);
    let sweep = sweep_between(a0, a1, clockwise);

    // Chord deviation of a step is about radius * step^2 / 8, so this keeps it
    // under the tolerance while the clamp bounds the worst case.
    let steps = ((sweep.abs() * (radius / (8.0 * tolerance)).sqrt()).ceil() as usize)
        .clamp(2, MAX_ARC_SEGMENTS);

    let mut points = Vec::with_capacity(steps + 1);
    points.push(start);

    for step in 1..=steps {
        let t = step as f64 / steps as f64;
        let angle = a0 + sweep * t;
        points.push([
            cx + radius * angle.cos(),
            cy + radius * angle.sin(),
            start[2] + (end[2] - start[2]) * t,
        ]);
    }

    points
}

/// Linearizes one cubic Bézier with control points offset from the start
/// (`i`/`j`) and from the end (`p`/`q`).
fn linearize_spline(
    start: [f64; 3],
    end: [f64; 3],
    i: f64,
    j: f64,
    p: f64,
    q: f64,
    samples: usize,
) -> Vec<[f64; 3]> {
    let c1 = [start[0] + i, start[1] + j];
    let c2 = [end[0] + p, end[1] + q];

    let mut points = Vec::with_capacity(samples + 1);
    points.push(start);

    for step in 1..=samples {
        let t = step as f64 / samples as f64;
        let u = 1.0 - t;
        points.push([
            u * u * u * start[0]
                + 3.0 * u * u * t * c1[0]
                + 3.0 * u * t * t * c2[0]
                + t * t * t * end[0],
            u * u * u * start[1]
                + 3.0 * u * u * t * c1[1]
                + 3.0 * u * t * t * c2[1]
                + t * t * t * end[1],
            start[2] + (end[2] - start[2]) * t,
        ]);
    }

    points
}

/// The signed sweep from `a0` to `a1`, following the arc's direction. Equal
/// angles mean the endpoints coincide, which G-code defines as a full turn.
fn sweep_between(a0: f64, a1: f64, clockwise: bool) -> f64 {
    let sweep = if clockwise {
        -(a0 - a1).rem_euclid(TAU)
    } else {
        (a1 - a0).rem_euclid(TAU)
    };

    if sweep == 0.0 {
        if clockwise { -TAU } else { TAU }
    } else {
        sweep
    }
}

/// Resolves the radius form. The center lies at `r` from both endpoints; two
/// candidates exist, and the commanded arc is the one no larger than a
/// semicircle, matching Marlin's radius interpretation.
fn arc_center_from_radius(
    start: [f64; 3],
    end: [f64; 3],
    radius: f64,
    clockwise: bool,
) -> Option<(f64, f64)> {
    let radius = radius.abs();
    let (x0, y0) = (start[0], start[1]);
    let (x1, y1) = (end[0], end[1]);
    let (dx, dy) = (x1 - x0, y1 - y0);
    let chord = dx.hypot(dy);

    if chord < 1e-12 || radius < chord / 2.0 {
        return None;
    }

    let (mx, my) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let height = (radius * radius - (chord / 2.0).powi(2)).sqrt();
    let (px, py) = (-dy / chord, dx / chord);

    [
        (mx + px * height, my + py * height),
        (mx - px * height, my - py * height),
    ]
    .into_iter()
    .find(|&(cx, cy)| {
        let a0 = (y0 - cy).atan2(x0 - cx);
        let a1 = (y1 - cy).atan2(x1 - cx);

        sweep_between(a0, a1, clockwise).abs() <= PI + 1e-9
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    /// The points projected onto the XY plane.
    fn xy(points: &[[f64; 3]]) -> Vec<[f64; 2]> {
        points.iter().map(|p| [p[0], p[1]]).collect()
    }

    #[test]
    fn a_linear_move_is_a_single_chord() {
        let points = linearize_curve(
            MoveKind::Feed,
            None,
            [1.0, 2.0, 0.0],
            [4.0, 6.0, 0.5],
            0.05,
            64,
        );

        assert_eq!(points, vec![[1.0, 2.0, 0.0], [4.0, 6.0, 0.5]]);
    }

    #[test]
    fn a_curve_without_offsets_falls_back_to_a_chord() {
        // An implicit arc line inherits the kind but carries no offsets, so the
        // only shape left to draw is the straight distance between endpoints.
        let points = linearize_curve(
            MoveKind::Arc { clockwise: true },
            None,
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            0.05,
            64,
        );

        assert_eq!(points, vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0]]);
    }

    #[test]
    fn a_quarter_arc_follows_its_circle() {
        // A counterclockwise quarter turn from (20, 0) to (0, 20) centered at
        // the origin: the offsets point back at the start's center.
        let points = linearize_curve(
            MoveKind::Arc { clockwise: false },
            Some(CurveKind::Arc {
                i: -20.0,
                j: 0.0,
                r: None,
            }),
            [20.0, 0.0, 0.0],
            [0.0, 20.0, 0.0],
            0.05,
            64,
        );

        assert!(points.len() > 3);
        for point in xy(&points) {
            assert!((point[0].hypot(point[1]) - 20.0).abs() < 1e-3, "{point:?}");
        }
        assert_eq!(points[0], [20.0, 0.0, 0.0]);
        let end = *points.last().unwrap();
        assert!(
            (end[0] - 0.0).abs() < 1e-3 && (end[1] - 20.0).abs() < 1e-3,
            "{end:?}"
        );
    }

    #[test]
    fn coincident_arc_endpoints_sweep_a_full_circle() {
        // G2/G3 with equal endpoints and center offsets is a full circle.
        let points = linearize_curve(
            MoveKind::Arc { clockwise: false },
            Some(CurveKind::Arc {
                i: 0.0,
                j: 10.0,
                r: None,
            }),
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            0.05,
            64,
        );
        let xs: Vec<f64> = xy(&points).iter().map(|p| p[0]).collect();
        let ys: Vec<f64> = xy(&points).iter().map(|p| p[1]).collect();
        let min_x = xs.iter().copied().fold(f64::INFINITY, f64::min);
        let max_x = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let min_y = ys.iter().copied().fold(f64::INFINITY, f64::min);
        let max_y = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);

        assert!((min_x + 10.0).abs() < 1e-3, "{min_x}");
        assert!((max_x - 10.0).abs() < 1e-3, "{max_x}");
        assert!(min_y.abs() < 1e-3, "{min_y}");
        assert!((max_y - 20.0).abs() < 1e-3, "{max_y}");
    }

    #[test]
    fn a_helical_arc_interpolates_z_along_the_sweep() {
        // The quarter turn rises 1 mm; the point at the 45 degree mark must sit
        // halfway up.
        let points = linearize_curve(
            MoveKind::Arc { clockwise: false },
            Some(CurveKind::Arc {
                i: -20.0,
                j: 0.0,
                r: None,
            }),
            [20.0, 0.0, 0.0],
            [0.0, 20.0, 1.0],
            0.05,
            64,
        );
        let middle = 20.0 / 2.0f64.sqrt();
        let point = points
            .into_iter()
            .min_by(|a, b| {
                let da = (a[0] - middle).hypot(a[1] - middle);
                let db = (b[0] - middle).hypot(b[1] - middle);
                da.total_cmp(&db)
            })
            .expect("the arc produces points");

        assert!((point[2] - 0.5).abs() < 0.1, "{point:?}");
    }

    #[rstest]
    #[case::clockwise(true, -6.245)]
    #[case::counterclockwise(false, 6.245)]
    fn the_radius_form_picks_the_minor_arc(#[case] clockwise: bool, #[case] center_y: f64) {
        // A clockwise arc from (0, 0) to (10, 0) with radius 8 centers below
        // the chord and bows above it; the other center would need the major
        // arc. Counterclockwise mirrors both.
        let points = linearize_curve(
            MoveKind::Arc { clockwise },
            Some(CurveKind::Arc {
                i: 0.0,
                j: 0.0,
                r: Some(8.0),
            }),
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            0.05,
            64,
        );
        let center = [5.0, center_y];
        let ys: Vec<f64> = xy(&points).iter().map(|p| p[1]).collect();
        let extreme = if clockwise {
            ys.iter().copied().fold(f64::NEG_INFINITY, f64::max)
        } else {
            ys.iter().copied().fold(f64::INFINITY, f64::min)
        };

        assert!(extreme.abs() > 0.1, "arc does not bow: {extreme}");
        for point in xy(&points) {
            assert!(
                ((point[0] - center[0]).hypot(point[1] - center[1]) - 8.0).abs() < 1e-3,
                "{point:?}"
            );
        }
    }

    #[test]
    fn a_radius_smaller_than_half_the_chord_falls_back_to_a_chord() {
        let points = linearize_curve(
            MoveKind::Arc { clockwise: true },
            Some(CurveKind::Arc {
                i: 0.0,
                j: 0.0,
                r: Some(4.0),
            }),
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            0.05,
            64,
        );

        assert_eq!(points, vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0]]);
    }

    #[test]
    fn a_spline_starts_and_ends_at_its_endpoints() {
        // Control points symmetric about x = 5 make the midpoint land at
        // (5, 3.75) for a cubic Bézier.
        let points = linearize_curve(
            MoveKind::Spline,
            Some(CurveKind::Spline {
                i: 3.0,
                j: 5.0,
                p: -3.0,
                q: 5.0,
            }),
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            0.05,
            64,
        );

        assert_eq!(points.len(), 65);
        assert_eq!(points[0], [0.0, 0.0, 0.0]);
        assert_eq!(*points.last().unwrap(), [10.0, 0.0, 0.0]);

        let midpoint = points[64 / 2];
        assert!((midpoint[0] - 5.0).abs() < 1e-3, "{midpoint:?}");
        assert!((midpoint[1] - 3.75).abs() < 1e-3, "{midpoint:?}");
    }
}
