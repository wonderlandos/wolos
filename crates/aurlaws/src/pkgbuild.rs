// SPDX-License-Identifier: GPL-2.0-or-later
//! A PKGBUILD and its neighbours as one static graph. makepkg's phases decide each stage, and the
//! top level counts as `Parse`, because makepkg runs it just by reading the file.
use crate::graph::{Graph, Stage};
use crate::shell::{Note, Scope, SourceEntry, Walker, is_url};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// What the package declares about itself.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    pub pkgnames: Vec<String>,
    pub pkgver: String,
    pub url: String,
    pub install: Vec<String>,
    pub backup: Vec<String>,
    pub sources: Vec<SourceEntry>,
}

pub struct Parsed {
    pub graph: Graph,
    pub meta: Meta,
    pub notes: Vec<Note>,
    pub truncated: bool,
}

const SUMS: &[&str] = &[
    "sha512sums",
    "sha384sums",
    "sha256sums",
    "sha224sums",
    "b2sums",
    "sha1sums",
    "md5sums",
    "cksums",
];
const SCRIPTLETS: &[&str] = &[
    "pre_install",
    "post_install",
    "pre_upgrade",
    "post_upgrade",
    "pre_remove",
    "post_remove",
];

/// `name::url#fragment` as makepkg reads it.
pub fn source_entry(raw: &str, sum: Option<String>, line: u32) -> SourceEntry {
    let (named, rest) = match raw.split_once("::") {
        Some((n, r)) if !n.contains('/') => (Some(n.to_owned()), r),
        _ => (None, raw),
    };
    if !is_url(rest) {
        return SourceEntry {
            name: named.unwrap_or_else(|| rest.to_owned()),
            url: None,
            sum,
            vcs: false,
            pinned: false,
            line,
        };
    }
    let scheme = rest.split("://").next().unwrap_or("");
    let vcs = ["git", "hg", "svn", "bzr", "fossil"]
        .iter()
        .any(|v| scheme == *v || scheme.starts_with(&format!("{v}+")));
    let (no_frag, frag) = rest.split_once('#').unwrap_or((rest, ""));
    let no_query = no_frag.split('?').next().unwrap_or(no_frag);
    let tail = no_query
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("");
    let name = named.unwrap_or_else(|| {
        if vcs {
            tail.trim_end_matches(".git").to_owned()
        } else {
            tail.to_owned()
        }
    });
    let pin = frag.split_once('=').is_some_and(|(k, v)| {
        let commit = k == "commit" && v.len() >= 7 && v.bytes().all(|b| b.is_ascii_hexdigit());
        commit || k == "tag" || k == "revision"
    });
    SourceEntry {
        name,
        url: Some(rest.to_owned()),
        sum,
        vcs,
        pinned: vcs && pin,
        line,
    }
}

fn sources_of(w: &Walker, suffix: &str) -> Vec<SourceEntry> {
    let name = format!("source{suffix}");
    let raw = w.var_values(&name);
    let sums = SUMS
        .iter()
        .map(|s| w.var_values(&format!("{s}{suffix}")))
        .find(|v| !v.is_empty())
        .unwrap_or_default();
    let line = w.var_line(&name);
    raw.iter()
        .enumerate()
        .map(|(i, r)| source_entry(r, sums.get(i).cloned(), line))
        .collect()
}

/// Build the static graph. `files` holds the text files beside the PKGBUILD.
pub fn parse(pkgbuild: &str, files: &HashMap<String, String>) -> Parsed {
    let mut graph = Graph::new();
    let mut w = Walker::new(
        &mut graph,
        Scope {
            helpers: files.clone(),
            sources: Vec::new(),
        },
    );
    w.walk_file(pkgbuild, "PKGBUILD", Stage::Parse);
    let mut sources = sources_of(&w, "");
    for arch in ["_x86_64", "_any"] {
        sources.extend(sources_of(&w, arch));
    }
    w.declare_sources(sources.clone(), "PKGBUILD");
    let names = w.function_names();
    for (f, stage) in [
        ("prepare", Stage::Prepare),
        ("pkgver", Stage::Prepare),
        ("build", Stage::Build),
        ("check", Stage::Build),
        ("package", Stage::Package),
    ] {
        w.walk_function(f, stage, "$srcdir");
    }
    for n in names.iter().filter(|n| n.starts_with("package_")) {
        w.walk_function(n, Stage::Package, "$srcdir");
    }
    w.walk_uncalled(Stage::Build);
    let mut install = w.var_values("install");
    install.retain(|i| !i.is_empty());
    for n in names.iter().filter(|n| n.starts_with("package_")) {
        let _ = n; // package_* can set install= inside; the scan below covers those files.
    }
    for name in files.keys() {
        if name.ends_with(".install") && !install.contains(name) {
            install.push(name.clone());
        }
    }
    install.sort();
    let mut truncated = false;
    let meta = Meta {
        pkgnames: w.var_values("pkgname"),
        pkgver: w.var_values("pkgver").first().cloned().unwrap_or_default(),
        url: w.var_values("url").first().cloned().unwrap_or_default(),
        install: install.clone(),
        backup: w.var_values("backup"),
        sources,
    };
    let mut notes = std::mem::take(&mut w.notes);
    truncated |= w.truncated;
    drop(w);
    for name in &install {
        let Some(text) = files.get(name) else {
            continue;
        };
        let mut sub = Graph::new();
        let mut s = Walker::new(&mut sub, Scope::default());
        s.walk_file(text, name, Stage::Scriptlet);
        for f in SCRIPTLETS {
            s.walk_function(f, Stage::Scriptlet, "/");
        }
        s.walk_uncalled(Stage::Scriptlet);
        notes.append(&mut s.notes);
        truncated |= s.truncated;
        drop(s);
        graph.merge(&sub);
    }
    Parsed {
        graph,
        meta,
        notes,
        truncated,
    }
}
