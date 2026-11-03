// SPDX-License-Identifier: BSD-2-Clause
//! The flex-like layout engine (plan section 2.4).
//!
//! # Model
//!
//! Every box is border-box. A node's size is decided by [`Engine::measure`] (what size does this
//! node take, given what the parent already knows and how much room there is) and its children are
//! placed by [`Engine::arrange`] once the size is final. Children are laid out by the flexbox
//! algorithm (single or multi line, grow and shrink with min/max freezing, justify, align, gaps,
//! auto margins, reverse directions, absolute children, overflow). Differences from CSS, all
//! deliberate and listed in `docs/modules/wolkit-style.md`:
//!
//! - `min-width`/`min-height` of `auto` is 0, not the content size; a `scroll` or `auto` container
//!   never shrinks its children (it scrolls).
//! - Percentages resolve against the parent's inner box when it is known and are `auto` otherwise.
//! - Content-sized width is `min(max-content, available)`; height is never clamped.
//! - `align-items: baseline` behaves as `start`.
//!
//! # Incremental layout
//!
//! `measure` results are cached per node (four entries keyed by what the parent knew and offered).
//! `arrange` is skipped for a node whose subtree is unchanged and whose size is unchanged. A change
//! invalidates the node and its ancestors up to the nearest *relayout boundary*: a node whose size
//! cannot depend on its content (both width and height in pixels) or that is out of the flow.
use super::tree::*;
use crate::scene::NIL;
use crate::style::value::*;
use std::collections::HashMap;

type O = Option<f32>;

fn finite(v: O) -> O {
    v.filter(|x| x.is_finite())
}

/// What the parent knew (width, height) and offered (width, height), packed: a missing value is a
/// NaN, which a finite offer never is, so two keys are equal exactly when all four bit patterns are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Key([u32; 4]);

const NONE_BITS: u32 = 0x7fc0_0000;
/// An unused cache slot (a pattern no key has: a NaN with a payload).
const EMPTY: Key = Key([0xffff_ffff; 4]);

fn pack(v: O) -> u32 {
    match finite(v) {
        // `+ 0.0` turns -0.0 into 0.0 so equal numbers have equal bits.
        Some(x) => (x + 0.0).to_bits(),
        None => NONE_BITS,
    }
}

fn unpack(b: u32) -> O {
    if b == NONE_BITS {
        None
    } else {
        Some(f32::from_bits(b))
    }
}

impl Key {
    fn new(known: (O, O), avail: (O, O)) -> Key {
        let (kw, kh) = (pack(known.0), pack(known.1));
        Key([
            kw,
            kh,
            if kw != NONE_BITS {
                NONE_BITS
            } else {
                pack(avail.0)
            },
            if kh != NONE_BITS {
                NONE_BITS
            } else {
                pack(avail.1)
            },
        ])
    }
    fn known(&self) -> (O, O) {
        (unpack(self.0[0]), unpack(self.0[1]))
    }
    fn avail(&self) -> (O, O) {
        (unpack(self.0[2]), unpack(self.0[3]))
    }
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    key: Key,
    size: Size,
}

const WAYS: usize = 4;
const EMPTY_ENTRY: Entry = Entry {
    key: EMPTY,
    size: Size::ZERO,
};

#[derive(Clone, Debug)]
pub struct NodeCache {
    entries: [Entry; WAYS],
    next: u8,
    arranged: bool,
    /// Size at the last arrange.
    size: Size,
    /// Rectangle in the parent's border box.
    rel: Rect,
    /// Rectangle in window space (before scroll).
    abs: Rect,
    abs_dirty: bool,
    sub_dirty: bool,
    /// Something below was invalidated behind a relayout boundary: visit the children even though
    /// this node's own layout stands.
    inner_dirty: bool,
    /// Extent of the children plus padding: what a scroll container scrolls over.
    content: Size,
}

impl Default for NodeCache {
    fn default() -> Self {
        NodeCache {
            entries: [EMPTY_ENTRY; WAYS],
            next: 0,
            arranged: false,
            size: Size::ZERO,
            rel: Rect::default(),
            abs: Rect::default(),
            abs_dirty: true,
            sub_dirty: false,
            inner_dirty: false,
            content: Size::ZERO,
        }
    }
}

impl NodeCache {
    fn lookup(&self, k: &Key) -> Option<Size> {
        self.entries.iter().find(|e| e.key == *k).map(|e| e.size)
    }
    fn store(&mut self, key: Key, size: Size) {
        self.entries[self.next as usize % WAYS] = Entry { key, size };
        self.next = (self.next + 1) % WAYS as u8;
    }
    fn clear(&mut self) {
        self.entries = [EMPTY_ENTRY; WAYS];
        self.arranged = false;
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Item {
    node: u32,
    /// Margins top, right, bottom, left (auto counts as 0 here).
    mm: [f32; 4],
    auto: [bool; 4],
    base: f32,
    hyp: f32,
    min: f32,
    max: f32,
    target: f32,
    frozen: bool,
    /// Border-box cross size.
    cross: f32,
    grow: f32,
    shrink: f32,
    viol: i8,
}

#[derive(Clone, Copy, Debug, Default)]
struct Line {
    start: usize,
    end: usize,
    sum: f32,
    cross: f32,
    cpos: f32,
}

/// Counters for tests and benches.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    pub measures: u64,
    pub hits: u64,
    pub arranges: u64,
    pub skipped: u64,
}

/// Persistent layout state of one window: per node caches plus scratch space.
#[derive(Default)]
pub struct LayoutCache {
    nodes: Vec<NodeCache>,
    items: Vec<Item>,
    lines: Vec<Line>,
    abs: Vec<u32>,
    overlays: Vec<u32>,
    vlists: HashMap<u32, super::vlist::VlistModel>,
    changed: Vec<(u32, Rect)>,
    rows: Vec<(u32, u32, Size, [f32; 4])>,
    /// Scratch for the scene adapter, kept so a frame allocates nothing once warm.
    pub(crate) dirty_scratch: Vec<(u32, u8)>,
    pub(crate) changed_scratch: Vec<(u32, Rect)>,
    pub stats: Stats,
}

impl LayoutCache {
    pub fn new() -> Self {
        Self::default()
    }

