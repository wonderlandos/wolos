// SPDX-License-Identifier: BSD-2-Clause
//! The compiled table, `.wtx` (WolOS text): one language of one unit, little-endian.
//!
//! ```text
//! 0   "WTX1"      magic
//! 4   u16 1       version          6  u16 0  flags
//! 8   [u8; 12]    language tag, NUL-padded ("pt-BR", "zh-Hans")
//! 20  u32         catalog: FNV-1a of the unit's key names (one per line, en.tsv order)
//! 24  u32 N       keys             28 u32 S  hash slots (a power of two, at least 2N)
//! 32  u32 P       plural blocks    36 u32 B  bytes of text
//! 40  N x (off u32, len u32)       text of key i; len PLURAL: off is a plural block; off ABSENT: none
//!     N x (off u32, len u32)       name of key i
//!     S x u32                      key index + 1 of the slot (0: empty), linear probing
//!     P x 6 x (off u32, len u32)   forms zero, one, two, few, many, other (off ABSENT: none)
//!     B bytes                      UTF-8 text and names
//! ```
//!
//! Everything is checked once, in [`Table::from_static`] / [`Table::from_vec`]: every range is
//! in the text and on character boundaries. A lookup then reads 8 bytes and slices: constant
//! time, no allocation, no parsing.
use super::{Lang, Plural, fnv1a};

/// Format magic.
pub const MAGIC: &[u8; 4] = b"WTX1";
/// Format version.
pub const VERSION: u16 = 1;
/// `off` of a key or form a table does not have.
pub const ABSENT: u32 = u32::MAX;
/// `len` of a key whose `off` is a plural block.
pub const PLURAL: u32 = u32::MAX;
/// Header bytes.
pub const HEADER: usize = 40;
/// Most keys one table holds.
pub const MAX_KEYS: u32 = 1 << 20;

/// Why a table was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WtxError {
    /// Not a `.wtx` file, or another version.
    Format,
    /// A size or range in the file does not fit its length.
    Range,
    /// The text is not UTF-8, or a range splits a character.
    Text,
    /// The language tag is not one WolOS knows.
    Language,
    /// A catalog needs an English table first (and only one).
    NotEnglish,
}

impl std::fmt::Display for WtxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            WtxError::Format => "not a text table of this version",
            WtxError::Range => "a range in the text table is out of bounds",
            WtxError::Text => "the text table holds text that is not UTF-8",
            WtxError::Language => "the text table names an unknown language",
            WtxError::NotEnglish => "the English text table is missing",
        })
    }
}

impl std::error::Error for WtxError {}

/// The index part of a table: borrowed from the binary or owned.
enum Bytes {
    Static(&'static [u8]),
    Owned(Box<[u8]>),
}

/// The text part of a table.
enum Text {
    Static(&'static str),
    Owned(Box<str>),
}

/// One parsed, checked table.
pub struct Table {
    lang: Lang,
    catalog: u32,
    n: u32,
    slots: u32,
    plurals: u32,
    index: Bytes,
    text: Text,
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes(s.try_into().ok()?))
}

fn us(v: u32) -> usize {
    usize::try_from(v).unwrap_or(usize::MAX)
}

/// Layout of a table's index from its header counts: (names at, slots at, plurals at, text at).
fn offsets(n: u32, slots: u32, plurals: u32) -> Option<(usize, usize, usize, usize)> {
    let names = HEADER.checked_add(us(n).checked_mul(8)?)?;
    let slot_at = names.checked_add(us(n).checked_mul(8)?)?;
    let plural_at = slot_at.checked_add(us(slots).checked_mul(4)?)?;
    let text_at = plural_at.checked_add(us(plurals).checked_mul(48)?)?;
    Some((names, slot_at, plural_at, text_at))
}

/// Header fields: (lang, catalog, n, slots, plurals, text bytes, text at). Err: not a table.
fn header(b: &[u8]) -> Result<(Lang, u32, u32, u32, u32, usize), WtxError> {
    if b.len() < HEADER || &b[..4] != MAGIC {
        return Err(WtxError::Format);
    }
    if u16::from_le_bytes([b[4], b[5]]) != VERSION {
        return Err(WtxError::Format);
    }
    let tag = std::str::from_utf8(&b[8..20]).map_err(|_| WtxError::Language)?;
    let lang = Lang::from_tag(tag.trim_end_matches('\0')).ok_or(WtxError::Language)?;
    let f = |at| u32_at(b, at).ok_or(WtxError::Range);
    let (catalog, n, slots, plurals, len) = (f(20)?, f(24)?, f(28)?, f(32)?, f(36)?);
    if n > MAX_KEYS || plurals > n || (slots != 0 && !slots.is_power_of_two()) || slots < n {
        return Err(WtxError::Range);
    }
    let (_, _, _, text_at) = offsets(n, slots, plurals).ok_or(WtxError::Range)?;
    if text_at.checked_add(us(len)) != Some(b.len()) {
        return Err(WtxError::Range);
    }
    Ok((lang, catalog, n, slots, plurals, text_at))
}

impl Table {
    /// A table built into the binary (`include_bytes!`). Err: see [`WtxError`].
    pub fn from_static(b: &'static [u8]) -> Result<Table, WtxError> {
        let (lang, catalog, n, slots, plurals, at) = header(b)?;
        let text = std::str::from_utf8(&b[at..]).map_err(|_| WtxError::Text)?;
        let t = Table {
            lang,
            catalog,
            n,
            slots,
            plurals,
            index: Bytes::Static(&b[..at]),
            text: Text::Static(text),
        };
        t.check()?;
        Ok(t)
    }

