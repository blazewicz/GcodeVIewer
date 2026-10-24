//! Turns the modal state machine's output into continuous move sequences.

use crate::Options;
use crate::gcode::{Chunks, InterpretedMotion, Line, MoveKind, Outcome, State, SyntaxError};
use crate::toolpath::Point;

use super::motion::{Move, Piece};

/// Why welding stopped at a line.
enum Cause {
    /// The strict lexer rejected the line's syntax.
    Syntax(SyntaxError),
    /// The command is outside the understood grammar.
    Command,
    /// The command is understood but carries parameters a generated curve
    /// cannot preserve.
    Parameters,
}

impl std::fmt::Display for Cause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Syntax(error) => write!(f, "{error}"),
            Self::Command => f.write_str("unsupported command"),
            Self::Parameters => f.write_str("unsupported parameters"),
        }
    }
}

/// Turns a stream of lexed lines into pieces.
///
/// The assembler owns the modal [`State`], the trust flag, the pending run, and
/// a running 1-based line number for reporting where welding stopped. The
/// trailing run stays buffered across calls so a run is never broken at a chunk
/// boundary; [`Assembler::finish`] releases it once the whole input has been
/// fed.
///
/// A line without its own command can inherit a motion from the modal state.
/// Welding replaces moves with a curve that rebinds the modal motion command,
/// so such a line would change meaning after any replacement. There is no safe
/// way to weld only the part of the file before it, so the assembler flags the
/// whole file incompatible and the caller discards the output.
pub(crate) struct Assembler<'a> {
    state: State,
    trusted: bool,
    run: Vec<Move<'a>>,
    line_number: usize,
    compatible: bool,
}

impl<'a> Assembler<'a> {
    pub(crate) fn new() -> Self {
        Self {
            state: State::new(),
            trusted: true,
            run: Vec::new(),
            line_number: 0,
            compatible: true,
        }
    }

    /// Whether the input can be welded at all. Once false, the caller must
    /// discard every piece the assembler produced.
    pub(crate) fn compatible(&self) -> bool {
        self.compatible
    }

    /// Feeds lexed `lines` through the modal state machine.
    pub(crate) fn assemble<I>(&mut self, lines: I, opts: &Options) -> Vec<Piece<'a>>
    where
        I: Iterator<Item = Line<'a>>,
    {
        let mut pieces = Vec::new();

        for line in lines {
            self.line_number += 1;

            let outcome = self.state.advance(&line);

            if !line.has_command {
                let inherited = matches!(&outcome, Outcome::Motion(m) if m.implicit);
                self.note_incompatible(&line, inherited);
                break;
            }

            let converted = match outcome {
                Outcome::Motion(m) => self.move_from(&m, &line, opts),
                Outcome::Unsupported => {
                    let cause = match &line.parsed {
                        Err(error) => Cause::Syntax(*error),
                        Ok(_) => Cause::Command,
                    };
                    self.distrust(&line, cause);
                    None
                }
                Outcome::Ignored => None,
            };

            match converted {
                Some(m) if m.kind.is_linear() => {
                    let compatible = self.run.last().is_none_or(|p| p.continues(&m));

                    if !compatible || self.run.len() >= opts.max_segments {
                        pieces.push(Piece::Run(std::mem::take(&mut self.run)));
                    }

                    self.run.push(m);
                }

                // Curves move the machine but are never welded, so like every
                // other line the state machine cannot turn into a linear move
                // they end the run and pass through verbatim.
                _ => {
                    if !self.run.is_empty() {
                        pieces.push(Piece::Run(std::mem::take(&mut self.run)));
                    }

                    pieces.push(Piece::Raw(line.raw));
                }
            }
        }

        pieces
    }

