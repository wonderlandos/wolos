// SPDX-License-Identifier: BSD-2-Clause
//! The font set: faces, family lookup, and the fallback chain.
//!
//! Faces are parsed by `read-fonts` (safe Rust, `forbid(unsafe_code)`); a face loaded from disk
//! is read lazily on first use. A system scan is avoided at start-up by an mtime-keyed index file
//! ([`FontIndex`]): when every recorded directory still has the same modification time the file
//! is trusted as is.
//!
//! Fallback order for a character (plan 2.5): the requested face, then the fonts registered for
//! the character's script, then the general chain (Latin fallback first), then the requested face
//! again so the visible `.notdef` ("tofu") glyph is drawn. Colour emoji slots into the chain in
//! phase 3.

use crate::num::trunc_u32;
use crate::scene::ids::{Ix, to_u16, to_u32};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::UNIX_EPOCH;

use harfrust::Script;
use skrifa::Font;

/// Largest font file read (16 MiB); bigger files are skipped.
pub const MAX_FONT_BYTES: u64 = 16 * 1024 * 1024;
/// Most font files a scan will index.
pub const MAX_SCAN_FILES: usize = 20_000;

/// A face handle, an index into the [`FontDb`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FontId(pub u32);

/// One font face.
pub struct Face {
    pub id: FontId,
    pub family: String,
    pub weight: u16,
    pub italic: bool,
    pub path: Option<PathBuf>,
    font: OnceLock<Option<Font>>,
}

impl Face {
    /// The parsed font, loading it from disk on first use. `None` when the file is unreadable or
    /// not a font.
    pub fn font(&self) -> Option<&Font> {
        self.font
            .get_or_init(|| {
                let path = self.path.as_ref()?;
                let md = std::fs::metadata(path).ok()?;
                if md.len() > MAX_FONT_BYTES {
                    return None;
                }
                Font::new(std::fs::read(path).ok()?, 0)
            })
            .as_ref()
    }

    /// True when the face maps `c` to a real glyph.
    pub fn has_char(&self, c: char) -> bool {
        match self.font() {
            Some(f) => f.charmap().map_unicode(c).is_some_and(|g| g.to_u32() != 0),
            None => false,
        }
    }
}

/// Weight of a requested face in the CSS 100..=900 scale.
pub type Weight = u16;

#[derive(Default)]
pub struct FontDb {
    faces: Vec<Face>,
    by_family: HashMap<String, Vec<FontId>>,
    chain: Vec<FontId>,
    script_fonts: Vec<(Script, Vec<FontId>)>,
}

impl FontDb {
    pub fn new() -> FontDb {
        FontDb::default()
    }

    pub fn len(&self) -> usize {
        self.faces.len()
    }

