// SPDX-License-Identifier: BSD-2-Clause
//! Keys (plan section 2.7). Every key goes to the app first as `KEY` (to the focused node's
//! receiver: `u32 keysym`, `u32 pressed`, `u32 mods`, `u32 repeat`), then the engine does what
//! the key means, in this order:
//!
//! 1. the focused text field's editing keys and clipboard shortcuts (`edit::key`);
//! 2. Tab and Shift+Tab move focus in tab order, inside the open modal layer if there is one;
//! 3. arrows, Home and End move between items of a list, or change a focused slider;
//! 4. Enter and Space activate a button or a toggle, Enter submits a text field;
//! 5. Escape asks the innermost overlay or modal layer to close (`CLOSE_REQUEST` with that node);
//! 6. Menu or Shift+F10 opens the focused node's context menu (`CONTEXT_MENU`);
//! 7. anything else with Control, Alt or Super (and F1 to F12 alone) is a `SHORTCUT`:
//!    `u32 keysym`, `u32 mods`, then its name in the `shared::view` shortcut form (`cmd+shift+s`;
//!    Command reaches apps as Control, as everywhere on WolOS).

use super::focus::{self, Move};
use super::{InputState, Request, edit, pointer};
use crate::event::kind as ev;
use crate::scene::prop::id;
use crate::scene::{Idx, Kind, NIL, PropValue, SceneRead, SceneWrite};
use crate::window::route::{disabled, receiver};
use crate::window::{MOD_ALT, MOD_CTRL, MOD_SHIFT, MOD_SUPER, Sink, WindowCore};

/// Keysyms the engine acts on (the XKB values).
pub mod sym {
    pub const BACKSPACE: u32 = 0xff08;
    pub const TAB: u32 = 0xff09;
    pub const ISO_LEFT_TAB: u32 = 0xfe20;
    pub const RETURN: u32 = 0xff0d;
    pub const KP_ENTER: u32 = 0xff8d;
    pub const ESCAPE: u32 = 0xff1b;
    pub const HOME: u32 = 0xff50;
    pub const LEFT: u32 = 0xff51;
    pub const UP: u32 = 0xff52;
    pub const RIGHT: u32 = 0xff53;
    pub const DOWN: u32 = 0xff54;
    pub const PAGE_UP: u32 = 0xff55;
    pub const PAGE_DOWN: u32 = 0xff56;
    pub const END: u32 = 0xff57;
    pub const MENU: u32 = 0xff67;
    pub const DELETE: u32 = 0xffff;
    pub const F1: u32 = 0xffbe;
    pub const F10: u32 = 0xffc7;
    pub const F12: u32 = 0xffc9;
    pub const SPACE: u32 = 0x20;
    pub const A: u32 = 0x61;
    pub const A_UPPER: u32 = 0x41;
    pub const C: u32 = 0x63;
    pub const V: u32 = 0x76;
    pub const X: u32 = 0x78;
    pub const Y: u32 = 0x79;
    pub const Z: u32 = 0x7a;
    pub const Z_UPPER: u32 = 0x5a;
    /// Modifier keys themselves (Shift_L .. Hyper_R): never a shortcut on their own.
    pub const MODIFIERS: std::ops::RangeInclusive<u32> = 0xffe1..=0xffee;
}

/// The `shared::view` shortcut name of a key with modifiers (`cmd+shift+s`, `f5`), if it is one.
pub fn shortcut_name(keysym: u32, mods: u32) -> Option<String> {
    let key = match keysym {
        sym::F1..=sym::F12 => format!("f{}", keysym - sym::F1 + 1),
        0x30..=0x39 | 0x61..=0x7a => char::from_u32(keysym)?.to_string(),
        0x41..=0x5a => char::from_u32(keysym + 0x20)?.to_string(),
        _ => return None,
    };
    let cmd = mods & (MOD_CTRL | MOD_SUPER) != 0;
    let function = key.starts_with('f') && key.len() > 1;
    if !cmd && mods & MOD_ALT == 0 && !function {
        return None;
    }
    let mut s = String::new();
    for (on, name) in [
        (cmd, "cmd+"),
        (mods & MOD_SHIFT != 0, "shift+"),
        (mods & MOD_ALT != 0, "alt+"),
    ] {
        if on {
            s.push_str(name);
        }
    }
    s.push_str(&key);
    shared::view::shortcut_ok(&s).then_some(s)
}

