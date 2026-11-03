// SPDX-License-Identifier: BSD-2-Clause
//! Shared by the layout tests: a hand-made tree, a deterministic random generator, and a reference
//! oracle, a second and deliberately plain implementation of the layout semantics (no caches, no
//! scratch stacks, `f64`, allocation everywhere) that the engine is held to.
#![allow(dead_code)]
use wolkit::layout::tree::*;
use wolkit::layout::{LayoutCache, layout};
use wolkit::style::value::*;

pub const NIL: u32 = u32::MAX;

#[derive(Clone, Debug)]
pub struct TNode {
    pub style: LayoutStyle,
    pub kids: Vec<u32>,
    pub parent: u32,
    pub leaf: Leaf,
    pub text: String,
    pub font: f32,
    pub intrinsic: Size,
    pub vlist: Option<VlistInfo>,
    pub row: u32,
    pub next: u32,
}

#[derive(Clone, Debug, Default)]
pub struct TTree {
    pub nodes: Vec<TNode>,
}

impl TTree {
    pub fn add(&mut self, parent: u32, style: LayoutStyle) -> u32 {
        let id = self.nodes.len() as u32;
        self.nodes.push(TNode {
            style,
            kids: vec![],
            parent,
            leaf: Leaf::None,
            text: String::new(),
            font: 16.0,
            intrinsic: Size::ZERO,
            vlist: None,
            row: 0,
            next: NIL,
        });
        if parent != NIL {
            if let Some(&last) = self.nodes[parent as usize].kids.last() {
                self.nodes[last as usize].next = id;
            }
            self.nodes[parent as usize].kids.push(id);
        }
        id
    }
    pub fn text(&mut self, parent: u32, style: LayoutStyle, text: &str) -> u32 {
        let id = self.add(parent, style);
        self.nodes[id as usize].leaf = Leaf::Text;
        self.nodes[id as usize].text = text.to_string();
        id
    }
    pub fn control(&mut self, parent: u32, style: LayoutStyle, size: Size) -> u32 {
        let id = self.add(parent, style);
        self.nodes[id as usize].leaf = Leaf::Control;
        self.nodes[id as usize].intrinsic = size;
        id
    }
    pub fn detach(&mut self, n: u32) {
        let p = self.nodes[n as usize].parent;
        if p != NIL {
            self.nodes[p as usize].kids.retain(|k| *k != n);
            let kids = self.nodes[p as usize].kids.clone();
            for (i, k) in kids.iter().enumerate() {
                self.nodes[*k as usize].next = kids.get(i + 1).copied().unwrap_or(NIL);
            }
        }
        self.nodes[n as usize].parent = NIL;
        self.nodes[n as usize].next = NIL;
    }
    pub fn style_mut(&mut self, n: u32) -> &mut LayoutStyle {
        &mut self.nodes[n as usize].style
    }
}

impl LayoutTree for TTree {
    fn first_child(&self, n: u32) -> u32 {
        self.nodes[n as usize].kids.first().copied().unwrap_or(NIL)
    }
    fn next_sibling(&self, n: u32) -> u32 {
        self.nodes[n as usize].next
    }
    fn parent(&self, n: u32) -> u32 {
        self.nodes[n as usize].parent
    }
    fn style(&self, n: u32) -> &LayoutStyle {
        &self.nodes[n as usize].style
    }
    fn leaf(&self, n: u32) -> Leaf {
        self.nodes[n as usize].leaf
    }
    fn vlist(&self, n: u32) -> Option<VlistInfo> {
        self.nodes[n as usize].vlist
    }
    fn row_index(&self, n: u32) -> u32 {
        self.nodes[n as usize].row
    }
}

pub struct TMeasure<'a>(pub &'a TTree);

impl Measure for TMeasure<'_> {
    fn text(&mut self, n: u32, max_w: Option<f32>) -> Size {
        let nd = &self.0.nodes[n as usize];
        approx_text(&nd.text, nd.font, max_w)
    }
    fn intrinsic(&mut self, n: u32) -> Size {
        self.0.nodes[n as usize].intrinsic
    }
}

/// Full layout of a fresh cache: window-space rectangle of every reachable node.
pub fn run_engine(t: &TTree, vw: f32, vh: f32) -> Vec<Rect> {
    let mut cache = LayoutCache::new();
    let mut out = vec![Rect::default(); t.nodes.len()];
    layout(
        t,
        &mut TMeasure(t),
        &mut cache,
        0,
        Size::new(vw, vh),
        &mut |n, r| out[n as usize] = r,
    );
    out
}

