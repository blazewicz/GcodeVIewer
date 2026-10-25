//! The Rust-side shader contract, mirrors shader/contract.wesl

use bytemuck::{Pod, Zeroable};

use crate::tessellate;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct Uniforms {
    pub(crate) view_proj: [f32; 16],
    pub(crate) viewport: [f32; 2],
    pub(crate) line_width: f32,
    /// Moves whose highest Z exceeds this are hidden whole.
    pub(crate) z_max: f32,
    pub(crate) kind_mask: u32,
    /// 0 colours segments by move kind, 1 by feedrate.
    pub(crate) color_mode: u32,
    /// Feedrates mapped onto the ends of the colour scale, in mm/min.
    pub(crate) feed_min: f32,
    pub(crate) feed_max: f32,
    /// Direction from the scene toward the eye; shades tube cross-sections.
    pub(crate) light_dir: [f32; 3],
    /// Radius of an extruding move in millimetres.
    pub(crate) extrusion_radius: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct PathInstance {
    start: [f32; 3],
    end: [f32; 3],
    kind: u32,
    /// Bit 0: cap the start; bit 1: cap the end. Only tubes read these.
    flags: u32,
    /// Active feedrate in mm/min, or `tessellate::NO_FEED` when unknown.
    feed: f32,
    /// Highest Z of the whole move; the shader hides moves above the Z limit
    /// as a unit.
    move_z_max: f32,
}

impl PathInstance {
    const ATTRIBUTES: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
        0 => Float32x3,
        1 => Float32x3,
        2 => Uint32,
        3 => Uint32,
        4 => Float32,
        5 => Float32,
    ];

    pub(crate) fn from_segment(segment: tessellate::Segment) -> Self {
        Self {
            start: segment.a,
            end: segment.b,
            kind: segment.kind as u32,
            flags: u32::from(segment.start_cap) | (u32::from(segment.end_cap) << 1),
            feed: segment.feed.unwrap_or(tessellate::NO_FEED),
            move_z_max: segment.move_z_max,
        }
    }

    pub(crate) fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

/// One domed cap instance: the tube end at a move endpoint. The renderer
/// cannot precompute the sphere center because it depends on the runtime
/// radius, so the shader derives it from `tip`, `axis`, and `room`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct PrintPathCapInstance {
    /// The move endpoint the dome must reach.
    tip: [f32; 3],
    /// Unit direction from the tube into the dome.
    axis: [f32; 3],
    /// Longest axial extent the dome may occupy, in millimetres.
    room: f32,
    kind: u32,
    /// Active feedrate in mm/min, or `tessellate::NO_FEED` when unknown.
    feed: f32,
    /// Highest Z of the whole move; the shader hides moves above the Z limit
    /// as a unit.
    move_z_max: f32,
}

impl PrintPathCapInstance {
    const ATTRIBUTES: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
        0 => Float32x3,
        1 => Float32x3,
        2 => Float32,
        3 => Uint32,
        4 => Float32,
        5 => Float32,
    ];

    /// The cap covering one end of a tube segment, or `None` when that end is
    /// an interior joint of its move.
    pub(crate) fn from_segment_end(segment: &tessellate::Segment, at_end: bool) -> Option<Self> {
        let capped = if at_end {
            segment.end_cap
        } else {
            segment.start_cap
        };
        if !capped {
            return None;
        }

        let (tip, inner) = if at_end {
            (segment.b, segment.a)
        } else {
            (segment.a, segment.b)
        };
        let mut axis = [tip[0] - inner[0], tip[1] - inner[1], tip[2] - inner[2]];
        let length = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2])
            .sqrt()
            .max(1e-6);
        for component in &mut axis {
            *component /= length;
        }

        // A dome may use at most half the segment when both ends are capped,
        // matching the inset the lateral shader applies.
        let room = if segment.start_cap && segment.end_cap {
            length * 0.5
        } else {
            length
        };

        Some(Self {
            tip,
            axis,
            room,
            kind: segment.kind as u32,
            feed: segment.feed.unwrap_or(tessellate::NO_FEED),
            move_z_max: segment.move_z_max,
        })
    }

    pub(crate) fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBUTES,
        }
    }
}
