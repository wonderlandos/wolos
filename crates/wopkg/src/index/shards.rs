// SPDX-License-Identifier: GPL-2.0-or-later
//! The curated shard index (wave 3 CR-4, docs/modules/shards.md, docs/RULES-crystal.md RCR-040 to
//! RCR-044): a signed list of the Crystal shards WolOS vouches for, each pinned to one full git
//! commit, and the edits `wol shard add|update` makes from it: the `dependencies:` of a project's
//! `shard.yml` pinned by `commit:`, and a `shard.lock` that `shards install --frozen --local`
//! accepts without the network. `wol shard check` reads the same two files against the same rules.
//!
//! Trust works as for the plugin index (index/plugins.rs): the Ed25519 signature is checked before
//! the text is parsed, a signed `serial` never goes back and a signed `expires` ends a list. The
//! keys are their own: `*.pub` files in [`KEYS_SUBDIR`] below the WolOS keys folder, so a shard key
//! never signs packages or plugins and the package keys never vouch for shards.
//!
//! `index.json`, format 1:
//! `{"format":1,"serial":N,"expires":T,"shards":[{"name","git","version","commit","license",
//! "summary","crystal","dependencies":[..]}]}`. An entry that breaks a rule is dropped on its own; a
//! second entry with the same name is dropped too. `dependencies` names other entries of the same
//! list: the lock carries the whole closure, so a curated shard only depends on curated shards.
use crate::ffi::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use wol_core::package::{Trust, verify_index};

/// Largest `index.json` read.
pub const MAX_INDEX: usize = 2 << 20;
/// Entries kept from one index.
pub const MAX_ENTRIES: usize = 2000;
/// Where the index lives (the `wolos-shards` repository's published files).
pub const DEFAULT_URL: &str = "https://shards.wolos.org/index.json";
/// The shard index keys: `<keys dir>/shards/*.pub` (and its own `revoked.list`).
pub const KEYS_SUBDIR: &str = "shards";
/// Below the wopkg cache: `index.json` and `index.json.sig`, kept only once verified.
pub const CACHE_SUBDIR: &str = "shard-index";
/// Largest `shard.yml` or `shard.lock` read.
pub const MAX_PROJECT_FILE: u64 = 256 << 10;
/// The lock format `shards` 0.19 writes and reads.
pub const LOCK_VERSION: &str = "2.0";
const MAX_URL: usize = 512;
const MAX_TEXT: usize = 200;
const MAX_NAME: usize = 50;
const MAX_DEPS: usize = 32;
const INDEX_FILE: &str = "index.json";
const SIG_FILE: &str = "index.json.sig";
const YML: &str = "shard.yml";
const LOCK: &str = "shard.lock";

/// One shard as the index lists it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Entry {
    /// The shard's name (its `shard.yml` `name:`), also the dependency key.
    pub name: String,
    /// `https://` clone URL.
    pub git: String,
    /// The `version:` of the shard at that commit.
    pub version: String,
    /// The vetted commit, 40 lowercase hex digits.
    pub commit: String,
    pub license: String,
    pub summary: String,
    /// The shard's `crystal:` requirement; empty = not stated.
    pub crystal: String,
    /// Names of the curated shards it depends on.
    pub dependencies: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ShardIndex {
    pub shards: Vec<Entry>,
    /// Signed publication number (0 in lists made without one).
    pub serial: u64,
    /// Signed end of validity, Unix seconds; 0 = none.
    pub expires: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    format: u32,
    #[serde(default)]
    serial: u64,
    #[serde(default)]
    expires: u64,
    #[serde(default)]
    shards: Vec<serde_json::Value>,
}

fn text_ok(s: &str, max: usize) -> bool {
    s.len() <= max && !s.chars().any(char::is_control)
}

fn hex40(s: &str) -> bool {
    s.len() == 40
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A shard name as `shards` accepts it, and safe as a YAML key: lowercase ASCII letter first, then
/// lowercase letters, digits, `-` and `_`.
pub fn is_shard_name(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= MAX_NAME
        && b[0].is_ascii_lowercase()
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-' || *c == b'_')
}

/// A shard version (`1.2.3`, `0.4.0-rc.1`): digit first, then letters, digits, `.` and `-`. No
/// `+`: the lock adds `+git.commit.<sha>` itself.
fn version_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.as_bytes()[0].is_ascii_digit()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
}

/// An `https://host/path` clone URL written unquoted into YAML: no user part, no query, no
/// fragment, no space or quote, nothing YAML reads as syntax.
fn git_ok(s: &str) -> bool {
    s.len() <= MAX_URL
        && s.strip_prefix("https://").is_some_and(|r| {
            let (host, path) = r.split_once('/').unwrap_or((r, ""));
            !host.is_empty()
                && host
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b':')
                && path.len() > 1
                && !path.split('/').any(|seg| seg == ".." || seg == ".")
                && path.bytes().all(|b| {
                    b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'-' | b'_' | b'~')
                })
        })
}

impl Entry {
    /// Every rule an entry must keep to be listed.
    pub fn is_valid(&self) -> bool {
        is_shard_name(&self.name)
            && git_ok(&self.git)
            && version_ok(&self.version)
            && hex40(&self.commit)
            && !self.license.trim().is_empty()
            && text_ok(&self.license, 64)
            && text_ok(&self.summary, MAX_TEXT)
            && text_ok(&self.crystal, 64)
            && self.dependencies.len() <= MAX_DEPS
            && self
                .dependencies
                .iter()
                .all(|d| is_shard_name(d) && *d != self.name)
    }

    /// `0.3.0+git.commit.<sha>`: how `shards` names a commit-pinned version in `shard.lock`.
    pub fn locked_version(&self) -> String {
        format!("{}+git.commit.{}", self.version, self.commit)
    }
}

/// Pure parse of `index.json` bytes (never panics). Unknown format is an error; bad entries are
/// dropped one by one.
pub fn parse(text: &[u8]) -> Result<ShardIndex> {
    if text.len() > MAX_INDEX {
        return Err(Error::untrusted("The shard list is too large."));
    }
    let raw: Raw =
        serde_json::from_slice(text).map_err(|_| Error::untrusted("The shard list is damaged."))?;
    if raw.format != 1 {
        return Err(Error::untrusted("The shard list is in an unknown format."));
    }
    let mut shards: Vec<Entry> = Vec::new();
    for v in raw.shards {
        if shards.len() >= MAX_ENTRIES {
            break;
        }
        let Ok(e) = serde_json::from_value::<Entry>(v) else {
            continue;
        };
        if e.is_valid() && !shards.iter().any(|s| s.name == e.name) {
            shards.push(e);
        }
    }
    Ok(ShardIndex {
        shards,
        serial: raw.serial,
        expires: raw.expires,
    })
}

