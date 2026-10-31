// SPDX-License-Identifier: GPL-2.0-or-later
//! Other languages' build scripts, read into the same graph. One walker over calls and strings,
//! told by tables what each call means. Commands found in strings go to the shell walker.
//! This reads build scripts, not applications: a network client doing its job is not a finding.
use crate::graph::{EdgeKind as E, Graph, Loc, NodeId, Stage};
use crate::shell::{Scope, Val, Walker};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tree_sitter::{Language, Node, ParseOptions, Parser, Tree};

const MAX_DEPTH: usize = 64;
/// A file that takes longer than this to parse is reported, not waited for.
const PARSE_BUDGET: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lang {
    Python,
    Js,
    Go,
    Rust,
    C,
}

impl Lang {
    fn grammar(self) -> Language {
        match self {
            Lang::Python => tree_sitter_python::LANGUAGE.into(),
            Lang::Js => tree_sitter_javascript::LANGUAGE.into(),
            Lang::Go => tree_sitter_go::LANGUAGE.into(),
            Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
            Lang::C => tree_sitter_c::LANGUAGE.into(),
        }
    }
}

/// Parse with a deadline, so hostile input cannot hold an install up.
pub(crate) fn parse(lang: &Language, text: &str) -> Option<Tree> {
    let mut p = Parser::new();
    p.set_language(lang).ok()?;
    let deadline = Instant::now() + PARSE_BUDGET;
    let mut progress = |_: &tree_sitter::ParseState| Instant::now() > deadline;
    let opts = ParseOptions::new().progress_callback(&mut progress);
    let bytes = text.as_bytes();
    p.parse_with_options(&mut |i, _| bytes.get(i..).unwrap_or(&[]), None, Some(opts))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Does {
    Code,
    Shell,
    Argv,
    Decode,
    Fetch,
    Connect,
    Read,
    Write,
    Open,
    Home,
    Join,
}

fn table(lang: Lang) -> &'static [(&'static str, Does)] {
    use Does::*;
    match lang {
        Lang::Python => &[
            ("exec", Code),
            ("eval", Code),
            ("os.system", Shell),
            ("os.popen", Shell),
            ("subprocess.getoutput", Shell),
            ("subprocess.getstatusoutput", Shell),
            ("commands.getoutput", Shell),
            ("subprocess.run", Argv),
            ("subprocess.call", Argv),
            ("subprocess.check_call", Argv),
            ("subprocess.check_output", Argv),
            ("subprocess.Popen", Argv),
            ("base64.b64decode", Decode),
            ("base64.b32decode", Decode),
            ("base64.b16decode", Decode),
            ("base64.b85decode", Decode),
            ("base64.a85decode", Decode),
            ("base64.urlsafe_b64decode", Decode),
            ("base64.decodebytes", Decode),
            ("binascii.unhexlify", Decode),
            ("binascii.a2b_base64", Decode),
            ("bytes.fromhex", Decode),
            ("codecs.decode", Decode),
            ("urllib.request.urlopen", Fetch),
            ("urllib.request.urlretrieve", Fetch),
            ("request.urlopen", Fetch),
            ("request.urlretrieve", Fetch),
            ("u.urlopen", Fetch),
            ("urlopen", Fetch),
            ("urlretrieve", Fetch),
            ("urllib2.urlopen", Fetch),
            ("requests.get", Fetch),
            ("requests.post", Fetch),
            ("requests.put", Fetch),
            ("httpx.get", Fetch),
            ("httpx.post", Fetch),
            ("socket.create_connection", Connect),
            ("open", Open),
            ("io.open", Open),
            ("os.path.expanduser", Home),
            ("os.path.join", Join),
            ("Path.home", Home),
        ],
        Lang::Js => &[
            ("eval", Code),
            ("Function", Code),
            ("vm.runInThisContext", Code),
            ("vm.runInNewContext", Code),
            ("exec", Shell),
            ("execSync", Shell),
            ("spawn", Argv),
            ("spawnSync", Argv),
            ("execFile", Argv),
            ("execFileSync", Argv),
            ("Buffer.from", Decode),
            ("atob", Decode),
            ("fetch", Fetch),
            ("axios.get", Fetch),
            ("axios.post", Fetch),
            ("http.get", Fetch),
            ("https.get", Fetch),
            ("http.request", Fetch),
            ("https.request", Fetch),
            ("net.connect", Connect),
            ("net.createConnection", Connect),
            ("fs.readFile", Read),
            ("fs.readFileSync", Read),
            ("fs.writeFile", Write),
            ("fs.writeFileSync", Write),
            ("fs.appendFile", Write),
            ("fs.appendFileSync", Write),
            ("os.homedir", Home),
            ("path.join", Join),
        ],
        Lang::Go => &[
            ("exec.Command", Argv),
            ("exec.CommandContext", Argv),
            ("base64.StdEncoding.DecodeString", Decode),
            ("base64.URLEncoding.DecodeString", Decode),
            ("hex.DecodeString", Decode),
            ("http.Get", Fetch),
            ("http.Post", Fetch),
            ("net.Dial", Connect),
            ("os.ReadFile", Read),
            ("ioutil.ReadFile", Read),
            ("os.Open", Read),
            ("os.WriteFile", Write),
            ("ioutil.WriteFile", Write),
            ("os.Create", Write),
            ("os.UserHomeDir", Home),
            ("filepath.Join", Join),
        ],
        Lang::Rust => &[
            ("Command::new", Argv),
            ("process::Command::new", Argv),
            ("reqwest::get", Fetch),
            ("reqwest::blocking::get", Fetch),
            ("ureq::get", Fetch),
            ("TcpStream::connect", Connect),
            ("fs::read", Read),
            ("fs::read_to_string", Read),
            ("File::open", Read),
            ("fs::write", Write),
            ("File::create", Write),
            ("dirs::home_dir", Home),
            ("env::home_dir", Home),
            ("base64::decode", Decode),
        ],
        Lang::C => &[
            ("system", Shell),
            ("popen", Shell),
            ("execl", Argv),
            ("execlp", Argv),
            ("fopen", Open),
            ("getenv", Home),
        ],
    }
}

