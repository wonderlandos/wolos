// SPDX-License-Identifier: BSD-2-Clause
//! The scene: a struct-of-arrays arena indexed by slot. Everything the pipeline reads about a node
//! is a column here; nothing is boxed per node. Applying a patch (`apply.rs`) is the only way an
//! app changes it; the pipeline stages write back only what they computed ([`SceneWrite`]).

use super::ids::{Idx, MAX_NODES, NIL, NodeId};
use super::kind::{Kind, dirty, state};
use super::patch::{MAX_CLASSES, PropValue};
use super::prop::MAX_WORDS;
use super::read::{A11y, Rect, SceneRead, SceneWrite};
use super::strings::{NO_STR, StrId, Strings};

/// A pending animation request from an `ANIM` op, drained by the animation clock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimReq {
    pub node: NodeId,
    pub prop: u16,
    pub target: f32,
    pub mode: u32,
    pub param: u32,
}

#[derive(Clone, Copy)]
pub(crate) struct PropEntry {
    pub next: u32,
    pub prop: u16,
    pub n: u8,
    pub v: [u32; MAX_WORDS],
}

pub struct Scene {
    pub(crate) live: Vec<bool>,
    pub(crate) gen_: Vec<u8>,
    pub(crate) kind: Vec<u8>,
    pub(crate) parent: Vec<Idx>,
    pub(crate) first: Vec<Idx>,
    pub(crate) last: Vec<Idx>,
    pub(crate) next: Vec<Idx>,
    pub(crate) prev: Vec<Idx>,
    pub(crate) nchild: Vec<u32>,
    /// Upper bound of the subtree height (a leaf is 1); exact after a recount.
    pub(crate) height: Vec<u8>,
    pub(crate) dirty: Vec<u8>,
    pub(crate) state: Vec<u32>,
    pub(crate) classes: Vec<[u32; MAX_CLASSES]>,
    pub(crate) nclass: Vec<u8>,
    pub(crate) text: Vec<StrId>,
    pub(crate) key: Vec<u64>,
    pub(crate) events: Vec<u32>,
    pub(crate) role: Vec<u8>,
    pub(crate) name: Vec<StrId>,
    pub(crate) desc: Vec<StrId>,
    pub(crate) prop_head: Vec<u32>,
    pub(crate) computed: Vec<u32>,
    pub(crate) rect: Vec<Rect>,
    /// Marked by FREE inside a patch being applied; the slot is released at commit.
    pub(crate) doomed: Vec<bool>,
    pub(crate) props: Vec<PropEntry>,
    pub(crate) prop_free: u32,
    pub(crate) strings: Strings,
    pub(crate) focus: Idx,
    pub(crate) count: usize,
    pub(crate) version: u64,
    pub(crate) anims: Vec<AnimReq>,
    pub(crate) last_seq: u32,
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}

macro_rules! columns {
    ($self:ident, $n:ident, $($col:ident = $v:expr),* $(,)?) => {
        $($self.$col.resize($n, $v);)*
    };
}

impl Scene {
    pub fn new() -> Scene {
        let mut s = Scene {
            live: Vec::new(),
            gen_: Vec::new(),
            kind: Vec::new(),
            parent: Vec::new(),
            first: Vec::new(),
            last: Vec::new(),
            next: Vec::new(),
            prev: Vec::new(),
            nchild: Vec::new(),
            height: Vec::new(),
            dirty: Vec::new(),
            state: Vec::new(),
            classes: Vec::new(),
            nclass: Vec::new(),
            text: Vec::new(),
            key: Vec::new(),
            events: Vec::new(),
            role: Vec::new(),
            name: Vec::new(),
            desc: Vec::new(),
            prop_head: Vec::new(),
            computed: Vec::new(),
            rect: Vec::new(),
            doomed: Vec::new(),
            props: Vec::new(),
            prop_free: NIL,
            strings: Strings::new(),
            focus: NIL,
            count: 0,
            version: 0,
            anims: Vec::new(),
            last_seq: 0,
        };
        s.ensure(0);
        s.live[0] = true;
        s.kind[0] = Kind::Box as u8;
        s.height[0] = 1;
        s.count = 1;
        s
    }

