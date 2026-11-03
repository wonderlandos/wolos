// SPDX-License-Identifier: BSD-2-Clause
//! What the layout engine needs to know about a tree: links, styles and the content of leaves.
//! The engine is generic over this trait so it runs over the scene (`layout::scene`) and over a
//! hand-made tree in tests. Nodes are `u32` slots; `NIL` means none.
use crate::scene::NIL;
use crate::style::value::LayoutStyle;

pub use crate::scene::Rect;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub w: f32,
    pub h: f32,
}

impl Size {
    pub const ZERO: Size = Size { w: 0.0, h: 0.0 };
    pub const fn new(w: f32, h: f32) -> Size {
        Size { w, h }
    }
}

/// What a node is made of, as far as sizing goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Leaf {
    /// A container (or an empty box): sized by its children.
    None,
    /// Text laid out by the `Measure` hook at the available width.
    Text,
    /// A control with an intrinsic size from the hook (toggle, slider, text input).
    Control,
    /// An image: the hook gives its natural size.
    Image,
}

/// Where an overlay goes: next to an anchor node, or centred.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverlaySpec {
    /// The anchor's slot, if any.
    pub anchor: u32,
    pub side: super::place::Side,
    pub align: super::place::Align,
}

pub trait LayoutTree {
    fn first_child(&self, n: u32) -> u32;
    fn next_sibling(&self, n: u32) -> u32;
    fn parent(&self, n: u32) -> u32;
    fn style(&self, n: u32) -> &LayoutStyle;
    fn leaf(&self, n: u32) -> Leaf;
    /// Overlays leave the flow; the engine places them after everything else.
    fn overlay(&self, _n: u32) -> Option<OverlaySpec> {
        None
    }
    /// A scroll container's list of virtual rows: (row count, estimated row height, scroll offset
    /// y), if `n` is one. The children of such a node are the realised rows, and `row_index`
    /// says which row each is.
    fn vlist(&self, _n: u32) -> Option<VlistInfo> {
        None
    }
    fn row_index(&self, _n: u32) -> u32 {
        0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VlistInfo {
    pub count: u32,
    pub estimate: f32,
}

/// Content sizes the engine cannot work out itself.
pub trait Measure {
    /// The size of the text of node `n` laid out in at most `max_w` pixels wide (`None`: on one
    /// line, as long as it is). Content box: padding is the engine's.
    fn text(&mut self, n: u32, max_w: Option<f32>) -> Size;
    /// The natural size of a control or image.
    fn intrinsic(&mut self, n: u32) -> Size;
}

/// A stand-in measure: every character is `0.55 em` wide, a line is `1.3 em` high, words wrap.
/// Headless runs and tests use it until a text-backed measure is plugged in.
pub struct ApproxMeasure<'a> {
    pub text: &'a dyn Fn(u32) -> (&'a str, f32),
    pub control: Size,
}

impl Measure for ApproxMeasure<'_> {
    fn text(&mut self, n: u32, max_w: Option<f32>) -> Size {
        let (t, size) = (self.text)(n);
        approx_text(t, size, max_w)
    }
    fn intrinsic(&mut self, _n: u32) -> Size {
        self.control
    }
}

/// Width and height of `t` at `size` px wrapped at `max_w`.
pub fn approx_text(t: &str, size: f32, max_w: Option<f32>) -> Size {
    let cw = size * 0.55;
    let lh = (size * 1.3).ceil();
    if t.is_empty() {
        return Size::new(0.0, lh);
    }
    let mut lines = 0u32;
    let mut widest = 0.0f32;
    for para in t.split('\n') {
        let mut cur = 0.0f32;
        let mut first = true;
        let mut para_lines = 1u32;
        for word in para.split(' ') {
            let w = word.chars().count() as f32 * cw;
            let sp = if first { 0.0 } else { cw };
            match max_w {
                Some(m) if !first && cur + sp + w > m => {
                    widest = widest.max(cur);
                    cur = w;
                    para_lines += 1;
                }
                _ => cur += sp + w,
            }
            first = false;
        }
        widest = widest.max(cur);
        lines += para_lines;
    }
    Size::new(widest, lines as f32 * lh)
}

/// A node slot that is not there.
pub const NONE: u32 = NIL;
