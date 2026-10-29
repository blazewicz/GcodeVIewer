//! Modal G-code state machine.
//!
//! [`State`] consumes lexed lines one at a time and tracks the interpreter
//! state that fixes where a motion starts and ends: position, extruder,
//! feedrate, positioning mode, units, and plane. [`State::advance`] returns the
//! interpreted [`Motion`] for every line that commands a coordinate or
//! extrusion word, and callers layer their own policies on top: the optimizer
//! decides there whether a motion is weldable, and the box query tests the
//! endpoints against its region.
//!
//! Coordinates, extrusion, and feedrate are stored in millimetres (per minute)
//! regardless of the file's unit mode, so they stay comparable across a
//! `G20`/`G21` switch.

use super::code::Code;
use super::lexer::{Line, Words};
use super::motion::{CurveKind, MoveKind};

/// Millimetres per coordinate unit while inch units are active.
const MM_PER_INCH: f64 = 25.4;

/// Partial coordinates of the 3D position and absolute position of the extruder.
/// `None` represents unspecified or undetermined coordinate.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct PartialPosition {
    pub(crate) xyz: [Option<f64>; 3],
    pub(crate) e: Option<f64>,
}

impl PartialPosition {
    /// The XYZ coordinates, when every axis is known.
    pub(crate) fn zip_xyz(self) -> Option<[f64; 3]> {
        match self.xyz {
            [Some(x), Some(y), Some(z)] => Some([x, y, z]),
            _ => None,
        }
    }
}

/// One interpreted motion command, linear or curve.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct InterpretedMotion<'a> {
    // State machine's position before the command.
    pub(crate) start: PartialPosition,
    // State machine's position after the command.
    pub(crate) end: PartialPosition,
    /// Signed extrusion delta, either:
    /// * the value of `E` param when `relative_e` is `true`,
    /// * the difference of two known positions when `relative_e` is `false`,
    /// * `Some(0.0)` when `E` word was not given,
    /// * `None` when previous value was unknown and `relative_e` is `false`.
    pub(crate) e_delta: Option<f64>,
    /// Whether command carried an `E` word.
    pub(crate) e_commanded: bool,
    /// Active feedrate after this line, in mm/min.
    pub(crate) feed: Option<f64>,
    /// Whether extrusion value is relative.
    pub(crate) relative_e: bool,
    /// What kind of move it is.
    pub(crate) kind: MoveKind,
    /// The control offsets that shape a curved move, scaled to millimetres.
    /// `None` when the line carries no offset words, as with implicit motion
    /// inherited from a modal curve command.
    pub(crate) curve: Option<CurveKind>,
    /// True when the line carried no motion command and `kind` was inherited
    /// from the modal state. Consumers must be aware that following lines could
    /// also depend on modal state so changes of the move kind can break file's
    /// integrity.
    pub(crate) implicit: bool,
    // Raw string of the original code line.
    pub(crate) raw: &'a str,
}

/// What one line did to the modal state.
///
/// The motion is the common variant; boxing it would add a heap allocation per
/// line on files with millions of moves.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Outcome<'a> {
    /// The line moved the machine or extruded material.
    Motion(InterpretedMotion<'a>),
    /// Syntax outside the understood grammar; the reference frame was cleared.
    Unsupported,
    /// A line that left the tracked state untouched, or only changed a mode.
    Ignored,
}

pub(crate) struct State {
    position: PartialPosition,
    feed: Option<f64>,
    relative: bool,
    relative_e: bool,
    mm: bool,
    xy: bool,
    /// The last motion command, which gives implicit parameter lines their meaning.
    modal: Option<MoveKind>,
}

impl State {
    /// The interpreter state assumed before any line has been read.
    pub(crate) fn new() -> Self {
        Self {
            position: PartialPosition::default(),
            feed: None,
            relative: false,
            relative_e: false,
            mm: true,
            xy: true,
            modal: None,
        }
    }

    pub(crate) fn relative(&self) -> bool {
        self.relative
    }

