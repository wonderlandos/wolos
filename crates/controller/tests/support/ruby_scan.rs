// SPDX-License-Identifier: GPL-2.0-or-later
//! Text scanners for the Ruby rules (docs/RULES-ruby.md, `rules_ruby.rs`). Each scanner reads one
//! file's text and returns its findings, (line number, why); nothing is parsed or run. A line is
//! seen three ways: as written, without its comment (`plain`), and with string contents blanked
//! (`code`, so a word inside a string never looks like a call). `=begin`/`=end` blocks and heredoc
//! bodies are comments here. Heuristics, by design: a bad fixture per rule proves each one fires.

/// (line number from 1, why).
pub type Finding = (usize, String);

/// One text rule: a file's text in, its findings out.
pub type Scanner = fn(&str) -> Vec<Finding>;

pub struct Line {
    pub n: usize,
    /// Comment removed, strings kept.
    pub plain: String,
    /// Comment removed, string contents replaced by spaces (quotes kept).
    pub code: String,
}

/// The code lines of `text` (comments, blank lines, `=begin` blocks and heredoc bodies left out).
pub fn lines(text: &str) -> Vec<Line> {
    let mut out = Vec::new();
    let mut in_doc = false;
    let mut heredoc: Option<String> = None;
    for (i, raw) in text.lines().enumerate() {
        if let Some(end) = &heredoc {
            if raw.trim() == end {
                heredoc = None;
            }
            continue;
        }
        if in_doc {
            in_doc = !raw.starts_with("=end");
            continue;
        }
        if raw.starts_with("=begin") {
            in_doc = true;
            continue;
        }
        let (plain, code) = split(raw);
        if code.trim().is_empty() {
            continue;
        }
        if let Some(p) = code.find("<<~").or_else(|| code.find("<<-")) {
            let tag: String = plain[p + 3..]
                .trim_start_matches(['\'', '"'])
                .chars()
                .take_while(|c| c.is_ascii_uppercase() || *c == '_')
                .collect();
            if !tag.is_empty() {
                heredoc = Some(tag);
            }
        }
        out.push(Line {
            n: i + 1,
            plain,
            code,
        });
    }
    out
}

/// (line without its comment, the same with string contents blanked).
fn split(raw: &str) -> (String, String) {
    let mut plain = String::new();
    let mut code = String::new();
    let mut quote: Option<char> = None;
    let mut esc = false;
    for c in raw.chars() {
        match quote {
            Some(q) => {
                plain.push(c);
                if esc {
                    esc = false;
                    code.push(' ');
                } else if c == '\\' {
                    esc = true;
                    code.push(' ');
                } else if c == q {
                    quote = None;
                    code.push(c);
                } else {
                    code.push(' ');
                }
            }
            None => {
                if c == '#' {
                    break;
                }
                if c == '"' || c == '\'' || c == '`' {
                    quote = Some(c);
                }
                plain.push(c);
                code.push(c);
            }
        }
    }
    (plain, code)
}

/// The comment lines right after the SPDX line (the purpose header), magic comments and the
/// fixtures' `# trips:` line left out.
pub fn header(text: &str) -> Vec<&str> {
    text.lines()
        .skip(2)
        .take_while(|l| l.starts_with('#'))
        .filter(|l| !is_magic(l) && !l.starts_with("# trips:"))
        .collect()
}

fn is_magic(l: &str) -> bool {
    let l = l.trim_start_matches('#').trim();
    l.starts_with("frozen_string_literal:") || l.starts_with("encoding:")
}

