// SPDX-License-Identifier: BSD-2-Clause
//! The pointer: hover (`POINTER_ENTER`/`POINTER_LEAVE` along the changed part of the ancestor
//! chain), press with pointer capture (moves and the release go to the pressed node even outside
//! it, so a slider drag or a button press that slides off behaves), click on release over the
//! same part, focus on press (no ring), scroll to the nearest scroll container under the pointer,
//! and the secondary button's `CONTEXT_MENU` (`f32 x`, `f32 y`).

use super::{BUTTON_RIGHT, InputState, Request, edit, focus, hit};
use crate::error::{Error, Result};
use crate::event::kind as ev;
use crate::scene::prop::id;
use crate::scene::{Idx, Kind, NIL, PropValue, SceneRead, SceneWrite, dirty, state};
use crate::window::route::{disabled, receiver};
use crate::window::{
    POINTER_DOWN, POINTER_LEAVE, POINTER_MOVE, POINTER_SCROLL, POINTER_UP, PointerInput, Sink,
    WindowCore,
};

fn f32s(a: f32, b: f32) -> [u8; 8] {
    let mut o = [0u8; 8];
    o[..4].copy_from_slice(&a.to_le_bytes());
    o[4..].copy_from_slice(&b.to_le_bytes());
    o
}

fn chain(scene: &dyn SceneRead, mut n: Idx, out: &mut Vec<Idx>) {
    out.clear();
    while n != NIL && out.len() <= crate::scene::MAX_DEPTH + 1 {
        out.push(n);
        n = scene.parent(n);
    }
}

/// Set a slider to `v` (clamped to its range and snapped to its step) and report it.
pub fn set_slider(w: &mut WindowCore, s: &mut Sink, node: Idx, v: f32) {
    let get = |p: u16, d: f32| w.scene.prop(node, p).map_or(d, |x| x.f32());
    let (min, max, step) = (
        get(id::SLIDER_MIN, 0.0),
        get(id::SLIDER_MAX, 1.0),
        get(id::SLIDER_STEP, 0.0),
    );
    if !v.is_finite() || max <= min {
        return;
    }
    let mut v = v.clamp(min, max);
    if step > 0.0 {
        v = (min + ((v - min) / step).round() * step).clamp(min, max);
    }
    if w.scene.prop(node, id::SLIDER_VALUE).map(|p| p.f32()) == Some(v) {
        return;
    }
    w.scene
        .set_prop(node, id::SLIDER_VALUE, &PropValue::new(&[v.to_bits()]));
    w.scene.mark_dirty(node, dirty::PAINT | dirty::A11Y);
    if let Some(r) = receiver(&w.scene, node, ev::SLIDER) {
        s.emit(ev::SLIDER, w.scene.id(r).0, &v.to_le_bytes(), &[]);
    }
}

fn slider_at(w: &mut WindowCore, s: &mut Sink, node: Idx, x: f32) {
    let r = hit::drawn_rect(&w.scene, node);
    if r.w <= 0.0 {
        return;
    }
    let get = |p: u16, d: f32| w.scene.prop(node, p).map_or(d, |x| x.f32());
    let (min, max) = (get(id::SLIDER_MIN, 0.0), get(id::SLIDER_MAX, 1.0));
    let v = min + ((x - r.x) / r.w).clamp(0.0, 1.0) * (max - min);
    set_slider(w, s, node, v);
}

fn toggle(w: &mut WindowCore, s: &mut Sink, node: Idx) {
    let on = w.scene.state(node) & state::CHECKED == 0;
    if let Some(st) = w.scene.state.get_mut(usize::try_from(node).unwrap_or(usize::MAX)) {
        *st ^= state::CHECKED;
    }
    w.scene
        .mark_dirty(node, dirty::STYLE | dirty::PAINT | dirty::A11Y);
    if let Some(r) = receiver(&w.scene, node, ev::TOGGLE) {
        s.emit(ev::TOGGLE, w.scene.id(r).0, &u32::from(on).to_le_bytes(), &[]);
    }
}

