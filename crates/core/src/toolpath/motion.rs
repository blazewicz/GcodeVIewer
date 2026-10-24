//! Toolpath model shared by the state machine, assembler, geometry, and welder.
//!
//! [`Move`] is the assembler's weldable reading of one
//! [`Motion`](crate::state::Motion): a trusted motion with both endpoints and
//! its extrusion, linear or curved. [`Piece`] groups adjacent moves that may be
//! welded while carrying unparsable lines through verbatim.

use crate::gcode::MoveKind;

use super::point::Point;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Move<'a> {
    pub(crate) start: Point,
    pub(crate) end: Point,
    pub(crate) e0: f64,
    pub(crate) e1: f64,
    pub(crate) relative_e: bool,
    pub(crate) feed: f64,
    pub(crate) kind: MoveKind,
    pub(crate) raw: &'a str,
}

impl Move<'_> {
    /// Whether `next` may share a run with this move. Motion kind, feed,
    /// extrusion mode, and extrusion direction are the properties a curve
    /// cannot span, so a change in any of them starts a new run.
    pub(crate) fn continues(&self, next: &Move<'_>) -> bool {
        self.feed == next.feed
            && self.kind == next.kind
            && self.relative_e == next.relative_e
            && (self.e1 > self.e0) == (next.e1 > next.e0)
    }
}

pub(crate) enum Piece<'a> {
    /// A single line to be preserved verbatim.
    Raw(&'a str),
    /// A sequence of linear moves.
    Run(Vec<Move<'a>>),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::move_between;

    fn a() -> Move<'static> {
        let mut m = move_between(Point(0.0, 0.0), Point(1.0, 0.0), "");
        m.e1 = 1.0;
        m
    }

    #[test]
    fn matching_moves_continue_a_run() {
        assert!(a().continues(&a()));
    }

    #[test]
    fn each_mismatched_property_starts_a_new_run() {
        let base = a();

        let mut feed = a();
        feed.feed = 900.0;
        assert!(!base.continues(&feed));

        let mut rapid = a();
        rapid.kind = MoveKind::Rapid;
        assert!(!base.continues(&rapid));

        let mut relative_e = a();
        relative_e.relative_e = true;
        assert!(!base.continues(&relative_e));

        let mut retract = a();
        retract.e1 = -1.0;
        assert!(!base.continues(&retract));
    }
}
