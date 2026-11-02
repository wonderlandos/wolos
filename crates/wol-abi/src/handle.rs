// SPDX-License-Identifier: BSD-2-Clause
//! Tagged handles (audit B7). Every opaque object libwol hands out starts with a [`Tag`]: a
//! per-type magic plus a generation number. Each entry point goes through [`live`] (or
//! `string::borrow`, which calls it), so a pointer of the wrong type, a stray pointer or a
//! handle already released is refused with `WOL_ERR_INVALID_ARGUMENT` instead of being used.
//! On drop the magic is overwritten with [`DEAD`], which catches use after release while the
//! memory is not reused yet. The C ABI (ADR 0020) is unchanged: handles stay plain pointers.
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};

/// Written over the magic when an object is dropped.
pub const DEAD: u64 = 0xdead_dead_dead_dead;

static GENERATION: AtomicU64 = AtomicU64::new(1);

/// A handle type and its magic.
pub trait Tagged: Sized {
    const MAGIC: u64;
    fn tag(&self) -> &Tag<Self>;
}

/// First field of every handle struct (`#[repr(C)]`, so its offset is 0).
#[repr(C)]
pub struct Tag<T> {
    magic: AtomicU64,
    generation: u64,
    _of: PhantomData<fn() -> T>,
}

impl<T: Tagged> Tag<T> {
    pub fn new() -> Self {
        Tag {
            magic: AtomicU64::new(T::MAGIC),
            generation: GENERATION.fetch_add(1, Ordering::Relaxed),
            _of: PhantomData,
        }
    }

    pub fn is_live(&self) -> bool {
        self.magic.load(Ordering::Acquire) == T::MAGIC && self.generation != 0
    }

    /// When this object was made: distinct for every handle of the process.
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

impl<T: Tagged> Default for Tag<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Drop for Tag<T> {
    fn drop(&mut self) {
        self.magic.store(DEAD, Ordering::Release);
    }
}

/// Reads only the tag first: a pointer that is not aligned for `T` or whose tag is not `T`'s
/// live magic gives None.
///
/// # Safety
/// `p` is NULL or points at readable memory at least the size of a [`Tag`] (any libwol handle).
pub unsafe fn live<'a, T: Tagged>(p: *const T) -> Option<&'a T> {
    if p.is_null() || !p.is_aligned() {
        return None;
    }
    // SAFETY: the tag is the first field of every `#[repr(C)]` handle; it is read before the
    // object is trusted as a `T`.
    let tag = unsafe { &*p.cast::<Tag<T>>() };
    if !tag.is_live() {
        return None;
    }
    // SAFETY: the tag says this is a live `T`.
    let t = unsafe { &*p };
    // Same object through the trait: catches a type whose tag is not its first field.
    std::ptr::eq(t.tag(), tag).then_some(t)
}

/// Declares the magic of a handle type. The struct must be `#[repr(C)]` with `tag: Tag<Self>`
/// as its first field.
macro_rules! tagged {
    ($t:ty, $magic:expr) => {
        impl $crate::handle::Tagged for $t {
            const MAGIC: u64 = $magic;
            fn tag(&self) -> &$crate::handle::Tag<Self> {
                &self.tag
            }
        }
    };
}
pub(crate) use tagged;

#[cfg(test)]
mod tests {
    use super::*;

    #[repr(C)]
    struct A {
        tag: Tag<A>,
        v: u32,
    }
    #[repr(C)]
    struct B {
        tag: Tag<B>,
        v: u32,
    }
    tagged!(A, 0x0a0a);
    tagged!(B, 0x0b0b);

    #[test]
    fn wrong_type_and_null_are_refused() {
        let a = A {
            tag: Tag::new(),
            v: 7,
        };
        let pa: *const A = &a;
        assert_eq!(unsafe { live(pa) }.map(|a| a.v), Some(7));
        assert!(unsafe { live::<B>(pa.cast()) }.is_none());
        assert!(unsafe { live::<A>(std::ptr::null()) }.is_none());
        let b = B {
            tag: Tag::new(),
            v: 1,
        };
        assert!(b.tag.generation() > a.tag.generation());
        assert_eq!(b.v, 1);
    }

    #[test]
    fn a_dropped_tag_is_dead() {
        let mut t = std::mem::ManuallyDrop::new(Tag::<A>::new());
        assert!(t.is_live());
        // SAFETY: dropped once, then only the atomic is read (still in this stack slot).
        unsafe { std::mem::ManuallyDrop::drop(&mut t) };
        assert!(!t.is_live());
    }
}
