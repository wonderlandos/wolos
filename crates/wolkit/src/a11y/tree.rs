// SPDX-License-Identifier: BSD-2-Clause
//! The accessibility tree of one window (plan section 2.7): built from the scene in one walk, so
//! it is always cheap to have; exported over the accessibility bus only while an assistive
//! technology listens (`a11y::bus`).
//!
//! Per node: role, name, description, states (disabled, focused, selected, checked, expanded,
//! busy and the ones they imply), bounds in window coordinates, the default action, the text of
//! a text field with its caret and selection (in characters, as the bus counts), and a slider's
//! value. Hidden parts are left out, and so is the label inside a button (it is the button's
//! name, read once).
//!
//! [`dump`] is the text form the goldens compare (`tests/a11y_tree.rs`).

use super::roles::{self, st, wk};
use crate::input::edit::Field;
use crate::input::focus::focusable;
use crate::input::hit::{drawn_rect, shown};
use crate::scene::prop::id;
use crate::scene::{Idx, Kind, NIL, Rect, SceneRead, state};
use crate::window::route::disabled;
use std::fmt::Write;

/// Nodes in one exported tree (the scene's limit).
pub const MAX_NODES: usize = crate::scene::MAX_NODES;
/// Longest name or description exported (characters beyond are cut at a boundary).
pub const MAX_LABEL: usize = 1024;

/// A text field's content as the bus sees it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextInfo {
    /// The text (bullets for a secure field).
    pub text: String,
    /// Caret and selection in characters.
    pub caret: usize,
    pub selection: (usize, usize),
}

/// A slider's value.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ValueInfo {
    pub min: f32,
    pub max: f32,
    pub current: f32,
    pub step: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct A11yNode {
    /// The scene node id (`NodeId.0`); 0 is the window.
    pub id: u32,
    pub slot: Idx,
    /// Index of the parent in [`Tree::nodes`]; `usize::MAX` for the window.
    pub parent: usize,
    pub children: Vec<usize>,
    /// `WOLKIT_ROLE_*`, after the kind's default.
    pub role: u32,
    /// The bus role number.
    pub wire_role: u32,
    pub name: String,
    pub description: String,
    /// Bit `n` set for bus state `n`.
    pub states: u64,
    pub bounds: Rect,
    /// The default action's name (`click`, `toggle`), if the node has one.
    pub action: Option<&'static str>,
    pub text: Option<TextInfo>,
    pub value: Option<ValueInfo>,
}