/// The index only counts when signed by a shard index key: the signature is checked first.
pub fn verify(text: &[u8], sig: Option<&[u8]>, keys: &[[u8; 32]]) -> Result<ShardIndex> {
    match verify_index(text, sig, keys) {
        Ok(Trust::Signed(_)) => parse(text),
        _ => Err(Error::untrusted(
            "The shard list is not signed by the WolOS shard index.",
        )),
    }
}

impl ShardIndex {
    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.shards.iter().find(|e| e.name == name)
    }

    /// Case-insensitive match on name, summary and license; name hits first.
    pub fn search(&self, term: &str) -> Vec<&Entry> {
        let t = term.trim().to_lowercase();
        if t.is_empty() {
            return self.shards.iter().collect();
        }
        let mut hits: Vec<(u8, &Entry)> = self
            .shards
            .iter()
            .filter_map(|e| {
                let rank = if e.name.contains(&t) {
                    0
                } else if e.summary.to_lowercase().contains(&t)
                    || e.license.to_lowercase().contains(&t)
                {
                    1
                } else {
                    return None;
                };
                Some((rank, e))
            })
            .collect();
        hits.sort_by_key(|h| h.0);
        hits.into_iter().map(|h| h.1).collect()
    }

    /// `names` and every curated shard they need, sorted by name. A name the list does not carry
    /// is an error: the lock has to name the whole tree.
    pub fn closure(&self, names: &[String]) -> Result<Vec<&Entry>> {
        let mut seen: BTreeMap<&str, &Entry> = BTreeMap::new();
        let mut todo: Vec<(String, Option<String>)> =
            names.iter().map(|n| (n.clone(), None)).collect();
        while let Some((n, by)) = todo.pop() {
            if seen.contains_key(n.as_str()) {
                continue;
            }
            let Some(e) = self.get(&n) else {
                return Err(match by {
                    None => Error::new(
                        crate::ffi::Status::NotFound,
                        format!("{n} is not in the WolOS shard list."),
                    ),
                    Some(b) => Error::untrusted(format!(
                        "{b} needs {n}, which the WolOS shard list does not carry."
                    )),
                });
            };
            seen.insert(e.name.as_str(), e);
            todo.extend(
                e.dependencies
                    .iter()
                    .map(|d| (d.clone(), Some(e.name.clone()))),
            );
        }
        Ok(seen.into_values().collect())
    }
}

// ---- shard.yml and shard.lock ----

/// Which dependency list of a `shard.yml` an entry sits in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    Dependencies,
    DevelopmentDependencies,
}

impl Section {
    fn header(self) -> &'static str {
        match self {
            Section::Dependencies => "dependencies:",
            Section::DevelopmentDependencies => "development_dependencies:",
        }
    }
}

/// One dependency of a `shard.yml`: where it is and what it says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dep {
    pub name: String,
    pub section: Section,
    /// 1-based line of `  name:`.
    pub line: usize,
    pub keys: BTreeMap<String, String>,
    /// Lines `[start, end)` of the block, trailing blank lines left out.
    start: usize,
    end: usize,
}

fn indent(l: &str) -> usize {
    l.len() - l.trim_start().len()
}

/// Comment-free text of a line (` #` starts a comment, as the RCR scanner reads it).
fn code(l: &str) -> &str {
    l.split(" #").next().unwrap_or(l)
}

fn top_value(lines: &[&str], key: &str) -> Option<String> {
    lines.iter().find_map(|l| {
        let t = code(l);
        if indent(t) != 0 {
            return None;
        }
        let (k, v) = t.split_once(':')?;
        (k.trim() == key).then(|| v.trim().trim_matches('"').trim_matches('\'').to_owned())
    })
}

/// The dependencies of both lists, read line by line the way `docs/RULES-crystal.md` RCR-040
/// reads them (two-space names, four-space keys).
pub fn read_deps(yml: &str) -> Vec<Dep> {
    let lines: Vec<&str> = yml.lines().collect();
    let mut out: Vec<Dep> = Vec::new();
    let mut section: Option<Section> = None;
    for (i, raw) in lines.iter().enumerate() {
        let l = code(raw);
        if l.trim().is_empty() || l.trim_start().starts_with('#') {
            continue;
        }
        let n = indent(l);
        let t = l.trim();
        if n == 0 {
            section = [Section::Dependencies, Section::DevelopmentDependencies]
                .into_iter()
                .find(|s| t == s.header());
            continue;
        }
        let Some(s) = section else { continue };
        if n <= 2 && t.ends_with(':') {
            out.push(Dep {
                name: t.trim_end_matches(':').to_owned(),
                section: s,
                line: i + 1,
                keys: BTreeMap::new(),
                start: i,
                end: i + 1,
            });
        } else if let Some(d) = out.last_mut()
            && d.section == s
        {
            if let Some((k, v)) = t.split_once(':') {
                d.keys.insert(
                    k.trim().to_owned(),
                    v.trim().trim_matches('"').trim_matches('\'').to_owned(),
                );
            }
            d.end = i + 1;
        }
    }
    out
}

/// `name -> version` and `name -> git` of a `shard.lock`.
pub fn read_lock(text: &str) -> BTreeMap<String, (String, String)> {
    let mut m: BTreeMap<String, (String, String)> = BTreeMap::new();
    let mut cur: Option<String> = None;
    let mut in_shards = false;
    for raw in text.lines() {
        let n = indent(raw);
        let t = raw.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if n == 0 {
            in_shards = t == "shards:";
            cur = None;
        } else if in_shards && n == 2 && t.ends_with(':') {
            let name = t.trim_end_matches(':').to_owned();
            m.insert(name.clone(), Default::default());
            cur = Some(name);
        } else if in_shards
            && n >= 4
            && let Some(c) = &cur
            && let Some((k, v)) = t.split_once(':')
        {
            let v = v.trim().trim_matches('"').to_owned();
            let slot = m.entry(c.clone()).or_default();
            match k.trim() {
                "version" => slot.0 = v,
                "git" => slot.1 = v,
                _ => {}
            }
        }
    }
    m
}

