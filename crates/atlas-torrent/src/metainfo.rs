// SPDX-License-Identifier: BSD-2-Clause
//! A bounded look at a `.torrent` file before the engine sees it: size, nesting depth, number of
//! items and of files are capped, and every file path is checked by [`crate::confine`] (no `..`, no
//! absolute path, no separator or NUL inside a component, no name longer than 255 bytes). A file
//! that fails any check is refused as a whole: nothing of it reaches the engine.
use crate::confine::{self, PathError};

/// Largest `.torrent` file accepted.
pub const MAX_TORRENT: usize = 4 * 1024 * 1024;
/// Deepest nesting of lists and dictionaries.
pub const MAX_DEPTH: usize = 32;
/// Most values (strings, integers, lists, dictionaries) in one file.
pub const MAX_ITEMS: usize = 1_000_000;
/// Most files in one torrent.
pub const MAX_FILES: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value<'a> {
    Int(i64),
    Bytes(&'a [u8]),
    List(Vec<Value<'a>>),
    Dict(Vec<(&'a [u8], Value<'a>)>),
}

impl<'a> Value<'a> {
    pub fn get(&self, key: &str) -> Option<&Value<'a>> {
        match self {
            Value::Dict(kv) => kv
                .iter()
                .find(|(k, _)| *k == key.as_bytes())
                .map(|(_, v)| v),
            _ => None,
        }
    }
    fn bytes(&self) -> Option<&'a [u8]> {
        match self {
            Value::Bytes(b) => Some(b),
            _ => None,
        }
    }
    fn int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetaError {
    TooLarge,
    TooDeep,
    TooManyItems,
    TooManyFiles,
    Malformed,
    Path(PathError),
}

struct Parser<'a> {
    b: &'a [u8],
    at: usize,
    items: usize,
}

impl<'a> Parser<'a> {
    fn value(&mut self, depth: usize) -> Result<Value<'a>, MetaError> {
        if depth > MAX_DEPTH {
            return Err(MetaError::TooDeep);
        }
        self.items += 1;
        if self.items > MAX_ITEMS {
            return Err(MetaError::TooManyItems);
        }
        match *self.b.get(self.at).ok_or(MetaError::Malformed)? {
            b'i' => {
                self.at += 1;
                let end = self.find(b'e')?;
                let s =
                    std::str::from_utf8(&self.b[self.at..end]).map_err(|_| MetaError::Malformed)?;
                if s.is_empty()
                    || s == "-0"
                    || (s.len() > 1 && s.starts_with('0'))
                    || s.starts_with("-0")
                {
                    return Err(MetaError::Malformed);
                }
                let i = s.parse::<i64>().map_err(|_| MetaError::Malformed)?;
                self.at = end + 1;
                Ok(Value::Int(i))
            }
            b'l' => {
                self.at += 1;
                let mut v = Vec::new();
                while *self.b.get(self.at).ok_or(MetaError::Malformed)? != b'e' {
                    v.push(self.value(depth + 1)?);
                }
                self.at += 1;
                Ok(Value::List(v))
            }
            b'd' => {
                self.at += 1;
                let mut v: Vec<(&'a [u8], Value<'a>)> = Vec::new();
                while *self.b.get(self.at).ok_or(MetaError::Malformed)? != b'e' {
                    let k = match self.value(depth + 1)? {
                        Value::Bytes(k) => k,
                        _ => return Err(MetaError::Malformed),
                    };
                    if v.last().is_some_and(|(prev, _)| *prev >= k) {
                        return Err(MetaError::Malformed); // keys sorted and unique (BEP 3)
                    }
                    let val = self.value(depth + 1)?;
                    v.push((k, val));
                }
                self.at += 1;
                Ok(Value::Dict(v))
            }
            b'0'..=b'9' => {
                let colon = self.find(b':')?;
                let n: usize = std::str::from_utf8(&self.b[self.at..colon])
                    .ok()
                    .filter(|s| s.len() <= 8)
                    .and_then(|s| s.parse().ok())
                    .ok_or(MetaError::Malformed)?;
                let start = colon + 1;
                let end = start
                    .checked_add(n)
                    .filter(|e| *e <= self.b.len())
                    .ok_or(MetaError::Malformed)?;
                self.at = end;
                Ok(Value::Bytes(&self.b[start..end]))
            }
            _ => Err(MetaError::Malformed),
        }
    }

    fn find(&self, c: u8) -> Result<usize, MetaError> {
        // integers and lengths are short: look at most 24 bytes ahead
        let lim = (self.at + 24).min(self.b.len());
        self.b[self.at..lim]
            .iter()
            .position(|&x| x == c)
            .map(|p| self.at + p)
            .ok_or(MetaError::Malformed)
    }
}