impl A11yNode {
    pub fn has(&self, state_bit: u32) -> bool {
        self.states & (1u64 << state_bit) != 0
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tree {
    pub window: u32,
    /// `nodes[0]` is the window (a frame named by the title).
    pub nodes: Vec<A11yNode>,
}

/// What the tree needs besides the scene.
pub struct Context<'a> {
    pub window: u32,
    pub title: &'a str,
    /// The window has keyboard focus (the frame is active).
    pub active: bool,
    pub resizable: bool,
    /// The focused field's editor, for caret and selection.
    pub field: Option<&'a Field>,
}

fn cut(s: &str) -> String {
    match s.char_indices().nth(MAX_LABEL) {
        Some((at, _)) => s[..at].to_owned(),
        None => s.to_owned(),
    }
}

fn flag(scene: &dyn SceneRead, n: Idx, p: u16) -> bool {
    scene.prop(n, p).is_some_and(|v| v.w[0] == 1)
}

fn prop_str(scene: &dyn SceneRead, n: Idx, p: u16) -> &str {
    scene.prop(n, p).map_or("", |v| scene.string(v.w[0]))
}

/// The first non-empty text below `n` (depth first, bounded), for a button's name.
fn inner_text(scene: &dyn SceneRead, n: Idx) -> Option<&str> {
    let mut found = None;
    let mut seen = 0usize;
    crate::scene::walk(scene, n, &mut |i, _| {
        seen += 1;
        if found.is_none() && i != n && seen < 256 && scene.kind(i) == Kind::Text && shown(scene, i)
        {
            let t = scene.text(i).trim();
            if !t.is_empty() {
                found = Some(i);
            }
        }
    });
    found.map(|i| scene.text(i).trim())
}

/// Name and whether it came from the content (then the content is not exported again).
fn name_of(scene: &dyn SceneRead, n: Idx, role: u32) -> (String, bool) {
    let a = scene.a11y(n);
    if !a.name.is_empty() {
        return (cut(a.name), false);
    }
    let own = scene.text(n).trim();
    match scene.kind(n) {
        Kind::Text => return (cut(own), false),
        Kind::TextInput => return (cut(prop_str(scene, n, id::PLACEHOLDER)), false),
        _ => {}
    }
    if roles::named_by_content(role) || scene.kind(n) == Kind::Button {
        if !own.is_empty() {
            return (cut(own), false);
        }
        if let Some(t) = inner_text(scene, n) {
            return (cut(t), true);
        }
    }
    (cut(prop_str(scene, n, id::TOOLTIP)), false)
}

fn chars_before(s: &str, byte: usize) -> usize {
    let mut b = byte.min(s.len());
    while !s.is_char_boundary(b) {
        b -= 1;
    }
    s[..b].chars().count()
}

fn text_info(scene: &dyn SceneRead, n: Idx, field: Option<&Field>) -> TextInfo {
    let secure = flag(scene, n, id::SECURE);
    let (text, caret, sel) = match field.filter(|f| f.node() == n) {
        Some(f) => {
            let t = f.editor.text();
            let s = f.editor.selection();
            let (c, a, b) = (
                chars_before(&t, s.head),
                chars_before(&t, s.start()),
                chars_before(&t, s.end()),
            );
            (t, c, (a, b))
        }
        None => {
            let t = scene.text(n).to_owned();
            let c = t.chars().count();
            (t, c, (c, c))
        }
    };
    let text = if secure {
        "\u{2022}".repeat(text.chars().count())
    } else {
        text
    };
    TextInfo {
        text,
        caret,
        selection: sel,
    }
}

fn states_of(scene: &dyn SceneRead, n: Idx, role: u32, kind: Kind) -> u64 {
    let mut s = 0u64;
    let mut on = |b: u32| s |= 1u64 << b;
    on(st::VISIBLE);
    on(st::SHOWING);
    let bits = scene.state(n);
    if !disabled(scene, n) {
        on(st::ENABLED);
        on(st::SENSITIVE);
    }
    if focusable(scene, n) {
        on(st::FOCUSABLE);
    }
    if scene.focus() == n && bits & state::FOCUS != 0 {
        on(st::FOCUSED);
    }
    let checkable = matches!(role, wk::TOGGLE | wk::CHECKBOX) || kind == Kind::Toggle;
    if checkable {
        on(st::CHECKABLE);
    }
    if bits & state::CHECKED != 0 {
        on(st::CHECKED);
        if role == wk::TOGGLE {
            on(st::PRESSED);
        }
    }
    if matches!(role, wk::LISTITEM | wk::TAB | wk::MENUITEM) {
        on(st::SELECTABLE);
    }
    if bits & state::SELECTED != 0 {
        on(st::SELECTED);
    }
    if bits & state::EXPANDED != 0 {
        on(st::EXPANDABLE);
        on(st::EXPANDED);
    }
    if bits & state::BUSY != 0 {
        on(st::BUSY);
    }
    if kind == Kind::TextInput {
        on(st::SINGLE_LINE);
        on(st::SELECTABLE_TEXT);
        if flag(scene, n, id::READONLY) {
            on(st::READ_ONLY);
        } else {
            on(st::EDITABLE);
        }
    }
    if kind == Kind::Slider {
        on(st::HORIZONTAL);
    }
    if flag(scene, n, id::MODAL) {
        on(st::MODAL);
    }
    s
}

fn action_of(scene: &dyn SceneRead, n: Idx, kind: Kind, role: u32) -> Option<&'static str> {
    if disabled(scene, n) {
        return None;
    }
    let clicks = crate::window::route::receiver(scene, n, crate::event::kind::CLICK) == Some(n);
    match (kind, role) {
        (Kind::Toggle, _) | (_, wk::TOGGLE | wk::CHECKBOX) => Some("toggle"),
        (Kind::Button, _) => Some("click"),
        _ if clicks => Some("click"),
        _ => None,
    }
}

/// Build the tree of the window.
pub fn build(scene: &dyn SceneRead, cx: &Context) -> Tree {
    let mut t = Tree {
        window: cx.window,
        nodes: Vec::new(),
    };
    let mut frame_states = (1u64 << st::VISIBLE) | (1u64 << st::SHOWING) | (1u64 << st::ENABLED);
    frame_states |= 1u64 << st::SENSITIVE;
    if cx.active {
        frame_states |= 1u64 << st::ACTIVE;
    }
    if cx.resizable {
        frame_states |= 1u64 << st::RESIZABLE;
    }
    t.nodes.push(A11yNode {
        id: 0,
        slot: 0,
        parent: usize::MAX,
        children: Vec::new(),
        role: wk::NONE,
        wire_role: roles::role::FRAME,
        name: cut(cx.title),
        description: String::new(),
        states: frame_states,
        bounds: scene.rect(0),
        action: None,
        text: None,
        value: None,
    });
    if !scene.is_live(0) {
        return t;
    }
    let mut c = scene.first_child(0);
    while c != NIL {
        add(scene, cx, &mut t, c, 0, 1);
        c = scene.next_sibling(c);
    }
    t
}

