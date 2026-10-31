// SPDX-License-Identifier: GPL-2.0-or-later
//! strace text to graph, the shape the static side uses. Reads wopkg's trace of
//! `-f -e trace=%file,%network,%process`. Failed calls count: the sandbox home is empty, so reaching
//! for `~/.ssh` answers ENOENT, and the attempt is the signal.
//!
//! The trace holds no read or write calls, so pipes are not followed. A secret read anywhere in a
//! run is taken as available to whatever connects later in that run. That over-approximates on
//! purpose, and a run that reads no secret is not affected.
use crate::graph::{EdgeKind as E, Graph, Loc, NodeId, NodeKind as K, Stage};
use crate::paths;
use std::collections::{HashMap, HashSet};

/// Files kept per trace. Laws only need the sensitive ones, which are always kept.
const MAX_FILES: usize = 5000;

/// Sandbox paths back to the names a PKGBUILD uses.
pub fn normalize(p: &str) -> String {
    let clean = squash(p);
    if let Some(r) = clean.strip_prefix("/work/.build/") {
        let mut it = r.splitn(3, '/');
        let _pkg = it.next();
        match (it.next(), it.next()) {
            (Some("src"), rest) => return join("$srcdir", rest),
            (Some("pkg"), Some(rest)) => {
                let tail = rest.split_once('/').map(|x| x.1);
                return join("$pkgdir", tail);
            }
            _ => {}
        }
    }
    if let Some(r) = clean.strip_prefix("/work/home") {
        if r.is_empty() || r.starts_with('/') {
            return format!("$HOME{r}");
        }
    }
    if let Some(r) = clean.strip_prefix("/work") {
        if r.is_empty() || r.starts_with('/') {
            return format!("$startdir{r}");
        }
    }
    clean
}

fn join(root: &str, rest: Option<&str>) -> String {
    match rest {
        Some(r) if !r.is_empty() => format!("{root}/{r}"),
        _ => root.to_owned(),
    }
}

fn squash(p: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for s in p.split('/') {
        match s {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            x => parts.push(x),
        }
    }
    format!("/{}", parts.join("/"))
}

/// The n-th double-quoted string in a syscall's arguments.
fn quoted(s: &str, n: usize) -> Option<String> {
    let b = s.as_bytes();
    let (mut i, mut seen) = (0, 0);
    while i < b.len() {
        if b[i] == b'"' {
            let mut out = Vec::new();
            i += 1;
            while i < b.len() && b[i] != b'"' {
                if b[i] == b'\\' && i + 1 < b.len() {
                    i += 1;
                }
                out.push(b[i]);
                i += 1;
            }
            if seen == n {
                return Some(String::from_utf8_lossy(&out).into_owned());
            }
            seen += 1;
        }
        i += 1;
    }
    None
}

/// Tools that read their own settings under the sandbox's empty home on every run.
fn own_config(label: &str, path: &str) -> bool {
    match label {
        "gpg" | "gpg2" | "gpgv" | "gpg-agent" | "dirmngr" | "gpgconf" | "gpgsm" | "keyboxd" => {
            path.contains("/.gnupg")
        }
        "npm" | "npx" | "pnpm" | "yarn" => path.contains("/.npmrc"),
        "cargo" | "rustc" => path.contains("/.cargo/credentials"),
        "git" | "git-remote-https" | "git-remote-http" => path.contains("/.git-credentials"),
        "curl" | "wget" => path.contains("/.netrc"),
        _ => false,
    }
}

#[derive(Default)]
struct Trace {
    proc_of: HashMap<u32, NodeId>,
    cwd: HashMap<u32, String>,
    fds: HashMap<(u32, i64), String>,
    pending: HashMap<u32, String>,
    seen: HashSet<(NodeId, NodeId, E)>,
    secrets: Vec<NodeId>,
    execs: u32,
    files: usize,
}

struct Run<'a> {
    g: Graph,
    t: Trace,
    stage: Stage,
    file: &'a str,
}