    /// A table read from a file (a language pack). Err: see [`WtxError`].
    pub fn from_vec(mut b: Vec<u8>) -> Result<Table, WtxError> {
        let (lang, catalog, n, slots, plurals, at) = header(&b)?;
        let text = String::from_utf8(b.split_off(at)).map_err(|_| WtxError::Text)?;
        let t = Table {
            lang,
            catalog,
            n,
            slots,
            plurals,
            index: Bytes::Owned(b.into_boxed_slice()),
            text: Text::Owned(text.into_boxed_str()),
        };
        t.check()?;
        Ok(t)
    }

    /// An English table with no keys.
    pub(crate) fn empty_english() -> Table {
        Table {
            lang: Lang::En,
            catalog: fnv1a(b""),
            n: 0,
            slots: 0,
            plurals: 0,
            index: Bytes::Static(&[]),
            text: Text::Static(""),
        }
    }

    fn idx(&self) -> &[u8] {
        match &self.index {
            Bytes::Static(b) => b,
            Bytes::Owned(b) => b,
        }
    }

    fn txt(&self) -> &str {
        match &self.text {
            Text::Static(s) => s,
            Text::Owned(s) => s,
        }
    }

    /// Every range valid, every slot pointing at a key, every plural block in bounds.
    fn check(&self) -> Result<(), WtxError> {
        let Some((names, slot_at, plural_at, _)) = offsets(self.n, self.slots, self.plurals) else {
            return Err(WtxError::Range);
        };
        let ok_range = |off: u32, len: u32| {
            let (a, z) = (us(off), us(off).checked_add(us(len)));
            z.is_some_and(|z| {
                z <= self.txt().len()
                    && self.txt().is_char_boundary(a)
                    && self.txt().is_char_boundary(z)
            })
        };
        let pair = |at: usize| Some((u32_at(self.idx(), at)?, u32_at(self.idx(), at + 4)?));
        for i in 0..us(self.n) {
            let (off, len) = pair(HEADER + i * 8).ok_or(WtxError::Range)?;
            let fine = (off == ABSENT && len == 0)
                || (len == PLURAL && off < self.plurals)
                || (len != PLURAL && ok_range(off, len));
            let (noff, nlen) = pair(names + i * 8).ok_or(WtxError::Range)?;
            if !fine || !ok_range(noff, nlen) {
                return Err(WtxError::Range);
            }
        }
        for s in 0..us(self.slots) {
            let v = u32_at(self.idx(), slot_at + s * 4).ok_or(WtxError::Range)?;
            if v > self.n {
                return Err(WtxError::Range);
            }
        }
        for f in 0..us(self.plurals) * 6 {
            let (off, len) = pair(plural_at + f * 8).ok_or(WtxError::Range)?;
            if !(off == ABSENT && len == 0) && !(len != PLURAL && ok_range(off, len)) {
                return Err(WtxError::Range);
            }
        }
        Ok(())
    }

