// SPDX-License-Identifier: BSD-2-Clause
//! Keyboard focus (plan section 2.7): tab order is tree order with positive `tab-index` values
//! first (ascending) and negative ones left out; arrow keys move between the items of a list;
//! an open modal layer traps focus and gives it back when it closes; the ring (`:focus-visible`)
//! shows only when the keyboard moved the focus.

use super::InputState;
use super::hit::{Layers, scene_is_inside, shown};
use crate::event::kind as ev;
use crate::scene::prop::id;
use crate::scene::{Idx, Kind, NIL, SceneRead, SceneWrite, state, walk};
use crate::window::route::{disabled, receiver};
use crate::window::{Sink, WindowCore};

/// A11y roles (`WOLKIT_ROLE_*`) whose children are items the arrow keys move between.
const LIST_ROLES: &[u32] = &[9, 13, 15];

pub fn focusable(scene: &dyn SceneRead, n: Idx) -> bool {
    if n == NIL || n == 0 || !scene.is_live(n) {
        return false;
    }
    let explicit = scene.prop(n, id::FOCUSABLE).map(|v| v.w[0] == 1);
    explicit.unwrap_or_else(|| scene.kind(n).focusable()) && !disabled(scene, n) && shown(scene, n)
}

/// The node itself or the nearest ancestor that takes focus.
pub fn nearest_focusable(scene: &dyn SceneRead, mut n: Idx) -> Idx {
    let mut guard = 0;
    while n != NIL && guard <= crate::scene::MAX_DEPTH + 1 {
        if focusable(scene, n) {
            return n;
        }
        n = scene.parent(n);
        guard += 1;
    }
    NIL
}

fn tab_index(scene: &dyn SceneRead, n: Idx) -> i32 {
    scene
        .prop(n, id::TAB_INDEX)
        .map_or(0, |v| v.w[0].cast_signed())
}

/// The next node in tab order inside `scope` (the window root or a modal layer), wrapping.
pub fn next_in_order(scene: &dyn SceneRead, scope: Idx, from: Idx, backwards: bool) -> Idx {
    let mut order: Vec<(i32, Idx)> = Vec::new();
    walk(scene, scope, &mut |i, _| {
        if focusable(scene, i) {
            let t = tab_index(scene, i);
            if t >= 0 {
                order.push((if t > 0 { t } else { i32::MAX }, i));
            }
        }
    });
    // Stable: tree order among equal tab indexes.
    order.sort_by_key(|(t, _)| *t);
    if order.is_empty() {
        return NIL;
    }
    let n = order.len();
    let k = match (order.iter().position(|(_, i)| *i == from), backwards) {
        (Some(a), false) => (a + 1) % n,
        (Some(a), true) => (a + n - 1) % n,
        (None, false) => 0,
        (None, true) => n - 1,
    };
    order[k].1
}

/// The first focusable node in tab order inside `scope`, or NIL.
pub fn first_in(scene: &dyn SceneRead, scope: Idx) -> Idx {
    next_in_order(scene, scope, NIL, false)
}

/// The scope tab order and arrows stay in: the innermost open modal layer, else the window.
pub fn scope(layers: &Layers) -> Idx {
    let m = layers.top_modal();
    if m == NIL { 0 } else { m }
}

/// The nearest list container above `n` (a `vlist` or a node with a list, tab-list or menu role)
/// and the direct child of it that holds `n`.
fn list_of(scene: &dyn SceneRead, n: Idx) -> Option<(Idx, Idx)> {
    let mut item = n;
    let mut p = scene.parent(n);
    let mut guard = 0;
    while p != NIL && guard <= crate::scene::MAX_DEPTH + 1 {
        if scene.kind(p) == Kind::Vlist || LIST_ROLES.contains(&scene.a11y(p).role) {
            return Some((p, item));
        }
        item = p;
        p = scene.parent(p);
        guard += 1;
    }
    None
}

/// Whether a list lays its items out in a row (Left and Right move) or a column (Up and Down).
pub fn list_is_row(scene: &dyn SceneRead, list: Idx) -> bool {
    match scene.prop(list, id::FLEX_DIRECTION) {
        Some(v) => v.w[0] == 0 || v.w[0] == 2,
        // A tab list is a row by default, other lists a column.
        None => scene.a11y(list).role == 13,
    }
}

/// Which way an arrow, Home or End key moves inside a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    Prev,
    Next,
    First,
    Last,
}