impl Run<'_> {
    fn loc(&self, line: u32) -> Option<Loc> {
        Some(Loc {
            file: self.file.to_owned(),
            line,
        })
    }

    fn proc(&mut self, pid: u32, line: u32) -> NodeId {
        if let Some(&p) = self.t.proc_of.get(&pid) {
            return p;
        }
        let key = format!("obs:{}:{pid}", self.file);
        let loc = self.loc(line);
        let id = self.g.node(K::Process, &key, "a process", loc, self.stage);
        self.t.proc_of.insert(pid, id);
        id
    }

    fn resolve(&self, pid: u32, dir: Option<i64>, path: &str) -> String {
        if path.starts_with('/') {
            return normalize(path);
        }
        let base = dir
            .and_then(|d| self.t.fds.get(&(pid, d)))
            .or_else(|| self.t.cwd.get(&pid))
            .cloned()
            .unwrap_or_else(|| "/work".to_owned());
        normalize(&format!("{base}/{path}"))
    }

    fn link(&mut self, src: NodeId, dst: NodeId, kind: E, line: u32) {
        if self.t.seen.insert((src, dst, kind)) {
            let loc = self.loc(line);
            self.g.edge(src, dst, kind, loc);
        }
    }

    fn sensitive(&self, p: &str) -> bool {
        paths::credential(p).is_some() || paths::persistence(p) || paths::outside_package(p)
    }

    fn file(&mut self, p: &str, line: u32) -> Option<NodeId> {
        if self.g.find(p).is_none() {
            if self.t.files >= MAX_FILES && !self.sensitive(p) {
                return None;
            }
            self.t.files += 1;
        }
        let loc = self.loc(line);
        Some(self.g.node(K::File, p, p, loc, self.stage))
    }

    fn write(&mut self, pid: u32, p: &str, line: u32) {
        // Python's bytecode cache is an attempt on every import, and nothing else.
        if p.contains("/__pycache__/") || paths::harmless(p) {
            return;
        }
        let proc = self.proc(pid, line);
        if let Some(f) = self.file(p, line) {
            self.link(proc, f, E::Writes, line);
        }
    }

    fn read(&mut self, pid: u32, p: &str, ok: bool, line: u32) {
        let secret = paths::credential(p).is_some();
        if !ok && !secret {
            return;
        }
        let proc = self.proc(pid, line);
        if secret {
            let label = self.g.nodes[proc as usize].label.clone();
            if own_config(&label, p) {
                return;
            }
        } else if !paths::inside_build(p) {
            return;
        }
        if let Some(f) = self.file(p, line) {
            self.link(f, proc, E::Reads, line);
            if secret {
                self.t.secrets.push(f);
            }
        }
    }

    fn connect(&mut self, pid: u32, args: &str, line: u32) {
        let addr = if args.contains("AF_INET6") {
            quoted(args, 0).or_else(|| Some("?".into()))
        } else if args.contains("AF_INET") {
            quoted(args, 0)
        } else {
            None
        };
        let Some(addr) = addr else { return };
        if addr.starts_with("127.") || addr == "::1" || addr == "0.0.0.0" {
            return;
        }
        let port = args
            .split("htons(")
            .nth(1)
            .and_then(|r| r.split(')').next())
            .unwrap_or("?");
        let proc = self.proc(pid, line);
        let key = format!("sock:{addr}:{port}");
        let label = format!("{addr}:{port}");
        let loc = self.loc(line);
        let s = self.g.node(K::Socket, &key, &label, loc, self.stage);
        self.link(s, proc, E::Fetches, line);
        self.link(proc, s, E::Sends, line);
        for f in self.t.secrets.clone() {
            self.link(f, proc, E::Reads, line);
        }
    }

    fn exec(&mut self, pid: u32, args: &str, line: u32) {
        let Some(path) = quoted(args, 0) else { return };
        let argv0 = quoted(args, 1).unwrap_or_else(|| path.clone());
        let label = argv0.rsplit('/').next().unwrap_or(&argv0).to_owned();
        self.t.execs += 1;
        let key = format!("obs:{}:{pid}:{}", self.file, self.t.execs);
        let loc = self.loc(line);
        let id = self
            .g
            .node(K::Process, &key, &label, loc.clone(), self.stage);
        let c = self.g.node(
            K::Command,
            &format!("cmd:{label}"),
            &label,
            None,
            self.stage,
        );
        self.g.edge(c, id, E::Calls, loc);
        if let Some(parent) = self.t.proc_of.insert(pid, id) {
            self.g.edge(parent, id, E::Calls, None);
        }
        let p = self.resolve(pid, None, &path);
        if !p.starts_with("/usr/") && !p.starts_with("/bin/") {
            if let Some(f) = self.file(&p, line) {
                self.link(f, id, E::Executes, line);
            }
        }
    }

    fn syscall(&mut self, pid: u32, name: &str, args: &str, ret: &str, line: u32) {
        let ok = !ret.starts_with('-');
        let num = ret
            .split_whitespace()
            .next()
            .and_then(|x| x.parse::<i64>().ok());
        let dirfd = || {
            args.split(',')
                .next()
                .and_then(|x| x.trim().parse::<i64>().ok())
        };
        match name {
            "execve" | "execveat" if ok => self.exec(pid, args, line),
            "clone" | "clone3" | "fork" | "vfork" => {
                if let Some(child) = num.filter(|n| *n > 0) {
                    let child = child as u32;
                    if let Some(p) = self.t.proc_of.get(&pid).copied() {
                        self.t.proc_of.entry(child).or_insert(p);
                    }
                    if let Some(c) = self.t.cwd.get(&pid).cloned() {
                        self.t.cwd.entry(child).or_insert(c);
                    }
                }
            }
            "chdir" if ok => {
                if let Some(p) = quoted(args, 0) {
                    let dir = if p.starts_with('/') {
                        p
                    } else {
                        format!(
                            "{}/{p}",
                            self.t
                                .cwd
                                .get(&pid)
                                .cloned()
                                .unwrap_or_else(|| "/work".into())
                        )
                    };
                    self.t.cwd.insert(pid, squash(&dir));
                }
            }
            "open" | "openat" | "openat2" | "creat" => {
                let Some(raw) = quoted(args, 0) else { return };
                let at = (name != "open" && name != "creat").then(dirfd).flatten();
                let p = self.resolve(pid, at, &raw);
                let writes = name == "creat"
                    || args.contains("O_WRONLY")
                    || args.contains("O_RDWR")
                    || args.contains("O_CREAT")
                    || args.contains("O_TRUNC")
                    || args.contains("O_APPEND");
                if writes {
                    self.write(pid, &p, line);
                } else {
                    self.read(pid, &p, ok, line);
                }
                if let Some(fd) = num.filter(|n| *n >= 0) {
                    self.t
                        .fds
                        .insert((pid, fd), squash(&raw_abs(&raw, self.t.cwd.get(&pid))));
                }
            }
            "stat" | "lstat" | "access" | "faccessat" | "faccessat2" | "newfstatat" | "statx"
            | "readlink" | "readlinkat" | "getdents64" => {
                if let Some(raw) = quoted(args, 0) {
                    let p = self.resolve(pid, None, &raw);
                    if paths::credential(&p).is_some() {
                        self.read(pid, &p, ok, line);
                    }
                }
            }
            "unlink" | "unlinkat" | "mkdir" | "mkdirat" | "rmdir" | "chmod" | "fchmodat"
            | "fchmodat2" | "chown" | "lchown" | "fchownat" | "truncate" | "mknod" | "mknodat"
            | "utimensat" => {
                if let Some(raw) = quoted(args, 0) {
                    let p = self.resolve(pid, None, &raw);
                    self.write(pid, &p, line);
                }
            }
            "rename" | "renameat" | "renameat2" | "link" | "linkat" | "symlink" | "symlinkat" => {
                let target = if name.starts_with("symlink")
                    || name.starts_with("link")
                    || name.starts_with("rename")
                {
                    1
                } else {
                    0
                };
                if let Some(raw) = quoted(args, target) {
                    let p = self.resolve(pid, None, &raw);
                    self.write(pid, &p, line);
                }
            }
            "connect" | "sendto" | "sendmsg" => self.connect(pid, args, line),
            _ => {}
        }
    }
}

