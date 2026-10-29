//! Splits raw text into line-aligned windows.
//!
//! Chunking is purely a text operation: it knows nothing about G-code, so the
//! streaming and parallel passes can hand whole lines to the assembler without
//! ever cutting one in half.

/// Splits `text` into chunks of roughly `chunk_bytes` bytes, cutting only after
/// a newline so every chunk is a whole number of lines. A chunk can exceed
/// `chunk_bytes` when a single line is longer.
pub(crate) struct Chunks<'a> {
    rest: &'a str,
    chunk_bytes: usize,
}

impl<'a> Chunks<'a> {
    pub(crate) fn new(text: &'a str, chunk_bytes: usize) -> Self {
        Self {
            rest: text,
            chunk_bytes,
        }
    }
}

impl<'a> Iterator for Chunks<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        if self.rest.is_empty() {
            return None;
        }

        let bytes = self.rest.as_bytes();
        let cut = if bytes.len() <= self.chunk_bytes {
            bytes.len()
        } else {
            bytes[..self.chunk_bytes]
                .iter()
                .rposition(|&b| b == b'\n')
                .map(|i| i + 1)
                .or_else(|| {
                    bytes[self.chunk_bytes..]
                        .iter()
                        .position(|&b| b == b'\n')
                        .map(|i| self.chunk_bytes + i + 1)
                })
                .unwrap_or(bytes.len())
        };

        let (chunk, rest) = self.rest.split_at(cut);
        self.rest = rest;
        Some(chunk)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_cover_the_text_at_line_boundaries() {
        let text = "G1 X0\nG1 X1\nG1 X2\nG1 X3\n";
        for chunk_bytes in [1, 5, 8, 12, 1000] {
            let parts: Vec<&str> = Chunks::new(text, chunk_bytes).collect();
            assert_eq!(parts.concat(), text, "chunk_bytes {chunk_bytes}");
            assert!(
                parts[..parts.len() - 1].iter().all(|c| c.ends_with('\n')),
                "a chunk ends mid-line for chunk_bytes {chunk_bytes}: {parts:?}"
            );
            assert!(parts.iter().all(|c| !c.is_empty()));
        }
    }

    #[test]
    fn a_line_longer_than_the_chunk_stays_whole() {
        let long = format!("G1 X{}\n", "1".repeat(100));
        let text = format!("G1 X0\n{long}G1 X2\n");
        let parts: Vec<&str> = Chunks::new(&text, 4).collect();
        assert_eq!(parts.concat(), text);
        assert!(parts.contains(&long.as_str()));
    }
}