// ---------------------------------------------------------------- random

pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    pub fn below(&mut self, n: u32) -> u32 {
        (self.next() >> 11) as u32 % n.max(1)
    }
    pub fn chance(&mut self, pct: u32) -> bool {
        self.below(100) < pct
    }
    pub fn pick<T: Copy>(&mut self, v: &[T]) -> T {
        v[self.below(v.len() as u32) as usize]
    }
}

fn px(r: &mut Rng) -> f32 {
    r.pick(&[0.0, 4.0, 8.0, 12.0, 16.0, 24.0])
}

pub fn random_style(r: &mut Rng, leaf: bool) -> LayoutStyle {
    let mut s = LayoutStyle::default();
    if r.chance(4) {
        s.display = Display::None;
    }
    if !leaf {
        s.direction = r.pick(Direction::ALL);
        s.wrap = if r.chance(25) {
            Wrap::Wrap
        } else {
            Wrap::NoWrap
        };
        s.justify = r.pick(Justify::ALL);
        s.align_items = r.pick(AlignItems::ALL);
        s.align_content = r.pick(AlignContent::ALL);
        s.row_gap = if r.chance(50) { px(r) } else { 0.0 };
        s.col_gap = if r.chance(50) { px(r) } else { 0.0 };
        s.overflow = r.pick(&[
            Overflow::Visible,
            Overflow::Visible,
            Overflow::Hidden,
            Overflow::Auto,
            Overflow::Scroll,
        ]);
    }
    if r.chance(30) {
        s.align_self = r.pick(AlignSelf::ALL);
    }
    s.grow = r.pick(&[0.0, 0.0, 0.0, 1.0, 1.0, 2.0, 0.5, 0.25]);
    s.shrink = r.pick(&[1.0, 1.0, 1.0, 0.0, 2.0, 0.5]);
    if r.chance(20) {
        s.basis = match r.below(3) {
            0 => Dim::Auto,
            1 => Dim::Px(r.pick(&[0.0, 20.0, 40.0, 64.0, 100.0])),
            _ => Dim::Pct(r.pick(&[10.0, 25.0, 50.0, 100.0])),
        };
    }
    for i in 0..4 {
        if r.chance(35) {
            s.padding[i] = px(r);
        }
        if r.chance(25) {
            s.margin[i] = if r.chance(8) {
                Dim::Auto
            } else {
                Dim::Px(px(r))
            };
        }
    }
    if r.chance(10) {
        s.border_width = r.pick(&[1.0, 2.0]);
    }
    let size = |r: &mut Rng| match r.below(10) {
        0..=4 => Dim::Auto,
        5..=7 => Dim::Px(r.pick(&[8.0, 20.0, 32.0, 40.0, 60.0, 80.0, 100.0, 120.0, 160.0])),
        _ => Dim::Pct(r.pick(&[10.0, 25.0, 50.0, 75.0, 100.0])),
    };
    s.width = size(r);
    s.height = size(r);
    if r.chance(15) {
        s.min_width = Dim::Px(r.pick(&[10.0, 40.0, 80.0]));
    }
    if r.chance(15) {
        s.max_width = Dim::Px(r.pick(&[30.0, 60.0, 120.0, 200.0]));
    }
    if r.chance(10) {
        s.min_height = Dim::Px(r.pick(&[10.0, 40.0]));
    }
    if r.chance(10) {
        s.max_height = Dim::Px(r.pick(&[30.0, 60.0, 120.0]));
    }
    if r.chance(6) {
        s.aspect = r.pick(&[0.5, 1.0, 2.0, 4.0]);
    }
    if r.chance(7) {
        s.position = Position::Absolute;
        for i in 0..4 {
            if r.chance(50) {
                s.inset[i] = Dim::Px(px(r));
            }
        }
    }
    s
}

