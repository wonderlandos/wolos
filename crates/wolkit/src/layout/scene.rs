// SPDX-License-Identifier: BSD-2-Clause
//! The scene as a layout tree: links from the arena, styles from the computed-style store, leaf
//! content from a [`Measure`]. [`run`] is what the window pipeline calls each frame.
use super::engine::{LayoutCache, layout};
use super::place::{Align, Side};
use super::tree::*;
use crate::scene::prop::id;
use crate::scene::{Idx, Kind, NIL, NodeId, SceneRead, SceneWrite, dirty};
use crate::style::Styles;
use crate::style::value::LayoutStyle;

pub struct SceneTree<'a> {
    pub scene: &'a dyn SceneRead,
    pub styles: &'a Styles,
}

impl LayoutTree for SceneTree<'_> {
    fn first_child(&self, n: u32) -> u32 {
        self.scene.first_child(n)
    }
    fn next_sibling(&self, n: u32) -> u32 {
        self.scene.next_sibling(n)
    }
    fn parent(&self, n: u32) -> u32 {
        self.scene.parent(n)
    }
    fn style(&self, n: u32) -> &LayoutStyle {
        &self.styles.get(self.scene.computed(n)).layout
    }
    fn leaf(&self, n: u32) -> Leaf {
        let s = self.scene;
        match s.kind(n) {
            Kind::Text => Leaf::Text,
            Kind::Image => Leaf::Image,
            Kind::Slider | Kind::Toggle | Kind::TextInput => Leaf::Control,
            // A button or any box that carries its own label and has no children is a text leaf.
            Kind::Button | Kind::Box if s.first_child(n) == NIL && !s.text(n).is_empty() => Leaf::Text,
            _ => Leaf::None,
        }
    }
    fn overlay(&self, n: u32) -> Option<OverlaySpec> {
        if self.scene.kind(n) != Kind::Overlay {
            return None;
        }
        let anchor = self
            .scene
            .prop(n, id::ANCHOR)
            .map(|v| NodeId(v.w[0]))
            .filter(|a| a.slot() < self.scene.capacity() && self.scene.is_live(a.slot()) && self.scene.id(a.slot()) == *a)
            .map_or(NIL, |a| a.slot());
        let side = self.scene.prop(n, id::ANCHOR_SIDE).map_or(Side::Bottom, |v| Side::from_wire(v.w[0]));
        let align = self.scene.prop(n, id::ANCHOR_ALIGN).map_or(Align::Start, |v| Align::from_wire(v.w[0]));
        Some(OverlaySpec { anchor, side, align })
    }
    fn vlist(&self, n: u32) -> Option<VlistInfo> {
        if self.scene.kind(n) != Kind::Vlist {
            return None;
        }
        let count = self.scene.prop(n, id::LIST_COUNT).map_or(0, |v| v.w[0]);
        let est = self.scene.prop(n, id::LIST_HEIGHT).map_or(0.0, |v| v.f32());
        Some(VlistInfo { count, estimate: if est > 0.0 { est } else { 32.0 } })
    }
    fn row_index(&self, n: u32) -> u32 {
        self.scene.key(n) as u32
    }
}

/// A measure that needs no font: 0.55 em per character, 1.3 em per line. The native backend plugs
/// in the shaper's measure instead.
pub struct SceneMeasure<'a> {
    pub scene: &'a dyn SceneRead,
    pub styles: &'a Styles,
}

impl Measure for SceneMeasure<'_> {
    fn text(&mut self, n: u32, max_w: Option<f32>) -> Size {
        let t = &self.styles.get(self.scene.computed(n)).text;
        let mut s = approx_text(self.scene.text(n), t.size, max_w);
        if t.line_height > 0.0 {
            s.h = s.h / (t.size * 1.3).ceil() * (t.size * t.line_height);
        }
        s
    }
    fn intrinsic(&mut self, n: u32) -> Size {
        use shared::theme::scaffold::CONTROL_HEIGHT;
        let h = CONTROL_HEIGHT as f32;
        match self.scene.kind(n) {
            Kind::Toggle => Size::new(40.0, h),
            Kind::Slider => Size::new(160.0, h),
            Kind::TextInput => {
                let t = &self.styles.get(self.scene.computed(n)).text;
                Size::new(160.0, (t.size * 1.3).ceil().max(h))
            }
            Kind::Image => Size::new(64.0, 64.0),
            _ => Size::ZERO,
        }
    }
}

fn collect(scene: &dyn SceneRead, n: Idx, out: &mut Vec<(Idx, u8)>) {
    let d = scene.dirty(n);
    if d & (dirty::LAYOUT | dirty::CHILDREN) != 0 {
        out.push((n, d & (dirty::LAYOUT | dirty::CHILDREN)));
    }
    if d & dirty::SUBTREE != 0 || d & (dirty::LAYOUT | dirty::CHILDREN) != 0 {
        let mut c = scene.first_child(n);
        while c != NIL {
            collect(scene, c, out);
            c = scene.next_sibling(c);
        }
    }
}

/// Lay out `scene` for a viewport of `width` x `height` logical pixels and store each node's
/// rectangle in window space (before scroll offsets) with `set_rect`. Only what is dirty, and what
/// depends on it, is laid out again.
pub fn run(
    scene: &mut dyn SceneWrite,
    styles: &Styles,
    cache: &mut LayoutCache,
    measure: Option<&mut dyn Measure>,
    width: f32,
    height: f32,
) {
    if !scene.is_live(0) {
        return;
    }
    let mut dirty_nodes = Vec::new();
    collect(scene, 0, &mut dirty_nodes);
    let mut changed: Vec<(Idx, Rect)> = Vec::new();
    {
        let rs: &dyn SceneRead = &*scene;
        let tree = SceneTree { scene: rs, styles };
        for (n, bits) in &dirty_nodes {
            if bits & dirty::LAYOUT != 0 {
                cache.invalidate_own(&tree, *n);
            }
            if bits & dirty::CHILDREN != 0 {
                cache.invalidate_content(&tree, *n);
            }
        }
        let mut default = SceneMeasure { scene: rs, styles };
        let m: &mut dyn Measure = match measure {
            Some(m) => m,
            None => &mut default,
        };
        let mut rep = |n: u32, r: Rect| changed.push((n, r));
        layout(&tree, &mut DynMeasure(m), cache, 0, Size::new(width, height), &mut rep);
    }
    for (n, r) in changed {
        if scene.is_live(n) && scene.rect(n) != r {
            scene.set_rect(n, r);
            scene.mark_dirty(n, dirty::PAINT);
        }
    }
    for (n, _) in dirty_nodes {
        scene.clear_dirty(n, dirty::LAYOUT | dirty::CHILDREN);
    }
}

struct DynMeasure<'a>(&'a mut dyn Measure);

impl Measure for DynMeasure<'_> {
    fn text(&mut self, n: u32, max_w: Option<f32>) -> Size {
        self.0.text(n, max_w)
    }
    fn intrinsic(&mut self, n: u32) -> Size {
        self.0.intrinsic(n)
    }
}