    pub fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }

    pub fn face(&self, id: FontId) -> Option<&Face> {
        self.faces.get(id.0.ix())
    }

    /// Adds a face from memory. The family name comes from the font's own `name` table.
    pub fn add_data(&mut self, data: Vec<u8>) -> Option<FontId> {
        let font = Font::new(data, 0)?;
        let family = family_of(&font)?;
        let (weight, italic) = attributes_of(&font);
        Some(self.push(Face {
            id: FontId(0),
            family,
            weight,
            italic,
            path: None,
            font: OnceLock::from(Some(font)),
        }))
    }

    /// Adds a face from a file, read now.
    pub fn add_file(&mut self, path: &Path) -> Option<FontId> {
        let md = std::fs::metadata(path).ok()?;
        if md.len() > MAX_FONT_BYTES {
            return None;
        }
        let id = self.add_data(std::fs::read(path).ok()?)?;
        self.faces[id.0.ix()].path = Some(path.to_path_buf());
        Some(id)
    }

    /// Adds a face from the index without reading the file; it is loaded on first use.
    pub fn add_indexed(&mut self, e: &IndexEntry) -> FontId {
        self.push(Face {
            id: FontId(0),
            family: e.family.clone(),
            weight: e.weight,
            italic: e.italic,
            path: Some(e.path.clone()),
            font: OnceLock::new(),
        })
    }

    fn push(&mut self, mut face: Face) -> FontId {
        let id = FontId(to_u32(self.faces.len()));
        face.id = id;
        self.by_family
            .entry(face.family.to_lowercase())
            .or_default()
            .push(id);
        self.faces.push(face);
        id
    }

    /// The best face of `family` for a weight and slant: closest weight wins, then matching slant.
    pub fn find(&self, family: &str, weight: Weight, italic: bool) -> Option<FontId> {
        let list = self.by_family.get(&family.to_lowercase())?;
        list.iter().copied().min_by_key(|id| {
            let f = &self.faces[id.0.ix()];
            (
                u32::from(f.italic != italic),
                u32::from(f.weight.abs_diff(weight)),
                id.0,
            )
        })
    }

    /// The first family of `families` that exists, matched to the weight and slant.
    pub fn select(&self, families: &[&str], weight: Weight, italic: bool) -> Option<FontId> {
        families.iter().find_map(|f| self.find(f, weight, italic))
    }

    /// Sets the general fallback chain, tried in order after the requested face.
    pub fn set_chain(&mut self, chain: &[FontId]) {
        self.chain = chain.to_vec();
    }

    /// Registers the fonts to try for characters of `script`, before the general chain.
    pub fn set_script_fallback(&mut self, script: Script, fonts: &[FontId]) {
        self.script_fonts.retain(|(s, _)| *s != script);
        self.script_fonts.push((script, fonts.to_vec()));
    }

    /// The face that draws `c`, starting from `preferred`.
    pub fn resolve(&self, c: char, script: Script, preferred: FontId) -> FontId {
        if never_switches(c) {
            return preferred;
        }
        if self.face(preferred).is_some_and(|f| f.has_char(c)) {
            return preferred;
        }
        if let Some((_, list)) = self.script_fonts.iter().find(|(s, _)| *s == script)
            && let Some(id) = list
                .iter()
                .copied()
                .find(|id| self.face(*id).is_some_and(|f| f.has_char(c)))
        {
            return id;
        }
        for id in &self.chain {
            if self.face(*id).is_some_and(|f| f.has_char(c)) {
                return *id;
            }
        }
        preferred
    }
}

/// Characters that stay in the surrounding run's font: spaces, controls, joiners, marks that
/// attach to whatever precedes them.
fn never_switches(c: char) -> bool {
    c.is_control()
        || c.is_whitespace()
        || matches!(c, '\u{200B}'..='\u{200F}' | '\u{2028}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{FE00}'..='\u{FE0F}' | '\u{FEFF}')
}

fn family_of(font: &Font) -> Option<String> {
    use skrifa::raw::types::NameId;
    let names = font.names();
    for id in [NameId::TYPOGRAPHIC_FAMILY_NAME, NameId::FAMILY_NAME] {
        if let Some(n) = names.get(id).and_then(|n| n.english_or_first()) {
            let s: String = n.name().chars().collect();
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    None
}

fn attributes_of(font: &Font) -> (u16, bool) {
    let a = font.default_attributes();
    let w = to_u16(trunc_u32(a.weight.value().round().clamp(1.0, 1000.0)));
    let italic = !matches!(a.style, skrifa::font::attribute::Style::Normal);
    (w, italic)
}

// ---------------------------------------------------------------------------------------------
// The mtime-keyed font index.
// ---------------------------------------------------------------------------------------------

/// One indexed font file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexEntry {
    pub path: PathBuf,
    pub size: u64,
    pub family: String,
    pub weight: u16,
    pub italic: bool,
}

/// A directory recorded with its modification time (nanoseconds since the epoch).
#[derive(Clone, Debug, PartialEq, Eq)]
struct DirStamp {
    path: PathBuf,
    mtime_ns: u128,
}

/// The list of installed fonts, loadable from a cache file without touching the font files.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FontIndex {
    dirs: Vec<DirStamp>,
    pub entries: Vec<IndexEntry>,
}

const INDEX_MAGIC: &str = "WKFI1";

impl FontIndex {
    /// Loads the cache file when it is still valid for `roots`; otherwise scans the roots,
    /// writes the cache (best effort) and returns the fresh index. The bool is true when the
    /// cache was used (no directory walk, no font file read).
    pub fn load_or_scan(roots: &[PathBuf], cache: &Path) -> (FontIndex, bool) {
        if let Some(idx) = FontIndex::read_cache(cache, roots) {
            return (idx, true);
        }
        let idx = FontIndex::scan(roots);
        if let Some(parent) = cache.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let tmp = cache.with_extension("tmp");
        if std::fs::write(&tmp, idx.serialize()).is_ok() {
            let _ = std::fs::rename(&tmp, cache);
        }
        (idx, false)
    }

