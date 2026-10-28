use glam::{DVec2, DVec3, Vec3Swizzles};

/// Marlin's `MIN_STEP`: the shortest parameter step its spline sampler takes.
const MARLIN_MIN_STEP: f64 = 0.002;

/// Marlin's `MAX_STEP`: the step the spline sampler starts from and the soft
/// cap it doubles against.
const MARLIN_MAX_STEP: f64 = 0.1;

/// Marlin's `SIGMA`: the largest midpoint deviation that still passes as a
/// straight segment, measured in the L1 distance.
const MARLIN_SIGMA: f64 = 0.1;

/// Linearizes one cubic Bézier the way Marlin's `cubic_b_spline()` does
/// (`planner_bezier.cpp`): the parameter step starts at `MAX_STEP`, halves
/// while the segment's midpoint strays too far from its chord, and otherwise
/// doubles until the `MAX_STEP` checks stop it. The emitted parameters follow
/// Marlin's sequence, so the flattened path matches the segmentation the
/// firmware commands. The sample count is unused: the stepping is adaptive.
pub(super) fn linearize_xy_bezier_cubic(
    start: DVec3,
    end: DVec3,
    i: f64,
    j: f64,
    p: f64,
    q: f64,
) -> Vec<DVec3> {
    let p0 = start.xy();
    let p1 = end.xy();
    let c1 = p0 + DVec2::new(i, j);
    let c2 = p1 + DVec2::new(p, q);

    let mut points = vec![start];
    // Marlin's `bez_target`: the last emitted point, where the next chord
    // starts.
    let mut bez_target = p0;
    let mut t = 0.0;

    // Marlin main restarts every segment from MAX_STEP: the carry-over of the
    // previous step was dropped upstream in #24366, so this port keeps none.
    let step = MARLIN_MAX_STEP;

    while t < 1.0 {
        // First try to reduce the step until the segment is sufficiently
        // close to a linear interpolation.
        let mut did_reduce = false;
        let mut new_t = (t + step).min(1.0);
        let mut new_pos = eval_bezier(p0, c1, c2, p1, new_t);

        loop {
            if new_t - t < MARLIN_MIN_STEP {
                break;
            }

            let candidate_t = 0.5 * (t + new_t);
            let candidate_pos = eval_bezier(p0, c1, c2, p1, candidate_t);
            let chord_mid = 0.5 * (bez_target + new_pos);

            if dist1(candidate_pos, chord_mid) <= MARLIN_SIGMA {
                break;
            }

            new_t = candidate_t;
            new_pos = candidate_pos;
            did_reduce = true;
        }

        // If nothing was reduced, maybe the step can be enlarged.
        if !did_reduce {
            loop {
                if new_t - t > MARLIN_MAX_STEP {
                    break;
                }

                let candidate_t = t + 2.0 * (new_t - t);
                if candidate_t >= 1.0 {
                    break;
                }

                let candidate_pos = eval_bezier(p0, c1, c2, p1, candidate_t);
                let chord_mid = 0.5 * (bez_target + candidate_pos);

                if dist1(new_pos, chord_mid) > MARLIN_SIGMA {
                    break;
                }

                new_t = candidate_t;
                new_pos = candidate_pos;
            }
        }

        t = new_t;
        let z = (1.0 - t) * start.z + t * end.z;
        points.push(new_pos.extend(z));
        bez_target = new_pos;
    }

    points
}

/// Evaluates a cubic Bézier with De Casteljau's algorithm, as Marlin does:
/// repeated linear interpolation between adjacent control points.
///
/// [`DVec2::lerp`] computes `a * (1 - t) + b * t`, the same expression as
/// Marlin's `interp()`, so the port rounds the same way.
fn eval_bezier(a: DVec2, b: DVec2, c: DVec2, d: DVec2, t: f64) -> DVec2 {
    let iab = a.lerp(b, t);
    let ibc = b.lerp(c, t);
    let icd = c.lerp(d, t);
    let iabc = iab.lerp(ibc, t);
    let ibcd = ibc.lerp(icd, t);
    iabc.lerp(ibcd, t)
}

