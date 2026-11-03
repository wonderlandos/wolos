// SPDX-License-Identifier: BSD-2-Clause
//! One frame of the shared pipeline: style, then layout. Backends call this before they read
//! rectangles (hit testing, `node_rect`, rendering) or present.

use super::WindowCore;
use crate::scene::{SceneRead, SceneWrite, dirty};

/// Bring styles and layout up to date for the window's size. Cheap when nothing changed.
pub fn ensure_layout(w: &mut WindowCore) {
    let key = (
        w.scene.version(),
        w.width,
        w.height,
        w.scale.to_bits(),
        w.style_version,
    );
    if w.laid_out == Some(key) {
        return;
    }
    crate::style::resolve(
        &mut w.scene,
        w.style.as_deref(),
        &mut w.styles,
        &w.env,
        w.width,
    );
    crate::layout::scene::run(
        &mut w.scene,
        &w.styles,
        &mut w.layout_cache,
        None,
        w.width as f32,
        w.height as f32,
    );
    w.laid_out = Some(key);
}

/// A frame as the engine sees it: bring layout up to date, "paint", and clear the paint and
/// accessibility bits. The headless backend has no screen to draw to, so this is also what a
/// test's `advance_time` runs. Allocates nothing.
pub fn frame(w: &mut WindowCore) {
    ensure_layout(w);
    clear_paint(&mut w.scene, 0);
}

fn clear_paint(scene: &mut dyn SceneWrite, root: u32) {
    // Only subtrees marked dirty are visited.
    let mut cur = root;
    loop {
        let d = scene.dirty(cur);
        if d & (dirty::PAINT | dirty::A11Y | dirty::SUBTREE | dirty::STYLE | dirty::CHILDREN) != 0 {
            scene.clear_dirty(
                cur,
                dirty::PAINT | dirty::A11Y | dirty::CHILDREN | dirty::STYLE,
            );
            let c = scene.first_child(cur);
            if d & dirty::SUBTREE != 0 && c != crate::scene::NIL {
                scene.clear_subtree(cur);
                cur = c;
                continue;
            }
        }
        loop {
            if cur == root {
                return;
            }
            let n = scene.next_sibling(cur);
            if n != crate::scene::NIL {
                cur = n;
                break;
            }
            cur = scene.parent(cur);
        }
    }
}
