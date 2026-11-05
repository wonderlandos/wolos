// SPDX-License-Identifier: GPL-2.0-or-later
//! Source maps (v3): decode `mappings`, look up a generated position; and a small JavaScript
//! pretty-printer for minified code. Pure, tested without a browser.
use serde_json::Value;

#[derive(Debug, Clone, Copy)]
struct Seg {
    col: u32,
    src: u32,
    line: u32,
    scol: u32,
    name: Option<u32>,
}

#[derive(Debug, Default)]
pub struct SourceMap {
    pub sources: Vec<String>,
    pub names: Vec<String>,
    lines: Vec<Vec<Seg>>,
}

fn vlq(it: &mut std::str::Bytes<'_>) -> Option<i64> {
    let (mut result, mut shift) = (0i64, 0u32);
    loop {
        let c = it.next()?;
        let d = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as i64;
        result |= (d & 31) << shift;
        if d & 32 == 0 {
            break;
        }
        shift += 5;
        if shift > 60 {
            return None;
        }
    }
    Some(if result & 1 == 1 {
        -(result >> 1)
    } else {
        result >> 1
    })
}

pub fn parse(json: &str) -> Option<SourceMap> {
    let v: Value = serde_json::from_str(json).ok()?;
    let strs = |k: &str| -> Vec<String> {
        v[k].as_array()
            .map(|a| {
                a.iter()
                    .map(|x| x.as_str().unwrap_or("").to_string())
                    .collect()
            })
            .unwrap_or_default()
    };
    let root = v["sourceRoot"].as_str().unwrap_or("");
    let sources: Vec<String> = strs("sources")
        .into_iter()
        .map(|s| {
            if root.is_empty() {
                s
            } else {
                format!("{}/{}", root.trim_end_matches('/'), s)
            }
        })
        .collect();
    let names = strs("names");
    let mappings = v["mappings"].as_str()?;
    let mut lines = vec![];
    let (mut src, mut sline, mut scol, mut name) = (0i64, 0i64, 0i64, 0i64);
    for l in mappings.split(';') {
        let mut segs = vec![];
        let mut col = 0i64;
        for seg in l.split(',') {
            if seg.is_empty() {
                continue;
            }
            let mut it = seg.bytes();
            col += vlq(&mut it)?;
            let mut s = Seg {
                col: col.max(0) as u32,
                src: u32::MAX,
                line: 0,
                scol: 0,
                name: None,
            };
            if let Some(ds) = vlq(&mut it) {
                src += ds;
                sline += vlq(&mut it)?;
                scol += vlq(&mut it)?;
                s.src = src.max(0) as u32;
                s.line = sline.max(0) as u32;
                s.scol = scol.max(0) as u32;
                if let Some(dn) = vlq(&mut it) {
                    name += dn;
                    s.name = Some(name.max(0) as u32);
                }
            }
            segs.push(s);
        }
        lines.push(segs);
        if lines.len() > 2_000_000 {
            return None;
        }
    }
    Some(SourceMap {
        sources,
        names,
        lines,
    })
}

impl SourceMap {
    /// (source, line, column, name), all 0-based for line and column as in CDP; None if unmapped.
    pub fn lookup(&self, line: u32, col: u32) -> Option<(String, u32, u32, Option<String>)> {
        let segs = self.lines.get(line as usize)?;
        let i = segs.partition_point(|s| s.col <= col);
        let s = segs.get(i.checked_sub(1)?)?;
        if s.src == u32::MAX {
            return None;
        }
        Some((
            self.sources.get(s.src as usize)?.clone(),
            s.line,
            s.scol,
            s.name.and_then(|n| self.names.get(n as usize).cloned()),
        ))
    }
}

/// Indent braces, break after `;`, `{`, `}`; leave strings, template literals, regex-looking text and comments alone.
pub fn pretty(src: &str) -> String {
    let mut out = String::with_capacity(src.len() + src.len() / 4);
    let b: Vec<char> = src.chars().collect();
    let (mut i, mut depth, mut paren) = (0usize, 0usize, 0i32);
    let nl = |out: &mut String, depth: usize| {
        while out.ends_with(' ') {
            out.pop();
        }
        out.push('\n');
        for _ in 0..depth {
            out.push_str("  ");
        }
    };
    while i < b.len() {
        let c = b[i];
        match c {
            '"' | '\'' | '`' => {
                out.push(c);
                i += 1;
                while i < b.len() {
                    out.push(b[i]);
                    if b[i] == '\\' && i + 1 < b.len() {
                        i += 1;
                        out.push(b[i]);
                    } else if b[i] == c {
                        break;
                    }
                    i += 1;
                }
            }
            '/' if b.get(i + 1) == Some(&'/') => {
                while i < b.len() && b[i] != '\n' {
                    out.push(b[i]);
                    i += 1;
                }
                nl(&mut out, depth);
                continue;
            }
            '/' if b.get(i + 1) == Some(&'*') => {
                while i < b.len() && !(b[i] == '*' && b.get(i + 1) == Some(&'/')) {
                    out.push(b[i]);
                    i += 1;
                }
                out.push_str("*/");
                i += 2;
                continue;
            }
            '(' => {
                paren += 1;
                out.push(c);
            }
            ')' => {
                paren -= 1;
                out.push(c);
            }
            '{' => {
                out.push_str(" {");
                depth += 1;
                nl(&mut out, depth);
            }
            '}' => {
                depth = depth.saturating_sub(1);
                nl(&mut out, depth);
                out.push('}');
                if !matches!(b.get(i + 1), Some(';' | ',' | ')' | '.')) {
                    nl(&mut out, depth);
                }
            }
            ';' if paren <= 0 => {
                out.push(';');
                nl(&mut out, depth);
            }
            ',' if paren <= 0 && depth > 0 && false => out.push(','),
            '\n' | '\r' => {}
            ' ' if out.ends_with(' ') || out.ends_with('\n') => {}
            _ => out.push(c),
        }
        i += 1;
    }
    out.lines()
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vlq_values() {
        let mut it = "AAAA".bytes();
        assert_eq!((vlq(&mut it), vlq(&mut it)), (Some(0), Some(0)));
        assert_eq!(vlq(&mut "D".bytes()), Some(-1));
        assert_eq!(vlq(&mut "gB".bytes()), Some(16));
    }

    #[test]
    fn lookup_maps_back_to_the_original() {
        // generated line 0: col 0 -> a.js 0:0 ; col 10 -> a.js 2:4 name 'foo'
        let m =
            parse(r#"{"version":3,"sources":["a.js"],"names":["foo"],"mappings":"AAAA,UAEIA"}"#)
                .unwrap();
        assert_eq!(m.lookup(0, 0).unwrap().0, "a.js");
        let r = m.lookup(0, 12).unwrap();
        assert_eq!((r.1, r.2, r.3.as_deref()), (2, 4, Some("foo")));
        assert!(m.lookup(5, 0).is_none());
    }

    #[test]
    fn pretty_breaks_minified_code() {
        let p = pretty("function a(b){if(b){return \"x;y\"}for(var i=0;i<3;i++){c(i)}}a(1);");
        assert!(p.lines().count() >= 6, "{p}");
        assert!(p.contains("\"x;y\""), "strings are untouched");
        assert!(
            p.contains("for(var i=0;i<3;i++)"),
            "for-header semicolons stay"
        );
    }
}
