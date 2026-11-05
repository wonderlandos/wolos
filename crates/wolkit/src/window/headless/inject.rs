// SPDX-License-Identifier: BSD-2-Clause
//! Injected input for the headless backend: hit testing, hover, press, click, focus, slider drag,
//! scroll and a one-line text editor. This is the routing the native backend shares in spirit
//! (the nearest ancestor that asked for a kind receives it); WK-W4 may move it into `input/`.
//! Edits are applied to the scene at once and reported as events carrying the last patch's `seq`,
//! so an app's controlled value can tell an echo from news.

use super::super::route::{disabled, receiver};
use super::super::{
    KeyInput, MOD_CTRL, MOD_SHIFT, POINTER_DOWN, POINTER_LEAVE, POINTER_MOVE, POINTER_SCROLL,
    POINTER_UP, PointerInput, Sink, WindowCore, pipeline,
};
use crate::error::{Error, Result};
use crate::event::kind as ev;
use crate::scene::ids::{Ix, to_u32};
use crate::scene::prop::id;
use crate::scene::{Idx, Kind, NIL, PropValue, SceneRead, SceneWrite, state};

pub const KEY_RETURN: u32 = 0xff0d;
pub const KEY_BACKSPACE: u32 = 0xff08;
pub const KEY_TAB: u32 = 0xff09;
pub const KEY_SPACE: u32 = 0x20;

fn f32s(a: f32, b: f32) -> [u8; 8] {
    let mut o = [0u8; 8];
    o[..4].copy_from_slice(&a.to_le_bytes());
    o[4..].copy_from_slice(&b.to_le_bytes());
    o
}

