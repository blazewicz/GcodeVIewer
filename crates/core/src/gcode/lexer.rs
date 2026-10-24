//! Lexical layer: turns one raw line into command words.
//!
//! The lexer is deliberately strict. Any syntax it does not understand becomes
//! an explicit [`SyntaxError`] so the caller can treat it as a barrier rather
//! than silently discarding it.

use smallvec::SmallVec;

/// A single command word: a letter and its numeric value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Word {
    pub(crate) letter: u8,
    pub(crate) value: f64,
}

/// The words of a single line, in order. The first word is the command (e.g.
/// `G1`); the words after it are the command's parameters. The lexer
/// guarantees this command-first layout.
///
/// Most lines carry a handful of words, so they stay inline; the strict lexer
/// runs once per line on files with millions of lines, where a heap allocation
/// per line dominates both time and allocator contention.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Words(SmallVec<[Word; 8]>);

impl std::ops::Deref for Words {
    type Target = [Word];

    fn deref(&self) -> &[Word] {
        &self.0
    }
}

/// Why [`Words::lex`] rejected a line's code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SyntaxError {
    /// A character where a word letter was expected, as in numbered blocks,
    /// `%` delimiters, and parenthesized comments.
    UnexpectedCharacter,
    /// A word letter without a number the strict grammar accepts.
    InvalidNumber,
    /// A number that is not finite or exceeds the accepted magnitude.
    OutOfRange,
    /// A letter on more than one word of the same line.
    DuplicateLetter,
}

impl std::fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::UnexpectedCharacter => "unexpected character where a word letter is expected",
            Self::InvalidNumber => "word letter without a valid number",
            Self::OutOfRange => "number out of range",
            Self::DuplicateLetter => "letter appears twice on one line",
        })
    }
}

impl Words {
    /// Lexes one line's code into words.
    fn lex(code: &str) -> Result<Self, SyntaxError> {
        let bytes = code.as_bytes();
        let mut i = 0;
        let mut out: SmallVec<[Word; 8]> = SmallVec::new();

        while i < bytes.len() {
            if bytes[i].is_ascii_whitespace() {
                i += 1;
                continue;
            }

            let letter = bytes[i].to_ascii_uppercase();

            if !letter.is_ascii_alphabetic() {
                return Err(SyntaxError::UnexpectedCharacter);
            }

            i += 1;

            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }

            let begin = i;

            if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
                i += 1;
            }

            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }

            let value = code[begin..i]
                .parse::<f64>()
                .map_err(|_| SyntaxError::InvalidNumber)?;

            if !value.is_finite() || value.abs() > 1e9 {
                return Err(SyntaxError::OutOfRange);
            }

            if out.iter().any(|w| w.letter == letter) {
                return Err(SyntaxError::DuplicateLetter);
            }

            out.push(Word { letter, value });
        }

        Ok(Self(out))
    }

    /// The value of the parameter with `letter`, if the line carries it.
    pub(crate) fn param(&self, letter: u8) -> Option<f64> {
        self.iter().find(|w| w.letter == letter).map(|w| w.value)
    }

    /// True when every parameter word (everything after the command) is one of
    /// `allowed`.
    pub(crate) fn params_within(&self, allowed: &[u8]) -> bool {
        self.iter().skip(1).all(|w| allowed.contains(&w.letter))
    }
}

/// Returns the code portion of a line, i.e. everything before a `;` comment.
fn code_of(raw: &str) -> &str {
    raw.split_once(';').map_or(raw, |(code, _)| code).trim()
}

/// A single input line, kept together with everything decoding it needs:
/// the raw slice to re-emit verbatim, the lexed words (or the [`SyntaxError`]
/// that makes the line a barrier), and whether the line carries a comment.
pub(crate) struct Line<'a> {
    pub(crate) raw: &'a str,
    pub(crate) parsed: Result<Words, SyntaxError>,
    pub(crate) comment: bool,
    /// True when the code is blank or opens with a command letter. False for
    /// parameter-only and numbered lines, whose meaning an interpreter has to
    /// inherit from the modal state.
    pub(crate) has_command: bool,
}