fn click(w: &mut WindowCore, s: &mut Sink, node: Idx, x: f32, y: f32, button: u32, mods: u32) {
    if disabled(&w.scene, node) {
        return;
    }
    if w.scene.kind(node) == Kind::Toggle {
        return toggle(w, s, node);
    }
    if let Some(r) = receiver(&w.scene, node, ev::CLICK) {
        let mut head = Vec::with_capacity(16);
        head.extend_from_slice(&button.to_le_bytes());
        head.extend_from_slice(&mods.to_le_bytes());
        head.extend_from_slice(&f32s(x, y));
        s.emit(ev::CLICK, w.scene.id(r).0, &head, &[]);
    }
}

/// Activate `node` the way Enter, Space or an assistive technology's default action does: a
/// toggle flips, anything else clicks (button 0, at the node's centre). Returns whether the node
/// has an activation.
pub fn activate(w: &mut WindowCore, s: &mut Sink, node: Idx, mods: u32) -> bool {
    if node == NIL || disabled(&w.scene, node) {
        return false;
    }
    let k = w.scene.kind(node);
    let clickable = receiver(&w.scene, node, ev::CLICK).is_some();
    if k != Kind::Toggle && k != Kind::Button && !clickable {
        return false;
    }
    let r = hit::drawn_rect(&w.scene, node);
    click(w, s, node, r.x + r.w / 2.0, r.y + r.h / 2.0, 0, mods);
    true
}

/// `CONTEXT_MENU` at a window point to `node`'s receiver.
pub fn context_menu(w: &mut WindowCore, s: &mut Sink, node: Idx, x: f32, y: f32) {
    if let Some(r) = receiver(&w.scene, node, ev::CONTEXT_MENU) {
        s.emit(ev::CONTEXT_MENU, w.scene.id(r).0, &f32s(x, y), &[]);
    }
}

fn hover(w: &mut WindowCore, s: &mut Sink, st: &mut InputState, hit: Idx, x: f32, y: f32) {
    if hit == w.hover {
        return;
    }
    let (mut old, mut new) = (std::mem::take(&mut st.chain_a), std::mem::take(&mut st.chain_b));
    chain(&w.scene, w.hover, &mut old);
    chain(&w.scene, hit, &mut new);
    for n in old.iter().filter(|n| !new.contains(n)) {
        if !w.scene.is_live(*n) {
            continue;
        }
        w.scene.set_engine_state(*n, state::HOVER, false);
        if w.scene.event_mask(*n) & crate::event::bit(ev::POINTER_LEAVE) != 0 {
            s.emit(ev::POINTER_LEAVE, w.scene.id(*n).0, &f32s(x, y), &[]);
        }
    }
    for n in new.iter().filter(|n| !old.contains(n)) {
        w.scene.set_engine_state(*n, state::HOVER, true);
        if w.scene.event_mask(*n) & crate::event::bit(ev::POINTER_ENTER) != 0 {
            s.emit(ev::POINTER_ENTER, w.scene.id(*n).0, &f32s(x, y), &[]);
        }
    }
    w.hover = hit;
    (st.chain_a, st.chain_b) = (old, new);
}

/// Scroll the nearest scroll container under the pointer by `(dx, dy)` logical pixels.
fn scroll(w: &mut WindowCore, s: &mut Sink, st: &InputState, dx: f32, dy: f32) {
    let (px, py) = w.pointer;
    let mut n = hit::hit_test(&w.scene, &st.layers, px, py);
    while n != NIL && !matches!(w.scene.kind(n), Kind::Scroll | Kind::Vlist) {
        n = w.scene.parent(n);
    }
    if n == NIL {
        return;
    }
    let cur = w.scene.prop(n, id::SCROLL_OFFSET).map_or((0.0, 0.0), |v| {
        (f32::from_bits(v.w[0]), f32::from_bits(v.w[1]))
    });
    let next = ((cur.0 + dx).max(0.0), (cur.1 + dy).max(0.0));
    if next == cur {
        return;
    }
    w.scene.set_prop(
        n,
        id::SCROLL_OFFSET,
        &PropValue::new(&[next.0.to_bits(), next.1.to_bits()]),
    );
    w.scene.mark_dirty(n, dirty::PAINT);
    if let Some(r) = receiver(&w.scene, n, ev::SCROLL) {
        s.emit(ev::SCROLL, w.scene.id(r).0, &f32s(next.0, next.1), &[]);
    }
}