fn raw_abs(raw: &str, cwd: Option<&String>) -> String {
    if raw.starts_with('/') {
        raw.to_owned()
    } else {
        format!("{}/{raw}", cwd.map_or("/work", |c| c.as_str()))
    }
}

/// One traced run as a graph. Lines that do not parse are skipped, never fatal.
pub fn parse(trace: &str, stage: Stage, file: &str) -> Graph {
    let mut r = Run {
        g: Graph::new(),
        t: Trace::default(),
        stage,
        file,
    };
    for (i, raw) in trace.lines().enumerate() {
        let line = i as u32 + 1;
        let Some((pid, rest)) = raw.split_once(' ') else {
            continue;
        };
        let Ok(pid) = pid.parse::<u32>() else {
            continue;
        };
        let rest = rest.trim_start();
        if let Some(head) = rest.strip_suffix("<unfinished ...>") {
            r.t.pending.insert(pid, head.trim_end().to_owned());
            continue;
        }
        let joined;
        let text = if let Some(tail) = rest.strip_prefix("<... ") {
            let Some((_, after)) = tail.split_once("resumed>") else {
                continue;
            };
            let Some(head) = r.t.pending.remove(&pid) else {
                continue;
            };
            joined = format!("{head}{after}");
            joined.as_str()
        } else {
            rest
        };
        let Some((name, tail)) = text.split_once('(') else {
            continue;
        };
        let Some((args, ret)) = tail.rsplit_once(" = ") else {
            continue;
        };
        r.syscall(pid, name, args, ret.trim(), line);
    }
    r.g
}

/// Why this trace cannot be trusted, if it cannot. The trace sits in a folder the build can write,
/// so a hostile build may have emptied, overwritten or punched holes in it.
pub fn damage(trace: &str, g: &Graph) -> Option<String> {
    if trace.trim().is_empty() {
        return Some("The build trace is empty, so what the build did was not seen.".to_owned());
    }
    if trace.contains('\0') {
        return Some(
            "The build trace has holes in it, so part of what the build did was not seen."
                .to_owned(),
        );
    }
    if !trace
        .lines()
        .any(|l| l.contains(" execve(") && l.ends_with("= 0"))
    {
        return Some(
            "The build trace shows no program starting, so it is not a trace of this build."
                .to_owned(),
        );
    }
    g.find("$startdir/.trace")
        .map(|_| "The build touched its own trace, so the trace cannot be trusted.".to_owned())
}
