//! Motion records over G-code toolpaths.
//!
//! [`motions`] drives the shared [`State`](crate::state::State) modal
//! interpreter and reports every motion in the file. Unlike the optimizer
//! these queries apply no welding policy: travel, relative coordinates, inch
//! units, Z changes, comments, and existing curves all stay interpretable,
//! because a record only needs endpoints and shape, not geometry that must be
//! preserved.
//!
//! [`Scanner`] accepts the text in arbitrary chunks, so a host can stream a
//! file without holding all of it in memory. Only lines whose newline has
//! arrived are interpreted; the trailing partial line is carried over to the
//! next [`Scanner::feed`] call and released by [`Scanner::finish`].

use glam::DVec3;

use crate::gcode::{CurveKind, Line, MoveKind, Outcome, State};

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
}

/// Returns every motion in `text`, in file order.
pub fn motions(text: &str) -> Vec<MotionRecord> {
    let mut found = Vec::new();
    let mut scanner = Scanner::new();

    scanner.feed(text, &mut found);
    scanner.finish(&mut found);

    found
}

/// The modal interpreter for a file fed in chunks.
///
/// The scanner keeps the state that spans chunk boundaries: the modal
/// [`State`] and the partial line left at the tail of the last feed. The
/// caller owns the output vector, so the records of one chunk can be consumed
/// and discarded before the next chunk arrives.
pub struct Scanner {
    state: State,
    /// Text after the last newline of the previous feeds.
    pending: String,
    line_number: usize,
}

impl Default for Scanner {
    fn default() -> Self {
        Self::new()
    }
}

impl Scanner {
    pub fn new() -> Self {
        Self {
            state: State::new(),
            pending: String::new(),
            line_number: 0,
        }
    }

    /// Feeds the next chunk of text, appending every motion it completes to
    /// `out`. Only lines terminated by `\n` are interpreted; the trailing
    /// partial line is kept for the next call.
    pub fn feed(&mut self, text: &str, out: &mut Vec<MotionRecord>) {
        let mut rest = text;

        if !self.pending.is_empty() {
            match text.find('\n') {
                Some(end) => {
                    self.pending.push_str(&text[..=end]);
                    let line = std::mem::take(&mut self.pending);
                    self.push_line(&line, out);
                    rest = &text[end + 1..];
                }
                None => {
                    self.pending.push_str(text);
                    return;
                }
            }
        }

        for line in rest.split_inclusive('\n') {
            if line.ends_with('\n') {
                self.push_line(line, out);
            } else {
                self.pending.push_str(line);
            }
        }
    }

    /// Interprets the trailing line even when the input did not end with a
    /// newline. Call once, after the last [`Scanner::feed`].
    pub fn finish(&mut self, out: &mut Vec<MotionRecord>) {
        if !self.pending.is_empty() {
            let line = std::mem::take(&mut self.pending);
            self.push_line(&line, out);
        }
    }

