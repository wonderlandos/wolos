// SPDX-License-Identifier: GPL-2.0-or-later
//! Shell to graph: a symbolic walk over the tree-sitter-bash tree. Nothing here runs anything.
//! Constant text is carried through variables, and literal base64 is folded, so an `eval` of a known
//! string is read as code. What cannot be known stays marked as unresolved: a finding, never a pass.
use crate::graph::{EdgeKind as E, Graph, Loc, NodeId, NodeKind as K, Stage};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use tree_sitter::{Node, Parser};

/// Hard limits on hostile input. Hitting one is reported as a coverage gap.
const MAX_DEPTH: usize = 128;
const MAX_STEPS: usize = 2_000_000;
const MAX_CALLS: u32 = 4;
const MAX_LOOP: usize = 8;

#[derive(Clone, Debug, Default)]
pub struct Val {
    pub text: String,
    /// Every part of `text` is known.
    pub complete: bool,
    /// Graph nodes whose content flows into this value.
    pub taints: Vec<NodeId>,
    /// Came from a file in the tree, so running it is judged by that file, not as unknown text.
    pub local: bool,
}

impl Val {
    fn lit(s: &str) -> Self {
        Val {
            text: s.to_owned(),
            complete: true,
            ..Val::default()
        }
    }
    pub fn known(s: &str) -> Self {
        Self::lit(s)
    }
    pub fn unknown(s: String) -> Self {
        Val {
            text: s,
            ..Val::default()
        }
    }
    pub fn tainted(n: NodeId) -> Self {
        Val {
            text: "(computed)".into(),
            taints: vec![n],
            ..Val::default()
        }
    }
    pub fn append(&mut self, o: Val) {
        self.push(o);
    }
    fn push(&mut self, o: Val) {
        self.text.push_str(&o.text);
        self.complete &= o.complete;
        self.taints.extend(o.taints);
    }
}

#[derive(Clone, Debug, Default)]
struct Var {
    vals: Vec<Val>,
    node: Option<NodeId>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Note {
    pub text: String,
    pub loc: Option<Loc>,
}

/// One entry of `source=()` as the PKGBUILD declares it.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SourceEntry {
    pub name: String,
    pub url: Option<String>,
    pub sum: Option<String>,
    pub vcs: bool,
    pub pinned: bool,
    pub line: u32,
}

/// Inputs the walk needs from around the file.
#[derive(Clone, Debug, Default)]
pub struct Scope {
    /// Files next to the PKGBUILD, by name: followed when something runs them.
    pub helpers: HashMap<String, String>,
    pub sources: Vec<SourceEntry>,
}

pub struct Walker<'g> {
    pub g: &'g mut Graph,
    pub notes: Vec<Note>,
    pub truncated: bool,
    /// Commands in variables are the norm here, so only what flows into them is judged.
    pub lenient: bool,
    src: Rc<str>,
    file: String,
    stage: Stage,
    fixed_line: Option<u32>,
    line_base: u32,
    env: HashMap<String, Var>,
    env_line: HashMap<String, u32>,
    funcs: HashMap<String, (Rc<str>, u32)>,
    called: HashMap<String, u32>,
    cwd: String,
    func: Option<NodeId>,
    func_name: String,
    stdin: Option<NodeId>,
    stdout: Option<NodeId>,
    pipe_text: HashMap<NodeId, String>,
    trees: Vec<(String, NodeId)>,
    fetched: Vec<NodeId>,
    counts: HashMap<String, u32>,
    in_test: u32,
    depth: usize,
    steps: usize,
    pipes: u32,
    scope: Scope,
    walked_helpers: HashSet<String>,
}

fn language() -> tree_sitter::Language {
    tree_sitter_bash::LANGUAGE.into()
}

fn base(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

pub fn is_url(s: &str) -> bool {
    matches!(s.split_once("://"), Some((p, r)) if !r.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'.' || b == b'-'))
}

const SHELLS: &[&str] = &["sh", "bash", "dash", "zsh", "ksh", "ash", "fish"];
const INTERPRETERS: &[&str] = &[
    "python", "python2", "python3", "perl", "ruby", "node", "nodejs", "php", "lua", "tclsh",
    "deno", "bun",
];
const WRAPPERS: &[&str] = &[
    "sudo",
    "doas",
    "pkexec",
    "env",
    "nohup",
    "command",
    "builtin",
    "exec",
    "time",
    "fakeroot",
    "setsid",
    "nice",
    "ionice",
    "chronic",
    "stdbuf",
    "arch-nspawn",
];
const BUILD_TOOLS: &[&str] = &[
    "make",
    "gmake",
    "ninja",
    "cmake",
    "meson",
    "scons",
    "cargo",
    "go",
    "gcc",
    "g++",
    "cc",
    "c++",
    "clang",
    "clang++",
    "autoreconf",
    "automake",
    "autoconf",
    "qmake",
    "qmake6",
    "mvn",
    "gradle",
    "ant",
    "dotnet",
    "zig",
    "nimble",
    "dune",
    "stack",
    "cabal",
];
const NET_TOOLS: &[&str] = &[
    "nc", "ncat", "netcat", "socat", "telnet", "ssh", "sftp", "ftp", "tftp",
];

impl<'g> Walker<'g> {
    pub fn new(g: &'g mut Graph, scope: Scope) -> Self {
        Walker {
            g,
            notes: Vec::new(),
            truncated: false,
            lenient: false,
            src: Rc::from(""),
            file: String::new(),
            stage: Stage::Parse,
            fixed_line: None,
            line_base: 0,
            env: HashMap::from([(
                "BASH_SOURCE".to_owned(),
                Var {
                    vals: vec![Val::lit("$startdir/PKGBUILD")],
                    node: None,
                },
            )]),
            env_line: HashMap::new(),
            funcs: HashMap::new(),
            called: HashMap::new(),
            cwd: "$srcdir".into(),
            func: None,
            func_name: String::new(),
            stdin: None,
            stdout: None,
            pipe_text: HashMap::new(),
            trees: Vec::new(),
            fetched: Vec::new(),
            counts: HashMap::new(),
            in_test: 0,
            depth: 0,
            steps: 0,
            pipes: 0,
            scope,
            walked_helpers: HashSet::new(),
        }
    }

    /// Set the sources once they are known, and put them in the graph.
    pub fn declare_sources(&mut self, sources: Vec<SourceEntry>, file: &str) {
        for s in &sources {
            let loc = Some(Loc {
                file: file.to_owned(),
                line: s.line,
            });
            let src = self.g.node(
                K::Source,
                &format!("src:{}", s.name),
                &s.name,
                loc.clone(),
                Stage::Prepare,
            );
            let f = self.g.node(
                K::File,
                &format!("$srcdir/{}", s.name),
                &s.name,
                loc.clone(),
                Stage::Prepare,
            );
            match &s.url {
                Some(u) => {
                    let n =
                        self.g
                            .node(K::Url, &format!("url:{u}"), u, loc.clone(), Stage::Prepare);
                    self.g.edge(n, src, E::Fetches, loc.clone());
                }
                None => {
                    let n = self.g.node(
                        K::File,
                        &format!("$startdir/{}", s.name),
                        &s.name,
                        loc.clone(),
                        Stage::Prepare,
                    );
                    self.g.edge(n, src, E::Reads, loc.clone());
                }
            }
            self.g.edge(src, f, E::Writes, loc.clone());
            let checked = s.pinned
                || s.sum
                    .as_deref()
                    .is_some_and(|x| !x.is_empty() && x != "SKIP");
            if checked {
                let c = self.g.node(
                    K::Command,
                    "check:makepkg",
                    "makepkg's checksum",
                    loc.clone(),
                    Stage::Prepare,
                );
                self.g.edge(c, src, E::Verifies, loc.clone());
            }
        }
        self.scope.sources = sources;
    }

    /// Run the top level of `text`, registering functions and following what it calls.
    pub fn walk_file(&mut self, text: &str, file: &str, stage: Stage) {
        self.file = file.to_owned();
        self.stage = stage;
        self.walk_text(text, None);
    }

    /// Walk one function by name, as the phase that runs it. Missing functions are nothing.
    pub fn walk_function(&mut self, name: &str, stage: Stage, cwd: &str) {
        let Some((body, line)) = self.funcs.get(name).cloned() else {
            return;
        };
        self.stage = stage;
        self.cwd = cwd.to_owned();
        let saved = self.enter(name, line);
        self.called.insert(name.to_owned(), 1);
        self.walk_body(&body, line);
        self.func = saved.0;
        self.func_name = saved.1;
    }

    /// Functions nothing called. A dynamic call could still reach them, so they are read too.
    pub fn walk_uncalled(&mut self, stage: Stage) {
        let names: Vec<String> = self
            .funcs
            .keys()
            .filter(|n| !self.called.contains_key(*n))
            .cloned()
            .collect();
        let mut names = names;
        names.sort();
        for n in names {
            let cwd = self.cwd.clone();
            self.walk_function(&n, stage, &cwd);
        }
    }