pub fn random_tree(r: &mut Rng, max_depth: u32, max_kids: u32) -> TTree {
    let mut t = TTree::default();
    let mut root = random_style(r, false);
    root.display = Display::Flex;
    root.position = Position::Relative;
    t.add(NIL, root);
    fn grow(t: &mut TTree, r: &mut Rng, parent: u32, depth: u32, max_depth: u32, max_kids: u32) {
        let n = r.below(max_kids + 1);
        for _ in 0..n {
            let leafish = depth + 1 >= max_depth || r.chance(40);
            let s = random_style(r, leafish);
            let id = if leafish {
                match r.below(3) {
                    0 => {
                        let words = [
                            "a",
                            "bb",
                            "ccc",
                            "dddd",
                            "hello",
                            "wide word",
                            "tiny text here",
                            "x y z",
                        ];
                        let mut txt = String::new();
                        for k in 0..r.below(5) + 1 {
                            if k > 0 {
                                txt.push(' ');
                            }
                            txt.push_str(r.pick(&words));
                        }
                        let id = t.text(parent, s, &txt);
                        t.nodes[id as usize].font = r.pick(&[12.0, 16.0, 20.0]);
                        id
                    }
                    1 => t.control(
                        parent,
                        s,
                        Size::new(r.pick(&[16.0, 40.0, 64.0]), r.pick(&[16.0, 24.0, 40.0])),
                    ),
                    _ => t.add(parent, s),
                }
            } else {
                t.add(parent, s)
            };
            if !leafish {
                grow(t, r, id, depth + 1, max_depth, max_kids);
            }
        }
    }
    grow(&mut t, r, 0, 0, max_depth, max_kids);
    t
}

// ---------------------------------------------------------------- oracle

type O = Option<f64>;

fn dim(d: Dim, base: O) -> O {
    match d {
        Dim::Auto => None,
        Dim::Px(v) => Some(f64::from(v)),
        Dim::Pct(p) => base.map(|b| b * f64::from(p) / 100.0),
    }
}

fn clampd(v: f64, min: Dim, max: Dim, base: O, floor: f64) -> f64 {
    let mut v = v;
    if let Some(m) = dim(max, base) {
        v = v.min(m);
    }
    let lo = dim(min, base).unwrap_or(0.0).max(floor);
    v.max(lo)
}

fn ins(s: &LayoutStyle) -> (f64, f64) {
    let b = f64::from(s.border_width) * 2.0;
    (
        f64::from(s.padding[1] + s.padding[3]) + b,
        f64::from(s.padding[0] + s.padding[2]) + b,
    )
}

fn marg(d: Dim, base: O) -> (f64, bool) {
    match d {
        Dim::Auto => (0.0, true),
        _ => (dim(d, base).unwrap_or(0.0), false),
    }
}

pub struct Oracle<'a> {
    pub t: &'a TTree,
}

#[derive(Clone, Debug)]
struct It {
    n: u32,
    /// Physical margins t r b l.
    m: [f64; 4],
    auto: [bool; 4],
    base: f64,
    min: f64,
    max: f64,
    hyp: f64,
    target: f64,
    cross: f64,
    grow: f64,
    shrink: f64,
}

struct Flex {
    content: (f64, f64),
    /// Child border boxes relative to the node's border box: (child, x, y, w, h).
    placed: Vec<(u32, f64, f64, f64, f64)>,
    extent: (f64, f64),
}