    /// Make room for `slot`. Columns grow together, 64 slots at a time.
    pub(crate) fn ensure(&mut self, slot: usize) {
        if slot < self.live.len() {
            return;
        }
        let n = (slot + 1 + 63) & !63;
        columns!(self, n,
            live = false, gen_ = 0, kind = 0, parent = NIL, first = NIL, last = NIL, next = NIL,
            prev = NIL, nchild = 0, height = 1, dirty = 0, state = 0, classes = [0; MAX_CLASSES],
            nclass = 0, text = NO_STR, key = 0, events = 0, role = 0, name = NO_STR,
            desc = NO_STR, prop_head = NIL, computed = 0, rect = Rect::default(), doomed = false);
    }

    pub fn strings(&self) -> &super::strings::Strings {
        &self.strings
    }

    /// Anything the animation clock has to start.
    pub fn take_anims(&mut self, out: &mut Vec<AnimReq>) {
        out.append(&mut self.anims);
    }

    /// Sequence number of the last patch applied (echoed in events).
    pub fn last_seq(&self) -> u32 {
        self.last_seq
    }

    /// Release spare memory (Memory Saver): compact text, shrink columns past the live slots.
    pub fn trim(&mut self) {
        self.strings.trim();
        self.props.shrink_to_fit();
        self.anims.shrink_to_fit();
    }

    /// Approximate bytes held, for `wolkit_engine_stats_json` and the memory budget test.
    pub fn memory(&self) -> usize {
        let n = self.live.len();
        n * (1 + 1 + 1 + 5 * 4 + 4 + 1 + 1 + 4 + 4 + 32 + 1 + 4 + 8 + 4 + 1 + 4 + 4 + 4 + 4 + 4 + 16 + 1)
            + self.props.len() * std::mem::size_of::<PropEntry>()
            + self.strings.bytes()
    }

    pub(crate) fn mark(&mut self, i: Idx, bits: u8) {
        let i = i as usize;
        if i >= self.dirty.len() {
            return;
        }
        self.dirty[i] |= bits;
        let mut p = self.parent[i];
        while p != NIL {
            let d = &mut self.dirty[p as usize];
            if *d & dirty::SUBTREE != 0 {
                break;
            }
            *d |= dirty::SUBTREE;
            p = self.parent[p as usize];
        }
    }

    // ---- inline property chain ----

    pub(crate) fn find_prop(&self, i: Idx, prop: u16) -> Option<u32> {
        let mut e = *self.prop_head.get(i as usize)?;
        while e != NIL {
            let pe = &self.props[e as usize];
            if pe.prop == prop {
                return Some(e);
            }
            e = pe.next;
        }
        None
    }

    /// Set a property; the previous value, if any.
    pub(crate) fn put_prop(&mut self, i: Idx, prop: u16, value: &PropValue) -> Option<PropValue> {
        if let Some(e) = self.find_prop(i, prop) {
            let pe = &mut self.props[e as usize];
            let old = PropValue { n: pe.n, w: pe.v };
            pe.n = value.n;
            pe.v = value.w;
            return Some(old);
        }
        let entry = PropEntry { next: NIL, prop, n: value.n, v: value.w };
        let idx = if self.prop_free != NIL {
            let f = self.prop_free;
            self.prop_free = self.props[f as usize].next;
            self.props[f as usize] = entry;
            f
        } else {
            self.props.push(entry);
            (self.props.len() - 1) as u32
        };
        // Append at the tail so iteration is in first-set order.
        let head = self.prop_head[i as usize];
        if head == NIL {
            self.prop_head[i as usize] = idx;
        } else {
            let mut t = head;
            while self.props[t as usize].next != NIL {
                t = self.props[t as usize].next;
            }
            self.props[t as usize].next = idx;
        }
        None
    }

    pub(crate) fn drop_prop(&mut self, i: Idx, prop: u16) {
        let mut prev = NIL;
        let mut e = self.prop_head[i as usize];
        while e != NIL {
            let next = self.props[e as usize].next;
            if self.props[e as usize].prop == prop {
                if prev == NIL {
                    self.prop_head[i as usize] = next;
                } else {
                    self.props[prev as usize].next = next;
                }
                self.props[e as usize].next = self.prop_free;
                self.prop_free = e;
                return;
            }
            prev = e;
            e = next;
        }
    }

