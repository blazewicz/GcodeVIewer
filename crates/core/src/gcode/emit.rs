//! Renders a fitted window of moves back to a single G-code line.
//!
//! Fitting returns a [`Curve`] describing the replacement without producing
//! text. A window is probed many times during the search for the longest
//! fitting prefix, so the descriptor stays cheap and rendering happens once per
//! probe into a caller-owned buffer.

use std::fmt::{self, Write};

use crate::toolpath::Point;

use super::motion::Move;

/// Precision of fitted control offsets in millimetres. Geometry modules snap
/// their control points to this step before validation, so the offsets they
/// check are exactly the ones printed here.
pub(crate) const CONTROL_STEP: f64 = 1e-3;

/// Decimals printed for extrusion words. Absolute E is echoed from the input
/// and relative E is a computed sum, so a fixed precision also keeps binary
/// noise out of the output.
const E_DECIMALS: usize = 5;

/// A fitted window ready to render.
#[derive(Clone, Copy)]
pub(crate) struct Curve {
    head: Head,
    tail: Tail,
}

#[derive(Clone, Copy)]
enum Head {
    Arc { clockwise: bool, i: f64, j: f64 },
    Spline { i: f64, j: f64, p: f64, q: f64 },
}

/// The endpoint, feed, extrusion, and line ending shared by every curve.
#[derive(Clone, Copy)]
struct Tail {
    end: Point,
    feed: f64,
    e: Option<f64>,
    ending: &'static str,
}

pub(crate) trait Emit {
    fn write_to(&self, out: &mut String);
}

impl Curve {
    pub(crate) fn arc(clockwise: bool, i: f64, j: f64, m: &[Move<'_>]) -> Self {
        Self {
            head: Head::Arc { clockwise, i, j },
            tail: Tail::of(m),
        }
    }

    pub(crate) fn spline(i: f64, j: f64, p: f64, q: f64, m: &[Move<'_>]) -> Self {
        Self {
            head: Head::Spline { i, j, p, q },
            tail: Tail::of(m),
        }
    }
}

impl Emit for Curve {
    /// Renders the line, reusing `out`'s buffer and replacing its contents.
    fn write_to(&self, out: &mut String) {
        out.clear();
        // Writing to a `String` cannot fail.
        let _ = self.write(out);
    }
}

impl Curve {
    fn write(&self, w: &mut impl Write) -> fmt::Result {
        match self.head {
            Head::Arc { clockwise, i, j } => {
                w.write_str(if clockwise { "G2 I" } else { "G3 I" })?;
                write_control(w, i)?;
                w.write_str(" J")?;
                write_control(w, j)?;
            }

            Head::Spline { i, j, p, q } => {
                w.write_str("G5 I")?;
                write_control(w, i)?;
                w.write_str(" J")?;
                write_control(w, j)?;
                w.write_str(" P")?;
                write_control(w, p)?;
                w.write_str(" Q")?;
                write_control(w, q)?;
            }
        }

        w.write_str(" X")?;
        write_num(w, self.tail.end.0)?;
        w.write_str(" Y")?;
        write_num(w, self.tail.end.1)?;
        w.write_str(" F")?;
        write_num(w, self.tail.feed)?;

        if let Some(e) = self.tail.e {
            w.write_str(" E")?;
            write_fixed(w, e, E_DECIMALS)?;
        }

        w.write_str(self.tail.ending)
    }
}

impl Tail {
    fn of(m: &[Move<'_>]) -> Self {
        let first = &m[0];
        let last = m.last().expect("a fitted window is never empty");
        Self {
            end: last.end,
            feed: first.feed,
            e: (last.e1 > first.e0).then_some(if first.relative_e {
                last.e1 - first.e0
            } else {
                last.e1
            }),
            ending: if last.raw.ends_with("\r\n") {
                "\r\n"
            } else if last.raw.ends_with('\n') {
                "\n"
            } else {
                ""
            },
        }
    }
}

/// Writes a number the way G-code expects: `0` rather than `0.0` or `-0`.
fn write_num(w: &mut impl Write, v: f64) -> fmt::Result {
    if v == 0.0 {
        w.write_str("0")
    } else {
        write!(w, "{v}")
    }
}

/// Writes a fitted control offset at [`CONTROL_STEP`] precision, trimming
/// trailing zeros and the bare `-0` that rounding can produce.
fn write_control(w: &mut impl Write, v: f64) -> fmt::Result {
    write_fixed(w, v, 3)
}

fn write_fixed(w: &mut impl Write, v: f64, decimals: usize) -> fmt::Result {
    if v == 0.0 {
        return w.write_str("0");
    }

    let formatted = format!("{v:.decimals$}");
    let trimmed = formatted.trim_end_matches('0').trim_end_matches('.');
    w.write_str(if trimmed == "-0" { "0" } else { trimmed })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trips(v: f64, decimals: usize) -> f64 {
        let mut out = String::new();
        write_fixed(&mut out, v, decimals).unwrap();
        out.parse().unwrap()
    }

    #[test]
    fn control_offsets_print_at_millimetre_thousandths() {
        let mut out = String::new();
        write_control(&mut out, 8.200725243).unwrap();
        write_control(&mut out, -19.999999999).unwrap();
        write_control(&mut out, 0.0004999).unwrap();

        assert_eq!(out, "8.201-200");
    }

    #[test]
    fn zero_and_negative_zero_print_unsigned() {
        let mut out = String::new();
        write_control(&mut out, 0.0).unwrap();
        write_control(&mut out, -0.0).unwrap();
        write_control(&mut out, -0.0004).unwrap();

        assert_eq!(out, "000");
    }

    #[test]
    fn extrusion_keeps_five_decimals_and_drops_binary_noise() {
        let mut out = String::new();
        write_fixed(&mut out, 3.4000000000000004, E_DECIMALS).unwrap();
        write_fixed(&mut out, -0.25, E_DECIMALS).unwrap();
        write_fixed(&mut out, 16.0, E_DECIMALS).unwrap();

        assert_eq!(out, "3.4-0.2516");
    }

    #[test]
    fn printed_control_offsets_round_trip_to_their_snapped_values() {
        for value in [8.2, -20.0, 0.0, 0.5, -0.005, 1e5, 1e-3] {
            let snapped = Point(value, 0.0).snapped(CONTROL_STEP).0;
            assert_eq!(round_trips(snapped, 3), snapped, "value {value}");
        }
    }
}