impl Oracle<'_> {
    fn st(&self, n: u32) -> &LayoutStyle {
        &self.t.nodes[n as usize].style
    }

    pub fn size(&self, n: u32, known: (O, O), avail: (O, O)) -> (f64, f64) {
        let s = self.st(n);
        if s.display == Display::None {
            return (0.0, 0.0);
        }
        let (iw, ih) = ins(s);
        // A known size makes the room on that axis irrelevant.
        let avail = (
            if known.0.is_some() { None } else { avail.0 },
            if known.1.is_some() { None } else { avail.1 },
        );
        let mut w = known.0.or_else(|| dim(s.width, avail.0));
        let mut h = known.1.or_else(|| dim(s.height, avail.1));
        let a = f64::from(s.aspect);
        if a > 0.0 {
            if w.is_none() && h.is_some() {
                w = h.map(|v| v * a);
            } else if h.is_none() && w.is_some() {
                h = w.map(|v| v / a);
            }
        }
        if known.0.is_none() {
            w = w.map(|v| clampd(v, s.min_width, s.max_width, avail.0, iw));
        }
        if known.1.is_none() {
            h = h.map(|v| clampd(v, s.min_height, s.max_height, avail.1, ih));
        }
        if let (Some(w), Some(h)) = (w, h) {
            return (w.max(iw), h.max(ih));
        }
        let nd = &self.t.nodes[n as usize];
        match nd.leaf {
            Leaf::Text => {
                let room = w
                    .map(|v| v - iw)
                    .or(avail.0.map(|a| a - iw))
                    .map(|v| v.max(0.0));
                let c = approx_text(&nd.text, nd.font, room.map(|v| v as f32));
                let (cw, chh) = (f64::from(c.w), f64::from(c.h));
                let wf =
                    w.unwrap_or_else(|| clampd(cw + iw, s.min_width, s.max_width, avail.0, iw));
                let hf =
                    h.unwrap_or_else(|| clampd(chh + ih, s.min_height, s.max_height, avail.1, ih));
                (wf, hf)
            }
            Leaf::Control | Leaf::Image => {
                let (cw, chh) = (
                    f64::from(nd.intrinsic.w) + iw,
                    f64::from(nd.intrinsic.h) + ih,
                );
                let wf = w.unwrap_or_else(|| clampd(cw, s.min_width, s.max_width, avail.0, iw));
                let hf = h.unwrap_or_else(|| clampd(chh, s.min_height, s.max_height, avail.1, ih));
                (wf, hf)
            }
            Leaf::None => {
                let iah = avail.1.map(|a| (a - ih).max(0.0));
                let wf = match w {
                    Some(v) => v,
                    None => {
                        let fx = self.flex(
                            n,
                            (None, None),
                            (avail.0.map(|a| (a - iw).max(0.0)), iah),
                            None,
                        );
                        let mut v = fx.content.0 + iw;
                        if let Some(a) = avail.0 {
                            v = v.min(a);
                        }
                        clampd(v, s.min_width, s.max_width, avail.0, iw)
                    }
                };
                let hf = match h {
                    Some(v) => v,
                    None => {
                        let inner_w = (wf - iw).max(0.0);
                        let fx = self.flex(n, (Some(inner_w), None), (Some(inner_w), iah), None);
                        clampd(fx.content.1 + ih, s.min_height, s.max_height, avail.1, ih)
                    }
                };
                (wf.max(iw), hf.max(ih))
            }
        }
    }

    fn kids(&self, n: u32) -> &[u32] {
        &self.t.nodes[n as usize].kids
    }

    /// The textbook "resolve flexible lengths" loop, written over indices.
    fn resolve(items: &mut [It], space: f64, no_shrink: bool) {
        let sum_hyp: f64 = items.iter().map(|i| i.hyp).sum();
        let growing = space > sum_hyp;
        for i in items.iter_mut() {
            i.target = i.hyp;
        }
        if !growing && no_shrink {
            return;
        }
        let factor = |i: &It| if growing { i.grow } else { i.shrink };
        let mut frozen: Vec<bool> = items
            .iter()
            .map(|i| {
                factor(i) == 0.0 || (growing && i.base > i.hyp) || (!growing && i.base < i.hyp)
            })
            .collect();
        let init_free = space
            - items
                .iter()
                .zip(&frozen)
                .map(|(i, f)| if *f { i.hyp } else { i.base })
                .sum::<f64>();
        loop {
            if frozen.iter().all(|f| *f) {
                break;
            }
            let used: f64 = items
                .iter()
                .zip(&frozen)
                .map(|(i, f)| if *f { i.target } else { i.base })
                .sum();
            let mut rem = space - used;
            let unfrozen: Vec<usize> = (0..items.len()).filter(|k| !frozen[*k]).collect();
            if growing {
                let sg: f64 = unfrozen.iter().map(|k| items[*k].grow).sum();
                if sg < 1.0 && (init_free * sg).abs() < rem.abs() {
                    rem = init_free * sg;
                }
                for k in &unfrozen {
                    items[*k].target = if sg > 0.0 {
                        items[*k].base + rem * items[*k].grow / sg
                    } else {
                        items[*k].base
                    };
                }
            } else {
                let ss: f64 = unfrozen
                    .iter()
                    .map(|k| items[*k].shrink * items[*k].base)
                    .sum();
                for k in &unfrozen {
                    let i = &mut items[*k];
                    i.target = if ss > 0.0 {
                        i.base + rem * (i.shrink * i.base) / ss
                    } else {
                        i.base
                    };
                }
            }
            let mut total = 0.0;
            let mut viol = vec![0i8; items.len()];
            for k in &unfrozen {
                let i = &mut items[*k];
                let c = i.target.min(i.max).max(i.min);
                total += c - i.target;
                viol[*k] = if c > i.target {
                    1
                } else if c < i.target {
                    -1
                } else {
                    0
                };
                i.target = c;
            }
            for k in &unfrozen {
                if total.abs() < 1e-4
                    || (total > 0.0 && viol[*k] > 0)
                    || (total < 0.0 && viol[*k] < 0)
                {
                    frozen[*k] = true;
                }
            }
        }
    }

    fn flex(&self, n: u32, kin: (O, O), ain: (O, O), arrange: Option<(f64, f64)>) -> Flex {
        let s = self.st(n);
        let row = matches!(s.direction, Direction::Row | Direction::RowReverse);
        let rev = matches!(
            s.direction,
            Direction::RowReverse | Direction::ColumnReverse
        );
        let sw = |a: O, b: O| if row { (a, b) } else { (b, a) };
        let (im, ic) = sw(kin.0, kin.1);
        let (am0, ac0) = sw(ain.0, ain.1);
        let (am, ac) = (im.or(am0), ic.or(ac0));
        let (gap_m, gap_c) = if row {
            (f64::from(s.col_gap), f64::from(s.row_gap))
        } else {
            (f64::from(s.row_gap), f64::from(s.col_gap))
        };
        let wbase = kin.0.or(ain.0);
        let mut items: Vec<It> = vec![];
        let mut abs_kids = vec![];
        let mut zero_kids = vec![];
        for &c in self.kids(n) {
            let cs = self.st(c);
            if cs.display == Display::None {
                zero_kids.push(c);
                continue;
            }
            if cs.position == Position::Absolute {
                abs_kids.push(c);
                continue;
            }
            let mut m = [0.0; 4];
            let mut auto = [false; 4];
            for k in 0..4 {
                let (v, a) = marg(cs.margin[k], wbase);
                m[k] = v;
                auto[k] = a;
            }
            let (mmain, mcross) = if row {
                (m[1] + m[3], m[0] + m[2])
            } else {
                (m[0] + m[2], m[1] + m[3])
            };
            let _ = mmain;
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
            let (ciw, cih) = ins(cs);
            let ins_m = if row { ciw } else { cih };
            let auto_c = if row {
                auto[0] || auto[2]
            } else {
                auto[1] || auto[3]
            };
            let al = match cs.align_self {
                AlignSelf::Auto => s.align_items,
                AlignSelf::Stretch => AlignItems::Stretch,
                AlignSelf::Start => AlignItems::Start,
                AlignSelf::Center => AlignItems::Center,
                AlignSelf::End => AlignItems::End,
                AlignSelf::Baseline => AlignItems::Baseline,
            };
            let stretch = al == AlignItems::Stretch && matches!(ccs, Dim::Auto) && !auto_c;
            let basis = match cs.basis {
                Dim::Px(v) => Some(f64::from(v)),
                Dim::Pct(p) => im.map(|x| x * f64::from(p) / 100.0),
                Dim::Auto => None,
            };
            let base = basis.or(dim(cms, im)).unwrap_or_else(|| {
                let kc = if stretch {
                    ic.map(|c| (c - mcross).max(0.0))
                } else {
                    None
                };
                let acx = ac.map(|a| (a - mcross).max(0.0));
                let (sz_w, sz_h) = self.size(c, sw(None, kc), sw(None, acx));
                if row { sz_w } else { sz_h }
            });
            let base = base.max(ins_m);
            let min = dim(cmin, im).unwrap_or(0.0).max(ins_m);
            let max = dim(cmax, im).unwrap_or(f64::INFINITY);
            let hyp = base.min(max).max(min);
            items.push(It {
                n: c,
                m,
                auto,
                base,
                min,
                max,
                hyp,
                target: hyp,
                cross: 0.0,
                grow: f64::from(cs.grow),
                shrink: f64::from(cs.shrink),
            });
        }
        let main_marg = |i: &It| {
            if row {
                i.m[1] + i.m[3]
            } else {
                i.m[0] + i.m[2]
            }
        };
        let cross_marg = |i: &It| {
            if row {
                i.m[0] + i.m[2]
            } else {
                i.m[1] + i.m[3]
            }
        };
        // Lines.
        let wrap = s.wrap == Wrap::Wrap;
        let limit = im.or(am);
        let mut lines: Vec<(usize, usize)> = vec![];
        let mut start = 0;
        let mut sum = 0.0;
        for (k, it) in items.iter().enumerate() {
            let o = it.hyp + main_marg(it);
            let brk = wrap && k > start && limit.is_some_and(|l| sum + gap_m + o > l + 0.001);
            if brk {
                lines.push((start, k));
                start = k;
                sum = o;
            } else {
                sum += if k > start { gap_m } else { 0.0 } + o;
            }
        }
        if !items.is_empty() {
            lines.push((start, items.len()));
        }
        let line_sum = |a: usize, b: usize, it: &[It]| -> f64 {
            it[a..b].iter().map(|i| i.hyp + main_marg(i)).sum::<f64>()
                + gap_m * (b - a).saturating_sub(1) as f64
        };
        let max_sum = lines
            .iter()
            .map(|(a, b)| line_sum(*a, *b, &items))
            .fold(0.0, f64::max);
        let cont_m = im.unwrap_or(max_sum);
        let no_shrink = matches!(s.overflow, Overflow::Scroll | Overflow::Auto);
        let mut line_cross: Vec<f64> = vec![];
        for (a, b) in &lines {
            let gaps = gap_m * (b - a).saturating_sub(1) as f64;
            let mm: f64 = items[*a..*b].iter().map(main_marg).sum();
            Self::resolve(&mut items[*a..*b], cont_m - gaps - mm, no_shrink);
            let mut lc = 0.0f64;
            for it in items[*a..*b].iter_mut() {
                let mc_ = cross_marg(it);
                let acx = ac.map(|x| (x - mc_).max(0.0));
                let (kw, kh) = sw(Some(it.target), None);
                let (aw, ah) = sw(None, acx);
                let (w, h) = self.size(it.n, (kw, kh), (aw, ah));
                it.cross = if row { h } else { w };
                lc = lc.max(it.cross + mc_);
            }
            line_cross.push(lc);
        }
        let single = !wrap || lines.len() <= 1;
        if single
            && !lines.is_empty()
            && let Some(c) = ic
        {
            line_cross[0] = c;
        }
        let nl = lines.len();
        let sum_cross: f64 = line_cross.iter().sum::<f64>() + gap_c * nl.saturating_sub(1) as f64;
        let cont_c = ic.unwrap_or(sum_cross);
        let content = if row {
            (cont_m, cont_c)
        } else {
            (cont_c, cont_m)
        };
        let Some(nsize) = arrange else {
            return Flex {
                content,
                placed: vec![],
                extent: (0.0, 0.0),
            };
        };
        let mut lead_c = 0.0;
        let mut between_c = gap_c;
        let free_c = cont_c - sum_cross;
        if (!single || (wrap && nl > 1)) && free_c > 0.0 {
            match s.align_content {
                AlignContent::Stretch => {
                    for lc in line_cross.iter_mut() {
                        *lc += free_c / nl as f64;
                    }
                }
                AlignContent::Start => {}
                AlignContent::Center => lead_c = free_c / 2.0,
                AlignContent::End => lead_c = free_c,
                AlignContent::SpaceBetween => {
                    if nl > 1 {
                        between_c += free_c / (nl - 1) as f64;
                    }
                }
                AlignContent::SpaceAround => {
                    lead_c = free_c / (2 * nl) as f64;
                    between_c += free_c / nl as f64;
                }
            }
        }
        let origin = (
            f64::from(s.border_width + s.padding[3]),
            f64::from(s.border_width + s.padding[0]),
        );
        let mut placed = vec![];
        let (mut ex, mut ey) = (0.0f64, 0.0f64);
        let mut cp = lead_c;
        for (li, (a, b)) in lines.iter().enumerate() {
            let lcross = line_cross[li];
            let cnt = b - a;
            let logical = |i: &It| -> (f64, f64, bool, bool) {
                if row {
                    if rev {
                        (i.m[1], i.m[3], i.auto[1], i.auto[3])
                    } else {
                        (i.m[3], i.m[1], i.auto[3], i.auto[1])
                    }
                } else if rev {
                    (i.m[2], i.m[0], i.auto[2], i.auto[0])
                } else {
                    (i.m[0], i.m[2], i.auto[0], i.auto[2])
                }
            };
            let mut used = gap_m * cnt.saturating_sub(1) as f64;
            let mut autos = 0;
            for i in &items[*a..*b] {
                let (ms, me, as_, ae) = logical(i);
                used += i.target + ms + me;
                autos += usize::from(as_) + usize::from(ae);
            }
            let free = cont_m - used;
            let (mut off, mut space, mut share) = (0.0, gap_m, 0.0);
            if autos > 0 && free > 0.0 {
                share = free / autos as f64;
            } else if free > 0.0 {
                match s.justify {
                    Justify::Start => {}
                    Justify::Center => off = free / 2.0,
                    Justify::End => off = free,
                    Justify::SpaceBetween => {
                        if cnt > 1 {
                            space += free / (cnt - 1) as f64;
                        }
                    }
                    Justify::SpaceAround => {
                        off = free / (2 * cnt) as f64;
                        space += free / cnt as f64;
                    }
                    Justify::SpaceEvenly => {
                        off = free / (cnt + 1) as f64;
                        space += free / (cnt + 1) as f64;
                    }
                }
            }
            let mut p = off;
            for i in &items[*a..*b] {
                let cs = self.st(i.n);
                let (ms, me, as_, ae) = logical(i);
                let ms = ms + if as_ { share } else { 0.0 };
                let me = me + if ae { share } else { 0.0 };
                let mut pos = p + ms;
                p += ms + i.target + me + space;
                if rev {
                    pos = cont_m - pos - i.target;
                }
                let (mt, mb, at, ab) = if row {
                    (i.m[0], i.m[2], i.auto[0], i.auto[2])
                } else {
                    (i.m[3], i.m[1], i.auto[3], i.auto[1])
                };
                let al = match cs.align_self {
                    AlignSelf::Auto => s.align_items,
                    AlignSelf::Stretch => AlignItems::Stretch,
                    AlignSelf::Start => AlignItems::Start,
                    AlignSelf::Center => AlignItems::Center,
                    AlignSelf::End => AlignItems::End,
                    AlignSelf::Baseline => AlignItems::Baseline,
                };
                let ccs = if row { cs.height } else { cs.width };
                let mut cross = i.cross;
                if al == AlignItems::Stretch && matches!(ccs, Dim::Auto) && !at && !ab {
                    let (cmin, cmax, insx) = if row {
                        (cs.min_height, cs.max_height, ins(cs).1)
                    } else {
                        (cs.min_width, cs.max_width, ins(cs).0)
                    };
                    cross = clampd((lcross - mt - mb).max(0.0), cmin, cmax, ic, insx);
                }
                let spare = lcross - cross - mt - mb;
                let mut cpos = mt;
                if (at || ab) && spare > 0.0 {
                    let sh = spare / (usize::from(at) + usize::from(ab)) as f64;
                    if at {
                        cpos += sh;
                    }
                } else if spare > 0.0 {
                    match al {
                        AlignItems::Center => cpos += spare / 2.0,
                        AlignItems::End => cpos += spare,
                        _ => {}
                    }
                }
                cpos += cp;
                let (x, y, w, h) = if row {
                    (pos, cpos, i.target, cross)
                } else {
                    (cpos, pos, cross, i.target)
                };
                let (rx, ry) = (origin.0 + x, origin.1 + y);
                ex = ex.max(rx + w + i.m[1]);
                ey = ey.max(ry + h + i.m[2]);
                placed.push((i.n, rx, ry, w, h));
            }
            cp += lcross + between_c;
        }
        // Absolute children.
        let b = f64::from(s.border_width);
        let (cbw, cbh) = ((nsize.0 - 2.0 * b).max(0.0), (nsize.1 - 2.0 * b).max(0.0));
        for c in abs_kids {
            let cs = self.st(c);
            let top = dim(cs.inset[0], Some(cbh));
            let right = dim(cs.inset[1], Some(cbw));
            let bottom = dim(cs.inset[2], Some(cbh));
            let left = dim(cs.inset[3], Some(cbw));
            let mt = marg(cs.margin[0], Some(cbw)).0;
            let mr = marg(cs.margin[1], Some(cbw)).0;
            let mb = marg(cs.margin[2], Some(cbw)).0;
            let ml = marg(cs.margin[3], Some(cbw)).0;
            let kw = dim(cs.width, Some(cbw)).or_else(|| match (left, right) {
                (Some(l), Some(r)) => Some((cbw - l - r - ml - mr).max(0.0)),
                _ => None,
            });
            let kh = dim(cs.height, Some(cbh)).or_else(|| match (top, bottom) {
                (Some(t_), Some(b_)) => Some((cbh - t_ - b_ - mt - mb).max(0.0)),
                _ => None,
            });
            let (w, h) = self.size(
                c,
                (kw, kh),
                (
                    Some((cbw - ml - mr).max(0.0)),
                    Some((cbh - mt - mb).max(0.0)),
                ),
            );
            let x = if let Some(l) = left {
                b + l + ml
            } else if let Some(r) = right {
                b + cbw - r - mr - w
            } else {
                b + f64::from(s.padding[3]) + ml
            };
            let y = if let Some(t_) = top {
                b + t_ + mt
            } else if let Some(b_) = bottom {
                b + cbh - b_ - mb - h
            } else {
                b + f64::from(s.padding[0]) + mt
            };
            placed.push((c, x, y, w, h));
        }
        let _ = zero_kids;
        Flex {
            content,
            placed,
            extent: (
                ex + f64::from(s.padding[1]) + b,
                ey + f64::from(s.padding[2]) + b,
            ),
        }
    }

    /// Window-space rectangle of every node.
    pub fn layout(&self, vw: f64, vh: f64) -> Vec<(f64, f64, f64, f64)> {
        let mut out = vec![(0.0, 0.0, 0.0, 0.0); self.t.nodes.len()];
        out[0] = (0.0, 0.0, vw, vh);
        self.arrange(0, (vw, vh), (0.0, 0.0), &mut out);
        out
    }

    fn arrange(
        &self,
        n: u32,
        size: (f64, f64),
        at: (f64, f64),
        out: &mut Vec<(f64, f64, f64, f64)>,
    ) {
        let nd = &self.t.nodes[n as usize];
        if nd.leaf != Leaf::None {
            return;
        }
        let s = &nd.style;
        let (iw, ih) = ins(s);
        let inner = (Some((size.0 - iw).max(0.0)), Some((size.1 - ih).max(0.0)));
        let fx = self.flex(n, inner, inner, Some(size));
        for (c, x, y, w, h) in fx.placed {
            out[c as usize] = (at.0 + x, at.1 + y, w, h);
            self.arrange(c, (w, h), (at.0 + x, at.1 + y), out);
        }
    }
}

