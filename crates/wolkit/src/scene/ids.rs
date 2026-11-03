// SPDX-License-Identifier: BSD-2-Clause
//! Node ids: a `u32` with a 24-bit slot and an 8-bit generation, chosen by the caller so creating
//! a node needs no round trip. Slot 0 is the window's root, always there, generation 0.

/// Most nodes in one window, the root included (plan section 2.8).
pub const MAX_NODES: usize = 65_536;
/// Deepest tree, the root being depth 0.
pub const MAX_DEPTH: usize = 64;

/// Slot index into the scene's arrays.
pub type Idx = u32;
/// No node (a link that leads nowhere).
pub const NIL: Idx = u32::MAX;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct NodeId(pub u32);

impl NodeId {
    pub const ROOT: NodeId = NodeId(0);

    pub const fn new(slot: u32, generation: u8) -> NodeId {
        NodeId((slot & 0x00ff_ffff) | ((generation as u32) << 24))
    }

    pub const fn slot(self) -> Idx {
        self.0 & 0x00ff_ffff
    }

    pub const fn generation(self) -> u8 {
        (self.0 >> 24) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_slot_and_generation() {
        let id = NodeId::new(65_535, 7);
        assert_eq!((id.slot(), id.generation()), (65_535, 7));
        assert_eq!(NodeId::ROOT.slot(), 0);
        assert_eq!(NodeId::new(0x0100_0005, 1).slot(), 5);
    }
}
