// SPDX-License-Identifier: GPL-2.0-or-later
//! The laws of `laws/LAWS.bend`, run over a graph. Each answers with the exact path it found.
use crate::graph::{Edge, EdgeKind as E, Graph, Loc, NodeId, NodeKind as K};
use crate::pkgbuild::Meta;
use serde::Serialize;
use std::collections::{HashSet, VecDeque};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LawId {
    FetchExecuteUnverified,
    DecodeExecute,
    UnresolvedDynamic,
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
}

/// Edges content can move along.
fn flows(k: E) -> bool {
    matches!(k, E::Fetches | E::Writes | E::Reads | E::Decodes | E::Executes)
}

fn network(g: &Graph, n: NodeId) -> bool {
    matches!(g.nodes[n as usize].kind, K::Url | K::Socket)
}

pub(crate) struct View<'a> {
    pub g: &'a Graph,
    pub meta: &'a Meta,
    into: Vec<Vec<u32>>,
}

impl<'a> View<'a> {
    pub fn new(g: &'a Graph, meta: &'a Meta) -> Self {
        let mut into = vec![Vec::new(); g.nodes.len()];
        for (i, e) in g.edges.iter().enumerate() {
            into[e.dst as usize].push(i as u32);
        }
        View { g, meta, into }
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
            && self.meta.sources.iter().any(|s| s.vcs && s.name == node.label && s.url.as_deref().is_some_and(|u| crate::hosts::same_project(&self.meta.url, u, &self.meta.pkgnames)))
    }

    /// Walk backwards from `start` to the first edge or node `goal` accepts, without crossing a
    /// node `stop` rejects. The result reads source first.
    fn back(&self, start: NodeId, stop: impl Fn(NodeId) -> bool, goal: impl Fn(&Edge, NodeId) -> bool) -> Option<Vec<u32>> {
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
        Step { node: node.key.clone(), label: node.label.clone(), loc: loc.or_else(|| node.loc.clone()) }
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
            let at = e.loc.as_ref().or(d.loc.as_ref()).map(|l| format!(" at {l}")).unwrap_or_default();
            let hidden = |l: &str| l == "a pipe";
            match e.kind {
                E::Fetches => parts.push(format!("{} downloads {}{at}", d.label, s.label)),
                E::Writes if d.kind == K::File && !hidden(&d.label) => parts.push(format!("{} saves it as {}{at}", s.label, d.label)),
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
            v.back(e.src, |n| v.checked_before(n, e.seq) || v.declared_vcs(n), |x, src| network(g, src) && x.kind == E::Fetches).map(hit)
        });
        let blocked = v.checked_before(e.src, e.seq) || v.declared_vcs(e.src);
        if let (Some(p), false) = (path, blocked) {
            explained.insert(sink);
            if seen.insert((LawId::FetchExecuteUnverified, sink)) {
                let (s, path) = v.narrate(&p);
                out.push(Finding { law: LawId::FetchExecuteUnverified, sentence: format!("{s}, and no checksum checks it first."), path });
            }
        }
        if let Some(p) = v.back(e.src, |_| false, |x, _| x.kind == E::Decodes).map(hit) {
            explained.insert(sink);
            if seen.insert((LawId::DecodeExecute, sink)) {
                let (s, path) = v.narrate(&p);
                out.push(Finding { law: LawId::DecodeExecute, sentence: format!("{s}, so decoded data is run as code."), path });
            }
        }
    }
    for (i, n) in g.nodes.iter().enumerate() {
        if n.kind == K::Process && n.unresolved && !explained.contains(&(i as NodeId)) {
            let at = n.loc.as_ref().map(|l| l.to_string()).unwrap_or_else(|| "an unknown place".into());
            let sentence = format!("{} at {at} runs text this analysis cannot resolve, so what it does is unknown.", n.label);
            out.push(Finding { law: LawId::UnresolvedDynamic, sentence, path: vec![v.step(i as NodeId, n.loc.clone())] });
        }
    }
    out
}