/// The node the focus moves to inside its list, or NIL when `from` is not in a list (or the move
/// leaves it). Items with nothing focusable are skipped.
pub fn in_list(scene: &dyn SceneRead, from: Idx, m: Move) -> Idx {
    let Some((list, item)) = list_of(scene, from) else {
        return NIL;
    };
    let target = |c: Idx| {
        if focusable(scene, c) {
            c
        } else {
            first_in(scene, c)
        }
    };
    let (mut c, step): (Idx, fn(&dyn SceneRead, Idx) -> Idx) = match m {
        Move::Prev => (scene.prev_sibling(item), |s, i| s.prev_sibling(i)),
        Move::Next => (scene.next_sibling(item), |s, i| s.next_sibling(i)),
        Move::First => (scene.first_child(list), |s, i| s.next_sibling(i)),
        Move::Last => (scene.last_child(list), |s, i| s.prev_sibling(i)),
    };
    while c != NIL {
        let t = target(c);
        if t != NIL {
            return t;
        }
        c = step(scene, c);
    }
    NIL
}

/// Move the focus to `to` (NIL clears it) and tell the old and the new receiver. `visible` puts
/// the ring on (keyboard) or off (pointer, a click).
pub fn set_focus(w: &mut WindowCore, s: &mut Sink, st: &mut InputState, to: Idx, visible: bool) {
    let old = st.focused;
    st.keyboard_mode = visible;
    if old == to && w.scene.focus() == to {
        if to != NIL {
            w.scene.set_engine_state(to, state::FOCUS_VISIBLE, visible);
        }
        return;
    }
    w.scene.focus = to;
    st.focused = to;
    st.focused_id = w.scene.id(to);
    if old != NIL && w.scene.is_live(old) {
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

/// Bring focus in line with the scene after a patch: a `FOCUS` op the app sent gets its state
/// bits and events, a focused node that went away, got disabled or hidden loses focus, a modal
/// layer that opened takes it (the old focus is remembered) and one that closed gives it back.
/// Returns whether the focus moved.
pub fn sync(w: &mut WindowCore, s: &mut Sink, st: &mut InputState) -> bool {
    st.layers.refresh(&w.scene);
    let before = st.focused;
    // A slot freed and reused within one patch is another node: the old focus is gone.
    if st.focused != NIL
        && (!w.scene.is_live(st.focused) || w.scene.id(st.focused) != st.focused_id)
    {
        st.focused = NIL;
    }
    // 1. The app moved focus with a FOCUS op (the scene's focus differs from ours).
    let wanted = w.scene.focus();
    if wanted != st.focused {
        // Restore the old one so `set_focus` clears its bits and reports the change.
        w.scene.focus = st.focused;
        let ring = st.keyboard_mode;
        set_focus(w, s, st, wanted, ring);
    }
    // 2. A focused node that can no longer be focused lets go.
    if st.focused != NIL && !focusable(&w.scene, st.focused) {
        let ring = st.keyboard_mode;
        set_focus(w, s, st, NIL, ring);
    }
    // 3. Modal layers.
    let modal = st.layers.top_modal();
    if modal != st.modal {
        let opened = modal != NIL && (st.modal == NIL || scene_is_inside(&w.scene, st.modal, modal));
        if opened {
            if st.focus_return.len() >= super::MAX_FOCUS_RETURN {
                st.focus_return.remove(0);
            }
            let f = st.focused;
            st.focus_return.push((f, w.scene.id(f)));
        }
        st.modal = modal;
        if opened {
            if st.focused == NIL || !scene_is_inside(&w.scene, modal, st.focused) {
                let to = first_in(&w.scene, modal);
                let ring = st.keyboard_mode;
                set_focus(w, s, st, to, ring);
            }
        } else {
            // Closed, or replaced by an outer one: focus goes back to the newest entry that is
            // still there and inside the scope now in force.
            let scope = if modal == NIL { 0 } else { modal };
            let mut to = NIL;
            while let Some((i, id)) = st.focus_return.pop() {
                let back = i != NIL && w.scene.is_live(i) && w.scene.id(i) == id;
                if back && focusable(&w.scene, i) && scene_is_inside(&w.scene, scope, i) {
                    to = i;
                    break;
                }
            }
            let outside = st.focused == NIL || !scene_is_inside(&w.scene, scope, st.focused);
            if to == NIL && outside && modal != NIL {
                to = first_in(&w.scene, modal);
            }
            if to != NIL {
                let ring = st.keyboard_mode;
                set_focus(w, s, st, to, ring);
            }
        }
    }
    st.focused != before
}
