// SPDX-License-Identifier: BSD-2-Clause
//! The `.wolcss` parser: a small CSS-like subset. One `Builder` takes any number of sheets and
//! produces a `StyleTable` plus diagnostics. `wolkit-stylec` and the engine's dev loader both use
//! it, so the two cannot diverge.
//!
//! ```text
//! .card        { background: card; padding: md; border-radius: m; }
//! .card:hover  { background: control; }
//! .row:hover > .actions { opacity: 1; }
//! @dark        { .card { box-shadow: md; } }
//! button       { cursor: pointer; transition: background fast ease; }
//! ```
use super::lint::{self as L, Diag, Lines, Severity};
use super::prop::*;
use super::select::*;
use super::tokens as tk;
use super::value::*;
use super::vparse::*;
use std::collections::HashMap;

/// The pragma that lifts the spacing and radius scale for a whole file (counted by a ratchet).
pub const PRAGMA_OFF_SCALE: &str = "wolkit: off-scale";

/// CSS names that exist but are not in 1.0; they get WK-009 instead of "unknown property".
fn is_unsupported_property(n: &str) -> bool {
    n.starts_with("grid")
        || n.starts_with("animation")
        || n.starts_with("transition-")
        || n.starts_with("overflow-")
        || n.starts_with("border-image")
        || n.starts_with("mask")
        || n.starts_with("scroll-")
        || n.starts_with("flex-") && !matches!(n, "flex-direction" | "flex-wrap")
        || matches!(
            n,
            "float"
                | "clear"
                | "filter"
                | "backdrop-filter"
                | "mix-blend-mode"
                | "text-shadow"
                | "flex"
                | "flex-flow"
                | "place-items"
                | "place-content"
                | "place-self"
                | "order"
                | "content"
                | "clip-path"
                | "will-change"
                | "contain"
                | "writing-mode"
                | "direction"
                | "font"
                | "background-image"
                | "background-color"
                | "background-size"
                | "background-position"
                | "background-repeat"
                | "list-style"
                | "table-layout"
                | "vertical-align"
                | "word-break"
                | "overflow-wrap"
                | "text-transform"
                | "text-indent"
                | "border-style"
                | "box-sizing"
                | "all"
                | "src"
        )
}

#[derive(Clone, Debug)]
struct Sel {
    /// Class name or kind id.
    class: Option<String>,
    kind: Option<u8>,
    state: u16,
    parent: Option<(String, u16)>,
    s: usize,
    e: usize,
}

/// Collects sheets into one table.
pub struct Builder {
    pub table: StyleTable,
    index: HashMap<String, u32>,
    pub diags: Vec<Diag>,
    /// Files that carry the off-scale pragma.
    pub optouts: u32,
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

struct FileCx<'a> {
    file: &'a str,
    /// Source with comment text blanked (offsets unchanged).
    src: &'a str,
    lines: Lines<'a>,
    off_scale: bool,
}

impl Builder {
    pub fn new() -> Self {
        Builder {
            table: StyleTable::default(),
            index: HashMap::new(),
            diags: vec![],
            optouts: 0,
        }
    }

    pub fn has_errors(&self) -> bool {
        self.diags.iter().any(|d| d.is_error())
    }

    /// Finish: variants in resolution order.
    pub fn finish(mut self) -> (StyleTable, Vec<Diag>) {
        for c in self
            .table
            .classes
            .iter_mut()
            .chain(self.table.kinds.iter_mut())
        {
            c.sort();
        }
        (self.table, self.diags)
    }

    fn diag(
        &mut self,
        cx: &FileCx,
        id: &'static str,
        s: usize,
        e: usize,
        msg: String,
        help: Option<String>,
    ) {
        let (line, col, text) = cx.lines.locate(s);
        let sev = L::def(id).map_or(Severity::Error, |d| d.severity);
        self.diags.push(Diag {
            id,
            severity: sev,
            file: cx.file.to_string(),
            line,
            col,
            len: cx.lines.span_chars(s, e),
            message: msg,
            help,
            source_line: text.to_string(),
        });
    }