fn u32s<const N: usize>(v: [u32; N]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// The deepest node under the point, later siblings on top; hidden nodes and nodes that ignore
/// the pointer do not count. A part that clips (`overflow` hidden, scroll or auto) hides the
/// children outside its rectangle; one that does not lets them be hit where they overflow.
pub fn hit_test(scene: &dyn SceneRead, x: f32, y: f32) -> Idx {
    fn skip(scene: &dyn SceneRead, c: Idx) -> bool {
        scene.prop(c, id::VISIBILITY).is_some_and(|v| v.w[0] == 1)
            || scene.prop(c, id::DISPLAY).is_some_and(|v| v.w[0] == 1)
            || scene
                .prop(c, id::POINTER_EVENTS)
                .is_some_and(|v| v.w[0] == 1)
    }
    fn at(scene: &dyn SceneRead, n: Idx, x: f32, y: f32, depth: usize) -> Option<Idx> {
        if depth > crate::scene::MAX_DEPTH + 1 {
            return None;
        }
        let inside = scene.rect(n).contains(x, y);
        let clips = scene.prop(n, id::OVERFLOW).is_some_and(|v| v.w[0] != 0);
        if !(clips && !inside) {
            let mut c = scene.last_child(n);
            while c != NIL {
                if !skip(scene, c)
                    && let Some(h) = at(scene, c, x, y, depth + 1)
                {
                    return Some(h);
                }
                c = scene.prev_sibling(c);
            }
        }
        inside.then_some(n)
    }
    at(scene, 0, x, y, 0).unwrap_or(0)
}

fn chain(scene: &dyn SceneRead, mut n: Idx, out: &mut Vec<Idx>) {
    out.clear();
    while n != NIL && out.len() <= crate::scene::MAX_DEPTH + 1 {
        out.push(n);
        n = scene.parent(n);
    }
}

fn focusable(scene: &dyn SceneRead, n: Idx) -> bool {
    let explicit = scene.prop(n, id::FOCUSABLE).map(|v| v.w[0] == 1);
    explicit.unwrap_or_else(|| scene.kind(n).focusable()) && !disabled(scene, n)
}

fn nearest_focusable(scene: &dyn SceneRead, mut n: Idx) -> Idx {
    while n != NIL {
        if focusable(scene, n) {
            return n;
        }
        n = scene.parent(n);
    }
    NIL
}

pub fn set_focus(w: &mut WindowCore, s: &mut Sink, to: Idx, visible: bool) {
    let old = w.scene.focus();
    if old == to {
        if to != NIL {
            w.scene.set_engine_state(to, state::FOCUS_VISIBLE, visible);
        }
        return;
    }
    w.scene.focus = to;
    if old != NIL {
        w.scene
            .set_engine_state(old, state::FOCUS | state::FOCUS_VISIBLE, false);
        if let Some(r) = receiver(&w.scene, old, ev::FOCUS) {
            s.emit(ev::FOCUS, w.scene.id(r).0, &0u32.to_le_bytes(), &[]);
        }
    }
    if to != NIL {
        w.scene.set_engine_state(to, state::FOCUS, true);
        w.scene.set_engine_state(to, state::FOCUS_VISIBLE, visible);
        if let Some(r) = receiver(&w.scene, to, ev::FOCUS) {
            s.emit(ev::FOCUS, w.scene.id(r).0, &1u32.to_le_bytes(), &[]);
        }
    }
}

fn set_chain_state(scene: &mut dyn SceneWrite, nodes: &[Idx], mask: u32, on: bool) {
    for n in nodes {
        scene.set_engine_state(*n, mask, on);
    }
}

fn slider_value(w: &mut WindowCore, s: &mut Sink, node: Idx, x: f32) {
    let get = |id: u16, d: f32| w.scene.prop(node, id).map_or(d, |v| v.f32());
    let (min, max) = (get(id::SLIDER_MIN, 0.0), get(id::SLIDER_MAX, 1.0));
    let step = get(id::SLIDER_STEP, 0.0);
    let r = w.scene.rect(node);
    if r.w <= 0.0 || max <= min {
        return;
    }
    let mut v = min + ((x - r.x) / r.w).clamp(0.0, 1.0) * (max - min);
    if step > 0.0 {
        v = (min + ((v - min) / step).round() * step).clamp(min, max);
    }
    if w.scene.prop(node, id::SLIDER_VALUE).map(|p| p.f32()) == Some(v) {
        return;
    }
    w.scene
        .set_prop(node, id::SLIDER_VALUE, &PropValue::new(&[v.to_bits()]));
    if let Some(r) = receiver(&w.scene, node, ev::SLIDER) {
        s.emit(ev::SLIDER, w.scene.id(r).0, &v.to_le_bytes(), &[]);
    }
}

fn toggle(w: &mut WindowCore, s: &mut Sink, node: Idx) {
    let on = w.scene.state(node) & state::CHECKED == 0;
    w.scene.state[node.ix()] ^= state::CHECKED;
    w.scene.mark_dirty(
        node,
        crate::scene::dirty::STYLE | crate::scene::dirty::PAINT | crate::scene::dirty::A11Y,
    );
    if let Some(r) = receiver(&w.scene, node, ev::TOGGLE) {
        s.emit(
            ev::TOGGLE,
            w.scene.id(r).0,
            &u32::from(on).to_le_bytes(),
            &[],
        );
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
        let mut head = u32s([button, mods]);
        head.extend_from_slice(&f32s(x, y));
        s.emit(ev::CLICK, w.scene.id(r).0, &head, &[]);
    }
}

pub fn pointer(w: &mut WindowCore, s: &mut Sink, p: PointerInput) -> Result<()> {
    if !p.x.is_finite() || !p.y.is_finite() {
        return Err(Error::invalid("A pointer position is a finite number."));
    }
    s.seq = w.scene.last_seq();
    pipeline::ensure_layout(w);
    if p.kind != POINTER_SCROLL {
        w.pointer = (p.x, p.y);
    }
    let hit = if p.kind == POINTER_LEAVE {
        NIL
    } else {
        hit_test(&w.scene, p.x, p.y)
    };
    let (mut old, mut new) = (Vec::new(), Vec::new());
    match p.kind {
        POINTER_MOVE | POINTER_LEAVE | POINTER_DOWN | POINTER_UP => {
            if hit != w.hover {
                chain(&w.scene, w.hover, &mut old);
                chain(&w.scene, hit, &mut new);
                for n in old.iter().filter(|n| !new.contains(n)) {
                    w.scene.set_engine_state(*n, state::HOVER, false);
                    if w.scene.event_mask(*n) & crate::event::bit(ev::POINTER_LEAVE) != 0 {
                        s.emit(ev::POINTER_LEAVE, w.scene.id(*n).0, &f32s(p.x, p.y), &[]);
                    }
                }
                for n in new.iter().filter(|n| !old.contains(n)) {
                    w.scene.set_engine_state(*n, state::HOVER, true);
                    if w.scene.event_mask(*n) & crate::event::bit(ev::POINTER_ENTER) != 0 {
                        s.emit(ev::POINTER_ENTER, w.scene.id(*n).0, &f32s(p.x, p.y), &[]);
                    }
                }
                w.hover = hit;
            }
        }
        POINTER_SCROLL => {}
        _ => return Err(Error::invalid("That is not a pointer event kind.")),
    }
    match p.kind {
        POINTER_DOWN => {
            if hit == NIL {
                return Ok(());
            }
            w.pressed = hit;
            chain(&w.scene, hit, &mut new);
            set_chain_state(&mut w.scene, &new, state::ACTIVE, true);
            if !disabled(&w.scene, hit) {
                let f = nearest_focusable(&w.scene, hit);
                set_focus(w, s, f, false);
                let target = if f != NIL { f } else { hit };
                if w.scene.kind(target) == Kind::Slider {
                    slider_value(w, s, target, p.x);
                }
            }
        }
        POINTER_MOVE => {
            if w.pressed != NIL
                && w.scene.kind(w.pressed) == Kind::Slider
                && !disabled(&w.scene, w.pressed)
            {
                let n = w.pressed;
                slider_value(w, s, n, p.x);
            }
        }
        POINTER_UP => {
            let pressed = std::mem::replace(&mut w.pressed, NIL);
            if pressed == NIL {
                return Ok(());
            }
            chain(&w.scene, pressed, &mut old);
            set_chain_state(&mut w.scene, &old, state::ACTIVE, false);
            // A click is press and release on the same part (or one inside the other).
            let same = hit == pressed
                || w.scene.is_ancestor(pressed, hit)
                || w.scene.is_ancestor(hit, pressed);
            if same && hit != NIL && w.scene.kind(pressed) != Kind::Slider {
                click(w, s, pressed, p.x, p.y, p.button, p.mods);
            }
        }
        POINTER_SCROLL => {
            let at = hit_test(&w.scene, w.pointer.0, w.pointer.1);
            let mut n = at;
            while n != NIL && !matches!(w.scene.kind(n), Kind::Scroll | Kind::Vlist) {
                n = w.scene.parent(n);
            }
            if n != NIL {
                let cur = w.scene.prop(n, id::SCROLL_OFFSET).map_or((0.0, 0.0), |v| {
                    (f32::from_bits(v.w[0]), f32::from_bits(v.w[1]))
                });
                let next = ((cur.0 + p.x).max(0.0), (cur.1 + p.y).max(0.0));
                w.scene.set_prop(
                    n,
                    id::SCROLL_OFFSET,
                    &PropValue::new(&[next.0.to_bits(), next.1.to_bits()]),
                );
                if let Some(r) = receiver(&w.scene, n, ev::SCROLL) {
                    s.emit(ev::SCROLL, w.scene.id(r).0, &f32s(next.0, next.1), &[]);
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn next_focus(scene: &dyn SceneRead, from: Idx, backwards: bool) -> Idx {
    // Tree order; a positive tab-index goes first (ascending), then the rest.
    let mut order: Vec<(i32, Idx)> = Vec::new();
    crate::scene::walk(scene, 0, &mut |i, _| {
        if i != 0 && focusable(scene, i) {
            let t = scene
                .prop(i, id::TAB_INDEX)
                .map_or(0, |v| v.w[0].cast_signed());
            if t >= 0 {
                order.push((if t > 0 { t } else { i32::MAX }, i));
            }
        }
    });
    order.sort_by_key(|(t, _)| *t);
    if order.is_empty() {
        return NIL;
    }
    let at = order.iter().position(|(_, i)| *i == from);
    let n = order.len();
    let k = match (at, backwards) {
        (Some(a), false) => (a + 1) % n,
        (Some(a), true) => (a + n - 1) % n,
        (None, false) => 0,
        (None, true) => n - 1,
    };
    order[k].1
}

pub fn key(w: &mut WindowCore, s: &mut Sink, k: KeyInput) -> Result<()> {
    s.seq = w.scene.last_seq();
    pipeline::ensure_layout(w);
    let target = if w.scene.focus() == NIL {
        0
    } else {
        w.scene.focus()
    };
    if let Some(r) = receiver(&w.scene, target, ev::KEY) {
        s.emit(
            ev::KEY,
            w.scene.id(r).0,
            &u32s([k.keysym, u32::from(k.pressed), k.mods, 0]),
            &[],
        );
    }
    if !k.pressed {
        return Ok(());
    }
    if k.keysym == KEY_TAB {
        let to = next_focus(&w.scene, w.scene.focus(), k.mods & MOD_SHIFT != 0);
        if to != NIL {
            set_focus(w, s, to, true);
        }
        return Ok(());
    }
    let f = w.scene.focus();
    if f == NIL || disabled(&w.scene, f) {
        return Ok(());
    }
    match (w.scene.kind(f), k.keysym) {
        (Kind::Button, KEY_RETURN | KEY_SPACE) => click(w, s, f, 0.0, 0.0, 0, k.mods),
        (Kind::Toggle, KEY_RETURN | KEY_SPACE) => toggle(w, s, f),
        // In a text area (`white-space: pre-wrap`) Enter breaks the line; the headless backend
        // only appends (it has no caret), the native one edits at the caret (`input::edit`).
        (Kind::TextInput, KEY_RETURN)
            if w.styles.get(w.scene.computed(f)).text.white_space
                == crate::style::value::WhiteSpace::PreWrap
                && k.mods & MOD_CTRL == 0 =>
        {
            let mut text = w.scene.text(f).to_owned();
            text.push('\n');
            if text.len() <= crate::scene::patch::MAX_STRING {
                edit(w, s, f, text);
            }
        }
        (Kind::TextInput, KEY_RETURN) => {
            if let Some(r) = receiver(&w.scene, f, ev::TEXT_SUBMIT) {
                let text = w.scene.text(f).to_owned();
                let n = to_u32(text.len());
                s.emit(
                    ev::TEXT_SUBMIT,
                    w.scene.id(r).0,
                    &u32s([n, n, n]),
                    text.as_bytes(),
                );
            }
        }
        (Kind::TextInput, KEY_BACKSPACE) => {
            let mut text = w.scene.text(f).to_owned();
            if text.pop().is_some() {
                edit(w, s, f, text);
            }
        }
        _ => {}
    }
    Ok(())
}

fn edit(w: &mut WindowCore, s: &mut Sink, node: Idx, text: String) {
    let mut cur = w.scene.text[node.ix()];
    match w.scene.set_str(&mut cur, &text) {
        Some(true) => {
            let old = w.scene.text[node.ix()];
            w.scene.text[node.ix()] = cur;
            w.scene.strings.release(old);
            w.scene.mark_dirty(
                node,
                crate::scene::dirty::LAYOUT
                    | crate::scene::dirty::PAINT
                    | crate::scene::dirty::A11Y,
            );
            w.scene.version += 1;
        }
        Some(false) => {}
        None => return,
    }
    if let Some(r) = receiver(&w.scene, node, ev::TEXT_CHANGED) {
        let n = to_u32(text.len());
        s.emit(
            ev::TEXT_CHANGED,
            w.scene.id(r).0,
            &u32s([n, n, n]),
            text.as_bytes(),
        );
    }
}

pub fn text(w: &mut WindowCore, s: &mut Sink, t: &str) -> Result<()> {
    s.seq = w.scene.last_seq();
    let f = w.scene.focus();
    if f == NIL || w.scene.kind(f) != Kind::TextInput || disabled(&w.scene, f) {
        return Ok(());
    }
    if w.scene.prop(f, id::READONLY).is_some_and(|v| v.w[0] == 1) {
        return Ok(());
    }
    let t: String = t.chars().filter(|c| !c.is_control()).collect();
    let mut next = w.scene.text(f).to_owned();
    next.push_str(&t);
    let max = w
        .scene
        .prop(f, id::MAX_LENGTH)
        .map_or(0, |v| v.w[0].cast_signed());
    if max > 0 {
        next = next
            .chars()
            .take(usize::try_from(max).unwrap_or(0))
            .collect();
    }
    if next.len() > crate::scene::patch::MAX_STRING {
        return Err(Error::limit("A text field holds at most 64 KiB."));
    }
    edit(w, s, f, next);
    Ok(())
}
