//! Streaming state for one G-code load.
//!
//! [`Loader`] owns everything that must survive between chunks of input: the
//! incremental modal scanner, reusable scratch space for the records and
//! segments of one chunk, and the running statistics. It deliberately depends
//! on no wgpu types, so the whole accumulation pipeline can be tested on the
//! host; the renderer feeds it text and uploads the segments it reports.

use glam::Vec3;

use gcode_viewer_gcode::{MotionRecord, Scanner};

use crate::tessellate::{self, MAX_SEGMENTS, Segment};

/// The running totals of one load, taken once the load finished.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LoadStats {
    /// Number of motion records parsed from the file.
    pub(crate) moves: usize,
    /// Number of drawable segments after flattening.
    pub(crate) segments: usize,
    /// XYZ extent of every record with a known endpoint.
    pub(crate) bounds: Option<(Vec3, Vec3)>,
    /// Lowest and highest known feedrate across the segments, in mm/min.
    pub(crate) feed_range: Option<(f32, f32)>,
}

/// Accumulates one load fed in chunks.
///
/// The loader is the chunk boundary of the whole pipeline: each call parses
/// the records of one chunk, flattens them, folds them into the statistics,
/// and hands the drawable segments back. Neither the input text nor the
/// flattened geometry of the whole file has to be retained.
pub(crate) struct Loader {
    scanner: Scanner,
    /// Records of the chunk being processed, reused across feeds.
    records: Vec<MotionRecord>,
    /// Segments of the chunk being processed, reused across feeds.
    segments: Vec<Segment>,
    stats: LoadStats,
    /// Whether the global segment cap stopped the flattening.
    capped: bool,
}

impl Loader {
    pub(crate) fn new() -> Self {
        Self {
            scanner: Scanner::new(),
            records: Vec::new(),
            segments: Vec::new(),
            stats: LoadStats {
                moves: 0,
                segments: 0,
                bounds: None,
                feed_range: None,
            },
            capped: false,
        }
    }

    /// Parses and flattens the next chunk. The returned segments are valid
    /// until the next call.
    pub(crate) fn feed(&mut self, text: &str) -> &[Segment] {
        self.records.clear();
        self.scanner.feed(text, &mut self.records);
        self.flatten()
    }

    /// Interprets the trailing line of an input that did not end with a
    /// newline. Call once, after the last [`Loader::feed`].
    pub(crate) fn finish(&mut self) -> &[Segment] {
        self.records.clear();
        self.scanner.finish(&mut self.records);
        self.flatten()
    }

    pub(crate) fn stats(&self) -> LoadStats {
        self.stats
    }

    /// Flattens the records of the current chunk and folds everything into
    /// the running statistics.
    fn flatten(&mut self) -> &[Segment] {
        self.stats.moves += self.records.len();
        self.extend_bounds();

        self.segments.clear();
        if !self.capped {
            let remaining = MAX_SEGMENTS.saturating_sub(self.stats.segments);

            if !tessellate::extend(&self.records, &mut self.segments, remaining) {
                self.capped = true;
                log::warn!("segment cap reached; remaining moves are not rendered");
            }
        }

        self.stats.segments += self.segments.len();

        if let Some((min, max)) = tessellate::feed_range(&self.segments) {
            self.stats.feed_range = Some(match self.stats.feed_range {
                Some((low, high)) => (low.min(min), high.max(max)),
                None => (min, max),
            });
        }

        &self.segments
    }

    /// Widens the bounds to cover every endpoint of the current records.
    fn extend_bounds(&mut self) {
        for point in self
            .records
            .iter()
            .flat_map(|record| [record.start, record.end])
            .flatten()
        {
            let point = point.as_vec3();

            self.stats.bounds = Some(match self.stats.bounds {
                Some((min, max)) => (min.min(point), max.max(point)),
                None => (point, point),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use rstest::{fixture, rstest};

    /// A file with every move kind, comments, and a non-ASCII comment.
    #[fixture]
    fn source() -> &'static str {
        indoc! {"
            G21
            G90
            M82
            G92 X0 Y0 Z0 E0
            G1 F1200
            G0 X0 Y0 Z5 ; liftoff
            G1 X10 Y0 Z0.2 E1
            G2 X12 Y2 Z0.2 I2 J0 E1.5
            G5 X14 Y4 Z0.2 I2 J0 P2 Q2 E2
            G1 X20 Y4 E3 ; już kończymy
        "}
    }

    /// Splits `text` into chunks of up to `chunk_bytes` bytes at character
    /// boundaries, the way a UTF-8 decoder hands a stream to the loader.
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

    /// Runs a whole load and returns its segments and statistics.
    fn load_chunked(source: &str, chunk_bytes: usize) -> (Vec<Segment>, LoadStats) {
        let mut loader = Loader::new();
        let mut segments = Vec::new();

        for chunk in split(source, chunk_bytes) {
            segments.extend_from_slice(loader.feed(chunk));
        }
        segments.extend_from_slice(loader.finish());

        (segments, loader.stats())
    }

    #[rstest]
    fn chunking_does_not_change_the_result(
        #[values(1, 2, 3, 7, 13, 64, 4096, usize::MAX)] chunk_bytes: usize,
        source: &str,
    ) {
        assert_eq!(
            load_chunked(source, chunk_bytes),
            load_chunked(source, usize::MAX)
        );
    }

    #[test]
    fn the_demo_asset_survives_chunked_feeding() {
        let source = include_str!("../../../ui/public/demo.gcode");

        assert_eq!(load_chunked(source, 64), load_chunked(source, usize::MAX));
    }

    #[test]
    fn the_stats_cover_moves_bounds_and_feeds() {
        // given
        let source = indoc! {"
            G21
            G90
            M82
            G92 X0 Y0 Z0 E0
            G1 F1200
            G1 X10 Y0 Z0.2 E1
            G1 X10 Y10 Z0.4 E2 F900
        "};
        let mut loader = Loader::new();

        // when
        let segments = loader.feed(source).to_vec();
        loader.finish();

        // then
        let stats = loader.stats();
        assert_eq!(stats.moves, 2);
        assert_eq!(stats.segments, 2);
        assert_eq!(segments.len(), 2);
        assert_eq!(
            stats.bounds,
            Some((Vec3::new(0.0, 0.0, 0.0), Vec3::new(10.0, 10.0, 0.4)))
        );
        assert_eq!(stats.feed_range, Some((900.0, 1200.0)));
    }

    #[test]
    fn a_file_without_any_known_position_yields_no_segments() {
        // given a file that never establishes where the machine is
        let source = "G21\nG90\nG1 X5 Y5 Z0.2 E1 F1200\n";
        let mut loader = Loader::new();

        // when
        let segments = loader.feed(source).to_vec();
        loader.finish();

        // then the motion is reported, but it cannot be drawn
        assert_eq!(loader.stats().moves, 1);
        assert_eq!(loader.stats().segments, 0);
        assert!(segments.is_empty());
    }
}