    fn ensure(&mut self, n: u32) {
        let need = n as usize + 1;
        if self.nodes.len() < need {
            self.nodes.resize_with(need, NodeCache::default);
        }
    }

    /// Forget everything (the style table changed).
    pub fn invalidate_all(&mut self) {
        for n in &mut self.nodes {
            n.clear();
            n.abs_dirty = true;
        }
    }

    /// The scrollable extent of `n` after the last layout.
    pub fn content_size(&self, n: u32) -> Size {
        self.nodes.get(n as usize).map_or(Size::ZERO, |c| c.content)
    }

    /// The window-space rectangle of `n` after the last layout.
    pub fn rect(&self, n: u32) -> Rect {
        self.nodes
            .get(n as usize)
            .map_or(Rect::default(), |c| c.abs)
    }

    /// The cache entry of a node as text, for test failures.
    pub fn debug_node(&self, n: u32) -> String {
        self.nodes.get(n as usize).map_or("none".into(), |c| {
            format!(
                "arranged={} size={:?} rel={:?} abs={:?} abs_dirty={} sub_dirty={} inner_dirty={}",
                c.arranged, c.size, c.rel, c.abs, c.abs_dirty, c.sub_dirty, c.inner_dirty
            )
        })
    }

    pub fn vlist(&self, n: u32) -> Option<&super::vlist::VlistModel> {
        self.vlists.get(&n)
    }

    pub fn vlist_mut(&mut self, n: u32) -> Option<&mut super::vlist::VlistModel> {
        self.vlists.get_mut(&n)
    }

    /// A node's own layout inputs changed (style, text, a property): the node and its parent chain
    /// up to a boundary must be laid out again.
    pub fn invalidate_own<T: LayoutTree>(&mut self, t: &T, n: u32) {
        self.ensure(n);
        self.nodes[n as usize].clear();
        let p = t.parent(n);
        if p != NIL {
            self.invalidate_content(t, p);
        }
    }

    /// Something below `n` changed (children added, removed or moved, or a descendant changed).
    pub fn invalidate_content<T: LayoutTree>(&mut self, t: &T, mut n: u32) {
        let mut guard = 0;
        while n != NIL && guard < 4096 {
            self.ensure(n);
            self.nodes[n as usize].clear();
            if is_boundary(t.style(n)) {
                // The ancestors keep their layout but must let the walk reach this node.
                let mut p = t.parent(n);
                while p != NIL && guard < 8192 {
                    self.ensure(p);
                    if self.nodes[p as usize].inner_dirty {
                        break;
                    }
                    self.nodes[p as usize].inner_dirty = true;
                    p = t.parent(p);
                    guard += 1;
                }
                break;
            }
            n = t.parent(n);
            guard += 1;
        }
    }

    /// Rectangles that changed in the last `layout` call.
    pub fn take_changed(&mut self) -> Vec<(u32, Rect)> {
        std::mem::take(&mut self.changed)
    }
}

/// A node whose outer size cannot depend on its content, or that leaves the flow.
pub fn is_boundary(s: &LayoutStyle) -> bool {
    s.display == Display::None
        || s.position == Position::Absolute
        || (matches!(s.width, Dim::Px(_)) && matches!(s.height, Dim::Px(_)))
}

fn insets(s: &LayoutStyle) -> (f32, f32) {
    let b = s.border_width * 2.0;
    (
        s.padding[1] + s.padding[3] + b,
        s.padding[0] + s.padding[2] + b,
    )
}

fn clamp_axis(v: f32, min: Dim, max: Dim, base: O, floor: f32) -> f32 {
    let lo = min.resolve(base).unwrap_or(0.0).max(floor);
    let mut v = v;
    if let Some(m) = max.resolve(base) {
        v = v.min(m);
    }
    v.max(lo)
}

fn margin(d: Dim, base: O) -> (f32, bool) {
    match d {
        Dim::Auto => (0.0, true),
        _ => (d.resolve(base).unwrap_or(0.0), false),
    }
}

fn mc<A: Copy>(row: bool, w: A, h: A) -> (A, A) {
    if row { (w, h) } else { (h, w) }
}

fn align_of(parent: &LayoutStyle, child: &LayoutStyle) -> AlignItems {
    match child.align_self {
        AlignSelf::Auto => parent.align_items,
        AlignSelf::Stretch => AlignItems::Stretch,
        AlignSelf::Start => AlignItems::Start,
        AlignSelf::Center => AlignItems::Center,
        AlignSelf::End => AlignItems::End,
        AlignSelf::Baseline => AlignItems::Baseline,
    }
}

pub struct Engine<'a, T: LayoutTree, M: Measure> {
    pub t: &'a T,
    pub m: &'a mut M,
    pub c: &'a mut LayoutCache,
}