/// Parse a whole bencoded value; trailing bytes are an error.
pub fn parse(b: &[u8]) -> Result<Value<'_>, MetaError> {
    if b.len() > MAX_TORRENT {
        return Err(MetaError::TooLarge);
    }
    let mut p = Parser { b, at: 0, items: 0 };
    let v = p.value(0)?;
    if p.at != b.len() {
        return Err(MetaError::Malformed);
    }
    Ok(v)
}

/// What the helper needs to know before handing the file to the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    /// The torrent's name: the job folder's name (already checked as one path component).
    pub name: String,
    /// Every file, as checked components under the job folder.
    pub files: Vec<(Vec<String>, u64)>,
    pub total: u64,
}

/// Check a `.torrent` file: bounded, well formed, every path confined.
pub fn check(b: &[u8]) -> Result<Summary, MetaError> {
    let root = parse(b)?;
    // a whole .torrent file, or the bare info dictionary a magnet link's peers send (BEP 9)
    let info = match root.get("info") {
        Some(i) => i,
        None if root.get("pieces").is_some() => &root,
        None => return Err(MetaError::Malformed),
    };
    let name_b = info
        .get("name")
        .and_then(Value::bytes)
        .ok_or(MetaError::Malformed)?;
    let name = confine::component(name_b).map_err(MetaError::Path)?;
    let plen = info
        .get("piece length")
        .and_then(Value::int)
        .ok_or(MetaError::Malformed)?;
    if !(16 * 1024..=64 * 1024 * 1024).contains(&plen) {
        return Err(MetaError::Malformed);
    }
    let pieces = info
        .get("pieces")
        .and_then(Value::bytes)
        .ok_or(MetaError::Malformed)?;
    if pieces.is_empty() || pieces.len() % 20 != 0 {
        return Err(MetaError::Malformed);
    }
    let mut files = Vec::new();
    let mut total: u64 = 0;
    match (info.get("length"), info.get("files")) {
        (Some(len), None) => {
            let l = len.int().filter(|l| *l >= 0).ok_or(MetaError::Malformed)? as u64;
            total = l;
            files.push((vec![name.clone()], l));
        }
        (None, Some(Value::List(list))) => {
            if list.is_empty() {
                return Err(MetaError::Malformed);
            }
            if list.len() > MAX_FILES {
                return Err(MetaError::TooManyFiles);
            }
            for f in list {
                let l = f
                    .get("length")
                    .and_then(Value::int)
                    .filter(|l| *l >= 0)
                    .ok_or(MetaError::Malformed)? as u64;
                let comps = match f.get("path") {
                    Some(Value::List(p)) => p
                        .iter()
                        .map(|c| c.bytes().ok_or(MetaError::Malformed))
                        .collect::<Result<Vec<_>, _>>()?,
                    _ => return Err(MetaError::Malformed),
                };
                let rel = confine::relative(&comps).map_err(MetaError::Path)?;
                total = total.checked_add(l).ok_or(MetaError::Malformed)?;
                files.push((rel, l));
            }
        }
        _ => return Err(MetaError::Malformed),
    }
    // the pieces cover the total length
    let need = total.div_ceil(plen as u64);
    if need != (pieces.len() / 20) as u64 {
        return Err(MetaError::Malformed);
    }
    Ok(Summary { name, files, total })
}

/// The raw bytes of the top-level `info` value of a whole `.torrent` file (None for a bare info
/// dictionary or a file without one). `b` must have passed [`parse`].
fn info_span(b: &[u8]) -> Option<&[u8]> {
    let mut p = Parser { b, at: 1, items: 0 };
    if b.first() != Some(&b'd') {
        return None;
    }
    while *b.get(p.at)? != b'e' {
        let k = match p.value(1).ok()? {
            Value::Bytes(k) => k,
            _ => return None,
        };
        let start = p.at;
        p.value(1).ok()?;
        if k == b"info" {
            return Some(&b[start..p.at]);
        }
    }
    None
}

fn bstr(out: &mut Vec<u8>, s: &[u8]) {
    out.extend_from_slice(s.len().to_string().as_bytes());
    out.push(b':');
    out.extend_from_slice(s);
}

