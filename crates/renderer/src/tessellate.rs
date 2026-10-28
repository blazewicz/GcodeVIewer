//! Turns motion records into straight drawable segments.
//!
//! The renderer draws instanced geometry, so every curved move is flattened
//! here once per load through the core [`linearize_curve`] routine. Linear moves
//! map to one segment; arcs are subdivided to a chord tolerance and splines
//! follow Marlin's adaptive sampler. Segments keep their Z so the camera can
//! orbit the toolpath in three dimensions, and carry their move kind so the
//! shader can colour rapids, feeds, arcs, and splines separately. Segments also
//! record whether the move deposits material, which the renderer uses to choose
//! between a thin line and a round tube, and the active feedrate, which the
//! shader maps onto a colour scale.

use glam::Vec3;

use gcode_viewer::{MotionRecord, MoveKind, linearize_curve};

/// Target chord deviation when flattening arcs, in millimetres.
const ARC_CHORD_TOLERANCE: f64 = 0.05;
/// Global bound so a huge file cannot exhaust browser memory.
const MAX_SEGMENTS: usize = 8_000_000;

/// The kind of motion a segment belongs to, used as the palette index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub(crate) enum Kind {
    Rapid = 0,
    Feed = 1,
    Arc = 2,
    Spline = 3,
}

impl Kind {
    /// Number of palette entries; one visibility bit each.
    pub(crate) const COUNT: u32 = Self::Spline as u32 + 1;

    fn of(kind: MoveKind) -> Self {
        match kind {
            MoveKind::Rapid => Self::Rapid,
            MoveKind::Feed => Self::Feed,
            MoveKind::Arc { .. } => Self::Arc,
            MoveKind::Spline => Self::Spline,
        }
    }
}

/// One straight drawable segment in millimetres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Segment {
    pub(crate) a: Vec3,
    pub(crate) b: Vec3,
    pub(crate) kind: Kind,
    /// Active feedrate after the move, in mm/min; `None` when no feedrate is
    /// known yet.
    pub(crate) feed: Option<f32>,
    /// Whether the move deposits material; the renderer draws these as tubes.
    pub(crate) extruding: bool,
    /// Whether `a` is the start of its move rather than an interior joint of a
    /// flattened curve. The renderer starts the tube with a socket there.
    pub(crate) start_cap: bool,
    /// Whether `b` is the end of its move rather than an interior joint. The
    /// renderer closes the tube with a cone there; interior joints let the
    /// cone reach into the following segment's socket.
    pub(crate) end_cap: bool,
    /// Highest Z the whole move reaches. Z interpolates linearly along arcs
    /// and splines, so this is the higher of the move's endpoints. The
    /// renderer hides a move whole when this exceeds its Z limit, which keeps
    /// the limit from splitting a flattened move across pieces.
    pub(crate) move_z_max: f32,
}

/// Feedrate sentinel the shader understands; feedrates are non-negative, so
/// any negative value is unambiguous. A host test pins the shader's copy.
pub(crate) const NO_FEED: f32 = -1.0;

/// The feedrate bounds across all segments, or `None` when no segment carries
/// one. Segments without a feedrate are excluded from the range.
pub(crate) fn feed_range(segments: &[Segment]) -> Option<(f32, f32)> {
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    let mut found = false;

    for feed in segments.iter().filter_map(|segment| segment.feed) {
        min = min.min(feed);
        max = max.max(feed);
        found = true;
    }

    found.then_some((min, max))
}

/// Flattens every record with known endpoints into segments, in file order.
pub(crate) fn segments(records: &[MotionRecord]) -> Vec<Segment> {
    let mut out = Vec::new();

    for record in records {
        if out.len() >= MAX_SEGMENTS {
            log::warn!("segment cap reached; remaining moves are not rendered");
            break;
        }

        push_record(record, &mut out);
    }

    out
}

fn push_record(record: &MotionRecord, out: &mut Vec<Segment>) {
    let (Some(start), Some(end)) = (record.start, record.end) else {
        return;
    };
    let extruding = is_extruding(record);
    let feed = record.feed.map(|f| f as f32);
    let first = out.len();

    let points = linearize_curve(record.curve, start, end, ARC_CHORD_TOLERANCE);
    let kind = Kind::of(record.kind);

    for pair in points.windows(2) {
        push_segment(
            pair[0].as_vec3(),
            pair[1].as_vec3(),
            kind,
            extruding,
            feed,
            out,
        );
    }

    // Every piece shares the move's highest Z so the Z limit hides or shows
    // the move as a unit instead of cutting it at the boundary.
    let move_z_max = start[2].max(end[2]) as f32;
    for segment in &mut out[first..] {
        segment.move_z_max = move_z_max;
    }

    // Only the first and last flattened pieces border the move itself; the
    // joints between them continue one move and overlap their cones and
    // sockets.
    if extruding && out.len() > first {
        out[first].start_cap = true;
        let last = out.len() - 1;
        out[last].end_cap = true;
    }
}