/// Route one pointer event (window coordinates; for `POINTER_SCROLL`, `x` and `y` are the deltas).
/// Returns the input-method request a focus change made.
pub fn pointer(
    w: &mut WindowCore,
    s: &mut Sink,
    st: &mut InputState,
    p: PointerInput,
) -> Result<Request> {
    if !p.x.is_finite() || !p.y.is_finite() {
        return Err(Error::invalid("A pointer position is a finite number."));
    }
    s.seq = w.scene.last_seq();
    crate::window::pipeline::ensure_layout(w);
    focus::sync(w, s, st);
    st.layers.refresh(&w.scene);
    if p.kind == POINTER_SCROLL {
        scroll(w, s, st, p.x, p.y);
        return Ok(Request::None);
    }
    if !matches!(p.kind, POINTER_MOVE | POINTER_LEAVE | POINTER_DOWN | POINTER_UP) {
        return Err(Error::invalid("That is not a pointer event kind."));
    }
    w.pointer = (p.x, p.y);
    let hit = if p.kind == POINTER_LEAVE {
        NIL
    } else {
        hit::hit_test(&w.scene, &st.layers, p.x, p.y)
    };
    // While a button is held, hover stays with the pressed node (capture).
    let captured = w.pressed != NIL && w.scene.is_live(w.pressed);
    if !captured || p.kind == POINTER_UP {
        hover(w, s, st, hit, p.x, p.y);
    }
    let mut req = Request::None;
    match p.kind {
        POINTER_DOWN if hit != NIL => {
            if p.button == BUTTON_RIGHT {
                context_menu(w, s, hit, p.x, p.y);
                return Ok(req);
            }
            w.pressed = hit;
            let mut c = std::mem::take(&mut st.chain_a);
            chain(&w.scene, hit, &mut c);
            for n in &c {
                w.scene.set_engine_state(*n, state::ACTIVE, true);
            }
            st.chain_a = c;
            if !disabled(&w.scene, hit) {
                let f = focus::nearest_focusable(&w.scene, hit);
                if f != NIL || st.focused != NIL {
                    focus::set_focus(w, s, st, f, false);
                    req = edit::focus_changed(w, st);
                }
                if f != NIL && w.scene.kind(f) == Kind::Slider {
                    w.pressed = f;
                    slider_at(w, s, f, p.x);
                }
            }
        }
        POINTER_MOVE if captured => {
            let n = w.pressed;
            if w.scene.kind(n) == Kind::Slider && !disabled(&w.scene, n) {
                slider_at(w, s, n, p.x);
            }
        }
        POINTER_UP => {
            let pressed = std::mem::replace(&mut w.pressed, NIL);
            if pressed == NIL || !w.scene.is_live(pressed) {
                return Ok(req);
            }
            let mut c = std::mem::take(&mut st.chain_a);
            chain(&w.scene, pressed, &mut c);
            for n in &c {
                w.scene.set_engine_state(*n, state::ACTIVE, false);
            }
            st.chain_a = c;
            // A click is press and release on the same part (or one inside the other).
            let same = hit != NIL
                && (hit == pressed
                    || hit::scene_is_inside(&w.scene, pressed, hit)
                    || hit::scene_is_inside(&w.scene, hit, pressed));
            if same && w.scene.kind(pressed) != Kind::Slider {
                click(w, s, pressed, p.x, p.y, p.button, p.mods);
            }
        }
        POINTER_LEAVE => {}
        _ => {}
    }
    Ok(req)
}
