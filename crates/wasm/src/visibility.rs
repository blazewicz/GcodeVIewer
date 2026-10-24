//! Per-kind visibility bits for the renderer.
//!
//! The mask reaches the shader as one `u32`, where bit `Kind as u32` decides
//! whether segments of that kind are drawn. The bit arithmetic lives here so
//! the host tests can cover it: the renderer itself only builds for wasm32.

use crate::tessellate::Kind;

/// Bit `kind` set for every palette kind.
const ALL: u32 = (1 << Kind::COUNT) - 1;

/// Visibility bits for all palette kinds, indexed by `Kind as u32`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct KindMask(u32);

impl KindMask {
    /// Every kind except rapids, so the first view of a file shows the cutting
    /// moves instead of the travels between them.
    pub(crate) const DEFAULT: Self = Self(ALL & !(1 << Kind::Rapid as u32));

    /// Shows or hides `kind`; out-of-range indices are ignored.
    pub(crate) fn set(&mut self, kind: u32, visible: bool) {
        if kind >= Kind::COUNT {
            return;
        }

        let bit = 1 << kind;
        if visible {
            self.0 |= bit;
        } else {
            self.0 &= !bit;
        }
    }

    /// The mask as the shader uniform value.
    pub(crate) fn bits(self) -> u32 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_shows_every_kind_but_rapids() {
        assert_eq!(KindMask::DEFAULT.bits(), 0b1110);
    }

    #[test]
    fn set_toggles_one_bit() {
        let mut mask = KindMask::DEFAULT;

        mask.set(Kind::Rapid as u32, true);
        assert_eq!(mask.bits(), 0b1111);

        mask.set(Kind::Arc as u32, false);
        assert_eq!(mask.bits(), 0b1011);
    }

    #[test]
    fn out_of_range_kinds_are_ignored() {
        let mut mask = KindMask::DEFAULT;

        mask.set(Kind::COUNT, true);
        mask.set(u32::MAX, true);

        assert_eq!(mask.bits(), 0b1110);
    }
}
