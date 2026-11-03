// SPDX-License-Identifier: BSD-2-Clause
//! The scene as a layout tree: links from the arena, styles from the computed-style store, leaf
//! content from a [`Measure`]. [`run`] is what the window pipeline calls each frame.
use super::engine::{LayoutCache, layout};
use super::place::{Align, Side};
use super::tree::{
    LayoutTree, Leaf, Measure, NodeInfo, OverlaySpec, Rect, Size, VlistInfo, approx_text,
};
use crate::scene::ids::to_u32;
use crate::scene::prop::id;
use crate::scene::{Idx, Kind, NIL, NodeId, SceneRead, SceneWrite, dirty};
use crate::style::Styles;
use crate::style::value::LayoutStyle;

/// The scene as a layout tree. Generic over the scene type so a concrete `Scene` is inlined; a
/// `&dyn SceneRead` works too.
pub struct SceneTree<'a, S: SceneRead + ?Sized = dyn SceneRead + 'a> {
    pub scene: &'a S,
    pub styles: &'a Styles,
}

impl<S: SceneRead + ?Sized> LayoutTree for SceneTree<'_, S> {
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
            Kind::Button | Kind::Box if s.first_child(n) == NIL && !s.text(n).is_empty() => {
                Leaf::Text
            }
            _ => Leaf::None,
        }
    }
    fn info(&self, n: u32) -> NodeInfo<'_> {
        let s = self.scene;
        let kind = s.kind(n);
        let leaf = match kind {
            Kind::Text => Leaf::Text,
            Kind::Image => Leaf::Image,
            Kind::Slider | Kind::Toggle | Kind::TextInput => Leaf::Control,
            Kind::Button | Kind::Box if s.first_child(n) == NIL && !s.text(n).is_empty() => {
                Leaf::Text
            }
            _ => Leaf::None,
        };
        NodeInfo {
            style: &self.styles.get(s.computed(n)).layout,
            leaf,
            overlay: kind == Kind::Overlay,
            vlist: kind == Kind::Vlist,
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
            .filter(|a| {
                a.slot() < self.scene.capacity()
                    && self.scene.is_live(a.slot())
                    && self.scene.id(a.slot()) == *a
            })
            .map_or(NIL, |a| a.slot());
        let side = self
            .scene
            .prop(n, id::ANCHOR_SIDE)
            .map_or(Side::Bottom, |v| Side::from_wire(v.w[0]));
        let align = self
            .scene
            .prop(n, id::ANCHOR_ALIGN)
            .map_or(Align::Start, |v| Align::from_wire(v.w[0]));
        Some(OverlaySpec {
            anchor,
            side,
            align,
        })
    }
    fn vlist(&self, n: u32) -> Option<VlistInfo> {
        if self.scene.kind(n) != Kind::Vlist {
            return None;
        }
        let count = self.scene.prop(n, id::LIST_COUNT).map_or(0, |v| v.w[0]);
        let est = self.scene.prop(n, id::LIST_HEIGHT).map_or(0.0, |v| v.f32());
        Some(VlistInfo {
            count,
            estimate: if est > 0.0 { est } else { 32.0 },
        })
    }
    fn row_index(&self, n: u32) -> u32 {
        to_u32(self.scene.key(n))
    }
}

/// The natural size of a control or image (text inputs are one line of their text style).
fn intrinsic_size<S: SceneRead + ?Sized>(scene: &S, n: u32, line: f32) -> Size {
    use shared::theme::scaffold::CONTROL_HEIGHT;
    let h = CONTROL_HEIGHT as f32;
    match scene.kind(n) {
        Kind::Toggle => Size::new(40.0, h),
        Kind::Slider => Size::new(160.0, h),
        Kind::TextInput => Size::new(160.0, line.max(h)),
        Kind::Image => Size::new(64.0, 64.0),
        _ => Size::ZERO,
    }
}

/// A measure that needs no font: 0.5 em per character, 1.3 em per line. What [`run`] uses when
/// it is given no measure (layout tests and benches, where text must not depend on fonts).
pub struct SceneMeasure<'a, S: SceneRead + ?Sized = dyn SceneRead + 'a> {
    pub scene: &'a S,
    pub styles: &'a Styles,
}

impl<S: SceneRead + ?Sized> Measure for SceneMeasure<'_, S> {
    fn text(&mut self, n: u32, max_w: Option<f32>) -> Size {
        let t = &self.styles.get(self.scene.computed(n)).text;
        let mut s = approx_text(self.scene.text(n), t.size, max_w);
        if t.line_height > 0.0 {
            s.h = s.h / (t.size * 1.3).ceil() * (t.size * t.line_height);
        }
        s
    }
    fn intrinsic(&mut self, n: u32) -> Size {
        let t = &self.styles.get(self.scene.computed(n)).text;
        intrinsic_size(self.scene, n, (t.size * 1.3).ceil())
    }
}