/// The `shard.lock` for `entries` (sorted, as `shards` writes it).
pub fn render_lock(entries: &[&Entry]) -> String {
    let mut sorted: Vec<&&Entry> = entries.iter().collect();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    let mut out = format!("version: {LOCK_VERSION}\nshards:\n");
    for e in sorted {
        out.push_str(&format!(
            "  {}:\n    git: {}\n    version: {}\n\n",
            e.name,
            e.git,
            e.locked_version()
        ));
    }
    if out.ends_with("\n\n") {
        out.pop();
    }
    out
}

fn dep_block(e: &Entry) -> Vec<String> {
    vec![
        format!("  {}:", e.name),
        format!("    git: {}", e.git),
        format!("    commit: {}", e.commit),
    ]
}

/// `yml` with `e` pinned in `section`: its block replaced when it is there (other keys such as
/// `version:`, `branch:` or `github:` go), else appended to that list (made when missing).
pub fn pin_in(yml: &str, e: &Entry, section: Section) -> String {
    let mut lines: Vec<String> = yml.lines().map(str::to_owned).collect();
    let deps = read_deps(yml);
    if let Some(d) = deps.iter().find(|d| d.name == e.name) {
        lines.splice(d.start..d.end, dep_block(e));
    } else {
        let header = lines
            .iter()
            .position(|l| indent(code(l)) == 0 && code(l).trim() == section.header());
        // `dependencies: {}` (or any inline value) becomes a block list.
        let inline = lines.iter().position(|l| {
            let t = code(l);
            indent(t) == 0
                && t.trim()
                    .strip_prefix(section.header())
                    .is_some_and(|rest| !rest.trim().is_empty())
        });
        match (header, inline) {
            (Some(h), _) => {
                // after the last line of the list (the next top-level line ends it)
                let mut at = h + 1;
                for (i, l) in lines.iter().enumerate().skip(h + 1) {
                    let c = code(l);
                    if c.trim().is_empty() || c.trim_start().starts_with('#') {
                        continue;
                    }
                    if indent(c) == 0 {
                        break;
                    }
                    at = i + 1;
                }
                lines.splice(at..at, dep_block(e));
            }
            (None, Some(i)) => {
                lines[i] = section.header().to_owned();
                lines.splice(i + 1..i + 1, dep_block(e));
            }
            (None, None) => {
                if lines.last().is_some_and(|l| !l.trim().is_empty()) {
                    lines.push(String::new());
                }
                lines.push(section.header().to_owned());
                lines.extend(dep_block(e));
            }
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// What `add` or `update` changed: one row per dependency whose pin moved or appeared.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Change {
    pub name: String,
    /// The commit pinned before; empty when the dependency is new or was not pinned by commit.
    pub from: String,
    pub to: String,
    pub version: String,
}

/// One shard `shard.lock` names: what a cache for `shards install --frozen --local` must hold.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Locked {
    pub name: String,
    pub git: String,
    pub commit: String,
    pub version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Pinned {
    pub changed: Vec<Change>,
    /// Every shard `shard.lock` now names (the closure), sorted by name.
    pub locked: Vec<Locked>,
    pub shard_yml: PathBuf,
    pub shard_lock: PathBuf,
}

fn read_project(dir: &Path) -> Result<String> {
    let p = dir.join(YML);
    if std::fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(Error::invalid(format!("{} is a link.", p.display())));
    }
    let bytes = shared::bounded::read_file(&p, MAX_PROJECT_FILE).ok_or_else(|| {
        Error::new(
            crate::ffi::Status::NotFound,
            format!("No {YML} in {}.", dir.display()),
        )
    })?;
    String::from_utf8(bytes).map_err(|_| Error::invalid(format!("{} is not UTF-8.", p.display())))
}

/// The pure part of `add` (`update` false) and `update`: the new `shard.yml` and `shard.lock`
/// texts and what moved. `add` needs names and puts new ones in `dependencies:`; `update`
/// re-pins the named dependencies (all of both lists when `names` is empty) to the list's commit.
pub fn plan_pin(
    index: &ShardIndex,
    yml: &str,
    names: &[String],
    update: bool,
) -> Result<(String, String, Vec<Change>)> {
    plan_pin_full(index, yml, names, update).map(|(y, l, c, _)| (y, l, c))
}

fn plan_pin_full(
    index: &ShardIndex,
    yml: &str,
    names: &[String],
    update: bool,
) -> Result<(String, String, Vec<Change>, Vec<Locked>)> {
    if !update && names.is_empty() {
        return Err(Error::invalid("Name the shard to add."));
    }
    for n in names {
        if !is_shard_name(n) {
            return Err(Error::invalid(format!("{n} is not a shard name.")));
        }
    }
    let before = read_deps(yml);
    let targets: Vec<String> = if update && names.is_empty() {
        before.iter().map(|d| d.name.clone()).collect()
    } else {
        names.to_vec()
    };
    let mut text = yml.to_owned();
    let mut changed = Vec::new();
    for n in &targets {
        let old = before.iter().find(|d| d.name == *n);
        if update && old.is_none() {
            return Err(Error::new(
                crate::ffi::Status::NotFound,
                format!("{n} is not a dependency in {YML}; add it first."),
            ));
        }
        let e = index.get(n).ok_or_else(|| {
            Error::new(
                crate::ffi::Status::NotFound,
                format!("{n} is not in the WolOS shard list."),
            )
        })?;
        let section = old.map_or(Section::Dependencies, |d| d.section);
        let from = old
            .and_then(|d| d.keys.get("commit"))
            .cloned()
            .unwrap_or_default();
        let same = old.is_some_and(|d| {
            d.keys.len() == 2
                && d.keys.get("git") == Some(&e.git)
                && d.keys.get("commit") == Some(&e.commit)
        });
        if !same {
            text = pin_in(&text, e, section);
            changed.push(Change {
                name: e.name.clone(),
                from,
                to: e.commit.clone(),
                version: e.version.clone(),
            });
        }
    }
    let all: Vec<String> = read_deps(&text).into_iter().map(|d| d.name).collect();
    let closure = index.closure(&all)?;
    let locked = closure
        .iter()
        .map(|e| Locked {
            name: e.name.clone(),
            git: e.git.clone(),
            commit: e.commit.clone(),
            version: e.version.clone(),
        })
        .collect();
    Ok((text, render_lock(&closure), changed, locked))
}

/// `add` / `update` on the project in `dir`: [`plan_pin`], then both files written whole (temp
/// file, then rename). Nothing is written when the plan fails.
pub fn pin(index: &ShardIndex, dir: &Path, names: &[String], update: bool) -> Result<Pinned> {
    let yml = read_project(dir)?;
    let (text, lock, changed, locked) = plan_pin_full(index, &yml, names, update)?;
    let cant = |p: &Path| Error::unavailable(format!("{} cannot be written.", p.display()));
    let (yp, lp) = (dir.join(YML), dir.join(LOCK));
    if text != yml {
        shared::atomic::atomic_write(&yp, 0o644, text.as_bytes()).map_err(|_| cant(&yp))?;
    }
    shared::atomic::atomic_write(&lp, 0o644, lock.as_bytes()).map_err(|_| cant(&lp))?;
    Ok(Pinned {
        changed,
        locked,
        shard_yml: yp,
        shard_lock: lp,
    })
}

/// One problem `check` found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Finding {
    /// `RCR-040`, `RCR-041` or `index`.
    pub rule: &'static str,
    /// 1-based line in `file`; 1 when the file as a whole is meant.
    pub line: usize,
    pub file: &'static str,
    pub message: String,
}

