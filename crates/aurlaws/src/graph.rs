// SPDX-License-Identifier: GPL-2.0-or-later
//! One graph shape for what a package says it does (static) and what it did (observed).
//! Edges point the way data moves, so a law is a path question: a `reads` edge runs file -> process,
//! a `writes` edge process -> file, an `executes` edge content -> the process that ran it.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type NodeId = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Source,
    Url,
    File,
    Env,
    Command,
    Function,
    Process,
    Socket,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Fetches,
    Writes,
    Reads,
    Executes,
    Decodes,
    Sends,
    /// A checksum, signature or commit id vouches for the destination.
    Verifies,
    /// Structure, not data: a function contains a process or calls a function.
    Calls,
}

/// When a node's code runs. `Scriptlet` runs as root on the person's machine.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Top level of a PKGBUILD: runs whenever makepkg reads the file.
    Parse,
    Prepare,
    #[default]
    Build,
    Package,
    Scriptlet,
    /// Build scripts inside the upstream source.
    Upstream,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Loc {
    pub file: String,
    pub line: u32,
}

impl std::fmt::Display for Loc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.file, self.line)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    pub kind: NodeKind,
    /// Stable across versions: `$pkgver` stays symbolic, so a graph diff shows real changes.
    pub key: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loc: Option<Loc>,
    #[serde(default)]
    pub stage: Stage,
    /// Could not be resolved statically: a dynamic command, eval or path.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unresolved: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub src: NodeId,
    pub dst: NodeId,
    pub kind: EdgeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loc: Option<Loc>,
    /// Visit order, so a check after the fact does not count as a check before it.
    #[serde(default)]
    pub seq: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    #[serde(skip)]
    index: HashMap<String, NodeId>,
    #[serde(skip)]
    seq: u32,
}

impl Graph {
    pub fn new() -> Self {
        Self::default()
    }

    /// The node for `key`, created on first sight. The first location wins.
    pub fn node(
        &mut self,
        kind: NodeKind,
        key: &str,
        label: &str,
        loc: Option<Loc>,
        stage: Stage,
    ) -> NodeId {
        if let Some(&id) = self.index.get(key) {
            return id;
        }
        let id = self.nodes.len() as NodeId;
        self.nodes.push(Node {
            kind,
            key: key.to_owned(),
            label: label.to_owned(),
            loc,
            stage,
            unresolved: false,
        });
        self.index.insert(key.to_owned(), id);
        id
    }

    pub fn find(&self, key: &str) -> Option<NodeId> {
        self.index.get(key).copied()
    }

    pub fn mark_unresolved(&mut self, id: NodeId) {
        self.nodes[id as usize].unresolved = true;
    }

    pub fn edge(&mut self, src: NodeId, dst: NodeId, kind: EdgeKind, loc: Option<Loc>) {
        self.seq += 1;
        let dup = self
            .edges
            .iter()
            .rev()
            .take(64)
            .any(|e| e.src == src && e.dst == dst && e.kind == kind);
        if !dup {
            self.edges.push(Edge {
                src,
                dst,
                kind,
                loc,
                seq: self.seq,
            });
        }
    }

    /// Add every node and edge of `other`, joining nodes by key.
    pub fn merge(&mut self, other: &Graph) {
        let map: Vec<NodeId> = other
            .nodes
            .iter()
            .map(|n| {
                let id = self.node(n.kind, &n.key, &n.label, n.loc.clone(), n.stage);
                if n.unresolved {
                    self.mark_unresolved(id);
                }
                id
            })
            .collect();
        for e in &other.edges {
            self.edge(
                map[e.src as usize],
                map[e.dst as usize],
                e.kind,
                e.loc.clone(),
            );
        }
    }

    /// Rebuild the key index after deserialising.
    pub fn reindex(&mut self) {
        self.index = self
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.key.clone(), i as NodeId))
            .collect();
        self.seq = self.edges.iter().map(|e| e.seq).max().unwrap_or(0);
    }

    /// Outgoing edge lists, for path searches.
    pub fn out(&self) -> Vec<Vec<u32>> {
        let mut out = vec![Vec::new(); self.nodes.len()];
        for (i, e) in self.edges.iter().enumerate() {
            out[e.src as usize].push(i as u32);
        }
        out
    }
}
