//! Semantic classification of a line's leading command.
//!
//! The lexer only tokenizes; deciding that `G90` is a positioning command, that
//! it must stand alone to take effect, and that `G92` only accepts the axes we
//! track are interpreter rules. They live here so [`State`](crate::state::State)
//! can match on intent instead of re-deriving it from raw command numbers.

use super::lexer::{Word, Words};

/// The optimizer-meaningful identity of a line's leading command.
///
/// Related G/M codes are grouped when the optimizer handles them identically.
/// Where behavior differs (relative vs. absolute, plane selection, ...) the
/// distinction is carried as data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Code {
    /// `G0`/`G1` linear move; `rapid` distinguishes travel (`G0`) from feed.
    LinearMove {
        rapid: bool,
    },
    // `G2`/`G3` arc or circle move; `G2` is clockwise and `G3` is counterclockwise.
    ArcMove {
        clockwise: bool,
    },
    // `G5` Bézier Cubic Spline Move
    SplineMove,
    /// `G90`/`G91`: `true` when positioning is relative.
    Positioning {
        relative: bool,
    },
    /// `M82`/`M83`: `true` when extrusion is relative.
    Extrusion {
        relative: bool,
    },
    /// `G20`/`G21`: `true` when units are millimeters.
    Units {
        mm: bool,
    },
    /// `G17`/`G18`/`G19`: `true` when the XY plane is selected.
    Plane {
        xy: bool,
    },
    /// `G92` set position.
    SetPosition,
    /// Recognized commands that leave optimizer state untouched.
    NoOp,
    /// Commands that invalidate the tracked reference frame.
    Reset,
    /// Unrecognized or unusable commands and numbered blocks.
    Other,
    /// Implicit motion without G, M or T command.
    Implicit,
}

impl Code {
    /// Classifies the leading command of a line. `None` means the line
    /// carries no words at all (blank lines and free-text messages).
    pub(crate) fn classify(words: &Words) -> Option<Self> {
        let Word { letter, value } = *words.first()?;
        let simple = words.len() == 1;

        Some(match (letter, value) {
            // G0/G1 Linear Move
            (b'G', 0.0) => Code::LinearMove { rapid: true },
            (b'G', 1.0) => Code::LinearMove { rapid: false },

            // G2/G3 Arc or Circle Move
            (b'G', 2.0) => Code::ArcMove { clockwise: true },
            (b'G', 3.0) => Code::ArcMove { clockwise: false },

            // G5 Bézier Cubic Spline Move
            (b'G', 5.0) => Code::SplineMove,

            // G90 Absolute Positioning / G91 Relative Positioning
            (b'G', 90.0) if simple => Code::Positioning { relative: false },
            (b'G', 91.0) if simple => Code::Positioning { relative: true },

            // M82 E Absolute / M83 E Relative
            (b'M', 82.0) if simple => Code::Extrusion { relative: false },
            (b'M', 83.0) if simple => Code::Extrusion { relative: true },

            // G20 Inch Units / G21 Millimeter Units
            (b'G', 20.0) if simple => Code::Units { mm: false },
            (b'G', 21.0) if simple => Code::Units { mm: true },

            // G17-G19 CNC Workspace Planes
            (b'G', 17.0) if simple => Code::Plane { xy: true },
            (b'G', 18.0 | 19.0) if simple => Code::Plane { xy: false },

            // G92 Set Position
            (b'G', 92.0) if !simple && words.params_within(b"XYZE") => Code::SetPosition,

            // G4 Dwell
            // M73 Set Print Progress
            // M75 Set Print Job Timer
            // M77 Set Print Job Timer
            // M104 Set Hotend Temperature
            // M105 Report Temperature
            // M106 Set Fan Speed
            // M107 Fan Off
            // M109 Wait for Hotend Temperature
            // M140 Set Bed Temperature
            // M155 Temperature Auth-Report
            // M190 Wait for Bed Temperature
            // M201 Print / Travel Move Limits
            // M203 Set Max Feedrate
            // M204 Set Starting Acceleration
            // M205 Set Advanced Settings
            // M220 Set Feedrate Percentage
            // M221 Set Flow Percentage
            // M400 Finish Moves
            // M900 Linear Advance Factor
            (
                b'M',
                104.0 | 105.0 | 109.0 | 140.0 | 190.0 | 106.0 | 107.0 | 155.0 | 201.0 | 203.0
                | 204.0 | 205.0 | 220.0 | 221.0 | 400.0 | 73.0 | 75.0 | 77.0 | 900.0,
            )
            | (b'G', 4.0) => Code::NoOp,

            // G28 Auto Home
            // G29 Bed Leveling
            // M18, M84 Disable Steppers
            // M420 Bed Leveling State
            // T0-T7 Select or Report Tool
            (b'G', 28.0 | 29.0) | (b'T', _) | (b'M', 18.0 | 84.0 | 420.0) => Code::Reset,

            // Unknown G and M codes and numbered blocks fall into Other.
            (b'G' | b'M' | b'N', _) => Code::Other,

            // A line whose leading word is a parameter is modal motion, e.g.
            // `X5 Y5` after `G1`.
            _ => Code::Implicit,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::lexer::{self, Words};
    use super::Code;
    use rstest::rstest;

    #[rstest]
    #[case("G0 X0", Code::LinearMove { rapid: true })]
    #[case("G1 X0", Code::LinearMove { rapid: false })]
    #[case("G90", Code::Positioning { relative: false })]
    #[case("G91", Code::Positioning { relative: true })]
    #[case("M82", Code::Extrusion { relative: false })]
    #[case("M83", Code::Extrusion { relative: true })]
    #[case("G20", Code::Units { mm: false })]
    #[case("G21", Code::Units { mm: true })]
    #[case("G17", Code::Plane { xy: true })]
    #[case("G18", Code::Plane { xy: false })]
    #[case("G19", Code::Plane { xy: false })]
    #[case("G92 X10 E5", Code::SetPosition)]
    #[case("G4 P100", Code::NoOp)]
    #[case("M104 S200", Code::NoOp)]
    #[case("G2 X0 Y0 I1 J1", Code::ArcMove{ clockwise: true })]
    #[case("G3 X0 Y0 I1 J1", Code::ArcMove{ clockwise: false })]
    #[case("G5 X0 Y0 I1 J1 P1 Q1", Code::SplineMove)]
    #[case("T0", Code::Reset)]
    #[case("G93", Code::Other)]
    #[case("G90 X0", Code::Other)]
    #[case("M82 X0", Code::Other)]
    #[case("G17 X0", Code::Other)]
    #[case("N5 G0 X10", Code::Other)]
    #[case("X0", Code::Implicit)]
    fn classifies_commands(#[case] input: &str, #[case] expected: Code) {
        let words = lexer::Line::decode(input).parsed.expect("Input must parse");

        let result = Code::classify(&words);

        assert_eq!(result, Some(expected));
    }

    #[test]
    fn is_none_without_a_command() {
        let words = Words::default();

        let result = Code::classify(&words);

        assert_eq!(result, None);
    }
}