fn finding(rule: &'static str, file: &'static str, line: usize, m: impl Into<String>) -> Finding {
    Finding {
        rule,
        line,
        file,
        message: m.into(),
    }
}

/// RCR-040 and RCR-041 on the texts (the same reading as `crates/controller/tests/rules_crystal.rs`),
/// plus, with a list, `index`: every dependency is curated, at the list's git URL and commit, and
/// the lock names exactly the curated closure at the list's commits.
pub fn check_texts(yml: &str, lock: Option<&str>, index: Option<&ShardIndex>) -> Vec<Finding> {
    let lines: Vec<&str> = yml.lines().collect();
    let mut v = Vec::new();
    for k in ["name", "version", "crystal", "license"] {
        if top_value(&lines, k).is_none() {
            v.push(finding("RCR-040", YML, 1, format!("{YML} has no `{k}:`")));
        }
    }
    let deps = read_deps(yml);
    for d in &deps {
        match d.keys.get("commit") {
            Some(c) if c.len() == 40 && c.bytes().all(|b| b.is_ascii_hexdigit()) => {}
            Some(c) => v.push(finding(
                "RCR-040",
                YML,
                d.line,
                format!("{}: commit `{c}` is not a full 40-character hash", d.name),
            )),
            None => v.push(finding(
                "RCR-040",
                YML,
                d.line,
                format!(
                    "{}: not pinned by `commit:` (run `wol shard update {}`)",
                    d.name, d.name
                ),
            )),
        }
        for k in ["branch", "tag", "path", "version"] {
            if d.keys.contains_key(k) {
                v.push(finding(
                    "RCR-040",
                    YML,
                    d.line,
                    format!("{}: `{k}:` is not a pin", d.name),
                ));
            }
        }
    }
    let locked = lock.map(read_lock);
    if !deps.is_empty() {
        match &locked {
            None => v.push(finding(
                "RCR-041",
                LOCK,
                1,
                format!("{YML} has dependencies but there is no {LOCK} (run `wol shard update`)"),
            )),
            Some(l) => {
                for d in &deps {
                    match (l.get(&d.name), d.keys.get("commit")) {
                        (None, _) => v.push(finding(
                            "RCR-041",
                            YML,
                            d.line,
                            format!("{}: not in {LOCK}", d.name),
                        )),
                        (Some((ver, _)), Some(c)) if !ver.contains(&format!("git.commit.{c}")) => v
                            .push(finding(
                                "RCR-041",
                                YML,
                                d.line,
                                format!("{}: {LOCK} says `{ver}`, {YML} pins {c}", d.name),
                            )),
                        _ => {}
                    }
                }
            }
        }
    }
    let Some(index) = index else { return v };
    for d in &deps {
        match index.get(&d.name) {
            None => v.push(finding(
                "index",
                YML,
                d.line,
                format!("{}: not in the WolOS shard list", d.name),
            )),
            Some(e) => {
                if d.keys.get("git").is_some_and(|g| *g != e.git) {
                    v.push(finding(
                        "index",
                        YML,
                        d.line,
                        format!("{}: git is not the listed {}", d.name, e.git),
                    ));
                }
                if let Some(c) = d.keys.get("commit")
                    && *c != e.commit
                {
                    v.push(finding(
                        "index",
                        YML,
                        d.line,
                        format!(
                            "{}: pinned at {c}, the list vouches for {} (run `wol shard update {}`)",
                            d.name, e.commit, d.name
                        ),
                    ));
                }
            }
        }
    }
    if let Some(l) = &locked {
        let names: Vec<String> = deps.iter().map(|d| d.name.clone()).collect();
        let want: BTreeMap<&str, &Entry> = index
            .closure(&names)
            .map(|c| c.into_iter().map(|e| (e.name.as_str(), e)).collect())
            .unwrap_or_default();
        let have: BTreeSet<&str> = l.keys().map(String::as_str).collect();
        for (n, (ver, git)) in l {
            match want.get(n.as_str()) {
                None => v.push(finding(
                    "index",
                    LOCK,
                    1,
                    format!("{n}: in {LOCK} but not needed by a curated dependency"),
                )),
                Some(e) if *ver != e.locked_version() || *git != e.git => v.push(finding(
                    "index",
                    LOCK,
                    1,
                    format!(
                        "{n}: {LOCK} says `{ver}`, the list says `{}`",
                        e.locked_version()
                    ),
                )),
                _ => {}
            }
        }
        for n in want.keys().filter(|n| !have.contains(*n)) {
            v.push(finding(
                "index",
                LOCK,
                1,
                format!("{n}: needed but not in {LOCK} (run `wol shard update`)"),
            ));
        }
    }
    v
}

/// `check` on the project in `dir`.
pub fn check(dir: &Path, index: Option<&ShardIndex>) -> Result<Vec<Finding>> {
    let yml = read_project(dir)?;
    let lock = shared::bounded::read_file(&dir.join(LOCK), MAX_PROJECT_FILE)
        .map(|b| String::from_utf8_lossy(&b).into_owned());
    Ok(check_texts(&yml, lock.as_deref(), index))
}

// ---- Keys, network and cache (blocking: callers run these off the UI thread) ----

/// The shard index keys below the WolOS keys folder (`keys_dir/shards/*.pub`, minus its revoked
/// list).
pub fn keys(keys_dir: &Path) -> Vec<[u8; 32]> {
    wol_core::package::read_keys(&keys_dir.join(KEYS_SUBDIR))
}