impl<'a> Line<'a> {
    /// Decodes one raw, newline-inclusive line. `raw` is kept verbatim so it can
    /// be written back unchanged.
    ///
    /// Text-message commands (`M117`/`M118`) carry free text rather than words, so
    /// they decode to an empty word list instead of poisoning the strict lexer.
    ///
    /// Parentheses are recognized as comment markers for [`Line::comment`] but are
    /// not stripped by [`code_of`]. A line carrying a parenthesized comment is
    /// therefore unparsable (`Err`), which makes it a barrier for the caller.
    pub(crate) fn decode(raw: &'a str) -> Self {
        let code = code_of(raw);
        let comment = raw.contains(';') || raw.contains('(');
        let has_command = code
            .as_bytes()
            .first()
            .is_none_or(|c| matches!(c.to_ascii_uppercase(), b'G' | b'M' | b'T'));
        let is_text_message = ["M117", "M118"].iter().any(|prefix| {
            code.starts_with(prefix)
                && code
                    .as_bytes()
                    .get(prefix.len())
                    .is_none_or(|c| c.is_ascii_whitespace())
        });
        Self {
            raw,
            parsed: if is_text_message {
                Ok(Words::default())
            } else {
                Words::lex(code)
            },
            comment,
            has_command,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    fn ws(pairs: &[(u8, f64)]) -> Words {
        Words(
            pairs
                .iter()
                .map(|&(letter, value)| Word { letter, value })
                .collect(),
        )
    }

    mod lex {
        use super::*;

        #[rstest]
        #[case::empty_text("")]
        #[case::only_whitespace("   \t")]
        fn returns_empty(#[case] text: &str) {
            let result = Words::lex(text).expect("valid G-code should parse");
            assert_eq!(result, Words::default());
        }

        #[rstest]
        #[case::spaces("G1 X1 Y2", &[(b'G', 1.0), (b'X', 1.0), (b'Y', 2.0)])]
        #[case::no_spaces("G1X1Y2", &[(b'G', 1.0), (b'X', 1.0), (b'Y', 2.0)])]
        #[case::space_between_letter_and_number("G1 X 1", &[(b'G', 1.0), (b'X', 1.0)])]
        fn accepts_word_separators(#[case] text: &str, #[case] expected: &[(u8, f64)]) {
            let result = Words::lex(text).expect("valid G-code should parse");
            assert_eq!(result, ws(expected));
        }

        #[rstest]
        #[case::integer("X12", 12.0)]
        #[case::negative("X-12", -12.0)]
        #[case::explicit_positive("X+12", 12.0)]
        #[case::decimal("X1.5", 1.5)]
        #[case::leading_decimal_point("X.5", 0.5)]
        #[case::at_upper_bound("X1000000000", 1.0e9)]
        #[case::at_lower_bound("X-1000000000", -1.0e9)]
        fn parses_numbers(#[case] text: &str, #[case] expected: f64) {
            let result = Words::lex(text).expect("valid G-code should parse");
            assert_eq!(result, ws(&[(b'X', expected)]));
        }

        #[rstest]
        #[case::uppercase("G1 X-1.5 Y+2 Z.5", &[(b'G', 1.0), (b'X', -1.5), (b'Y', 2.0), (b'Z', 0.5)])]
        #[case::lowercase("g1 x-1.5 y+2 z.5", &[(b'G', 1.0), (b'X', -1.5), (b'Y', 2.0), (b'Z', 0.5)])]
        fn normalizes_case(#[case] text: &str, #[case] expected: &[(u8, f64)]) {
            let result = Words::lex(text).expect("valid G-code should parse");
            assert_eq!(result, ws(expected));
        }

        #[rstest]
        #[case::no_number("X", SyntaxError::InvalidNumber)]
        #[case::sign_only("X-", SyntaxError::InvalidNumber)]
        #[case::plus_only("X+", SyntaxError::InvalidNumber)]
        #[case::stray_dot("X.", SyntaxError::InvalidNumber)]
        #[case::multiple_dots("X1.2.3", SyntaxError::InvalidNumber)]
        #[case::space_inside_sign("X - 1", SyntaxError::InvalidNumber)]
        #[case::nan_letters("Xnan", SyntaxError::InvalidNumber)]
        #[case::inf_letters("Xinf", SyntaxError::InvalidNumber)]
        #[case::above_bound("X1000000001", SyntaxError::OutOfRange)]
        #[case::below_bound("X-1000000001", SyntaxError::OutOfRange)]
        #[case::overflowing_value(&format!("X{}", "9".repeat(400)), SyntaxError::OutOfRange)]
        #[case::numbered("1 G1 X0", SyntaxError::UnexpectedCharacter)]
        #[case::malformed("% comment", SyntaxError::UnexpectedCharacter)]
        #[case::duplicate_param("X1 Y2 X3", SyntaxError::DuplicateLetter)]
        #[case::duplicate_command("G1 G2", SyntaxError::DuplicateLetter)]
        fn rejects_invalid_text(#[case] text: &str, #[case] expected: SyntaxError) {
            let result = Words::lex(text);

            assert_eq!(result, Err(expected));
        }
    }

    mod code_of {
        use super::*;

        #[rstest]
        #[case::with_comment("G1 X0 ; move\n", "G1 X0")]
        #[case::without_comment("G1 X0\n", "G1 X0")]
        #[case::leading_whitespace("   G1 X0  \n", "G1 X0")]
        #[case::just_comment("; only comment", "")]
        #[case::parenthesis_not_stripped("G1 X0 (note)", "G1 X0 (note)")]
        fn trims_code(#[case] raw: &str, #[case] expected: &str) {
            let result = code_of(raw);

            assert_eq!(result, expected);
        }
    }

    mod has_command {
        use super::*;

        #[rstest]
        #[case::empty("")]
        #[case::whitespace("   \n")]
        #[case::just_comment("; comment\n")]
        #[case::with_param("G1 X0\n")]
        #[case::with_param_lowercase("m104 S200\n")]
        #[case::no_param("T0\n")]
        #[case::no_param_lowercase("g1\n")]
        #[case::tool_lowercase("t0\n")]
        fn true_for_blank_and_command_lines(#[case] raw: &str) {
            let result = Line::decode(raw);

            assert!(result.has_command);
        }

        #[rstest]
        #[case::implicit("X0 Y0\n")]
        #[case::numbered("1 G1 X0\n")]
        #[case::malformed("%\n")]
        #[case::parenthesis_comment("(comment)\n")]
        fn false_for_parameter_and_unparsable_lines(#[case] raw: &str) {
            let result = Line::decode(raw);

            assert!(!result.has_command);
        }
    }

    mod decode {
        use super::*;

        #[rstest]
        #[case::no_params("G21", Ok(ws(&[(b'G', 21.0)])), false)]
        #[case::with_comment("G1 X0 ; move", Ok(ws(&[(b'G', 1.0), (b'X', 0.0)])), true)]
        #[case::just_comment("; comment", Ok(Words::default()), true)]
        #[case::text_message_m117("M117 hello world", Ok(Words::default()), false)]
        #[case::text_message_m118("M118 hello world", Ok(Words::default()), false)]
        #[case::text_message_bare("M117", Ok(Words::default()), false)]
        #[case::text_message_with_comment("M117 hello ; note", Ok(Words::default()), true)]
        #[case::m117_prefixed_number("M1174", Ok(ws(&[(b'M', 1174.0)])), false)]
        #[case::m117_decimal("M117.0", Ok(ws(&[(b'M', 117.0)])), false)]
        #[case::parenthesis_comment("(comment)", Err(SyntaxError::UnexpectedCharacter), true)]
        #[case::parenthesis_in_code("G1 X0 (note)", Err(SyntaxError::UnexpectedCharacter), true)]
        #[case::unsupported("G1 X", Err(SyntaxError::InvalidNumber), false)]
        fn decodes(
            #[case] raw: &str,
            #[case] parsed: Result<Words, SyntaxError>,
            #[case] comment: bool,
        ) {
            let result = Line::decode(raw);

            assert_eq!(result.raw, raw);
            assert_eq!(result.parsed, parsed);
            assert_eq!(result.comment, comment);
        }
    }

    #[rstest]
    #[case::command_letter(&[(b'G', 1.0), (b'X', -1.5)], b'G', Some(1.0))]
    #[case::existing(&[(b'G', 1.0), (b'X', -1.5)], b'X', Some(-1.5))]
    #[case::nonexisting(&[(b'G', 1.0), (b'X', -1.5)], b'Z', None)]
    #[case::only_command(&[(b'G', 1.0)], b'G', Some(1.0))]
    #[case::absent_from_command_only(&[(b'G', 1.0)], b'X', None)]
    fn param_returns_value(
        #[case] pairs: &[(u8, f64)],
        #[case] letter: u8,
        #[case] expected: Option<f64>,
    ) {
        let words = ws(pairs);

        let result = words.param(letter);

        assert_eq!(result, expected);
    }

    #[rstest]
    #[case::supported("G1 X-1.5 Y+2 E0 F900", b"XYZEF", true)]
    #[case::unsupported("G0 U-1.5 V+2 W0", b"XYZEF", false)]
    #[case::command_only("G1", b"XYZEF", true)]
    #[case::command_only_empty_allowed("G1", b"", true)]
    #[case::parameter_not_allowed("G1 X0", b"", false)]
    fn params_within_checks(#[case] code: &str, #[case] params: &[u8], #[case] expected: bool) {
        let words = Words::lex(code).unwrap();

        let result = words.params_within(params);

        assert_eq!(result, expected);
    }
}