    fn perr(&mut self, cx: &FileCx, e: PErr) {
        self.diag(cx, e.id, e.s, e.e, e.msg, e.help);
    }

    /// Parse one sheet. `file` is only used in diagnostics.
    pub fn add(&mut self, file: &str, original: &str) {
        let (clean, pragma, unterminated) = blank_comments(original);
        let lines = Lines::new(original);
        let off_scale = pragma.is_some();
        let cx = FileCx {
            file,
            src: &clean,
            lines,
            off_scale,
        };
        if let Some(at) = unterminated {
            self.diag(
                &cx,
                L::SYNTAX,
                at,
                at + 2,
                "This comment is never closed.".into(),
                Some("end it with */".into()),
            );
        }
        if let Some(at) = pragma {
            self.optouts += 1;
            self.diag(
                &cx,
                L::OPT_OUT,
                at,
                at + 2,
                "This file opts out of the spacing and radius scale.".into(),
                Some("opt-outs are counted; prefer the named steps".into()),
            );
        }
        let mut i = 0;
        let n = clean.len();
        self.items(&cx, &mut i, n, 0, true);
    }

    fn skip_ws(src: &str, i: &mut usize, end: usize) {
        let b = src.as_bytes();
        while *i < end && b[*i].is_ascii_whitespace() {
            *i += 1;
        }
    }

    /// Items between `*i` and `end`: rules and (at top level) media blocks.
    fn items(&mut self, cx: &FileCx, i: &mut usize, end: usize, media: u8, top: bool) {
        let b = cx.src.as_bytes();
        loop {
            Self::skip_ws(cx.src, i, end);
            if *i >= end {
                return;
            }
            let start = *i;
            match b[*i] {
                b'}' => {
                    self.diag(
                        cx,
                        L::SYNTAX,
                        start,
                        start + 1,
                        "This `}` closes nothing.".into(),
                        Some("remove it".into()),
                    );
                    *i += 1;
                }
                b'@' => {
                    if !top {
                        self.diag(
                            cx,
                            L::SELECTOR_FORM,
                            start,
                            start + 1,
                            "Media blocks do not nest.".into(),
                            Some("put the condition outside, as @dark @contrast { ... }".into()),
                        );
                    }
                    self.at_rule(cx, i, end, top);
                }
                _ => self.rule(cx, i, end, media),
            }
        }
    }