    pub fn function_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.funcs.keys().cloned().collect();
        v.sort();
        v
    }

    pub fn var_values(&self, name: &str) -> Vec<String> {
        self.env
            .get(name)
            .map(|v| v.vals.iter().map(|x| x.text.clone()).collect())
            .unwrap_or_default()
    }

    pub fn var_line(&self, name: &str) -> u32 {
        self.env_line.get(name).copied().unwrap_or(0)
    }

    fn enter(&mut self, name: &str, line: u32) -> (Option<NodeId>, String) {
        let loc = Some(Loc {
            file: self.file.clone(),
            line,
        });
        let f = self.g.node(
            K::Function,
            &format!("fn:{}:{name}", self.file),
            name,
            loc.clone(),
            self.stage,
        );
        if let Some(p) = self.func {
            self.g.edge(p, f, E::Calls, loc);
        }
        let saved = (
            self.func,
            std::mem::replace(&mut self.func_name, name.to_owned()),
        );
        self.func = Some(f);
        saved
    }

    fn walk_body(&mut self, body: &Rc<str>, line: u32) {
        let saved = self.fixed_line.take();
        let src = std::mem::replace(&mut self.src, body.clone());
        let base = std::mem::replace(&mut self.line_base, line.saturating_sub(1));
        self.parse_and_walk();
        self.src = src;
        self.line_base = base;
        self.fixed_line = saved;
    }

    /// Parse text and walk it. `at` pins every location to one line, for code held in a string.
    fn walk_text(&mut self, text: &str, at: Option<u32>) {
        if self.depth >= MAX_DEPTH {
            self.truncated = true;
            return;
        }
        let saved = std::mem::replace(&mut self.fixed_line, at);
        let src = std::mem::replace(&mut self.src, Rc::from(text));
        let base = std::mem::replace(&mut self.line_base, 0);
        self.parse_and_walk();
        self.src = src;
        self.line_base = base;
        self.fixed_line = saved;
    }

    fn parse_and_walk(&mut self) {
        let mut p = Parser::new();
        if p.set_language(&language()).is_err() {
            self.truncated = true;
            return;
        }
        let src = self.src.clone();
        let Some(tree) = p.parse(src.as_bytes(), None) else {
            self.truncated = true;
            return;
        };
        self.depth += 1;
        self.stmt(tree.root_node());
        self.depth -= 1;
    }

    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.src.as_bytes()).unwrap_or("")
    }

    fn loc(&self, n: Node) -> Option<Loc> {
        let line = self
            .fixed_line
            .unwrap_or(n.start_position().row as u32 + 1 + self.line_base);
        Some(Loc {
            file: self.file.clone(),
            line,
        })
    }

    fn stmt(&mut self, n: Node) {
        self.steps += 1;
        if self.steps > MAX_STEPS || self.depth >= MAX_DEPTH {
            self.truncated = true;
            return;
        }
        self.depth += 1;
        self.stmt_inner(n);
        self.depth -= 1;
    }

    fn children(n: Node<'_>) -> Vec<Node<'_>> {
        let mut c = n.walk();
        n.named_children(&mut c).collect()
    }

    fn stmt_inner(&mut self, n: Node) {
        match n.kind() {
            "comment" | "heredoc_body" | "heredoc_start" | "heredoc_end" => {}
            "function_definition" => {
                let (Some(name), Some(body)) =
                    (n.child_by_field_name("name"), n.child_by_field_name("body"))
                else {
                    return;
                };
                let name = self.text(name).to_owned();
                let text: Rc<str> = Rc::from(self.text(body));
                let line = body.start_position().row as u32 + 1 + self.line_base;
                self.funcs.insert(name, (text, line));
            }
            "variable_assignment" => self.assign(n, false),
            "declaration_command" => {
                for c in Self::children(n) {
                    match c.kind() {
                        "variable_assignment" => self.assign(c, false),
                        _ => {
                            self.word(c);
                        }
                    }
                }
            }
            "command" => self.command(n),
            "pipeline" => self.pipeline(n),
            "redirected_statement" => self.redirected(n),
            "if_statement"
            | "elif_clause"
            | "else_clause"
            | "while_statement"
            | "until_statement"
            | "case_statement"
            | "case_item"
            | "do_group"
            | "compound_statement"
            | "subshell"
            | "program"
            | "list"
            | "negated_command"
            | "test_command"
            | "ternary_expression"
            | "binary_expression"
            | "unary_expression"
            | "parenthesized_expression" => {
                if n.kind() == "test_command" {
                    self.test(n);
                    return;
                }
                let cwd = (n.kind() == "subshell").then(|| self.cwd.clone());
                for c in Self::children(n) {
                    self.stmt(c);
                }
                if let Some(c) = cwd {
                    self.cwd = c;
                }
            }
            "for_statement" => self.for_loop(n),
            "c_style_for_statement" => {
                if let Some(b) = n.child_by_field_name("body") {
                    self.stmt(b);
                }
            }
            "command_substitution"
            | "process_substitution"
            | "string"
            | "concatenation"
            | "expansion"
            | "simple_expansion" => {
                self.word(n);
            }
            _ => {
                if n.named_child_count() > 0 {
                    for c in Self::children(n) {
                        self.stmt(c);
                    }
                }
            }
        }
    }

    fn test(&mut self, n: Node) {
        self.in_test += 1;
        let mut stack = Self::children(n);
        while let Some(c) = stack.pop() {
            match c.kind() {
                "word"
                | "string"
                | "raw_string"
                | "concatenation"
                | "expansion"
                | "simple_expansion"
                | "command_substitution" => {
                    // Asked about, not opened. A substitution in the word still runs.
                    self.word(c);
                }
                _ => stack.extend(Self::children(c)),
            }
        }
        self.in_test -= 1;
    }

    fn for_loop(&mut self, n: Node) {
        let name = n
            .child_by_field_name("variable")
            .map(|v| self.text(v).to_owned())
            .unwrap_or_default();
        let mut c = n.walk();
        let vals: Vec<Node> = n.children_by_field_name("value", &mut c).collect();
        let mut items = Vec::new();
        for v in vals {
            if let Some(list) = self.array_of(v) {
                items.extend(list);
            } else {
                let w = self.word(v);
                items.push(w);
            }
        }
        let Some(body) = n.child_by_field_name("body") else {
            return;
        };
        if items.is_empty() {
            items.push(Val::unknown(format!("${{{name}}}")));
        }
        items.truncate(MAX_LOOP);
        for it in items {
            self.env.insert(
                name.clone(),
                Var {
                    vals: vec![it],
                    node: None,
                },
            );
            self.stmt(body);
        }
    }

    /// `"${arr[@]}"` as its elements.
    fn array_of(&mut self, n: Node) -> Option<Vec<Val>> {
        let t = self.text(n).trim_matches('"').to_owned();
        let name = t.strip_prefix("${")?.strip_suffix("[@]}")?;
        Some(self.env.get(name)?.vals.clone())
    }

    fn assign(&mut self, n: Node, append: bool) {
        let (Some(name), value) = (
            n.child_by_field_name("name"),
            n.child_by_field_name("value"),
        ) else {
            return;
        };
        let name = self.text(name).to_owned();
        let name = name.split('[').next().unwrap_or("").to_owned();
        let append = append || self.text(n).contains("+=");
        let mut vals = Vec::new();
        if let Some(v) = value {
            if v.kind() == "array" {
                for c in Self::children(v) {
                    if c.kind() == "comment" {
                        continue;
                    }
                    let expanded = self.array_of(c);
                    match expanded {
                        Some(list) => vals.extend(list),
                        None => vals.push(self.word(c)),
                    }
                }
            } else {
                vals.push(self.word(v));
            }
        } else {
            vals.push(Val::lit(""));
        }
        let line = n.start_position().row as u32 + 1 + self.line_base;
        self.env_line.entry(name.clone()).or_insert(line);
        self.set_var(&name, vals, append, self.loc(n));
    }

    fn set_var(&mut self, name: &str, mut vals: Vec<Val>, append: bool, loc: Option<Loc>) {
        let taints: Vec<NodeId> = vals.iter().flat_map(|v| v.taints.iter().copied()).collect();
        let mut var = if append {
            self.env.remove(name).unwrap_or_default()
        } else {
            Var::default()
        };
        if !taints.is_empty() {
            let id = self.g.node(
                K::Env,
                &format!("var:{}:{name}", self.func_name),
                name,
                loc.clone(),
                self.stage,
            );
            for t in taints {
                self.g.edge(t, id, E::Writes, loc.clone());
            }
            var.node = Some(id);
        }
        var.vals.append(&mut vals);
        self.env.insert(name.to_owned(), var);
    }

    fn lookup(&self, name: &str) -> Val {
        if let Some(v) = self.env.get(name) {
            let mut out = Val {
                complete: true,
                ..Val::default()
            };
            for (i, x) in v.vals.iter().enumerate() {
                if i > 0 {
                    out.text.push(' ');
                }
                out.push(x.clone());
            }
            if let Some(n) = v.node {
                out.taints = vec![n];
            }
            return out;
        }
        match name {
            "srcdir" | "pkgdir" | "startdir" | "HOME" => Val::lit(&format!("${name}")),
            "PWD" => Val::lit(&self.cwd.clone()),
            "CARCH" => Val::lit("x86_64"),
            "SHELL" | "CONFIG_SHELL" => Val::lit("sh"),
            "CC" | "CXX" | "CPP" | "LD" | "AR" | "RANLIB" | "STRIP" | "MAKE" | "AWK" | "SED"
            | "GREP" | "EGREP" | "FGREP" | "RM" | "CP" | "MV" | "LN" | "LN_S" | "MKDIR_P"
            | "INSTALL" | "ECHO" => Val::lit(&name.to_ascii_lowercase()),
            _ => Val::unknown(format!("${{{name}}}")),
        }
    }

    /// The value of any word-like node. Substitutions inside it run for their effects.
    fn word(&mut self, n: Node) -> Val {
        self.steps += 1;
        if self.depth >= MAX_DEPTH {
            self.truncated = true;
            return Val::unknown("$?".into());
        }
        self.depth += 1;
        let v = self.word_inner(n);
        self.depth -= 1;
        v
    }

    fn word_inner(&mut self, n: Node) -> Val {
        match n.kind() {
            "word" | "number" => {
                let t = self.text(n);
                let mut s = String::with_capacity(t.len());
                let mut it = t.chars();
                while let Some(c) = it.next() {
                    if c == '\\' {
                        s.extend(it.next());
                    } else {
                        s.push(c);
                    }
                }
                if let Some(r) = s.strip_prefix('~') {
                    if r.is_empty() || r.starts_with('/') {
                        s = format!("$HOME{r}");
                    }
                }
                Val::lit(&s)
            }
            "raw_string" => Val::lit(self.text(n).trim_matches('\'')),
            "ansi_c_string" => {
                let t = self
                    .text(n)
                    .trim_start_matches('$')
                    .trim_matches('\'')
                    .to_owned();
                Val::lit(&unescape(&t))
            }
            "string" | "concatenation" => {
                let mut out = Val {
                    complete: true,
                    ..Val::default()
                };
                let quoted = n.kind() == "string";
                let start = n.start_byte() + usize::from(quoted);
                let end = n.end_byte() - usize::from(quoted);
                let mut at = start;
                let src = self.src.clone();
                for c in Self::children(n) {
                    if c.start_byte() > at {
                        out.text.push_str(src.get(at..c.start_byte()).unwrap_or(""));
                    }
                    if c.kind() == "string_content" {
                        out.text.push_str(&unslash(self.text(c)));
                    } else {
                        let v = self.word(c);
                        out.push(v);
                    }
                    at = c.end_byte();
                }
                if end > at {
                    out.text.push_str(src.get(at..end).unwrap_or(""));
                }
                out
            }
            "simple_expansion" => {
                let t = self.text(n).trim_start_matches('$').to_owned();
                self.lookup(&t)
            }
            "expansion" => self.expansion(n),
            "command_substitution" | "process_substitution" => self.substitution(n),
            "arithmetic_expansion" => Val::lit("0"),
            "array" => {
                let mut out = Val {
                    complete: true,
                    ..Val::default()
                };
                for c in Self::children(n) {
                    let v = self.word(c);
                    out.push(v);
                    out.text.push(' ');
                }
                out
            }
            _ => Val::unknown(self.text(n).to_owned()),
        }
    }

    fn substitution(&mut self, n: Node) -> Val {
        let pipe = self.pipe(self.loc(n));
        let saved = (self.stdout.replace(pipe), self.cwd.clone());
        for c in Self::children(n) {
            self.stmt(c);
        }
        self.stdout = saved.0;
        self.cwd = saved.1;
        match self.pipe_text.get(&pipe) {
            Some(t) => Val {
                text: t.trim_end_matches('\n').to_owned(),
                complete: true,
                taints: vec![pipe],
                local: false,
            },
            None => Val {
                text: "$(...)".into(),
                complete: false,
                taints: vec![pipe],
                local: false,
            },
        }
    }

    fn expansion(&mut self, n: Node) -> Val {
        let t = self.text(n);
        let inner = t.trim_start_matches("${").trim_end_matches('}').to_owned();
        if inner.starts_with('!') {
            return Val::unknown(t.to_owned());
        }
        if let Some(name) = inner.strip_prefix('#') {
            if !name.is_empty() {
                return Val::lit("0");
            }
        }
        let end = inner
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(inner.len());
        let (name, rest) = inner.split_at(end);
        if rest.starts_with('[') {
            let idx = rest.trim_start_matches('[').split(']').next().unwrap_or("");
            if let Some(v) = self.env.get(name) {
                return match idx {
                    "@" | "*" => self.lookup(name),
                    i => v
                        .vals
                        .get(i.parse::<usize>().unwrap_or(0))
                        .cloned()
                        .unwrap_or_else(|| Val::unknown(t.to_owned())),
                };
            }
            return Val::unknown(t.to_owned());
        }
        let mut v = self.lookup(name);
        if rest.is_empty() {
            return v;
        }
        let arg = |skip: usize| rest.get(skip..).unwrap_or("").to_owned();
        let default = |s: String| -> Val {
            if s.contains('$') {
                Val::unknown(s)
            } else {
                Val::lit(&s)
            }
        };
        if !v.complete
            && !(rest.starts_with(":-") || rest.starts_with('-') || rest.starts_with(":="))
        {
            return Val {
                text: t.to_owned(),
                taints: v.taints,
                ..Val::default()
            };
        }
        if rest.starts_with(":-") || rest.starts_with(":=") {
            return if v.complete && !v.text.is_empty() {
                v
            } else {
                default(arg(2))
            };
        }
        if rest.starts_with(":+") {
            return if v.complete && !v.text.is_empty() {
                default(arg(2))
            } else {
                Val::lit("")
            };
        }
        if rest.starts_with(':') {
            return Val::unknown(t.to_owned());
        }
        if rest.starts_with("^^") {
            v.text = v.text.to_uppercase();
        } else if rest.starts_with(",,") {
            v.text = v.text.to_lowercase();
        } else if rest.starts_with("%%") {
            v.text = strip_suffix(&v.text, &arg(2), true);
        } else if rest.starts_with('%') {
            v.text = strip_suffix(&v.text, &arg(1), false);
        } else if rest.starts_with("##") {
            v.text = strip_prefix(&v.text, &arg(2), true);
        } else if rest.starts_with('#') {
            v.text = strip_prefix(&v.text, &arg(1), false);
        } else if rest.starts_with("//") || rest.starts_with('/') {
            let all = rest.starts_with("//");
            let body = arg(if all { 2 } else { 1 });
            let (from, to) = body.split_once('/').unwrap_or((&body, ""));
            if from.contains(['*', '?', '[']) {
                return Val::unknown(t.to_owned());
            }
            v.text = if all {
                v.text.replace(from, to)
            } else {
                v.text.replacen(from, to, 1)
            };
        } else {
            return Val::unknown(t.to_owned());
        }
        v
    }

    fn pipe(&mut self, loc: Option<Loc>) -> NodeId {
        self.pipes += 1;
        let key = format!("pipe:{}:{}", self.func_name, self.pipes);
        self.g.node(K::File, &key, "a pipe", loc, self.stage)
    }

    fn pipeline(&mut self, n: Node) {
        let parts = Self::children(n);
        let outer = (self.stdin, self.stdout);
        let mut prev = outer.0;
        for (i, c) in parts.iter().enumerate() {
            let last = i + 1 == parts.len();
            let next = if last {
                outer.1
            } else {
                Some(self.pipe(self.loc(*c)))
            };
            self.stdin = prev;
            self.stdout = next;
            self.stmt(*c);
            prev = next;
        }
        self.stdin = outer.0;
        self.stdout = outer.1;
    }

    fn redirected(&mut self, n: Node) {
        let saved = (self.stdin, self.stdout);
        let loc = self.loc(n);
        let mut pre = Vec::new();
        for c in Self::children(n) {
            match c.kind() {
                "file_redirect" => {
                    let op = self
                        .text(c)
                        .trim_start_matches(|x: char| x.is_ascii_digit())
                        .to_owned();
                    let Some(dest) = c
                        .child_by_field_name("destination")
                        .or_else(|| c.named_child(c.named_child_count().saturating_sub(1)))
                    else {
                        continue;
                    };
                    let v = self.word(dest);
                    if v.text.is_empty() || v.text == "1" || v.text == "2" || v.text == "/dev/null"
                    {
                        continue;
                    }
                    if let Some(r) = v
                        .text
                        .strip_prefix("/dev/tcp/")
                        .or_else(|| v.text.strip_prefix("/dev/udp/"))
                    {
                        let id = self.g.node(
                            K::Socket,
                            &format!("sock:{r}"),
                            r,
                            loc.clone(),
                            self.stage,
                        );
                        self.stdin = Some(id);
                        self.stdout = Some(id);
                        continue;
                    }
                    if op.starts_with('<') {
                        let p = self.path(&v.text);
                        let f = self.file_node(&p, loc.clone());
                        self.stdin = Some(f);
                    } else {
                        let p = self.path(&v.text);
                        let f = self.file_node(&p, loc.clone());
                        self.stdout = Some(f);
                    }
                }
                "heredoc_redirect" => {
                    let p = self.pipe(loc.clone());
                    let mut body = String::new();
                    let mut c2 = c.walk();
                    for b in c.named_children(&mut c2) {
                        if b.kind() == "heredoc_body" {
                            body.push_str(self.text(b));
                        } else if b.kind() == "pipeline"
                            || b.kind() == "command"
                            || b.kind() == "list"
                        {
                            pre.push(b);
                        }
                    }
                    self.pipe_text.insert(p, body);
                    self.stdin = Some(p);
                }
                "herestring_redirect" => {
                    let p = self.pipe(loc.clone());
                    if let Some(w) = c.named_child(0) {
                        let v = self.word(w);
                        for t in &v.taints {
                            self.g.edge(*t, p, E::Writes, loc.clone());
                        }
                        if v.complete {
                            self.pipe_text.insert(p, v.text);
                        }
                    }
                    self.stdin = Some(p);
                }
                _ => {}
            }
        }
        if let Some(b) = n.child_by_field_name("body") {
            if pre.is_empty() {
                self.stmt(b);
            } else {
                // `cat <<EOF | sh`: the pipeline hangs off the heredoc; its head reads the document.
                let doc = self.stdin;
                let next = self.pipe(loc.clone());
                self.stdout = Some(next);
                self.stmt(b);
                if let Some(d) = doc {
                    let text = self.pipe_text.get(&d).cloned();
                    if let Some(t) = text {
                        self.pipe_text.insert(next, t);
                    }
                }
                self.stdin = Some(next);
                self.stdout = saved.1;
                for p in pre {
                    self.stmt(p);
                }
            }
        }
        self.stdin = saved.0;
        self.stdout = saved.1;
    }

    fn command(&mut self, n: Node) {
        let loc = self.loc(n);
        let mut c = n.walk();
        let mut assigns = Vec::new();
        for ch in n.named_children(&mut c) {
            if ch.kind() == "variable_assignment" {
                assigns.push(ch);
            }
        }
        for a in assigns {
            self.assign(a, false);
        }
        let Some(name) = n.child_by_field_name("name") else {
            return;
        };
        let name_node = name.named_child(0).unwrap_or(name);
        let nv = self.word(name_node);
        let mut c = n.walk();
        let argn: Vec<Node> = n.children_by_field_name("argument", &mut c).collect();
        let mut args = Vec::with_capacity(argn.len());
        for a in argn {
            match self.array_of(a) {
                Some(l) => args.extend(l),
                None => args.push(self.word(a)),
            }
        }
        self.run(nv, args, loc);
    }

    pub(crate) fn process(&mut self, name: &str, loc: Option<Loc>) -> NodeId {
        let k = format!("{}:{}:{name}", self.file, self.func_name);
        let c = self.counts.entry(k.clone()).or_insert(0);
        *c += 1;
        let key = format!("proc:{k}:{c}");
        let id = self.g.node(K::Process, &key, name, loc.clone(), self.stage);
        let cmd = self
            .g
            .node(K::Command, &format!("cmd:{name}"), name, None, self.stage);
        self.g.edge(cmd, id, E::Calls, loc.clone());
        if let Some(f) = self.func {
            self.g.edge(f, id, E::Calls, loc);
        }
        id
    }

    fn path(&self, p: &str) -> String {
        if p.starts_with('/') || p.starts_with('$') || p.starts_with('~') {
            return normalize(p);
        }
        normalize(&format!("{}/{p}", self.cwd))
    }

    fn file_node(&mut self, path: &str, loc: Option<Loc>) -> NodeId {
        let scriptlet_path = if self.stage == Stage::Scriptlet && path.starts_with('/') {
            Some(format!("$pkgdir{path}"))
        } else {
            None
        };
        if let Some(id) = scriptlet_path.and_then(|p| self.g.find(&p)) {
            return id;
        }
        if let Some(id) = self.g.find(path) {
            return id;
        }
        let id = self.g.node(K::File, path, path, loc.clone(), self.stage);
        let tree = self
            .trees
            .iter()
            .find(|(p, _)| {
                path.len() > p.len()
                    && path.starts_with(p.as_str())
                    && path.as_bytes()[p.len()] == b'/'
            })
            .map(|x| x.1);
        if let Some(t) = tree {
            self.g.edge(t, id, E::Writes, loc);
        }
        id
    }

    fn url(&mut self, u: &str, loc: Option<Loc>) -> NodeId {
        self.g.node(K::Url, &format!("url:{u}"), u, loc, self.stage)
    }

    fn connect_stdio(&mut self, proc: NodeId, loc: &Option<Loc>) {
        if let Some(i) = self.stdin {
            self.g.edge(i, proc, E::Reads, loc.clone());
        }
        if let Some(o) = self.stdout {
            if self.g.nodes[o as usize].kind == K::Socket {
                self.g.edge(proc, o, E::Sends, loc.clone());
            } else {
                self.g.edge(proc, o, E::Writes, loc.clone());
            }
        }
    }

    fn feed(&mut self, args: &[Val], proc: NodeId, loc: &Option<Loc>) {
        for a in args {
            for t in &a.taints {
                self.g.edge(*t, proc, E::Reads, loc.clone());
            }
        }
    }

    fn stdin_text(&self) -> Option<String> {
        self.stdin.and_then(|s| self.pipe_text.get(&s).cloned())
    }

    fn write_stdout_text(&mut self, text: Option<String>) {
        if let (Some(o), Some(t)) = (self.stdout, text) {
            self.pipe_text.insert(o, t);
        }
    }

    /// One command with its words evaluated.
    fn run(&mut self, name: Val, mut args: Vec<Val>, loc: Option<Loc>) {
        if !name.complete {
            if self.lenient && name.taints.is_empty() {
                return self.plain("(a command in a variable)", &args, loc);
            }
            let proc = self.process("(dynamic command)", loc.clone());
            self.dark(proc);
            self.feed(std::slice::from_ref(&name), proc, &loc);
            self.feed(&args, proc, &loc);
            for t in &name.taints {
                self.g.edge(*t, proc, E::Executes, loc.clone());
            }
            self.connect_stdio(proc, &loc);
            return;
        }
        let mut cmd = name.text.clone();
        // Peel wrappers such as `sudo env FOO=1 nohup sh x`.
        for _ in 0..8 {
            let b = base(&cmd).to_owned();
            if WRAPPERS.contains(&b.as_str()) {
                while args.first().is_some_and(|a| {
                    a.text.starts_with('-') || (b == "env" && a.text.contains('='))
                }) {
                    args.remove(0);
                }
                if args.is_empty() {
                    break;
                }
                let n = args.remove(0);
                if !n.complete {
                    return self.run(n, args, loc);
                }
                cmd = n.text;
            } else if b == "timeout" && args.len() >= 2 {
                args.remove(0);
                let n = args.remove(0);
                cmd = n.text;
            } else if b == "busybox" && !args.is_empty() {
                cmd = args.remove(0).text;
            } else {
                break;
            }
        }
        if let Some(f) = self.funcs.get(&cmd).cloned() {
            return self.call(&cmd, f, args, loc);
        }
        let b = base(&cmd).to_owned();
        match b.as_str() {
            "cd" | "pushd" => {
                if let Some(a) = args.iter().find(|a| !a.text.starts_with('-')) {
                    if a.complete {
                        self.cwd = self.path(&a.text);
                    }
                }
            }
            "popd" | ":" | "true" | "false" | "exit" | "return" | "set" | "shopt" | "export"
            | "unset" | "local" | "shift" | "wait" | "trap" | "read" | "msg" | "msg2"
            | "warning" | "plain" | "error" => {
                if b == "read" {
                    if let Some(a) = args.iter().rfind(|a| !a.text.starts_with('-')) {
                        let n = a.text.clone();
                        if let Some(i) = self.stdin {
                            let mut v = Val::unknown("$(read)".into());
                            v.taints.push(i);
                            self.set_var(&n, vec![v], false, loc);
                        }
                    }
                }
                if b == "export" {
                    return;
                }
            }
            "dirname" | "basename" => {
                let proc = self.process(&b, loc.clone());
                self.feed(&args, proc, &loc);
                self.connect_stdio(proc, &loc);
                let known = args
                    .iter()
                    .find(|a| !a.text.starts_with('-'))
                    .filter(|a| a.complete)
                    .map(|a| {
                        let t = a.text.trim_end_matches('/');
                        if b == "basename" {
                            base(t).to_owned()
                        } else {
                            t.rsplit_once('/')
                                .map_or(".".to_owned(), |x| x.0.to_owned())
                        }
                    });
                self.write_stdout_text(known);
            }
            "echo" | "printf" => {
                let proc = self.process(&b, loc.clone());
                self.feed(&args, proc, &loc);
                self.connect_stdio(proc, &loc);
                let hex = args
                    .iter()
                    .any(|a| a.text.contains("\\x") || a.text.contains("\\0"));
                let vals: Vec<&Val> = args
                    .iter()
                    .filter(|a| !(b == "echo" && a.text.starts_with('-')))
                    .collect();
                if vals.iter().all(|a| a.complete) {
                    let t = vals
                        .iter()
                        .map(|a| a.text.as_str())
                        .collect::<Vec<_>>()
                        .join(" ");
                    self.write_stdout_text(Some(if hex { unescape(&t) } else { format!("{t}\n") }));
                }
                if hex {
                    if let Some(o) = self.stdout {
                        self.g.edge(proc, o, E::Decodes, loc);
                    }
                }
            }
            "cat" if args.is_empty() => {
                let proc = self.process("cat", loc.clone());
                self.connect_stdio(proc, &loc);
                let t = self.stdin_text();
                self.write_stdout_text(t);
            }
            "curl" | "wget" | "aria2c" | "lwp-download" | "http" | "https" | "fetch" => {
                self.fetch(&b, &args, loc)
            }
            "git" | "svn" | "hg" => self.vcs(&b, &args, loc),
            "scp" | "rsync"
                if args
                    .iter()
                    .any(|a| a.text.contains(':') && !a.text.starts_with('-')) =>
            {
                self.remote_copy(&b, &args, loc)
            }
            x if NET_TOOLS.contains(&x) => self.net_tool(x, &args, loc),
            "sha256sum" | "sha512sum" | "sha1sum" | "sha224sum" | "sha384sum" | "md5sum"
            | "b2sum" | "cksum" | "gpg" | "gpg2" | "minisign" | "cosign" | "signify"
            | "pacman-key" => self.verify(&b, &args, loc),
            "base64" | "base32" | "basenc" | "xxd" | "uudecode" | "openssl" | "tr" | "rev"
            | "xz" | "gzip" | "bzip2" | "zstd" | "lzma" | "zcat" | "xzcat" | "bzcat"
            | "zstdcat" | "gunzip" | "unxz" => self.decode(&b, &args, loc),
            "test" | "[" => {
                self.process(&b, loc);
            }
            "eval" => {
                let proc = self.process("eval", loc.clone());
                let mut v = Val {
                    complete: true,
                    ..Val::default()
                };
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        v.text.push(' ');
                    }
                    v.push(a.clone());
                }
                self.exec_text(&v, proc, &loc);
                self.connect_stdio(proc, &loc);
            }
            "source" | "." => {
                if let Some(a) = args.first() {
                    self.exec_path(a.clone(), "source", loc);
                }
            }
            x if SHELLS.contains(&x) => self.shell(x, &args, loc),
            x if interp(x) => self.interpreter(x, &args, loc),
            "chmod" | "chown" | "chgrp" | "setcap" | "chattr" => {
                let proc = self.process(&b, loc.clone());
                self.feed(&args, proc, &loc);
                for a in positional(&args).iter().skip(1) {
                    let p = self.path(&a.text);
                    let f = self.file_node(&p, loc.clone());
                    self.g.edge(proc, f, E::Writes, loc.clone());
                }
            }
            "cp" | "mv" | "ln" | "install" | "rsync" | "dd" | "tee" | "touch" | "mkdir" | "rm"
            | "rmdir" | "sed" | "patch" | "tar" | "bsdtar" | "unzip" | "ar" | "7z" | "cpio"
            | "unsquashfs" | "bsdcpio" => self.file_tool(&b, &args, loc),
            "systemctl" | "crontab" | "at" | "loginctl" | "launchctl" => {
                self.persist(&b, &args, loc)
            }
            "npm" | "yarn" | "pnpm" | "pip" | "pip3" | "gem" | "bundle" | "composer" => {
                self.registry(&b, &args, loc)
            }
            "find"
                if args
                    .iter()
                    .any(|a| a.text == "-exec" || a.text == "-execdir") =>
            {
                let i = args
                    .iter()
                    .position(|a| a.text.starts_with("-exec"))
                    .unwrap_or(0);
                let mut inner: Vec<Val> = args[i + 1..]
                    .iter()
                    .filter(|a| a.text != ";" && a.text != "+" && a.text != "\\;")
                    .cloned()
                    .collect();
                if !inner.is_empty() {
                    let n = inner.remove(0);
                    self.run(n, inner, loc);
                }
            }
            "xargs" => {
                let mut inner: Vec<Val> = args
                    .iter()
                    .filter(|a| !a.text.starts_with('-'))
                    .cloned()
                    .collect();
                if !inner.is_empty() {
                    let n = inner.remove(0);
                    self.run(n, inner, loc);
                }
            }
            _ if cmd.contains('/') => self.exec_path(Val::lit(&cmd), &b, loc),
            x if BUILD_TOOLS.contains(&x) => self.build_tool(x, &args, loc),
            _ => self.plain(&b, &args, loc),
        }
    }

    fn call(&mut self, name: &str, f: (Rc<str>, u32), args: Vec<Val>, loc: Option<Loc>) {
        let n = self.called.entry(name.to_owned()).or_insert(0);
        *n += 1;
        let first = *n <= MAX_CALLS;
        let saved = self.enter(name, f.1);
        if let (Some(id), l) = (self.func, loc) {
            if let Some(p) = saved.0 {
                self.g.edge(p, id, E::Calls, l);
            }
        }
        if first {
            let mut old = Vec::new();
            for (i, a) in args.into_iter().enumerate() {
                let key = (i + 1).to_string();
                old.push((
                    key.clone(),
                    self.env.insert(
                        key,
                        Var {
                            vals: vec![a],
                            node: None,
                        },
                    ),
                ));
            }
            self.walk_body(&f.0, f.1);
            for (k, v) in old {
                match v {
                    Some(v) => {
                        self.env.insert(k, v);
                    }
                    None => {
                        self.env.remove(&k);
                    }
                }
            }
        }
        self.func = saved.0;
        self.func_name = saved.1;
    }

    fn plain(&mut self, b: &str, args: &[Val], loc: Option<Loc>) {
        let proc = self.process(b, loc.clone());
        self.feed(args, proc, &loc);
        self.connect_stdio(proc, &loc);
        for a in args {
            if a.text.starts_with('-') || !path_like(&a.text) {
                continue;
            }
            let p = self.path(&a.text);
            let f = self.file_node(&p, loc.clone());
            self.g.edge(f, proc, E::Reads, loc.clone());
        }
    }

    fn fetch(&mut self, b: &str, args: &[Val], loc: Option<Loc>) {
        let proc = self.process(b, loc.clone());
        self.feed(args, proc, &loc);
        let (mut out, mut remote_name, mut urls, mut uploads) =
            (None, false, Vec::new(), Vec::new());
        let mut i = 0;
        while i < args.len() {
            let a = &args[i];
            let t = a.text.as_str();
            let next = args.get(i + 1);
            let long_value = |o: &str| t.strip_prefix(o).and_then(|r| r.strip_prefix('='));
            if t == "--output" || t == "--output-document" {
                out = next.cloned();
                i += 2;
                continue;
            }
            if let Some(v) = long_value("--output").or_else(|| long_value("--output-document")) {
                out = Some(Val::lit(v));
            } else if t == "--remote-name" {
                remote_name = true;
            } else if matches!(
                t,
                "-d" | "--data"
                    | "--data-binary"
                    | "--data-raw"
                    | "--data-urlencode"
                    | "-F"
                    | "--form"
                    | "-T"
                    | "--upload-file"
                    | "--post-file"
                    | "--body-file"
            ) {
                if let Some(n) = next {
                    uploads.push((
                        n.clone(),
                        matches!(t, "-T" | "--upload-file" | "--post-file" | "--body-file"),
                    ));
                }
                i += 2;
                continue;
            } else if let Some(v) =
                long_value("--post-file").or_else(|| long_value("--upload-file"))
            {
                uploads.push((Val::lit(v), true));
            } else if let Some(v) = long_value("--data-binary").or_else(|| long_value("--data")) {
                uploads.push((Val::lit(v), false));
            } else if matches!(
                t,
                "-A" | "-H"
                    | "-e"
                    | "-u"
                    | "-x"
                    | "-b"
                    | "-c"
                    | "-m"
                    | "-w"
                    | "-X"
                    | "-U"
                    | "--header"
                    | "--user-agent"
                    | "--retry"
                    | "--connect-timeout"
                    | "--max-time"
                    | "--proxy"
                    | "--cookie"
            ) {
                i += 2;
                continue;
            } else if t.starts_with('-') && !t.starts_with("--") {
                let flags = t.trim_start_matches('-');
                if let Some(pos) = flags.rfind(['o', 'O']) {
                    let letter = flags.as_bytes()[pos];
                    let rest = &flags[pos + 1..];
                    let lower_takes = b == "curl" && letter == b'o';
                    let upper_takes = b != "curl" && letter == b'O';
                    if lower_takes || upper_takes {
                        if !rest.is_empty() {
                            out = Some(Val::lit(rest));
                        } else {
                            out = next.cloned();
                            i += 1;
                        }
                    } else if b == "curl" && letter == b'O' {
                        remote_name = true;
                    }
                }
            } else if !t.starts_with('-') {
                urls.push(a.clone());
            }
            i += 1;
        }
        for u in &urls {
            let id = self.url(&u.text, loc.clone());
            if !u.complete {
                self.g.mark_unresolved(id);
            }
            self.g.edge(id, proc, E::Fetches, loc.clone());
        }
        for (u, whole_file) in uploads {
            self.feed(std::slice::from_ref(&u), proc, &loc);
            // `-d @file` and `-F x=@file` name a file; plain `-d x=1` is data.
            let body = if whole_file {
                Some(u.text.as_str())
            } else {
                u.text.split_once('@').map(|x| x.1)
            };
            if let Some(body) = body {
                let p = self.path(body);
                let f = self.file_node(&p, loc.clone());
                self.g.edge(f, proc, E::Reads, loc.clone());
            }
        }
        // Anything read into the request leaves the machine with it.
        for u in &urls {
            let id = self.url(&u.text, loc.clone());
            self.g.edge(proc, id, E::Sends, loc.clone());
        }
        let wget_default = b == "wget" && out.is_none();
        let stdout_marker = out.as_ref().is_some_and(|o| o.text == "-");
        if stdout_marker || (out.is_none() && !remote_name && !wget_default) {
            self.connect_stdio(proc, &loc);
            self.fetched.push(proc);
        } else {
            let target = match out {
                Some(o) => o.text,
                None => urls
                    .first()
                    .map(|u| base(&u.text).to_owned())
                    .unwrap_or_default(),
            };
            let p = self.path(&target);
            let f = self.file_node(&p, loc.clone());
            self.g.edge(proc, f, E::Writes, loc.clone());
            self.fetched.push(f);
            if let Some(i) = self.stdin {
                self.g.edge(i, proc, E::Reads, loc);
            }
        }
    }

    fn vcs(&mut self, b: &str, args: &[Val], loc: Option<Loc>) {
        let sub = args.first().map(|a| a.text.as_str()).unwrap_or("");
        if !matches!(
            sub,
            "clone"
                | "fetch"
                | "pull"
                | "checkout"
                | "co"
                | "submodule"
                | "push"
                | "archive"
                | "ls-remote"
        ) {
            return self.plain(b, args, loc);
        }
        let proc = self.process(b, loc.clone());
        self.feed(args, proc, &loc);
        let pos = positional(&args[1..]);
        let url = pos
            .iter()
            .find(|a| is_url(&a.text) || a.text.contains('@') && a.text.contains(':'));
        if let Some(u) = url {
            let id = self.url(&u.text, loc.clone());
            if sub == "push" {
                self.g.edge(proc, id, E::Sends, loc.clone());
            } else {
                self.g.edge(id, proc, E::Fetches, loc.clone());
            }
            let dir = pos
                .iter()
                .rev()
                .find(|a| !std::ptr::eq(**a, *u))
                .map(|a| a.text.clone())
                .unwrap_or_else(|| base(&u.text).trim_end_matches(".git").to_owned());
            let p = self.path(&dir);
            let f = self.file_node(&p, loc.clone());
            self.g.edge(proc, f, E::Writes, loc.clone());
            self.trees.push((p, f));
            self.fetched.push(f);
        }
        self.connect_stdio(proc, &loc);
    }

    fn remote_copy(&mut self, b: &str, args: &[Val], loc: Option<Loc>) {
        let proc = self.process(b, loc.clone());
        self.feed(args, proc, &loc);
        let pos = positional(args);
        let Some((dst, srcs)) = pos.split_last() else {
            return;
        };
        if dst.text.contains(':') {
            for s in srcs {
                let p = self.path(&s.text);
                let f = self.file_node(&p, loc.clone());
                self.g.edge(f, proc, E::Reads, loc.clone());
            }
            let id = self.url(&dst.text, loc.clone());
            self.g.edge(proc, id, E::Sends, loc);
        } else {
            for s in srcs {
                let id = self.url(&s.text, loc.clone());
                self.g.edge(id, proc, E::Fetches, loc.clone());
            }
            let p = self.path(&dst.text);
            let f = self.file_node(&p, loc.clone());
            self.g.edge(proc, f, E::Writes, loc.clone());
            self.fetched.push(f);
        }
    }

    fn net_tool(&mut self, b: &str, args: &[Val], loc: Option<Loc>) {
        let proc = self.process(b, loc.clone());
        self.feed(args, proc, &loc);
        let pos = positional(args);
        let peer = pos
            .iter()
            .map(|a| a.text.as_str())
            .collect::<Vec<_>>()
            .join(":");
        let id = self.g.node(
            K::Socket,
            &format!("sock:{peer}"),
            &peer,
            loc.clone(),
            self.stage,
        );
        self.g.edge(id, proc, E::Fetches, loc.clone());
        self.g.edge(proc, id, E::Sends, loc.clone());
        // `nc -e /bin/sh host port` hands the shell to the peer.
        if args
            .iter()
            .any(|a| matches!(a.text.as_str(), "-e" | "-c" | "--exec" | "--sh-exec"))
            || args.iter().any(|a| a.text.contains("EXEC:"))
        {
            self.g.edge(id, proc, E::Executes, loc.clone());
        }
        self.connect_stdio(proc, &loc);
        self.fetched.push(proc);
    }

    fn verify(&mut self, b: &str, args: &[Val], loc: Option<Loc>) {
        let proc = self.process(b, loc.clone());
        self.feed(args, proc, &loc);
        let checks = args.iter().any(|a| {
            matches!(
                a.text.as_str(),
                "-c" | "--check" | "--verify" | "-V" | "verify" | "verify-blob"
            )
        });
        if !checks && self.in_test == 0 {
            if b.starts_with("gpg") {
                let decrypts = args
                    .iter()
                    .any(|a| matches!(a.text.as_str(), "-d" | "--decrypt"));
                if decrypts {
                    self.connect_stdio(proc, &loc);
                    if let Some(o) = self.stdout {
                        self.g.edge(proc, o, E::Decodes, loc);
                    }
                    return;
                }
            }
            self.connect_stdio(proc, &loc);
            for a in positional(args) {
                if path_like(&a.text) {
                    let p = self.path(&a.text);
                    let f = self.file_node(&p, loc.clone());
                    self.g.edge(f, proc, E::Reads, loc.clone());
                }
            }
            return;
        }
        let mut named = Vec::new();
        if let Some(t) = self.stdin_text() {
            for line in t.lines() {
                if let Some(f) = line.split_whitespace().nth(1) {
                    named.push(f.trim_start_matches('*').to_owned());
                }
            }
        }
        if self.in_test > 0 {
            named.extend(positional(args).iter().map(|a| a.text.clone()));
        }
        if named.is_empty() {
            let last: Vec<NodeId> = self.fetched.clone();
            for f in last {
                self.g.edge(proc, f, E::Verifies, loc.clone());
            }
            if b.starts_with("gpg") || b == "minisign" || b == "cosign" {
                for a in positional(args) {
                    let p = self.path(&a.text);
                    let f = self.file_node(&p, loc.clone());
                    self.g.edge(proc, f, E::Verifies, loc.clone());
                }
            }
        }
        for n in named {
            let p = self.path(&n);
            let f = self.file_node(&p, loc.clone());
            self.g.edge(proc, f, E::Verifies, loc.clone());
        }
        self.connect_stdio(proc, &loc);
    }

    fn decode(&mut self, b: &str, args: &[Val], loc: Option<Loc>) {
        let flag = |s: &[&str]| args.iter().any(|a| s.contains(&a.text.as_str()));
        let decoding = match b {
            "base64" | "base32" => flag(&["-d", "-D", "--decode", "-di", "-id"]),
            "basenc" => flag(&["-d", "--decode"]),
            "xxd" => flag(&["-r", "-ps", "-rp", "-r -p"]) && flag(&["-r", "-rp"]),
            "uudecode" | "rev" | "zcat" | "xzcat" | "bzcat" | "zstdcat" | "gunzip" | "unxz" => true,
            "xz" | "gzip" | "bzip2" | "zstd" | "lzma" => {
                flag(&["-d", "-dc", "-cd", "-dk", "--decompress"])
            }
            "openssl" => flag(&["-d"]) || flag(&["-decrypt"]),
            // A byte-for-byte swap hides text; case folding and deleting characters do not.
            "tr" => {
                let sets: Vec<&Val> = args.iter().filter(|a| !a.text.starts_with('-')).collect();
                // Libtool's macros swap bytes all over; what they feed is not hidden text.
                !self.lenient
                    && !flag(&["-d", "-s", "-c", "-cd", "-dc"])
                    && sets.len() >= 2
                    && !sets.iter().any(|a| {
                        a.text.contains("[:") || a.text.contains("a-z") || a.text.contains("A-Z")
                    })
            }
            _ => false,
        };
        let proc = self.process(b, loc.clone());
        self.feed(args, proc, &loc);
        let inputs: Vec<&Val> = positional(args)
            .into_iter()
            .filter(|a| path_like(&a.text) || (b == "base64" && !a.text.is_empty()))
            .collect();
        let has_stdin = self.stdin.is_some();
        for a in inputs {
            if b == "tr" {
                continue;
            }
            let p = self.path(&a.text);
            let f = self.file_node(&p, loc.clone());
            self.g.edge(
                f,
                proc,
                if decoding { E::Decodes } else { E::Reads },
                loc.clone(),
            );
        }
        self.connect_stdio(proc, &loc);
        if decoding {
            if let (Some(i), true) = (self.stdin, has_stdin) {
                self.g.edge(i, proc, E::Decodes, loc.clone());
            }
            if let Some(t) = self.stdin_text() {
                let folded = match b {
                    "base64" => unbase64(&t),
                    "rev" => Some(
                        t.lines()
                            .map(|l| l.chars().rev().collect::<String>())
                            .collect::<Vec<_>>()
                            .join("\n"),
                    ),
                    _ => None,
                };
                self.write_stdout_text(folded);
            }
        }
    }

    /// Run text as code: read what is known, tie what flowed in, and mark what is neither.
    fn exec_text(&mut self, v: &Val, proc: NodeId, loc: &Option<Loc>) {
        for t in &v.taints {
            self.g.edge(*t, proc, E::Executes, loc.clone());
        }
        if v.complete {
            let (line, text) = (loc.as_ref().map(|l| l.line), v.text.clone());
            self.walk_text(&text, line);
        } else if !assignment_only(&v.text) && !v.local {
            self.dark(proc);
        }
    }

    /// Text run by another language's call. Shell text is read as shell; a language's own code
    /// held in a string goes to that language.
    pub(crate) fn exec_value(
        &mut self,
        v: &Val,
        proc: NodeId,
        loc: &Option<Loc>,
        lang: crate::langs::Lang,
    ) {
        use crate::langs::Lang;
        if v.complete && matches!(lang, Lang::Python | Lang::Js) {
            let line = loc.as_ref().map_or(0, |l| l.line);
            let file = self.file.clone();
            crate::langs::walk(lang, &v.text.clone(), &file, self.stage, self, Some(line));
            return;
        }
        self.exec_text(v, proc, loc);
    }

    /// A value spelled out in the script itself, as the origin of a decode.
    pub(crate) fn literal_source(&mut self, loc: &Option<Loc>) -> NodeId {
        self.g.node(
            K::Env,
            &format!("literal:{}", self.file),
            "data in the script",
            loc.clone(),
            self.stage,
        )
    }

    pub(crate) fn url_node(&mut self, u: &str, loc: Option<Loc>) -> NodeId {
        self.url(u, loc)
    }

    pub(crate) fn pipe_node(&mut self, loc: Option<Loc>) -> NodeId {
        self.pipe(loc)
    }

    pub(crate) fn path_of(&self, p: &str) -> String {
        self.path(p)
    }

    pub(crate) fn file_at(&mut self, p: &str, loc: Option<Loc>) -> NodeId {
        self.file_node(p, loc)
    }

    /// Set the file and stage that locations and new nodes belong to.
    pub fn enter_file(&mut self, file: &str, stage: Stage) {
        self.file = file.to_owned();
        self.stage = stage;
        self.func_name = file.to_owned();
    }

    /// Shell text out of another file's format, pinned to that file's line.
    pub fn snippet(&mut self, text: &str, file: &str, line: u32, stage: Stage) {
        self.file = file.to_owned();
        self.stage = stage;
        self.walk_text(text, Some(line));
    }

    pub(crate) fn run_argv(&mut self, mut argv: Vec<Val>, loc: Option<Loc>) {
        if !argv.is_empty() {
            let name = argv.remove(0);
            self.run(name, argv, loc);
        }
    }

    pub(crate) fn argv_text(&mut self, argv: Vec<String>, file: &str, line: u32) {
        self.file = file.to_owned();
        let loc = Some(Loc {
            file: file.to_owned(),
            line,
        });
        self.run_argv(argv.iter().map(|a| Val::lit(a)).collect(), loc);
    }

    /// A download a build system does itself.
    pub(crate) fn download(&mut self, url: &str, dest: &str, hashed: bool, file: &str, line: u32) {
        self.file = file.to_owned();
        let loc = Some(Loc {
            file: file.to_owned(),
            line,
        });
        let proc = self.process("download", loc.clone());
        let u = self.url(url, loc.clone());
        self.g.edge(u, proc, E::Fetches, loc.clone());
        let p = self.path(dest);
        let f = self.file_node(&p, loc.clone());
        self.g.edge(proc, f, E::Writes, loc.clone());
        if hashed {
            self.g.edge(proc, f, E::Verifies, loc);
        }
    }

    fn shell(&mut self, b: &str, args: &[Val], loc: Option<Loc>) {
        let proc = self.process(b, loc.clone());
        let dash_c = args.iter().position(|a| {
            a.text == "-c"
                || (a.text.starts_with('-') && !a.text.starts_with("--") && a.text.ends_with('c'))
        });
        if let Some(i) = dash_c {
            if let Some(code) = args.get(i + 1) {
                self.exec_text(&code.clone(), proc, &loc);
            }
            self.connect_stdio(proc, &loc);
            return;
        }
        match positional(args).first() {
            Some(f) => {
                let f = (*f).clone();
                self.feed(std::slice::from_ref(&f), proc, &loc);
                self.exec_file(&f, proc, &loc);
                self.connect_stdio(proc, &loc);
            }
            None => {
                // `... | sh`: stdin is the program.
                if let Some(i) = self.stdin {
                    self.g.edge(i, proc, E::Executes, loc.clone());
                    match self.stdin_text() {
                        Some(t) => {
                            let line = loc.as_ref().map(|l| l.line);
                            self.stdin = None;
                            self.walk_text(&t, line);
                            self.stdin = Some(i);
                        }
                        None => self.dark(proc),
                    }
                }
                if let Some(o) = self.stdout {
                    self.g.edge(proc, o, E::Writes, loc);
                }
            }
        }
    }

    fn interpreter(&mut self, b: &str, args: &[Val], loc: Option<Loc>) {
        let proc = self.process(b, loc.clone());
        self.feed(args, proc, &loc);
        let code_flag = args
            .iter()
            .position(|a| matches!(a.text.as_str(), "-c" | "-e" | "-E" | "-r" | "--eval"));
        if let Some(i) = code_flag {
            if let Some(code) = args.get(i + 1).cloned() {
                for t in &code.taints {
                    self.g.edge(*t, proc, E::Executes, loc.clone());
                }
                if code.complete {
                    let line = loc.as_ref().map(|l| l.line).unwrap_or(0);
                    crate::langs::inline(self.g, b, &code.text, &self.file, line, self.stage, proc);
                } else {
                    self.dark(proc);
                }
            }
            self.connect_stdio(proc, &loc);
            return;
        }
        let pos = positional(args);
        let first = pos
            .iter()
            .find(|a| !a.text.starts_with('-'))
            .map(|a| (*a).clone());
        if first.as_ref().is_some_and(|f| base(&f.text) == "setup.py")
            || b.starts_with("python") && args.iter().any(|a| a.text == "-m")
        {
            self.exec_tree(proc, &loc);
        } else if let Some(f) = first {
            self.exec_file(&f, proc, &loc);
        } else if let Some(i) = self.stdin {
            self.g.edge(i, proc, E::Executes, loc.clone());
            self.dark(proc);
        }
        self.connect_stdio(proc, &loc);
    }

    fn exec_path(&mut self, f: Val, label: &str, loc: Option<Loc>) {
        let proc = self.process(label, loc.clone());
        self.feed(std::slice::from_ref(&f), proc, &loc);
        self.exec_file(&f, proc, &loc);
        self.connect_stdio(proc, &loc);
    }

    /// This process runs something the walk cannot see.
    fn dark(&mut self, proc: NodeId) {
        if !self.lenient {
            self.g.mark_unresolved(proc);
        }
    }

    /// `f` is run by `proc`: tie its content in, and follow it if it is a helper we hold.
    fn exec_file(&mut self, f: &Val, proc: NodeId, loc: &Option<Loc>) {
        if !f.complete {
            self.dark(proc);
            return;
        }
        let p = self.path(&f.text);
        let id = self.file_node(&p, loc.clone());
        self.g.edge(id, proc, E::Executes, loc.clone());
        for s in self.sources_of(&p) {
            self.g.edge(s, proc, E::Executes, loc.clone());
        }
        let name = base(&p).to_owned();
        let inside = p.starts_with("$srcdir/") || p.starts_with("$startdir/");
        if inside
            && self.scope.helpers.contains_key(&name)
            && self.walked_helpers.insert(name.clone())
        {
            let text = self.scope.helpers[&name].clone();
            self.follow(&name, &text);
        }
    }

    fn follow(&mut self, name: &str, text: &str) {
        let (file, line) = (
            std::mem::replace(&mut self.file, name.to_owned()),
            self.fixed_line.take(),
        );
        let src = std::mem::replace(&mut self.src, Rc::from(text));
        let base_line = std::mem::replace(&mut self.line_base, 0);
        self.parse_and_walk();
        self.src = src;
        self.line_base = base_line;
        self.fixed_line = line;
        self.file = file;
    }

    /// Declared sources a path inside `$srcdir` plausibly came from.
    fn sources_of(&self, path: &str) -> Vec<NodeId> {
        let Some(rel) = path.strip_prefix("$srcdir/") else {
            return Vec::new();
        };
        let first = rel.split('/').next().unwrap_or(rel);
        let trees: Vec<&SourceEntry> = self
            .scope
            .sources
            .iter()
            .filter(|s| s.url.is_some() && (s.vcs || is_archive(&s.name)))
            .collect();
        let exact: Vec<&SourceEntry> = self
            .scope
            .sources
            .iter()
            .filter(|s| {
                s.name == rel
                    || (s.vcs && s.name == first)
                    || (is_archive(&s.name) && stem(&s.name) == first)
            })
            .collect();
        let chosen = if !exact.is_empty() { exact } else { trees };
        chosen
            .iter()
            .filter_map(|s| self.g.find(&format!("src:{}", s.name)))
            .collect()
    }

    fn exec_tree(&mut self, proc: NodeId, loc: &Option<Loc>) {
        let here = format!("{}/x", self.cwd);
        for s in self.sources_of(&here) {
            self.g.edge(s, proc, E::Executes, loc.clone());
        }
    }

    fn build_tool(&mut self, b: &str, args: &[Val], loc: Option<Loc>) {
        let proc = self.process(b, loc.clone());
        self.feed(args, proc, &loc);
        self.exec_tree(proc, &loc);
        self.connect_stdio(proc, &loc);
        let networked = match b {
            "cargo" => {
                args.first().is_some_and(|a| {
                    matches!(
                        a.text.as_str(),
                        "fetch" | "build" | "install" | "run" | "test"
                    )
                }) && !args
                    .iter()
                    .any(|a| matches!(a.text.as_str(), "--locked" | "--frozen" | "--offline"))
            }
            "go" => args.first().is_some_and(|a| a.text == "get"),
            _ => false,
        };
        if networked {
            self.notes.push(Note {
                text: format!("{b} may download code that no lockfile pins"),
                loc,
            });
        }
    }

    fn registry(&mut self, b: &str, args: &[Val], loc: Option<Loc>) {
        let proc = self.process(b, loc.clone());
        self.feed(args, proc, &loc);
        self.exec_tree(proc, &loc);
        self.connect_stdio(proc, &loc);
        let sub = args.first().map(|a| a.text.as_str()).unwrap_or("install");
        let installing = matches!(sub, "install" | "i" | "add" | "update")
            || (b == "yarn" && !matches!(sub, "run" | "build"));
        let locked = sub == "ci"
            || args.iter().any(|a| {
                matches!(
                    a.text.as_str(),
                    "--frozen-lockfile"
                        | "--immutable"
                        | "--require-hashes"
                        | "--ignore-scripts"
                        | "--offline"
                        | "--no-index"
                )
            });
        if installing && !locked {
            self.notes.push(Note {
                text: format!("{b} {sub} downloads and may run code that no lockfile pins"),
                loc,
            });
        }
    }

    fn persist(&mut self, b: &str, args: &[Val], loc: Option<Loc>) {
        let proc = self.process(b, loc.clone());
        self.feed(args, proc, &loc);
        self.connect_stdio(proc, &loc);
        let sub = args
            .iter()
            .find(|a| !a.text.starts_with('-'))
            .map(|a| a.text.as_str())
            .unwrap_or("");
        let target = match (b, sub) {
            ("systemctl", "enable" | "reenable" | "link" | "preset" | "add-wants") => args
                .iter()
                .rfind(|a| !a.text.starts_with('-'))
                .map(|a| format!("/etc/systemd/system/enabled/{}", a.text)),
            ("crontab", _) => Some("/var/spool/cron/crontab".to_owned()),
            ("at", _) => Some("/var/spool/at/job".to_owned()),
            ("loginctl", "enable-linger") => Some("/var/lib/systemd/linger/user".to_owned()),
            _ => None,
        };
        if let Some(t) = target {
            let f = self.file_node(&t, loc.clone());
            self.g.edge(proc, f, E::Writes, loc);
        }
    }

    fn file_tool(&mut self, b: &str, args: &[Val], loc: Option<Loc>) {
        let proc = self.process(b, loc.clone());
        self.feed(args, proc, &loc);
        self.connect_stdio(proc, &loc);
        let pos = positional_opts(b, args);
        let write = |w: &mut Self, t: &str, loc: &Option<Loc>| {
            let p = w.path(t);
            let f = w.file_node(&p, loc.clone());
            w.g.edge(proc, f, E::Writes, loc.clone());
            f
        };
        let read = |w: &mut Self, t: &str, loc: &Option<Loc>| {
            let p = w.path(t);
            let f = w.file_node(&p, loc.clone());
            w.g.edge(f, proc, E::Reads, loc.clone());
            f
        };
        let target_dir = args
            .iter()
            .position(|a| a.text == "-t" || a.text == "--target-directory")
            .and_then(|i| args.get(i + 1))
            .map(|a| a.text.clone());
        match b {
            "tee" | "touch" | "mkdir" | "rm" | "rmdir" => {
                for a in &pos {
                    write(self, &a.text, &loc);
                }
            }
            "dd" => {
                for a in args {
                    if let Some(v) = a.text.strip_prefix("of=") {
                        write(self, v, &loc);
                    } else if let Some(v) = a.text.strip_prefix("if=") {
                        read(self, v, &loc);
                    }
                }
            }
            "sed" => {
                let inplace = args
                    .iter()
                    .any(|a| a.text.starts_with("-i") || a.text == "--in-place");
                for a in pos.iter().skip(1) {
                    if inplace {
                        write(self, &a.text, &loc);
                    } else {
                        read(self, &a.text, &loc);
                    }
                }
            }
            "patch" => {}
            "tar" | "bsdtar" | "unzip" | "ar" | "7z" | "cpio" | "unsquashfs" | "bsdcpio" => {
                // Only the first word bundles flags: `tar czf out in`, never a file named `src`.
                let bundled = args.first().is_some_and(|a| {
                    !a.text.starts_with("--") && a.text.trim_start_matches('-').contains('c')
                });
                let creates = matches!(b, "tar" | "bsdtar")
                    && (bundled || args.iter().any(|a| a.text == "--create"));
                if creates {
                    // Without `f` the archive goes to stdout, and every word is an input.
                    let to_file = args
                        .first()
                        .is_some_and(|a| !a.text.starts_with("--") && a.text.contains('f'))
                        || args.iter().any(|a| a.text == "-f" || a.text == "--file");
                    let inputs = match pos.split_first() {
                        Some((out, rest)) if to_file => {
                            write(self, &out.text, &loc);
                            rest
                        }
                        _ => &pos[..],
                    };
                    for a in inputs {
                        read(self, &a.text, &loc);
                    }
                    return;
                }
                let dir = args
                    .iter()
                    .position(|a| matches!(a.text.as_str(), "-C" | "-d" | "--directory"))
                    .and_then(|i| args.get(i + 1))
                    .map(|a| self.path(&a.text))
                    .unwrap_or_else(|| self.cwd.clone());
                let tree = self.file_node(&dir, loc.clone());
                for a in pos
                    .iter()
                    .filter(|a| path_like(&a.text) || is_archive(&a.text))
                {
                    read(self, &a.text, &loc);
                }
                self.g.edge(proc, tree, E::Writes, loc.clone());
                self.trees.push((dir, tree));
            }
            _ => {
                if let Some(d) = target_dir {
                    for a in &pos {
                        read(self, &a.text, &loc);
                        let name = base(&a.text).to_owned();
                        write(self, &format!("{d}/{name}"), &loc);
                    }
                } else if b == "install"
                    && args.iter().any(|a| {
                        a.text == "-d"
                            || (a.text.starts_with('-')
                                && !a.text.starts_with("--")
                                && a.text.contains('d')
                                && !a.text.contains("D"))
                    })
                {
                    for a in &pos {
                        write(self, &a.text, &loc);
                    }
                } else if let Some((dst, srcs)) = pos.split_last() {
                    let dst_text = dst.text.clone();
                    for s in srcs {
                        read(self, &s.text, &loc);
                    }
                    if srcs.len() > 1 || dst_text.ends_with('/') {
                        for s in srcs {
                            write(
                                self,
                                &format!("{}/{}", dst_text.trim_end_matches('/'), base(&s.text)),
                                &loc,
                            );
                        }
                    } else {
                        write(self, &dst_text, &loc);
                    }
                }
            }
        }
    }
}

