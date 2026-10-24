use std::sync::atomic::{AtomicBool, Ordering};

use rayon::prelude::*;

use crate::Options;
use crate::gcode::{Chunks, Line, Piece};

use super::{assemble::Assembler, welder};

/// Drives the modal state machine over `text` in chunks, handing each batch of
/// pieces to `consume`; `false` from `consume` stops early.
///
/// Returns `false` when the caller must discard the output: the input contains
/// a line without its own command, or `consume` stopped the pass. Welding a
/// file with an inherited command would change its meaning, so the caller
/// discards every piece, including the ones already consumed.
fn assemble_batches<'a, I, F>(
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

fn decode_lines<'a>(text: &'a str) -> impl Iterator<Item = Line<'a>> {
    text.split_inclusive('\n').map(Line::decode)
}

/// Weld every piece in order on the calling thread.
pub(crate) fn run_sequential(text: &str, opts: &Options) -> (String, usize) {
    run_sequential_with_chunk_bytes(text, opts, 256 * 1024)
}

fn run_sequential_with_chunk_bytes(
    text: &str,
    opts: &Options,
    chunk_bytes: usize,
) -> (String, usize) {
    let mut output = String::with_capacity(text.len());
    let mut count = 0usize;

    // TODO: We pass welder::weld as a closure, as if assembler was consumer agnostic,
    // but it isn't. Assembler has the weldable_motion method - already dependent on the
    // same consumer kind. We should move code to ensure that welding and assembly are
    // coupled at a single layer.
    let compatible = assemble_batches(text, chunk_bytes, opts, decode_lines, |pieces| {
        for piece in &pieces {
            let (part, n) = welder::weld(piece, opts);
            output.push_str(&part);
            count += n;
        }
        true
    });

    if !compatible {
        return (text.to_owned(), 0);
    }

    (output, count)
}

fn par_decode_lines<'a>(text: &'a str) -> impl Iterator<Item = Line<'a>> {
    // Materialize the raw lines so `par_iter` is indexed: an indexed collect
    // reserves the output once and writes each `Line` in place, whereas an
    // unindexed source makes rayon collect through per-task buffers and a final
    // append.
    text.split_inclusive('\n')
        .collect::<Vec<&str>>()
        .par_iter()
        .copied()
        .map(Line::decode)
        .collect::<Vec<Line>>()
        .into_iter()
}

/// Stream chunks through a two-stage pipeline: one producer decodes the lines
/// and runs the modal state machine (which is sequential), while welding fans
/// out over the pool. The bounded channel gives backpressure so only a few
/// chunks' tokens are alive at once.
pub(crate) fn run_multithreaded(text: &str, opts: &Options) -> (String, usize) {
    let workers = rayon::current_num_threads().max(1);
    // Enough chunks to balance the pool, but capped: beyond a point the
    // per-chunk orchestration costs more than the extra balance buys.
    let chunk_count = (workers * 8).clamp(1, 128);
    let chunk_bytes = (text.len() / chunk_count).clamp(256 * 1024, 8 * 1024 * 1024);

    run_multithreaded_with_chunk_bytes(text, opts, chunk_bytes)
}

