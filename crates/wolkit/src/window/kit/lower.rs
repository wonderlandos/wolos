// SPDX-License-Identifier: BSD-2-Clause
//! Lowering: the scene to the Kit host's `shared::view` tree. This is the kit backend's "level 0":
//! the host draws its own parts, so custom styling, canvases, popovers and long lists do not
//! survive (the degradation table is in `docs/modules/libwolkit.md`). What survives: structure,
//! spacing on the Kit scale, text roles, button labels, switches, sliders, fields and icons.
//!
//! A part's wire id is its node id, so a click on the wire maps straight back to a node.

use crate::copy;
use crate::error::{Error, Result};
use crate::scene::prop::id;
use crate::scene::{Idx, Kind, NIL, SceneRead, state};
use shared::view::{IMAGE_SIZES, MAX_DEPTH, MAX_NODES, MAX_TEXT, Node, Role, SPACING};

struct Ctx<'a> {
    scene: &'a dyn SceneRead,
    count: usize,
}

fn nearest<T: Copy + Ord>(table: &[T], want: f32, as_f32: impl Fn(T) -> f32) -> T {
    let mut best = table[0];
    for t in table {
        if (as_f32(*t) - want).abs() < (as_f32(best) - want).abs() {
            best = *t;
        }
    }
    best
}

fn prop_f32(s: &dyn SceneRead, i: Idx, p: u16) -> Option<f32> {
    s.prop(i, p).map(|v| v.f32())
}

fn spacing(s: &dyn SceneRead, i: Idx, row: bool) -> u8 {
    let gap = prop_f32(s, i, if row { id::COLUMN_GAP } else { id::ROW_GAP })
        .or_else(|| prop_f32(s, i, id::GAP))
        .unwrap_or(0.0);
    nearest(&SPACING, gap, f32::from)
}

fn hidden(s: &dyn SceneRead, i: Idx) -> bool {
    s.prop(i, id::DISPLAY).is_some_and(|v| v.w[0] == 1)
        || s.prop(i, id::VISIBILITY).is_some_and(|v| v.w[0] == 1)
}

/// The text a part shows: its own, else its first descendant text.
fn label(s: &dyn SceneRead, i: Idx) -> String {
    if !s.text(i).is_empty() {
        return s.text(i).to_owned();
    }
    let mut out = String::new();
    crate::scene::walk(s, i, &mut |n, _| {
        if out.is_empty() && n != i && s.kind(n) == Kind::Text {
            out = s.text(n).to_owned();
        }
    });
    if out.is_empty() {
        out = s.a11y(i).name.to_owned();
    }
    out
}

fn role(s: &dyn SceneRead, i: Idx) -> Role {
    match s.prop(i, id::FONT_SIZE).map(|v| v.w[0]) {
        Some(0) => Role::Display,
        Some(1) => Role::Large,
        Some(2) => Role::Title,
        Some(4) => Role::Secondary,
        Some(5) => Role::Caption,
        _ => Role::Body,
    }
}