pub fn close(a: f64, b: f32) -> bool {
    let b = f64::from(b);
    (a - b).abs() <= 0.02 + 1e-4 * a.abs().max(b.abs())
}

pub fn first_diff(
    t: &TTree,
    oracle: &[(f64, f64, f64, f64)],
    got: &[Rect],
    reach: &dyn Fn(u32) -> bool,
) -> Option<String> {
    for (i, o) in oracle.iter().enumerate() {
        if !reach(i as u32) {
            continue;
        }
        let g = got[i];
        if !(close(o.0, g.x) && close(o.1, g.y) && close(o.2, g.w) && close(o.3, g.h)) {
            let _ = t;
            return Some(format!(
                "node {i}: oracle ({:.3},{:.3},{:.3},{:.3}) engine ({:.3},{:.3},{:.3},{:.3})",
                o.0, o.1, o.2, o.3, g.x, g.y, g.w, g.h
            ));
        }
    }
    None
}

/// Nodes laid out: reachable from the root through nodes that are not display:none.
pub fn reachable(t: &TTree) -> Vec<bool> {
    let mut seen = vec![false; t.nodes.len()];
    fn go(t: &TTree, n: u32, seen: &mut Vec<bool>) {
        seen[n as usize] = true;
        if t.nodes[n as usize].style.display == Display::None {
            return;
        }
        for &k in &t.nodes[n as usize].kids {
            go(t, k, seen);
        }
    }
    go(t, 0, &mut seen);
    // A display:none node itself has an empty rectangle: not compared (the engine leaves it zero).
    for (i, nd) in t.nodes.iter().enumerate() {
        if nd.style.display == Display::None {
            seen[i] = false;
        }
    }
    // Descendants of a display:none node are not reachable either.
    fn hide(t: &TTree, n: u32, seen: &mut Vec<bool>, dead: bool) {
        let dead = dead || t.nodes[n as usize].style.display == Display::None;
        if dead {
            seen[n as usize] = false;
        }
        for &k in &t.nodes[n as usize].kids {
            hide(t, k, seen, dead);
        }
    }
    hide(t, 0, &mut seen, false);
    seen
}
