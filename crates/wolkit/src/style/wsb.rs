// SPDX-License-Identifier: BSD-2-Clause
//! `WSB1`, the compiled style table: what `wolkit-stylec` writes, what the app registers with one
//! `wolkit_style_table_load` call and what the engine's dev loader reads back. Little endian:
//!
//! ```text
//! "WSB1"  u16 version(1)  u16 flags(0)
//! u32 kinds(14)  u32 classes
//! kinds:   variants...                      (one list per node kind, in KINDS order)
//! classes: u8 name_len, name, variants...   (class id = position)
//! variants: u8 count(<=8), then each:
//!   u16 state  u8 media  u8 has_parent  [u32 parent_class  u16 parent_state]
//!   u128 mask  then the value of each set property in ascending id order
//! u32 FNV-1a of everything before it
//! ```
//! The loader treats the bytes as untrusted: every count is bounded before it is used, every value
//! is range checked, and nothing panics (`fuzz_targets/wolkit_wsb`).
use super::prop::{Decls, PropId, get_prop, put_prop};
use super::select::{
    ClassDef, KINDS, MAX_CLASS_NAME, MAX_CLASSES, MAX_TABLE_BYTES, MAX_VARIANTS, MEDIA, ParentSel,
    STATES, StyleTable, Variant,
};
use super::value::{Props, Rd, Wr};
use crate::scene::ids::{Ix, to_u8, to_u32};

pub const MAGIC: &[u8; 4] = b"WSB1";
pub const VERSION: u16 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WsbError {
    TooBig,
    Magic,
    Version(u16),
    Truncated,
    Checksum,
    Limit(&'static str),
    Name,
    Value(&'static str),
    Trailing,
}

impl std::fmt::Display for WsbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WsbError::TooBig => write!(f, "The style table is over {MAX_TABLE_BYTES} bytes."),
            WsbError::Magic => write!(
                f,
                "This is not a style table (it does not start with WSB1)."
            ),
            WsbError::Version(v) => write!(
                f,
                "This style table is version {v}; this engine reads version {VERSION}."
            ),
            WsbError::Truncated => write!(f, "The style table ends too early."),
            WsbError::Checksum => write!(f, "The style table does not match its checksum."),
            WsbError::Limit(w) => write!(f, "The style table has too many {w}."),
            WsbError::Name => write!(
                f,
                "The style table has a class name that is not valid or not unique."
            ),
            WsbError::Value(w) => write!(f, "The style table has a {w} that is out of range."),
            WsbError::Trailing => write!(f, "The style table has bytes after its checksum."),
        }
    }
}

impl std::error::Error for WsbError {}