    pub(crate) fn mm(&self) -> bool {
        self.mm
    }

    pub(crate) fn xy(&self) -> bool {
        self.xy
    }

    /// Drops the tracked feedrate, for callers that cannot attribute the print
    /// feed to a rapid using its own configured rate.
    pub(crate) fn clear_feed(&mut self) {
        self.feed = None;
    }

    /// Homing, tool changes, stepper disables, and unrecognized commands all
    /// destroy the position, extrusion origin, and feedrate.
    fn invalidate(&mut self) {
        self.position = PartialPosition::default();
        self.feed = None;
    }

    /// Consumes one parsed line, updating modal state and reporting what the
    /// line did.
    pub(crate) fn advance<'a>(&mut self, line: &Line<'a>) -> Outcome<'a> {
        let Ok(words) = &line.parsed else {
            self.invalidate();
            return Outcome::Unsupported;
        };

        match Code::classify(words) {
            Some(Code::LinearMove { rapid }) => {
                let kind = if rapid {
                    MoveKind::Rapid
                } else {
                    MoveKind::Feed
                };
                self.motion(words, line.raw, kind, false)
            }

            Some(Code::ArcMove { clockwise }) => {
                self.motion(words, line.raw, MoveKind::Arc { clockwise }, false)
            }

            Some(Code::SplineMove) => self.motion(words, line.raw, MoveKind::Spline, false),

            Some(Code::Implicit) => self.implicit(words, line.raw),

            Some(Code::Positioning { relative }) => {
                self.relative = relative;
                self.relative_e = relative;

                Outcome::Ignored
            }

            Some(Code::Extrusion { relative }) => {
                self.relative_e = relative;

                Outcome::Ignored
            }

            Some(Code::Units { mm }) => {
                self.mm = mm;

                Outcome::Ignored
            }

            Some(Code::Plane { xy }) => {
                self.xy = xy;

                Outcome::Ignored
            }

            Some(Code::SetPosition) => {
                for (axis, value) in Self::iter_given_xyz(words) {
                    self.position.xyz[axis] = Some(self.scaled(value));
                }

                if let Some(value) = words.param(b'E') {
                    self.position.e = Some(self.scaled(value));
                }

                Outcome::Ignored
            }

            Some(Code::Reset) => {
                self.invalidate();

                Outcome::Ignored
            }

            Some(Code::Other) => {
                self.invalidate();

                Outcome::Unsupported
            }

            Some(Code::NoOp) | None => Outcome::Ignored,
        }
    }

    fn implicit<'a>(&mut self, words: &Words, raw: &'a str) -> Outcome<'a> {
        match self.modal {
            Some(kind) => self.motion(words, raw, kind, true),

            // Without a modal motion command the parameters cannot be placed.
            None => {
                self.invalidate();

                Outcome::Unsupported
            }
        }
    }

    /// Advances position, extrusion, and feedrate from a motion line's words.
    /// A line that commands neither a coordinate nor extrusion only updates the
    /// modal motion command. `implicit` records whether the motion command was
    /// inherited rather than written on the line.
    fn motion<'a>(
        &mut self,
        words: &Words,
        raw: &'a str,
        kind: MoveKind,
        implicit: bool,
    ) -> Outcome<'a> {
        let before = self.position;
        let mut moved = false;

        for (axis, value) in Self::iter_given_xyz(words) {
            moved = true;
            self.position.xyz[axis] = self.axis(before.xyz[axis], value);
        }

        let commanded = words.param(b'E');
        let (e1, e_delta) = match commanded {
            None => (before.e, Some(0.0)),
            Some(value) => {
                let value = self.scaled(value);
                if self.relative_e {
                    (before.e.map(|e| e + value), Some(value))
                } else {
                    (Some(value), before.e.map(|e| value - e))
                }
            }
        };
        self.position.e = e1;

        if let Some(f) = words.param(b'F') {
            self.feed = Some(self.scaled(f));
        }

        self.modal = Some(kind);

        if !moved && commanded.is_none() {
            return Outcome::Ignored;
        }

        let curve = self.curve_params(kind, words);

        Outcome::Motion(InterpretedMotion {
            // TODO: The only case when positions are `None` is when a
            // relative move is command from an unknown position.
            // I suppose we could treat that as program error and
            // return an error. Then we could replace PartialPosition
            // in MotionCommand with a fully defined Position. In a
            // valid G-code file every move should be clearly defined.
            start: before,
            end: self.position,
            e_delta,
            e_commanded: commanded.is_some(),
            feed: self.feed,
            relative_e: self.relative_e,
            kind,
            curve,
            implicit,
            raw,
        })
    }

    fn iter_given_xyz(words: &Words) -> impl Iterator<Item = (usize, f64)> {
        b"XYZ"
            .iter()
            .enumerate()
            .filter_map(|(axis, letter)| words.param(*letter).map(|value| (axis, value)))
    }

    /// Reads the curve's control offsets from the line, in millimetres. A curve
    /// command without offsets has no shape to describe, so it reports `None`.
    fn curve_params(&self, kind: MoveKind, words: &Words) -> Option<CurveKind> {
        match kind {
            MoveKind::Arc { clockwise } => {
                let (i, j, r) = (words.param(b'I'), words.param(b'J'), words.param(b'R'));

                (i.is_some() || j.is_some() || r.is_some()).then(|| CurveKind::Arc {
                    clockwise,
                    i: self.scaled(i.unwrap_or(0.0)),
                    j: self.scaled(j.unwrap_or(0.0)),
                    r: r.map(|value| self.scaled(value)),
                })
            }

            MoveKind::Spline => {
                let (i, j, p, q) = (
                    words.param(b'I'),
                    words.param(b'J'),
                    words.param(b'P'),
                    words.param(b'Q'),
                );

                (i.is_some() || j.is_some() || p.is_some() || q.is_some()).then(|| {
                    CurveKind::Spline {
                        i: self.scaled(i.unwrap_or(0.0)),
                        j: self.scaled(j.unwrap_or(0.0)),
                        p: self.scaled(p.unwrap_or(0.0)),
                        q: self.scaled(q.unwrap_or(0.0)),
                    }
                })
            }

            MoveKind::Rapid | MoveKind::Feed => None,
        }
    }

    /// One axis after an absolute or relative command. A relative command from
    /// an unknown position leaves the axis unknown rather than inventing zero.
    fn axis(&self, before: Option<f64>, value: f64) -> Option<f64> {
        let value = self.scaled(value);
        if self.relative {
            // TODO: Shouldn't we return an error when a relative move is
            // commanded from an unknown position?
            before.map(|b| b + value)
        } else {
            Some(value)
        }
    }

    /// Applies the modal units, returning millimetres.
    fn scaled(&self, value: f64) -> f64 {
        if self.mm { value } else { value * MM_PER_INCH }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use rstest::{fixture, rstest};

    /// Runs one decoded line through the state machine.
    fn step<'a>(state: &mut State, text: &'a str) -> Outcome<'a> {
        state.advance(&Line::decode(text))
    }

    /// Runs one decoded line that must be an interpreted motion.
    fn motion<'a>(state: &mut State, text: &'a str) -> InterpretedMotion<'a> {
        match step(state, text) {
            Outcome::Motion(m) => m,
            other => panic!("expected a motion, got {other:?}"),
        }
    }

    /// Absolute positioning, millimetres, absolute extrusion, a print feedrate,
    /// and a known position at the origin.
    #[fixture]
    fn preamble() -> &'static str {
        indoc! {"
            G21
            G90
            M82
            G1 X0 Y0 Z0.2 F1200 E0
        "}
    }

    /// A state with position, feed, and E established by the preamble.
    #[fixture]
    fn ready() -> State {
        let mut state = State::new();
        for line in preamble().lines() {
            step(&mut state, line);
        }
        state
    }

    #[rstest]
    #[case::all_axes_known(
        [Some(1.0), Some(2.0), Some(3.0)],
        Some([1.0, 2.0, 3.0])
    )]
    #[case::unknown_x(
        [None, Some(2.0), Some(3.0)],
        None
    )]
    #[case::unknown_y(
        [Some(1.0), None, Some(3.0)],
        None
    )]
    #[case::unknown_z(
        [Some(1.0), Some(2.0), None],
        None
    )]
    #[case::nothing_known([None; 3], None)]
    fn known_xyz_needs_all_axes_and_ignores_e(
        #[values(Some(4.0), None)] e: Option<f64>,
        #[case] xyz: [Option<f64>; 3],
        #[case] expected: Option<[f64; 3]>,
    ) {
        let position = PartialPosition { xyz, e };
        assert_eq!(position.zip_xyz(), expected);
    }

    #[rstest]
    fn a_move_reports_both_endpoints_and_extrusion(mut ready: State) {
        // when
        let m = motion(&mut ready, "G1 X10 Y2 E1");

        // then
        assert_eq!(
            m,
            InterpretedMotion {
                start: PartialPosition {
                    xyz: [Some(0.0), Some(0.0), Some(0.2)],
                    e: Some(0.0),
                },
                end: PartialPosition {
                    xyz: [Some(10.0), Some(2.0), Some(0.2)],
                    e: Some(1.0),
                },
                e_delta: Some(1.0),
                e_commanded: true,
                feed: Some(1200.0),
                relative_e: false,
                kind: MoveKind::Feed,
                curve: None,
                implicit: false,
                raw: "G1 X10 Y2 E1",
            }
        );
    }

    #[rstest]
    fn unlisted_axes_keep_their_previous_values(mut ready: State) {
        let m = motion(&mut ready, "G1 X10 E1");

        assert_eq!(
            m,
            InterpretedMotion {
                start: PartialPosition {
                    xyz: [Some(0.0), Some(0.0), Some(0.2)],
                    e: Some(0.0),
                },
                end: PartialPosition {
                    xyz: [Some(10.0), Some(0.0), Some(0.2)],
                    e: Some(1.0),
                },
                e_delta: Some(1.0),
                e_commanded: true,
                feed: Some(1200.0),
                relative_e: false,
                kind: MoveKind::Feed,
                curve: None,
                implicit: false,
                raw: "G1 X10 E1",
            }
        );
    }

    #[rstest]
    fn feed_update_applies_to_the_same_line_and_later_moves(mut ready: State) {
        // given
        let m = motion(&mut ready, "G1 X1 Y0 E1 F900");

        // when
        let next = motion(&mut ready, "G1 X2 Y0 E2");

        // then
        assert_eq!(m.feed, Some(900.0));
        assert_eq!(next.feed, Some(900.0));
    }

    #[rstest]
    fn a_line_without_an_e_word_extrudes_nothing(mut ready: State) {
        // given
        step(&mut ready, "G92 E5");

        // when
        let m = motion(&mut ready, "G1 X10 Y0");

        // then
        assert_eq!(
            (m.start.e, m.end.e, m.e_delta, m.e_commanded),
            (Some(5.0), Some(5.0), Some(0.0), false)
        );
    }

    #[rstest]
    fn absolute_extrusion_without_an_origin_has_no_delta() {
        // given an E origin that was never established
        let mut state = State::new();
        step(&mut state, "G92 X0 Y0 Z0.2");

        // when
        let m = motion(&mut state, "G1 X10 Y0 E1 F1200");

        // then
        assert_eq!((m.start.e, m.end.e, m.e_delta), (None, Some(1.0), None));
    }

    #[rstest]
    fn relative_extrusion_is_carried_on_the_move(mut ready: State) {
        // given
        step(&mut ready, "M83");

        // when
        let m = motion(&mut ready, "G1 X10 Y0 E1");

        // then
        assert_eq!(
            (m.start.e, m.end.e, m.e_delta, m.e_commanded, m.relative_e),
            (Some(0.0), Some(1.0), Some(1.0), true, true)
        );
    }

    #[rstest]
    fn relative_extrusion_delta_is_known_without_an_origin() {
        // given a relative E mode with no established origin
        let mut state = State::new();
        step(&mut state, "G92 X0 Y0 Z0");
        step(&mut state, "M83");

        // when
        let m = motion(&mut state, "G1 X10 Y0 E1");

        // then the commanded amount is known even though the absolute end is not
        assert_eq!((m.start.e, m.end.e, m.e_delta), (None, None, Some(1.0)));
    }

    #[rstest]
    fn a_zero_length_move_is_still_interpreted(mut ready: State) {
        // when
        let m = motion(&mut ready, "G1 X0 Y0 E1");

        // then the state machine reports the motion; weldability is policy
        assert_eq!(m.start.xyz, m.end.xyz);
    }

    #[rstest]
    fn a_z_change_is_still_interpreted(mut ready: State) {
        // when
        let m = motion(&mut ready, "G1 X10 Y0 Z0.5 E1");

        // then
        assert_eq!(m.end.xyz, [Some(10.0), Some(0.0), Some(0.5)]);
    }

    #[rstest]
    fn a_move_from_an_unknown_position_has_an_unknown_start() {
        // given
        let mut state = State::new();

        // when
        let first = motion(&mut state, "G1 X10 Y0 Z0.2 E1 F1200");

        // then
        assert_eq!(first.start.xyz, [None; 3]);
        assert_eq!(first.end.xyz, [Some(10.0), Some(0.0), Some(0.2)]);
        assert_eq!(
            (first.start.e, first.end.e, first.e_delta),
            (None, Some(1.0), None)
        );

        // then the established position anchors the next move
        let second = motion(&mut state, "G1 X20 Y0 E2");
        assert_eq!(second.start.xyz, [Some(10.0), Some(0.0), Some(0.2)]);
        assert_eq!(
            (second.start.e, second.end.e, second.e_delta),
            (Some(1.0), Some(2.0), Some(1.0))
        );
    }

    #[rstest]
    fn a_motion_without_a_known_feed_reports_no_feed() {
        // given a fully known position without a feedrate
        let mut state = State::new();
        step(&mut state, "G92 X0 Y0 Z0.2 E0");

        // when
        let m = motion(&mut state, "G1 X10 Y0 E1");

        // then
        assert_eq!(m.feed, None);
    }

    #[rstest]
    #[case::clockwise_arc(
        "G2 X10 Y0 I5 J5 F900 E1",
        MoveKind::Arc { clockwise: true }
    )]
    #[case::counterclockwise_arc(
        "G3 X10 Y0 I5 J5 F900 E1",
        MoveKind::Arc { clockwise: false }
    )]
    #[case::radius_arc(
        "G2 X10 Y0 R8 F900 E1",
        MoveKind::Arc { clockwise: true }
    )]
    #[case::spline("G5 X10 Y0 I5 J5 P5 Q5 F900 E1", MoveKind::Spline)]
    fn curves_are_interpreted_with_their_kind(
        #[case] line: &str,
        #[case] kind: MoveKind,
        mut ready: State,
    ) {
        // when
        let curve = motion(&mut ready, line);

        // then the curve is a trusted motion that does not reset the frame
        assert_eq!(curve.kind, kind);
        assert_eq!(curve.start.xyz, [Some(0.0), Some(0.0), Some(0.2)]);
        assert_eq!(curve.end.xyz, [Some(10.0), Some(0.0), Some(0.2)]);
        assert_eq!(curve.feed, Some(900.0));

        // then its endpoint anchors the next run
        let next = motion(&mut ready, "G1 X20 Y0 E2");
        assert_eq!(next.start.xyz, [Some(10.0), Some(0.0), Some(0.2)]);
    }

    #[rstest]
    #[case::arc_center_form(
        "G2 X10 Y0 I5 J5 E1",
        CurveKind::Arc { clockwise: true, i: 5.0, j: 5.0, r: None }
    )]
    #[case::arc_radius_form(
        "G2 X10 Y0 R8 E1",
        CurveKind::Arc { clockwise: true, i: 0.0, j: 0.0, r: Some(8.0) }
    )]
    #[case::spline(
        "G5 X10 Y0 I5 J5 P5 Q5 E1",
        CurveKind::Spline { i: 5.0, j: 5.0, p: 5.0, q: 5.0 }
    )]
    fn curve_offsets_are_read_from_the_line(
        #[case] line: &str,
        #[case] expected: CurveKind,
        mut ready: State,
    ) {
        // when
        let curve = motion(&mut ready, line);

        // then
        assert_eq!(curve.curve, Some(expected));
    }

    #[rstest]
    fn curve_offsets_are_converted_to_millimetres(mut ready: State) {
        // given inch units
        step(&mut ready, "G20");

        // when
        let arc = motion(&mut ready, "G2 X1 Y0 I0.5 J0.5 E0.04");

        // then
        assert_eq!(
            arc.curve,
            Some(CurveKind::Arc {
                clockwise: true,
                i: 12.7,
                j: 12.7,
                r: None,
            })
        );
    }

    #[rstest]
    fn an_implicit_curve_line_has_no_offsets(mut ready: State) {
        // given a modal arc command
        step(&mut ready, "G2 X0 Y10 I0 J5 E1");

        // when the next line inherits the motion kind
        let implicit = motion(&mut ready, "X0 Y20 E2");

        // then the offsets are not modal and cannot be recovered
        assert_eq!(implicit.kind, MoveKind::Arc { clockwise: true });
        assert_eq!(implicit.curve, None);
    }

    #[rstest]
    fn a_full_circle_is_interpreted(mut ready: State) {
        // when
        let curve = motion(&mut ready, "G2 X0 Y0 I0 J5 E1");

        // then a curve needs no XY motion, unlike a weldable linear run
        assert_eq!(curve.start.xyz, curve.end.xyz);
        assert_eq!(curve.kind, MoveKind::Arc { clockwise: true });
    }

    #[rstest]
    fn a_helical_curve_reports_the_new_z(mut ready: State) {
        // when
        let curve = motion(&mut ready, "G2 X10 Y0 Z0.4 I5 J5 E1");

        // then the raised Z anchors the linear runs that follow
        assert_eq!(curve.end.xyz, [Some(10.0), Some(0.0), Some(0.4)]);

        let next = motion(&mut ready, "G1 X20 Y0 E2");
        assert_eq!(next.start.xyz, [Some(10.0), Some(0.0), Some(0.4)]);
    }

    #[rstest]
    fn a_commented_move_is_still_interpreted(mut ready: State) {
        // when
        let m = motion(&mut ready, "G1 X10 Y0 E1 ; note");

        // then the comment blocks rewriting, not interpretation
        assert_eq!(m.raw, "G1 X10 Y0 E1 ; note");
        assert_eq!(m.end.xyz, [Some(10.0), Some(0.0), Some(0.2)]);
    }

    #[rstest]
    fn text_message_leaves_state_intact(mut ready: State) {
        // when
        let message = step(&mut ready, "M117 hello");

        // then
        assert_eq!(message, Outcome::Ignored);

        let m = motion(&mut ready, "G1 X10 Y0 E1");
        assert_eq!(m.start.xyz, [Some(0.0), Some(0.0), Some(0.2)]);
    }

    #[rstest]
    fn relative_positioning_is_reported_on_the_move(mut ready: State) {
        // given
        step(&mut ready, "G91");

        // when
        let m = motion(&mut ready, "G1 X5 Y2 E1");

        // then G91 makes E relative as well
        assert_eq!(m.start.xyz, [Some(0.0), Some(0.0), Some(0.2)]);
        assert_eq!(m.end.xyz, [Some(5.0), Some(2.0), Some(0.2)]);
        assert!(m.relative_e);
        assert_eq!(m.e_delta, Some(1.0));
    }

    #[rstest]
    fn absolute_positioning_restores_absolute_moves(mut ready: State) {
        step(&mut ready, "G91");
        step(&mut ready, "G1 X5 Y2 E1");
        step(&mut ready, "G90");

        let m = motion(&mut ready, "G1 X10 Y0 E2");

        assert_eq!(m.start.xyz, [Some(5.0), Some(2.0), Some(0.2)]);
        assert_eq!(m.end.xyz, [Some(10.0), Some(0.0), Some(0.2)]);
        assert!(!m.relative_e);
        assert_eq!(
            (m.start.e, m.end.e, m.e_delta),
            (Some(1.0), Some(2.0), Some(1.0))
        );
    }

    #[rstest]
    fn m83_toggles_relative_extrusion_independently(mut ready: State) {
        step(&mut ready, "M83");
        let relative = motion(&mut ready, "G1 E2");

        assert!(!ready.relative);
        assert!(relative.relative_e);
        assert_eq!(relative.e_delta, Some(2.0));
        assert_eq!(ready.position.e, Some(2.0));

        step(&mut ready, "M82");
        let absolute = motion(&mut ready, "G1 E5");

        assert!(!ready.relative);
        assert!(!absolute.relative_e);
        assert_eq!(ready.position.e, Some(5.0));
    }

    #[rstest]
    fn units_are_converted_to_millimetres(mut ready: State) {
        // given
        step(&mut ready, "G20");

        // when
        let m = motion(&mut ready, "G1 X1 Y1 Z0.04 E0.04");

        // then
        assert_eq!(m.end.xyz, [Some(25.4), Some(25.4), Some(1.016)]);
        assert_eq!(m.e_delta, Some(1.016));
    }

    #[rstest]
    fn a_units_switch_keeps_the_reference_frame(mut ready: State) {
        // given an inch move that established the position
        step(&mut ready, "G20");
        step(&mut ready, "G1 X1 Y0 Z0.04 E0.04");

        // when
        step(&mut ready, "G21");

        // then the stored millimetre position anchors the next move
        let m = motion(&mut ready, "G1 X30 Y0 E1");
        assert_eq!(m.start.xyz, [Some(25.4), Some(0.0), Some(1.016)]);
    }

    #[rstest]
    fn plane_selection_only_sets_the_flag(mut ready: State) {
        // when
        step(&mut ready, "G18");

        // then interpretation continues; the optimizer applies the plane gate
        assert!(!ready.xy);
        let m = motion(&mut ready, "G1 X10 Y0 E1");
        assert_eq!(m.end.xyz, [Some(10.0), Some(0.0), Some(0.2)]);
    }

    #[rstest]
    fn g92_sets_only_listed_axes(mut ready: State) {
        // given
        step(&mut ready, "G92 X10 E5");

        // then
        assert_eq!(ready.position.xyz, [Some(10.0), Some(0.0), Some(0.2)]);
        assert_eq!(ready.position.e, Some(5.0));
    }

    #[rstest]
    fn g92_does_not_reset_the_feed(mut ready: State) {
        // when
        step(&mut ready, "G92 X10 E5");

        // then
        let m = motion(&mut ready, "G1 X20 Y0 E6");
        assert_eq!(m.feed, Some(1200.0));
    }

    #[rstest]
    #[case::hotend_temperature("M104 S200")]
    #[case::dwell("G4 P100")]
    #[case::report_temperature("M105")]
    fn noop_commands_leave_state_intact(mut ready: State, #[case] line: &str) {
        // when
        let command = step(&mut ready, line);

        // then
        assert_eq!(command, Outcome::Ignored);

        let m = motion(&mut ready, "G1 X10 Y0 E1");
        assert_eq!(m.start.xyz, [Some(0.0), Some(0.0), Some(0.2)]);
    }

    #[rstest]
    #[case::home("G28")]
    #[case::tool_change("T0")]
    #[case::disable_steppers("M84")]
    fn reset_commands_clear_position_and_feed(mut ready: State, #[case] line: &str) {
        // when
        let command = step(&mut ready, line);

        // then
        assert_eq!(command, Outcome::Ignored);
        assert_eq!(ready.position.xyz, [None; 3]);
        assert_eq!(ready.position.e, None);
        assert_eq!(ready.feed, None);
    }

    #[rstest]
    fn an_unparsable_line_clears_the_reference_frame(mut ready: State) {
        // when
        let outcome = step(&mut ready, "G1 X");

        // then
        assert_eq!(outcome, Outcome::Unsupported);
        assert_eq!(ready.position.xyz, [None; 3]);
        assert_eq!(ready.feed, None);
    }

    #[rstest]
    #[case::unrecognized_command("G93")]
    #[case::bare_set_position("G92")]
    #[case::positioning_with_parameters("G90 X0")]
    fn unrecognized_commands_clear_the_reference_frame(mut ready: State, #[case] line: &str) {
        // when
        let outcome = step(&mut ready, line);

        // then
        assert_eq!(outcome, Outcome::Unsupported);
        assert_eq!(ready.position.xyz, [None; 3]);
    }

    #[rstest]
    fn unsupported_parameters_do_not_disturb_interpretation(mut ready: State) {
        // when
        let m = motion(&mut ready, "G1 X10 Y0 E1 S100");

        // then the syntax gate belongs to the optimizer
        assert_eq!(m.end.xyz, [Some(10.0), Some(0.0), Some(0.2)]);
    }

    #[rstest]
    fn implicit_motion_uses_the_modal_command() {
        // given a file that established G0
        let mut state = State::new();
        step(&mut state, "G0 X1 Y1 Z1");

        // when
        let m = motion(&mut state, "X2 Y2 Z2");

        // then
        assert_eq!(m.start.xyz, [Some(1.0), Some(1.0), Some(1.0)]);
        assert_eq!(m.end.xyz, [Some(2.0), Some(2.0), Some(2.0)]);
        assert_eq!(m.kind, MoveKind::Rapid);
        assert!(m.implicit);
    }

    #[rstest]
    fn implicit_arc_uses_the_modal_kind(mut ready: State) {
        // given
        step(&mut ready, "G2 X0 Y10 I0 J5 E1");

        // when
        let m = motion(&mut ready, "X0 Y20 I0 J5 E2");

        // then
        assert_eq!(m.start.xyz, [Some(0.0), Some(10.0), Some(0.2)]);
        assert_eq!(m.end.xyz, [Some(0.0), Some(20.0), Some(0.2)]);
        assert_eq!(m.kind, MoveKind::Arc { clockwise: true });
        assert!(m.implicit);
    }

    #[rstest]
    fn implicit_motion_without_a_modal_command_is_unsupported() {
        // given
        let mut state = State::new();

        // when
        let outcome = step(&mut state, "X5 Y5 Z0");

        // then
        assert_eq!(outcome, Outcome::Unsupported);
        assert_eq!(state.position.xyz, [None; 3]);
    }

    #[rstest]
    fn a_feed_only_motion_updates_the_feed_without_a_motion(mut ready: State) {
        // when
        let command = step(&mut ready, "G1 F900");

        // then
        assert_eq!(command, Outcome::Ignored);
        assert_eq!(ready.feed, Some(900.0));

        // then an implicit feed line also updates the mode
        let implicit = step(&mut ready, "F950");
        assert_eq!(implicit, Outcome::Ignored);
        assert_eq!(ready.feed, Some(950.0));
    }

    #[rstest]
    fn a_bare_motion_command_is_ignored(mut ready: State) {
        // when
        let command = step(&mut ready, "G1");

        // then
        assert_eq!(command, Outcome::Ignored);
        assert_eq!(ready.feed, Some(1200.0));
    }
}
