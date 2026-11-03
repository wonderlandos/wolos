// SPDX-License-Identifier: BSD-2-Clause
//! Pointer coalescing (plan section 2.7): many motion events between two frames become the last
//! position, many scroll steps their sum. Anything that is not a move or a scroll (a button, a
//! key, the pointer leaving) first flushes what is pending, so order is kept: a press always sees
//! the position the pointer had when it was pressed.

use crate::window::{POINTER_MOVE, POINTER_SCROLL, PointerInput};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Coalescer {
    motion: Option<PointerInput>,
    scroll: Option<(f32, f32, u32)>,
    /// Raw events taken in, and events handed out, for the frame ring and tests.
    pub taken: u64,
    pub given: u64,
}

impl Coalescer {
    /// Take one event. Moves and scrolls are held; anything else is returned after what was held,
    /// in `out`, in order.
    pub fn push(&mut self, p: PointerInput, out: &mut Vec<PointerInput>) {
        self.taken += 1;
        match p.kind {
            POINTER_MOVE => {
                // A scroll before this move must stay before it (it scrolls under the old point).
                if self.scroll.is_some() {
                    self.flush(out);
                }
                self.motion = Some(p);
            }
            POINTER_SCROLL => {
                let (x, y, _) = self.scroll.unwrap_or((0.0, 0.0, p.mods));
                self.scroll = Some((x + p.x, y + p.y, p.mods));
            }
            _ => {
                self.flush(out);
                self.given += 1;
                out.push(p);
            }
        }
    }

    /// Hand out what is held: the last move, then the summed scroll.
    pub fn flush(&mut self, out: &mut Vec<PointerInput>) {
        if let Some(m) = self.motion.take() {
            self.given += 1;
            out.push(m);
        }
        if let Some((x, y, mods)) = self.scroll.take()
            && (x != 0.0 || y != 0.0)
        {
            self.given += 1;
            out.push(PointerInput {
                kind: POINTER_SCROLL,
                x,
                y,
                button: 0,
                mods,
            });
        }
    }

    pub fn is_empty(&self) -> bool {
        self.motion.is_none() && self.scroll.is_none()
    }
}
