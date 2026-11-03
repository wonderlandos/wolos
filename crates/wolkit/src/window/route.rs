// SPDX-License-Identifier: BSD-2-Clause
//! Which node an event goes to: the nearest ancestor (or the node itself) that asked for that kind
//! with `BIND_EVENT`. One event, one receiver, never the whole path.

use crate::event::bit;
use crate::scene::{Idx, NIL, SceneRead, state};

/// The receiver of an event of `kind` that happened at `node`, if any asked for it.
pub fn receiver(scene: &dyn SceneRead, node: Idx, kind: u16) -> Option<Idx> {
    let mut n = node;
    let mut guard = 0;
    while n != NIL && guard <= crate::scene::MAX_DEPTH + 1 {
        if scene.event_mask(n) & bit(kind) != 0 {
            return Some(n);
        }
        n = scene.parent(n);
        guard += 1;
    }
    None
}

/// Whether `node` or an ancestor is disabled.
pub fn disabled(scene: &dyn SceneRead, node: Idx) -> bool {
    let mut n = node;
    let mut guard = 0;
    while n != NIL && guard <= crate::scene::MAX_DEPTH + 1 {
        if scene.state(n) & state::DISABLED != 0 {
            return true;
        }
        n = scene.parent(n);
        guard += 1;
    }
    false
}