fn one_line(t: &str) -> String {
    t.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

fn check_text(t: &str) -> Result<()> {
    if t.len() > MAX_TEXT {
        return Err(Error::limit(copy::KIT_TEXT_LONG));
    }
    Ok(())
}

impl Ctx<'_> {
    fn node(&mut self, i: Idx, depth: usize) -> Result<Option<Node>> {
        let s = self.scene;
        if hidden(s, i) {
            return Ok(None);
        }
        if depth > MAX_DEPTH {
            return Err(Error::limit(copy::KIT_TOO_DEEP));
        }
        self.count += 1;
        if self.count > MAX_NODES {
            return Err(Error::limit(copy::KIT_TOO_MANY_PARTS));
        }
        let wire = s.id(i).0;
        Ok(Some(match s.kind(i) {
            Kind::Box | Kind::Scroll | Kind::Vlist => {
                let dir = s.prop(i, id::FLEX_DIRECTION).map_or(1, |v| v.w[0]);
                let row = dir == 0 || dir == 2;
                if s.first_child(i) == NIL {
                    self.count -= 1;
                    if prop_f32(s, i, id::GROW).is_some_and(|g| g > 0.0) {
                        self.count += 1;
                        return Ok(Some(Node::Spacer));
                    }
                    return Ok(None);
                }
                let mut children = Vec::new();
                let mut c = s.first_child(i);
                while c != NIL {
                    if let Some(n) = self.node(c, depth + 1)? {
                        children.push(n);
                    }
                    c = s.next_sibling(c);
                }
                if s.kind(i) == Kind::Vlist {
                    Node::List { children }
                } else if row {
                    Node::Row {
                        spacing: spacing(s, i, true),
                        children,
                    }
                } else {
                    Node::Stack {
                        spacing: spacing(s, i, false),
                        children,
                    }
                }
            }
            Kind::Text => {
                let text = one_line_keep_newlines(s.text(i));
                check_text(&text)?;
                Node::Label {
                    text,
                    role: role(s, i),
                }
            }
            Kind::Button => {
                let l = one_line(&label(s, i));
                if l.trim().is_empty() {
                    return Err(Error::invalid(copy::KIT_LABEL_NEEDED));
                }
                check_text(&l)?;
                Node::Button { id: wire, label: l }
            }
            Kind::Toggle => {
                let l = one_line(&label(s, i));
                check_text(&l)?;
                Node::Toggle {
                    id: wire,
                    on: s.state(i) & state::CHECKED != 0,
                    label: l,
                }
            }
            Kind::Slider => {
                let min = prop_f32(s, i, id::SLIDER_MIN).unwrap_or(0.0);
                let max = prop_f32(s, i, id::SLIDER_MAX).unwrap_or(1.0);
                let (min, max) = if max > min {
                    (f64::from(min), f64::from(max))
                } else {
                    (0.0, 1.0)
                };
                let value =
                    f64::from(prop_f32(s, i, id::SLIDER_VALUE).unwrap_or(0.0)).clamp(min, max);
                Node::Slider {
                    id: wire,
                    value,
                    min,
                    max,
                }
            }
            Kind::TextInput => {
                let text = one_line(s.text(i));
                let placeholder = s
                    .prop(i, id::PLACEHOLDER)
                    .map_or(String::new(), |v| one_line(s.string(v.w[0])));
                check_text(&text)?;
                check_text(&placeholder)?;
                Node::TextField {
                    id: wire,
                    text,
                    placeholder,
                }
            }
            Kind::Image => {
                let source = s
                    .prop(i, id::IMAGE_SOURCE)
                    .map_or(String::new(), |v| s.string(v.w[0]).to_owned());
                if source.is_empty() {
                    self.count -= 1;
                    return Ok(None);
                }
                let want = match s.prop(i, id::WIDTH) {
                    Some(v) if v.w[0] == crate::scene::prop::DIM_PX => f32::from_bits(v.w[1]),
                    _ => 24.0,
                };
                Node::Image {
                    source,
                    size: nearest(&IMAGE_SIZES, want, f32::from),
                }
            }
            Kind::Spacer => Node::Spacer,
            Kind::Divider => {
                self.count -= 1;
                return Ok(None);
            }
            Kind::Canvas | Kind::Surface => Node::Label {
                text: copy::KIT_NO_CANVAS.to_owned(),
                role: Role::Secondary,
            },
            Kind::Overlay => {
                self.count -= 1;
                return Ok(None);
            }
        }))
    }
}

fn one_line_keep_newlines(t: &str) -> String {
    t.chars()
        .map(|c| if c.is_control() && c != '\n' { ' ' } else { c })
        .collect()
}

/// The Kit tree for a window's scene: the root's children in a stack.
pub fn lower(scene: &dyn SceneRead) -> Result<Node> {
    let mut cx = Ctx { scene, count: 1 };
    let mut children = Vec::new();
    let mut c = scene.first_child(0);
    while c != NIL {
        if let Some(n) = cx.node(c, 2)? {
            children.push(n);
        }
        c = scene.next_sibling(c);
    }
    let root = Node::Stack {
        spacing: spacing(scene, 0, false),
        children,
    };
    root.validate().map_err(Error::invalid)?;
    Ok(root)
}

/// The scene node a wire widget id stands for.
pub fn node_of(scene: &dyn SceneRead, widget: u32) -> Option<Idx> {
    let id = crate::scene::NodeId(widget);
    let i = id.slot();
    (scene.is_live(i) && scene.id(i) == id).then_some(i)
}
