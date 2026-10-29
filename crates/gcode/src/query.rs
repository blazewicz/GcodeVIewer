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
    Scanner::new(None).run(text)
}

/// Returns every motion whose start or end lies inside the closed box spanned
/// by `min` and `max`. Corners may be given in any order; a NaN corner matches
/// nothing because every comparison against NaN is false.
pub fn moves_in_box(text: &str, min: DVec3, max: DVec3) -> Vec<MotionRecord> {
    let mut min = min;
    let mut max = max;
    for axis in 0..3 {
        if min[axis] > max[axis] {
            std::mem::swap(&mut min[axis], &mut max[axis]);
        }
    }

    Scanner::new(Some((min, max))).run(text)
}

/// The shared modal interpreter plus the optional region filter and the
/// collected matches.
struct Scanner {
    state: State,
    region: Option<(DVec3, DVec3)>,
    found: Vec<MotionRecord>,
}

impl Scanner {
    fn new(region: Option<(DVec3, DVec3)>) -> Self {
        Self {
            state: State::new(),
            region,
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

        if self.matches(start.as_ref(), end.as_ref()) {
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

    fn matches(&self, start: Option<&DVec3>, end: Option<&DVec3>) -> bool {
        self.region
            .is_none_or(|(min, max)| self.contains(start, min, max) || self.contains(end, min, max))
    }

    fn contains(&self, point: Option<&DVec3>, min: DVec3, max: DVec3) -> bool {
        point.is_some_and(|p| (0..3).all(|axis| min[axis] <= p[axis] && p[axis] <= max[axis]))
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

    fn found(source: &str, min: DVec3, max: DVec3) -> Vec<MotionRecord> {
        moves_in_box(source, min, max)
    }

    #[rstest]
    fn reports_a_move_with_an_endpoint_inside(ready: &str) {
        // given
        let source = format!("{ready}G1 X5 Y5 Z1 E1\n");

        // when
        let moves = found(
            &source,
            DVec3::new(4.0, 4.0, 0.0),
            DVec3::new(5.0, 5.0, 1.0),
        );

        // then
        assert_eq!(
            moves,
            vec![MotionRecord {
                start: Some(DVec3::new(0.0, 0.0, 0.0)),
                end: Some(DVec3::new(5.0, 5.0, 1.0)),
                feed: Some(1200.0),
                extrusion: Some(1.0),
                kind: MoveKind::Feed,
                curve: None,
                line_number: 6,
                line: "G1 X5 Y5 Z1 E1".into(),
            }]
        );
    }

    #[rstest]
    #[case::inside(DVec3::new(4.0, 4.0, 0.0), DVec3::new(5.0, 5.0, 1.0), 1)]
    #[case::x_outside(DVec3::new(5.5, 4.0, 0.0), DVec3::new(6.0, 5.0, 1.0), 0)]
    #[case::y_outside(DVec3::new(4.0, 5.5, 0.0), DVec3::new(5.0, 6.0, 1.0), 0)]
    #[case::z_outside(DVec3::new(4.0, 4.0, 1.5), DVec3::new(5.0, 5.0, 2.0), 0)]
    fn every_axis_must_contain_the_endpoint(
        #[case] min: DVec3,
        #[case] max: DVec3,
        #[case] expected: usize,
        ready: &str,
    ) {
        // given
        let source = format!("{ready}G1 X5 Y5 Z1 E1\n");

        // when
        let moves = found(&source, min, max);

        // then
        assert_eq!(moves.len(), expected);
    }

    #[rstest]
    fn box_bounds_are_inclusive(ready: &str) {
        let source = format!("{ready}G1 X5 Y5 Z1 E1\n");

        let moves = found(
            &source,
            DVec3::new(5.0, 5.0, 1.0),
            DVec3::new(5.0, 5.0, 1.0),
        );

        assert_eq!(moves.len(), 1);
    }

    #[rstest]
    fn corners_may_be_given_in_any_order(ready: &str) {
        let source = format!("{ready}G1 X5 Y5 Z1 E1\n");

        let forward = found(
            &source,
            DVec3::new(4.0, 4.0, 0.0),
            DVec3::new(5.0, 5.0, 1.0),
        );
        let reversed = found(
            &source,
            DVec3::new(5.0, 5.0, 1.0),
            DVec3::new(4.0, 4.0, 0.0),
        );

        assert_eq!(forward, reversed);
        assert_eq!(forward.len(), 1);
    }

    #[rstest]
    fn a_move_crossing_the_box_without_an_endpoint_inside_is_skipped(
        ready: &str,
        origin_box: (DVec3, DVec3),
    ) {
        // A segment through the box from one side to the other; neither
        // endpoint is inside, and the query does not test the path between.
        let source = format!("{ready}G92 X-10 Y0 Z0\nG1 X10 Y0 Z0 E1\n");

        let moves = found(&source, origin_box.0, origin_box.1);

        assert_eq!(moves, Vec::new());
    }

    #[rstest]
    fn a_move_is_reported_when_only_its_start_is_inside(ready: &str, origin_box: (DVec3, DVec3)) {
        let source = format!("{ready}G1 X20 Y0 Z0 E1\n");

        let moves = found(&source, origin_box.0, origin_box.1);

        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].start, Some(DVec3::new(0.0, 0.0, 0.0)));
        assert_eq!(moves[0].end, Some(DVec3::new(20.0, 0.0, 0.0)));
    }

    #[rstest]
    fn a_move_is_reported_when_only_its_end_is_inside(ready: &str, origin_box: (DVec3, DVec3)) {
        let source = format!("{ready}G92 X50 Y50 Z10\nG1 X0 Y0 Z0 E2\n");

        let moves = found(&source, origin_box.0, origin_box.1);

        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].start, Some(DVec3::new(50.0, 50.0, 10.0)));
        assert_eq!(moves[0].end, Some(DVec3::new(0.0, 0.0, 0.0)));
    }