    /// Advances the interpreter over one whole line.
    fn push_line(&mut self, raw: &str, out: &mut Vec<MotionRecord>) {
        self.line_number += 1;

        if let Outcome::Motion(m) = self.state.advance(&Line::decode(raw)) {
            out.push(MotionRecord {
                start: m.start.zip_xyz().map(DVec3::from_array),
                end: m.end.zip_xyz().map(DVec3::from_array),
                feed: m.feed,
                extrusion: m.e_delta,
                kind: m.kind,
                curve: m.curve,
                line_number: self.line_number,
            });
        }
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

    /// A file with every move kind, comments, and a non-ASCII comment.
    #[fixture]
    fn streamable() -> &'static str {
        indoc! {"
            G21
            G90
            M82
            G92 X0 Y0 Z0 E0
            G1 F1200
            G0 X1 Y0 Z0 ; liftoff
            G1 X2 Y0 Z0 E1
            G2 X3 Y1 Z0 I0.5 J0.5 E2
            G5 X4 Y2 Z0 I0.5 J0 P0 Q0.5 E3
            G1 X5 Y2 E4 ; już kończymy
        "}
    }

    /// Splits `text` into chunks of up to `chunk_bytes` bytes at character
    /// boundaries, the way a UTF-8 decoder hands a stream to the scanner.
    fn split(text: &str, chunk_bytes: usize) -> Vec<&str> {
        let mut rest = text;
        let mut chunks = Vec::new();

        while !rest.is_empty() {
            let mut cut = chunk_bytes.min(rest.len());

            while !rest.is_char_boundary(cut) {
                cut -= 1;
            }

            // A character wider than a chunk still has to make progress.
            if cut == 0 {
                cut = rest.char_indices().nth(1).map_or(rest.len(), |(i, _)| i);
            }

            chunks.push(&rest[..cut]);
            rest = &rest[cut..];
        }

        chunks
    }

    /// Feeds `source` through a scanner in `chunk_bytes`-sized chunks.
    fn streamed(source: &str, chunk_bytes: usize) -> Vec<MotionRecord> {
        let mut scanner = Scanner::new();
        let mut found = Vec::new();

        for chunk in split(source, chunk_bytes) {
            scanner.feed(chunk, &mut found);
        }
        scanner.finish(&mut found);

        found
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

    #[rstest]
    fn streaming_matches_one_shot_parsing_for_any_chunking(
        #[values(1, 2, 3, 7, 13, 64, 4096)] chunk_bytes: usize,
        streamable: &str,
    ) {
        assert_eq!(streamed(streamable, chunk_bytes), motions(streamable));
    }

    #[test]
    fn a_line_split_mid_text_is_interpreted_once_complete() {
        // given a scanner fed up to the middle of the last line
        let mut scanner = Scanner::new();
        let mut found = Vec::new();
        scanner.feed(
            "G21\nG90\nM82\nG92 X0 Y0 Z0 E0\nG1 F1200\nG1 X10",
            &mut found,
        );

        // when the rest of the line arrives
        assert!(found.is_empty());
        scanner.feed(" Y10 E1\n", &mut found);

        // then
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].end, Some(DVec3::new(10.0, 10.0, 0.0)));
    }

    #[test]
    fn a_trailing_line_without_a_newline_is_released_by_finish() {
        // given a scanner fed a file whose last line has no newline
        let mut scanner = Scanner::new();
        let mut found = Vec::new();
        scanner.feed("G21\nG90\nM82\nG92 X0 Y0 Z0 E0\nG1 F1200\n", &mut found);
        scanner.feed("G1 X4 Y0 E1", &mut found);

        // when
        assert!(found.is_empty());
        scanner.finish(&mut found);

        // then
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].end, Some(DVec3::new(4.0, 0.0, 0.0)));
    }

    #[test]
    fn empty_feeds_do_not_disturb_a_partial_line() {
        // given a scanner carrying a partial line
        let mut scanner = Scanner::new();
        let mut found = Vec::new();
        scanner.feed(
            "G21\nG90\nM82\nG92 X0 Y0 Z0 E0\nG1 F1200\nG1 X2",
            &mut found,
        );

        // when empty chunks arrive
        scanner.feed("", &mut found);
        scanner.feed(" Y0 E1\n", &mut found);

        // then the partial line was not lost
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].end, Some(DVec3::new(2.0, 0.0, 0.0)));
    }

    #[test]
    fn a_crlf_split_across_feeds_is_one_line() {
        // given a chunk ending between the carriage return and the newline
        let mut scanner = Scanner::new();
        let mut found = Vec::new();
        scanner.feed("G21\nG90\nM82\nG92 X0 Y0 Z0 E0\nG1 F1200\r", &mut found);

        // when the next chunk opens with the newline
        scanner.feed("\nG1 X3 Y0 E1\r\n", &mut found);

        // then the line was interpreted once, with its feedrate
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].end, Some(DVec3::new(3.0, 0.0, 0.0)));
        assert_eq!(found[0].feed, Some(1200.0));
    }

    #[test]
    fn line_numbers_count_across_feeds() {
        // given
        let mut scanner = Scanner::new();
        let mut found = Vec::new();
        scanner.feed("G21\nG90\nM82\n", &mut found);
        scanner.feed("G92 X0 Y0 Z0 E0\nG1 X1 Y0 E1 F1200\n", &mut found);
        scanner.finish(&mut found);

        // then the motion sits on the line the whole file puts it on
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].line_number, 5);
    }
}