    /// The table's language.
    pub fn lang(&self) -> Lang {
        self.lang
    }

    /// The identity of the key list the table was compiled against.
    pub fn catalog(&self) -> u32 {
        self.catalog
    }

    /// Number of keys.
    pub fn len(&self) -> u32 {
        self.n
    }

    /// Whether the table has no keys.
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    fn slice(&self, at: usize) -> Option<&str> {
        let off = u32_at(self.idx(), at)?;
        let len = u32_at(self.idx(), at + 4)?;
        if off == ABSENT || len == PLURAL {
            return None;
        }
        self.txt().get(us(off)..us(off).checked_add(us(len))?)
    }

    /// The text of key `i` in category `cat` (a key that is not plural ignores `cat`); a plural
    /// key without that form reads its `other` form. None: the table does not have the key.
    pub fn get(&self, i: u32, cat: Plural) -> Option<&str> {
        if i >= self.n {
            return None;
        }
        let at = HEADER + us(i) * 8;
        let (off, len) = (u32_at(self.idx(), at)?, u32_at(self.idx(), at + 4)?);
        if len != PLURAL {
            return self.slice(at);
        }
        let (_, _, plural_at, _) = offsets(self.n, self.slots, self.plurals)?;
        let block = plural_at + us(off) * 48;
        self.slice(block + cat.index() * 8)
            .or_else(|| self.slice(block + Plural::Other.index() * 8))
    }

    /// The name of key `i`; None: no such key.
    pub fn key_name(&self, i: u32) -> Option<&str> {
        if i >= self.n {
            return None;
        }
        let (names, _, _, _) = offsets(self.n, self.slots, self.plurals)?;
        let at = names + us(i) * 8;
        let (off, len) = (u32_at(self.idx(), at)?, u32_at(self.idx(), at + 4)?);
        self.txt().get(us(off)..us(off).checked_add(us(len))?)
    }

    /// The index of the key named `name` (expected constant time: one hash, short probe).
    pub fn find(&self, name: &str) -> Option<u32> {
        if self.slots == 0 {
            return None;
        }
        let (_, slot_at, _, _) = offsets(self.n, self.slots, self.plurals)?;
        let mask = self.slots - 1;
        let mut s = fnv1a(name.as_bytes()) & mask;
        for _ in 0..self.slots {
            let v = u32_at(self.idx(), slot_at + us(s) * 4)?;
            if v == 0 {
                return None;
            }
            if self.key_name(v - 1) == Some(name) {
                return Some(v - 1);
            }
            s = (s + 1) & mask;
        }
        None
    }
}

/// What one key holds in a table being written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// The table has no text for the key (the chain's next language answers).
    Absent,
    /// One text.
    Text(String),
    /// Plural forms, in [`Plural::ALL`] order (None: that form is not there).
    Plural([Option<String>; 6]),
}

