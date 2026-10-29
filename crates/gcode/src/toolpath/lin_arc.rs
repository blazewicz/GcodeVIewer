use std::f64::consts::TAU;

use glam::{DVec2, DVec3, Vec3Swizzles};

/// Upper bound on segments for one arc.
const MAX_ARC_SEGMENTS: usize = 1024;

/// Linearizes one arc. The center comes from the `i`/`j` offsets, or from the
/// radius form when `r` is present; a degenerate shape falls back to a chord.
pub(super) fn linearize_xy_arc(
    start: DVec3,
    end: DVec3,
    ij: DVec2,
    r: Option<f64>,
    clockwise: bool,
    tolerance: f64,
) -> Vec<DVec3> {
    let p0 = start.xy();
    let p1 = end.xy();

    let center = match r {
        Some(radius) => match arc_center_from_radius(start, end, radius, clockwise) {
            Some(center) => center,
            None => return vec![start, end],
        },
        None => p0 + ij,
    };

    let v0 = p0 - center;
    let v1 = p1 - center;
    let radius = v0.length();

    if radius < 1e-12 {
        return vec![start, end];
    }

    let a0 = v0.y.atan2(v0.x);
    let a1 = v1.y.atan2(v1.x);
    let sweep = sweep_between(a0, a1, clockwise);

    let steps = ((sweep.abs() * (radius / (8.0 * tolerance)).sqrt()).ceil() as usize)
        .clamp(2, MAX_ARC_SEGMENTS);

    let mut points = Vec::with_capacity(steps + 1);
    points.push(start);

    for step in 1..steps {
        let t = step as f64 / steps as f64;
        let angle = a0 + sweep * t;
        let xy = center + DVec2::from_angle(angle) * radius;

        points.push(xy.extend(start.z + (end.z - start.z) * t));
    }

    points.push(end);
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
fn arc_center_from_radius(start: DVec3, end: DVec3, radius: f64, clockwise: bool) -> Option<DVec2> {
    let p0 = start.xy();
    let d = end.xy() - p0;
    let chord_sq = d.length_squared();
    let radius_sq = radius * radius;

    if !chord_sq.is_finite()
        || !radius_sq.is_finite()
        || chord_sq < 1e-24
        || chord_sq > 4.0 * radius_sq
    {
        return None;
    }

    let height = (radius_sq / chord_sq - 0.25).sqrt();
    let normal = DVec2::new(-d.y, d.x);

    let offset = normal * height * (if clockwise { -1.0 } else { 1.0 });

    Some(p0 + d * 0.5 + offset)
}

#[cfg(test)]
mod test {
    use super::*;
    use rstest::rstest;

    /// The points projected onto the XY plane.
    fn xy(points: &[DVec3]) -> Vec<[f64; 2]> {
        points.iter().map(|p| [p[0], p[1]]).collect()
    }

    #[test]
    fn a_quarter_arc_follows_its_circle() {
        // A counterclockwise quarter turn from (20, 0) to (0, 20) centered at
        // the origin: the offsets point back at the start's center.
        let points = linearize_xy_arc(
            DVec3::new(20.0, 0.0, 0.0),
            DVec3::new(0.0, 20.0, 0.0),
            DVec2::new(-20.0, 0.0),
            None,
            false,
            0.05,
        );

        assert!(points.len() > 3);
        for point in xy(&points) {
            assert!((point[0].hypot(point[1]) - 20.0).abs() < 1e-3, "{point:?}");
        }
        assert_eq!(points[0], DVec3::new(20.0, 0.0, 0.0));
        let end = *points.last().unwrap();
        assert!(
            (end[0] - 0.0).abs() < 1e-3 && (end[1] - 20.0).abs() < 1e-3,
            "{end:?}"
        );
    }

    #[test]
    fn coincident_arc_endpoints_sweep_a_full_circle() {
        // G2/G3 with equal endpoints and center offsets is a full circle.
        let points = linearize_xy_arc(
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(0.0, 0.0, 0.0),
            DVec2::new(0.0, 10.0),
            None,
            false,
            0.05,
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
        let points = linearize_xy_arc(
            DVec3::new(20.0, 0.0, 0.0),
            DVec3::new(0.0, 20.0, 1.0),
            DVec2::new(-20.0, 0.0),
            None,
            false,
            0.05,
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
        let points = linearize_xy_arc(
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(10.0, 0.0, 0.0),
            DVec2::new(0.0, 0.0),
            Some(8.0),
            clockwise,
            0.05,
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
        let points = linearize_xy_arc(
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(10.0, 0.0, 0.0),
            DVec2::new(0.0, 0.0),
            Some(4.0),
            true,
            0.05,
        );

        assert_eq!(
            points,
            vec![DVec3::new(0.0, 0.0, 0.0), DVec3::new(10.0, 0.0, 0.0)]
        );
    }
}