fn interp(x: &str) -> bool {
    INTERPRETERS.contains(&x)
        || (x.starts_with("python") && x[6..].bytes().all(|b| b.is_ascii_digit() || b == b'.'))
}

fn positional(args: &[Val]) -> Vec<&Val> {
    args.iter().filter(|a| !a.text.starts_with('-')).collect()
}

/// Positionals, skipping the values of the options these tools take.
fn positional_opts<'v>(b: &str, args: &'v [Val]) -> Vec<&'v Val> {
    let takes: &[&str] = match b {
        "install" => &[
            "-m",
            "--mode",
            "-o",
            "--owner",
            "-g",
            "--group",
            "-t",
            "--target-directory",
            "-S",
            "--suffix",
        ],
        "cp" | "mv" | "ln" => &["-t", "--target-directory", "-S"],
        "tar" | "bsdtar" => &[
            "-C",
            "--directory",
            "-f",
            "--file",
            "--exclude",
            "--strip-components",
        ],
        "unzip" => &["-d", "-x"],
        "sed" => &["-e", "-f"],
        "tee" | "touch" | "mkdir" | "rm" => &["-m", "--mode", "-r", "-d"],
        _ => &[],
    };
    let mut out = Vec::new();
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
            continue;
        }
        if takes.contains(&a.text.as_str()) {
            skip = !matches!(
                (b, a.text.as_str()),
                ("tee" | "touch" | "mkdir" | "rm", "-r" | "-d")
            );
            if matches!(b, "tar" | "bsdtar") && (a.text == "-f" || a.text == "--file") {
                skip = false;
            }
            continue;
        }
        if !a.text.starts_with('-') {
            out.push(a);
        }
    }
    if matches!(b, "tar" | "bsdtar") {
        // The first bundled word (`xf`, `czf`) is flags, not a file.
        if out
            .first()
            .is_some_and(|a| a.text.len() <= 6 && !a.text.contains('.') && !a.text.contains('/'))
        {
            out.remove(0);
        }
    }
    out
}