/// Marlin's `dist1()`: Euclidean distance approximated by the sum of the
/// coordinate offsets, the L1 norm.
fn dist1(a: DVec2, b: DVec2) -> f64 {
    (a - b).abs().element_sum()
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn a_spline_starts_and_ends_at_its_endpoints() {
        // Control points symmetric about x = 5 make the midpoint land at
        // (5, 3.75) for a cubic Bézier.
        let points = linearize_xy_bezier_cubic(
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(10.0, 0.0, 0.0),
            3.0,
            5.0,
            -3.0,
            5.0,
        );

        assert_eq!(points[0], DVec3::new(0.0, 0.0, 0.0));
        assert_eq!(*points.last().unwrap(), DVec3::new(10.0, 0.0, 0.0));

        // Adaptive stepping needs far fewer points than the fixed 64-sample
        // fallback would.
        assert!(points.len() < 65, "{}", points.len());

        let apex = DVec2::new(5.0, 3.75);
        let nearest = points
            .iter()
            .map(|point| point.xy().distance(apex))
            .fold(f64::INFINITY, f64::min);
        assert!(nearest < 0.01, "{nearest}");
    }

    #[test]
    fn the_marlinfw_spline_emits_the_reference_point_sequence() {
        // Control points symmetric about x = 5, ending 2 mm higher. The
        // expected points come from a line-by-line port of Marlin's
        // `cubic_b_spline()` for this curve: every emitted step stays at
        // MAX_STEP, and the final segment is the sliver left by the clamp to
        // t = 1.
        let points = linearize_xy_bezier_cubic(
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(10.0, 0.0, 2.0),
            3.0,
            5.0,
            -3.0,
            5.0,
        );

        assert_eq!(
            points,
            vec![
                DVec3::new(0.0, 0.0, 0.0),
                DVec3::new(0.9280000000000002, 1.35, 0.2),
                DVec3::new(1.9040000000000006, 2.4000000000000004, 0.4),
                DVec3::new(2.9160000000000004, 3.1500000000000004, 0.6000000000000001),
                DVec3::new(3.952, 3.6, 0.8),
                DVec3::new(5.0, 3.75, 1.0),
                DVec3::new(6.048, 3.6, 1.2),
                DVec3::new(7.0840000000000005, 3.1500000000000004, 1.4),
                DVec3::new(8.095999999999998, 2.400000000000001, 1.5999999999999999),
                DVec3::new(9.072000000000001, 1.350000000000001, 1.7999999999999998),
                DVec3::new(
                    9.999999999999995,
                    1.6653345369377346e-15,
                    1.9999999999999998
                ),
                DVec3::new(10.0, 0.0, 2.0),
            ]
        );
    }

    #[test]
    fn a_straight_spline_enlarges_its_step_within_the_marlin_caps() {
        // Collinear control points keep the curve exactly straight, so the
        // sampler never reduces and doubles until the MAX_STEP checks and the
        // candidate-t guard stop it. The ending Z doubles as a probe of the
        // emitted parameters: it interpolates to exactly t.
        let points = linearize_xy_bezier_cubic(
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(9.0, 0.0, 1.0),
            3.0,
            0.0,
            -3.0,
            0.0,
        );

        assert_eq!(
            points,
            vec![
                DVec3::new(0.0, 0.0, 0.0),
                DVec3::new(1.8000000000000005, 0.0, 0.2),
                DVec3::new(2.7, 0.0, 0.30000000000000004),
                DVec3::new(4.5, 0.0, 0.5),
                DVec3::new(6.299999999999999, 0.0, 0.7),
                DVec3::new(8.099999999999998, 0.0, 0.8999999999999999),
                DVec3::new(8.999999999999995, 0.0, 0.9999999999999999),
                DVec3::new(9.0, 0.0, 1.0),
            ]
        );
    }
}
