// SPDX-License-Identifier: GPL-2.0-or-later
//! The laws of `laws/LAWS.bend`, run over a graph. Each answers with the exact path it found.
use crate::graph::{Edge, EdgeKind as E, Graph, Loc, NodeId, NodeKind as K, Stage};
use crate::pkgbuild::Meta;
use crate::{hosts, paths};
use serde::Serialize;
use std::collections::{HashSet, VecDeque};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LawId {
    FetchExecuteUnverified,
    DecodeExecute,
    UnresolvedDynamic,
    WriteOutsidePackage,
    ReadCredentials,
    CredentialToNetwork,
    ScriptletUndeclaredFile,
    SourceOffUpstream,
    UndeclaredPersistence,
    PostinstallNetwork,
    BinChecksumUpstream,
    UnexplainedNewEdge,
    ObservedMismatch,
    /// Something could not be read or trusted, so the answer rests on less than it should.
    IncompleteEvidence,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Step {
    pub node: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loc: Option<Loc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub law: LawId,
    /// The exact path, in one plain sentence, with file and line.
    pub sentence: String,
    pub path: Vec<Step>,
    pub origin: Origin,
}

/// Which graph showed it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    #[default]
    Static,
    Observed,
}

fn finding(law: LawId, sentence: String, path: Vec<Step>) -> Finding {
    Finding {
        law,
        sentence,
        path,
        origin: Origin::Static,
    }
}

/// Edges content can move along.
fn flows(k: E) -> bool {
    matches!(
        k,
        E::Fetches | E::Writes | E::Reads | E::Decodes | E::Executes
    )
}

fn network(g: &Graph, n: NodeId) -> bool {
    matches!(g.nodes[n as usize].kind, K::Url | K::Socket)
}

pub(crate) struct View<'a> {
    pub g: &'a Graph,
    pub meta: &'a Meta,
    /// The built package's own list, when there is one: the truest answer to "what is declared".
    pub package_files: &'a [String],
    into: Vec<Vec<u32>>,
}

impl<'a> View<'a> {
    pub fn new(g: &'a Graph, meta: &'a Meta, package_files: &'a [String]) -> Self {
        let mut into = vec![Vec::new(); g.nodes.len()];
        for (i, e) in g.edges.iter().enumerate() {
            into[e.dst as usize].push(i as u32);
        }
        View {
            g,
            meta,
            package_files,
            into,
        }
    }

    fn checked_before(&self, n: NodeId, seq: u32) -> bool {
        self.into[n as usize].iter().any(|&i| {
            let e = &self.g.edges[i as usize];
            e.kind == E::Verifies && e.seq < seq
        })
    }

    /// A floating VCS source on the project's own host is declared tracking, not a stray download.
    fn declared_vcs(&self, n: NodeId) -> bool {
        let node = &self.g.nodes[n as usize];
        node.kind == K::Source
            && self.meta.sources.iter().any(|s| {
                s.vcs
                    && s.name == node.label
                    && s.url.as_deref().is_some_and(|u| {
                        crate::hosts::same_project(&self.meta.url, u, &self.meta.pkgnames)
                    })
            })
    }

    /// Walk backwards from `start` to the first edge or node `goal` accepts, without crossing a
    /// node `stop` rejects. The result reads source first.
    fn back(
        &self,
        start: NodeId,
        stop: impl Fn(NodeId) -> bool,
        goal: impl Fn(&Edge, NodeId) -> bool,
    ) -> Option<Vec<u32>> {
        let mut parent: Vec<Option<u32>> = vec![None; self.g.nodes.len()];
        let mut seen = HashSet::from([start]);
        let mut q = VecDeque::from([start]);
        while let Some(n) = q.pop_front() {
            for &i in &self.into[n as usize] {
                let e = &self.g.edges[i as usize];
                if !flows(e.kind) || stop(e.src) {
                    continue;
                }
                if goal(e, e.src) {
                    let mut path = vec![i];
                    let mut at = n;
                    while let Some(p) = parent[at as usize] {
                        path.push(p);
                        at = self.g.edges[p as usize].dst;
                    }
                    return Some(path);
                }
                if seen.insert(e.src) {
                    parent[e.src as usize] = Some(i);
                    q.push_back(e.src);
                }
            }
        }
        None
    }

