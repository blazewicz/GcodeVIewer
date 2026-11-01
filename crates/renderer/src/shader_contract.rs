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
    /// Direction from the scene toward the eye; `w` is unused padding that
    /// keeps the uniform block 16-byte aligned.
    pub(crate) light_dir: [f32; 4],
    /// `x`: reference tube radius in millimetres, `y`: reference flow rate in
    /// mm of filament per mm of path, `z` and `w`: the multiples of the
    /// reference radius a tube may span.
    pub(crate) extrusion: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct PathInstance {
    start: [f32; 3],
    end: [f32; 3],
    kind: u32,
    /// Bit 0: the segment starts its move; bit 1: it ends one. Print paths
    /// keep move-final cones inside their segment and let interior cones reach
    /// into the following segment's socket.
    flags: u32,
    /// Active feedrate in mm/min, or `tessellate::NO_FEED` when unknown.
    feed: f32,
    /// Highest Z of the whole move; the shader hides moves above the Z limit
    /// as a unit.
    move_z_max: f32,
    /// Extrusion per millimetre of path, or `tessellate::NO_RATE` when the
    /// move's delta is unknown.
    rate: f32,
}

impl PathInstance {
    const ATTRIBUTES: [wgpu::VertexAttribute; 7] = wgpu::vertex_attr_array![
        0 => Float32x3,
        1 => Float32x3,
        2 => Uint32,
        3 => Uint32,
        4 => Float32,
        5 => Float32,
        6 => Float32,
    ];

    pub(crate) fn from_segment(segment: tessellate::Segment) -> Self {
        Self {
            start: segment.a.into(),
            end: segment.b.into(),
            kind: segment.kind as u32,
            flags: u32::from(segment.start_cap) | (u32::from(segment.end_cap) << 1),
            feed: segment.feed.unwrap_or(tessellate::NO_FEED),
            move_z_max: segment.move_z_max,
            rate: segment.rate.unwrap_or(tessellate::NO_RATE),
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

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use std::mem::{offset_of, size_of};

    #[test]
    fn the_instance_layout_matches_the_shader_contract() {
        assert_eq!(size_of::<PathInstance>(), 44);
        assert_eq!(offset_of!(PathInstance, start), 0);
        assert_eq!(offset_of!(PathInstance, end), 12);
        assert_eq!(offset_of!(PathInstance, kind), 24);
        assert_eq!(offset_of!(PathInstance, flags), 28);
        assert_eq!(offset_of!(PathInstance, feed), 32);
        assert_eq!(offset_of!(PathInstance, move_z_max), 36);
        assert_eq!(offset_of!(PathInstance, rate), 40);
    }

    #[test]
    fn the_uniform_layout_matches_the_shader_contract() {
        assert_eq!(size_of::<Uniforms>(), 128);
        assert_eq!(offset_of!(Uniforms, light_dir), 96);
        assert_eq!(offset_of!(Uniforms, extrusion), 112);
    }

    #[test]
    fn an_unknown_flow_rate_carries_the_sentinel() {
        let segment = tessellate::Segment {
            a: Vec3::ZERO,
            b: Vec3::X,
            kind: tessellate::Kind::Feed,
            feed: None,
            extruding: true,
            rate: None,
            start_cap: true,
            end_cap: true,
            move_z_max: 0.0,
        };

        assert_eq!(
            PathInstance::from_segment(segment).rate,
            tessellate::NO_RATE
        );
    }
}