    fn find_block_end(&mut self, cx: &FileCx, open: usize, end: usize) -> Option<usize> {
        let b = cx.src.as_bytes();
        let mut j = open + 1;
        let mut depth = 1;
        while j < end {
            match b[j] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(j);
                    }
                }
                _ => {}
            }
            j += 1;
        }
        self.diag(
            cx,
            L::SYNTAX,
            open,
            open + 1,
            "This block is never closed.".into(),
            Some("add a closing }".into()),
        );
        None
    }

    fn at_rule(&mut self, cx: &FileCx, i: &mut usize, end: usize, top: bool) {
        let b = cx.src.as_bytes();
        let start = *i;
        let mut media = 0u8;
        let mut ok = true;
        let mut names_end = start;
        // One or more `@name` words, then `{`.
        loop {
            Self::skip_ws(cx.src, i, end);
            if *i < end && b[*i] == b'@' {
                let ns = *i + 1;
                let mut j = ns;
                while j < end && (b[j].is_ascii_alphanumeric() || b[j] == b'-') {
                    j += 1;
                }
                let name = &cx.src[ns..j];
                match media_bit(name) {
                    Some(bit) => media |= bit,
                    None => {
                        ok = false;
                        let (id, msg, help) = if matches!(
                            name,
                            "import"
                                | "font-face"
                                | "keyframes"
                                | "media"
                                | "supports"
                                | "layer"
                                | "container"
                                | "namespace"
                                | "charset"
                                | "property"
                        ) {
                            (L::UNSUPPORTED, format!("`@{name}` is not in version 1.0."), Some("styles are plain classes; conditions are @dark, @light, @contrast, @reduce-motion, @reduce-transparency and @compact".to_string()))
                        } else {
                            let help = tk::suggest(name, MEDIA.iter().copied())
                                .map(|n| format!("did you mean `@{n}`?"));
                            (L::UNKNOWN_STATE, format!("`@{name}` is not a media condition."), help.or(Some("the conditions are dark, light, contrast, reduce-motion, reduce-transparency and compact".into())))
                        };
                        self.diag(cx, id, ns - 1, j, msg, help);
                    }
                }
                *i = j.max(ns);
                names_end = j.max(ns);
                if j == ns {
                    *i = ns;
                }
            } else {
                break;
            }
        }
        if media & MEDIA_DARK != 0 && media & MEDIA_LIGHT != 0 {
            ok = false;
            self.diag(
                cx,
                L::CONFLICTING_MEDIA,
                start,
                names_end,
                "@dark and @light never hold together.".into(),
                Some("write two blocks, one for each".into()),
            );
        }
        Self::skip_ws(cx.src, i, end);
        if *i < end && b[*i] == b'{' {
            let open = *i;
            if let Some(close) = self.find_block_end(cx, open, end) {
                *i = open + 1;
                if ok && top {
                    self.items(cx, i, close, media, false);
                }
                *i = close + 1;
            } else {
                *i = end;
            }
        } else {
            // `@import "x";`-style statement: skip to `;`.
            while *i < end && b[*i] != b';' && b[*i] != b'{' {
                *i += 1;
            }
            if *i < end && b[*i] == b'{' {
                if let Some(close) = self.find_block_end(cx, *i, end) {
                    *i = close + 1;
                } else {
                    *i = end;
                }
            } else {
                *i = (*i + 1).min(end);
            }
        }
    }

    fn rule(&mut self, cx: &FileCx, i: &mut usize, end: usize, media: u8) {
        let b = cx.src.as_bytes();
        let sel_start = *i;
        let mut j = *i;
        while j < end && b[j] != b'{' && b[j] != b'}' && b[j] != b';' {
            j += 1;
        }
        if j >= end || b[j] != b'{' {
            self.diag(
                cx,
                L::SYNTAX,
                sel_start,
                (sel_start + 1).min(end),
                "A selector is followed by a block in braces.".into(),
                Some("write .name { property: value; }".into()),
            );
            *i = (j + 1).min(end);
            return;
        }
        let open = j;
        let Some(close) = self.find_block_end(cx, open, end) else {
            *i = end;
            return;
        };
        *i = close + 1;
        // Selectors.
        let mut sels = Vec::new();
        let mut part_start = sel_start;
        let mut depth = 0i32;
        for k in sel_start..=open {
            let ch = if k < open {
                b.get(k).copied().unwrap_or(b',')
            } else {
                b','
            };
            match ch {
                b'(' => depth += 1,
                b')' => depth -= 1,
                b',' if depth <= 0 => {
                    if let Some(s) = self.selector(cx, part_start, k) {
                        sels.push(s);
                    }
                    part_start = k + 1;
                }
                _ => {}
            }
        }
        // Declarations.
        let (decls, any) = self.declarations(cx, open + 1, close);
        if !any {
            let sel_end = sel_start + cx.src[sel_start..open].trim_end().len();
            self.diag(
                cx,
                L::EMPTY_RULE,
                sel_start,
                sel_end,
                "This rule sets nothing.".into(),
                Some("add a declaration or remove the rule".into()),
            );
            return;
        }
        if decls.is_empty() {
            return;
        }
        for s in sels {
            self.register(cx, &s, media, &decls);
        }
    }

    fn selector(&mut self, cx: &FileCx, from: usize, to: usize) -> Option<Sel> {
        let raw = &cx.src[from..to];
        let lead = raw.len() - raw.trim_start().len();
        let s0 = from + lead;
        let text = raw.trim();
        let e0 = s0 + text.len();
        if text.is_empty() {
            self.diag(
                cx,
                L::SYNTAX,
                from,
                from + 1,
                "A selector is missing before the comma or brace.".into(),
                None,
            );
            return None;
        }
        if text.contains("::") {
            let at = s0 + text.find("::").unwrap_or(0);
            self.diag(
                cx,
                L::UNSUPPORTED,
                at,
                e0,
                "Pseudo-elements are not in version 1.0.".into(),
                Some("add a child node instead".into()),
            );
            return None;
        }
        let form_err = |me: &mut Self, at: usize, what: &str| {
            me.diag(
                cx,
                L::SELECTOR_FORM,
                at,
                e0,
                format!("{what} is not a selector form in version 1.0."),
                Some("use .class, kind, .class:state, or .parent:state > .child".into()),
            );
        };
        let gt: Vec<usize> = text.match_indices('>').map(|(k, _)| k).collect();
        if gt.len() > 1 {
            form_err(self, s0 + gt[1], "More than one `>`");
            return None;
        }
        let (left, right) = if let Some(&g) = gt.first() {
            (&text[..g], Some(&text[g + 1..]))
        } else {
            (text, None)
        };
        let l_off = s0;
        let mut sel = Sel {
            class: None,
            kind: None,
            state: 0,
            parent: None,
            s: s0,
            e: e0,
        };
        match right {
            None => {
                let (name_is_class, name, state) = self.simple(
                    cx,
                    left.trim(),
                    l_off + (left.len() - left.trim_start().len()),
                    e0,
                    true,
                )?;
                if name_is_class {
                    sel.class = Some(name);
                } else {
                    sel.kind = Some(kind_id(&name)?);
                }
                sel.state = state;
            }
            Some(r) => {
                let (pc, pname, pstate) = self.simple(
                    cx,
                    left.trim(),
                    l_off + (left.len() - left.trim_start().len()),
                    e0,
                    true,
                )?;
                if !pc {
                    form_err(self, s0, "A kind as the parent");
                    return None;
                }
                let r_off = s0 + left.len() + 1;
                let (cc, cname, cstate) = self.simple(
                    cx,
                    r.trim(),
                    r_off + (r.len() - r.trim_start().len()),
                    e0,
                    true,
                )?;
                if !cc || cstate != 0 {
                    form_err(self, r_off, "A kind or a state on the child");
                    return None;
                }
                sel.class = Some(cname);
                sel.parent = Some((pname, pstate));
            }
        }
        Some(sel)
    }

    /// `.name[:state]` or `kind[:state]`: (is_class, name, state).
    fn simple(
        &mut self,
        cx: &FileCx,
        t: &str,
        at: usize,
        e0: usize,
        _allow_state: bool,
    ) -> Option<(bool, String, u16)> {
        let bad = |me: &mut Self, what: &str| {
            me.diag(
                cx,
                L::SELECTOR_FORM,
                at,
                e0,
                format!("{what} is not a selector form in version 1.0."),
                Some("use .class, kind, .class:state, or .parent:state > .child".into()),
            );
        };
        if t.is_empty() {
            bad(self, "An empty selector");
            return None;
        }
        if t.contains(char::is_whitespace) {
            bad(self, "A descendant selector");
            return None;
        }
        if t.contains(['+', '~', '*', '[', '#']) {
            bad(self, "This combinator or attribute");
            return None;
        }
        let (head, state) = match t.split_once(':') {
            Some((h, s)) => (h, Some(s)),
            None => (t, None),
        };
        if let Some(s) = state
            && s.contains(':')
        {
            bad(self, "More than one state");
            return None;
        }
        let mut st = 0u16;
        if let Some(s) = state {
            match state_bit(s) {
                Some(b) => st = b,
                None => {
                    let id = L::UNKNOWN_STATE;
                    let help = tk::suggest(s, STATES.iter().copied())
                        .map(|n| format!("did you mean `:{n}`?"));
                    let at_s = at + head.len() + 1;
                    self.diag(cx, id, at_s, at_s + s.len(), format!("`:{s}` is not a state."), help.or(Some("the states are hover, active, focus, focus-visible, disabled, selected, checked, expanded and empty".into())));
                    return None;
                }
            }
        }
        if let Some(cls) = head.strip_prefix('.') {
            if cls.len() > MAX_CLASS_NAME || !valid_class(cls) {
                self.diag(
                    cx,
                    L::SYNTAX,
                    at,
                    at + head.len(),
                    format!("`.{cls}` is not a class name."),
                    Some(
                        "class names are lower-case words joined by hyphens, such as .nav-item"
                            .into(),
                    ),
                );
                return None;
            }
            Some((true, cls.to_string(), st))
        } else {
            if kind_id(head).is_none() {
                let help = tk::suggest(head, KINDS.iter().copied())
                    .map(|n| format!("did you mean `{n}`?"));
                self.diag(
                    cx,
                    L::UNKNOWN_NAME,
                    at,
                    at + head.len(),
                    format!("`{head}` is not a node kind."),
                    help.or(Some(format!("the kinds are {}", KINDS.join(", ")))),
                );
                return None;
            }
            Some((false, head.to_string(), st))
        }
    }

    /// Returns the declarations and whether the block had any statement at all.
    fn declarations(&mut self, cx: &FileCx, from: usize, to: usize) -> (Decls, bool) {
        let b = cx.src.as_bytes();
        let mut decls = Decls::default();
        let mut any = false;
        let mut seg_start = from;
        let mut depth = 0i32;
        let mut k = from;
        while k <= to {
            let ch = if k < to { b[k] } else { b';' };
            match ch {
                b'(' => depth += 1,
                b')' => depth -= 1,
                b';' if depth <= 0 => {
                    if self.declaration(cx, seg_start, k, &mut decls) {
                        any = true;
                    }
                    seg_start = k + 1;
                    depth = 0;
                }
                _ => {}
            }
            k += 1;
        }
        (decls, any)
    }

    fn declaration(&mut self, cx: &FileCx, from: usize, to: usize, out: &mut Decls) -> bool {
        let raw = &cx.src[from..to];
        let text = raw.trim();
        if text.is_empty() {
            return false;
        }
        let ns = from + (raw.len() - raw.trim_start().len());
        let Some(colon) = text.find(':') else {
            self.diag(
                cx,
                L::SYNTAX,
                ns,
                ns + text.len(),
                format!("`{text}` needs a colon and a value."),
                Some("write property: value;".into()),
            );
            return true;
        };
        let name = text[..colon].trim_end();
        let vs = ns + colon + 1;
        let toks = tokenize(cx.src, vs, to);
        // `!important`.
        if let Some(p) = toks.iter().position(|t| t.t == T::Bang) {
            self.diag(
                cx,
                L::IMPORTANT,
                toks[p].s,
                toks.get(p + 1).map_or(toks[p].e, |t| t.e),
                "`!important` is not available; the order of resolution is fixed.".into(),
                Some(
                    "class order, then state variants, then inline overrides decide; remove it"
                        .into(),
                ),
            );
            return true;
        }
        let name_end = ns + name.len();
        if let Some(sh) = SHORTHANDS.iter().find(|s| s.name == name) {
            let mut c = Cur::new(toks, sh.name, to, cx.off_scale);
            match parse_shorthand(sh, &mut c) {
                Ok(tmp) => {
                    for (id, src) in tmp {
                        if out.mask & id.bit() != 0 {
                            self.diag(
                                cx,
                                L::DUPLICATE_PROPERTY,
                                ns,
                                name_end,
                                format!("`{}` is set twice in this rule.", id.name()),
                                Some("the later one wins; remove the other".into()),
                            );
                        }
                        out.set_from(id, &src);
                    }
                }
                Err(e) => self.perr(cx, e),
            }
            return true;
        }
        let Some(id) = PropId::parse_name(name) else {
            if is_unsupported_property(name) {
                self.diag(
                    cx,
                    L::UNSUPPORTED,
                    ns,
                    name_end,
                    format!("`{name}` is not in version 1.0."),
                    Some(unsupported_hint(name)),
                );
            } else {
                let all = PROPS
                    .iter()
                    .map(|p| p.name)
                    .chain(SHORTHANDS.iter().map(|s| s.name));
                let help = tk::suggest(name, all).map(|n| format!("did you mean `{n}`?"));
                self.diag(
                    cx,
                    L::UNKNOWN_PROPERTY,
                    ns,
                    name_end,
                    format!("`{name}` is not a property."),
                    help.or(Some("docs/modules/wolkit-style.md lists them".into())),
                );
            }
            return true;
        };
        // Values that exist in CSS but not here.
        if let Some(Tok {
            t: T::Ident(v),
            s,
            e,
        }) = toks.first()
        {
            let uns = match (id, v.as_str()) {
                (PropId::Position, "fixed" | "sticky") => {
                    Some("`position: fixed` and `sticky` are not in version 1.0.".to_string())
                }
                (PropId::Display, "grid" | "inline-grid") => {
                    Some("`display: grid` is planned for 1.1.".to_string())
                }
                _ => None,
            };
            if let Some(m) = uns {
                self.diag(
                    cx,
                    L::UNSUPPORTED,
                    *s,
                    *e,
                    m,
                    Some("use flex, and `position: absolute` with insets".into()),
                );
                return true;
            }
        }
        let mut c = Cur::new(toks, id.name(), to, cx.off_scale);
        let mut tmp = Props::default();
        let r = parse_prop(id, &mut c, &mut tmp).and_then(|_| c.expect_end());
        match r {
            Ok(()) => {
                if out.mask & id.bit() != 0 {
                    self.diag(
                        cx,
                        L::DUPLICATE_PROPERTY,
                        ns,
                        name_end,
                        format!("`{name}` is set twice in this rule."),
                        Some("the later one wins; remove the other".into()),
                    );
                }
                out.set_from(id, &tmp);
            }
            Err(e) => self.perr(cx, e),
        }
        true
    }

    fn class_index(&mut self, cx: &FileCx, name: &str, at: usize, end: usize) -> Option<u32> {
        if let Some(i) = self.index.get(name) {
            return Some(*i);
        }
        if self.table.classes.len() >= MAX_CLASSES {
            self.diag(
                cx,
                L::TABLE_LIMIT,
                at,
                end,
                format!("A style table holds at most {MAX_CLASSES} classes."),
                Some("split the app or drop unused classes".into()),
            );
            return None;
        }
        let i = self.table.classes.len() as u32;
        self.table.classes.push(ClassDef {
            name: name.to_string(),
            variants: vec![],
        });
        self.index.insert(name.to_string(), i);
        Some(i)
    }

    fn register(&mut self, cx: &FileCx, s: &Sel, media: u8, decls: &Decls) {
        let parent = match &s.parent {
            Some((pn, ps)) => match self.class_index(cx, pn, s.s, s.e) {
                Some(c) => Some(ParentSel {
                    class: c,
                    state: *ps,
                }),
                None => return,
            },
            None => None,
        };
        let v = Variant {
            state: s.state,
            media,
            parent,
            decls: decls.clone(),
        };
        let target: &mut ClassDef = if let Some(k) = s.kind {
            &mut self.table.kinds[k as usize]
        } else {
            let name = s.class.clone().unwrap_or_default();
            let Some(ci) = self.class_index(cx, &name, s.s, s.e) else {
                return;
            };
            &mut self.table.classes[ci as usize]
        };
        if let Some(existing) = target.variants.iter_mut().find(|x| x.key() == v.key()) {
            existing.decls.merge(&v.decls);
            let name = target.name.clone();
            self.diag(
                cx,
                L::MERGED_VARIANT,
                s.s,
                s.e,
                format!("`{name}` already has a block for this selector; the two are merged."),
                Some("keep one block per selector".into()),
            );
            return;
        }
        if target.variants.len() >= MAX_VARIANTS {
            let name = target.name.clone();
            self.diag(
                cx,
                L::TOO_MANY_VARIANTS,
                s.s,
                s.e,
                format!("`{name}` would have more than {MAX_VARIANTS} variants."),
                Some("split it into two classes".into()),
            );
            return;
        }
        target.variants.push(v);
    }
}

