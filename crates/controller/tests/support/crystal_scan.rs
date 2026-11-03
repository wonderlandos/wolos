// SPDX-License-Identifier: GPL-2.0-or-later
//! The Crystal lexer and the per-file scanners of `rules_crystal.rs` (docs/RULES-crystal.md): a small
//! scanner that blanks comments, strings, regexes and heredocs, the rule scanners that read what
//! is left, the shard.yml/shard.lock/build-file scanners, and the registry the fixtures run through.
#![allow(dead_code)] // the including test uses most of these, the fixture test the rest
use std::collections::BTreeMap;

/// RCR-043: top-level names of the Crystal standard library (`require "name/..."`); anything else
/// that is not a relative path is a shard.
pub const STDLIB: &[&str] = &[
    "annotations",
    "array",
    "atomic",
    "base64",
    "benchmark",
    "big",
    "bit_array",
    "bool",
    "box",
    "channel",
    "char",
    "class",
    "colorize",
    "comparable",
    "compiler",
    "complex",
    "compress",
    "concurrent",
    "crypto",
    "crystal",
    "csv",
    "deque",
    "digest",
    "dir",
    "ecr",
    "enum",
    "enumerable",
    "env",
    "errno",
    "exception",
    "fiber",
    "file",
    "file_utils",
    "float",
    "gc",
    "hash",
    "html",
    "http",
    "humanize",
    "indexable",
    "ini",
    "int",
    "io",
    "iterable",
    "iterator",
    "json",
    "kernel",
    "levenshtein",
    "log",
    "math",
    "mime",
    "mutex",
    "named_tuple",
    "nil",
    "number",
    "oauth",
    "oauth2",
    "object",
    "openssl",
    "option_parser",
    "path",
    "pointer",
    "prelude",
    "pretty_print",
    "proc",
    "process",
    "random",
    "range",
    "reference",
    "regex",
    "semantic_version",
    "set",
    "signal",
    "slice",
    "socket",
    "spec",
    "static_array",
    "string",
    "string_pool",
    "string_scanner",
    "struct",
    "symbol",
    "system",
    "termios",
    "time",
    "tuple",
    "unicode",
    "union",
    "uri",
    "uuid",
    "value",
    "weak_ref",
    "xml",
    "yaml",
];

// ---------------------------------------------------------------- the Crystal lexer
//
// A small scanner, not a parser: it blanks comments, string bodies, regex bodies, percent
// literals and heredoc bodies (keeping every byte offset and newline), so a pattern in a message
// or a PKGBUILD snippet inside a spec is not mistaken for code. `bare` has all of that blanked,
// `code` has only comments and heredocs blanked, and `strs` holds the string bodies by line.

pub struct Lexed {
    pub bare: String,
    pub code: String,
    pub strs: Vec<(usize, String)>,
    pub starts: Vec<usize>,
}

impl Lexed {
    pub fn line_of(&self, pos: usize) -> usize {
        match self.starts.binary_search(&pos) {
            Ok(i) => i + 1,
            Err(i) => i,
        }
    }
    pub fn bare_lines(&self) -> Vec<(usize, &str)> {
        self.bare
            .split('\n')
            .enumerate()
            .map(|(i, l)| (i + 1, l))
            .collect()
    }
}

pub fn blank(v: &mut [u8], a: usize, b: usize) {
    let end = b.min(v.len());
    for x in v.iter_mut().take(end).skip(a) {
        if *x != b'\n' {
            *x = b' ';
        }
    }
}

/// Index just after the closing quote of the double-quoted string that opens at `i`.
pub fn scan_dq(c: &[u8], i: usize) -> usize {
    let mut j = i + 1;
    while j < c.len() {
        match c[j] {
            b'\\' => j += 2,
            b'"' => return j + 1,
            b'#' if c.get(j + 1) == Some(&b'{') => j = skip_interp(c, j + 2),
            _ => j += 1,
        }
    }
    c.len()
}

/// Index just after the `}` that closes an interpolation whose body starts at `j`.
pub fn skip_interp(c: &[u8], mut j: usize) -> usize {
    let mut depth = 1;
    while j < c.len() {
        match c[j] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return j + 1;
                }
            }
            b'"' => {
                j = scan_dq(c, j);
                continue;
            }
            _ => {}
        }
        j += 1;
    }
    c.len()
}

pub fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

