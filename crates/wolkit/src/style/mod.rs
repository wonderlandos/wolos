// SPDX-License-Identifier: BSD-2-Clause
//! Style (plan section 2.3): the `.wolcss` subset, theme tokens, the property table, the `WSB1`
//! style table and the cascade that turns classes, state and inline properties into one interned
//! computed style per node.
//!
//! - [`tokens`], [`value`], [`prop`]: the vocabulary (13 colours, scales, the property table)
//! - [`wolcss`], [`vparse`], [`lint`]: the compiler front end and its diagnostics (`WK-nnn`)
//! - [`wsb`]: the binary table (`WSB1`) and its bounded loader
//! - [`select`]: table, cascade, memo, interning
//! - [`inline`]: the wire (tier 1) properties as declarations
//! - [`transition`]: curves, springs and the plan of what animates when a style changes
//! - [`doc`]: `docs/modules/wolkit-style.md`, generated from the tables above
pub mod doc;
pub mod inline;
pub mod lint;
pub mod prop;
pub mod select;
pub mod tokens;
pub mod transition;
pub mod value;
pub mod vparse;
pub mod wolcss;
pub mod wsb;

use crate::env::Environment;
use crate::error::{Error, Result};
use crate::scene::{Idx, NIL, SceneWrite, dirty};
use select::*;
use std::sync::atomic::{AtomicU64, Ordering};
use value::Props;

pub use select::{StyleId, StyleStore};
pub use wsb::{MAGIC, WsbError};

/// Largest style table (plan section 2.8).
pub const MAX_TABLE: usize = MAX_TABLE_BYTES;

/// A loaded style table. Immutable and shareable between windows.
#[derive(Debug)]
pub struct Table {
    pub table: StyleTable,
    /// Distinct for every table ever loaded, so a window can tell the table changed.
    pub generation: u64,
}

static GENERATION: AtomicU64 = AtomicU64::new(1);

impl Table {
    pub fn new(table: StyleTable) -> Table {
        Table { table, generation: GENERATION.fetch_add(1, Ordering::Relaxed) }
    }
}

/// Parse and validate a `WSB1` table. Untrusted input: bounded, never panics (fuzz target
/// `wolkit_wsb`).
pub fn load(bytes: &[u8]) -> Result<Table> {
    match wsb::load(bytes) {
        Ok(t) => Ok(Table::new(t)),
        Err(WsbError::TooBig) => Err(Error::limit(WsbError::TooBig.to_string())),
        Err(e) => Err(Error::invalid(e.to_string())),
    }
}

/// Compile `.wolcss` source to a `WSB1` table (dev reload and the CLI). The error carries every
/// diagnostic, rendered.
pub fn compile(src: &str) -> Result<Vec<u8>> {
    compile_sources(&[("style.wolcss", src)]).map(|c| c.bytes).map_err(|d| Error::invalid(render_all(&d)))
}

pub fn render_all(d: &[lint::Diag]) -> String {
    d.iter().map(|x| x.render()).collect::<Vec<_>>().join("\n")
}

/// The result of compiling sheets: the bytes, the table they decode to and the warnings.
pub struct Compiled {
    pub bytes: Vec<u8>,
    pub table: StyleTable,
    pub warnings: Vec<lint::Diag>,
    /// Files that used the off-scale pragma.
    pub optouts: u32,
}

/// Compile sheets (file name, source) into one table. Errors stop the output; warnings do not.
pub fn compile_sources(files: &[(&str, &str)]) -> std::result::Result<Compiled, Vec<lint::Diag>> {
    let mut b = wolcss::Builder::new();
    for (name, src) in files {
        b.add(name, src);
    }
    let optouts = b.optouts;
    let (table, diags) = b.finish();
    if diags.iter().any(|d| d.is_error()) {
        return Err(diags);
    }
    match wsb::write(&table) {
        Ok(bytes) => Ok(Compiled { bytes, table, warnings: diags, optouts }),
        Err(e) => {
            let id = lint::TABLE_LIMIT;
            let mut v = diags;
            v.push(lint::Diag {
                id,
                severity: lint::Severity::Error,
                file: files.first().map_or("", |f| f.0).to_string(),
                line: 1,
                col: 1,
                len: 1,
                message: e.to_string(),
                help: Some("split the app or drop unused classes".into()),
                source_line: String::new(),
            });
            Err(v)
        }
    }
}

/// Per window: the interned computed styles and the memo of the cascade.
#[derive(Default)]
pub struct Styles {
    store: StyleStore,
    resolver: Resolver,
    last: Option<(u64, u8, u32)>,
}