    fn step(&self, n: NodeId, loc: Option<Loc>) -> Step {
        let node = &self.g.nodes[n as usize];
        Step {
            node: node.key.clone(),
            label: node.label.clone(),
            loc: loc.or_else(|| node.loc.clone()),
        }
    }

    fn at(e: &Edge, n: &crate::graph::Node) -> String {
        e.loc
            .as_ref()
            .or(n.loc.as_ref())
            .map(|l| format!(" at {l}"))
            .unwrap_or_default()
    }

    /// A scriptlet may change what its package installs, the package's own folders, and nothing else.
    fn declared(&self, p: &str) -> bool {
        let g = self.g;
        if self
            .package_files
            .iter()
            .any(|f| f == p || f.starts_with(&format!("{p}/")))
            || g.find(&format!("$pkgdir{p}")).is_some()
            || self
                .meta
                .backup
                .iter()
                .any(|b| format!("/{}", b.trim_start_matches('/')) == p)
        {
            return true;
        }
        let parent = format!("$pkgdir{p}/");
        if g.nodes.iter().any(|n| n.key.starts_with(&parent)) {
            return true;
        }
        self.meta.pkgnames.iter().any(|n| {
            let core = n.trim_end_matches("-git").trim_end_matches("-bin");
            [
                "/opt/",
                "/usr/share/",
                "/usr/lib/",
                "/etc/",
                "/var/lib/",
                "/var/log/",
                "/var/cache/",
            ]
            .iter()
            .any(|d| {
                [n.as_str(), core].iter().any(|x| {
                    p.strip_prefix(*d)
                        .is_some_and(|r| r == *x || r.starts_with(&format!("{x}/")))
                })
            })
        })
    }

    /// Did this writer copy the file from the package's own sources?
    fn from_sources(&self, proc: NodeId) -> bool {
        self.into[proc as usize].iter().any(|&i| {
            let e = &self.g.edges[i as usize];
            e.kind == E::Reads && self.g.nodes[e.src as usize].key.starts_with("$srcdir/")
        })
    }