/// The measure windows use: text shaped with the real fonts, through the window's paragraph
/// cache (plan section 2.4: keyed by text, width constraint and style), so paint reuses what
/// layout shaped.
pub struct TextMeasure<'a, S: SceneRead + ?Sized = dyn SceneRead + 'a> {
    pub scene: &'a S,
    pub styles: &'a Styles,
    pub text: &'a mut crate::text::system::TextSystem,
    pub cache: &'a mut crate::text::measure::TextCache,
}

impl<S: SceneRead + ?Sized> Measure for TextMeasure<'_, S> {
    fn text(&mut self, n: u32, max_w: Option<f32>) -> Size {
        let t = &self.styles.get(self.scene.computed(n)).text;
        let (w, h) = self.cache.measure(self.text, self.scene.text(n), t, max_w);
        Size::new(w, h)
    }
    fn intrinsic(&mut self, n: u32) -> Size {
        let t = &self.styles.get(self.scene.computed(n)).text;
        let (_, line) = self.cache.measure(self.text, "", t, None);
        intrinsic_size(self.scene, n, line)
    }
}

fn collect<S: SceneRead + ?Sized>(scene: &S, n: Idx, out: &mut Vec<(Idx, u8)>) {
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
pub fn run<S: SceneWrite + ?Sized>(
    scene: &mut S,
    styles: &Styles,
    cache: &mut LayoutCache,
    measure: Option<&mut dyn Measure>,
    width: f32,
    height: f32,
) {
    let with = match measure {
        Some(m) => With::Custom(m),
        None => With::Approx,
    };
    run_with(scene, styles, cache, with, width, height);
}

/// Like [`run`], measuring text with the real fonts through the window's paragraph cache
/// ([`TextMeasure`]). What windows do every frame.
pub fn run_text<S: SceneWrite + ?Sized>(
    scene: &mut S,
    styles: &Styles,
    cache: &mut LayoutCache,
    text: (&mut crate::text::system::TextSystem, &mut crate::text::measure::TextCache),
    width: f32,
    height: f32,
) {
    let with = With::Text(text.0, text.1);
    run_with(scene, styles, cache, with, width, height);
}

/// Where the measure comes from: the scene is borrowed mutably by `run`, so a measure that reads
/// it is built inside.
enum With<'m> {
    Approx,
    Custom(&'m mut dyn Measure),
    Text(
        &'m mut crate::text::system::TextSystem,
        &'m mut crate::text::measure::TextCache,
    ),
}

fn run_with<S: SceneWrite + ?Sized>(
    scene: &mut S,
    styles: &Styles,
    cache: &mut LayoutCache,
    with: With<'_>,
    width: f32,
    height: f32,
) {
    if !scene.is_live(0) {
        return;
    }
    let mut dirty_nodes = std::mem::take(&mut cache.dirty_scratch);
    let mut changed = std::mem::take(&mut cache.changed_scratch);
    dirty_nodes.clear();
    changed.clear();
    collect(scene, 0, &mut dirty_nodes);
    {
        let rs: &S = &*scene;
        let tree = SceneTree { scene: rs, styles };
        for (n, bits) in &dirty_nodes {
            if bits & dirty::LAYOUT != 0 {
                cache.invalidate_own(&tree, *n);
            }
            if bits & dirty::CHILDREN != 0 {
                cache.invalidate_content(&tree, *n);
            }
        }
        let mut approx;
        let mut shaped;
        let m: &mut dyn Measure = match with {
            With::Custom(m) => m,
            With::Approx => {
                approx = SceneMeasure { scene: rs, styles };
                &mut approx
            }
            With::Text(text, tcache) => {
                shaped = TextMeasure {
                    scene: rs,
                    styles,
                    text,
                    cache: tcache,
                };
                &mut shaped
            }
        };
        let mut rep = |n: u32, r: Rect| changed.push((n, r));
        layout(
            &tree,
            &mut DynMeasure(m),
            cache,
            0,
            Size::new(width, height),
            &mut rep,
        );
    }
    for (n, r) in changed.drain(..) {
        if scene.is_live(n) && scene.rect(n) != r {
            scene.set_rect(n, r);
            scene.mark_dirty(n, dirty::PAINT);
        }
    }
    for (n, _) in dirty_nodes.drain(..) {
        scene.clear_dirty(n, dirty::LAYOUT | dirty::CHILDREN);
    }
    cache.dirty_scratch = dirty_nodes;
    cache.changed_scratch = changed;
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