fn add(scene: &dyn SceneRead, cx: &Context, t: &mut Tree, n: Idx, parent: usize, depth: usize) {
    if depth > crate::scene::MAX_DEPTH + 1 || t.nodes.len() >= MAX_NODES {
        return;
    }
    let hidden = |p: u16| flag(scene, n, p);
    if hidden(id::VISIBILITY) || hidden(id::DISPLAY) {
        return;
    }
    let kind = scene.kind(n);
    let role = roles::engine_role(kind, scene.a11y(n).role);
    let (name, from_content) = name_of(scene, n, role);
    let a = scene.a11y(n);
    let description = if !a.description.is_empty() {
        cut(a.description)
    } else if name != prop_str(scene, n, id::TOOLTIP) {
        cut(prop_str(scene, n, id::TOOLTIP))
    } else {
        String::new()
    };
    let value = (kind == Kind::Slider).then(|| {
        let g = |p: u16, d: f32| scene.prop(n, p).map_or(d, |v| v.f32());
        ValueInfo {
            min: g(id::SLIDER_MIN, 0.0),
            max: g(id::SLIDER_MAX, 1.0),
            current: g(id::SLIDER_VALUE, g(id::SLIDER_MIN, 0.0)),
            step: g(id::SLIDER_STEP, 0.0),
        }
    });
    let me = t.nodes.len();
    t.nodes.push(A11yNode {
        id: scene.id(n).0,
        slot: n,
        parent,
        children: Vec::new(),
        role,
        wire_role: roles::wire_role(kind, role, flag(scene, n, id::SECURE)),
        name,
        description,
        states: states_of(scene, n, role, kind),
        bounds: drawn_rect(scene, n),
        action: action_of(scene, n, kind, role),
        text: (kind == Kind::TextInput).then(|| text_info(scene, n, cx.field)),
        value,
    });
    t.nodes[parent].children.push(me);
    // A control named by its label does not export the label again.
    if from_content {
        return;
    }
    let mut c = scene.first_child(n);
    while c != NIL {
        add(scene, cx, t, c, me, depth + 1);
        c = scene.next_sibling(c);
    }
}

/// Interactive parts with no name: the node id and one sentence each. The native backend reports
/// them as warnings; a headless test run treats them as errors (plan section 2.7).
pub fn lint(t: &Tree) -> Vec<(u32, String)> {
    t.nodes
        .iter()
        .skip(1)
        .filter(|n| (n.has(st::FOCUSABLE) || n.action.is_some()) && n.name.trim().is_empty())
        .map(|n| {
            let role = roles::role_name(n.wire_role);
            (
                n.id,
                format!(
                    "Part {} ({role}) has no accessible name: give it text, a text child or a name with SET_A11Y.",
                    n.id
                ),
            )
        })
        .collect()
}

/// The text form of the tree, one line per node indented by depth:
/// `id role "name" [states] x,y wxh` then `description=`, `action=`, `text=` with `caret=` and
/// `selection=`, `value=` as they apply.
pub fn dump(t: &Tree) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "a11y window={} nodes={}", t.window, t.nodes.len());
    let mut stack = vec![(0usize, 0usize)];
    while let Some((i, depth)) = stack.pop() {
        let n = &t.nodes[i];
        let states: Vec<&str> = st::NAMES
            .iter()
            .filter(|(b, _)| n.has(*b))
            .map(|(_, s)| *s)
            .collect();
        let b = n.bounds;
        let _ = write!(
            out,
            "{}{} {} {:?} [{}] {},{} {}x{}",
            "  ".repeat(depth),
            n.id,
            roles::role_name(n.wire_role).replace(' ', "-"),
            n.name,
            states.join(","),
            b.x,
            b.y,
            b.w,
            b.h
        );
        if !n.description.is_empty() {
            let _ = write!(out, " description={:?}", n.description);
        }
        if let Some(a) = n.action {
            let _ = write!(out, " action={a}");
        }
        if let Some(x) = &n.text {
            let _ = write!(
                out,
                " text={:?} caret={} selection={}..{}",
                x.text, x.caret, x.selection.0, x.selection.1
            );
        }
        if let Some(v) = n.value {
            let _ = write!(
                out,
                " value={} min={} max={} step={}",
                v.current, v.min, v.max, v.step
            );
        }
        out.push('\n');
        for c in n.children.iter().rev() {
            stack.push((*c, depth + 1));
        }
    }
    out
}
