// SPDX-License-Identifier: GPL-2.0-or-later
//! Graphs of what an AUR package says it does and what it did, and the laws over them (ADR 0022).
//! A broken law means "needs review". No broken law means only that: never "safe".
pub mod graph;
pub mod hosts;
pub mod langs;
pub mod laws;
pub mod observed;
pub mod paths;
pub mod pkgbuild;
pub mod render;
pub mod shell;
pub mod upstream;

pub use laws::{Finding, LawId, Step};
use serde::Serialize;
use std::collections::HashMap;

pub const DISCLAIMER: &str = "A broken law means this package needs review. No broken law means only that: it does not mean the package is safe. Code can hide logic that no graph resolves, so the sandbox stays the main defence.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    NeedsReview,
    NoLawBroken,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub verdict: Verdict,
    pub findings: Vec<Finding>,
    pub notes: Vec<shell::Note>,
    /// Limits were hit, so part of the input was not read.
    pub truncated: bool,
    pub disclaimer: String,
    pub graph: graph::Graph,
    pub meta: pkgbuild::Meta,
}

fn judge(mut p: pkgbuild::Parsed, ev: &Evidence) -> Report {
    for g in &ev.upstream {
        p.graph.merge(g);
    }
    let mut findings = {
        let v = laws::View::new(&p.graph, &p.meta, &ev.package_files);
        let mut f = laws::check(&v);
        v.check_sources(&mut f);
        f
    };
    for obs in &ev.observed {
        let v = laws::View::new(obs, &p.meta, &ev.package_files);
        findings.extend(laws::check(&v).into_iter().map(|mut f| {
            f.origin = laws::Origin::Observed;
            f
        }));
        findings.extend(laws::mismatch(&p.graph, obs));
    }
    if let Some(list) = &ev.published {
        findings.extend(laws::published(&p.meta, list));
    }
    if let Some(old) = &ev.previous {
        findings.extend(laws::diff(&p.graph, old, &ev.explained));
    }
    if p.truncated {
        findings.push(laws::incomplete(
            "The package's files went past the analyser's limits, so part of them was not read."
                .to_owned(),
        ));
    }
    findings.extend(ev.gaps.iter().cloned().map(laws::incomplete));
    let verdict = if findings.is_empty() {
        Verdict::NoLawBroken
    } else {
        Verdict::NeedsReview
    };
    Report {
        verdict,
        findings,
        notes: p.notes,
        truncated: p.truncated,
        disclaimer: DISCLAIMER.to_owned(),
        graph: p.graph,
        meta: p.meta,
    }
}

/// What is known beyond the files: traced runs, and the version accepted before.
#[derive(Clone, Debug, Default)]
pub struct Evidence {
    pub observed: Vec<graph::Graph>,
    pub previous: Option<graph::Graph>,
    /// Node keys a person has explained, so their new edges stop being findings.
    pub explained: Vec<String>,
    /// What the built package holds, as installed paths.
    pub package_files: Vec<String>,
    /// Evidence that should exist and does not, in plain sentences.
    pub gaps: Vec<String>,
    /// Graphs of the upstream source's build scripts.
    pub upstream: Vec<graph::Graph>,
    /// Checksums upstream published, as (sum, file name). `None` means no list was found.
    pub published: Option<Vec<(String, String)>>,
}

/// `audit_text`, with what is known beyond the files.
pub fn audit_text_with(pkgbuild: &str, install: Option<&str>, ev: &Evidence) -> Report {
    let mut files = HashMap::new();
    if let Some(i) = install {
        files.insert("demo.install".to_owned(), i.to_owned());
    }
    judge(pkgbuild::parse(pkgbuild, &files), ev)
}

/// Judge a PKGBUILD and the text files beside it.
pub fn judge_files(pkgbuild: &str, files: &HashMap<String, String>) -> Report {
    judge_with(pkgbuild, files, &Evidence::default())
}

/// `judge_files`, with what is known beyond the files.
pub fn judge_with(pkgbuild: &str, files: &HashMap<String, String>, ev: &Evidence) -> Report {
    judge(pkgbuild::parse(pkgbuild, files), ev)
}

/// Judge a PKGBUILD held in memory, with its `.install` text as `demo.install`.
pub fn audit_text(pkgbuild: &str, install: Option<&str>) -> Report {
    audit_text_with(pkgbuild, install, &Evidence::default())
}
