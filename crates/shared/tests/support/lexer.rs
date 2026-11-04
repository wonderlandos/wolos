// SPDX-License-Identifier: BSD-2-Clause
//! The one Rust lexer of the shared source scanners (`rules_rust.rs`, `unsafe_ratchet.rs`;
//! audit8 standards #15). Include it with `#[path = "support/lexer.rs"] mod lexer;`.

/// A byte of an identifier.
pub fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// The code of `text` with comments, strings and char literals blanked (newlines kept, length
/// kept), and the byte spans of the string literals (quotes and prefix included).
pub fn lex(text: &str) -> (Vec<u8>, Vec<(usize, usize)>) {
    let b = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut lits = Vec::new();
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        out.extend(
            b[from..to.min(b.len())]
                .iter()
                .map(|c| if *c == b'\n' { b'\n' } else { b' ' }),
        );
    };
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let prev_ident = i > 0 && is_ident(b[i - 1]);
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            let end = b[i..]
                .iter()
                .position(|c| *c == b'\n')
                .map_or(b.len(), |n| i + n);
            blank(&mut out, i, end);
            i = end;
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            let (mut j, mut depth) = (i + 2, 1usize);
            while j < b.len() && depth > 0 {
                if b[j] == b'/' && b.get(j + 1) == Some(&b'*') {
                    depth += 1;
                    j += 2;
                } else if b[j] == b'*' && b.get(j + 1) == Some(&b'/') {
                    depth -= 1;
                    j += 2;
                } else {
                    j += 1;
                }
            }
            blank(&mut out, i, j);
            i = j;
        } else if !prev_ident && (c == b'r' || (c == b'b' && b.get(i + 1) == Some(&b'r'))) && {
            let k = i + 1 + usize::from(c == b'b');
            let h = b[k.min(b.len())..]
                .iter()
                .take_while(|c| **c == b'#')
                .count();
            b.get(k + h) == Some(&b'"')
        } {
            let k = i + 1 + usize::from(c == b'b');
            let h = b[k..].iter().take_while(|c| **c == b'#').count();
            let mut j = k + h + 1;
            while j < b.len()
                && !(b[j] == b'"' && b[j + 1..].iter().take(h).filter(|c| **c == b'#').count() == h)
            {
                j += 1;
            }
            let end = (j + 1 + h).min(b.len());
            lits.push((i, end));
            blank(&mut out, i, end);
            i = end;
        } else if c == b'"' {
            let mut j = i + 1;
            while j < b.len() && b[j] != b'"' {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            let end = (j + 1).min(b.len());
            lits.push((i, end));
            blank(&mut out, i, end);
            i = end;
        } else if c == b'\'' && (b.get(i + 1) == Some(&b'\\') || b.get(i + 2) == Some(&b'\'')) {
            let mut j = i + 1;
            while j < b.len() && b[j] != b'\'' {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            let end = (j + 1).min(b.len());
            blank(&mut out, i, end);
            i = end;
        } else {
            out.push(c);
            i += 1;
        }
    }
    (out, lits)
}