/// GET `url` into memory, at most `max` bytes (a longer body is refused, not cut).
fn get_bounded(net: &crate::fetch::Net, url: &str, max: u64) -> Result<Vec<u8>> {
    net.check_url(url)?;
    let resp = net
        .agent
        .get(url)
        .call()
        .map_err(|_| Error::unavailable("The shard list could not be reached."))?;
    if resp.status().as_u16() != 200 {
        return Err(Error::unavailable("The shard list could not be reached."));
    }
    let mut out = Vec::new();
    resp.into_body()
        .into_reader()
        .take(max + 1)
        .read_to_end(&mut out)
        .map_err(|_| Error::unavailable("The download stopped."))?;
    if out.len() as u64 > max {
        return Err(Error::untrusted(
            "The download is larger than it should be.",
        ));
    }
    Ok(out)
}

/// The cached list once its signature checks; expiry is not looked at here.
fn cached_any(dir: &Path, keys: &[[u8; 32]]) -> Result<ShardIndex> {
    let text = shared::bounded::read_file(&dir.join(INDEX_FILE), MAX_INDEX as u64)
        .ok_or_else(|| Error::unavailable("The shard list has not been downloaded yet."))?;
    let sig = shared::bounded::read_file(&dir.join(SIG_FILE), 64);
    verify(&text, sig.as_deref(), keys)
}

/// The cached list in `dir`, checked again on every read and refused once past its `expires`.
pub fn cached(dir: &Path, keys: &[[u8; 32]]) -> Result<ShardIndex> {
    let i = cached_any(dir, keys)?;
    crate::release::check_fresh(
        "shard list",
        i.serial,
        i.expires,
        0,
        crate::release::unix_now(),
    )?;
    Ok(i)
}

/// Downloads `url` and `url.sig`, verifies them, refuses an older serial than the cached one or
/// an expired list, and only then replaces the cache in `dir`.
pub fn refresh(
    net: &crate::fetch::Net,
    url: &str,
    dir: &Path,
    keys: &[[u8; 32]],
) -> Result<ShardIndex> {
    let text = get_bounded(net, url, MAX_INDEX as u64)?;
    let sig = get_bounded(net, &format!("{url}.sig"), 64)?;
    let i = verify(&text, Some(&sig), keys)?;
    let last = cached_any(dir, keys).map_or(0, |c| c.serial);
    crate::release::check_fresh(
        "shard list",
        i.serial,
        i.expires,
        last,
        crate::release::unix_now(),
    )?;
    let cant = || Error::unavailable("The shard list cannot be saved.");
    std::fs::create_dir_all(dir).map_err(|_| cant())?;
    for (name, bytes) in [(SIG_FILE, &sig), (INDEX_FILE, &text)] {
        shared::atomic::atomic_write(&dir.join(name), 0o644, bytes).map_err(|_| cant())?;
    }
    Ok(i)
}

/// The list: a download when `fresh` (falling back to a valid cache), else the valid cache, and a
/// download when there is none.
pub fn load(
    net: &crate::fetch::Net,
    url: &str,
    dir: &Path,
    keys: &[[u8; 32]],
    fresh: bool,
) -> Result<ShardIndex> {
    if keys.is_empty() {
        return Err(Error::untrusted(
            "No WolOS shard index key is installed (/usr/share/wolos/keys/shards).",
        ));
    }
    if fresh {
        return refresh(net, url, dir, keys).or_else(|e| cached(dir, keys).map_err(|_| e));
    }
    cached(dir, keys).or_else(|_| refresh(net, url, dir, keys))
}

// ---- The JSON requests of the C entry points (wopkg.h `wopkg_shard_*`) ----

/// `wopkg_shard_index` request.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct IndexRequest {
    /// Download even when a valid cached copy exists.
    pub fresh: bool,
    /// Filter (name, summary, license); empty = all.
    pub term: String,
    /// Another signed list (same keys); empty = [`DEFAULT_URL`].
    pub url: String,
}

/// `wopkg_shard_pin` request.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PinRequest {
    /// The project folder (holds `shard.yml`).
    pub dir: PathBuf,
    pub names: Vec<String>,
    /// `update` (re-pin) instead of `add`.
    pub update: bool,
    pub fresh: bool,
    pub url: String,
}

/// `wopkg_shard_check` request.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CheckRequest {
    pub dir: PathBuf,
    /// Check against the local files only (no list, no `index` findings).
    pub offline: bool,
    pub url: String,
}

fn url_or_default(u: &str) -> &str {
    if u.is_empty() { DEFAULT_URL } else { u }
}

fn context_index(c: &crate::context::Context, url: &str, fresh: bool) -> Result<ShardIndex> {
    load(
        &c.net,
        url_or_default(url),
        &c.cache.join(CACHE_SUBDIR),
        &keys(&c.keys),
        fresh,
    )
}

/// `{"shards":[..],"serial":N,"expires":T}`.
pub fn index_json(c: &crate::context::Context, r: &IndexRequest) -> Result<serde_json::Value> {
    let i = context_index(c, &r.url, r.fresh)?;
    Ok(serde_json::json!({
        "shards": i.search(&r.term),
        "serial": i.serial,
        "expires": i.expires,
    }))
}

/// `{"changed":[..],"locked":[..],"shard_yml":..,"shard_lock":..}`.
pub fn pin_json(c: &crate::context::Context, r: &PinRequest) -> Result<serde_json::Value> {
    if r.dir.as_os_str().is_empty() {
        return Err(Error::invalid("The request names no project folder."));
    }
    let i = context_index(c, &r.url, r.fresh)?;
    serde_json::to_value(pin(&i, &r.dir, &r.names, r.update)?)
        .map_err(|_| Error::internal("A result could not be written."))
}

