mod chunks;
mod gcode;
mod linearize;
mod query;
mod toolpath;

pub use gcode::{CurveKind, MoveKind};
pub use linearize::linearize_curve;
pub use query::{MotionRecord, Scanner, motions};

#[derive(Clone)]
pub struct Options {
    pub tolerance: f64,
    pub mode: String,
    pub extrusion_tolerance: f64,
    pub travel: bool,
    pub max_segments: usize,
}
