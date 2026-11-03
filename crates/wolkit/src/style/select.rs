// SPDX-License-Identifier: BSD-2-Clause
//! The style table and the cascade. Resolution order is fixed (no specificity, no `!important`):
//! rules of the element kind, then the classes in the order given (later wins), then state and
//! media variants, then inline overrides. Results are interned (`StyleStore`) and memoised on
//! (parent style, kind, classes, state, media), so flipping `:hover` re-resolves one node.
use super::prop::*;
use super::value::*;
use std::collections::HashMap;

/// Interaction states, one bit each (`StateMask`).
pub const STATES: [&str; 9] = [
    "hover",
    "active",
    "focus",
    "focus-visible",
    "disabled",
    "selected",
    "checked",
    "expanded",
    "empty",
];
/// Media conditions, one bit each (`MediaMask`).
pub const MEDIA: [&str; 6] = [
    "dark",
    "light",
    "contrast",
    "reduce-motion",
    "reduce-transparency",
    "compact",
];
/// Node kinds a rule can name (`button { ... }`), in id order.
pub const KINDS: [&str; 14] = [
    "box",
    "text",
    "image",
    "spacer",
    "divider",
    "button",
    "toggle",
    "slider",
    "text-input",
    "scroll",
    "vlist",
    "canvas",
    "surface",
    "overlay",
];

pub const MEDIA_DARK: u8 = 1;
pub const MEDIA_LIGHT: u8 = 2;
pub const MEDIA_CONTRAST: u8 = 4;
pub const STATE_HOVER: u16 = 1;

pub fn state_bit(name: &str) -> Option<u16> {
    STATES.iter().position(|s| *s == name).map(|i| 1u16 << i)
}
pub fn media_bit(name: &str) -> Option<u8> {
    MEDIA.iter().position(|s| *s == name).map(|i| 1u8 << i)
}
pub fn kind_id(name: &str) -> Option<u8> {
    KINDS.iter().position(|s| *s == name).map(|i| i as u8)
}

/// Limits of a style table.
pub const MAX_CLASSES: usize = 4096;
pub const MAX_VARIANTS: usize = 8;
pub const MAX_TABLE_BYTES: usize = 4 << 20;
pub const MAX_CLASS_NAME: usize = 64;
/// Classes a node can carry (`SET_STYLE`).
pub const MAX_NODE_CLASSES: usize = 8;

/// `.parent:state > .child`: the variant applies when the parent has the class and the state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParentSel {
    pub class: u32,
    pub state: u16,
}

/// One block of a class: where it applies and what it sets.
#[derive(Clone, Debug, PartialEq)]
pub struct Variant {
    /// 0 or one state bit.
    pub state: u16,
    pub media: u8,
    pub parent: Option<ParentSel>,
    pub decls: Decls,
}

