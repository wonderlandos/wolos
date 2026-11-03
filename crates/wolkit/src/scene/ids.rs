// SPDX-License-Identifier: BSD-2-Clause
//! Node ids: a `u32` with a 24-bit slot and an 8-bit generation, chosen by the caller so creating
//! a node needs no round trip. Slot 0 is the window's root, always there, generation 0.

/// Most nodes in one window, the root included (plan section 2.8).
pub const MAX_NODES: usize = 65_536;
/// Deepest tree, the root being depth 0.
pub const MAX_DEPTH: usize = 64;

/// Index conversions that cannot truncate on the 64-bit targets WolOS runs on, without the `as`
/// casts `RR-017` forbids: `.ix()` widens to a `usize` index, `to_u32` and friends narrow with
/// saturation (a value that big is refused by a limit long before it saturates).
pub trait Ix {
    fn ix(self) -> usize;
}

macro_rules! ix {
    ($($t:ty),*) => {$(
        impl Ix for $t {
            fn ix(self) -> usize {
                usize::try_from(self).unwrap_or(usize::MAX)
            }
        }
    )*};
}
ix!(u8, u16, u32, u64);

pub fn to_u32(n: impl TryInto<u32>) -> u32 {
    n.try_into().unwrap_or(u32::MAX)
}

pub fn to_u16(n: impl TryInto<u16>) -> u16 {
    n.try_into().unwrap_or(u16::MAX)
}

pub fn to_u8(n: impl TryInto<u8>) -> u8 {
    n.try_into().unwrap_or(u8::MAX)
}

/// Slot index into the scene's arrays.
pub type Idx = u32;
/// No node (a link that leads nowhere).
pub const NIL: Idx = u32::MAX;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct NodeId(pub u32);

impl NodeId {
    pub const ROOT: NodeId = NodeId(0);

    pub fn new(slot: u32, generation: u8) -> NodeId {
        NodeId((slot & 0x00ff_ffff) | (u32::from(generation) << 24))
    }

    pub const fn slot(self) -> Idx {
        self.0 & 0x00ff_ffff
    }

    pub fn generation(self) -> u8 {
        to_u8(self.0 >> 24)
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