impl<T: LayoutTree, M: Measure> Engine<'_, T, M> {
    /// The outer (border-box) size of `n` when the parent fixes `known` and offers `avail`.
    pub fn measure(&mut self, n: u32, known: (O, O), avail: (O, O)) -> Size {
        let t = self.t;
        let info = t.info(n);
        let st = info.style;
        if st.display == Display::None {
            return Size::ZERO;
        }
        let mut key = Key::new(known, avail);
        // A leaf looks at the room on an axis only through percentages (and text through the
        // width it wraps at): ignoring the rest lets the same answer serve every container that
        // asks with a different offer on the other axis.
        let leaf = info.leaf;
        if leaf != Leaf::None {
            let pct = |d: Dim| matches!(d, Dim::Pct(_));
            if !(pct(st.height) || pct(st.min_height) || pct(st.max_height)) {
                key.0[3] = NONE_BITS;
            }
            if leaf != Leaf::Text && !(pct(st.width) || pct(st.min_width) || pct(st.max_width)) {
                key.0[2] = NONE_BITS;
            }
        }
        self.c.ensure(n);
        if let Some(s) = self.c.nodes[n as usize].lookup(&key) {
            self.c.stats.hits += 1;
            return s;
        }
        self.c.stats.measures += 1;
        let s = self.measure_uncached(n, st, leaf, key.known(), key.avail());
        self.c.nodes[n as usize].store(key, s);
        s
    }

    fn measure_uncached(
        &mut self,
        n: u32,
        s: &LayoutStyle,
        leaf: Leaf,
        known: (O, O),
        avail: (O, O),
    ) -> Size {
        let (iw, ih) = insets(s);
        let mut w = known.0.or_else(|| s.width.resolve(avail.0));
        let mut h = known.1.or_else(|| s.height.resolve(avail.1));
        if s.aspect > 0.0 {
            if w.is_none() && h.is_some() {
                w = h.map(|v| v * s.aspect);
            } else if h.is_none() && w.is_some() {
                h = w.map(|v| v / s.aspect);
            }
        }
        if known.0.is_none() {
            w = w.map(|v| clamp_axis(v, s.min_width, s.max_width, avail.0, iw));
        }
        if known.1.is_none() {
            h = h.map(|v| clamp_axis(v, s.min_height, s.max_height, avail.1, ih));
        }
        if let (Some(w), Some(h)) = (w, h) {
            return Size::new(w.max(iw), h.max(ih));
        }
        match leaf {
            Leaf::Text => {
                let room = w
                    .map(|v| v - iw)
                    .or(avail.0.map(|a| a - iw))
                    .map(|v| v.max(0.0));
                let c = self.m.text(n, room);
                let wf = w
                    .unwrap_or_else(|| clamp_axis(c.w + iw, s.min_width, s.max_width, avail.0, iw));
                let hf = h.unwrap_or_else(|| {
                    clamp_axis(c.h + ih, s.min_height, s.max_height, avail.1, ih)
                });
                Size::new(wf, hf)
            }
            Leaf::Control | Leaf::Image => {
                let i = self.m.intrinsic(n);
                let (cw, ch) = (i.w + iw, i.h + ih);
                let (wf, hf) = match (w, h) {
                    (Some(w), None) if i.w > 0.0 && leaf == Leaf::Image => {
                        (w, ((w - iw) * i.h / i.w + ih).max(ih))
                    }
                    (None, Some(h)) if i.h > 0.0 && leaf == Leaf::Image => {
                        (((h - ih) * i.w / i.h + iw).max(iw), h)
                    }
                    _ => (w.unwrap_or(cw), h.unwrap_or(ch)),
                };
                let wf = if w.is_none() {
                    clamp_axis(wf, s.min_width, s.max_width, avail.0, iw)
                } else {
                    wf
                };
                let hf = if h.is_none() {
                    clamp_axis(hf, s.min_height, s.max_height, avail.1, ih)
                } else {
                    hf
                };
                Size::new(wf, hf)
            }
            Leaf::None => {
                let inner_avail_h = avail.1.map(|a| (a - ih).max(0.0));
                let wf = match w {
                    Some(v) => v,
                    None => {
                        let ia = (avail.0.map(|a| (a - iw).max(0.0)), inner_avail_h);
                        let content = self.flex_run(n, (None, None), ia, None);
                        let mut v = content.w + iw;
                        if let Some(a) = avail.0 {
                            v = v.min(a);
                        }
                        clamp_axis(v, s.min_width, s.max_width, avail.0, iw)
                    }
                };
                let hf = match h {
                    Some(v) => v,
                    None => {
                        let ia = (Some((wf - iw).max(0.0)), inner_avail_h);
                        let content = self.flex_run(n, (Some((wf - iw).max(0.0)), None), ia, None);
                        clamp_axis(content.h + ih, s.min_height, s.max_height, avail.1, ih)
                    }
                };
                Size::new(wf.max(iw), hf.max(ih))
            }
        }
    }

    /// Run the flex algorithm over the children of `n` inside an inner box whose sizes may be
    /// known (`known`, content box) and within `avail`. With `arrange` set (the node's border-box
    /// size) the children are placed; otherwise only the content size is computed.
    fn flex_run(&mut self, n: u32, known: (O, O), avail: (O, O), arrange: Option<Size>) -> Size {
        let t = self.t;
        let s = t.style(n);
        let row = matches!(s.direction, Direction::Row | Direction::RowReverse);
        let rev = matches!(
            s.direction,
            Direction::RowReverse | Direction::ColumnReverse
        );
        let (im, ic) = mc(row, known.0, known.1);
        let (am0, ac0) = mc(row, avail.0, avail.1);
        let (am, ac) = (im.or(am0), ic.or(ac0));
        let (gap_m, gap_c) = if row {
            (s.col_gap, s.row_gap)
        } else {
            (s.row_gap, s.col_gap)
        };
        let wbase = known.0.or(avail.0);
        let (ib, lb, ab) = (self.c.items.len(), self.c.lines.len(), self.c.abs.len());
        let (b_x, b_y) = (s.border_width + s.padding[3], s.border_width + s.padding[0]);

        // ---- collect in-flow children and their flex base sizes
        let mut ch = t.first_child(n);
        while ch != NIL {
            let ci = t.info(ch);
            let cs = ci.style;
            let next = t.next_sibling(ch);
            if cs.display == Display::None {
                if arrange.is_some() {
                    self.zero(ch);
                }
                ch = next;
                continue;
            }
            if cs.position == Position::Absolute || ci.overlay {
                if arrange.is_some() {
                    if ci.overlay {
                        if !self.c.overlays.contains(&ch) {
                            self.c.overlays.push(ch);
                        }
                    } else {
                        self.c.abs.push(ch);
                    }
                }
                ch = next;
                continue;
            }
            let (mt, at) = margin(cs.margin[0], wbase);
            let (mr, ar) = margin(cs.margin[1], wbase);
            let (mb, ab_) = margin(cs.margin[2], wbase);
            let (ml, al) = margin(cs.margin[3], wbase);
            let mm = [mt, mr, mb, ml];
            let (_, marg_c) = if row {
                (ml + mr, mt + mb)
            } else {
                (mt + mb, ml + mr)
            };
            let (cms, ccs) = if row {
                (cs.width, cs.height)
            } else {
                (cs.height, cs.width)
            };
            let (cmin, cmax) = if row {
                (cs.min_width, cs.max_width)
            } else {
                (cs.min_height, cs.max_height)
            };
            let (cw_ins, ch_ins) = insets(cs);
            let ins_m = if row { cw_ins } else { ch_ins };
            let auto = [at, ar, ab_, al];
            let auto_c = if row { at || ab_ } else { ar || al };
            let stretch = align_of(s, cs) == AlignItems::Stretch && ccs.is_auto() && !auto_c;
            let basis = match cs.basis {
                Dim::Px(v) => Some(v),
                Dim::Pct(p) => im.map(|m| m * p / 100.0),
                Dim::Auto => None,
            };
            let base = match basis.or(cms.resolve(im)) {
                Some(v) => v,
                None => {
                    let kc = if stretch {
                        ic.map(|c| (c - marg_c).max(0.0))
                    } else {
                        None
                    };
                    let ac_adj = ac.map(|a| (a - marg_c).max(0.0));
                    let (kw, kh) = mc(row, None, kc);
                    let (aw, ah) = mc(row, None, ac_adj);
                    let sz = self.measure(ch, (kw, kh), (aw, ah));
                    if row { sz.w } else { sz.h }
                }
            };
            let base = base.max(ins_m);
            let min = cmin.resolve(im).unwrap_or(0.0).max(ins_m);
            let max = cmax.resolve(im).unwrap_or(f32::INFINITY);
            let hyp = base.min(max).max(min);
            self.c.items.push(Item {
                node: ch,
                mm,
                auto,
                base,
                hyp,
                min,
                max,
                grow: cs.grow,
                shrink: cs.shrink,
                target: hyp,
                ..Item::default()
            });
            ch = next;
        }
        let ie = self.c.items.len();

        // ---- lines
        let wrap = s.wrap == Wrap::Wrap;
        let limit = im.or(am);
        let outer_m = |it: &Item| -> f32 {
            it.hyp
                + if row {
                    it.mm[1] + it.mm[3]
                } else {
                    it.mm[0] + it.mm[2]
                }
        };
        let mut start = ib;
        let mut sum = 0.0f32;
        for i in ib..ie {
            let o = outer_m(&self.c.items[i]);
            let break_here =
                wrap && i > start && limit.is_some_and(|l| sum + gap_m + o > l + 0.001);
            if break_here {
                self.c.lines.push(Line {
                    start,
                    end: i,
                    sum,
                    ..Line::default()
                });
                start = i;
                sum = o;
            } else {
                sum += if i > start { gap_m } else { 0.0 } + o;
            }
        }
        if ie > ib {
            self.c.lines.push(Line {
                start,
                end: ie,
                sum,
                ..Line::default()
            });
        }
        let nl = self.c.lines.len() - lb;
        let max_sum = self.c.lines[lb..]
            .iter()
            .map(|l| l.sum)
            .fold(0.0f32, f32::max);
        let cont_m = im.unwrap_or(max_sum);
        let no_shrink = matches!(s.overflow, Overflow::Scroll | Overflow::Auto);

        // ---- flexible lengths per line, then cross sizes
        for li in lb..lb + nl {
            let Line { start, end, .. } = self.c.lines[li];
            let marg: f32 = self.c.items[start..end]
                .iter()
                .map(|it| {
                    if row {
                        it.mm[1] + it.mm[3]
                    } else {
                        it.mm[0] + it.mm[2]
                    }
                })
                .sum();
            let gaps = gap_m * (end - start).saturating_sub(1) as f32;
            resolve_flex(
                &mut self.c.items[start..end],
                cont_m - gaps - marg,
                no_shrink,
            );
            let mut line_cross = 0.0f32;
            for i in start..end {
                let it = self.c.items[i];
                let cs = t.style(it.node);
                let marg_c = if row {
                    it.mm[0] + it.mm[2]
                } else {
                    it.mm[1] + it.mm[3]
                };
                let ac_adj = ac.map(|a| (a - marg_c).max(0.0));
                let (kw, kh) = mc(row, Some(it.target), None);
                let (aw, ah) = mc(row, None, ac_adj);
                let sz = self.measure(it.node, (kw, kh), (aw, ah));
                let cross = if row { sz.h } else { sz.w };
                self.c.items[i].cross = cross;
                let _ = cs;
                line_cross = line_cross.max(cross + marg_c);
            }
            self.c.lines[li].cross = line_cross;
        }
        let single = !wrap || nl <= 1;
        if single
            && nl >= 1
            && let Some(c) = ic
        {
            self.c.lines[lb].cross = c;
        }
        let gaps_c = gap_c * nl.saturating_sub(1) as f32;
        let sum_cross: f32 = self.c.lines[lb..].iter().map(|l| l.cross).sum::<f32>() + gaps_c;
        let cont_c = ic.unwrap_or(sum_cross);
        let content = {
            let (w, h) = mc(row, cont_m, cont_c);
            Size::new(w, h)
        };
        let Some(node_size) = arrange else {
            self.c.items.truncate(ib);
            self.c.lines.truncate(lb);
            self.c.abs.truncate(ab);
            return content;
        };

        // ---- align-content: distribute spare cross space between lines
        let free_c = cont_c - sum_cross;
        let (mut lead_c, mut between_c) = (0.0f32, gap_c);
        if (!single || (wrap && nl > 1)) && free_c > 0.0 {
            match s.align_content {
                AlignContent::Stretch => {
                    let add = free_c / nl as f32;
                    for l in &mut self.c.lines[lb..] {
                        l.cross += add;
                    }
                }
                AlignContent::Start => {}
                AlignContent::Center => lead_c = free_c / 2.0,
                AlignContent::End => lead_c = free_c,
                AlignContent::SpaceBetween => {
                    if nl > 1 {
                        between_c += free_c / (nl - 1) as f32;
                    }
                }
                AlignContent::SpaceAround => {
                    lead_c = free_c / (2 * nl) as f32;
                    between_c += free_c / nl as f32;
                }
            }
        }
        let mut cp = lead_c;
        for l in &mut self.c.lines[lb..] {
            l.cpos = cp;
            cp += l.cross + between_c;
        }

        // ---- place items
        let mut ext_x = 0.0f32;
        let mut ext_y = 0.0f32;
        for li in lb..lb + nl {
            let Line {
                start,
                end,
                cross: line_cross,
                cpos: line_cpos,
                ..
            } = self.c.lines[li];
            let cnt = end - start;
            let mut used = gap_m * cnt.saturating_sub(1) as f32;
            let mut autos = 0u32;
            for it in &self.c.items[start..end] {
                let (ms, me, as_, ae) = if row {
                    if rev {
                        (it.mm[1], it.mm[3], it.auto[1], it.auto[3])
                    } else {
                        (it.mm[3], it.mm[1], it.auto[3], it.auto[1])
                    }
                } else if rev {
                    (it.mm[2], it.mm[0], it.auto[2], it.auto[0])
                } else {
                    (it.mm[0], it.mm[2], it.auto[0], it.auto[2])
                };
                used += it.target + ms + me;
                autos += u32::from(as_) + u32::from(ae);
            }
            let free = cont_m - used;
            let (mut off, mut space, mut auto_share) = (0.0f32, gap_m, 0.0f32);
            if autos > 0 && free > 0.0 {
                auto_share = free / autos as f32;
            } else if free > 0.0 {
                match s.justify {
                    Justify::Start => {}
                    Justify::Center => off = free / 2.0,
                    Justify::End => off = free,
                    Justify::SpaceBetween => {
                        if cnt > 1 {
                            space += free / (cnt - 1) as f32;
                        }
                    }
                    Justify::SpaceAround => {
                        off = free / (2 * cnt) as f32;
                        space += free / cnt as f32;
                    }
                    Justify::SpaceEvenly => {
                        off = free / (cnt + 1) as f32;
                        space += free / (cnt + 1) as f32;
                    }
                }
            }
            let mut p = off;
            for i in start..end {
                let it = self.c.items[i];
                let cs = t.style(it.node);
                let (ms, me, as_, ae) = if row {
                    if rev {
                        (it.mm[1], it.mm[3], it.auto[1], it.auto[3])
                    } else {
                        (it.mm[3], it.mm[1], it.auto[3], it.auto[1])
                    }
                } else if rev {
                    (it.mm[2], it.mm[0], it.auto[2], it.auto[0])
                } else {
                    (it.mm[0], it.mm[2], it.auto[0], it.auto[2])
                };
                let ms = ms + if as_ { auto_share } else { 0.0 };
                let me = me + if ae { auto_share } else { 0.0 };
                let mut pos = p + ms;
                p += ms + it.target + me + space;
                if rev {
                    // Mirror inside the container: the logical start is the physical far side.
                    pos = cont_m - pos - it.target;
                }
                // cross
                let (cs_t, cs_b) = if row {
                    (it.mm[0], it.mm[2])
                } else {
                    (it.mm[3], it.mm[1])
                };
                let (a_t, a_b) = if row {
                    (it.auto[0], it.auto[2])
                } else {
                    (it.auto[3], it.auto[1])
                };
                let mut cross = it.cross;
                let al = align_of(s, cs);
                let ccs = if row { cs.height } else { cs.width };
                let marg_c = cs_t + cs_b;
                if al == AlignItems::Stretch && ccs.is_auto() && !a_t && !a_b {
                    let (cmin, cmax, ins) = if row {
                        (cs.min_height, cs.max_height, insets(cs).1)
                    } else {
                        (cs.min_width, cs.max_width, insets(cs).0)
                    };
                    let v = (line_cross - marg_c).max(0.0);
                    cross = clamp_axis(v, cmin, cmax, ic, ins);
                }
                let spare = line_cross - cross - marg_c;
                let mut cpos = cs_t;
                if (a_t || a_b) && spare > 0.0 {
                    let share = spare / (u32::from(a_t) + u32::from(a_b)) as f32;
                    if a_t {
                        cpos += share;
                    }
                } else if spare > 0.0 {
                    match al {
                        AlignItems::Center => cpos += spare / 2.0,
                        AlignItems::End => cpos += spare,
                        _ => {}
                    }
                }
                cpos += line_cpos;
                let (x, y, w, h) = if row {
                    (pos, cpos, it.target, cross)
                } else {
                    (cpos, pos, cross, it.target)
                };
                let rel = Rect::new(b_x + x, b_y + y, w, h);
                ext_x = ext_x.max(rel.x + rel.w + it.mm[1]);
                ext_y = ext_y.max(rel.y + rel.h + it.mm[2]);
                self.set_rel(it.node, rel);
                self.arrange(it.node, Size::new(w, h));
            }
        }
        self.c.items.truncate(ib);
        self.c.lines.truncate(lb);

        // ---- absolute children
        // Entries above `ab` are ours; nested calls push above them and truncate back.
        let end = self.c.abs.len();
        for k in ab..end {
            let a = self.c.abs[k];
            self.place_absolute(n, node_size, a);
        }
        self.c.abs.truncate(ab);
        let c = &mut self.c.nodes[n as usize];
        c.content = Size::new(
            ext_x + s.padding[1] + s.border_width,
            ext_y + s.padding[2] + s.border_width,
        );
        content
    }

    fn set_rel(&mut self, n: u32, r: Rect) {
        self.c.ensure(n);
        self.c.nodes[n as usize].rel = r;
    }

    /// A display:none node and everything below it has no rectangle.
    fn zero(&mut self, n: u32) {
        self.c.ensure(n);
        let c = &mut self.c.nodes[n as usize];
        c.rel = Rect::default();
        c.size = Size::ZERO;
        c.arranged = false;
        c.abs_dirty = true;
        c.content = Size::ZERO;
    }

    fn place_absolute(&mut self, parent: u32, psize: Size, ch: u32) {
        let t = self.t;
        let ps = t.style(parent);
        let cs = t.style(ch);
        let b = ps.border_width;
        let (cbw, cbh) = ((psize.w - 2.0 * b).max(0.0), (psize.h - 2.0 * b).max(0.0));
        let inset = |d: Dim, base: f32| d.resolve(Some(base));
        let (top, right, bottom, left) = (
            inset(cs.inset[0], cbh),
            inset(cs.inset[1], cbw),
            inset(cs.inset[2], cbh),
            inset(cs.inset[3], cbw),
        );
        let (mt, _) = margin(cs.margin[0], Some(cbw));
        let (mr, _) = margin(cs.margin[1], Some(cbw));
        let (mb, _) = margin(cs.margin[2], Some(cbw));
        let (ml, _) = margin(cs.margin[3], Some(cbw));
        let kw = cs.width.resolve(Some(cbw)).or_else(|| match (left, right) {
            (Some(l), Some(r)) => Some((cbw - l - r - ml - mr).max(0.0)),
            _ => None,
        });
        let kh = cs
            .height
            .resolve(Some(cbh))
            .or_else(|| match (top, bottom) {
                (Some(tp), Some(bt)) => Some((cbh - tp - bt - mt - mb).max(0.0)),
                _ => None,
            });
        let sz = self.measure(
            ch,
            (kw, kh),
            (
                Some((cbw - ml - mr).max(0.0)),
                Some((cbh - mt - mb).max(0.0)),
            ),
        );
        let x = if let Some(l) = left {
            b + l + ml
        } else if let Some(r) = right {
            b + cbw - r - mr - sz.w
        } else {
            b + ps.padding[3] + ml
        };
        let y = if let Some(tp) = top {
            b + tp + mt
        } else if let Some(bt) = bottom {
            b + cbh - bt - mb - sz.h
        } else {
            b + ps.padding[0] + mt
        };
        self.set_rel(ch, Rect::new(x, y, sz.w, sz.h));
        self.arrange(ch, sz);
    }

    /// Place the children of `n` now that its border-box `size` is final.
    pub fn arrange(&mut self, n: u32, size: Size) {
        self.c.ensure(n);
        {
            let c = &self.c.nodes[n as usize];
            if c.arranged && c.size == size {
                if !c.inner_dirty {
                    self.c.stats.skipped += 1;
                    return;
                }
                self.c.nodes[n as usize].inner_dirty = false;
                let t = self.t;
                let mut ch = t.first_child(n);
                while ch != NIL {
                    self.c.ensure(ch);
                    let cc = &self.c.nodes[ch as usize];
                    let ci = t.info(ch);
                    if (!cc.arranged || cc.inner_dirty)
                        && ci.style.display != Display::None
                        && !ci.overlay
                    {
                        if ci.style.position == Position::Absolute {
                            // An absolute child sizes itself from its content: the parent
                            // measures and places it again.
                            self.place_absolute(n, size, ch);
                        } else {
                            let sz = cc.size;
                            self.arrange(ch, sz);
                        }
                    }
                    ch = t.next_sibling(ch);
                }
                return;
            }
        }
        self.c.stats.arranges += 1;
        {
            let c = &mut self.c.nodes[n as usize];
            c.size = size;
            c.arranged = true;
            c.abs_dirty = true;
            c.inner_dirty = false;
        }
        // Tell the ancestors something below them moved.
        let mut p = self.t.parent(n);
        let mut guard = 0;
        while p != NIL && guard < 4096 {
            self.c.ensure(p);
            let pc = &mut self.c.nodes[p as usize];
            if pc.sub_dirty {
                break;
            }
            pc.sub_dirty = true;
            p = self.t.parent(p);
            guard += 1;
        }
        let t = self.t;
        let info = t.info(n);
        if info.leaf != Leaf::None {
            self.c.nodes[n as usize].content = size;
            return;
        }
        let s = info.style;
        let (iw, ih) = insets(s);
        let inner = (Some((size.w - iw).max(0.0)), Some((size.h - ih).max(0.0)));
        if let Some(v) = info.vlist.then(|| t.vlist(n)).flatten() {
            self.arrange_vlist(n, size, v);
        } else {
            self.flex_run(n, inner, inner, Some(size));
        }
    }

    fn arrange_vlist(&mut self, n: u32, size: Size, info: VlistInfo) {
        let t = self.t;
        let s = t.style(n);
        let (iw, _ih) = insets(s);
        let inner_w = (size.w - iw).max(0.0);
        let model = self
            .c
            .vlists
            .entry(n)
            .or_insert_with(|| super::vlist::VlistModel::new(info.count, info.estimate));
        model.reset(info.count, info.estimate);
        // Measure the realised rows at the list's width, record their heights.
        let rb = self.c.rows.len();
        let mut ch = t.first_child(n);
        while ch != NIL {
            let cs = t.style(ch);
            if cs.display != Display::None {
                let (mt, _) = margin(cs.margin[0], Some(inner_w));
                let (mr, _) = margin(cs.margin[1], Some(inner_w));
                let (mb, _) = margin(cs.margin[2], Some(inner_w));
                let (ml, _) = margin(cs.margin[3], Some(inner_w));
                let stretch = cs.width.is_auto() && align_of(s, cs) == AlignItems::Stretch;
                let kw = if stretch {
                    Some((inner_w - ml - mr).max(0.0))
                } else {
                    None
                };
                let sz = self.measure(ch, (kw, None), (Some((inner_w - ml - mr).max(0.0)), None));
                self.c
                    .rows
                    .push((ch, t.row_index(ch), sz, [mt, mr, mb, ml]));
            }
            ch = t.next_sibling(ch);
        }
        let re = self.c.rows.len();
        if let Some(m) = self.c.vlists.get_mut(&n) {
            for (_, idx, sz, mm) in &self.c.rows[rb..re] {
                m.set_measured(*idx, sz.h + mm[0] + mm[2]);
            }
        }
        let origin = (s.border_width + s.padding[3], s.border_width + s.padding[0]);
        let mut total = 0.0;
        if let Some(m) = self.c.vlists.get(&n) {
            total = m.total();
        }
        for k in rb..re {
            let (ch, idx, sz, mm) = self.c.rows[k];
            let y = self.c.vlists.get(&n).map_or(0.0, |m| m.offset(idx));
            self.set_rel(
                ch,
                Rect::new(origin.0 + mm[3], origin.1 + y + mm[0], sz.w, sz.h),
            );
            self.arrange(ch, sz);
        }
        self.c.rows.truncate(rb);
        self.c.nodes[n as usize].content = Size::new(
            size.w,
            total + s.padding[0] + s.padding[2] + 2.0 * s.border_width,
        );
    }

    /// Turn parent-relative rectangles into window-space ones for what moved.
    fn place_abs(&mut self, n: u32, origin: (f32, f32)) {
        let t = self.t;
        self.c.ensure(n);
        let nc = &self.c.nodes[n as usize];
        let abs = Rect::new(origin.0 + nc.rel.x, origin.1 + nc.rel.y, nc.rel.w, nc.rel.h);
        let moved = abs != nc.abs || nc.abs_dirty;
        let descend = moved || nc.sub_dirty;
        if moved {
            self.c.changed.push((n, abs));
        }
        let nc = &mut self.c.nodes[n as usize];
        nc.abs = abs;
        nc.abs_dirty = false;
        nc.sub_dirty = false;
        if !descend {
            return;
        }
        let mut ch = t.first_child(n);
        while ch != NIL {
            let ci = t.info(ch);
            if ci.style.display == Display::None {
                self.place_zero(ch);
            } else if !ci.overlay {
                self.place_abs(ch, (abs.x, abs.y));
            }
            ch = t.next_sibling(ch);
        }
    }

    fn place_zero(&mut self, n: u32) {
        self.c.ensure(n);
        let z = Rect::default();
        let c = &mut self.c.nodes[n as usize];
        if c.abs != z || c.abs_dirty {
            c.abs = z;
            c.abs_dirty = false;
            self.c.changed.push((n, z));
        }
    }

    /// Overlays: next to their anchor, or centred, after everything else is in place.
    fn place_overlays(&mut self, viewport: Size) {
        let t = self.t;
        let count = self.c.overlays.len();
        for r in 0..count {
            let n = self.c.overlays[r];
            let Some(spec) = t.overlay(n) else {
                self.c.overlays[r] = NIL;
                continue;
            };
            if t.style(n).display == Display::None {
                self.place_zero(n);
                continue;
            }
            let bounds = Rect::new(0.0, 0.0, viewport.w, viewport.h);
            let size = self.measure(n, (None, None), (Some(viewport.w), Some(viewport.h)));
            let rect = if spec.anchor != NIL && (spec.anchor as usize) < self.c.nodes.len() {
                let a = self.c.nodes[spec.anchor as usize].abs;
                let opts = super::place::PlaceOpts {
                    side: spec.side,
                    align: spec.align,
                    ..Default::default()
                };
                super::place::place(a, size, bounds, opts).rect
            } else {
                super::place::centre(size, bounds)
            };
            let parent = t.parent(n);
            let pabs = if parent != NIL {
                self.c.nodes[parent as usize].abs
            } else {
                Rect::default()
            };
            self.set_rel(
                n,
                Rect::new(rect.x - pabs.x, rect.y - pabs.y, size.w, size.h),
            );
            self.arrange(n, size);
            self.place_abs(n, (pabs.x, pabs.y));
        }
        self.c.overlays.retain(|n| *n != NIL);
    }
}