    /// Stops welding everything after `line` and reports the first line that
    /// caused it. Once trust is gone there is nothing left to disable, so later
    /// causes are not reported.
    fn distrust(&mut self, line: &Line<'_>, cause: Cause) {
        if !self.trusted {
            return;
        }
        self.trusted = false;

        log::warn!(
            target: "gcode_viewer",
            "optimization stops at line {}: {cause}: {}",
            self.line_number,
            line.raw.trim_end_matches(['\n', '\r'])
        );
    }

    /// Marks the whole file incompatible and reports the first line that did
    /// it, distinguishing a motion inherited from the modal state from a line
    /// the lexer cannot attribute to any command (e.g. a numbered block).
    fn note_incompatible(&mut self, line: &Line<'_>, inherited_motion: bool) {
        if !self.compatible {
            return;
        }
        self.compatible = false;

        let cause = if inherited_motion {
            "inherits the modal motion command"
        } else {
            "has no command"
        };
        log::warn!(
            target: "gcode_viewer",
            "file cannot be optimized: line {} {cause}: {}",
            self.line_number,
            line.raw.trim_end_matches(['\n', '\r'])
        );
    }

    /// Applies the optimizer's syntax and frame policy to an interpreted
    /// motion. Parameters outside the understood set disable the rest of the
    /// file, because a generated curve cannot be trusted to preserve them.
    fn move_from(
        &mut self,
        m: &InterpretedMotion<'a>,
        line: &Line<'a>,
        opts: &Options,
    ) -> Option<Move<'a>> {
        if !self.trusted {
            return None;
        }

        let words = line.parsed.as_ref().ok()?;

        let allowed: &[u8] = if m.kind.is_linear() {
            b"XYZEF"
        } else {
            b"XYZEFIJKPQR"
        };

        if !words.params_within(allowed) {
            self.distrust(line, Cause::Parameters);
            return None;
        }

        // G0 may use a separately configured rapid feedrate. Without the
        // shared-feedrate travel opt-in, wait for an explicit G1 F before
        // trusting the print feedrate.
        if m.kind == MoveKind::Rapid && !opts.travel {
            self.state.clear_feed();
        }

        // Lines with comments are preserved as-is.
        if line.comment {
            return None;
        }

        self.weldable_motion(m, opts)
    }

    /// Builds the replacement [`Move`] for a motion, or [`None`] when a faithful
    /// replacement is impossible.
    fn weldable_motion(&self, m: &InterpretedMotion<'a>, opts: &Options) -> Option<Move<'a>> {
        if self.state.relative()
            || !self.state.mm() // TODO: state supports unit change, do we still need this condition?
            || !self.state.xy()
            || (m.kind == MoveKind::Rapid && !opts.travel)
        {
            return None;
        }

        let [x, y, z] = m.start.zip_xyz()?;
        let [nx, ny, nz] = m.end.zip_xyz()?;
        let feed = m.feed?;
        let delta_e = m.e_delta?;

        let (e0, e1) = match m.start.e.zip(m.end.e) {
            Some(endpoints) => endpoints,
            // For extruding moves both absolute E endpoints must be known.
            None if m.e_commanded => return None,
            // Non-extruding moves emit no E word; use placeholders with zero delta.
            None => (0.0, 0.0),
        };

        let weldable = !m.kind.is_linear()
            || (z == nz && (delta_e > 0.0 || opts.travel) && (x != nx || y != ny));

        let is_within_bounds = [x, y, nx, ny].iter().all(|v| v.abs() <= 1e6);

        (is_within_bounds && feed > 0.0 && delta_e >= 0.0 && weldable).then_some(Move {
            start: Point(x, y),
            end: Point(nx, ny),
            e0,
            e1,
            relative_e: m.relative_e,
            feed,
            kind: m.kind,
            raw: m.raw,
        })
    }

    /// Releases the run buffered across the last batch, if any.
    pub(crate) fn finish(mut self) -> Option<Piece<'a>> {
        (!self.run.is_empty()).then_some(Piece::Run(std::mem::take(&mut self.run)))
    }
}