fn unsupported_hint(n: &str) -> String {
    if n.starts_with("grid") || n.starts_with("place-") {
        "grid is planned for 1.1; use flex rows and columns".into()
    } else if n.starts_with("animation") || n.starts_with("transition-") {
        "use `transition:` or `spring:`".into()
    } else if n == "flex" || n == "flex-flow" || n.starts_with("flex-") {
        "use grow, shrink, basis, flex-direction and flex-wrap".into()
    } else if n == "background-color" || n == "background-image" {
        "use `background`".into()
    } else {
        "docs/modules/wolkit-style.md lists what exists".into()
    }
}

pub fn valid_class(s: &str) -> bool {
    let mut ch = s.chars();
    matches!(ch.next(), Some('a'..='z'))
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !s.ends_with('-')
        && !s.contains("--")
}

/// Copy the value of property `from` in `src` into property `to` of `dst` (same type).
fn move_value(from: PropId, to: PropId, src: &Props, dst: &mut Props) {
    let mut w = Wr::default();
    put_prop(from, src, &mut w);
    let mut r = Rd::new(&w.0);
    let _ = get_prop(to, dst, &mut r);
}

/// Parse a shorthand into (longhand, holder) pairs; the holder carries the value in that longhand.
fn parse_shorthand(sh: &Shorthand, c: &mut Cur) -> PRes<Vec<(PropId, Props)>> {
    let mut out: Vec<(PropId, Props)> = Vec::new();
    if sh.name == "border" || sh.name == "outline" {
        let t = c.take()?;
        c.i -= 1;
        if matches!(&t.t, T::Ident(s) if s == "none") {
            c.i += 1;
            c.expect_end()?;
            let mut p = Props::default();
            if sh.name == "border" {
                p.layout.border_width = 0.0;
            } else {
                p.paint.outline_width = 0.0;
            }
            return Ok(vec![(sh.sets[0], p)]);
        }
        let mut p = Props::default();
        parse_prop(sh.sets[0], c, &mut p)?;
        out.push((sh.sets[0], p));
        if !c.at_end() {
            let mut q = Props::default();
            parse_prop(sh.sets[1], c, &mut q)?;
            out.push((sh.sets[1], q));
        }
        c.expect_end()?;
        return Ok(out);
    }
    let n_t = sh.sets.len();
    let mut vals: Vec<Props> = Vec::new();
    while !c.at_end() {
        if vals.len() == n_t {
            c.expect_end()?;
        }
        let mut p = Props::default();
        parse_prop(sh.sets[0], c, &mut p)?;
        vals.push(p);
    }
    if vals.is_empty() {
        let _ = c.take()?;
    }
    let n = vals.len();
    let pick: Vec<usize> = if n_t == 2 {
        if n == 1 { vec![0, 0] } else { vec![0, 1] }
    } else {
        match n {
            1 => vec![0, 0, 0, 0],
            2 => vec![0, 1, 0, 1],
            3 => vec![0, 1, 2, 1],
            _ => vec![0, 1, 2, 3],
        }
    };
    for (k, id) in sh.sets.iter().enumerate() {
        let mut holder = Props::default();
        move_value(sh.sets[0], *id, &vals[pick[k]], &mut holder);
        out.push((*id, holder));
    }
    Ok(out)
}