pub fn fnv1a(b: &[u8]) -> u32 {
    let mut h = 0x811c_9dc5u32;
    for x in b {
        h ^= u32::from(*x);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

fn put_variants(w: &mut Wr, defs: &[Variant]) {
    w.u8(to_u8(defs.len()));
    for v in defs {
        w.u16(v.state);
        w.u8(v.media);
        match v.parent {
            Some(p) => {
                w.u8(1);
                w.u32(p.class);
                w.u16(p.state);
            }
            None => w.u8(0),
        }
        w.u128(v.decls.mask);
        let mut m = v.decls.mask;
        while m != 0 {
            let b = to_u8(m.trailing_zeros());
            m &= m - 1;
            if let Some(id) = PropId::from_u8(b) {
                put_prop(id, &v.decls.p, w);
            }
        }
    }
}

/// Serialise a table. Fails only when it is over the size limit.
pub fn write(t: &StyleTable) -> Result<Vec<u8>, WsbError> {
    if t.classes.len() > MAX_CLASSES {
        return Err(WsbError::Limit("classes"));
    }
    let mut w = Wr::default();
    w.0.extend_from_slice(MAGIC);
    w.u16(VERSION);
    w.u16(0);
    w.u32(to_u32(t.kinds.len()));
    w.u32(to_u32(t.classes.len()));
    for k in &t.kinds {
        put_variants(&mut w, &k.variants);
    }
    for c in &t.classes {
        w.u8(to_u8(c.name.len()));
        w.0.extend_from_slice(c.name.as_bytes());
        put_variants(&mut w, &c.variants);
    }
    let sum = fnv1a(&w.0);
    w.u32(sum);
    if w.0.len() > MAX_TABLE_BYTES {
        return Err(WsbError::TooBig);
    }
    Ok(w.0)
}

/// Mask of the properties a transition may name.
pub fn anim_mask() -> u128 {
    PropId::ALL
        .iter()
        .filter(|p| p.info().anim)
        .fold(0, |m, p| m | p.bit())
}

fn get_variants(r: &mut Rd, n_classes: u32) -> Result<Vec<Variant>, WsbError> {
    let count = r.u8().ok_or(WsbError::Truncated)?.ix();
    if count > MAX_VARIANTS {
        return Err(WsbError::Limit("variants"));
    }
    let valid_states = (1u16 << STATES.len()) - 1;
    let valid_media = to_u8((1u16 << MEDIA.len()) - 1);
    let mut out: Vec<Variant> = Vec::with_capacity(count);
    for _ in 0..count {
        let state = r.u16().ok_or(WsbError::Truncated)?;
        if state & !valid_states != 0 || state.count_ones() > 1 {
            return Err(WsbError::Value("state"));
        }
        let media = r.u8().ok_or(WsbError::Truncated)?;
        if media & !valid_media != 0 {
            return Err(WsbError::Value("media"));
        }
        let parent = match r.u8().ok_or(WsbError::Truncated)? {
            0 => None,
            1 => {
                let class = r.u32().ok_or(WsbError::Truncated)?;
                let pstate = r.u16().ok_or(WsbError::Truncated)?;
                if class >= n_classes || pstate & !valid_states != 0 || pstate.count_ones() > 1 {
                    return Err(WsbError::Value("parent selector"));
                }
                Some(ParentSel {
                    class,
                    state: pstate,
                })
            }
            _ => return Err(WsbError::Value("parent flag")),
        };
        let mask = r.u128().ok_or(WsbError::Truncated)?;
        if mask >> PropId::COUNT != 0 {
            return Err(WsbError::Value("property mask"));
        }
        let mut decls = Decls {
            mask,
            p: Props::default(),
        };
        let mut m = mask;
        while m != 0 {
            let b = to_u8(m.trailing_zeros());
            m &= m - 1;
            let id = PropId::from_u8(b).ok_or(WsbError::Value("property id"))?;
            get_prop(id, &mut decls.p, r).ok_or(WsbError::Value("property value"))?;
        }
        if decls.p.motion.transition.props & !anim_mask() != 0 {
            return Err(WsbError::Value("transition"));
        }
        let v = Variant {
            state,
            media,
            parent,
            decls,
        };
        if out.iter().any(|x| x.key() == v.key()) {
            return Err(WsbError::Value("duplicate variant"));
        }
        out.push(v);
    }
    Ok(out)
}

/// Read a table from untrusted bytes.
pub fn load(b: &[u8]) -> Result<StyleTable, WsbError> {
    if b.len() > MAX_TABLE_BYTES {
        return Err(WsbError::TooBig);
    }
    if b.len() < 4 || &b[..4] != MAGIC {
        return Err(WsbError::Magic);
    }
    if b.len() < 4 + 4 + 4 {
        return Err(WsbError::Truncated);
    }
    let mut r = Rd::new(b);
    r.take(4);
    let version = r.u16().ok_or(WsbError::Truncated)?;
    if version != VERSION {
        return Err(WsbError::Version(version));
    }
    r.u16().ok_or(WsbError::Truncated)?;
    let n_kinds = r.u32().ok_or(WsbError::Truncated)?;
    let n_classes = r.u32().ok_or(WsbError::Truncated)?;
    if n_kinds.ix() != KINDS.len() {
        return Err(WsbError::Limit("kinds"));
    }
    if n_classes.ix() > MAX_CLASSES {
        return Err(WsbError::Limit("classes"));
    }
    if b.len() < 4 {
        return Err(WsbError::Truncated);
    }
    let body = &b[..b.len() - 4];
    let want = u32::from_le_bytes([
        b[b.len() - 4],
        b[b.len() - 3],
        b[b.len() - 2],
        b[b.len() - 1],
    ]);
    if fnv1a(body) != want {
        return Err(WsbError::Checksum);
    }
    let mut r = Rd { b: body, at: r.at };
    let mut t = StyleTable::default();
    for k in t.kinds.iter_mut() {
        k.variants = get_variants(&mut r, n_classes)?;
    }
    let mut seen = std::collections::HashSet::new();
    for _ in 0..n_classes {
        let len = r.u8().ok_or(WsbError::Truncated)?.ix();
        let name = r.take(len).ok_or(WsbError::Truncated)?;
        let name = std::str::from_utf8(name).map_err(|_| WsbError::Name)?;
        if len > MAX_CLASS_NAME
            || !super::wolcss::valid_class(name)
            || !seen.insert(name.to_string())
        {
            return Err(WsbError::Name);
        }
        let variants = get_variants(&mut r, n_classes)?;
        t.classes.push(ClassDef {
            name: name.to_string(),
            variants,
        });
    }
    if r.left() != 0 {
        return Err(WsbError::Trailing);
    }
    Ok(t)
}

/// `src/gen/styles.cr`: one typed id per class, for the shard (`Wolkit::Styles::CARD`).
pub fn crystal_constants(t: &StyleTable) -> String {
    let mut s = String::from(
        "# Generated by wolkit-stylec from the .wolcss sheets. Do not edit.\nmodule Wolkit::Styles\n",
    );
    for (i, c) in t.classes.iter().enumerate() {
        s.push_str(&format!(
            "  {} = {}_u32 # .{}\n",
            c.name.to_ascii_uppercase().replace('-', "_"),
            i,
            c.name
        ));
    }
    s.push_str("  NAMES = {\n");
    for (i, c) in t.classes.iter().enumerate() {
        s.push_str(&format!("    \"{}\" => {}_u32,\n", c.name, i));
    }
    s.push_str("  } of String => UInt32\nend\n");
    s
}