impl Styles {
    /// The computed style behind the id the scene stores (`SceneRead::computed`).
    pub fn get(&self, id: u32) -> &Props {
        self.store.get(StyleId(id))
    }
    pub fn len(&self) -> usize {
        self.store.len()
    }
    pub fn is_empty(&self) -> bool {
        self.store.is_empty()
    }
    pub fn stats(&self) -> (u64, u64) {
        (self.resolver.hits, self.resolver.misses)
    }
}

/// Window width under which `@compact` holds (logical pixels).
pub const COMPACT_WIDTH: u32 = 640;

/// The media conditions that hold for an environment and a window width.
pub fn media_mask(env: &Environment, width: u32) -> u8 {
    let mut m = if env.dark { MEDIA_DARK } else { MEDIA_LIGHT };
    if env.contrast {
        m |= MEDIA_CONTRAST;
    }
    if env.reduce_motion {
        m |= 1 << 3;
    }
    if env.reduce_transparency {
        m |= 1 << 4;
    }
    if width < COMPACT_WIDTH {
        m |= 1 << 5;
    }
    m
}

struct Cx<'a> {
    table: &'a StyleTable,
    media: u8,
    scale: f32,
    parent_rules: bool,
}

fn empty_table() -> &'static StyleTable {
    static E: std::sync::OnceLock<StyleTable> = std::sync::OnceLock::new();
    E.get_or_init(StyleTable::default)
}

/// Resolve computed styles for the dirty nodes of `scene` and set the layout and paint bits that
/// follow from what changed: a changed table, environment or window class restyles everything;
/// otherwise only nodes with the style bit (and what inherits from them).
pub fn resolve(scene: &mut dyn SceneWrite, table: Option<&Table>, styles: &mut Styles, env: &Environment, width: u32) {
    let media = media_mask(env, width);
    let key = (table.map_or(0, |t| t.generation), media, env.text_scale.to_bits());
    let full = styles.last != Some(key);
    if full {
        styles.resolver.clear();
        styles.last = Some(key);
    }
    let tbl: &StyleTable = match table {
        Some(t) => &t.table,
        None => empty_table(),
    };
    let parent_rules = tbl.classes.iter().any(|c| c.has_parent_rules());
    let scale = if env.text_scale.is_finite() { env.text_scale.clamp(0.5, 3.0) } else { 1.0 };
    let cx = Cx { table: tbl, media, scale, parent_rules };
    if !scene.is_live(0) {
        return;
    }
    visit(scene, styles, &cx, 0, None, full);
}

#[derive(Clone, Copy)]
struct ParentInfo {
    style: StyleId,
    classes: [u32; 8],
    n: usize,
    state: u16,
}

fn visit(scene: &mut dyn SceneWrite, styles: &mut Styles, cx: &Cx, i: Idx, parent: Option<ParentInfo>, forced: bool) {
    let d = scene.dirty(i);
    let need = forced || d & dirty::STYLE != 0;
    if !need && d & dirty::SUBTREE == 0 {
        return;
    }
    let mut cur = StyleId(scene.computed(i));
    let mut buf = [0u32; 8];
    let cl = scene.classes(i);
    let n = cl.len().min(8);
    buf[..n].copy_from_slice(&cl[..n]);
    let state = (scene.state(i) & 0x1ff) as u16;
    let mut changed = false;
    if need {
        let mut inline = prop::Decls::default();
        scene.each_prop(i, &mut |p, v| inline::decl_from_wire(p, v, &mut inline));
        let inp = Input {
            kind: (scene.kind(i) as u8).saturating_sub(1),
            classes: &buf[..n],
            state,
            media: cx.media,
            parent: parent.map(|p| p.style),
            parent_classes: parent.as_ref().map_or(&[], |p| &p.classes[..p.n]),
            parent_state: parent.map_or(0, |p| p.state),
            inline: if inline.is_empty() { None } else { Some(&inline) },
            text_scale: cx.scale,
        };
        let new = styles.resolver.resolve(cx.table, &mut styles.store, &inp);
        if new != cur {
            let ch = diff(styles.store.get(cur), styles.store.get(new));
            let bits = dirty::PAINT | if needs_layout(ch) { dirty::LAYOUT } else { 0 };
            scene.set_computed(i, new.0);
            scene.mark_dirty(i, bits);
            cur = new;
            changed = true;
        }
        scene.clear_dirty(i, dirty::STYLE);
    }
    let force_kids = forced || changed || (need && cx.parent_rules);
    let me = ParentInfo { style: cur, classes: buf, n, state };
    let mut c = scene.first_child(i);
    while c != NIL {
        let next = scene.next_sibling(c);
        visit(scene, styles, cx, c, Some(me), force_kids);
        c = next;
    }
}