pub fn path_like(s: &str) -> bool {
    if s.is_empty() || is_url(s) || s.contains('\n') {
        return false;
    }
    // An escaped space is part of a name; a space in anything else is prose.
    let rooted = s.starts_with('/') || s.starts_with('$') || s.starts_with('~');
    rooted || (s.contains('/') && !s.contains(' '))
}

pub fn is_archive(n: &str) -> bool {
    let n = n.to_ascii_lowercase();
    [
        ".tar",
        ".tar.gz",
        ".tgz",
        ".tar.xz",
        ".txz",
        ".tar.bz2",
        ".tbz",
        ".tbz2",
        ".tar.zst",
        ".tar.lz",
        ".tar.lz4",
        ".zip",
        ".7z",
        ".deb",
        ".rpm",
        ".whl",
        ".crate",
        ".gem",
        ".appimage",
    ]
    .iter()
    .any(|e| n.ends_with(e))
}

pub fn stem(n: &str) -> &str {
    for e in [
        ".tar.gz", ".tar.xz", ".tar.bz2", ".tar.zst", ".tar.lz", ".tar.lz4", ".tgz", ".txz",
        ".tbz2", ".tbz", ".tar", ".zip", ".7z", ".crate",
    ] {
        if let Some(s) = n.strip_suffix(e) {
            return s;
        }
    }
    n
}