struct Calls<'w, 'g> {
    w: &'w mut Walker<'g>,
    lang: Lang,
    src: &'w str,
    file: String,
    vars: HashMap<String, Val>,
    child_process: bool,
    depth: usize,
    /// Set for code held in a string: every location is that one line.
    at: Option<u32>,
}

impl Calls<'_, '_> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.src.as_bytes()).unwrap_or("")
    }

    fn loc(&self, n: Node) -> Option<Loc> {
        Some(Loc {
            file: self.file.clone(),
            line: self.at.unwrap_or(n.start_position().row as u32 + 1),
        })
    }

    fn named(n: Node<'_>) -> Vec<Node<'_>> {
        let mut c = n.walk();
        n.named_children(&mut c).collect()
    }

    /// Everything a statement does, then what it means for variables.
    fn visit(&mut self, n: Node) {
        if self.depth >= MAX_DEPTH {
            self.w.truncated = true;
            return;
        }
        self.depth += 1;
        self.visit_inner(n);
        self.depth -= 1;
    }

    fn visit_inner(&mut self, n: Node) {
        match n.kind() {
            "assignment"
            | "assignment_expression"
            | "variable_declarator"
            | "let_declaration"
            | "init_declarator"
            | "short_var_declaration"
            | "assignment_statement"
            | "var_spec"
            | "const_spec" => {
                let (name, value) = (self.target(n), self.value(n));
                if let (Some(name), Some(v)) = (name, value) {
                    let val = self.eval(v);
                    self.vars.insert(name, val);
                    return;
                }
                for c in Self::named(n) {
                    self.visit(c);
                }
            }
            "call" | "call_expression" | "new_expression" => {
                self.eval(n);
            }
            "comment" | "line_comment" | "block_comment" => {
                if self.lang == Lang::Go {
                    if let Some(cmd) = self.text(n).strip_prefix("//go:generate ") {
                        let (cmd, line) = (cmd.to_owned(), n.start_position().row as u32 + 1);
                        let file = self.file.clone();
                        self.w.snippet(&cmd, &file, line, Stage::Upstream);
                    }
                }
            }
            _ => {
                for c in Self::named(n) {
                    self.visit(c);
                }
            }
        }
    }

    fn field<'t>(n: Node<'t>, names: &[&str]) -> Option<Node<'t>> {
        names.iter().find_map(|f| n.child_by_field_name(f))
    }

    fn target(&self, n: Node) -> Option<String> {
        let t = Self::field(n, &["left", "name", "pattern", "declarator"])?;
        let s = self
            .text(t)
            .trim_start_matches('*')
            .trim_start_matches("mut ")
            .trim();
        s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
            .then(|| s.to_owned())
    }

    fn value<'t>(&self, n: Node<'t>) -> Option<Node<'t>> {
        Self::field(n, &["right", "value"])
    }

    fn literal(&self, n: Node) -> Option<String> {
        let t = self.text(n);
        match n.kind() {
            "string"
            | "string_literal"
            | "interpreted_string_literal"
            | "raw_string_literal"
            | "template_string" => {
                let has_hole = Self::named(n)
                    .iter()
                    .any(|c| matches!(c.kind(), "interpolation" | "template_substitution"));
                if has_hole {
                    return None;
                }
                let body = t.trim_start_matches(|c: char| c.is_ascii_alphabetic());
                let q = body.chars().next()?;
                let triple: String = std::iter::repeat_n(q, 3).collect();
                let inner = if body.starts_with(&triple) && body.len() >= 6 {
                    &body[3..body.len() - 3]
                } else if body.len() >= 2 {
                    &body[1..body.len() - 1]
                } else {
                    ""
                };
                Some(
                    inner
                        .replace("\\n", "\n")
                        .replace("\\t", "\t")
                        .replace("\\\"", "\"")
                        .replace("\\'", "'"),
                )
            }
            _ => None,
        }
    }

    fn eval(&mut self, n: Node) -> Val {
        if self.depth >= MAX_DEPTH {
            self.w.truncated = true;
            return Val::default();
        }
        self.depth += 1;
        let v = self.eval_inner(n);
        self.depth -= 1;
        v
    }

    fn union(&mut self, nodes: &[Node]) -> Val {
        let mut out = Val::default();
        for c in nodes {
            let v = self.eval(*c);
            out.taints.extend(v.taints);
            out.local |= v.local;
        }
        out
    }

    fn eval_inner(&mut self, n: Node) -> Val {
        if let Some(s) = self.literal(n) {
            return Val::known(&s);
        }
        match n.kind() {
            "identifier" => self
                .vars
                .get(self.text(n))
                .cloned()
                .unwrap_or_else(|| Val::unknown(self.text(n).to_owned())),
            "call" | "call_expression" | "new_expression" => self.call(n),
            "binary_operator" | "binary_expression" => {
                let parts = Self::named(n);
                let mut out = Val::known("");
                for p in parts {
                    if p.is_named() {
                        let v = self.eval(p);
                        out.append(v);
                    }
                }
                out
            }
            "parenthesized_expression"
            | "await_expression"
            | "try_expression"
            | "reference_expression"
            | "unary_expression"
                if n.named_child_count() == 1 =>
            {
                n.named_child(0).map(|c| self.eval(c)).unwrap_or_default()
            }
            _ => {
                let kids = Self::named(n);
                self.union(&kids)
            }
        }
    }

    fn args<'t>(&self, n: Node<'t>) -> Vec<Node<'t>> {
        let a = Self::field(n, &["arguments"]);
        a.map(|a| {
            Self::named(a)
                .into_iter()
                .filter(|c| c.kind() != "comment")
                .collect()
        })
        .unwrap_or_default()
    }

    fn callee(&self, n: Node) -> String {
        let f = Self::field(n, &["function", "constructor"]);
        f.map(|f| self.text(f).split_whitespace().collect::<String>())
            .unwrap_or_default()
    }

    /// The longest table entry the callee ends with, on a name boundary.
    fn meaning(&self, callee: &str) -> Option<Does> {
        let t = table(self.lang);
        t.iter()
            .filter(|(k, _)| {
                callee == *k
                    || callee
                        .strip_suffix(k)
                        .is_some_and(|p| p.ends_with(['.', ':']))
            })
            .max_by_key(|(k, _)| k.len())
            .map(|x| x.1)
    }

    fn call(&mut self, n: Node) -> Val {
        let callee = self.callee(n);
        let args = self.args(n);
        let loc = self.loc(n);
        let does = self.meaning(&callee);
        let does = match (self.lang, does) {
            // `regex.exec()` and `obj.exec()` are not child_process.
            (Lang::Js, Some(Does::Shell | Does::Argv))
                if !(self.child_process || callee.contains("child_process")) =>
            {
                None
            }
            (Lang::Js, Some(Does::Decode))
                if callee.ends_with("Buffer.from")
                    && !args.iter().any(|a| {
                        matches!(
                            self.text(*a).trim_matches(['"', '\'']),
                            "base64" | "hex" | "base64url"
                        )
                    }) =>
            {
                None
            }
            // `re.compile` and `model.eval()` are methods; the builtins stand alone.
            (Lang::Python, Some(Does::Code)) if callee.contains('.') => None,
            (_, d) => d,
        };
        let name = callee
            .rsplit(['.', ':'])
            .next()
            .unwrap_or(&callee)
            .to_owned();
        match does {
            Some(Does::Code) => {
                let v = args.first().map(|a| self.eval(*a)).unwrap_or_default();
                let proc = self.w.process(&name, loc.clone());
                self.w.exec_value(&v, proc, &loc, self.lang);
                Val::default()
            }
            Some(Does::Shell) => {
                let v = args.first().map(|a| self.eval(*a)).unwrap_or_default();
                let proc = self.w.process(&name, loc.clone());
                self.w.exec_value(&v, proc, &loc, Lang::C);
                self.piped(proc, &loc)
            }
            Some(Does::Argv) => self.argv(n, &args, loc),
            Some(Does::Decode) => {
                let v = args.first().map(|a| self.eval(*a)).unwrap_or_default();
                let proc = self.w.process(&name, loc.clone());
                for t in &v.taints {
                    self.w.g.edge(*t, proc, E::Decodes, loc.clone());
                }
                if v.taints.is_empty() {
                    let src = self.w.literal_source(&loc);
                    self.w.g.edge(src, proc, E::Decodes, loc.clone());
                }
                self.piped(proc, &loc)
            }
            Some(Does::Fetch | Does::Connect) => {
                let v = args.first().map(|a| self.eval(*a)).unwrap_or_default();
                let rest: Vec<Node> = args.iter().skip(1).copied().collect();
                let body = self.union(&rest);
                let proc = self.w.process(&name, loc.clone());
                let url = self.w.url_node(
                    if v.complete {
                        &v.text
                    } else {
                        "an address the script computes"
                    },
                    loc.clone(),
                );
                self.w.g.edge(url, proc, E::Fetches, loc.clone());
                for t in v.taints.iter().chain(&body.taints) {
                    self.w.g.edge(*t, proc, E::Reads, loc.clone());
                }
                self.w.g.edge(proc, url, E::Sends, loc.clone());
                self.piped(proc, &loc)
            }
            Some(Does::Open) => {
                let path = args.first().map(|a| self.eval(*a)).unwrap_or_default();
                let mode = args
                    .get(1)
                    .map(|a| self.text(*a).to_owned())
                    .unwrap_or_default();
                if mode.contains(['w', 'a', 'x', '+']) {
                    self.file_write(&path, &name, &loc)
                } else {
                    self.file_read(&path, &name, &loc)
                }
            }
            Some(Does::Read) => {
                let path = args.first().map(|a| self.eval(*a)).unwrap_or_default();
                self.file_read(&path, &name, &loc)
            }
            Some(Does::Write) => {
                let path = args.first().map(|a| self.eval(*a)).unwrap_or_default();
                self.file_write(&path, &name, &loc)
            }
            Some(Does::Home) => {
                let v = args.first().map(|a| self.eval(*a)).unwrap_or_default();
                match v.text.strip_prefix('~') {
                    Some(r) if v.complete => Val::known(&format!("$HOME{r}")),
                    _ => Val::known("$HOME"),
                }
            }
            Some(Does::Join) => {
                let mut parts = Vec::new();
                let mut out = Val::known("");
                for a in &args {
                    let v = self.eval(*a);
                    parts.push(v.text.clone());
                    out.taints.extend(v.taints);
                    out.complete &= v.complete;
                }
                out.text = parts.join("/");
                out
            }
            None => {
                // A method passes on what its receiver and arguments carried: `open(x).read()`.
                let f = Self::field(n, &["function"]);
                let recv = f.and_then(|f| Self::field(f, &["object", "operand", "value"]));
                let mut kids: Vec<Node> = args;
                kids.extend(recv);
                self.union(&kids)
            }
        }
    }

    fn piped(&mut self, proc: NodeId, loc: &Option<Loc>) -> Val {
        let p = self.w.pipe_node(loc.clone());
        self.w.g.edge(proc, p, E::Writes, loc.clone());
        Val::tainted(p)
    }

    fn file_read(&mut self, path: &Val, name: &str, loc: &Option<Loc>) -> Val {
        let proc = self.w.process(name, loc.clone());
        for t in &path.taints {
            self.w.g.edge(*t, proc, E::Reads, loc.clone());
        }
        let mut v = self.piped(proc, loc);
        v.local = true;
        if path.complete {
            let p = self.w.path_of(&path.text);
            let f = self.w.file_at(&p, loc.clone());
            self.w.g.edge(f, proc, E::Reads, loc.clone());
        }
        v
    }

    fn file_write(&mut self, path: &Val, name: &str, loc: &Option<Loc>) -> Val {
        let proc = self.w.process(name, loc.clone());
        if path.complete {
            let p = self.w.path_of(&path.text);
            let f = self.w.file_at(&p, loc.clone());
            self.w.g.edge(proc, f, E::Writes, loc.clone());
        }
        Val::default()
    }

    /// A command as a list. Rust builds its list with `.arg()` calls chained on `Command::new`.
    fn argv(&mut self, n: Node, args: &[Node], loc: Option<Loc>) -> Val {
        let mut argv = Vec::new();
        let flatten = |s: &mut Self, a: Node, argv: &mut Vec<Val>| {
            if matches!(a.kind(), "list" | "array" | "tuple" | "array_expression") {
                for e in Self::named(a) {
                    argv.push(s.eval(e));
                }
            } else if !matches!(a.kind(), "keyword_argument" | "object") {
                argv.push(s.eval(a));
            }
        };
        let shell_true = args.iter().any(|a| {
            a.kind() == "keyword_argument" && self.text(*a).replace(' ', "") == "shell=True"
        });
        for a in args {
            flatten(self, *a, &mut argv);
        }
        if self.lang == Lang::Go && self.callee(n).ends_with("CommandContext") && !argv.is_empty() {
            argv.remove(0);
        }
        let single_string = argv.len() == 1 && argv[0].text.contains(' ');
        let mut up = n.parent();
        while let Some(p) = up {
            if !matches!(
                p.kind(),
                "field_expression" | "call_expression" | "arguments"
            ) {
                break;
            }
            if p.kind() == "call_expression" {
                let method = self.callee(p);
                if method.ends_with(".arg") {
                    for a in self.args(p) {
                        argv.push(self.eval(a));
                    }
                } else if method.ends_with(".args") {
                    for a in self.args(p) {
                        for e in Self::named(a) {
                            argv.push(self.eval(e));
                        }
                    }
                }
            }
            up = p.parent();
        }
        if argv.is_empty() {
            return Val::default();
        }
        if shell_true || single_string {
            let v = argv.remove(0);
            let proc = self.w.process("shell", loc.clone());
            self.w.exec_value(&v, proc, &loc, Lang::C);
            return self.piped(proc, &loc);
        }
        // A list built elsewhere is a command in a variable, not text run blind.
        let strict = std::mem::replace(&mut self.w.lenient, true);
        self.w.run_argv(argv, loc);
        self.w.lenient = strict;
        Val::default()
    }
}

/// Read one file's calls into the graph.
pub fn walk(
    lang: Lang,
    text: &str,
    file: &str,
    stage: Stage,
    w: &mut Walker,
    at: Option<u32>,
) -> bool {
    let Some(tree) = parse(&lang.grammar(), text) else {
        return false;
    };
    w.enter_file(file, stage);
    let mut c = Calls {
        lang,
        src: text,
        file: file.to_owned(),
        vars: HashMap::new(),
        child_process: text.contains("child_process"),
        depth: 0,
        w,
        at,
    };
    c.visit(tree.root_node());
    true
}

/// Code given on an interpreter's command line, such as `python -c`.
pub fn inline(
    g: &mut Graph,
    interp: &str,
    code: &str,
    file: &str,
    line: u32,
    stage: Stage,
    _proc: NodeId,
) {
    let lang = match interp {
        x if x.starts_with("python") => Lang::Python,
        "node" | "nodejs" | "deno" | "bun" => Lang::Js,
        _ => return,
    };
    let mut sub = Graph::new();
    let mut w = Walker::new(&mut sub, Scope::default());
    if !walk(lang, code, file, stage, &mut w, Some(line)) {
        w.truncated = true;
    }
    drop(w);
    g.merge(&sub);
}