    pub(crate) fn drop_all_props(&mut self, i: Idx) {
        let mut e = self.prop_head[i as usize];
        while e != NIL {
            let next = self.props[e as usize].next;
            self.props[e as usize].next = self.prop_free;
            self.prop_free = e;
            e = next;
        }
        self.prop_head[i as usize] = NIL;
    }

    // ---- links ----

    /// Detach `child` from its parent (no-op for a detached node).
    pub(crate) fn unlink(&mut self, child: Idx) {
        let c = child as usize;
        let p = self.parent[c];
        if p == NIL {
            return;
        }
        let (prev, next) = (self.prev[c], self.next[c]);
        if prev == NIL {
            self.first[p as usize] = next;
        } else {
            self.next[prev as usize] = next;
        }
        if next == NIL {
            self.last[p as usize] = prev;
        } else {
            self.prev[next as usize] = prev;
        }
        self.nchild[p as usize] -= 1;
        self.parent[c] = NIL;
        self.prev[c] = NIL;
        self.next[c] = NIL;
        self.refresh_empty(p);
        self.mark(p, dirty::CHILDREN | dirty::LAYOUT | dirty::PAINT | dirty::A11Y);
    }

    /// Attach a detached `child` under `parent` before sibling `before` (NIL appends).
    pub(crate) fn link(&mut self, parent: Idx, child: Idx, before: Idx) {
        let (p, c) = (parent as usize, child as usize);
        let prev = if before == NIL { self.last[p] } else { self.prev[before as usize] };
        self.parent[c] = parent;
        self.prev[c] = prev;
        self.next[c] = before;
        if prev == NIL {
            self.first[p] = child;
        } else {
            self.next[prev as usize] = child;
        }
        if before == NIL {
            self.last[p] = child;
        } else {
            self.prev[before as usize] = child;
        }
        self.nchild[p] += 1;
        self.refresh_empty(parent);
        // Taller subtrees raise the bound of every ancestor.
        let (mut a, mut h) = (parent, self.height[c].saturating_add(1));
        while a != NIL && self.height[a as usize] < h {
            self.height[a as usize] = h;
            a = self.parent[a as usize];
            h = h.saturating_add(1);
        }
        self.mark(child, dirty::ALL);
        self.mark(parent, dirty::CHILDREN | dirty::LAYOUT | dirty::PAINT | dirty::A11Y);
    }

    fn refresh_empty(&mut self, p: Idx) {
        let s = &mut self.state[p as usize];
        if self.nchild[p as usize] == 0 {
            *s |= state::EMPTY;
        } else {
            *s &= !state::EMPTY;
        }
    }

    /// The sibling at `index` under `parent` (the one a new child goes before), NIL to append.
    pub(crate) fn nth_child(&self, parent: Idx, index: u32) -> Idx {
        if index >= self.nchild[parent as usize] {
            return NIL;
        }
        // From the nearer end.
        if index <= self.nchild[parent as usize] / 2 {
            let mut c = self.first[parent as usize];
            for _ in 0..index {
                c = self.next[c as usize];
            }
            c
        } else {
            let mut c = self.last[parent as usize];
            for _ in 0..(self.nchild[parent as usize] - 1 - index) {
                c = self.prev[c as usize];
            }
            c
        }
    }

    /// Distance from the root (the root is 0); None if `i` hangs in a detached subtree.
    pub(crate) fn depth_of(&self, i: Idx) -> Option<usize> {
        let (mut d, mut c) = (0, i);
        while c != 0 {
            c = self.parent[c as usize];
            if c == NIL || d > super::ids::MAX_DEPTH + 1 {
                return None;
            }
            d += 1;
        }
        Some(d)
    }

    /// Exact height of the subtree at `root` (a leaf is 1); stored as the new bound.
    pub(crate) fn recount_height(&mut self, root: Idx) -> usize {
        let (mut cur, mut d, mut best) = (root, 1usize, 1usize);
        loop {
            best = best.max(d);
            let c = self.first[cur as usize];
            if c != NIL {
                cur = c;
                d += 1;
                continue;
            }
            loop {
                if cur == root {
                    self.height[root as usize] = best.min(255) as u8;
                    return best;
                }
                let n = self.next[cur as usize];
                if n != NIL {
                    cur = n;
                    break;
                }
                cur = self.parent[cur as usize];
                d -= 1;
            }
        }
    }