/// `eval a=\$b` only moves a value around: no substitution inside, so nothing new runs.
fn assignment_only(t: &str) -> bool {
    let t = t.trim();
    let Some((name, rest)) = t.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b == b'{' || b == b'}')
        && !rest.contains("$(")
        && !rest.contains('`')
        && !rest.contains(';')
        && !rest.contains('|')
}

fn normalize(p: &str) -> String {
    let (root, rest) = if let Some(r) = p.strip_prefix('/') {
        ("/".to_owned(), r)
    } else if let Some(i) = p.find('/') {
        (p[..=i].to_owned(), &p[i + 1..])
    } else {
        return p.to_owned();
    };
    let mut parts: Vec<&str> = Vec::new();
    for s in rest.split('/') {
        match s {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            x => parts.push(x),
        }
    }
    format!("{root}{}", parts.join("/"))
        .trim_end_matches('/')
        .to_owned()
}

fn unslash(s: &str) -> String {
    s.replace("\\\"", "\"")
        .replace("\\$", "$")
        .replace("\\`", "`")
        .replace("\\\\", "\\")
}

fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() {
            match b[i + 1] {
                b'x' => {
                    let h = s
                        .get(i + 2..i + 4)
                        .and_then(|h| u8::from_str_radix(h, 16).ok());
                    if let Some(v) = h {
                        out.push(v);
                        i += 4;
                        continue;
                    }
                }
                b'n' => {
                    out.push(b'\n');
                    i += 2;
                    continue;
                }
                b't' => {
                    out.push(b'\t');
                    i += 2;
                    continue;
                }
                _ => {}
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn unbase64(t: &str) -> Option<String> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for c in t.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=') {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    String::from_utf8(out).ok()
}

fn glob_tail(pat: &str) -> (bool, bool, &str) {
    let lead = pat.starts_with('*');
    let trail = pat.ends_with('*');
    (lead, trail, pat.trim_matches('*'))
}

fn strip_suffix(v: &str, pat: &str, greedy: bool) -> String {
    let (_, trail, core) = glob_tail(pat);
    if trail {
        let at = if greedy { v.find(core) } else { v.rfind(core) };
        return at.map_or(v, |i| &v[..i]).to_owned();
    }
    v.strip_suffix(core).unwrap_or(v).to_owned()
}

fn strip_prefix(v: &str, pat: &str, greedy: bool) -> String {
    let (lead, _, core) = glob_tail(pat);
    if lead {
        let at = if greedy { v.rfind(core) } else { v.find(core) };
        return at.map_or(v, |i| &v[i + core.len()..]).to_owned();
    }
    v.strip_prefix(core).unwrap_or(v).to_owned()
}