/// What the engine gets of a checked `.torrent` file (DL-SEC-03): the same `info` dictionary (so
/// the same info hash) and only the trackers [`crate::trackers::keep`] allows, at most
/// [`crate::trackers::MAX_TRACKERS`], from `announce` and `announce-list`; every other top-level key
/// (comments, web seeds, extensions) is dropped. A bare info dictionary is wrapped likewise.
pub fn rebuild(b: &[u8], allow_local: bool) -> Result<(Vec<u8>, Vec<String>), MetaError> {
    let root = parse(b)?;
    let info: &[u8] = if root.get("info").is_some() {
        info_span(b).ok_or(MetaError::Malformed)?
    } else if root.get("pieces").is_some() {
        b
    } else {
        return Err(MetaError::Malformed);
    };
    let mut named: Vec<&str> = Vec::new();
    if let Some(a) = root.get("announce").and_then(Value::bytes)
        && let Ok(s) = std::str::from_utf8(a)
    {
        named.push(s);
    }
    if let Some(Value::List(tiers)) = root.get("announce-list") {
        for t in tiers {
            if let Value::List(urls) = t {
                for u in urls {
                    if let Some(Ok(s)) = u.bytes().map(std::str::from_utf8) {
                        named.push(s);
                    }
                }
            }
        }
    }
    let trackers = crate::trackers::keep(named, allow_local);
    let mut out = Vec::with_capacity(info.len() + 64 * trackers.len() + 64);
    out.push(b'd');
    if let Some(first) = trackers.first() {
        bstr(&mut out, b"announce");
        bstr(&mut out, first.as_bytes());
        bstr(&mut out, b"announce-list");
        out.push(b'l');
        for t in &trackers {
            out.push(b'l');
            bstr(&mut out, t.as_bytes());
            out.push(b'e');
        }
        out.push(b'e');
    }
    bstr(&mut out, b"info");
    out.extend_from_slice(info);
    out.push(b'e');
    Ok((out, trackers))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A one-file torrent of `len` bytes named `name` (pieces of 16 KiB, zero hashes).
    pub fn one_file(name: &str, len: u64) -> Vec<u8> {
        let n = len.div_ceil(16384).max(1) as usize;
        let mut v = Vec::new();
        v.extend_from_slice(b"d4:infod6:lengthi");
        v.extend_from_slice(len.to_string().as_bytes());
        v.extend_from_slice(format!("e4:name{}:{}", name.len(), name).as_bytes());
        v.extend_from_slice(b"12:piece lengthi16384e6:pieces");
        v.extend_from_slice(format!("{}:", n * 20).as_bytes());
        v.extend(std::iter::repeat_n(0u8, n * 20));
        v.extend_from_slice(b"ee");
        v
    }

    fn multi(paths: &[&[&str]]) -> Vec<u8> {
        let mut v = b"d4:infod5:filesl".to_vec();
        for p in paths {
            v.extend_from_slice(b"d6:lengthi1e4:pathl");
            for c in *p {
                v.extend_from_slice(format!("{}:{}", c.len(), c).as_bytes());
            }
            v.extend_from_slice(b"ee");
        }
        let n = paths.len().div_ceil(16384).max(1);
        v.extend_from_slice(b"e4:name3:dir12:piece lengthi16384e6:pieces");
        v.extend_from_slice(format!("{}:", n * 20).as_bytes());
        v.extend(std::iter::repeat_n(0u8, n * 20));
        v.extend_from_slice(b"ee");
        v
    }

    #[test]
    fn a_plain_file_passes() {
        let s = check(&one_file("test.bin", 20 * 1024 * 1024)).unwrap();
        assert_eq!(s.name, "test.bin");
        assert_eq!(s.total, 20 * 1024 * 1024);
        assert_eq!(
            s.files,
            vec![(vec!["test.bin".to_string()], 20 * 1024 * 1024)]
        );
        let m = check(&multi(&[&["a", "b.txt"], &["c.txt"]])).unwrap();
        assert_eq!(m.files.len(), 2);
    }

    #[test]
    fn paths_that_leave_the_folder_are_refused() {
        for bad in [
            &["..", "etc", "passwd"][..],
            &["a", "..", "..", "x"][..],
            &["/etc/passwd"][..],
            &["a/b"][..],
            &["."][..],
            &[""][..],
        ] {
            assert!(
                matches!(check(&multi(&[bad])), Err(MetaError::Path(_))),
                "{bad:?}"
            );
        }
        assert!(matches!(check(&one_file("..", 1)), Err(MetaError::Path(_))));
        assert!(matches!(
            check(&one_file("a\\b", 1)),
            Err(MetaError::Path(_))
        ));
    }

    #[test]
    fn parsing_is_bounded() {
        // nesting
        let deep = format!("{}{}", "l".repeat(MAX_DEPTH + 2), "e".repeat(MAX_DEPTH + 2));
        assert_eq!(parse(deep.as_bytes()), Err(MetaError::TooDeep));
        // size
        let big = vec![b'0'; MAX_TORRENT + 1];
        assert_eq!(parse(&big), Err(MetaError::TooLarge));
        // a string length past the end
        assert_eq!(parse(b"99999999:x"), Err(MetaError::Malformed));
        // a huge length that would overflow
        assert_eq!(parse(b"999999999999999999999:x"), Err(MetaError::Malformed));
        // trailing data, unsorted keys, non-canonical integers
        assert_eq!(parse(b"i1ei2e"), Err(MetaError::Malformed));
        assert_eq!(parse(b"d1:bi1e1:ai2ee"), Err(MetaError::Malformed));
        assert_eq!(parse(b"i03e"), Err(MetaError::Malformed));
        assert_eq!(parse(b"i-0e"), Err(MetaError::Malformed));
        // unterminated
        assert_eq!(parse(b"l1:a"), Err(MetaError::Malformed));
        // items
        let many = format!("l{}e", "i1e".repeat(MAX_ITEMS + 1));
        assert!(matches!(
            parse(many.as_bytes()),
            Err(MetaError::TooManyItems) | Err(MetaError::TooLarge)
        ));
    }

    #[test]
    fn a_bare_info_dictionary_passes() {
        let t = one_file("f.bin", 100);
        let info = &t[b"d4:info".len()..t.len() - 1];
        assert_eq!(check(info).unwrap().name, "f.bin");
        assert_eq!(check(b"d1:ai1ee"), Err(MetaError::Malformed));
    }

    #[test]
    fn pieces_must_cover_the_length() {
        let mut t = one_file("x", 16384 * 3);
        // claim a longer file than the pieces cover
        let s = String::from_utf8_lossy(&t).replace("i49152e", "i99999e");
        t = s.into_bytes();
        assert_eq!(check(&t), Err(MetaError::Malformed));
    }

    /// `t` with an `announce` and an `announce-list` of `urls` (one tier each).
    fn with_trackers(t: &[u8], urls: &[String]) -> Vec<u8> {
        let mut out = b"d".to_vec();
        bstr(&mut out, b"announce");
        bstr(&mut out, urls[0].as_bytes());
        bstr(&mut out, b"announce-list");
        out.push(b'l');
        for u in urls {
            out.push(b'l');
            bstr(&mut out, u.as_bytes());
            out.push(b'e');
        }
        out.push(b'e');
        bstr(&mut out, b"comment");
        bstr(&mut out, b"x");
        out.extend_from_slice(&t[1..]); // "4:info...e" of the plain file
        out
    }

    #[test]
    fn trackers_are_bounded_and_filtered() {
        let t = one_file("f.bin", 100);
        let mut urls = vec!["http://127.0.0.1:631/".to_string()];
        urls.extend((0..5000).map(|i| format!("udp://t{i}.example.org:80/announce")));
        let big = with_trackers(&t, &urls);
        assert!(check(&big).is_ok()); // accepted as a file ...
        let (out, kept) = rebuild(&big, false).unwrap();
        assert_eq!(kept.len(), crate::trackers::MAX_TRACKERS); // ... but the engine gets 32
        assert_eq!(kept[0], "udp://t0.example.org:80/announce"); // and never the loopback one
        let out_s = String::from_utf8_lossy(&out);
        assert!(!out_s.contains("127.0.0.1") && !out_s.contains("comment"));
        // the same info dictionary: the same info hash
        assert_eq!(info_span(&out), info_span(&big));
        assert_eq!(check(&out).unwrap(), check(&big).unwrap());
        // no tracker at all: just the info
        let (bare, none) = rebuild(&t, false).unwrap();
        assert!(none.is_empty());
        assert_eq!(bare, t);
        // the bare info dictionary of a magnet's peers is wrapped
        let info = &t[b"d4:info".len()..t.len() - 1];
        assert_eq!(rebuild(info, false).unwrap().0, t);
    }

    #[test]
    fn garbage_never_panics() {
        // a cheap deterministic fuzz: every prefix and every one-byte change of a valid file
        let good = one_file("f", 40000);
        for i in 0..good.len() {
            let _ = check(&good[..i]);
            let mut m = good.clone();
            m[i] = m[i].wrapping_add(7);
            let _ = check(&m);
        }
    }
}