pub fn lex(text: &str) -> Lexed {
    let c = text.as_bytes();
    let n = c.len();
    let mut bare = c.to_vec();
    let mut code = c.to_vec();
    let mut strs = Vec::new();
    let mut starts = vec![0usize];
    for (i, b) in c.iter().enumerate() {
        if *b == b'\n' {
            starts.push(i + 1);
        }
    }
    let line_at = |p: usize, starts: &Vec<usize>| match starts.binary_search(&p) {
        Ok(i) => i + 1,
        Err(i) => i,
    };
    let mut pending: Vec<Vec<u8>> = Vec::new();
    let mut i = 0;
    while i < n {
        let ch = c[i];
        match ch {
            b'\n' => {
                i += 1;
                // heredoc bodies start on the line after their opener
                for term in std::mem::take(&mut pending) {
                    while i < n {
                        let e = c[i..].iter().position(|&x| x == b'\n').map_or(n, |p| i + p);
                        let is_end = std::str::from_utf8(&c[i..e])
                            .map(|l| l.trim().as_bytes() == term.as_slice())
                            .unwrap_or(false);
                        blank(&mut bare, i, e);
                        blank(&mut code, i, e);
                        i = (e + 1).min(n);
                        if is_end {
                            break;
                        }
                    }
                }
            }
            b'#' => {
                let e = c[i..].iter().position(|&x| x == b'\n').map_or(n, |p| i + p);
                blank(&mut bare, i, e);
                blank(&mut code, i, e);
                i = e;
            }
            b'"' => {
                let e = scan_dq(c, i);
                let body_end = e.saturating_sub(1).max(i + 1);
                strs.push((
                    line_at(i, &starts),
                    String::from_utf8_lossy(&c[i + 1..body_end]).into_owned(),
                ));
                blank(&mut bare, i + 1, body_end);
                i = e;
            }
            b'\'' => {
                // char literal: 'a', '\n', '\u{1F600}'
                let lim = (i + 12).min(n);
                let mut j = i + 1;
                if j < n && c[j] == b'\\' {
                    j += 2;
                } else {
                    j += 1;
                }
                while j < lim && c[j] != b'\'' && c[j] != b'\n' {
                    j += 1;
                }
                if j < n && c[j] == b'\'' && j > i + 1 {
                    blank(&mut bare, i + 1, j);
                    blank(&mut code, i + 1, j);
                    i = j + 1;
                } else {
                    i += 1;
                }
            }
            b'`' => {
                let e = c[i + 1..]
                    .iter()
                    .position(|&x| x == b'`')
                    .map_or(n, |p| i + 1 + p + 1);
                blank(&mut bare, i, e);
                bare[i] = 1; // marker: a command literal
                i = e;
            }
            b'%' => {
                let prev = if i > 0 { c[i - 1] } else { b'\n' };
                let nx = c.get(i + 1).copied().unwrap_or(0);
                let (kind, open) = if b"wWiIqQrx".contains(&nx)
                    && b"([{<|".contains(&c.get(i + 2).copied().unwrap_or(0))
                {
                    (nx, i + 2)
                } else if b"([{<".contains(&nx) {
                    (b'Q', i + 1)
                } else {
                    (0, 0)
                };
                if kind == 0 || is_ident(prev) || prev == b')' || prev == b']' || prev == b'}' {
                    i += 1;
                    continue;
                }
                let op = c[open];
                let cl = match op {
                    b'(' => b')',
                    b'[' => b']',
                    b'{' => b'}',
                    b'<' => b'>',
                    o => o,
                };
                let mut depth = 1;
                let mut j = open + 1;
                while j < n {
                    if c[j] == b'\\' {
                        j += 2;
                        continue;
                    }
                    if c[j] == cl && cl != op {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    } else if c[j] == cl {
                        break;
                    } else if c[j] == op {
                        depth += 1;
                    }
                    j += 1;
                }
                let j = j.min(n);
                if kind != b'r' {
                    strs.push((
                        line_at(i, &starts),
                        String::from_utf8_lossy(&c[open + 1..j]).into_owned(),
                    ));
                }
                blank(&mut bare, open + 1, j);
                if kind == b'x' {
                    bare[i] = 1;
                }
                i = (j + 1).min(n);
            }
            b'/' => {
                // a regex literal when a value cannot end just before it
                let mut k = i;
                while k > 0 && (c[k - 1] == b' ' || c[k - 1] == b'\t') {
                    k -= 1;
                }
                let prev = if k > 0 { c[k - 1] } else { b'\n' };
                let regex = (if prev == b'\n' || b"(,=~!|&{[:;?+-*%^<>".contains(&prev) {
                    true
                } else if is_ident(prev) {
                    let mut s = k;
                    while s > 0 && is_ident(c[s - 1]) {
                        s -= 1;
                    }
                    let w = &c[s..k];
                    [
                        &b"if"[..],
                        b"unless",
                        b"when",
                        b"and",
                        b"or",
                        b"not",
                        b"return",
                        b"puts",
                        b"in",
                        b"then",
                        b"while",
                        b"until",
                        b"elsif",
                        b"case",
                        b"else",
                    ]
                    .contains(&w)
                } else {
                    false
                }) && c.get(i + 1) != Some(&b'/');
                if !regex {
                    i += 1;
                    continue;
                }
                let mut j = i + 1;
                while j < n && c[j] != b'\n' {
                    if c[j] == b'\\' {
                        j += 2;
                        continue;
                    }
                    if c[j] == b'#' && c.get(j + 1) == Some(&b'{') {
                        j = skip_interp(c, j + 2);
                        continue;
                    }
                    if c[j] == b'/' {
                        break;
                    }
                    j += 1;
                }
                let j = j.min(n);
                if j < n && c[j] == b'/' {
                    blank(&mut bare, i + 1, j);
                    i = j + 1;
                } else {
                    i += 1;
                }
            }
            b'<' => {
                if c.get(i + 1) == Some(&b'<') && c.get(i + 2) == Some(&b'-') {
                    let mut j = i + 3;
                    let quote = matches!(c.get(j), Some(b'\'') | Some(b'"'));
                    if quote {
                        j += 1;
                    }
                    let s = j;
                    while j < n && is_ident(c[j]) {
                        j += 1;
                    }
                    if j > s && (c[s].is_ascii_uppercase() || c[s] == b'_') {
                        pending.push(c[s..j].to_vec());
                    }
                    i = j.max(i + 1);
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    Lexed {
        bare: String::from_utf8_lossy(&bare).into_owned(),
        code: String::from_utf8_lossy(&code).into_owned(),
        strs,
        starts,
    }
}

// ---------------------------------------------------------------- scanning helpers

/// One finding: (line, why).
pub type Finding = (usize, String);

pub struct Src {
    pub text: String,
    pub lx: Lexed,
}

impl Src {
    pub fn new(_rel: &str, text: &str) -> Src {
        Src {
            text: text.to_string(),
            lx: lex(text),
        }
    }
}

/// `tok` in `line` with identifier boundaries on the sides that are identifier characters.
pub fn has_tok(line: &str, tok: &str) -> bool {
    let b = line.as_bytes();
    let t = tok.as_bytes();
    let mut from = 0;
    while let Some(p) = line[from..].find(tok) {
        let s = from + p;
        let e = s + t.len();
        let before_ok = !is_ident(t[0]) || s == 0 || !is_ident(b[s - 1]);
        let after_ok = !is_ident(t[t.len() - 1]) || e >= b.len() || !is_ident(b[e]);
        if before_ok && after_ok {
            return true;
        }
        from = s + 1;
    }
    false
}

/// Byte ranges of the top-level, comma-separated pieces of the argument list of the call whose
/// name ends at `after` in `bare`: inside `( )` when there is one, else to the end of the line.
pub fn call_pieces(bare: &str, after: usize) -> Vec<(usize, usize)> {
    let b = bare.as_bytes();
    let mut a = after;
    while a < b.len() && (b[a] == b' ' || b[a] == b'\t') {
        a += 1;
    }
    let (start, end) = if a < b.len() && b[a] == b'(' {
        let mut depth = 0i32;
        let mut j = a;
        while j < b.len() {
            match b[j] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        (a + 1, j.min(b.len()))
    } else {
        let e = bare[a..].find('\n').map_or(b.len(), |p| a + p);
        (a, e)
    };
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut s = start;
    for (j, &ch) in b.iter().enumerate().take(end).skip(start) {
        match ch {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b',' if depth == 0 => {
                out.push((s, j));
                s = j + 1;
            }
            _ => {}
        }
    }
    if s < end || out.is_empty() {
        out.push((s, end));
    }
    out
}

/// `name: value` piece (a named argument), not `Foo::Bar` or a ternary.
pub fn named_arg(piece: &str) -> Option<&str> {
    let t = piece.trim_start();
    let end = t.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))?;
    if end == 0 {
        return None;
    }
    let rest = &t[end..];
    (rest.starts_with(':') && !rest.starts_with("::")).then_some(&t[..end])
}

pub fn find_all(hay: &str, needle: &str) -> Vec<usize> {
    let mut v = Vec::new();
    let mut from = 0;
    while let Some(p) = hay[from..].find(needle) {
        v.push(from + p);
        from += p + needle.len();
    }
    v
}

// ---------------------------------------------------------------- the per-file rules

/// RCR-001: line 1 is the SPDX line.
pub fn scan_spdx(s: &Src) -> Vec<Finding> {
    let l1 = s.text.lines().next().unwrap_or("");
    match l1.strip_prefix("# SPDX-License-Identifier: ") {
        Some(id) if !id.trim().is_empty() => vec![],
        _ => vec![(1, "line 1 is not `# SPDX-License-Identifier: <id>`".into())],
    }
}

/// RCR-002: a purpose comment: 40+ characters of comment (SPDX and notices left out) in the first
/// 30 lines. Specs name their unit in `describe` and are not asked.
pub fn scan_purpose(s: &Src) -> Vec<Finding> {
    let mut n = 0;
    for l in s.text.lines().take(30) {
        let t = l.trim_start();
        if let Some(c) = t.strip_prefix('#') {
            let c = c.trim();
            if !c.starts_with("SPDX-") && !c.starts_with("Portions:") {
                n += c.chars().count();
            }
        }
    }
    if n >= 40 {
        vec![]
    } else {
        vec![(
            2,
            format!("only {n} characters of purpose comment in the first 30 lines (need 40)"),
        )]
    }
}

/// RCR-003: no carriage returns, a final newline, no trailing blank line, no tabs in code.
pub fn scan_text_hygiene(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    if let Some(p) = s.text.find('\r') {
        v.push((s.lx.line_of(p), "carriage return".into()));
    }
    if !s.text.is_empty() && !s.text.ends_with('\n') {
        v.push((
            s.text.lines().count(),
            "no newline at the end of the file".into(),
        ));
    }
    if s.text.ends_with("\n\n") {
        v.push((
            s.text.lines().count(),
            "blank line at the end of the file".into(),
        ));
    }
    v
}

pub const UNSAFE_TOKENS: &[&str] = &[
    "Pointer",
    "pointerof",
    "uninitialized",
    "to_unsafe",
    "unsafe_as",
    "unsafe_fetch",
    "unsafe_put",
    "unsafe_chr",
    "realloc",
    "LibC",
    "address",
];

/// RCR-010: raw pointers, unchecked access and casts to pointer types.
pub fn scan_unsafe(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    for (n, l) in s.lx.bare_lines() {
        for t in UNSAFE_TOKENS {
            if has_tok(l, t) && (*t != "address" || l.contains(".address")) {
                v.push((n, format!("`{t}`")));
            }
        }
        // `Box(T).box(x)` / `Box(T).unbox(p)` turn an object into a raw pointer and back
        if l.contains("Box(") && has_tok(l, "Box") || l.contains(".unbox(") {
            v.push((n, "`Box` (object <-> raw pointer)".into()));
        }
        // `.as(Void*)`, `.as(UInt8*)`: a cast to a pointer type
        for p in find_all(l, ".as(") {
            if let Some(e) = l[p..].find(')')
                && l[p + 4..p + e].trim_end().ends_with('*')
            {
                v.push((n, "cast to a pointer type".into()));
            }
        }
    }
    v
}

/// RCR-011: `lib`, `fun` and `@[Link]` declarations.
pub fn scan_lib(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    for (n, l) in s.lx.bare_lines() {
        let t = l.trim_start();
        if t.starts_with("lib ") || t.starts_with("fun ") || t.starts_with("@[Link") {
            v.push((n, "C binding declaration".into()));
        }
    }
    v
}

pub const SHELLS: &[&str] = &[
    "sh",
    "bash",
    "dash",
    "zsh",
    "ksh",
    "fish",
    "/bin/sh",
    "/bin/bash",
    "/usr/bin/sh",
    "/usr/bin/bash",
];

/// RCR-020: a command goes out as program + argument array. Not allowed: `shell: true`, a shell
/// with `-c`, a program string that carries arguments or shell syntax, backticks, `%x()`,
/// `system()`.
pub fn scan_process(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    let bare = &s.lx.bare;
    if let Some(p) = bare.find('\u{1}') {
        v.push((s.lx.line_of(p), "command literal (backticks or %x)".into()));
    }
    for p in find_all(bare, "system(") {
        let prev = bare[..p].chars().last().unwrap_or(' ');
        if !is_ident(prev as u8) && prev != '.' && prev != ':' {
            v.push((s.lx.line_of(p), "`system(` runs a shell command".into()));
        }
    }
    for name in ["run", "new", "exec", "capture", "capture_result", "fork"] {
        let pat = format!("Process.{name}");
        for p in find_all(bare, &pat) {
            let end = p + pat.len();
            if bare.as_bytes().get(end).is_some_and(|b| is_ident(*b)) {
                continue;
            }
            let pieces = call_pieces(bare, end);
            let orig: Vec<&str> = pieces.iter().map(|(a, b)| &s.text[*a..*b]).collect();
            let bp: Vec<&str> = pieces.iter().map(|(a, b)| &bare[*a..*b]).collect();
            let line = s.lx.line_of(p);
            let positional: Vec<usize> = (0..bp.len())
                .filter(|i| named_arg(bp[*i]).is_none())
                .collect();
            for i in 0..bp.len() {
                if named_arg(bp[i]) == Some("shell") && orig[i].contains("true") {
                    v.push((line, "`shell: true`".into()));
                }
            }
            if let Some(&f) = positional.first() {
                let prog = orig[f].trim();
                if let Some(lit) = prog.strip_prefix('"').and_then(|x| x.strip_suffix('"')) {
                    let plain = strip_interp(lit);
                    if plain.contains(char::is_whitespace)
                        || plain.contains([';', '|', '&', '<', '>', '$', '`'])
                    {
                        v.push((
                            line,
                            format!("program string `{lit}` carries arguments or shell syntax"),
                        ));
                    }
                    if SHELLS.contains(&lit) && orig[f + 1..].iter().any(|o| has_dash_c(o)) {
                        v.push((line, format!("`{lit} -c`: a shell command string")));
                    }
                }
            }
        }
    }
    v
}

pub fn strip_interp(s: &str) -> String {
    let mut out = String::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'#' && b.get(i + 1) == Some(&b'{') {
            i = skip_interp(b, i + 2);
        } else {
            out.push(b[i] as char);
            i += 1;
        }
    }
    out
}

pub fn has_dash_c(piece: &str) -> bool {
    ["\"-c\"", "\"-lc\"", "\"-ec\"", "\"-ic\"", "%w(-c", "%w[-c"]
        .iter()
        .any(|p| piece.contains(p))
}

/// RCR-021: code that is built or loaded at run time.
pub fn scan_dynamic_code(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    for (n, l) in s.lx.bare_lines() {
        let mut rest = l;
        while let Some(p) = rest.find(['{', '%']) {
            let tail = &rest[p..];
            if tail.starts_with("{{") || tail.starts_with("{%") {
                let inner = &tail[2..];
                let inner = inner.split("}}").next().unwrap_or(inner);
                if has_tok(inner, "run") || has_tok(inner, "system") || inner.contains('\u{1}') {
                    v.push((n, "macro runs a program at compile time".into()));
                }
            }
            rest = &rest[p + 1..];
        }
        if has_tok(l, "dlopen") || has_tok(l, "dlsym") || has_tok(l, "Dl_info") {
            v.push((n, "dynamic library loading".into()));
        }
    }
    for (n, st) in &s.lx.strs {
        if st == "crystal" || st.starts_with("crystal ") {
            v.push((*n, "starts the compiler at run time".into()));
        }
        if st.starts_with("compiler/")
            && s.text
                .lines()
                .nth(*n - 1)
                .is_some_and(|l| l.trim_start().starts_with("require"))
        {
            v.push((*n, "requires the compiler".into()));
        }
    }
    v
}

pub const HTTP_CLASS: &[&str] = &[
    "get",
    "post",
    "put",
    "patch",
    "delete",
    "head",
    "exec",
    "options",
    "get_response",
];

/// RCR-022: network clients set connect and read timeouts. The `HTTP::Client.get` family cannot.
pub fn scan_http_timeouts(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    let bare = &s.lx.bare;
    for m in HTTP_CLASS {
        let pat = format!("HTTP::Client.{m}");
        for p in find_all(bare, &pat) {
            if bare
                .as_bytes()
                .get(p + pat.len())
                .is_some_and(|b| is_ident(*b))
            {
                continue;
            }
            v.push((
                s.lx.line_of(p),
                format!("`HTTP::Client.{m}` has no timeouts; build a client and set them"),
            ));
        }
    }
    let has_connect = bare.contains("connect_timeout");
    let has_read = bare.contains("read_timeout");
    for p in find_all(bare, "HTTP::Client.new") {
        if !(has_connect && has_read) {
            v.push((
                s.lx.line_of(p),
                "`HTTP::Client.new` without connect_timeout and read_timeout in the file".into(),
            ));
        }
    }
    for p in find_all(bare, "TCPSocket.new") {
        let pieces = call_pieces(bare, p + "TCPSocket.new".len());
        if !pieces
            .iter()
            .any(|(a, b)| bare[*a..*b].contains("connect_timeout"))
        {
            v.push((
                s.lx.line_of(p),
                "`TCPSocket.new` without connect_timeout".into(),
            ));
        }
    }
    v
}

/// RCR-023: TLS certificate and host name checks stay on.
pub fn scan_tls(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    for (n, l) in s.lx.bare_lines() {
        if l.contains("VerifyMode::NONE")
            || l.contains("verify_mode =")
            || l.contains("Context::Client.insecure")
            || has_tok(l, "insecure")
        {
            v.push((n, "TLS verification switched off or weakened".into()));
        }
    }
    v
}

/// RCR-024: untyped decoding of JSON and YAML. Specs are not asked (they read their own output).
pub fn scan_untyped_parse(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    for (n, l) in s.lx.bare_lines() {
        for pat in [
            "JSON.parse",
            "YAML.parse",
            "YAML::Nodes.parse",
            "JSON::PullParser.new",
            "YAML::PullParser.new",
        ] {
            for p in find_all(l, pat) {
                if !l
                    .as_bytes()
                    .get(p + pat.len())
                    .is_some_and(|b| is_ident(*b))
                    || pat.ends_with("parse")
                {
                    v.push((n, format!("`{pat}`")));
                }
            }
        }
    }
    v
}

/// RCR-025: nothing rescues `Exception` itself.
pub fn scan_rescue_exception(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    for (n, l) in s.lx.bare_lines() {
        if let Some(p) = l.find("rescue")
            && has_tok(&l[p..], "Exception")
        {
            v.push((n, "rescues `Exception`".into()));
        }
    }
    v
}

/// RCR-026: catch-all rescues: bare `rescue`, `rescue e` with no type, and the modifier form
/// `expr rescue value`. Specs are not asked.
pub fn scan_catch_all(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    for (n, l) in s.lx.bare_lines() {
        let t = l.trim();
        if t == "rescue" {
            v.push((n, "bare `rescue`".into()));
        } else if let Some(rest) = t.strip_prefix("rescue ") {
            let r = rest.trim();
            let simple =
                !r.is_empty() && r.bytes().all(is_ident) && r.as_bytes()[0].is_ascii_lowercase();
            if simple {
                v.push((n, "`rescue e` with no exception type".into()));
            }
        } else if has_tok(t, "rescue") && !t.starts_with("rescue") && !t.starts_with('#') {
            v.push((n, "modifier `rescue` swallows every exception".into()));
        }
    }
    v
}

/// RCR-030: fixed /tmp paths.
pub fn scan_tmp(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    for (n, st) in &s.lx.strs {
        let fixed = ["/tmp/", "/var/tmp/", "/dev/shm/"]
            .iter()
            .any(|p| st.contains(p))
            || st
                .split_whitespace()
                .any(|w| w == "/tmp" || w == "/var/tmp")
            || st == "/tmp";
        let tempdir_name = st.contains("Dir.tempdir}/") && !st.ends_with("Dir.tempdir}/");
        if fixed || tempdir_name {
            v.push((
                *n,
                format!(
                    "fixed temporary path in `{}`",
                    st.chars().take(50).collect::<String>()
                ),
            ));
        }
    }
    for (n, l) in s.lx.code.lines().enumerate() {
        if l.contains("Dir.tempdir,") && l.contains("File.join") {
            v.push((n + 1, "File.join(Dir.tempdir, <fixed name>)".into()));
        }
    }
    v
}

/// RCR-031: nothing is created or made world-writable.
pub fn scan_world_writable(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    for (n, l) in s.lx.bare_lines() {
        if l.contains("OtherWrite") || l.contains("Permissions::All") {
            v.push((n, "world-writable permission flag".into()));
        }
        let ctx = [
            "chmod",
            "perm",
            "mkdir",
            "File.new",
            "File.open",
            "File.write",
            "umask",
            "mkfifo",
            "tempfile",
            "File.copy",
        ]
        .iter()
        .any(|w| l.contains(w));
        if ctx {
            for p in find_all(l, "0o") {
                let digits: String = l[p + 2..].chars().take_while(|c| c.is_digit(8)).collect();
                if digits.len() >= 3 && matches!(digits.chars().last(), Some('2' | '3' | '6' | '7'))
                {
                    v.push((n, format!("mode 0o{digits} lets other users write")));
                }
            }
        }
    }
    for (n, st) in &s.lx.strs {
        let t = st.trim();
        let mode_word = matches!(
            t,
            "777"
                | "0777"
                | "666"
                | "0666"
                | "1777"
                | "a+w"
                | "o+w"
                | "go+w"
                | "ugo+w"
                | "a+rwx"
                | "o+rwx"
                | "a=rwx"
        );
        if mode_word
            || t.contains(" 777")
            || t.contains("chmod 666")
            || t.contains("a+w")
            || t.contains("o+w")
        {
            v.push((*n, format!("`{t}` makes something world-writable")));
        }
    }
    v
}

/// RCR-032: no MD5 or SHA-1 where a digest protects anything.
pub fn scan_weak_hash(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    for (n, l) in s.lx.bare_lines() {
        if l.contains("Digest::MD5") || l.contains("Digest::SHA1") || l.contains("Digest::MD4") {
            v.push((n, "MD5 or SHA-1 digest".into()));
        }
    }
    for (n, st) in &s.lx.strs {
        let up = st.to_ascii_uppercase();
        if matches!(up.as_str(), "MD5" | "MD4" | "SHA1")
            || st == "digest/md5"
            || st == "digest/sha1"
        {
            v.push((*n, format!("`{st}`")));
        }
    }
    v
}

/// RCR-033: secrets come from `Random::Secure`.
pub fn scan_random(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    for (n, l) in s.lx.bare_lines() {
        if l.contains("Random::Secure") {
            continue;
        }
        if l.contains("Random.rand")
            || l.contains("Random.new")
            || l.contains("Random::DEFAULT")
            || l.contains("Random::PCG32")
            || l.contains("Random::ISAAC")
            || has_tok(l, "rand") && l.contains("rand(") && !l.contains(".rand(")
        {
            v.push((n, "non-secure random generator".into()));
        }
    }
    v
}

/// RCR-034: no literal credentials.
pub fn scan_credentials(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    for (n, l) in s.lx.code.lines().enumerate() {
        let low = l.to_ascii_lowercase();
        for key in [
            "password",
            "passwd",
            "passphrase",
            "secret",
            "token",
            "api_key",
            "apikey",
            "private_key",
        ] {
            if let Some(p) = low.find(key) {
                let after = low[p + key.len()..]
                    .trim_start_matches(|c: char| is_ident(c as u8))
                    .trim_start();
                if (after.starts_with("= \"")
                    || after.starts_with("=\"")
                    || after.starts_with(": \""))
                    && !after.contains("#{")
                {
                    let lit = after
                        .trim_start_matches(['=', ':', ' '])
                        .trim_start_matches('"');
                    let body = lit.split('"').next().unwrap_or("");
                    if body.len() >= 4 {
                        v.push((n + 1, format!("literal value assigned to `{key}`")));
                    }
                }
            }
        }
    }
    for (n, l) in s.text.lines().enumerate() {
        if l.contains("-----BEGIN") && l.contains("PRIVATE KEY")
            || l.contains("ghp_") && l.len() > 40
            || l.contains("AKIA") && l.matches(char::is_uppercase).count() > 12
        {
            v.push((n + 1, "a key or token pasted in the source".into()));
        }
    }
    v
}

/// RCR-035: no /home/ or /Users/ literals.
pub fn scan_home(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    for (n, st) in &s.lx.strs {
        if st.contains("/home/") || st.contains("/Users/") {
            v.push((*n, "home directory literal".into()));
        }
    }
    v
}

/// RCR-043: a `require` names the standard library or a file of this repo.
pub fn scan_requires(s: &Src) -> Vec<Finding> {
    let mut v = Vec::new();
    for (n, l) in s.lx.code.lines().enumerate() {
        let t = l.trim_start();
        let Some(r) = t.strip_prefix("require ") else {
            continue;
        };
        let name = r.trim().trim_matches('"');
        if name.starts_with("./") || name.starts_with("../") {
            continue;
        }
        let top = name.split('/').next().unwrap_or("");
        if !STDLIB.contains(&top) {
            v.push((
                n + 1,
                format!("`require \"{name}\"` is not the standard library: a shard"),
            ));
        }
    }
    v
}

// ---------------------------------------------------------------- shard.yml, shard.lock, build files

#[derive(Debug, Default, Clone)]
pub struct Dep {
    pub name: String,
    pub line: usize,
    pub keys: BTreeMap<String, String>,
}

/// The `dependencies:` and `development_dependencies:` entries of a shard.yml, and its top keys.
pub fn parse_shard_yml(text: &str) -> (BTreeMap<String, String>, Vec<Dep>) {
    let mut top = BTreeMap::new();
    let mut deps: Vec<Dep> = Vec::new();
    let mut in_deps = false;
    for (i, raw) in text.lines().enumerate() {
        let l = raw.split(" #").next().unwrap_or(raw);
        if l.trim().is_empty() || l.trim_start().starts_with('#') {
            continue;
        }
        let indent = l.len() - l.trim_start().len();
        let t = l.trim();
        if indent == 0 {
            in_deps = t == "dependencies:" || t == "development_dependencies:";
            if let Some((k, v)) = t.split_once(':') {
                top.insert(k.trim().to_string(), v.trim().trim_matches('"').to_string());
            }
        } else if in_deps {
            if indent <= 2 && t.ends_with(':') {
                deps.push(Dep {
                    name: t.trim_end_matches(':').to_string(),
                    line: i + 1,
                    keys: BTreeMap::new(),
                });
            } else if let (Some(d), Some((k, v))) = (deps.last_mut(), t.split_once(':')) {
                d.keys
                    .insert(k.trim().to_string(), v.trim().trim_matches('"').to_string());
            }
        }
    }
    (top, deps)
}

pub fn is_sha40(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// RCR-040: shard.yml names `crystal:` and a licence, and every dependency is pinned by a full
/// commit. A tag or a version range can move.
pub fn scan_shard_yml(text: &str) -> Vec<Finding> {
    let (top, deps) = parse_shard_yml(text);
    let mut v = Vec::new();
    for k in ["name", "version", "crystal", "license"] {
        if !top.contains_key(k) {
            v.push((1, format!("shard.yml has no `{k}:`")));
        }
    }
    for d in &deps {
        match d.keys.get("commit") {
            Some(c) if is_sha40(c) => {}
            Some(c) => v.push((
                d.line,
                format!("{}: commit `{c}` is not a full 40-character hash", d.name),
            )),
            None => v.push((
                d.line,
                format!(
                    "{}: not pinned by `commit:` (a tag, branch or version range can move)",
                    d.name
                ),
            )),
        }
        for k in ["branch", "tag", "path"] {
            if d.keys.contains_key(k) {
                v.push((d.line, format!("{}: `{k}:` is not a pin", d.name)));
            }
        }
        if let Some(ver) = d.keys.get("version") {
            v.push((
                d.line,
                format!(
                    "{}: `version: {ver}` beside the commit (a range or tag moves)",
                    d.name
                ),
            ));
        }
    }
    v
}

/// The `name -> version` entries of a shard.lock.
pub fn parse_shard_lock(text: &str) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    let mut cur: Option<String> = None;
    let mut in_shards = false;
    for raw in text.lines() {
        let indent = raw.len() - raw.trim_start().len();
        let t = raw.trim();
        if t.is_empty() {
            continue;
        }
        if indent == 0 {
            in_shards = t == "shards:";
        } else if in_shards && indent == 2 && t.ends_with(':') {
            cur = Some(t.trim_end_matches(':').to_string());
            m.insert(cur.clone().unwrap(), String::new());
        } else if in_shards
            && indent >= 4
            && let (Some(c), Some(v)) = (&cur, t.strip_prefix("version:"))
        {
            m.insert(c.clone(), v.trim().trim_matches('"').to_string());
        }
    }
    m
}

/// RCR-041: a shard.yml with dependencies has a shard.lock, the lock names every dependency, and
/// the locked version of each carries the commit that shard.yml pins.
pub fn scan_shard_lock(yml: &str, lock: Option<&str>) -> Vec<Finding> {
    let (_, deps) = parse_shard_yml(yml);
    if deps.is_empty() {
        return vec![];
    }
    let Some(lock) = lock else {
        return vec![(
            1,
            "shard.yml has dependencies but there is no shard.lock".into(),
        )];
    };
    let locked = parse_shard_lock(lock);
    let mut v = Vec::new();
    for d in &deps {
        match locked.get(&d.name) {
            None => v.push((d.line, format!("{}: not in shard.lock", d.name))),
            Some(ver) => {
                if let Some(c) = d.keys.get("commit")
                    && !ver.contains(&format!("git.commit.{c}"))
                {
                    v.push((
                        d.line,
                        format!("{}: shard.lock says `{ver}`, shard.yml pins {c}", d.name),
                    ));
                }
            }
        }
    }
    v
}

/// RCR-044: a build file that runs `shards` fetches nothing from the network: `install` and
/// `update` are not run; `shards build/check/run` only with `--frozen --local`.
pub fn scan_shards_network(text: &str) -> Vec<Finding> {
    let mut v = Vec::new();
    for (n, l) in text.lines().enumerate() {
        let t = l.trim_start();
        if t.starts_with('#') {
            continue;
        }
        let code = l.split(" #").next().unwrap_or(l);
        let words: Vec<&str> = code.split_whitespace().collect();
        for (i, w) in words.iter().enumerate() {
            let cmd = w.trim_start_matches(['(', '"', '\'', '`', '$']);
            if cmd != "shards" {
                continue;
            }
            let Some(sub) = words.get(i + 1) else {
                continue;
            };
            if ["install", "update", "build", "check", "run", "prune"].contains(sub) {
                let rest = &words[i + 1..];
                if !(rest.contains(&"--frozen") && rest.contains(&"--local")) {
                    v.push((
                        n + 1,
                        format!("`shards {sub}` without --frozen --local fetches from the network"),
                    ));
                }
            }
        }
    }
    v
}

// ---------------------------------------------------------------- the registry used by the tests and fixtures

pub type Scanner = fn(&Src) -> Vec<Finding>;

/// Every per-file `.cr` scanner by rule id (what a fixture is run through).
pub const FILE_SCANNERS: &[(&str, Scanner)] = &[
    ("RCR-001", scan_spdx),
    ("RCR-002", scan_purpose),
    ("RCR-003", scan_text_hygiene),
    ("RCR-010", scan_unsafe),
    ("RCR-011", scan_lib),
    ("RCR-020", scan_process),
    ("RCR-021", scan_dynamic_code),
    ("RCR-022", scan_http_timeouts),
    ("RCR-023", scan_tls),
    ("RCR-024", scan_untyped_parse),
    ("RCR-025", scan_rescue_exception),
    ("RCR-026", scan_catch_all),
    ("RCR-030", scan_tmp),
    ("RCR-031", scan_world_writable),
    ("RCR-032", scan_weak_hash),
    ("RCR-033", scan_random),
    ("RCR-034", scan_credentials),
    ("RCR-035", scan_home),
    ("RCR-043", scan_requires),
];

pub fn scanner(id: &str) -> Scanner {
    FILE_SCANNERS.iter().find(|(i, _)| *i == id).unwrap().1
}