/// Drives the modal state machine over `text` in chunks, handing each batch of
/// pieces to `consume`; `false` from `consume` stops early.
///
/// Returns `false` when the caller must discard the output: the input contains
/// a line without its own command, or `consume` stopped the pass. Welding a
/// file with an inherited command would change its meaning, so the caller
/// discards every piece, including the ones already consumed.
#[allow(dead_code)]
pub(crate) fn assemble_batches<'a, I, F>(
    text: &'a str,
    chunk_bytes: usize,
    opts: &Options,
    decode: F,
    mut consume: impl FnMut(Vec<Piece<'a>>) -> bool,
) -> bool
where
    F: Fn(&'a str) -> I,
    I: Iterator<Item = Line<'a>>,
{
    let mut assembler = Assembler::new();

    for chunk in Chunks::new(text, chunk_bytes) {
        let pieces = assembler.assemble(decode(chunk), opts);

        if !assembler.compatible() {
            return false;
        }

        if !consume(pieces) {
            return false;
        }
    }

    if let Some(piece) = assembler.finish() {
        consume(vec![piece]);
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{options, travel_options};
    use indoc::indoc;
    use rstest::rstest;

    /// The length of each piece: `0` for a raw line, the move count for a run.
    fn shape(pieces: &[Piece<'_>]) -> Vec<usize> {
        pieces
            .iter()
            .map(|p| match p {
                Piece::Run(moves) => moves.len(),
                Piece::Raw(_) => 0,
            })
            .collect()
    }

    fn assemble_from_text<'a>(text: &'a str, opts: &Options) -> Vec<Piece<'a>> {
        let mut assembler = Assembler::new();

        let mut pieces = assembler.assemble(text.split_inclusive('\n').map(Line::decode), opts);

        if let Some(piece) = assembler.finish() {
            pieces.push(piece);
        }

        pieces
    }

    /// Establishes absolute positioning, millimetres, absolute extrusion, a
    /// print feedrate, and a known position. Its own `G1` emits no move because
    /// the starting position is unknown, so it stays a raw line.
    const PREAMBLE: &str = "G21\nG90\nM82\nG1 X0 Y0 Z0.2 F1200 E0\n";

    #[test]
    fn adjacent_moves_form_a_single_run() {
        let text = format!(
            "{PREAMBLE}{}",
            indoc! {"
                G1 X1 Y0 E1
                G1 X2 Y0 E2
                G1 X3 Y0 E3
            "}
        );
        assert_eq!(
            shape(&assemble_from_text(&text, &options("both"))),
            [0, 0, 0, 0, 3]
        );
    }

    #[test]
    fn a_non_move_line_splits_runs() {
        let text = format!(
            "{PREAMBLE}{}",
            indoc! {"
                G1 X1 Y0 E1
                G1 X2 Y0 E2
                M104 S200
                G1 X3 Y0 E3
                G1 X4 Y0 E4
            "}
        );
        // `M104` is recognized as a no-op, so it neither poisons trust nor
        // belongs to a run: it is carried verbatim between two runs.
        assert_eq!(
            shape(&assemble_from_text(&text, &options("both"))),
            [0, 0, 0, 0, 2, 0, 2]
        );
    }

    #[test]
    fn a_blank_line_splits_runs() {
        let text = format!(
            "{PREAMBLE}{}",
            indoc! {"
                G1 X1 Y0 E1

                G1 X2 Y0 E2
            "}
        );
        assert_eq!(
            shape(&assemble_from_text(&text, &options("both"))),
            [0, 0, 0, 0, 1, 0, 1]
        );
    }

    #[test]
    fn a_feed_change_splits_runs() {
        let text = format!(
            "{PREAMBLE}{}",
            indoc! {"
                G1 X1 Y0 E1
                G1 X2 Y0 E2 F900
                G1 X3 Y0 E3
            "}
        );
        assert_eq!(
            shape(&assemble_from_text(&text, &options("both"))),
            [0, 0, 0, 0, 1, 2]
        );
    }

    #[test]
    fn a_rapid_change_splits_runs() {
        let text = format!(
            "{PREAMBLE}{}",
            indoc! {"
                G1 X1 Y0 E1
                G0 X2 Y0
                G1 X3 Y0 E2
            "}
        );
        assert_eq!(
            shape(&assemble_from_text(&text, &travel_options("both"))),
            [0, 0, 0, 0, 1, 1, 1]
        );
    }

    #[test]
    fn a_travel_move_splits_extrusion_runs() {
        let text = format!(
            "{PREAMBLE}{}",
            indoc! {"
                G1 X1 Y0 E1
                G1 X2 Y0
                G1 X3 Y0 E2
            "}
        );
        // A move without an `E` word extrudes nothing while the surrounding
        // moves do, so it cannot share their curve.
        assert_eq!(
            shape(&assemble_from_text(&text, &travel_options("both"))),
            [0, 0, 0, 0, 1, 1, 1]
        );
    }

    #[test]
    fn a_relative_extrusion_change_splits_runs() {
        let text = format!(
            "{PREAMBLE}{}",
            indoc! {"
                G1 X1 Y0 E1
                M83
                G1 X2 Y0 E1
            "}
        );
        assert_eq!(
            shape(&assemble_from_text(&text, &options("both"))),
            [0, 0, 0, 0, 1, 0, 1]
        );
    }

    #[test]
    fn a_curve_splits_runs_and_is_preserved_verbatim() {
        let text = format!(
            "{PREAMBLE}{}",
            indoc! {"
                G1 X1 Y0 E1
                G2 X2 Y1 I1 J0 E2
                G1 X3 Y2 E3
            "}
        );
        // The curve is a trusted motion but never joins a run: it ends the run
        // before it and anchors the one after it at its endpoint.
        assert_eq!(
            shape(&assemble_from_text(&text, &options("both"))),
            [0, 0, 0, 0, 1, 0, 1]
        );
    }

    #[test]
    fn max_segments_caps_each_run() {
        let text = format!(
            "{PREAMBLE}{}",
            indoc! {"
                G1 X1 Y0 E1
                G1 X2 Y0 E2
                G1 X3 Y0 E3
                G1 X4 Y0 E4
                G1 X5 Y0 E5
                G1 X6 Y0 E6
                G1 X7 Y0 E7
            "}
        );
        let o = Options {
            max_segments: 3,
            ..options("both")
        };
        assert_eq!(shape(&assemble_from_text(&text, &o)), [0, 0, 0, 0, 3, 3, 1]);
    }

    #[test]
    fn empty_input_yields_no_pieces() {
        assert!(assemble_from_text("", &options("both")).is_empty());
    }

    #[test]
    fn a_zero_length_move_splits_runs() {
        let text = format!(
            "{PREAMBLE}{}",
            indoc! {"
                G1 X1 Y0 E1
                G1 X1 Y0 E2
                G1 X2 Y0 E3
            "}
        );
        // A move with no XY motion cannot share a curve with its neighbours.
        assert_eq!(
            shape(&assemble_from_text(&text, &options("both"))),
            [0, 0, 0, 0, 1, 0, 1]
        );
    }

    #[test]
    fn a_z_change_splits_runs() {
        let text = format!(
            "{PREAMBLE}{}",
            indoc! {"
                G1 X1 Y0 E1
                G1 X2 Y0 Z0.5 E2
                G1 X3 Y0 E3
            "}
        );
        assert_eq!(
            shape(&assemble_from_text(&text, &options("both"))),
            [0, 0, 0, 0, 1, 0, 1]
        );
    }

    #[test]
    fn a_retraction_splits_runs() {
        let text = format!(
            "{PREAMBLE}{}",
            indoc! {"
                G1 X1 Y0 E1
                G1 X2 Y0 E0
                G1 X3 Y0 E3
            "}
        );
        assert_eq!(
            shape(&assemble_from_text(&text, &options("both"))),
            [0, 0, 0, 0, 1, 0, 1]
        );
    }

    #[test]
    fn an_out_of_range_axis_is_not_welded() {
        let text = format!(
            "{PREAMBLE}{}",
            indoc! {"
                G1 X1 Y0 E1
                G1 X1000001 Y0 E2
            "}
        );
        assert_eq!(
            shape(&assemble_from_text(&text, &options("both"))),
            [0, 0, 0, 0, 1, 0]
        );
    }

    #[test]
    fn a_non_positive_feed_leaves_later_moves_unweldable() {
        let text = format!(
            "{PREAMBLE}{}",
            indoc! {"
                G1 X1 Y0 E1
                G1 X2 Y0 E2 F0
                G1 X3 Y0 E3
            "}
        );
        assert_eq!(
            shape(&assemble_from_text(&text, &options("both"))),
            [0, 0, 0, 0, 1, 0, 0]
        );
    }

    #[rstest]
    #[case::unsupported_parameter("G1 X1 Y0 E1 S100")]
    #[case::unsupported_arc_parameter("G2 X1 Y0 I1 J1 S100")]
    #[case::unrecognized_command("G93")]
    #[case::unparsable("G1 X")]
    fn poisoned_trust_disables_the_rest_of_the_file(#[case] line: &str) {
        let text = format!("{PREAMBLE}{line}\nG1 X2 Y0 E2\nG1 X3 Y0 E3\n");

        assert_eq!(
            shape(&assemble_from_text(&text, &options("both"))),
            [0, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn a_commented_move_is_carried_and_anchors_the_next() {
        let text = format!(
            "{PREAMBLE}{}",
            indoc! {"
                G1 X1 Y0 E1 ; note
                G1 X2 Y0 E2
            "}
        );
        // The commented move cannot be rewritten, but its endpoint still
        // anchors the run after it.
        assert_eq!(
            shape(&assemble_from_text(&text, &options("both"))),
            [0, 0, 0, 0, 0, 1]
        );
    }

    #[rstest]
    #[case::inches("G20")]
    #[case::relative("G91")]
    #[case::xz_plane("G18")]
    fn unweldable_modes_split_runs(#[case] mode: &str) {
        let text = format!(
            "{PREAMBLE}{mode}\n{}",
            indoc! {"
                G1 X1 Y0 E1
                G1 X2 Y0 E2
            "}
        );

        assert_eq!(
            shape(&assemble_from_text(&text, &options("both"))),
            [0, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn relative_extrusion_without_an_origin_is_not_welded() {
        // M83 deltas are known, but a curve must emit absolute E endpoints, so
        // the moves stay raw until an origin is established.
        let text = indoc! {"
            G21
            G90
            M83
            G92 X0 Y0 Z0
            G1 X1 Y0 E1 F1200
            G1 X2 Y0 E1
            G1 X3 Y0 E1
        "};

        assert_eq!(
            shape(&assemble_from_text(text, &options("both"))),
            [0, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn a_rapid_without_travel_clears_the_print_feed() {
        let text = format!(
            "{PREAMBLE}{}",
            indoc! {"
                G1 X1 Y0 E1
                G0 X5 Y0
                G1 X6 Y0 E2
            "}
        );
        // The rapid is not trusted for welding, and the print feed it carried
        // must not be attributed to the move after it.
        assert_eq!(
            shape(&assemble_from_text(&text, &options("both"))),
            [0, 0, 0, 0, 1, 0, 0]
        );
    }

    #[test]
    fn a_rapid_with_travel_keeps_the_feed() {
        let text = format!(
            "{PREAMBLE}{}",
            indoc! {"
                G1 X1 Y0 E1
                G0 X5 Y0
                G1 X6 Y0 E2
            "}
        );
        assert_eq!(
            shape(&assemble_from_text(&text, &travel_options("both"))),
            [0, 0, 0, 0, 1, 1, 1]
        );
    }
}