fn words<const N: usize>(v: [u32; N]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// A slider moves by its step, or a hundredth of its range; Page keys by a tenth.
fn slider_key(w: &mut WindowCore, s: &mut Sink, n: Idx, keysym: u32) -> bool {
    let get = |p: u16, d: f32| w.scene.prop(n, p).map_or(d, |v| v.f32());
    let (min, max) = (get(id::SLIDER_MIN, 0.0), get(id::SLIDER_MAX, 1.0));
    if max <= min {
        return false;
    }
    let step = get(id::SLIDER_STEP, 0.0);
    let small = if step > 0.0 { step } else { (max - min) / 100.0 };
    let big = ((max - min) / 10.0).max(small);
    let cur = get(id::SLIDER_VALUE, min);
    let v = match keysym {
        sym::LEFT | sym::DOWN => cur - small,
        sym::RIGHT | sym::UP => cur + small,
        sym::PAGE_DOWN => cur - big,
        sym::PAGE_UP => cur + big,
        sym::HOME => min,
        sym::END => max,
        _ => return false,
    };
    pointer::set_slider(w, s, n, v);
    true
}

/// The innermost overlay or modal layer around `from` (or open at all), or NIL.
fn closable(w: &WindowCore, st: &InputState, from: Idx) -> Idx {
    let mut n = from;
    let mut guard = 0;
    while n != NIL && guard <= crate::scene::MAX_DEPTH + 1 {
        if w.scene.kind(n) == Kind::Overlay || super::hit::is_modal(&w.scene, n) {
            return n;
        }
        n = w.scene.parent(n);
        guard += 1;
    }
    st.layers.list.last().map_or(NIL, |(i, _)| *i)
}

/// Route one key press or release. `text` is what the key types (from the keymap), if anything.
pub fn key(
    w: &mut WindowCore,
    s: &mut Sink,
    st: &mut InputState,
    keysym: u32,
    pressed: bool,
    mods: u32,
    repeat: bool,
    text: &str,
) -> Request {
    s.seq = w.scene.last_seq();
    crate::window::pipeline::ensure_layout(w);
    focus::sync(w, s, st);
    let f = st.focused;
    let target = if f == NIL { 0 } else { f };
    if let Some(r) = receiver(&w.scene, target, ev::KEY) {
        let head = words([keysym, u32::from(pressed), mods, u32::from(repeat)]);
        s.emit(ev::KEY, w.scene.id(r).0, &head, &[]);
    }
    if !pressed || sym::MODIFIERS.contains(&keysym) {
        return Request::None;
    }
    // 1. The text field.
    let in_field = f != NIL && w.scene.kind(f) == Kind::TextInput;
    if in_field {
        if let Some(r) = edit::key(w, s, st, keysym, mods) {
            return r;
        }
        if matches!(keysym, sym::RETURN | sym::KP_ENTER) {
            edit::submit(w, s, st);
            return Request::None;
        }
        let plain = mods & (MOD_CTRL | MOD_ALT | MOD_SUPER) == 0;
        if plain && !text.is_empty() && !text.chars().any(char::is_control) {
            edit::type_text(w, s, st, text);
            return Request::None;
        }
    }
    // 2. Tab order.
    if matches!(keysym, sym::TAB | sym::ISO_LEFT_TAB) && mods & (MOD_CTRL | MOD_ALT) == 0 {
        let back = keysym == sym::ISO_LEFT_TAB || mods & MOD_SHIFT != 0;
        let to = focus::next_in_order(&w.scene, focus::scope(&st.layers), f, back);
        if to != NIL {
            focus::set_focus(w, s, st, to, true);
        }
        return edit::focus_changed(w, st);
    }
    let plain = mods & (MOD_CTRL | MOD_ALT | MOD_SUPER) == 0;
    // 3. Lists and sliders.
    if plain && f != NIL && !disabled(&w.scene, f) {
        if w.scene.kind(f) == Kind::Slider && slider_key(w, s, f, keysym) {
            st.keyboard_mode = true;
            return Request::None;
        }
        let row = |w: &WindowCore| {
            let mut p = w.scene.parent(f);
            while p != NIL && w.scene.kind(p) != Kind::Vlist && w.scene.a11y(p).role != 9
                && w.scene.a11y(p).role != 13 && w.scene.a11y(p).role != 15
            {
                p = w.scene.parent(p);
            }
            p != NIL && focus::list_is_row(&w.scene, p)
        };
        let mv = match keysym {
            sym::UP if !row(w) => Some(Move::Prev),
            sym::DOWN if !row(w) => Some(Move::Next),
            sym::LEFT if row(w) => Some(Move::Prev),
            sym::RIGHT if row(w) => Some(Move::Next),
            sym::HOME if !in_field => Some(Move::First),
            sym::END if !in_field => Some(Move::Last),
            _ => None,
        };
        if let Some(m) = mv {
            let to = focus::in_list(&w.scene, f, m);
            if to != NIL {
                focus::set_focus(w, s, st, to, true);
                scroll_into_view(w, s, to);
                return edit::focus_changed(w, st);
            }
        }
    }
    // 4. Activation.
    if plain && f != NIL && matches!(keysym, sym::RETURN | sym::KP_ENTER | sym::SPACE) {
        st.keyboard_mode = true;
        w.scene
            .set_engine_state(f, crate::scene::state::FOCUS_VISIBLE, true);
        if pointer::activate(w, s, f, mods) {
            return Request::None;
        }
    }
    // 5. Escape.
    if keysym == sym::ESCAPE && plain {
        let l = closable(w, st, f);
        if l != NIL
            && let Some(r) = receiver(&w.scene, l, ev::CLOSE_REQUEST)
        {
            s.emit(ev::CLOSE_REQUEST, w.scene.id(r).0, &[], &[]);
        }
        return Request::None;
    }
    // 6. Context menu from the keyboard.
    if keysym == sym::MENU || (keysym == sym::F10 && mods == MOD_SHIFT) {
        let n = if f == NIL { 0 } else { f };
        let r = super::hit::drawn_rect(&w.scene, n);
        pointer::context_menu(w, s, n, r.x + r.w / 2.0, r.y + r.h / 2.0);
        return Request::None;
    }
    // 7. Shortcuts.
    if let Some(name) = shortcut_name(keysym, mods)
        && let Some(r) = receiver(&w.scene, target, ev::SHORTCUT)
    {
        s.emit(ev::SHORTCUT, w.scene.id(r).0, &words([keysym, mods]), name.as_bytes());
    }
    Request::None
}

/// Move the nearest scroll container so `n` is inside it (keyboard focus moved off screen).
pub fn scroll_into_view(w: &mut WindowCore, s: &mut Sink, n: Idx) {
    let mut p = w.scene.parent(n);
    let mut guard = 0;
    while p != NIL && guard <= crate::scene::MAX_DEPTH + 1 {
        if matches!(w.scene.kind(p), Kind::Scroll | Kind::Vlist) {
            break;
        }
        p = w.scene.parent(p);
        guard += 1;
    }
    if p == NIL {
        return;
    }
    let (view, item) = (w.scene.rect(p), w.scene.rect(n));
    let (ox, oy) = w.scene.prop(p, id::SCROLL_OFFSET).map_or((0.0, 0.0), |v| {
        (f32::from_bits(v.w[0]), f32::from_bits(v.w[1]))
    });
    // In the container's content coordinates the visible band is [view.y + oy, + view.h).
    let fit = |o: f32, start: f32, len: f32, vstart: f32, vlen: f32| {
        if start < vstart + o {
            (start - vstart).max(0.0)
        } else if start + len > vstart + o + vlen {
            (start + len - vstart - vlen).max(0.0)
        } else {
            o
        }
    };
    let next = (
        fit(ox, item.x, item.w, view.x, view.w),
        fit(oy, item.y, item.h, view.y, view.h),
    );
    if next != (ox, oy) {
        w.scene.set_prop(
            p,
            id::SCROLL_OFFSET,
            &PropValue::new(&[next.0.to_bits(), next.1.to_bits()]),
        );
        if let Some(r) = receiver(&w.scene, p, ev::SCROLL) {
            let mut head = [0u8; 8];
            head[..4].copy_from_slice(&next.0.to_le_bytes());
            head[4..].copy_from_slice(&next.1.to_le_bytes());
            s.emit(ev::SCROLL, w.scene.id(r).0, &head, &[]);
        }
    }
}