/// Whether the move pushes filament. A rapid never does. A non-rapid move does
/// when it commands positive extrusion, and also when absolute E has no known
/// origin: the E word itself marks a printing move, while a missing origin only
/// hides the amount. Retractions and moves without an E word stay thin.
fn is_extruding(record: &MotionRecord) -> bool {
    record.kind != MoveKind::Rapid && record.extrusion.is_none_or(|e| e > 0.0)
}

/// Emits one segment.
fn push_segment(
    a: Vec3,
    b: Vec3,
    kind: Kind,
    extruding: bool,
    feed: Option<f32>,
    out: &mut Vec<Segment>,
) {
    if a.distance(b) < 1e-12 {
        return;
    }

    out.push(Segment {
        a,
        b,
        kind,
        feed,
        extruding,
        start_cap: false,
        end_cap: false,
        move_z_max: a[2].max(b[2]),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use gcode_viewer::CurveKind;
    use glam::DVec3;
    use rstest::rstest;

    fn record(kind: MoveKind, curve: Option<CurveKind>, start: DVec3, end: DVec3) -> MotionRecord {
        MotionRecord {
            start: Some(start),
            end: Some(end),
            feed: Some(1200.0),
            extrusion: Some(1.0),
            kind,
            curve,
            line_number: 1,
            line: String::new(),
        }
    }

    fn arc(
        clockwise: bool,
        i: f64,
        j: f64,
        r: Option<f64>,
        start: DVec3,
        end: DVec3,
    ) -> MotionRecord {
        record(
            MoveKind::Arc { clockwise },
            Some(CurveKind::Arc { clockwise, i, j, r }),
            start,
            end,
        )
    }

    #[rstest]
    #[case::feed(MoveKind::Feed, Kind::Feed, true, true)]
    #[case::rapid(MoveKind::Rapid, Kind::Rapid, false, false)]
    fn a_linear_move_becomes_one_segment(
        #[case] kind: MoveKind,
        #[case] expected: Kind,
        #[case] extruding: bool,
        #[case] capped: bool,
    ) {
        // given
        let records = [record(
            kind,
            None,
            DVec3::new(1.0, 2.0, 0.0),
            DVec3::new(4.0, 6.0, 0.5),
        )];

        // when
        let segments = segments(&records);

        // then
        assert_eq!(
            segments,
            vec![Segment {
                a: Vec3::new(1.0, 2.0, 0.0),
                b: Vec3::new(4.0, 6.0, 0.5),
                kind: expected,
                feed: Some(1200.0),
                extruding,
                start_cap: capped,
                end_cap: capped,
                move_z_max: 0.5,
            }]
        );
    }

    #[rstest]
    #[case::extrude(Some(1.0), true)]
    #[case::travel(Some(0.0), false)]
    #[case::retract(Some(-1.0), false)]
    #[case::unknown_origin(None, true)]
    fn extrusion_decides_whether_a_move_is_a_tube(
        #[case] extrusion: Option<f64>,
        #[case] extruding: bool,
    ) {
        let mut record = record(MoveKind::Feed, None, DVec3::ZERO, DVec3::new(1.0, 0.0, 0.0));
        record.extrusion = extrusion;

        let segments = segments(&[record]);

        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].extruding, extruding);
        assert_eq!(segments[0].start_cap, extruding);
        assert_eq!(segments[0].end_cap, extruding);
    }

    #[test]
    fn only_the_ends_of_a_flattened_move_are_capped() {
        // An arc flattens to many segments, but it is one move: only the first
        // and last pieces border the move itself.
        let records = [arc(
            false,
            -20.0,
            0.0,
            None,
            DVec3::new(20.0, 0.0, 0.0),
            DVec3::new(0.0, 20.0, 0.0),
        )];

        let segments = segments(&records);

        assert!(segments.len() > 1);
        for (index, segment) in segments.iter().enumerate() {
            assert_eq!(segment.start_cap, index == 0, "{index}");
            assert_eq!(segment.end_cap, index == segments.len() - 1, "{index}");
        }
    }

    #[test]
    fn consecutive_moves_each_get_their_own_caps() {
        let records = [
            record(MoveKind::Feed, None, DVec3::ZERO, DVec3::new(1.0, 0.0, 0.0)),
            record(
                MoveKind::Feed,
                None,
                DVec3::new(1.0, 0.0, 0.0),
                DVec3::new(2.0, 0.0, 0.0),
            ),
        ];

        let segments = segments(&records);

        assert_eq!(segments.len(), 2);
        for segment in &segments {
            assert!(segment.start_cap && segment.end_cap);
        }
    }

    #[test]
    fn every_piece_of_a_flattened_move_carries_its_feedrate() {
        let records = [arc(
            false,
            -20.0,
            0.0,
            None,
            DVec3::new(20.0, 0.0, 0.0),
            DVec3::new(0.0, 20.0, 0.0),
        )];

        let segments = segments(&records);

        assert!(segments.len() > 1);
        assert!(segments.iter().all(|s| s.feed == Some(1200.0)));
    }

    #[test]
    fn a_move_without_a_feedrate_carries_none() {
        let mut record = record(MoveKind::Feed, None, DVec3::ZERO, DVec3::new(1.0, 0.0, 0.0));
        record.feed = None;

        let segments = segments(&[record]);

        assert_eq!(segments[0].feed, None);
    }

    #[rstest]
    #[case::spans_the_bounds(vec![Some(300.0), Some(1200.0), Some(600.0)], Some((300.0, 1200.0)))]
    #[case::excludes_unknown(vec![None, Some(600.0)], Some((600.0, 600.0)))]
    #[case::no_known_feeds(vec![None, None], None)]
    fn the_feed_range_spans_known_feedrates(
        #[case] feeds: Vec<Option<f32>>,
        #[case] expected: Option<(f32, f32)>,
    ) {
        let records: Vec<MotionRecord> = feeds
            .into_iter()
            .enumerate()
            .map(|(index, feed)| {
                let x = index as f64;
                let mut record = record(
                    MoveKind::Feed,
                    None,
                    DVec3::new(x, 0.0, 0.0),
                    DVec3::new(x + 1.0, 0.0, 0.0),
                );
                record.feed = feed.map(f64::from);
                record
            })
            .collect();
        let segments = segments(&records);

        assert_eq!(feed_range(&segments), expected);
    }

    #[test]
    fn the_shader_agrees_on_the_no_feed_sentinel() {
        let sentinel = wesl_core::include_wesl!("shader")
            .lines()
            .find_map(|line| line.split_once("NO_FEED: f32 = ").map(|(_, value)| value))
            .and_then(|value| value.trim_end_matches(';').parse::<f32>().ok())
            .expect("compiled shader declares NO_FEED");

        assert_eq!(sentinel, NO_FEED);
    }

    #[test]
    fn every_piece_of_an_extruding_arc_is_a_tube() {
        let records = [arc(
            false,
            -20.0,
            0.0,
            None,
            DVec3::new(20.0, 0.0, 0.0),
            DVec3::new(0.0, 20.0, 0.0),
        )];

        let segments = segments(&records);

        assert!(segments.len() > 1);
        assert!(segments.iter().all(|s| s.extruding));
    }

    #[test]
    fn a_vertical_move_becomes_one_segment() {
        // Z-only moves are real motion and must be visible when the view is
        // orbited away from the top.
        let records = [record(
            MoveKind::Feed,
            None,
            DVec3::new(1.0, 1.0, 0.0),
            DVec3::new(1.0, 1.0, 5.0),
        )];

        let segments = segments(&records);

        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].a, Vec3::new(1.0, 1.0, 0.0));
        assert_eq!(segments[0].b, Vec3::new(1.0, 1.0, 5.0));
    }

    #[test]
    fn an_unknown_endpoint_is_skipped() {
        let mut record = record(MoveKind::Feed, None, DVec3::ZERO, DVec3::new(4.0, 0.0, 0.0));
        record.start = None;

        assert!(segments(&[record]).is_empty());
    }

    #[test]
    fn a_zero_length_move_is_skipped() {
        let records = [record(
            MoveKind::Feed,
            None,
            DVec3::new(3.0, 3.0, 3.0),
            DVec3::new(3.0, 3.0, 3.0),
        )];

        assert!(segments(&records).is_empty());
    }

    #[test]
    fn a_curve_without_offsets_falls_back_to_a_chord() {
        // An implicit arc line inherits the kind but carries no offsets, so the
        // only shape left to draw is the straight distance between endpoints.
        let records = [record(
            MoveKind::Arc { clockwise: true },
            None,
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(10.0, 0.0, 0.0),
        )];

        let segments = segments(&records);

        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].kind, Kind::Arc);
        assert_eq!(segments[0].b, Vec3::new(10.0, 0.0, 0.0));
    }

    #[test]
    fn every_piece_of_a_flattened_move_carries_the_moves_highest_z() {
        // The Z limit applies to the move, not the piece: a helical arc rising
        // through the limit must vanish whole rather than up to the crossing.
        let records = [arc(
            false,
            -20.0,
            0.0,
            None,
            DVec3::new(20.0, 0.0, 0.0),
            DVec3::new(0.0, 20.0, 1.0),
        )];

        let segments = segments(&records);

        assert!(segments.len() > 1);
        assert!(segments.iter().all(|s| s.move_z_max == 1.0));
    }
}
