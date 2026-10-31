//! Streams a G-code file into the renderer, chunk by chunk.
//!
//! Lines are read one at a time and accumulated into larger chunks, so the
//! loader sees the same kind of chunk boundaries it gets from the browser
//! host. Decoding is lossy, so a stray non-UTF-8 byte in a comment does not
//! fail the load, and a whole line is always handed over as one piece, never
//! split inside a character.

use std::io::{self, BufRead};

/// Text handed to the loader per call.
pub(crate) const CHUNK_BYTES: usize = 1 << 20;

/// Reads `reader` to the end, calling `feed` with successive pieces of text.
pub(crate) fn stream(reader: impl BufRead, feed: impl FnMut(&str)) -> io::Result<()> {
    stream_with_chunk(reader, CHUNK_BYTES, feed)
}

/// [`stream`] with a configurable chunk size, so tests can keep the chunks
/// tiny.
fn stream_with_chunk(
    mut reader: impl BufRead,
    chunk_bytes: usize,
    mut feed: impl FnMut(&str),
) -> io::Result<()> {
    let mut line = Vec::new();
    let mut chunk = String::new();

    loop {
        line.clear();

        if reader.read_until(b'\n', &mut line)? == 0 {
            break;
        }

        chunk.push_str(&String::from_utf8_lossy(&line));

        if chunk.len() >= chunk_bytes {
            feed(&chunk);
            chunk.clear();
        }
    }

    if !chunk.is_empty() {
        feed(&chunk);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use rstest::rstest;

    use super::*;

    /// The chunks `stream` produces for `data` with a given threshold.
    fn chunks(data: &[u8], chunk_bytes: usize) -> Vec<String> {
        let mut chunks = Vec::new();
        stream_with_chunk(Cursor::new(data), chunk_bytes, |chunk| {
            chunks.push(chunk.to_owned());
        })
        .expect("reading from a cursor cannot fail");
        chunks
    }

    /// A file with a multi-byte comment, a malformed byte, and no trailing
    /// newline.
    fn source() -> Vec<u8> {
        let mut data = "G1 X0 Y0 ; już kończymy\nG1 X1".as_bytes().to_vec();
        data.push(0xFF);
        data.extend_from_slice(b" Y1\nG1 X2 Y2");
        data
    }

    #[rstest]
    #[case::single_bytes(1)]
    #[case::two_bytes(2)]
    #[case::seven_bytes(7)]
    #[case::one_line(64)]
    #[case::whole_file(usize::MAX)]
    fn chunking_does_not_change_the_text(#[case] chunk_bytes: usize) {
        let source = source();
        let expected = String::from_utf8_lossy(&source);

        let joined: String = chunks(&source, chunk_bytes).concat();

        assert_eq!(joined, expected);
    }

    #[rstest]
    #[case::single_bytes(1)]
    #[case::short_lines(10)]
    #[case::whole_file(usize::MAX)]
    fn every_chunk_but_the_last_ends_at_a_line_boundary(#[case] chunk_bytes: usize) {
        let produced = chunks(b"G1 X0\nG1 X1\nG1 X2\n", chunk_bytes);

        for chunk in &produced[..produced.len() - 1] {
            assert!(chunk.ends_with('\n'), "{chunk:?}");
        }
    }

    #[rstest]
    #[case::empty(b"")]
    #[case::no_newline(b"G1 X0")]
    fn short_inputs_produce_at_most_one_chunk(#[case] data: &[u8]) {
        let produced = chunks(data, 16);

        assert!(produced.len() <= 1, "{produced:?}");
    }
}
