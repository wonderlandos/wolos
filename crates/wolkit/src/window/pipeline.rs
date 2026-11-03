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
    let text = w.text.clone();
    let mut sys = crate::text::measure::lock(&text);
    crate::layout::scene::run_text(
        &mut w.scene,
        &w.styles,
        &mut w.layout_cache,
        (&mut sys, &mut w.text_cache),
        w.width as f32,
        w.height as f32,
    );
    w.laid_out = Some(key);
}

/// Draw the window on the CPU into `out` (RGBA8 straight alpha, `width * height * 4` bytes of
/// device pixels at `scale`): style and layout first, then the scene lowered with its styles, real
/// text and the token palette. The headless backend's `render_rgba` and a backend's `wl_shm` path.
pub fn render_rgba(w: &mut WindowCore, width: u32, height: u32, scale: f32, out: &mut [u8]) {
    ensure_layout(w);
    let text = w.text.clone();
    let mut sys = crate::text::measure::lock(&text);
    let frame = crate::cpu::Frame {
        scene: &w.scene,
        styles: &w.styles,
        env: &w.env,
        width,
        height,
        scale,
        canvases: Some(&w.canvases),
    };
    crate::cpu::render_frame(&frame, &mut sys, &mut w.text_cache, out);
}

/// Asks for every picture the window shows and waits (at most five seconds) until none is
/// decoding, so a frame drawn next does not depend on timing. The headless backend's
/// `render_rgba`; never on a display backend's frame path.
pub fn settle_images(w: &mut WindowCore) {
    ensure_layout(w);
    if crate::paint::scene::request_images(&w.scene) > 0 {
        crate::image::settle(std::time::Duration::from_secs(5));
    }
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
