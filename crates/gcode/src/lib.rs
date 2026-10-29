mod gcode;
mod query;
mod toolpath;
// mod transform;

pub use gcode::{CurveKind, MoveKind};
pub use query::{MotionRecord, motions, moves_in_box};
pub use toolpath::linearize_curve;

#[derive(Clone)]
pub struct Options {
    pub tolerance: f64,
    pub mode: String,
    pub extrusion_tolerance: f64,
    pub travel: bool,
    pub max_segments: usize,
}