fn word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Byte offsets where `word` stands alone in `s` (no word character or `.` before it when
/// `bare`, no word character after it).
pub fn word_at(s: &str, word: &str, bare: bool) -> Vec<usize> {
    let mut v = Vec::new();
    let mut from = 0;
    while let Some(p) = s[from..].find(word) {
        let at = from + p;
        let before = s[..at].chars().next_back();
        let after = s[at + word.len()..].chars().next();
        let ok_before = before.is_none_or(|c| !word_char(c) && !(bare && (c == '.' || c == ':')));
        let ok_after = after.is_none_or(|c| !word_char(c) && c != '?' && c != '!');
        if ok_before && ok_after {
            v.push(at);
        }
        from = at + word.len();
    }
    v
}

/// The argument text after a call at `at` (the call name `len` long): optional `(` and spaces
/// skipped.
fn args_after(s: &str, at: usize, len: usize) -> &str {
    s[at + len..]
        .trim_start()
        .trim_start_matches('(')
        .trim_start()
}

fn find_all(m: &[Line], f: impl Fn(&Line) -> Option<String>) -> Vec<Finding> {
    m.iter().filter_map(|l| f(l).map(|w| (l.n, w))).collect()
}

/// RB-001: shipped scripts start `#!/usr/bin/ruby`.
pub fn scan_shebang(text: &str) -> Vec<Finding> {
    match text.lines().next() {
        Some("#!/usr/bin/ruby") => vec![],
        l => vec![(
            1,
            format!("first line is {:?}, not #!/usr/bin/ruby", l.unwrap_or("")),
        )],
    }
}

/// RB-002: SPDX on line 2.
pub fn scan_spdx(text: &str) -> Vec<Finding> {
    let l2 = text.lines().nth(1).unwrap_or("");
    if l2.starts_with("# SPDX-License-Identifier: ") {
        vec![]
    } else {
        vec![(2, "line 2 is not the SPDX line".into())]
    }
}

/// RB-003: a purpose header of 2+ comment lines, 60+ characters together.
pub fn scan_header(text: &str) -> Vec<Finding> {
    let h = header(text);
    if h.len() >= 2 && h.iter().map(|l| l.len()).sum::<usize>() >= 60 {
        vec![]
    } else {
        vec![(3, "no purpose header (2+ comment lines after SPDX)".into())]
    }
}

/// RB-004: `# frozen_string_literal: true` before the first code line.
pub fn scan_frozen(text: &str) -> Vec<Finding> {
    for l in text.lines() {
        let t = l.trim();
        if t == "# frozen_string_literal: true" {
            return vec![];
        }
        if !t.is_empty() && !t.starts_with('#') {
            break;
        }
    }
    vec![(
        1,
        "no `# frozen_string_literal: true` before the code".into(),
    )]
}

/// RB-005: no code built at run time.
pub fn scan_eval(text: &str) -> Vec<Finding> {
    find_all(&lines(text), |l| {
        if !word_at(&l.code, "eval", true).is_empty() || l.code.contains("binding.eval") {
            return Some("eval".into());
        }
        for w in ["instance_eval", "class_eval", "module_eval"] {
            for at in word_at(&l.code, w, false) {
                if args_after(&l.code, at, w.len()).starts_with(['"', '\'']) {
                    return Some(format!("{w} of a string"));
                }
            }
        }
        for w in ["send", "public_send", "__send__"] {
            for at in word_at(&l.code, w, false) {
                if !l.code[..at].ends_with('.') {
                    continue;
                }
                let a = args_after(&l.code, at, w.len());
                if a.starts_with(|c: char| c.is_ascii_lowercase() || c == '@' || c == '$') {
                    return Some(format!("{w} of a name from data"));
                }
            }
        }
        None
    })
}

/// RB-006: commands go out as an argv, never one string a shell reads.
pub fn scan_shell_string(text: &str) -> Vec<Finding> {
    find_all(&lines(text), |l| {
        if l.code.contains('`') {
            return Some("backticks".into());
        }
        if l.plain.contains("%x(") || l.plain.contains("%x{") || l.plain.contains("%x[") {
            return Some("%x".into());
        }
        let calls = [
            ("system", true),
            ("exec", true),
            ("spawn", true),
            ("Process.spawn", false),
            ("IO.popen", false),
            ("Open3.capture2", false),
            ("Open3.capture2e", false),
            ("Open3.capture3", false),
            ("Open3.popen3", false),
            ("Open3.popen2", false),
            ("Open3.popen2e", false),
        ];
        for (w, bare) in calls {
            for at in word_at(&l.code, w, bare) {
                let a = args_after(&l.code, at, w.len());
                if one_argument(a) {
                    return Some(format!("{w} with one command string"));
                }
            }
        }
        None
    })
}

/// The argument list `a` (after the call's `(`) is a single string or a single name, not a list.
fn one_argument(a: &str) -> bool {
    let first = if let Some(q) = a.strip_prefix('"').or_else(|| a.strip_prefix('\'')) {
        let Some(end) = q.find(['"', '\'']) else {
            return false;
        };
        &q[end + 1..]
    } else if a.starts_with(|c: char| c.is_ascii_lowercase() || c == '_' || c == '@') {
        let n = a
            .find(|c: char| !word_char(c) && c != '@')
            .unwrap_or(a.len());
        if a[n..].trim_start().starts_with(['.', '[']) {
            return false; // a method call or an index: unknown, left to review
        }
        &a[n..]
    } else {
        return false; // a splat (*argv), a hash of env first, a constant
    };
    let rest = first.trim_start();
    !rest.starts_with(',')
}

/// RB-007: no pipe opens, no open-uri.
pub fn scan_open(text: &str) -> Vec<Finding> {
    find_all(&lines(text), |l| {
        let p = &l.plain;
        if p.contains("open-uri") || p.contains("URI.open") || p.contains("Kernel.open") {
            return Some("open-uri / Kernel.open".into());
        }
        for at in word_at(&l.code, "open", true) {
            let a = p[at + 4..]
                .trim_start()
                .trim_start_matches('(')
                .trim_start();
            if a.starts_with("\"|") || a.starts_with("'|") {
                return Some("open of a pipe".into());
            }
        }
        None
    })
}

/// RB-008: no decoders that build arbitrary objects.
pub fn scan_unsafe_load(text: &str) -> Vec<Finding> {
    const BAD: &[&str] = &[
        "Marshal.load",
        "Marshal.restore",
        "YAML.load",
        "YAML.unsafe_load",
        "Psych.load",
        "Psych.unsafe_load",
        "JSON.load",
    ];
    find_all(&lines(text), |l| {
        BAD.iter()
            .find(|w| !word_at(&l.code, w, false).is_empty())
            .map(|w| w.to_string())
    })
}

/// RB-009: the default gems a script may require (stdlib that ships with the interpreter).
pub const STDLIB: &[&str] = &[
    "digest",
    "fileutils",
    "io/console",
    "json",
    "logger",
    "net/http",
    "open3",
    "optparse",
    "pathname",
    "securerandom",
    "shellwords",
    "socket",
    "tempfile",
    "time",
    "tmpdir",
    "uri",
];

/// RB-009: `require` only from [`STDLIB`]; no gems, no bundler.
pub fn scan_require(text: &str) -> Vec<Finding> {
    find_all(&lines(text), |l| {
        let t = l.plain.trim();
        if t.starts_with("gem ") || t.starts_with("gem(") || t.contains("Bundler") {
            return Some("gem / Bundler".into());
        }
        for at in word_at(&l.code, "require", true) {
            let a = args_after(&l.plain, at, "require".len());
            let name: String = a
                .trim_start_matches(['"', '\''])
                .chars()
                .take_while(|c| !matches!(c, '"' | '\''))
                .collect();
            if !STDLIB.contains(&name.as_str()) {
                return Some(format!("require {name:?} is not in the default-gem list"));
            }
        }
        None
    })
}

/// RB-010: no `rescue Exception`, no `rescue nil` modifier.
pub fn scan_rescue(text: &str) -> Vec<Finding> {
    find_all(&lines(text), |l| {
        for at in word_at(&l.code, "rescue", true) {
            let a = l.code[at + 6..].trim_start();
            if a.strip_prefix("Exception")
                .is_some_and(|r| !r.starts_with(word_char))
            {
                return Some("rescue Exception".into());
            }
            if a == "nil" || a.starts_with("nil ") || a.starts_with("nil;") {
                return Some("rescue nil".into());
            }
        }
        None
    })
}

/// RB-011: no fixed /tmp paths.
pub fn scan_tmp(text: &str) -> Vec<Finding> {
    find_all(&lines(text), |l| {
        (l.plain.contains("\"/tmp/") || l.plain.contains("'/tmp/"))
            .then(|| "fixed /tmp path".into())
    })
}

/// RB-012: nothing made world-writable.
pub fn scan_world_writable(text: &str) -> Vec<Finding> {
    find_all(&lines(text), |l| {
        let c = &l.code;
        if l.plain.contains("o+w") || l.plain.contains("a+w") {
            return Some("o+w / a+w".into());
        }
        if !(c.contains("chmod")
            || c.contains("mode:")
            || c.contains("mkdir")
            || c.contains("umask"))
        {
            return None;
        }
        for tok in c.split(|ch: char| !ch.is_ascii_alphanumeric()) {
            let digits = tok.strip_prefix("0o").or_else(|| tok.strip_prefix('0'));
            if let Some(d) = digits
                && (2..=4).contains(&d.len())
                && d.chars().all(|ch| ('0'..='7').contains(&ch))
                && matches!(d.chars().last(), Some('2' | '3' | '6' | '7'))
                && !c.contains("umask")
            {
                return Some(format!("mode {tok} lets anyone write"));
            }
        }
        None
    })
}

/// RB-013: a recursive delete of an interpolated path names its guard on the same line.
pub fn scan_rm_rf(text: &str) -> Vec<Finding> {
    find_all(&lines(text), |l| {
        for w in [
            "rm_rf",
            "rm_r",
            "remove_entry",
            "remove_entry_secure",
            "rmtree",
        ] {
            for at in word_at(&l.code, w, false) {
                let a = args_after(&l.plain, at, w.len());
                let guarded =
                    word_at(&l.code, "if", true).len() + word_at(&l.code, "unless", true).len() > 0;
                if a.starts_with('"') && a.contains("#{") && !guarded {
                    return Some(format!("{w} of an interpolated path without a guard"));
                }
            }
        }
        None
    })
}

/// RB-014: no home directory literals.
pub fn scan_home(text: &str) -> Vec<Finding> {
    find_all(&lines(text), |l| {
        l.plain.contains("/home/").then(|| "/home/ literal".into())
    })
}

/// RB-015: no debugger or tracing left in.
pub fn scan_debug(text: &str) -> Vec<Finding> {
    find_all(&lines(text), |l| {
        let c = &l.code;
        for w in ["binding.irb", "binding.pry", "set_trace_func", "TracePoint"] {
            if c.contains(w) {
                return Some(w.into());
            }
        }
        for w in ["byebug", "debugger"] {
            if !word_at(c, w, true).is_empty() {
                return Some(w.into());
            }
        }
        if c.replace(' ', "").contains("$DEBUG=true") {
            return Some("$DEBUG = true".into());
        }
        None
    })
}

/// RB-016: Net::HTTP sets open and read timeouts; no class-method shortcuts (no way to set one).
pub fn scan_http(text: &str) -> Vec<Finding> {
    let m = lines(text);
    let mut out = find_all(&m, |l| {
        for w in [
            "Net::HTTP.get",
            "Net::HTTP.post",
            "Net::HTTP.get_response",
            "Net::HTTP.post_form",
        ] {
            if !word_at(&l.code, w, false).is_empty() {
                return Some(format!("{w} cannot set a timeout"));
            }
        }
        None
    });
    let uses = m.iter().find(|l| l.code.contains("Net::HTTP"));
    let set = |w: &str| m.iter().any(|l| l.code.contains(w));
    if let Some(l) = uses
        && out.is_empty()
        && !(set("open_timeout") && set("read_timeout"))
    {
        out.push((
            l.n,
            "Net::HTTP without open_timeout and read_timeout".into(),
        ));
    }
    out
}

/// RB-017: no literal credentials; no secret put in argv from a variable.
pub fn scan_credential(text: &str) -> Vec<Finding> {
    find_all(&lines(text), |l| {
        let low = l.plain.to_ascii_lowercase();
        for key in ["password", "passwd", "secret", "token", "api_key", "apikey"] {
            for at in word_at(&low, key, false) {
                let rest = low[at + key.len()..].trim_start();
                let rest = rest
                    .strip_prefix(':')
                    .or_else(|| rest.strip_prefix('='))
                    .map(str::trim_start);
                if let Some(r) = rest
                    && let Some(q) = r.strip_prefix('"').or_else(|| r.strip_prefix('\''))
                    && q.find(['"', '\'']).is_some_and(|e| e >= 6)
                {
                    return Some(format!("literal {key}"));
                }
            }
            if low.contains(&format!("--{key}=#{{")) || low.contains(&format!("--{key}\", ")) {
                return Some(format!("{key} in argv"));
            }
        }
        None
    })
}

/// RB-018: no carriage returns; the file ends with a newline.
pub fn scan_text(text: &str) -> Vec<Finding> {
    let mut v = Vec::new();
    if let Some(i) = text.lines().position(|l| l.contains('\r')) {
        v.push((i + 1, "carriage return".into()));
    }
    if !text.is_empty() && !text.ends_with('\n') {
        v.push((text.lines().count(), "no newline at the end".into()));
    }
    v
}

/// RB-022: `Dir.mktmpdir` without a block is cleaned up in an `ensure`.
pub fn scan_mktmpdir(text: &str) -> Vec<Finding> {
    let m = lines(text);
    let has_ensure = m
        .iter()
        .any(|l| !word_at(&l.code, "ensure", true).is_empty());
    find_all(&m, |l| {
        let at = l.code.find("Dir.mktmpdir")?;
        let rest = &l.code[at..];
        let block = !word_at(rest, "do", true).is_empty() || rest.contains('{');
        (!block && !has_ensure).then(|| "Dir.mktmpdir without a block or an ensure".into())
    })
}

/// Every text scanner, by rule id (the tool rules RB-019, RB-020, RB-032 and the tree rules
/// RB-021, RB-030, RB-031 are tests of their own).
pub const FILE_SCANNERS: &[(&str, Scanner)] = &[
    ("RB-001", scan_shebang),
    ("RB-002", scan_spdx),
    ("RB-003", scan_header),
    ("RB-004", scan_frozen),
    ("RB-005", scan_eval),
    ("RB-006", scan_shell_string),
    ("RB-007", scan_open),
    ("RB-008", scan_unsafe_load),
    ("RB-009", scan_require),
    ("RB-010", scan_rescue),
    ("RB-011", scan_tmp),
    ("RB-012", scan_world_writable),
    ("RB-013", scan_rm_rf),
    ("RB-014", scan_home),
    ("RB-015", scan_debug),
    ("RB-016", scan_http),
    ("RB-017", scan_credential),
    ("RB-018", scan_text),
    ("RB-022", scan_mktmpdir),
];

pub fn scanner(id: &str) -> Scanner {
    FILE_SCANNERS
        .iter()
        .find(|(i, _)| *i == id)
        .map(|(_, f)| *f)
        .unwrap_or_else(|| panic!("no scanner {id}"))
}
