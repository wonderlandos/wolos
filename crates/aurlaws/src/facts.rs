// SPDX-License-Identifier: GPL-2.0-or-later
//! A report's graph as a term of `laws/rules.bend`. The path and host tables stay here; what is left
//! for Bend is the shape of the graph. The difference test runs both checkers over the same terms.
use crate::graph::{EdgeKind as E, NodeKind as K, Stage};
use crate::laws::View;
use crate::{Report, hosts, paths};
use std::fmt::Write;

/// In the order of `type Law` in `laws/rules.bend`.
pub const LAW_NAMES: [&str; 14] = [
    "FetchExecuteUnverified",
    "DecodeExecute",
    "UnresolvedDynamic",
    "WriteOutsidePackage",
    "ReadCredentials",
    "CredentialToNetwork",
    "ScriptletUndeclaredFile",
    "SourceOffUpstream",
    "UndeclaredPersistence",
    "PostinstallNetwork",
    "BinChecksumUpstream",
    "UnexplainedNewEdge",
    "ObservedMismatch",
    "IncompleteEvidence",
];

fn hop(k: E) -> Option<&'static str> {
    Some(match k {
        E::Fetches => "R.Fetches{}",
        E::Writes => "R.Writes{}",
        E::Reads => "R.Reads{}",
        E::Executes => "R.Executes{}",
        E::Decodes => "R.Decodes{}",
        E::Sends => "R.Sends{}",
        E::Verifies => "R.Verifies{}",
        E::Calls => return None,
    })
}

/// A `Graph` term for the report, with its static facts.
pub fn term(r: &Report, package_files: &[String]) -> String {
    let v = View::new(&r.graph, &r.meta, package_files);
    let mut tagged = String::new();
    let tags: Vec<Vec<&str>> = (0..r.graph.nodes.len() as u32)
        .map(|i| {
            let n = &r.graph.nodes[i as usize];
            let mut t = Vec::new();
            if matches!(n.kind, K::Url | K::Socket) {
                t.push("R.Net{}");
            }
            if n.kind == K::File {
                if paths::credential(&n.key).is_some() {
                    t.push("R.Secret{}");
                }
                if paths::outside_package(&n.key) {
                    t.push("R.Outside{}");
                }
                if paths::persistence(&n.key) {
                    t.push("R.Autostart{}");
                }
                if n.key.starts_with("$pkgdir") {
                    t.push("R.Packaged{}");
                }
                if n.key.starts_with('/')
                    && !paths::harmless(&n.key)
                    && !paths::persistence(&n.key)
                    && !v.declared(&n.key)
                {
                    t.push("R.Foreign{}");
                }
            }
            if v.declared_vcs(i) {
                t.push("R.Tracking{}");
            }
            if n.kind == K::Process {
                if n.unresolved {
                    t.push("R.Dynamic{}");
                }
                match n.stage {
                    Stage::Scriptlet => t.push("R.Scriptlet{}"),
                    Stage::Upstream => t.push("R.Upstream{}"),
                    _ => {}
                }
                if matches!(n.label.as_str(), "chmod" | "chown" | "chgrp") {
                    t.push("R.Perms{}");
                }
                if matches!(n.label.as_str(), "setcap" | "chattr") {
                    t.push("R.Caps{}");
                }
                if v.from_sources(i) {
                    t.push("R.Copies{}");
                }
            }
            t
        })
        .collect();
    let node = |i: u32| format!("R.Node{{{i}, [{}]}}", tags[i as usize].join(", "));
    // Only nodes the edges do not carry and the node laws look at need listing.
    let mut extra: Vec<String> = Vec::new();
    for (i, t) in tags.iter().enumerate() {
        if t.contains(&"R.Dynamic{}") {
            extra.push(node(i as u32));
        }
    }
    let base = r.graph.nodes.len() as u32;
    let m = &r.meta;
    let bin = m.pkgnames.iter().any(|n| n.ends_with("-bin"));
    for (k, s) in m.sources.iter().enumerate() {
        let Some(url) = &s.url else { continue };
        let summed = s
            .sum
            .as_deref()
            .is_some_and(|x| !x.is_empty() && x != "SKIP");
        let mut t = Vec::new();
        if !hosts::on_upstream(&m.url, url, &m.pkgnames, s.pinned || summed) {
            t.push("R.OffHome{}");
        }
        if bin && !s.vcs && !summed {
            t.push("R.NoSum{}");
        }
        if !t.is_empty() {
            extra.push(format!("R.Node{{{}, [{}]}}", base + k as u32, t.join(", ")));
        }
    }
    if r.truncated {
        extra.push(format!(
            "R.Node{{{}, [R.Gap{{}}]}}",
            base + m.sources.len() as u32
        ));
    }
    let _ = write!(tagged, "R.Graph{{[{}], [", extra.join(", "));
    let mut first = true;
    for e in &r.graph.edges {
        let Some(h) = hop(e.kind) else { continue };
        // The file laws speak of a process writing a file; other writes only carry data along.
        if !first {
            tagged.push_str(", ");
        }
        first = false;
        let _ = write!(
            tagged,
            "R.Edge{{{}, {}, {h}, {}}}",
            node(e.src),
            node(e.dst),
            e.seq
        );
    }
    let _ = write!(
        tagged,
        "], {}}}",
        if package_files.is_empty() {
            "False{}"
        } else {
            "True{}"
        }
    );
    tagged
}