    fn check_files(&self, out: &mut Vec<Finding>) {
        let g = self.g;
        let mut seen: HashSet<(LawId, NodeId)> = HashSet::new();
        let mut add = |law: LawId, n: NodeId, sentence: String, path: Vec<Step>| {
            if seen.insert((law, n)) {
                out.push(finding(law, sentence, path));
            }
        };
        for e in &g.edges {
            let (s, d) = (&g.nodes[e.src as usize], &g.nodes[e.dst as usize]);
            match e.kind {
                E::Reads | E::Decodes if s.kind == K::File => {
                    if let Some(what) = paths::credential(&s.key) {
                        let at = Self::at(e, d);
                        add(
                            LawId::ReadCredentials,
                            e.src,
                            format!("{} reads {} ({what}){at}.", d.label, s.label),
                            vec![self.step(e.src, None), self.step(e.dst, e.loc.clone())],
                        );
                    }
                }
                E::Writes if d.kind == K::File && s.kind == K::Process => {
                    let at = Self::at(e, s);
                    let steps = vec![self.step(e.src, e.loc.clone()), self.step(e.dst, None)];
                    let path = &d.key;
                    let scriptlet = s.stage == Stage::Scriptlet;
                    let content = !matches!(s.label.as_str(), "chmod" | "chown" | "chgrp");
                    if content && paths::persistence(path) {
                        let shipped = path.starts_with("$pkgdir") && self.from_sources(e.src);
                        if !shipped {
                            add(
                                LawId::UndeclaredPersistence,
                                e.dst,
                                format!(
                                    "{} writes {}{at}, which starts again by itself, and the package does not declare it.",
                                    s.label, d.label
                                ),
                                steps.clone(),
                            );
                        }
                    }
                    if scriptlet {
                        if path.starts_with('/')
                            && !paths::harmless(path)
                            && !paths::persistence(path)
                            && !self.declared(path)
                        {
                            add(
                                LawId::ScriptletUndeclaredFile,
                                e.dst,
                                format!(
                                    "{} changes {}{at} as the administrator, and the package does not declare that file.",
                                    s.label, d.label
                                ),
                                steps,
                            );
                        }
                    } else if s.stage != Stage::Upstream && paths::outside_package(path) {
                        add(
                            LawId::WriteOutsidePackage,
                            e.dst,
                            format!(
                                "{} writes {}{at}, outside the package and source folders.",
                                s.label, d.label
                            ),
                            steps,
                        );
                    }
                }
                E::Fetches | E::Sends => {
                    let (proc, peer) = if e.kind == E::Fetches {
                        (e.dst, s)
                    } else {
                        (e.src, d)
                    };
                    let p = &g.nodes[proc as usize];
                    if p.stage == Stage::Scriptlet && p.kind == K::Process {
                        let at = Self::at(e, p);
                        add(
                            LawId::PostinstallNetwork,
                            proc,
                            format!(
                                "{} contacts {}{at} while the package installs, as the administrator.",
                                p.label, peer.label
                            ),
                            vec![self.step(proc, e.loc.clone())],
                        );
                    }
                }
                _ => {}
            }
            if e.kind == E::Sends {
                let hit = self.back(
                    e.src,
                    |_| false,
                    |x, src| {
                        flows(x.kind)
                            && g.nodes[src as usize].kind == K::File
                            && paths::credential(&g.nodes[src as usize].key).is_some()
                    },
                );
                if let Some(mut p) = hit {
                    p.push(g.edges.iter().position(|x| std::ptr::eq(x, e)).unwrap_or(0) as u32);
                    let (s2, path) = self.narrate(&p);
                    let first = path.first().map(|x| x.label.clone()).unwrap_or_default();
                    add(
                        LawId::CredentialToNetwork,
                        e.dst,
                        format!("{first} is secret, and {s2}."),
                        path,
                    );
                }
            }
        }
    }

    pub(crate) fn check_sources(&self, out: &mut Vec<Finding>) {
        let m = self.meta;
        let bin = m.pkgnames.iter().any(|n| n.ends_with("-bin"));
        for s in &m.sources {
            let Some(url) = &s.url else { continue };
            let loc = Loc {
                file: "PKGBUILD".into(),
                line: s.line,
            };
            let step = Step {
                node: format!("url:{url}"),
                label: url.clone(),
                loc: Some(loc.clone()),
            };
            if !hosts::on_upstream(&m.url, url, &m.pkgnames) {
                let sentence = format!(
                    "{loc} downloads {url}, which is not on the project's declared home {}.",
                    m.url
                );
                out.push(finding(
                    LawId::SourceOffUpstream,
                    sentence,
                    vec![step.clone()],
                ));
            }
            let unsummed = s.sum.as_deref().is_none_or(|x| x.is_empty() || x == "SKIP");
            if bin && !s.vcs && unsummed {
                let sentence = format!(
                    "{loc} downloads the binary {url} with no checksum, so nothing ties it to what upstream published."
                );
                out.push(finding(LawId::BinChecksumUpstream, sentence, vec![step]));
            }
        }
    }

    /// One clause per hop, in plain words.
    fn narrate(&self, edges: &[u32]) -> (String, Vec<Step>) {
        let mut parts = Vec::new();
        let mut steps = Vec::new();
        for &i in edges {
            let e = &self.g.edges[i as usize];
            let (s, d) = (&self.g.nodes[e.src as usize], &self.g.nodes[e.dst as usize]);
            if steps.is_empty() {
                steps.push(self.step(e.src, s.loc.clone()));
            }
            steps.push(self.step(e.dst, e.loc.clone()));
            let at = e
                .loc
                .as_ref()
                .or(d.loc.as_ref())
                .map(|l| format!(" at {l}"))
                .unwrap_or_default();
            let hidden = |l: &str| l == "a pipe";
            match e.kind {
                E::Fetches => parts.push(format!("{} downloads {}{at}", d.label, s.label)),
                E::Writes if d.kind == K::File && !hidden(&d.label) => {
                    parts.push(format!("{} saves it as {}{at}", s.label, d.label))
                }
                E::Reads if d.kind == K::Process => parts.push(format!("{} reads it{at}", d.label)),
                E::Decodes => parts.push(format!("{} decodes it{at}", d.label)),
                E::Executes => parts.push(format!("{} runs it{at}", d.label)),
                E::Sends => parts.push(format!("{} sends it to {}{at}", s.label, d.label)),
                _ => {}
            }
        }
        (parts.join(", then "), steps)
    }
}