/// Writes a table: `keys[i]` is named by `names[i]`. Err: more than [`MAX_KEYS`] keys, text
/// over 4 GiB, or `names` and `keys` of different lengths.
pub fn encode(lang: Lang, names: &[String], keys: &[Entry]) -> Result<Vec<u8>, WtxError> {
    if names.len() != keys.len() {
        return Err(WtxError::Range);
    }
    let n = u32::try_from(keys.len()).map_err(|_| WtxError::Range)?;
    if n > MAX_KEYS {
        return Err(WtxError::Range);
    }
    let slots = if n == 0 {
        0
    } else {
        (n * 2).next_power_of_two()
    };
    let mut text = String::new();
    let mut put = |s: &str| -> Result<(u32, u32), WtxError> {
        let off = u32::try_from(text.len()).map_err(|_| WtxError::Range)?;
        text.push_str(s);
        Ok((off, u32::try_from(s.len()).map_err(|_| WtxError::Range)?))
    };
    let mut entries = Vec::with_capacity(keys.len());
    let mut blocks: Vec<[(u32, u32); 6]> = Vec::new();
    for k in keys {
        entries.push(match k {
            Entry::Absent => (ABSENT, 0),
            Entry::Text(s) => put(s)?,
            Entry::Plural(forms) => {
                let mut b = [(ABSENT, 0); 6];
                for (slot, f) in b.iter_mut().zip(forms) {
                    if let Some(s) = f {
                        *slot = put(s)?;
                    }
                }
                blocks.push(b);
                (
                    u32::try_from(blocks.len() - 1).map_err(|_| WtxError::Range)?,
                    PLURAL,
                )
            }
        });
    }
    let name_ranges = names
        .iter()
        .map(|s| put(s))
        .collect::<Result<Vec<_>, _>>()?;
    let mut table = vec![0u32; us(slots)];
    for (i, name) in names.iter().enumerate() {
        let mask = slots - 1;
        let mut s = fnv1a(name.as_bytes()) & mask;
        while table[us(s)] != 0 {
            s = (s + 1) & mask;
        }
        table[us(s)] = u32::try_from(i + 1).map_err(|_| WtxError::Range)?;
    }
    let catalog = fnv1a(names.join("\n").as_bytes());
    let mut out = Vec::with_capacity(HEADER + keys.len() * 20 + text.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    let mut tag = [0u8; 12];
    for (d, s) in tag.iter_mut().zip(lang.tag().bytes()) {
        *d = s;
    }
    out.extend_from_slice(&tag);
    let blocks_n = u32::try_from(blocks.len()).map_err(|_| WtxError::Range)?;
    let text_n = u32::try_from(text.len()).map_err(|_| WtxError::Range)?;
    for v in [catalog, n, slots, blocks_n, text_n] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    let push_pair = |out: &mut Vec<u8>, (a, b): (u32, u32)| {
        out.extend_from_slice(&a.to_le_bytes());
        out.extend_from_slice(&b.to_le_bytes());
    };
    for p in entries.iter().chain(&name_ranges) {
        push_pair(&mut out, *p);
    }
    for v in &table {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for p in blocks.iter().flatten() {
        push_pair(&mut out, *p);
    }
    out.extend_from_slice(text.as_bytes());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        let names: Vec<String> = ["a.title", "a.files", "a.gone"].map(String::from).into();
        let mut forms: [Option<String>; 6] = Default::default();
        forms[Plural::One.index()] = Some("{n} file".into());
        forms[Plural::Other.index()] = Some("{n} files".into());
        let keys = [
            Entry::Text("Título".into()),
            Entry::Plural(forms),
            Entry::Absent,
        ];
        encode(Lang::PtBr, &names, &keys).expect("three keys encode")
    }

    #[test]
    fn a_table_reads_back_what_was_written() {
        let t = Table::from_vec(sample()).expect("sample parses");
        assert_eq!(t.lang(), Lang::PtBr);
        assert_eq!(t.len(), 3);
        assert_eq!(t.get(0, Plural::Other), Some("Título"));
        assert_eq!(t.get(1, Plural::One), Some("{n} file"));
        assert_eq!(
            t.get(1, Plural::Many),
            Some("{n} files"),
            "missing form reads other"
        );
        assert_eq!(t.get(2, Plural::Other), None);
        assert_eq!(t.get(3, Plural::Other), None);
        assert_eq!(t.find("a.files"), Some(1));
        assert_eq!(t.find("a.nope"), None);
        assert_eq!(t.key_name(2), Some("a.gone"));
    }

    #[test]
    fn a_damaged_table_is_refused_not_read() {
        let good = sample();
        assert_eq!(
            Table::from_vec(good[..10].to_vec()).err(),
            Some(WtxError::Format)
        );
        let mut short = good.clone();
        short.pop();
        assert_eq!(Table::from_vec(short).err(), Some(WtxError::Range));
        let mut lang = good.clone();
        lang[8] = b'q';
        assert_eq!(Table::from_vec(lang).err(), Some(WtxError::Language));
        // Every single-byte change either parses into a valid table or is refused; none panics.
        for i in 0..good.len() {
            let mut b = good.clone();
            b[i] ^= 0xa5;
            if let Ok(t) = Table::from_vec(b) {
                for k in 0..t.len() {
                    let _ = t.get(k, Plural::One);
                    let _ = t.key_name(k);
                }
                let _ = t.find("a.title");
            }
        }
    }

    #[test]
    fn an_empty_table_has_no_keys_and_finds_nothing() {
        let b = encode(Lang::En, &[], &[]).expect("empty encodes");
        let t = Table::from_vec(b).expect("empty parses");
        assert!(t.is_empty());
        assert_eq!(t.find("x"), None);
    }
}