    /// Walks `roots` (depth first, sorted) and reads every font file's names.
    pub fn scan(roots: &[PathBuf]) -> FontIndex {
        let mut idx = FontIndex::default();
        let mut stack: Vec<PathBuf> = roots.iter().rev().cloned().collect();
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else {
                continue;
            };
            if let Some(m) = mtime_ns(&dir) {
                idx.dirs.push(DirStamp {
                    path: dir.clone(),
                    mtime_ns: m,
                });
            }
            let mut children: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
            children.sort();
            for p in children.into_iter().rev() {
                if p.is_dir() {
                    stack.push(p);
                } else if is_font_file(&p)
                    && idx.entries.len() < MAX_SCAN_FILES
                    && let Some(e) = entry_of(&p)
                {
                    idx.entries.push(e);
                }
            }
        }
        idx.entries.sort_by(|a, b| a.path.cmp(&b.path));
        idx
    }

    fn read_cache(cache: &Path, roots: &[PathBuf]) -> Option<FontIndex> {
        let text = std::fs::read_to_string(cache).ok()?;
        let mut lines = text.lines();
        if lines.next()? != INDEX_MAGIC {
            return None;
        }
        let mut idx = FontIndex::default();
        let mut root_list = Vec::new();
        for line in lines {
            let mut f = line.split('\t');
            match f.next()? {
                "R" => root_list.push(PathBuf::from(f.next()?)),
                "D" => idx.dirs.push(DirStamp {
                    path: PathBuf::from(f.next()?),
                    mtime_ns: f.next()?.parse().ok()?,
                }),
                "F" => idx.entries.push(IndexEntry {
                    path: PathBuf::from(f.next()?),
                    size: f.next()?.parse().ok()?,
                    weight: f.next()?.parse().ok()?,
                    italic: f.next()? == "1",
                    family: f.next()?.to_string(),
                }),
                _ => return None,
            }
        }
        if root_list != roots {
            return None;
        }
        // Valid only while every scanned directory is unchanged.
        for d in &idx.dirs {
            if mtime_ns(&d.path) != Some(d.mtime_ns) {
                return None;
            }
        }
        Some(idx)
    }

    fn serialize(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "{INDEX_MAGIC}");
        // Roots are the first stamps with no parent among the stamps; store them explicitly.
        for r in self.roots() {
            let _ = writeln!(s, "R\t{}", r.display());
        }
        for d in &self.dirs {
            let _ = writeln!(s, "D\t{}\t{}", d.path.display(), d.mtime_ns);
        }
        for e in &self.entries {
            let _ = writeln!(
                s,
                "F\t{}\t{}\t{}\t{}\t{}",
                e.path.display(),
                e.size,
                e.weight,
                u8::from(e.italic),
                e.family
            );
        }
        s
    }

    fn roots(&self) -> Vec<&Path> {
        self.dirs
            .iter()
            .filter(|d| {
                !self
                    .dirs
                    .iter()
                    .any(|o| o.path != d.path && d.path.starts_with(&o.path))
            })
            .map(|d| d.path.as_path())
            .collect()
    }

    /// Adds every indexed file to `db` without reading any of them.
    pub fn register(&self, db: &mut FontDb) -> Vec<FontId> {
        self.entries.iter().map(|e| db.add_indexed(e)).collect()
    }
}

fn mtime_ns(p: &Path) -> Option<u128> {
    let m = std::fs::metadata(p).ok()?.modified().ok()?;
    Some(m.duration_since(UNIX_EPOCH).ok()?.as_nanos())
}

fn is_font_file(p: &Path) -> bool {
    matches!(
        p.extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("ttf" | "otf")
    )
}

fn entry_of(p: &Path) -> Option<IndexEntry> {
    let md = std::fs::metadata(p).ok()?;
    if md.len() > MAX_FONT_BYTES {
        return None;
    }
    let font = Font::new(std::fs::read(p).ok()?, 0)?;
    let family = family_of(&font)?;
    if family.contains(['\t', '\n']) {
        return None;
    }
    let (weight, italic) = attributes_of(&font);
    Some(IndexEntry {
        path: p.to_path_buf(),
        size: md.len(),
        family,
        weight,
        italic,
    })
}