    /// Whether `a` is `b` or one of its ancestors' descendants chain: is `anc` above `node`?
    pub(crate) fn is_ancestor(&self, anc: Idx, node: Idx) -> bool {
        let mut c = node;
        let mut guard = 0;
        while c != NIL && guard <= MAX_NODES {
            if c == anc {
                return true;
            }
            c = self.parent[c as usize];
            guard += 1;
        }
        false
    }

    /// Release one node's own data (not its links): strings, properties, flags.
    pub(crate) fn release_node(&mut self, i: Idx) {
        let n = i as usize;
        let (t, nm, d) = (self.text[n], self.name[n], self.desc[n]);
        self.strings.release(t);
        self.strings.release(nm);
        self.strings.release(d);
        // Properties holding a string (placeholder, tooltip, image source).
        let mut e = self.prop_head[n];
        while e != NIL {
            let pe = self.props[e as usize];
            if super::prop::def(u32::from(pe.prop)).is_some_and(|d| d.ty == super::prop::Ty::Str) {
                self.strings.release(pe.v[0]);
            }
            e = pe.next;
        }
        self.drop_all_props(i);
        self.live[n] = false;
        self.doomed[n] = false;
        self.text[n] = NO_STR;
        self.name[n] = NO_STR;
        self.desc[n] = NO_STR;
        self.state[n] = 0;
        self.nclass[n] = 0;
        self.key[n] = 0;
        self.events[n] = 0;
        self.role[n] = 0;
        self.computed[n] = 0;
        self.rect[n] = Rect::default();
        self.dirty[n] = 0;
        self.nchild[n] = 0;
        self.first[n] = NIL;
        self.last[n] = NIL;
        self.height[n] = 1;
        self.count -= 1;
        if self.focus == i {
            self.focus = NIL;
        }
    }

    /// Free `root` and everything below it. `root` is detached already.
    pub(crate) fn free_subtree(&mut self, root: Idx) {
        let mut cur = root;
        loop {
            while self.first[cur as usize] != NIL {
                cur = self.first[cur as usize];
            }
            let (parent, next) = (self.parent[cur as usize], self.next[cur as usize]);
            self.release_node(cur);
            self.parent[cur as usize] = NIL;
            self.next[cur as usize] = NIL;
            self.prev[cur as usize] = NIL;
            if cur == root || parent == NIL {
                break;
            }
            self.first[parent as usize] = next;
            if next != NIL {
                self.prev[next as usize] = NIL;
            }
            cur = parent;
        }
    }

    pub(crate) fn set_str(&mut self, slot: &mut StrId, text: &str) -> Option<bool> {
        let new = self.strings.intern(text)?;
        if new == *slot {
            // Same text: drop the extra reference the intern took.
            self.strings.release(new);
            return Some(false);
        }
        *slot = new;
        Some(true)
    }
}

