// SPDX-License-Identifier: GPL-2.0-or-later
//! Graphs of what an AUR package says it does and what it did, and the laws over them (ADR 0022).
//! A broken law means "needs review". No broken law means only that: never "safe".
pub mod graph;
pub mod hosts;
pub mod langs;
pub mod laws;
pub mod pkgbuild;
pub mod shell;

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

fn judge(p: pkgbuild::Parsed) -> Report {
    let findings = {
        let v = laws::View::new(&p.graph, &p.meta);
        laws::check(&v)
    };
    let verdict = if findings.is_empty() { Verdict::NoLawBroken } else { Verdict::NeedsReview };
    Report { verdict, findings, notes: p.notes, truncated: p.truncated, disclaimer: DISCLAIMER.to_owned(), graph: p.graph, meta: p.meta }
}

/// Judge a PKGBUILD held in memory, with its `.install` text as `demo.install`.
pub fn audit_text(pkgbuild: &str, install: Option<&str>) -> Report {
    let mut files = HashMap::new();
    if let Some(i) = install {
        files.insert("demo.install".to_owned(), i.to_owned());
    }
    judge(pkgbuild::parse(pkgbuild, &files))
}