/// `{"ok":bool,"index":bool,"findings":[..]}`: `index` says whether the list was read; without
/// it (offline, or no list could be loaded) only RCR-040 and RCR-041 are checked.
pub fn check_json(c: &crate::context::Context, r: &CheckRequest) -> Result<serde_json::Value> {
    if r.dir.as_os_str().is_empty() {
        return Err(Error::invalid("The request names no project folder."));
    }
    let i = if r.offline {
        None
    } else {
        context_index(c, &r.url, false).ok()
    };
    let f = check(&r.dir, i.as_ref())?;
    Ok(serde_json::json!({ "ok": f.is_empty(), "index": i.is_some(), "findings": f }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: [u8; 32] = [5; 32];
    const C1: &str = "1111111111111111111111111111111111111111";
    const C2: &str = "2222222222222222222222222222222222222222";
    const C3: &str = "3333333333333333333333333333333333333333";

    fn entry(name: &str, commit: &str, deps: &[&str]) -> serde_json::Value {
        serde_json::json!({
            "name": name, "git": format!("https://git.wolos.org/shards/{name}.git"),
            "version": "0.3.0", "commit": commit, "license": "BSD-2-Clause",
            "summary": format!("The {name} shard"), "crystal": ">= 1.22.0",
            "dependencies": deps,
        })
    }

    fn index(shards: Vec<serde_json::Value>) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"format": 1, "serial": 3, "shards": shards}))
            .unwrap()
    }

    fn list() -> ShardIndex {
        parse(&index(vec![
            entry("wolkit", C1, &["cryswol"]),
            entry("cryswol", C2, &[]),
            entry("lonely", C3, &[]),
        ]))
        .unwrap()
    }

    const APP: &str = "name: demo\nversion: 0.1.0\ncrystal: \">= 1.22.0\"\nlicense: MIT\n\ntargets:\n  demo:\n    main: src/demo.cr\n";

    #[test]
    fn a_signed_index_lists_its_valid_entries() {
        let text = index(vec![entry("wolkit", C1, &[]), entry("cryswol", C2, &[])]);
        let sig = wol_core::package::sign(&SEED, &text);
        let keys = [wol_core::package::public_of(&SEED)];
        let i = verify(&text, Some(&sig), &keys).unwrap();
        assert_eq!(i.shards.len(), 2);
        assert_eq!(i.serial, 3);
        assert_eq!(
            i.get("wolkit").unwrap().locked_version(),
            format!("0.3.0+git.commit.{C1}")
        );
        assert!(verify(&text, None, &keys).is_err(), "unsigned");
        assert!(
            verify(&text, Some(&[0; 64]), &keys).is_err(),
            "bad signature"
        );
        let other = [wol_core::package::public_of(&[9; 32])];
        assert!(verify(&text, Some(&sig), &other).is_err(), "another key");
        let mut changed = text.clone();
        changed[20] ^= 1;
        assert!(
            verify(&changed, Some(&sig), &keys).is_err(),
            "changed after signing"
        );
    }

    #[test]
    fn bad_entries_are_dropped_one_by_one() {
        let mut bad = vec![];
        for (k, v) in [
            ("name", serde_json::json!("Wolkit")),
            ("name", serde_json::json!("9lives")),
            ("name", serde_json::json!("a b")),
            ("name", serde_json::json!("x".repeat(51))),
            ("git", serde_json::json!("http://git.wolos.org/x.git")),
            ("git", serde_json::json!("https://user@git.wolos.org/x.git")),
            ("git", serde_json::json!("https://git.wolos.org/x.git#main")),
            ("git", serde_json::json!("https://git.wolos.org/x y.git")),
            ("git", serde_json::json!("https://git.wolos.org/../x.git")),
            ("git", serde_json::json!("https://git.wolos.org/")),
            ("git", serde_json::json!("file:///srv/x.git")),
            ("commit", serde_json::json!("abc")),
            (
                "commit",
                serde_json::json!(C1.to_uppercase().replace('1', "A")),
            ),
            ("version", serde_json::json!("v1.0")),
            ("version", serde_json::json!("1.0+meta")),
            ("license", serde_json::json!("")),
            ("summary", serde_json::json!("bell\u{7}")),
            ("dependencies", serde_json::json!(["Bad"])),
            ("dependencies", serde_json::json!(["alice"])),
            ("dependencies", serde_json::json!("cryswol")),
        ] {
            let mut e = entry("alice", C1, &[]);
            e[k] = v;
            bad.push(e);
        }
        bad.push(serde_json::json!(7));
        bad.push(entry("good", C1, &[]));
        bad.push(entry("good", C2, &[]));
        let i = parse(&index(bad)).unwrap();
        let names: Vec<&str> = i.shards.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["good"]);
        assert_eq!(i.get("good").unwrap().commit, C1, "the first one wins");
    }

    #[test]
    fn parse_is_bounded_and_total() {
        assert!(parse(b"").is_err());
        assert!(parse(b"{\"format\":2,\"shards\":[]}").is_err());
        assert!(parse(&vec![b' '; MAX_INDEX + 1]).is_err());
        assert_eq!(parse(b"{\"format\":1}").unwrap(), ShardIndex::default());
        let text = index(vec![entry("wolkit", C1, &[])]);
        for cut in 0..text.len() {
            let _ = parse(&text[..cut]);
        }
        let mut x: u32 = 11;
        for _ in 0..300 {
            let mut t = text.clone();
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let i = (x as usize) % t.len();
            t[i] = (x >> 16) as u8;
            let _ = parse(&t);
        }
    }

    #[test]
    fn search_ranks_names_first_and_closure_follows_dependencies() {
        let i = list();
        let hits: Vec<&str> = i
            .search("CRYSWOL")
            .iter()
            .map(|e| e.name.as_str())
            .collect();
        assert_eq!(hits, ["cryswol"]);
        assert_eq!(i.search("shard").len(), 3);
        assert_eq!(i.search(" ").len(), 3);
        let c: Vec<&str> = i
            .closure(&["wolkit".into()])
            .unwrap()
            .iter()
            .map(|e| e.name.as_str())
            .collect();
        assert_eq!(c, ["cryswol", "wolkit"]);
        let err = i.closure(&["nope".into()]).unwrap_err();
        assert_eq!(err.status, crate::ffi::Status::NotFound);
        // A dependency the list does not carry: the lock cannot be complete.
        let broken = parse(&index(vec![entry("wolkit", C1, &["ghost"])])).unwrap();
        let err = broken.closure(&["wolkit".into()]).unwrap_err();
        assert!(err.message.contains("ghost"), "{err}");
        // A cycle ends.
        let cyc = parse(&index(vec![entry("a", C1, &["b"]), entry("b", C2, &["a"])])).unwrap();
        assert_eq!(cyc.closure(&["a".into()]).unwrap().len(), 2);
    }

    /// The checks of crates/controller/tests/support/crystal_scan.rs (RCR-040/041) on our output.
    fn rcr_clean(yml: &str, lock: &str) {
        let f = check_texts(yml, Some(lock), None);
        assert!(f.is_empty(), "{f:?}\n{yml}\n{lock}");
    }

    #[test]
    fn add_pins_by_commit_and_locks_the_closure() {
        let i = list();
        let (yml, lock, changed) = plan_pin(&i, APP, &["wolkit".into()], false).unwrap();
        assert!(yml.starts_with(APP), "the rest of the file is kept:\n{yml}");
        assert!(yml.ends_with(&format!(
            "dependencies:\n  wolkit:\n    git: https://git.wolos.org/shards/wolkit.git\n    commit: {C1}\n"
        )), "{yml}");
        assert_eq!(
            lock,
            format!(
                "version: 2.0\n\
                    shards:\n  cryswol:\n    git: https://git.wolos.org/shards/cryswol.git\n    version: 0.3.0+git.commit.{C2}\n\n  \
                    wolkit:\n    git: https://git.wolos.org/shards/wolkit.git\n    version: 0.3.0+git.commit.{C1}\n"
            )
        );
        assert_eq!(changed.len(), 1);
        assert_eq!((changed[0].from.as_str(), changed[0].to.as_str()), ("", C1));
        rcr_clean(&yml, &lock);
        assert!(check_texts(&yml, Some(&lock), Some(&i)).is_empty());
        // Adding again changes nothing; a second shard joins the same list.
        let (again, lock2, c2) = plan_pin(&i, &yml, &["wolkit".into()], false).unwrap();
        assert_eq!(
            (again.as_str(), lock2.as_str(), c2.len()),
            (yml.as_str(), lock.as_str(), 0)
        );
        let (two, lock3, _) = plan_pin(&i, &yml, &["lonely".into()], false).unwrap();
        assert_eq!(read_deps(&two).len(), 2);
        assert_eq!(read_lock(&lock3).len(), 3);
        rcr_clean(&two, &lock3);
        // Unknown names, bad names and no names are refused.
        assert_eq!(
            plan_pin(&i, APP, &["nope".into()], false)
                .unwrap_err()
                .status,
            crate::ffi::Status::NotFound
        );
        assert!(plan_pin(&i, APP, &["../x".into()], false).is_err());
        assert!(plan_pin(&i, APP, &[], false).is_err());
    }

    #[test]
    fn add_replaces_a_loose_dependency_and_fills_inline_lists() {
        let i = list();
        let loose = format!(
            "{APP}\n\
                dependencies:\n  wolkit:\n    github: someone/wolkit\n    version: ~> 0.2\n  # kept\n\n\
                development_dependencies:\n  cryswol:\n    git: https://git.wolos.org/shards/cryswol.git\n    branch: main\n"
        );
        let (yml, lock, changed) = plan_pin(&i, &loose, &[], true).unwrap();
        assert_eq!(changed.len(), 2);
        let deps = read_deps(&yml);
        assert_eq!(deps[0].section, Section::Dependencies);
        assert_eq!(
            deps[1].section,
            Section::DevelopmentDependencies,
            "stays in its list"
        );
        assert!(!yml.contains("github:") && !yml.contains("branch:") && !yml.contains("~>"));
        assert!(yml.contains("# kept"));
        rcr_clean(&yml, &lock);
        let inline = APP.replace("targets:", "dependencies: {}\ntargets:");
        let (yml, lock, _) = plan_pin(&i, &inline, &["lonely".into()], false).unwrap();
        assert!(yml.contains("dependencies:\n  lonely:\n"), "{yml}");
        assert!(!yml.contains("{}"));
        rcr_clean(&yml, &lock);
        // Before another top-level key: the new entry goes inside the list, not after it.
        let mid = APP.replace(
            "targets:",
            &format!("dependencies:\n  cryswol:\n    git: https://git.wolos.org/shards/cryswol.git\n    commit: {C2}\n\ntargets:"),
        );
        let (yml, lock, _) = plan_pin(&i, &mid, &["lonely".into()], false).unwrap();
        assert!(yml.contains(&format!("commit: {C2}\n  lonely:\n")), "{yml}");
        assert!(yml.contains("targets:\n  demo:"));
        rcr_clean(&yml, &lock);
    }

    #[test]
    fn update_moves_pins_to_the_list_and_reports_them() {
        let old = list();
        let (yml, _, _) = plan_pin(&old, APP, &["wolkit".into()], false).unwrap();
        let newer = parse(&index(vec![
            entry("wolkit", C3, &["cryswol"]),
            entry("cryswol", C2, &[]),
        ]))
        .unwrap();
        let (moved, lock, changed) = plan_pin(&newer, &yml, &[], true).unwrap();
        assert_eq!(changed.len(), 1);
        assert_eq!((changed[0].from.as_str(), changed[0].to.as_str()), (C1, C3));
        assert!(moved.contains(C3) && !moved.contains(C1));
        rcr_clean(&moved, &lock);
        // `check` against the newer list sees the old pin and the old lock as stale.
        let old_lock = render_lock(&old.closure(&["wolkit".into()]).unwrap());
        let f = check_texts(&yml, Some(&old_lock), Some(&newer));
        assert!(
            f.iter()
                .any(|f| f.rule == "index" && f.message.contains("vouches for")),
            "{f:?}"
        );
        assert!(f.iter().any(|f| f.file == "shard.lock"), "{f:?}");
        // update of a name that is not a dependency is refused.
        assert!(plan_pin(&newer, &yml, &["cryswol".into()], true).is_err());
    }

    #[test]
    fn check_finds_what_rcr_040_and_041_forbid() {
        let i = list();
        let f = check_texts("name: x\n", None, None);
        assert_eq!(f.iter().filter(|f| f.rule == "RCR-040").count(), 3, "{f:?}");
        let loose = format!("{APP}dependencies:\n  wolkit:\n    github: x/wolkit\n    tag: v1\n");
        let f = check_texts(&loose, None, Some(&i));
        let rules: BTreeSet<&str> = f.iter().map(|f| f.rule).collect();
        assert_eq!(rules, ["RCR-040", "RCR-041"].into(), "{f:?}");
        let pinned = format!(
            "{APP}dependencies:\n  ghost:\n    git: https://git.wolos.org/shards/ghost.git\n    commit: {C1}\n"
        );
        let lock = format!(
            "version: 2.0\nshards:\n  ghost:\n    git: https://git.wolos.org/shards/ghost.git\n    version: 0.1.0+git.commit.{C1}\n"
        );
        assert!(check_texts(&pinned, Some(&lock), None).is_empty());
        let f = check_texts(&pinned, Some(&lock), Some(&i));
        assert!(
            f.iter()
                .any(|f| f.message.contains("not in the WolOS shard list")),
            "{f:?}"
        );
        let wrong = lock.replace(C1, C2);
        let f = check_texts(&pinned, Some(&wrong), None);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].rule, "RCR-041");
        // No dependencies: no lock needed.
        assert!(check_texts(APP, None, Some(&i)).is_empty());
    }

    #[test]
    fn pin_writes_both_files_and_nothing_on_failure() {
        let dir = crate::testutil::temp("shard-pin");
        std::fs::create_dir_all(&dir).unwrap();
        let i = list();
        assert_eq!(
            pin(&i, &dir, &["wolkit".into()], false).unwrap_err().status,
            crate::ffi::Status::NotFound,
            "no shard.yml"
        );
        std::fs::write(dir.join("shard.yml"), APP).unwrap();
        assert!(pin(&i, &dir, &["nope".into()], false).is_err());
        assert_eq!(std::fs::read_to_string(dir.join("shard.yml")).unwrap(), APP);
        assert!(!dir.join("shard.lock").exists());
        let p = pin(&i, &dir, &["wolkit".into()], false).unwrap();
        let names: Vec<&str> = p.locked.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["cryswol", "wolkit"]);
        assert_eq!(p.locked[1].commit, C1);
        assert!(check(&dir, Some(&i)).unwrap().is_empty());
        let left = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(left, 2, "no temp files left");
        // A linked shard.yml is refused.
        let link = crate::testutil::temp("shard-link");
        std::fs::create_dir_all(&link).unwrap();
        std::os::unix::fs::symlink(dir.join("shard.yml"), link.join("shard.yml")).unwrap();
        assert!(pin(&i, &link, &["wolkit".into()], false).is_err());
    }

    #[test]
    fn keys_are_their_own_folder() {
        let dir = crate::testutil::temp("shard-keys");
        std::fs::create_dir_all(dir.join(KEYS_SUBDIR)).unwrap();
        let pk = |seed: &[u8; 32]| {
            let k = wol_core::package::public_of(seed);
            wol_core::package::public_pem(&k)
        };
        // A key beside the package keys is not a shard key; one in shards/ is.
        std::fs::write(dir.join("wolos-2026.pub"), pk(&[1; 32])).unwrap();
        assert!(keys(&dir).is_empty());
        std::fs::write(dir.join(KEYS_SUBDIR).join("shards-2026.pub"), pk(&SEED)).unwrap();
        assert_eq!(keys(&dir), [wol_core::package::public_of(&SEED)]);
        let net = crate::fetch::Net::new(true);
        let err = load(&net, DEFAULT_URL, &dir.join("cache"), &[], false).unwrap_err();
        assert_eq!(err.status, crate::ffi::Status::Untrusted);
    }

    #[test]
    fn refresh_keeps_only_a_verified_list_and_never_goes_back() {
        use crate::testutil::{Served, serve, temp};
        let keys = [wol_core::package::public_of(&SEED)];
        let signed = |t: &[u8]| Served::bytes(wol_core::package::sign(&SEED, t).to_vec());
        let v = |serial: u64, expires: u64| {
            serde_json::to_vec(&serde_json::json!({
                "format": 1, "serial": serial, "expires": expires,
                "shards": [entry("wolkit", C1, &[])]
            }))
            .unwrap()
        };
        let (five, four, gone) = (v(5, 0), v(4, 0), v(9, 1));
        let s = serve(vec![
            ("/index.json", Served::bytes(five.clone())),
            ("/index.json.sig", signed(&five)),
            ("/old/index.json", Served::bytes(four.clone())),
            ("/old/index.json.sig", signed(&four)),
            ("/exp/index.json", Served::bytes(gone.clone())),
            ("/exp/index.json.sig", signed(&gone)),
            ("/bad/index.json", Served::bytes(five.clone())),
            ("/bad/index.json.sig", Served::bytes(vec![0; 64])),
        ]);
        let net = crate::fetch::Net::new(true);
        let dir = temp("shard-index");
        assert!(cached(&dir, &keys).is_err());
        let i = load(&net, &s.url("/index.json"), &dir, &keys, false).unwrap();
        assert_eq!(i.serial, 5);
        assert_eq!(cached(&dir, &keys).unwrap(), i);
        for bad in ["/old/index.json", "/exp/index.json", "/bad/index.json"] {
            assert!(refresh(&net, &s.url(bad), &dir, &keys).is_err(), "{bad}");
            assert_eq!(cached(&dir, &keys).unwrap().serial, 5, "{bad}");
        }
        // `fresh` falls back to the valid cache when the download fails.
        let missing = s.url("/missing/index.json");
        assert_eq!(load(&net, &missing, &dir, &keys, true).unwrap().serial, 5);
        // A changed cache is refused on read.
        std::fs::write(dir.join(INDEX_FILE), b"{\"format\":1,\"shards\":[]}").unwrap();
        assert!(cached(&dir, &keys).is_err());
    }

    /// The signed list the wopkg Crystal specs use (`wopkg/spec/fixtures/shards/`), signed by the
    /// test seed above, never expiring. `WOL_BLESS=1` writes it again.
    #[test]
    fn the_crystal_spec_fixture_is_signed_by_the_test_key() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../wopkg/spec/fixtures/shards");
        let mut text = serde_json::to_vec_pretty(&serde_json::json!({
            "format": 1, "serial": 1, "expires": 0,
            "shards": [entry("wolkit", C1, &["cryswol"]), entry("cryswol", C2, &[])]
        }))
        .unwrap();
        text.push(b'\n');
        let sig = wol_core::package::sign(&SEED, &text).to_vec();
        let key = wol_core::package::public_pem(&wol_core::package::public_of(&SEED));
        let files: [(&str, &[u8]); 3] = [
            ("index.json", &text),
            ("index.json.sig", &sig),
            ("keys/shards/test.pub", key.as_bytes()),
        ];
        for (rel, want) in files {
            let p = dir.join(rel);
            if std::env::var_os("WOL_BLESS").is_some() {
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(&p, want).unwrap();
            }
            let have = std::fs::read(&p).unwrap_or_default();
            assert!(
                have == want,
                "{} is stale: run WOL_BLESS=1 cargo test -p wopkg --lib index::shards",
                p.display()
            );
        }
        let keys = keys(&dir.join("keys"));
        assert_eq!(verify(&text, Some(&sig), &keys).unwrap().shards.len(), 2);
    }
}
