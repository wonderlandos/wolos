// SPDX-License-Identifier: BSD-2-Clause
//! Diagnostics of the style compiler. Every message is a sentence with `file:line:col`, the source
//! line, a caret and (where one exists) a suggestion. The word "failed" never appears (RU-003).
//! Ids are `WK-nnn`, catalogued here and in `docs/modules/wolkit-style.md`, one test each in
//! `tests/lints.rs`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

impl Severity {
    pub fn word(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

/// A catalogued lint.
pub struct LintDef {
    pub id: &'static str,
    pub severity: Severity,
    /// What the lint checks, one sentence.
    pub summary: &'static str,
    /// The rule it mirrors, if any (`docs/RULES-ui.md`).
    pub rule: Option<&'static str>,
}

macro_rules! lints {
    ($($const:ident = $id:literal, $sev:ident, $rule:expr, $sum:literal;)+) => {
        $(pub const $const: &str = $id;)+
        pub const LINTS: &[LintDef] = &[
            $(LintDef { id: $id, severity: Severity::$sev, summary: $sum, rule: $rule }),+
        ];
    };
}

lints! {
    HEX_LITERAL = "WK-001", Error, Some("RU-062"), "A hex colour literal; colours are theme tokens.";
    IMPORTANT = "WK-002", Error, Some("RU-056"), "`!important`; the order of resolution is fixed and needs no override.";
    FONT_TOO_SMALL = "WK-003", Error, Some("RU-059"), "A font size below 11 px.";
    DURATION_TOO_LONG = "WK-004", Error, Some("RU-060"), "A transition longer than 500 ms.";
    OFF_SCALE_SPACING = "WK-005", Error, None, "Spacing off the scale 0, 4, 8, 12, 16, 24 px (padding, margin, gap, inset).";
    OFF_SCALE_RADIUS = "WK-006", Error, None, "A corner radius off the scale 0, 6, 10, 14, 20 px.";
    UNKNOWN_TOKEN = "WK-007", Error, None, "A colour that is not one of the 13 theme tokens.";
    UNKNOWN_PROPERTY = "WK-008", Error, None, "A property the subset does not have.";
    UNSUPPORTED = "WK-009", Error, None, "A feature that is not in version 1.0 (grid, floats, calc, filters, @import, pseudo-elements, fixed and sticky).";
    BAD_VALUE = "WK-010", Error, None, "A value the property does not accept.";
    UNKNOWN_STATE = "WK-011", Error, None, "A state or media name that does not exist.";
    SELECTOR_FORM = "WK-012", Error, None, "A selector outside the supported forms.";
    DUPLICATE_PROPERTY = "WK-013", Warning, None, "A property set twice in one rule; the later one wins.";
    TOO_MANY_VARIANTS = "WK-014", Error, None, "A class with more than 8 state or media variants.";
    EMPTY_RULE = "WK-015", Warning, None, "A rule without declarations.";
    TABLE_LIMIT = "WK-016", Error, None, "The style table is over 4096 classes or 4 MiB.";
    BAD_WEIGHT = "WK-017", Error, None, "A font weight other than 400, 500, 600 or 700.";
    SYNTAX = "WK-018", Error, None, "The sheet is not well formed (braces, colons, semicolons, comments).";
    UNKNOWN_NAME = "WK-019", Error, None, "A scale, ease, spring or role name that does not exist.";
    OPT_OUT = "WK-020", Warning, None, "The off-scale pragma is in use; opt-outs are counted by a ratchet.";
    MERGED_VARIANT = "WK-021", Warning, None, "The same selector appears twice for a class; the blocks are merged.";
    NOT_ANIMATABLE = "WK-022", Error, None, "A transition on a property that does not animate.";
    COLOR_FUNCTION = "WK-023", Error, Some("RU-062"), "A raw colour function (rgb, hsl); colours are theme tokens.";
    CONFLICTING_MEDIA = "WK-024", Error, None, "Media conditions that can never hold together (@dark with @light).";
}

pub fn def(id: &str) -> Option<&'static LintDef> {
    LINTS.iter().find(|l| l.id == id)
}

/// One finding with its place in the source.
#[derive(Clone, Debug, PartialEq)]
pub struct Diag {
    pub id: &'static str,
    pub severity: Severity,
    pub file: String,
    /// 1-based.
    pub line: u32,
    /// 1-based, in characters.
    pub col: u32,
    /// Characters under the caret (at least 1).
    pub len: u32,
    pub message: String,
    pub help: Option<String>,
    pub source_line: String,
}

impl Diag {
    /// `file:line:col: error[WK-003]: message`, the source line, a caret line and `help:`.
    pub fn render(&self) -> String {
        let mut s = format!(
            "{}:{}:{}: {}[{}]: {}\n",
            self.file,
            self.line,
            self.col,
            self.severity.word(),
            self.id,
            self.message
        );
        let gutter = self.line.to_string();
        let pad = " ".repeat(gutter.len());
        s.push_str(&format!(
            "{pad} |\n{gutter} | {}\n{pad} | ",
            self.source_line
        ));
        for ch in self
            .source_line
            .chars()
            .take(self.col.saturating_sub(1) as usize)
        {
            s.push(if ch == '\t' { '\t' } else { ' ' });
        }
        s.push_str(&"^".repeat(self.len.max(1) as usize));
        s.push('\n');
        if let Some(h) = &self.help {
            s.push_str(&format!("{pad} = help: {h}\n"));
        }
        s
    }
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// Line table of a source text: byte offset to line, column and line text.
pub struct Lines<'a> {
    src: &'a str,
    starts: Vec<usize>,
}

/// The largest char boundary at or below `i` (a span end can fall inside a multi-byte character).
fn boundary(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

impl<'a> Lines<'a> {
    pub fn new(src: &'a str) -> Self {
        let mut starts = vec![0];
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i + 1);
            }
        }
        Lines { src, starts }
    }
    pub fn locate(&self, off: usize) -> (u32, u32, &'a str) {
        let off = boundary(self.src, off);
        let li = match self.starts.binary_search(&off) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let start = self.starts[li];
        let end = self.src[start..]
            .find('\n')
            .map_or(self.src.len(), |n| start + n);
        let text = self.src[start..end].trim_end_matches('\r');
        let col = self.src[start..off.max(start)].chars().count() as u32 + 1;
        (li as u32 + 1, col, text)
    }
    pub fn span_chars(&self, from: usize, to: usize) -> u32 {
        let from = boundary(self.src, from);
        let to = boundary(self.src, to).max(from);
        let (l1, _, _) = self.locate(from);
        let (l2, _, _) = self.locate(to);
        if l1 != l2 {
            // A caret under the first line only.
            let (_, c, t) = self.locate(from);
            return (t.chars().count() as u32 + 1).saturating_sub(c).max(1);
        }
        self.src[from..to].chars().count().max(1) as u32
    }
}