pub(crate) fn check(v: &View) -> Vec<Finding> {
    let mut out = Vec::new();
    let g = v.g;
    let mut explained: HashSet<NodeId> = HashSet::new();
    let mut seen: HashSet<(LawId, NodeId)> = HashSet::new();
    for (i, e) in g.edges.iter().enumerate() {
        if e.kind != E::Executes {
            continue;
        }
        let edge = i as u32;
        let sink = e.dst;
        let hit = |p: Vec<u32>| {
            let mut p = p;
            p.push(edge);
            p
        };
        // The executed thing may itself be the network, as with `nc -e`.
        let direct = network(g, e.src).then(|| vec![edge]);
        let path = direct.or_else(|| {
            v.back(
                e.src,
                |n| v.checked_before(n, e.seq) || v.declared_vcs(n),
                |x, src| network(g, src) && x.kind == E::Fetches,
            )
            .map(hit)
        });
        let blocked = v.checked_before(e.src, e.seq) || v.declared_vcs(e.src);
        if let (Some(p), false) = (path, blocked) {
            explained.insert(sink);
            if seen.insert((LawId::FetchExecuteUnverified, sink)) {
                let (s, path) = v.narrate(&p);
                out.push(finding(
                    LawId::FetchExecuteUnverified,
                    format!("{s}, and no checksum checks it first."),
                    path,
                ));
            }
        }
        if let Some(p) = v
            .back(e.src, |_| false, |x, _| x.kind == E::Decodes)
            .map(hit)
        {
            explained.insert(sink);
            if seen.insert((LawId::DecodeExecute, sink)) {
                let (s, path) = v.narrate(&p);
                out.push(finding(
                    LawId::DecodeExecute,
                    format!("{s}, so decoded data is run as code."),
                    path,
                ));
            }
        }
    }
    v.check_files(&mut out);
    for (i, n) in g.nodes.iter().enumerate() {
        if n.kind == K::Process && n.unresolved && !explained.contains(&(i as NodeId)) {
            let at = n
                .loc
                .as_ref()
                .map(|l| l.to_string())
                .unwrap_or_else(|| "an unknown place".into());
            let sentence = format!(
                "{} at {at} runs text this analysis cannot resolve, so what it does is unknown.",
                n.label
            );
            out.push(finding(
                LawId::UnresolvedDynamic,
                sentence,
                vec![v.step(i as NodeId, n.loc.clone())],
            ));
        }
    }
    out
}

fn step_of(g: &Graph, n: NodeId) -> Step {
    let node = &g.nodes[n as usize];
    Step {
        node: node.key.clone(),
        label: node.label.clone(),
        loc: node.loc.clone(),
    }
}

