/// The interpolation a move describes between its endpoints.
///
/// Linear kinds (`Rapid`, `Feed`) are what the optimizer can weld; `Arc` and
/// `Spline` carry the control words that shape the path between the endpoints.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MoveKind {
    Rapid,
    Feed,
    Arc { clockwise: bool },
    Spline,
}

/// The control offsets that shape a curved move, in millimetres regardless of
/// the file's unit mode.
///
/// Arc offsets are relative to the start; `r` is the radius form, mutually
/// exclusive with the center offsets. Spline offsets are relative to the start
/// (`i`/`j`) and to the end (`p`/`q`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CurveKind {
    Arc { i: f64, j: f64, r: Option<f64> },
    Spline { i: f64, j: f64, p: f64, q: f64 },
}

impl MoveKind {
    pub(crate) fn is_linear(self) -> bool {
        matches!(self, Self::Rapid | Self::Feed)
    }
}