/// Replace comment text with spaces (keeping newlines and offsets). Returns the cleaned text, the
/// offset of the off-scale pragma if any, and the offset of an unterminated comment.
pub fn blank_comments(src: &str) -> (String, Option<usize>, Option<usize>) {
    let b = src.as_bytes();
    let mut out = b.to_vec();
    let mut pragma = None;
    let mut unterminated = None;
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i] == b'/' && b[i + 1] == b'*' {
            let start = i;
            let mut j = i + 2;
            let mut closed = false;
            while j + 1 < b.len() {
                if b[j] == b'*' && b[j + 1] == b'/' {
                    closed = true;
                    break;
                }
                j += 1;
            }
            let end = if closed { j + 2 } else { b.len() };
            if !closed {
                unterminated = Some(start);
            } else if src[start + 2..j].trim() == PRAGMA_OFF_SCALE && pragma.is_none() {
                pragma = Some(start);
            }
            for x in out.iter_mut().take(end).skip(start) {
                if *x != b'\n' {
                    *x = b' ';
                }
            }
            i = end;
        } else {
            i += 1;
        }
    }
    // Only ASCII bytes were replaced, so the text stays valid UTF-8.
    (
        String::from_utf8(out).unwrap_or_else(|_| src.to_string()),
        pragma,
        unterminated,
    )
}