    #[test]
    fn an_unknown_start_leaves_only_the_end_testable() {
        // Only Z is established, so X and Y are unknown until this move sets
        // them; the end is fully known and can match on its own.
        let source = "G21\nG90\nG92 Z0\nG1 F1200\nG1 X5 Y5 Z1 E1\n";

        let moves = found(source, DVec3::new(4.0, 4.0, 0.0), DVec3::new(5.0, 5.0, 1.0));

        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].start, None);
        assert_eq!(moves[0].end, Some(DVec3::new(5.0, 5.0, 1.0)));
    }

    #[rstest]
    fn a_reset_makes_relative_endpoints_unknowable(ready: &str, origin_box: (DVec3, DVec3)) {
        // G28 clears the position, so the relative move has no anchor.
        let source = format!("{ready}G28\nG91\nG1 X5 Y5 Z1 E1\n");

        let moves = found(&source, origin_box.0, origin_box.1);

        assert_eq!(moves, Vec::new());
    }

    #[rstest]
    fn an_absolute_move_after_a_reset_is_still_testable(ready: &str) {
        let source = format!("{ready}G28\nG1 X5 Y5 Z1 E1\n");

        let moves = found(
            &source,
            DVec3::new(4.0, 4.0, 0.0),
            DVec3::new(5.0, 5.0, 1.0),
        );

        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].start, None);
    }

    #[rstest]
    fn an_unlexable_line_clears_the_position(ready: &str) {
        let source = format!("{ready}G1 X\nG1 X5 Y5 Z1 E1\n");

        let moves = found(
            &source,
            DVec3::new(4.0, 4.0, 0.0),
            DVec3::new(5.0, 5.0, 1.0),
        );

        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].start, None);
    }

    #[rstest]
    fn relative_moves_are_resolved_against_the_current_position(ready: &str) {
        let source = format!("{ready}G91\nG1 X5 Y3 Z1 E1\nG90\nG1 X50 Y50 Z1 E2\n");

        let moves = found(
            &source,
            DVec3::new(4.0, 2.0, 0.0),
            DVec3::new(5.0, 3.0, 1.0),
        );

        // The relative move ends inside the box and the absolute move starts
        // there, so both are reported.
        assert_eq!(moves.len(), 2);
        assert_eq!(moves[0].start, Some(DVec3::new(0.0, 0.0, 0.0)));
        assert_eq!(moves[0].end, Some(DVec3::new(5.0, 3.0, 1.0)));
        assert_eq!(moves[1].start, Some(DVec3::new(5.0, 3.0, 1.0)));
    }

    #[rstest]
    fn inch_units_are_converted_to_millimetres(ready: &str) {
        let source = format!("{ready}G20\nG1 X1 Y1 Z0.04 E0.04\n");

        let moves = found(
            &source,
            DVec3::new(25.0, 25.0, 1.0),
            DVec3::new(26.0, 26.0, 1.1),
        );

        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].end, Some(DVec3::new(25.4, 25.4, 1.016)));
    }

    #[rstest]
    #[case::clockwise("G2 X0 Y10 Z0 I0 J5 E1", MoveKind::Arc { clockwise: true })]
    #[case::counterclockwise("G3 X0 Y10 Z0 I0 J5 E1", MoveKind::Arc { clockwise: false })]
    #[case::spline("G5 X0 Y10 Z0 I0 J5 P0 Q5 E1", MoveKind::Spline)]
    fn existing_curves_are_moves_by_endpoint(
        #[case] command: &str,
        #[case] kind: MoveKind,
        ready: &str,
    ) {
        // given
        let source = format!("{ready}{command}\nG1 X0 Y20 Z0 E2\n");

        // when
        let moves = found(
            &source,
            DVec3::new(-1.0, 9.0, -1.0),
            DVec3::new(1.0, 10.0, 1.0),
        );

        // then the curve ends inside and the next move starts there
        assert_eq!(moves.len(), 2);
        assert_eq!(moves[0].end, Some(DVec3::new(0.0, 10.0, 0.0)));
        assert_eq!(moves[0].kind, kind);
        assert_eq!(moves[1].start, Some(DVec3::new(0.0, 10.0, 0.0)));
    }

    #[rstest]
    fn implicit_motion_uses_the_modal_command(ready: &str) {
        let source = format!("{ready}G0 X1 Y1 Z1\nX2 Y2 Z2\n");

        let moves = found(
            &source,
            DVec3::new(1.5, 1.5, 1.5),
            DVec3::new(2.5, 2.5, 2.5),
        );

        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].end, Some(DVec3::new(2.0, 2.0, 2.0)));
        assert_eq!(moves[0].kind, MoveKind::Rapid);
    }

    #[test]
    fn implicit_motion_without_a_modal_command_is_ignored() {
        let source = "G21\nG90\nX5 Y5 Z0\n";

        let moves = found(
            source,
            DVec3::new(-1.0, -1.0, -1.0),
            DVec3::new(1.0, 1.0, 1.0),
        );

        assert_eq!(moves, Vec::new());
    }

    #[rstest]
    fn an_extrusion_only_line_is_a_move_at_the_current_position(
        ready: &str,
        origin_box: (DVec3, DVec3),
    ) {
        let source = format!("{ready}G1 E-1\n");

        let moves = found(&source, origin_box.0, origin_box.1);

        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].start, Some(DVec3::new(0.0, 0.0, 0.0)));
        assert_eq!(moves[0].end, Some(DVec3::new(0.0, 0.0, 0.0)));
        assert_eq!(moves[0].extrusion, Some(-1.0));
    }

    #[rstest]
    fn a_feed_only_line_is_not_a_move(ready: &str, origin_box: (DVec3, DVec3)) {
        let source = format!("{ready}G1 F900\n");

        let moves = found(&source, origin_box.0, origin_box.1);

        assert_eq!(moves, Vec::new());
    }

    #[rstest]
    fn feed_carries_over_and_extrusion_is_the_delta(ready: &str) {
        let source = format!("{ready}G1 X5 Y5 Z0 E2\nG1 X6 Y6 Z0 E5 F900\n");

        let moves = found(
            &source,
            DVec3::new(4.0, 4.0, -1.0),
            DVec3::new(6.0, 6.0, 1.0),
        );

        assert_eq!(moves.len(), 2);
        assert_eq!(moves[0].feed, Some(1200.0));
        assert_eq!(moves[0].extrusion, Some(2.0));
        assert_eq!(moves[1].feed, Some(900.0));
        assert_eq!(moves[1].extrusion, Some(3.0));
    }

    #[test]
    fn absolute_extrusion_without_a_known_origin_is_unknown() {
        // E has never been established, so the absolute E delta is unknowable
        // while the position is fully known.
        let source = "G21\nG90\nG92 X0 Y0 Z0\nG1 X5 Y5 Z0 E1 F1200\n";

        let moves = found(
            source,
            DVec3::new(-1.0, -1.0, -1.0),
            DVec3::new(6.0, 6.0, 1.0),
        );

        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].extrusion, None);
        assert_eq!(moves[0].feed, Some(1200.0));
    }

    #[rstest]
    fn text_messages_leave_the_position_known(ready: &str) {
        let source = format!("{ready}M117 hello\nG1 X5 Y5 Z1 E1\n");

        let moves = found(
            &source,
            DVec3::new(4.0, 4.0, 0.0),
            DVec3::new(5.0, 5.0, 1.0),
        );

        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].start, Some(DVec3::new(0.0, 0.0, 0.0)));
    }

    #[rstest]
    fn the_reported_line_keeps_comments_and_drops_the_ending(ready: &str) {
        let source = format!("{ready}G1 X5 Y5 Z1 E1 ; note\r\n");

        let moves = found(
            &source,
            DVec3::new(4.0, 4.0, 0.0),
            DVec3::new(5.0, 5.0, 1.0),
        );

        assert_eq!(moves[0].line, "G1 X5 Y5 Z1 E1 ; note");
    }

    #[rstest]
    fn relative_extrusion_delta_is_known_without_an_origin() {
        // M83 makes the E word a delta, so the commanded amount is known even
        // though no absolute origin was ever established.
        let source = "G21\nG90\nG92 X0 Y0 Z0\nM83\nG1 X5 Y5 Z0 E1\n";

        let moves = found(
            source,
            DVec3::new(-1.0, -1.0, -1.0),
            DVec3::new(6.0, 6.0, 1.0),
        );

        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].extrusion, Some(1.0));
    }

    #[rstest]
    fn a_rapid_keeps_the_print_feed(ready: &str) {
        // The query reports the feed a firmware would still have active; only
        // the optimizer clears it to stay conservative.
        let source = format!("{ready}G0 X1 Y1 Z0\n");

        let moves = found(
            &source,
            DVec3::new(-1.0, -1.0, -1.0),
            DVec3::new(1.0, 1.0, 1.0),
        );

        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].kind, MoveKind::Rapid);
        assert_eq!(moves[0].feed, Some(1200.0));
    }

    #[test]
    fn empty_input_yields_no_moves() {
        assert!(found("", DVec3::splat(-1.0), DVec3::splat(1.0)).is_empty());
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