fn run_multithreaded_with_chunk_bytes<'a>(
    text: &'a str,
    opts: &Options,
    chunk_bytes: usize,
) -> (String, usize) {
    let mut output = String::with_capacity(text.len());
    let mut count = 0usize;
    let compatible = AtomicBool::new(true);

    rayon::scope(|scope| {
        let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<Piece<'a>>>(4);
        let compatible = &compatible;

        scope.spawn(move |_| {
            let result = assemble_batches(text, chunk_bytes, opts, par_decode_lines, |pieces| {
                tx.send(pieces).is_ok()
            });
            compatible.store(result, Ordering::Relaxed);
        });

        for pieces in rx {
            let welded: Vec<_> = pieces
                .par_iter()
                .map(|piece| welder::weld(piece, opts))
                .collect();

            for (part, n) in welded {
                output.push_str(&part);
                count += n;
            }
        }
    });

    if !compatible.load(Ordering::Relaxed) {
        return (text.to_owned(), 0);
    }

    (output, count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::options;
    use indoc::indoc;
    use rstest::rstest;

    #[derive(Clone, Copy)]
    enum Backend {
        Sequential,
        Multithreaded,
    }

    impl Backend {
        fn run(self, text: &str, opts: &Options, chunk_bytes: usize) -> (String, usize) {
            match self {
                Backend::Sequential => run_sequential_with_chunk_bytes(text, opts, chunk_bytes),
                Backend::Multithreaded => {
                    run_multithreaded_with_chunk_bytes(text, opts, chunk_bytes)
                }
            }
        }
    }

    const PREAMBLE: &str = indoc! {"
        G21
        G90
        M82
        G92 E0
        G1 X20 Y0 Z0.2 F1200 E0
    "};

    /// A quarter turn from `(20, 0)` to `(0, 20)` at radius 20, as 20 chords:
    /// coarse enough to be worth welding, fine enough to meet tolerance 0.02.
    const ARC: &str = indoc! {"
        G1 X19.938346675 Y1.569181915 E0.800000000
        G1 X19.753766812 Y3.128689301 E1.600000000
        G1 X19.447398408 Y4.668907277 E2.400000000
        G1 X19.021130326 Y6.180339887 E3.200000000
        G1 X18.477590650 Y7.653668647 E4.000000000
        G1 X17.820130484 Y9.079809995 E4.800000000
        G1 X17.052803287 Y10.449971294 E5.600000000
        G1 X16.180339887 Y11.755705046 E6.400000000
        G1 X15.208119312 Y12.988960967 E7.200000000
        G1 X14.142135624 Y14.142135624 E8.000000000
        G1 X12.988960967 Y15.208119312 E8.800000000
        G1 X11.755705046 Y16.180339887 E9.600000000
        G1 X10.449971294 Y17.052803287 E10.400000000
        G1 X9.079809995 Y17.820130484 E11.200000000
        G1 X7.653668647 Y18.477590650 E12.000000000
        G1 X6.180339887 Y19.021130326 E12.800000000
        G1 X4.668907277 Y19.447398408 E13.600000000
        G1 X3.128689301 Y19.753766812 E14.400000000
        G1 X1.569181915 Y19.938346675 E15.200000000
        G1 X0.000000000 Y20.000000000 E16.000000000
    "};

    /// A second quarter turn continuing from `(0, 20)` to `(-20, 0)`, with the
    /// extrusion carried on from `ARC`.
    const ARC_CONTINUED: &str = indoc! {"
        G1 X-1.569181915 Y19.938346675 E16.800000000
        G1 X-3.128689301 Y19.753766812 E17.600000000
        G1 X-4.668907277 Y19.447398408 E18.400000000
        G1 X-6.180339887 Y19.021130326 E19.200000000
        G1 X-7.653668647 Y18.477590650 E20.000000000
        G1 X-9.079809995 Y17.820130484 E20.800000000
        G1 X-10.449971294 Y17.052803287 E21.600000000
        G1 X-11.755705046 Y16.180339887 E22.400000000
        G1 X-12.988960967 Y15.208119312 E23.200000000
        G1 X-14.142135624 Y14.142135624 E24.000000000
        G1 X-15.208119312 Y12.988960967 E24.800000000
        G1 X-16.180339887 Y11.755705046 E25.600000000
        G1 X-17.052803287 Y10.449971294 E26.400000000
        G1 X-17.820130484 Y9.079809995 E27.200000000
        G1 X-18.477590650 Y7.653668647 E28.000000000
        G1 X-19.021130326 Y6.180339887 E28.800000000
        G1 X-19.447398408 Y4.668907277 E29.600000000
        G1 X-19.753766812 Y3.128689301 E30.400000000
        G1 X-19.938346675 Y1.569181915 E31.200000000
        G1 X-20.000000000 Y0.000000000 E32.000000000
    "};

    /// One welded quarter turn, from the preamble through `ARC`.
    const WELDED_ARC: &str = indoc! {"
        G21
        G90
        M82
        G92 E0
        G1 X20 Y0 Z0.2 F1200 E0
        G3 I-20 J0 X0 Y20 F1200 E16
    "};

    /// Implicit motion: a line without a leading command letter.
    const IMPLICIT_MOTION: &str = indoc! {"
        G21
        G90
        X10 Y10
    "};

    /// A numbered block, which the optimizer does not understand.
    const NUMBERED_BLOCK: &str = indoc! {"
        G21
        G90
        1 G1 X0 Y0
    "};

    #[rstest]
    fn unsupported_files_are_returned_untouched(
        #[values(Backend::Sequential, Backend::Multithreaded)] backend: Backend,
        #[values(IMPLICIT_MOTION, NUMBERED_BLOCK)] source: &str,
    ) {
        let result = backend.run(source, &options("arcs"), 64);

        assert_eq!(result, (source.to_owned(), 0));
    }

    #[rstest]
    fn an_inherited_command_after_a_fitting_run_discards_the_whole_file(
        #[values(Backend::Sequential, Backend::Multithreaded)] backend: Backend,
    ) {
        // Earlier chunks weld into a curve before the implicit line is reached.
        // A generated curve rebinds the modal command this line inherits, so
        // the output must be discarded whole, not just from the line on.
        let source = format!("{PREAMBLE}{ARC}X40 Y0 E16.5\n");

        let result = backend.run(&source, &options("arcs"), 64);

        assert_eq!(result, (source, 0));
    }

    #[rstest]
    fn a_fitting_run_is_welded_and_counted(
        #[values(Backend::Sequential, Backend::Multithreaded)] backend: Backend,
    ) {
        let source = format!("{PREAMBLE}{ARC}");

        let result = backend.run(&source, &options("arcs"), 1 << 20);

        assert_eq!(result, (WELDED_ARC.to_owned(), 1));
    }

    #[rstest]
    fn pieces_are_emitted_in_order_and_counts_summed(
        #[values(Backend::Sequential, Backend::Multithreaded)] backend: Backend,
    ) {
        // The raw `M104` splits the file into two runs, so the pipeline must
        // keep both curves and report two replacements.
        let source = format!("{PREAMBLE}{ARC}M104 S200\n{ARC_CONTINUED}");
        let expected = indoc! {"
            G21
            G90
            M82
            G92 E0
            G1 X20 Y0 Z0.2 F1200 E0
            G3 I-20 J0 X0 Y20 F1200 E16
            M104 S200
            G3 I0 J-20 X-20 Y0 F1200 E32
        "};

        let result = backend.run(&source, &options("arcs"), 1 << 20);

        assert_eq!(result, (expected.to_owned(), 2));
    }

    #[rstest]
    fn an_existing_curve_is_preserved_and_splits_runs(
        #[values(Backend::Sequential, Backend::Multithreaded)] backend: Backend,
    ) {
        // The input curve is a trusted motion but is never rewritten, so the
        // chords after it weld independently of what came before.
        let source = format!("{PREAMBLE}G3 X0 Y20 I-20 J0 E16\n{ARC_CONTINUED}");
        let expected = indoc! {"
            G21
            G90
            M82
            G92 E0
            G1 X20 Y0 Z0.2 F1200 E0
            G3 X0 Y20 I-20 J0 E16
            G3 I0 J-20 X-20 Y0 F1200 E32
        "};

        let result = backend.run(&source, &options("arcs"), 1 << 20);

        assert_eq!(result, (expected.to_owned(), 1));
    }

    #[rstest]
    fn a_run_split_across_chunks_is_welded_whole(
        #[values(Backend::Sequential, Backend::Multithreaded)] backend: Backend,
    ) {
        // A tiny chunk cuts between the arc's lines. The trailing run lives in
        // the engine across the cut, so the result matches the single-chunk
        // pass instead of fragmenting the curve.
        let source = format!("{PREAMBLE}{ARC}");

        let result = backend.run(&source, &options("arcs"), 64);

        assert_eq!(result, (WELDED_ARC.to_owned(), 1));
    }

    #[rstest]
    fn a_supported_file_without_fitting_runs_is_verbatim(
        #[values(Backend::Sequential, Backend::Multithreaded)] backend: Backend,
    ) {
        // Straight collinear moves never fit an arc; the pipeline must still
        // reproduce the input byte for byte.
        let source = indoc! {"
            G21
            G90
            M82
            G92 E0
            G1 X0 Y0 Z0.2 F1200 E0
            G1 X1 Y0 E1
            G1 X2 Y0 E2
            G1 X3 Y0 E3
        "};

        let result = backend.run(source, &options("arcs"), 64);

        assert_eq!(result, (source.to_owned(), 0));
    }
}
