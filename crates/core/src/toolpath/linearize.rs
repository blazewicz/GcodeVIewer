//! Linearizes a curved motion back into a polyline.
//!
//! [`linearize_curve`] is the inverse of the fitting in [`super::arc`] and
//! [`super::spline`]: welding replaces a window of linear moves with an arc or
//! cubic, and linearization samples that curve back into the straight pieces a
//! renderer or measurement pass consumes. Both directions share the same offset
//! conventions, so they live together and are cross-tested. Arcs are sampled to
//! the chord deviation bound `radius * angle^2 / 8`; splines follow Marlin's
//! planner and adapt their step to the curve's local straightness.

use glam::{DVec2, DVec3};

use crate::gcode::CurveKind;

use super::lin_arc::linearize_xy_arc;
use super::lin_bezier_cubic::linearize_xy_bezier_cubic;

/// Linearizes the motion `kind` with optional `curve` into a polyline.
///
/// The first point is `start` and the last is `end`, so consecutive pairs are
/// the straight pieces of the path. Linear motions, curves without offsets, and
/// degenerate shapes fall back to the single `start`-`end` chord. Arc Z follows
/// the sweep linearly, so a helical move stays a helix; spline Z follows the
/// parameter linearly.
///
/// `tolerance` is the target chord deviation in millimetres and
/// `spline_samples` the Bézier sample count of the sample-based spline
/// linearizer; the active Marlin-faithful one is adaptive and ignores it.
pub fn linearize_curve(
    start: DVec3,
    end: DVec3,
    curve: Option<CurveKind>,
    tolerance: f64,
) -> Vec<DVec3> {
    match curve {
        Some(CurveKind::Arc { clockwise, i, j, r }) => {
            linearize_xy_arc(start, end, DVec2::new(i, j), r, clockwise, tolerance)
        }

        Some(CurveKind::Spline { i, j, p, q }) => {
            linearize_xy_bezier_cubic(start, end, DVec2::new(i, j), DVec2::new(p, q))
        }

        _ => vec![start, end],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    #[test]
    fn a_linear_move_is_a_single_chord() {
        let points = linearize_curve(
            DVec3::new(1.0, 2.0, 0.0),
            DVec3::new(4.0, 6.0, 0.5),
            None,
            0.05,
        );

        assert_eq!(
            points,
            vec![DVec3::new(1.0, 2.0, 0.0), DVec3::new(4.0, 6.0, 0.5)]
        );
    }
}