impl Variant {
    pub fn is_base(&self) -> bool {
        self.state == 0 && self.media == 0 && self.parent.is_none()
    }
    /// Variants sort base first, then by how many conditions they carry.
    pub fn weight(&self) -> u32 {
        self.state.count_ones() + self.media.count_ones() + u32::from(self.parent.is_some()) * 2
    }
    pub fn key(&self) -> (u16, u8, Option<ParentSel>) {
        (self.state, self.media, self.parent)
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct ClassDef {
    pub name: String,
    pub variants: Vec<Variant>,
}

impl ClassDef {
    pub fn has_parent_rules(&self) -> bool {
        self.variants.iter().any(|v| v.parent.is_some())
    }
    /// Keep variants in resolution order (stable).
    pub fn sort(&mut self) {
        self.variants.sort_by_key(|v| v.weight());
    }
}

/// A loaded style table: one rule list per node kind and the named classes.
#[derive(Clone, Debug, PartialEq)]
pub struct StyleTable {
    pub kinds: Vec<ClassDef>,
    pub classes: Vec<ClassDef>,
}

impl Default for StyleTable {
    fn default() -> Self {
        StyleTable {
            kinds: KINDS
                .iter()
                .map(|k| ClassDef {
                    name: k.to_string(),
                    variants: vec![],
                })
                .collect(),
            classes: vec![],
        }
    }
}

impl StyleTable {
    pub fn class_id(&self, name: &str) -> Option<u32> {
        self.classes
            .iter()
            .position(|c| c.name == name)
            .map(|i| i as u32)
    }
}

/// Index of an interned computed style. 0 is the default style.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct StyleId(pub u32);

/// Interned computed styles: equal styles share an id, so equality is an integer compare.
pub struct StyleStore {
    styles: Vec<Props>,
    index: HashMap<Vec<u8>, u32>,
    scratch: Vec<u8>,
}

impl Default for StyleStore {
    fn default() -> Self {
        let mut s = StyleStore {
            styles: vec![],
            index: HashMap::new(),
            scratch: vec![],
        };
        s.intern(&Props::default());
        s
    }
}

fn encode_all(p: &Props, out: &mut Vec<u8>) {
    let mut w = Wr(std::mem::take(out));
    w.0.clear();
    for id in PropId::ALL {
        put_prop(*id, p, &mut w);
    }
    *out = w.0;
}

impl StyleStore {
    pub fn len(&self) -> usize {
        self.styles.len()
    }
    pub fn is_empty(&self) -> bool {
        self.styles.is_empty()
    }
    pub fn get(&self, id: StyleId) -> &Props {
        &self.styles[id.0 as usize]
    }
    pub fn intern(&mut self, p: &Props) -> StyleId {
        let mut key = std::mem::take(&mut self.scratch);
        encode_all(p, &mut key);
        let id = match self.index.get(&key) {
            Some(i) => *i,
            None => {
                let i = self.styles.len() as u32;
                self.styles.push(p.clone());
                self.index.insert(key.clone(), i);
                i
            }
        };
        self.scratch = key;
        StyleId(id)
    }
}

/// Everything the cascade looks at for one node.
#[derive(Clone, Copy, Debug)]
pub struct Input<'a> {
    pub kind: u8,
    pub classes: &'a [u32],
    pub state: u16,
    pub media: u8,
    pub parent: Option<StyleId>,
    pub parent_classes: &'a [u32],
    pub parent_state: u16,
    pub inline: Option<&'a Decls>,
    /// The person's text-size preference; scales every declared `font-size`.
    pub text_scale: f32,
}

impl Default for Input<'_> {
    fn default() -> Self {
        Input {
            kind: 0,
            classes: &[],
            state: 0,
            media: 0,
            parent: None,
            parent_classes: &[],
            parent_state: 0,
            inline: None,
            text_scale: 1.0,
        }
    }
}

/// The set of properties that differ between two styles.
pub fn diff(a: &Props, b: &Props) -> u128 {
    let mut m = 0u128;
    for id in PropId::ALL {
        if !eq_prop(*id, a, b) {
            m |= id.bit();
        }
    }
    m
}

/// True when the changed properties include one that needs a new layout.
pub fn needs_layout(changed: u128) -> bool {
    changed & layout_mask() != 0
}

#[derive(Default)]
pub struct Resolver {
    memo: HashMap<Box<[u32]>, StyleId>,
    key: Vec<u32>,
    pub hits: u64,
    pub misses: u64,
}

impl Resolver {
    /// Forget memoised results (after the table or a media preference changed).
    pub fn clear(&mut self) {
        self.memo.clear();
    }

    pub fn resolve(&mut self, table: &StyleTable, store: &mut StyleStore, inp: &Input) -> StyleId {
        let parent_rules = inp.classes.iter().any(|c| {
            table
                .classes
                .get(*c as usize)
                .is_some_and(|d| d.has_parent_rules())
        });
        if inp.inline.is_none() {
            self.key.clear();
            self.key.push(u32::from(inp.kind));
            self.key.push(inp.parent.map_or(0, |p| p.0 + 1));
            self.key
                .push(u32::from(inp.state) | (u32::from(inp.media) << 16));
            self.key.push(inp.text_scale.to_bits());
            self.key.push(inp.classes.len() as u32);
            self.key.extend_from_slice(inp.classes);
            if parent_rules {
                self.key.push(u32::from(inp.parent_state));
                self.key.extend_from_slice(inp.parent_classes);
            }
            if let Some(id) = self.memo.get(&self.key[..]) {
                self.hits += 1;
                return *id;
            }
        }
        self.misses += 1;
        let props = cascade(table, store, inp);
        let id = store.intern(&props);
        if inp.inline.is_none() {
            self.memo.insert(self.key.clone().into_boxed_slice(), id);
        }
        id
    }
}