/// What the run did that the package's text never said. Absence from the static graph is the signal.
pub(crate) fn mismatch(stat: &Graph, obs: &Graph) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let says_network = stat
        .edges
        .iter()
        .any(|e| e.kind == E::Fetches && stat.nodes[e.dst as usize].kind == K::Process);
    for e in &obs.edges {
        let (s, d) = (&obs.nodes[e.src as usize], &obs.nodes[e.dst as usize]);
        let at = e
            .loc
            .as_ref()
            .map(|l| format!(" at {l}"))
            .unwrap_or_default();
        let hit = match e.kind {
            E::Reads
                if s.kind == K::File
                    && paths::credential(&s.key).is_some()
                    && stat.find(&s.key).is_none() =>
            {
                Some((
                    e.src,
                    format!(
                        "{} read {}{at}, which nothing in the package's files says it does.",
                        d.label, s.label
                    ),
                ))
            }
            E::Writes
                if d.kind == K::File
                    && (paths::outside_package(&d.key) || paths::persistence(&d.key))
                    && stat.find(&d.key).is_none() =>
            {
                Some((
                    e.dst,
                    format!(
                        "{} wrote {}{at}, which nothing in the package's files says it does.",
                        s.label, d.label
                    ),
                ))
            }
            E::Sends if d.kind == K::Socket && s.stage != Stage::Prepare && !says_network => {
                Some((
                    e.dst,
                    format!(
                        "{} tried to reach {}{at}, and nothing in the package's files says it uses the network.",
                        s.label, d.label
                    ),
                ))
            }
            _ => None,
        };
        if let Some((n, sentence)) = hit {
            if seen.insert(n) {
                let mut f = finding(LawId::ObservedMismatch, sentence, vec![step_of(obs, n)]);
                f.origin = Origin::Observed;
                out.push(f);
            }
        }
    }
    out
}

/// The version out of a key, and a VCS fragment with it, so a bump is not a change.
fn plain(key: &str, ver: &str) -> String {
    let k = key.split('#').next().unwrap_or(key);
    if ver.is_empty() {
        k.to_owned()
    } else {
        k.replace(ver, "$pkgver")
    }
}

/// Edges that matter when they appear between versions, and checks that vanish.
pub(crate) fn diff(new: &Graph, old: &Graph, explained: &[String]) -> Vec<Finding> {
    let edge_key = |g: &Graph, e: &Edge| {
        (
            e.kind,
            plain(&g.nodes[e.src as usize].key, &g.pkgver),
            plain(&g.nodes[e.dst as usize].key, &g.pkgver),
        )
    };
    let before: HashSet<_> = old.edges.iter().map(|e| edge_key(old, e)).collect();
    let after: HashSet<_> = new.edges.iter().map(|e| edge_key(new, e)).collect();
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for e in &new.edges {
        let (s, d) = (&new.nodes[e.src as usize], &new.nodes[e.dst as usize]);
        let scriptlet = s.stage == Stage::Scriptlet || d.stage == Stage::Scriptlet;
        let wanted = matches!(e.kind, E::Fetches | E::Executes | E::Decodes | E::Sends)
            || (scriptlet && matches!(e.kind, E::Reads | E::Writes));
        if !wanted || before.contains(&edge_key(new, e)) {
            continue;
        }
        // Name the thing, not the process: one new URL is one question, however many edges it adds.
        let (anchor, other) = if s.kind == K::Process { (d, s) } else { (s, d) };
        let key = plain(&anchor.key, &new.pkgver);
        if explained.iter().any(|x| *x == key || *x == anchor.key) || !seen.insert(key) {
            continue;
        }
        let at = e
            .loc
            .as_ref()
            .map(|l| format!(" at {l}"))
            .unwrap_or_default();
        let sentence = format!(
            "{} now touches {}{at}, which the accepted version did not, and nobody has explained why.",
            other.label, anchor.label
        );
        out.push(finding(
            LawId::UnexplainedNewEdge,
            sentence,
            vec![step_of(new, e.src), step_of(new, e.dst)],
        ));
    }
    for e in old.edges.iter().filter(|e| e.kind == E::Verifies) {
        let k = edge_key(old, e);
        if !after.contains(&k) && !explained.contains(&k.2) && seen.insert(k.2.clone()) {
            let d = &old.nodes[e.dst as usize];
            let sentence = format!(
                "{} had a checksum in the accepted version and has none now, and nobody has explained why.",
                d.label
            );
            out.push(finding(
                LawId::UnexplainedNewEdge,
                sentence,
                vec![step_of(old, e.dst)],
            ));
        }
    }
    out
}

/// A gap in what was read, as a finding: what was not seen must not read as clean.
pub(crate) fn incomplete(what: String) -> Finding {
    finding(LawId::IncompleteEvidence, what, Vec::new())
}