fn resolve_flex(items: &mut [Item], space: f32, no_shrink: bool) {
    let sum_hyp: f32 = items.iter().map(|i| i.hyp).sum();
    let growing = space > sum_hyp;
    for it in items.iter_mut() {
        it.target = it.hyp;
    }
    if !growing && no_shrink {
        return;
    }
    let factor = |it: &Item| if growing { it.grow } else { it.shrink };
    for it in items.iter_mut() {
        it.frozen =
            factor(it) == 0.0 || (growing && it.base > it.hyp) || (!growing && it.base < it.hyp);
    }
    let init_free = space
        - items
            .iter()
            .map(|i| if i.frozen { i.hyp } else { i.base })
            .sum::<f32>();
    for _ in 0..=items.len() {
        if items.iter().all(|i| i.frozen) {
            break;
        }
        let used: f32 = items
            .iter()
            .map(|i| if i.frozen { i.target } else { i.base })
            .sum();
        let mut rem = space - used;
        if growing {
            let sum_g: f32 = items.iter().filter(|i| !i.frozen).map(|i| i.grow).sum();
            if sum_g < 1.0 && (init_free * sum_g).abs() < rem.abs() {
                rem = init_free * sum_g;
            }
            for i in items.iter_mut().filter(|i| !i.frozen) {
                i.target = if sum_g > 0.0 {
                    i.base + rem * i.grow / sum_g
                } else {
                    i.base
                };
            }
        } else {
            let sum_s: f32 = items
                .iter()
                .filter(|i| !i.frozen)
                .map(|i| i.shrink * i.base)
                .sum();
            for i in items.iter_mut().filter(|i| !i.frozen) {
                i.target = if sum_s > 0.0 {
                    i.base + rem * (i.shrink * i.base) / sum_s
                } else {
                    i.base
                };
            }
        }
        let mut total = 0.0f32;
        for i in items.iter_mut().filter(|i| !i.frozen) {
            let c = i.target.min(i.max).max(i.min);
            total += c - i.target;
            i.viol = if c > i.target {
                1
            } else if c < i.target {
                -1
            } else {
                0
            };
            i.target = c;
        }
        for i in items.iter_mut().filter(|i| !i.frozen) {
            if total.abs() < 1e-4 || (total > 0.0 && i.viol > 0) || (total < 0.0 && i.viol < 0) {
                i.frozen = true;
            }
        }
    }
}

/// Lay out the tree under `root` for a viewport, incrementally. `report` is called for each node
/// whose window-space rectangle changed (nodes that became display:none report a zero rectangle).
pub fn layout<T: LayoutTree, M: Measure>(
    t: &T,
    m: &mut M,
    cache: &mut LayoutCache,
    root: u32,
    viewport: Size,
    report: &mut dyn FnMut(u32, Rect),
) {
    let mut e = Engine { t, m, c: cache };
    e.set_rel(root, Rect::new(0.0, 0.0, viewport.w, viewport.h));
    e.arrange(root, viewport);
    e.place_abs(root, (0.0, 0.0));
    e.place_overlays(viewport);
    for (n, r) in e.c.changed.drain(..) {
        report(n, r);
    }
}