fn matches(v: &Variant, inp: &Input) -> bool {
    if v.state & !inp.state != 0 || v.media & !inp.media != 0 {
        return false;
    }
    match v.parent {
        None => true,
        Some(ps) => inp.parent_classes.contains(&ps.class) && ps.state & !inp.parent_state == 0,
    }
}

/// Rules every node kind starts from, below the app's table: spacers grow, scrollers scroll.
pub fn builtin_kinds() -> &'static [ClassDef] {
    static B: std::sync::OnceLock<Vec<ClassDef>> = std::sync::OnceLock::new();
    B.get_or_init(|| {
        let mut v: Vec<ClassDef> = KINDS
            .iter()
            .map(|k| ClassDef {
                name: (*k).to_string(),
                variants: vec![],
            })
            .collect();
        let mut set = |kind: &str, f: &dyn Fn(&mut Props), ids: &[PropId]| {
            let mut d = Decls::default();
            f(&mut d.p);
            for id in ids {
                d.mask |= id.bit();
            }
            if let Some(k) = kind_id(kind) {
                v[k as usize].variants.push(Variant {
                    state: 0,
                    media: 0,
                    parent: None,
                    decls: d,
                });
            }
        };
        set("spacer", &|p| p.layout.grow = 1.0, &[PropId::Grow]);
        for k in ["scroll", "vlist"] {
            set(
                k,
                &|p| {
                    p.layout.overflow = Overflow::Auto;
                    p.layout.direction = Direction::Column;
                },
                &[PropId::Overflow, PropId::Direction],
            );
        }
        v
    })
}

fn apply(p: &mut Props, d: &Decls, scale: f32) {
    apply_mask(p, &d.p, d.mask);
    if d.mask & PropId::FontSize.bit() != 0 {
        p.text.size *= scale;
    }
}

/// The cascade without memo or interning: the reference for the resolver.
pub fn cascade(table: &StyleTable, store: &StyleStore, inp: &Input) -> Props {
    let mut p = Props::default();
    p.text.size *= inp.text_scale;
    if let Some(par) = inp.parent {
        let pp = store.get(par);
        for id in inherited_ids() {
            copy_prop(id, &mut p, pp);
        }
    }
    // Kind rules, then up to eight classes: a fixed array, so a restyle allocates nothing.
    let empty = ClassDef::default();
    let mut layers: [&ClassDef; 2 + MAX_NODE_CLASSES] = [&empty; 2 + MAX_NODE_CLASSES];
    let mut n = 0;
    if let Some(k) = builtin_kinds().get(inp.kind as usize) {
        layers[n] = k;
        n += 1;
    }
    if let Some(k) = table.kinds.get(inp.kind as usize) {
        layers[n] = k;
        n += 1;
    }
    for c in inp.classes.iter().take(MAX_NODE_CLASSES) {
        if let Some(d) = table.classes.get(*c as usize) {
            layers[n] = d;
            n += 1;
        }
    }
    let layers = &layers[..n];
    // Base blocks of the kind and every class first, then the conditional variants.
    for l in layers {
        for v in l.variants.iter().filter(|v| v.is_base()) {
            apply(&mut p, &v.decls, inp.text_scale);
        }
    }
    for l in layers {
        for v in l
            .variants
            .iter()
            .filter(|v| !v.is_base() && matches(v, inp))
        {
            apply(&mut p, &v.decls, inp.text_scale);
        }
    }
    if let Some(d) = inp.inline {
        apply(&mut p, d, inp.text_scale);
    }
    p
}