impl SceneRead for Scene {
    fn capacity(&self) -> Idx {
        self.live.len() as Idx
    }
    fn is_live(&self, i: Idx) -> bool {
        self.live.get(i as usize).copied().unwrap_or(false)
    }
    fn id(&self, i: Idx) -> NodeId {
        NodeId::new(i, self.gen_.get(i as usize).copied().unwrap_or(0))
    }
    fn kind(&self, i: Idx) -> Kind {
        self.kind
            .get(i as usize)
            .and_then(|k| Kind::from_u32(u32::from(*k)))
            .unwrap_or(Kind::Box)
    }
    fn parent(&self, i: Idx) -> Idx {
        self.parent.get(i as usize).copied().unwrap_or(NIL)
    }
    fn first_child(&self, i: Idx) -> Idx {
        self.first.get(i as usize).copied().unwrap_or(NIL)
    }
    fn last_child(&self, i: Idx) -> Idx {
        self.last.get(i as usize).copied().unwrap_or(NIL)
    }
    fn next_sibling(&self, i: Idx) -> Idx {
        self.next.get(i as usize).copied().unwrap_or(NIL)
    }
    fn prev_sibling(&self, i: Idx) -> Idx {
        self.prev.get(i as usize).copied().unwrap_or(NIL)
    }
    fn child_count(&self, i: Idx) -> u32 {
        self.nchild.get(i as usize).copied().unwrap_or(0)
    }
    fn state(&self, i: Idx) -> u32 {
        self.state.get(i as usize).copied().unwrap_or(0)
    }
    fn classes(&self, i: Idx) -> &[u32] {
        match (self.classes.get(i as usize), self.nclass.get(i as usize)) {
            (Some(c), Some(n)) => &c[..(*n as usize).min(MAX_CLASSES)],
            _ => &[],
        }
    }
    fn text(&self, i: Idx) -> &str {
        self.strings.get(self.text.get(i as usize).copied().unwrap_or(NO_STR))
    }
    fn key(&self, i: Idx) -> u64 {
        self.key.get(i as usize).copied().unwrap_or(0)
    }
    fn event_mask(&self, i: Idx) -> u32 {
        self.events.get(i as usize).copied().unwrap_or(0)
    }
    fn a11y(&self, i: Idx) -> A11y<'_> {
        let n = i as usize;
        A11y {
            role: u32::from(self.role.get(n).copied().unwrap_or(0)),
            name: self.strings.get(self.name.get(n).copied().unwrap_or(NO_STR)),
            description: self.strings.get(self.desc.get(n).copied().unwrap_or(NO_STR)),
        }
    }
    fn prop(&self, i: Idx, prop: u16) -> Option<PropValue> {
        let e = self.find_prop(i, prop)?;
        let pe = &self.props[e as usize];
        Some(PropValue { n: pe.n, w: pe.v })
    }
    fn each_prop(&self, i: Idx, f: &mut dyn FnMut(u16, &PropValue)) {
        let Some(mut e) = self.prop_head.get(i as usize).copied() else {
            return;
        };
        while e != NIL {
            let pe = &self.props[e as usize];
            f(pe.prop, &PropValue { n: pe.n, w: pe.v });
            e = pe.next;
        }
    }
    fn dirty(&self, i: Idx) -> u8 {
        self.dirty.get(i as usize).copied().unwrap_or(0)
    }
    fn computed(&self, i: Idx) -> u32 {
        self.computed.get(i as usize).copied().unwrap_or(0)
    }
    fn rect(&self, i: Idx) -> Rect {
        self.rect.get(i as usize).copied().unwrap_or_default()
    }
    fn focus(&self) -> Idx {
        self.focus
    }
    fn version(&self) -> u64 {
        self.version
    }
    fn node_count(&self) -> usize {
        self.count
    }
    fn string(&self, id: u32) -> &str {
        self.strings.get(id)
    }
}

impl SceneWrite for Scene {
    fn set_computed(&mut self, i: Idx, style: u32) {
        if let Some(c) = self.computed.get_mut(i as usize) {
            *c = style;
        }
    }
    fn set_rect(&mut self, i: Idx, r: Rect) {
        if let Some(c) = self.rect.get_mut(i as usize) {
            *c = r;
        }
    }
    fn clear_dirty(&mut self, i: Idx, bits: u8) {
        if let Some(d) = self.dirty.get_mut(i as usize) {
            *d &= !bits;
        }
    }
    fn clear_subtree(&mut self, i: Idx) {
        if let Some(d) = self.dirty.get_mut(i as usize) {
            *d &= !dirty::SUBTREE;
        }
    }
    fn mark_dirty(&mut self, i: Idx, bits: u8) {
        self.mark(i, bits);
    }
    fn set_engine_state(&mut self, i: Idx, mask: u32, on: bool) {
        let Some(s) = self.state.get_mut(i as usize) else {
            return;
        };
        let was = *s;
        let mask = mask & state::ENGINE & !state::EMPTY;
        if on {
            *s |= mask;
        } else {
            *s &= !mask;
        }
        if *s != was {
            self.mark(i, dirty::STYLE | dirty::PAINT);
        }
    }
    fn set_prop(&mut self, i: Idx, prop: u16, value: &PropValue) {
        if self.is_live(i) {
            self.put_prop(i, prop, value);
            self.mark(i, dirty::PAINT);
        }
    }
}
