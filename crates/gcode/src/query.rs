//! Motion records over G-code toolpaths.
//!
//! [`motions`] drives the shared [`State`](crate::state::State) modal
//! interpreter and reports every motion in the file; [`moves_in_box`] reports
//! the subset with a known start or end inside an axis-aligned box. Unlike the
//! optimizer these queries apply no welding policy: travel, relative
//! coordinates, inch units, Z changes, comments, and existing curves all stay
//! interpretable, because a record only needs endpoints and shape, not
//! geometry that must be preserved.

use glam::DVec3;

use crate::gcode::{CurveKind, InterpretedMotion, Line, MoveKind, Outcome, State};

/// One interpreted motion: a linear move or a curve.
///
/// Coordinates are millimetres regardless of the file's unit mode. An endpoint
/// is `None` when any of its axes was never established, which is the case at
/// the start of a file and after resets such as `G28`; the other endpoint is
/// still reported. Curve offsets are in millimetres too.
#[derive(Clone, Debug, PartialEq)]
pub struct MotionRecord {
    pub start: Option<DVec3>,
    pub end: Option<DVec3>,
    /// Active feedrate after this line, in mm/min.
    pub feed: Option<f64>,
    /// Signed extrusion delta in millimetres, `Some(0.0)` when the line carries
    /// no `E` word, and `None` when absolute E has no known origin.
    pub extrusion: Option<f64>,
    /// The motion command that produced the record.
    pub kind: MoveKind,
    /// The control offsets that shape a curved move; `None` for linear moves
    /// and for curve commands without offset words.
    pub curve: Option<CurveKind>,
    /// 1-based physical line number.
    pub line_number: usize,
    /// Raw line with its line ending removed.
    pub line: String,
}

/// Returns every motion in `text`, in file order.
pub fn motions(text: &str) -> Vec<MotionRecord> {
    Scanner::new().run(text)
}

/// The shared modal interpreter plus the optional region filter and the
/// collected matches.
struct Scanner {
    state: State,
    found: Vec<MotionRecord>,
}

impl Scanner {
    fn new() -> Self {
        Self {
            state: State::new(),
            found: Vec::new(),
        }
    }

    fn run(mut self, text: &str) -> Vec<MotionRecord> {
        for (index, raw) in text.split_inclusive('\n').enumerate() {
            if let Outcome::Motion(m) = self.state.advance(&Line::decode(raw)) {
                self.record(&m, raw, index + 1);
            }
        }

        self.found
    }

    /// Records the motion when it matches the region. Without a region every
    /// motion is kept; with one, an unknown endpoint can never match but the
    /// other endpoint still can.
    fn record(&mut self, m: &InterpretedMotion<'_>, raw: &str, line_number: usize) {
        let start = m.start.zip_xyz().map(DVec3::from_array);
        let end = m.end.zip_xyz().map(DVec3::from_array);

        self.found.push(MotionRecord {
            start,
            end,
            feed: m.feed,
            extrusion: m.e_delta,
            kind: m.kind,
            curve: m.curve,
            line_number,
            line: raw.trim_end_matches(['\n', '\r']).to_owned(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use rstest::{fixture, rstest};

    /// Absolute millimetres, a feedrate, tool modal G1, and a known origin. The
    /// feed-only `G1` is not itself a move.
    #[fixture]
    fn ready() -> &'static str {
        indoc! {"
            G21
            G90
            M82
            G92 X0 Y0 Z0 E0
            G1 F1200
        "}
    }

    /// A box around the origin, one millimetre in each direction.
    #[fixture]
    fn origin_box() -> (DVec3, DVec3) {
        (DVec3::splat(-1.0), DVec3::splat(1.0))
    }

    #[rstest]
    fn motions_reports_every_motion_in_order(ready: &str) {
        // given a rapid, a feed, an arc, and a spline
        let source = format!(
            "{ready}G0 X1 Y0 Z0\nG1 X2 Y0 Z0 E1\nG2 X3 Y1 Z0 I0.5 J0.5 E2\nG5 X4 Y2 Z0 I0.5 J0 P0 Q0.5 E3\n"
        );

        // when
        let moves = motions(&source);

        // then
        assert_eq!(
            moves.iter().map(|m| m.kind).collect::<Vec<_>>(),
            vec![
                MoveKind::Rapid,
                MoveKind::Feed,
                MoveKind::Arc { clockwise: true },
                MoveKind::Spline,
            ]
        );
    }

    #[rstest]
    fn motions_records_curve_offsets(ready: &str) {
        // given
        let source = format!("{ready}G2 X3 Y1 Z0 I0.5 J0.5 E2\nG5 X4 Y2 Z0 I0.5 J0 P0 Q0.5 E3\n");

        // when
        let moves = motions(&source);

        // then
        assert_eq!(
            moves[0].curve,
            Some(CurveKind::Arc {
                clockwise: true,
                i: 0.5,
                j: 0.5,
                r: None,
            })
        );
        assert_eq!(
            moves[1].curve,
            Some(CurveKind::Spline {
                i: 0.5,
                j: 0.0,
                p: 0.0,
                q: 0.5
            })
        );
    }

    #[rstest]
    fn motions_records_the_radius_form(ready: &str) {
        // given
        let source = format!("{ready}G2 X3 Y1 Z0 R2 E2\n");

        // when
        let moves = motions(&source);

        // then
        assert_eq!(
            moves[0].curve,
            Some(CurveKind::Arc {
                clockwise: true,
                i: 0.0,
                j: 0.0,
                r: Some(2.0),
            })
        );
    }

    #[test]
    fn motions_reports_moves_with_unknown_endpoints() {
        // A file that never establishes a position still describes a motion
        // whose end is known; a consumer decides how to anchor it.
        let source = "G21\nG90\nG1 X5 Y5 Z0.2 E1 F1200\n";

        let moves = motions(source);

        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].start, None);
        assert_eq!(moves[0].end, Some(DVec3::new(5.0, 5.0, 0.2)));
    }

    #[test]
    fn motions_on_empty_input_yields_nothing() {
        assert!(motions("").is_empty());
    }
}
