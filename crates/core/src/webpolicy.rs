// SPDX-License-Identifier: BSD-2-Clause
//! Per-app web policy, one format for both tracks of the web-app runtime:
//! `/usr/share/wolos/webpolicy/<app>.json`. Track 2 (WolOS's own runtime) reads it in the engine
//! adaptor and edits no file. Track 1 (apps that bundle their own engine) applies it three ways:
//! the flag rows of [`crate::webruntime`] the policy allows go into the session's flag files; byte
//! patches go through the pacman hook (`files/usr/lib/wolos/webpolicy`) at every install and
//! upgrade; AppImages are never edited (their image is signed and compressed), only their launcher
//! gets the allowed flags (`wopkg::appimage`).
//!
//! Invariants: a policy is data, a patch never runs code. A patch edits one regular file under
//! [`PATCH_ROOTS`] and only when the whole file still has the `sha256` it was written for and the
//! bytes at `offset` are `find`; otherwise it is refused, never forced. The shell helper does not
//! parse JSON: [`plan`] compiles each policy into `<app>.plan` lines the helper checks again.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

use crate::webruntime::{Knob, Role, Row};

/// Shipped policies.
pub const DIR: &str = "/usr/share/wolos/webpolicy";
/// What the helper applied: `<app>.state`, one `<file> <patched sha256>` line per patch.
pub const STATE_DIR: &str = "/var/lib/wolos/webpolicy";
/// Largest policy file read, in bytes.
pub const MAX_BYTES: usize = 64 * 1024;
/// Most patches one policy file may carry.
pub const MAX_PATCHES: usize = 64;
/// Longest `find` or `with`, in bytes.
pub const MAX_EDIT: usize = 4096;
/// Longest list (`hosts`, `beacon_paths`, `features`).
pub const MAX_LIST: usize = 256;
/// Where a patch may write (packaged app trees, never configuration or home).
pub const PATCH_ROOTS: &[&str] = &["/usr/lib/", "/opt/"];

/// Requests to the app's analytics hosts and beacon paths are dropped.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Telemetry {
    pub hosts: Vec<String>,
    pub beacon_paths: Vec<String>,
}

/// Jank fixes: busy timers clamped, animations of hidden windows stopped.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Jank {
    pub busy_timers: bool,
    pub hidden_animations: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
/// What a [`Patch`] does with the bytes it found: put `with` in their place, or drop them.
pub enum Op {
    Replace,
    Remove,
}

/// One byte edit of a packaged file (track 1 only).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Patch {
    /// Absolute path under [`PATCH_ROOTS`].
    pub file: String,
    /// sha256 of the whole original file, lowercase hex.
    pub sha256: String,
    pub op: Op,
    /// Byte offset of `find` in the original file.
    pub offset: u64,
    pub find: String,
    /// Replacement (`replace` only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub with: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// One app's policy file, `<DIR>/<app>.json` (docs/modules/webapps.md); every key is optional.
pub struct Policy {
    pub telemetry_off: Telemetry,
    pub jank: Jank,
    /// Ceiling for the heap-cap flag rows; none: the app takes no heap cap.
    pub heap_mb: Option<u32>,
    /// Allows the background-throttling and timer-alignment rows.
    pub background_throttle: bool,
    /// Web features the app needs (the engine's feature vocabulary).
    pub features: Vec<String>,
    pub patches: Vec<Patch>,
}

/// Why a policy is refused. `Display` is one sentence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    TooLarge,
    Syntax(String),
    BadId,
    BadList(&'static str),
    BadHeap,
    TooManyPatches,
    BadPatch(usize, &'static str),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::TooLarge => write!(f, "The policy is larger than {MAX_BYTES} bytes."),
            Error::Syntax(e) => write!(f, "The policy is not valid: {e}."),
            Error::BadId => f.write_str("The policy file name is not an app id."),
            Error::BadList(k) => {
                write!(f, "The policy's {k} list has an entry that is not allowed.")
            }
            Error::BadHeap => f.write_str("heap_mb is outside 128 to 65536."),
            Error::TooManyPatches => write!(f, "The policy has more than {MAX_PATCHES} patches."),
            Error::BadPatch(i, why) => write!(f, "Patch {i} is refused: {why}."),
        }
    }
}

/// App ids are file stems: lowercase letters, digits, `-`, `.`, `_`, 1 to 64 bytes, no leading dot.
pub fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && !id.starts_with('.')
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-._".contains(&b))
}

fn word(s: &str, extra: &[u8]) -> bool {
    !s.is_empty()
        && s.len() <= 253
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._".contains(&b) || extra.contains(&b))
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A patch target: absolute, under a root, no `..`, `.`, empty part, or byte outside a plain path.
pub fn valid_target(p: &str) -> bool {
    PATCH_ROOTS.iter().any(|r| p.starts_with(r))
        && p.len() <= 4096
        && p[1..]
            .split('/')
            .all(|c| !c.is_empty() && c != "." && c != "..")
        && p.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/-._+@".contains(&b))
}

impl Patch {
    fn check(&self, i: usize) -> Result<(), Error> {
        let bad = |why| Err(Error::BadPatch(i, why));
        if !valid_target(&self.file) {
            return bad("the file is not a plain path under /usr/lib or /opt");
        }
        if !is_hex64(&self.sha256) {
            return bad("sha256 is not 64 lowercase hex digits");
        }
        if self.find.is_empty() || self.find.len() > MAX_EDIT {
            return bad("find is empty or too long");
        }
        match (self.op, &self.with) {
            (Op::Replace, Some(w)) if w.len() <= MAX_EDIT => Ok(()),
            (Op::Replace, _) => bad("replace needs a with of at most 4096 bytes"),
            (Op::Remove, None) => Ok(()),
            (Op::Remove, Some(_)) => bad("remove takes no with"),
        }
    }

    /// The bytes put in place of `find`: `with`, or nothing for `remove`.
    pub fn replacement(&self) -> &str {
        self.with.as_deref().unwrap_or("")
    }

    /// The edit on the original bytes: refused unless the hash and the bytes at the offset match.
    pub fn apply(&self, original: &[u8]) -> Result<Vec<u8>, &'static str> {
        if sha256_hex(original) != self.sha256 {
            return Err("the file is not the version the patch was written for");
        }
        let at = usize::try_from(self.offset).map_err(|_| "offset out of range")?;
        let end = at
            .checked_add(self.find.len())
            .ok_or("offset out of range")?;
        if original.get(at..end) != Some(self.find.as_bytes()) {
            return Err("the bytes at the offset are not the ones to edit");
        }
        let mut out =
            Vec::with_capacity(original.len() - self.find.len() + self.replacement().len());
        out.extend_from_slice(&original[..at]);
        out.extend_from_slice(self.replacement().as_bytes());
        out.extend_from_slice(&original[end..]);
        Ok(out)
    }
}

/// sha256 of `b` as 64 lowercase hex digits (the form patches and state lines carry).
pub fn sha256_hex(b: &[u8]) -> String {
    Sha256::digest(b)
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect()
}

/// Parse and check one policy. Patches on one file must not overlap and are listed by offset.
pub fn parse(text: &[u8]) -> Result<Policy, Error> {
    if text.len() > MAX_BYTES {
        return Err(Error::TooLarge);
    }
    let p: Policy = serde_json::from_slice(text).map_err(|e| Error::Syntax(e.to_string()))?;
    let lists = [
        ("hosts", &p.telemetry_off.hosts, &b""[..]),
        ("beacon_paths", &p.telemetry_off.beacon_paths, &b"/"[..]),
        ("features", &p.features, &b""[..]),
    ];
    for (k, v, extra) in lists {
        if v.len() > MAX_LIST || !v.iter().all(|s| word(s, extra)) {
            return Err(Error::BadList(k));
        }
    }
    if p.telemetry_off
        .beacon_paths
        .iter()
        .any(|s| !s.starts_with('/'))
    {
        return Err(Error::BadList("beacon_paths"));
    }
    if p.heap_mb.is_some_and(|h| !(128..=65536).contains(&h)) {
        return Err(Error::BadHeap);
    }
    if p.patches.len() > MAX_PATCHES {
        return Err(Error::TooManyPatches);
    }
    for (i, x) in p.patches.iter().enumerate() {
        x.check(i)?;
        if let Some(prev) = p.patches[..i].iter().rev().find(|q| q.file == x.file) {
            if prev.sha256 != x.sha256 {
                return Err(Error::BadPatch(i, "two hashes for one file"));
            }
            if prev.offset + prev.find.len() as u64 > x.offset {
                return Err(Error::BadPatch(
                    i,
                    "overlaps or comes before the previous edit",
                ));
            }
        }
    }
    Ok(p)
}

/// Read `<dir>/<id>.json`: `None` when there is none, the error when it is refused.
pub fn load(dir: &Path, id: &str) -> Option<Result<Policy, Error>> {
    if !valid_id(id) {
        return Some(Err(Error::BadId));
    }
    let b = std::fs::read(dir.join(format!("{id}.json"))).ok()?;
    Some(parse(&b))
}

/// The pacman hook: runs the helper after any transaction that installs or upgrades a plan or a
/// file a plan patches (`targets`: every patched file of every shipped policy).
pub fn hook(targets: &[&str]) -> String {
    let mut t: Vec<&str> = targets.iter().map(|f| f.trim_start_matches('/')).collect();
    t.sort();
    t.dedup();
    let mut s = String::from(
        "# Web policy, track 1 (docs/modules/webapps.md): re-apply the byte patches of the shipped\n\
         # policies whenever a patched file or a policy plan is installed or upgraded. Generated by\n\
         # wol_core::webpolicy::hook; a refused patch is reported and the file left as packaged.\n\
         [Trigger]\nType = Path\nOperation = Install\nOperation = Upgrade\n\
         Target = usr/share/wolos/webpolicy/*.plan\n",
    );
    for f in t {
        s.push_str(&format!("Target = {f}\n"));
    }
    s.push_str(
        "\n[Action]\nDescription = Applying WolOS web-app policies...\nWhen = PostTransaction\n\
         Exec = /usr/lib/wolos/webpolicy\n",
    );
    s
}

/// The browser's policy as shipped: the only source of its managed-policy rows.
pub const HELIUM_JSON: &str = include_str!("../../../files/usr/share/wolos/webpolicy/helium.json");

/// Managed browser policies: shipped table rows the browser's policy allows.
pub fn browser_policies(p: &Policy) -> serde_json::Map<String, serde_json::Value> {
    crate::webruntime::browser_policies(&|row| allows(p, row))
}

/// One line of `<STATE_DIR>/<app>.state`, written by the helper:
/// `patched <file> <sha256>` or `refused <file> <unix time> <expected sha256> <found sha256 or -> <reason>`.
/// Settings reads the refusals to show "policy not applied".
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StateLine {
    Patched {
        file: String,
        sha256: String,
    },
    Refused {
        file: String,
        time: u64,
        expected: String,
        found: Option<String>,
        reason: String,
    },
}

/// Lines that do not parse are skipped (the helper is the only writer).
pub fn parse_state(text: &str) -> Vec<StateLine> {
    let mut out = Vec::new();
    for l in text.lines() {
        let mut w = l.splitn(6, ' ');
        match (w.next(), w.next()) {
            (Some("patched"), Some(file)) => {
                if let Some(sha) = w.next().filter(|s| is_hex64(s)) {
                    out.push(StateLine::Patched {
                        file: file.into(),
                        sha256: sha.into(),
                    });
                }
            }
            (Some("refused"), Some(file)) => {
                let (t, e, f, r) = (w.next(), w.next(), w.next(), w.next());
                if let (Some(Ok(time)), Some(e), Some(f), Some(r)) = (t.map(str::parse), e, f, r) {
                    out.push(StateLine::Refused {
                        file: file.into(),
                        time,
                        expected: e.into(),
                        found: (f != "-").then(|| f.to_owned()),
                        reason: r.into(),
                    });
                }
            }
            _ => {}
        }
    }
    out
}

/// The policy that governs a role's flag file: the shared-engine apps share one.
pub fn role_id(r: Role) -> &'static str {
    match r {
        Role::App => "electron",
        Role::Editor => "code",
        Role::Browser => "helium",
    }
}

fn heap_mb(row: &Row) -> Option<u32> {
    match row.knob {
        Knob::JsFlag(f) => f.strip_prefix("--max-old-space-size=")?.parse().ok(),
        _ => None,
    }
}

/// Whether the policy lets a flag row through. Baseline rows always; heap caps only up to
/// `heap_mb`; throttling only with `background_throttle`; every other row as the table says.
pub fn allows(p: &Policy, row: &Row) -> bool {
    use crate::webruntime::Status;
    if row.status == Status::Baseline {
        return true;
    }
    if let Some(mb) = heap_mb(row) {
        return p.heap_mb.is_some_and(|cap| mb <= cap);
    }
    let throttle = matches!(row.knob, Knob::Feature(_))
        || matches!(row.knob, Knob::Policy(k, _) if k.contains("WakeUp"));
    if throttle {
        return p.background_throttle;
    }
    true
}

/// One helper line per patch: `<file> <sha256> <offset> <find hex> <with octal or ->`. Every field
/// is one word, so the POSIX helper reads it with `read -r` and no parser. Lines are grouped by
/// file and run from the last offset to the first, so each edit leaves the earlier offsets valid.
pub fn plan(p: &Policy) -> String {
    let mut s = String::from(
        "# webpolicy plan, generated from the JSON next to it: file sha256 offset find with\n",
    );
    let mut order: Vec<&Patch> = p.patches.iter().collect();
    order.sort_by(|a, b| a.file.cmp(&b.file).then(b.offset.cmp(&a.offset)));
    for x in order {
        let find: String = x.find.bytes().map(|b| format!("{b:02x}")).collect();
        let with: String = if x.replacement().is_empty() {
            "-".into()
        } else {
            x.replacement()
                .bytes()
                .map(|b| format!("\\0{b:03o}"))
                .collect()
        };
        s.push_str(&format!(
            "{} {} {} {find} {with}\n",
            x.file, x.sha256, x.offset
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::webruntime::{self, Gpu, Profile, ROWS};
    use std::path::PathBuf;

    fn shipped(rel: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../files/usr/share/wolos/webpolicy")
            .join(rel)
    }

    fn patch(file: &str, original: &[u8], offset: u64, find: &str, with: Option<&str>) -> Patch {
        Patch {
            file: file.into(),
            sha256: sha256_hex(original),
            op: if with.is_some() {
                Op::Replace
            } else {
                Op::Remove
            },
            offset,
            find: find.into(),
            with: with.map(Into::into),
        }
    }

    #[test]
    fn the_schema_is_strict() {
        let ok =
            br#"{"telemetry_off":{"hosts":["stats.example.com"],"beacon_paths":["/api/track"]},
            "jank":{"busy_timers":true,"hidden_animations":true},"heap_mb":1024,
            "background_throttle":true,"features":["webrtc"],"patches":[]}"#;
        let p = parse(ok).unwrap();
        assert_eq!(p.heap_mb, Some(1024));
        assert!(p.jank.busy_timers && p.background_throttle);
        assert_eq!(parse(b"{}").unwrap(), Policy::default());
        for bad in [
            &br#"{"heap":1}"#[..],
            br#"{"jank":{"busy":true}}"#,
            br#"{"heap_mb":64}"#,
            br#"{"features":["a b"]}"#,
            br#"{"telemetry_off":{"hosts":["x/y"]}}"#,
            br#"{"telemetry_off":{"beacon_paths":["track"]}}"#,
            b"not json",
        ] {
            assert!(parse(bad).is_err(), "{}", String::from_utf8_lossy(bad));
        }
        assert_eq!(parse(&vec![b' '; MAX_BYTES + 1]), Err(Error::TooLarge));
    }

    #[test]
    fn patches_are_checked_before_anything_is_read() {
        let o = b"a();track();b();";
        let good = patch("/usr/lib/app/main.js", o, 4, "track();", None);
        for (p, why) in [
            (
                Patch {
                    file: "/etc/passwd".into(),
                    ..good.clone()
                },
                "plain path",
            ),
            (
                Patch {
                    file: "/usr/lib/../etc/x".into(),
                    ..good.clone()
                },
                "plain path",
            ),
            (
                Patch {
                    file: "/usr/lib/a b".into(),
                    ..good.clone()
                },
                "plain path",
            ),
            (
                Patch {
                    sha256: "ABC".into(),
                    ..good.clone()
                },
                "sha256",
            ),
            (
                Patch {
                    find: String::new(),
                    ..good.clone()
                },
                "find",
            ),
            (
                Patch {
                    with: Some("x".into()),
                    ..good.clone()
                },
                "remove takes no with",
            ),
            (
                Patch {
                    op: Op::Replace,
                    ..good.clone()
                },
                "replace needs",
            ),
        ] {
            let pol = Policy {
                patches: vec![p],
                ..Policy::default()
            };
            let e = parse(&serde_json::to_vec(&pol).unwrap())
                .unwrap_err()
                .to_string();
            assert!(e.contains(why), "{e}");
        }
        let two = Policy {
            patches: vec![
                good.clone(),
                Patch {
                    offset: 6,
                    ..good.clone()
                },
            ],
            ..Policy::default()
        };
        assert!(
            parse(&serde_json::to_vec(&two).unwrap()).is_err(),
            "overlap"
        );
        let ok = Policy {
            patches: vec![good],
            ..Policy::default()
        };
        assert!(parse(&serde_json::to_vec(&ok).unwrap()).is_ok());
    }

    #[test]
    fn a_patch_applies_only_to_its_version_and_offset() {
        let o = b"a();track();b();";
        let rm = patch("/usr/lib/app/main.js", o, 4, "track();", None);
        assert_eq!(rm.apply(o).unwrap(), b"a();b();");
        let rep = patch("/usr/lib/app/main.js", o, 4, "track", Some("void"));
        assert_eq!(rep.apply(o).unwrap(), b"a();void();b();");
        assert!(
            rm.apply(b"a();track();b();c").is_err(),
            "the file moved: refused"
        );
        let off = Patch {
            offset: 3,
            ..rm.clone()
        };
        assert!(off.apply(o).is_err(), "bytes at the offset differ");
        let far = Patch {
            offset: u64::MAX,
            ..rm
        };
        assert!(far.apply(o).is_err());
    }

    #[test]
    fn plan_lines_are_one_word_per_field() {
        let o = "x = \"a b\";\n".as_bytes();
        let p = Policy {
            patches: vec![patch("/opt/app/x.js", o, 4, "\"a b\"", Some("\"\u{e9}\""))],
            ..Policy::default()
        };
        let text = plan(&p);
        let line = text.lines().nth(1).unwrap();
        let f: Vec<_> = line.split(' ').collect();
        assert_eq!(f.len(), 5, "{line}");
        assert_eq!(f[3], "22612062 22".replace(' ', ""));
        assert_eq!(f[4], "\\0042\\0303\\0251\\0042");
        assert!(
            plan(&Policy {
                patches: vec![patch("/opt/a", o, 0, "x", None)],
                ..Policy::default()
            })
            .ends_with(" 78 -\n")
        );
    }

    #[test]
    fn plans_run_backwards_per_file_and_hooks_name_the_files() {
        let o = b"one two three";
        let p = Policy {
            patches: vec![
                patch("/usr/lib/b/x.js", o, 0, "one", None),
                patch("/usr/lib/a/x.js", o, 0, "one", None),
                patch("/usr/lib/a/x.js", o, 8, "three", Some("3")),
            ],
            ..Policy::default()
        };
        let lines: Vec<(String, String)> = plan(&p)
            .lines()
            .skip(1)
            .map(|l| {
                let f: Vec<_> = l.split(' ').collect();
                (f[0].to_owned(), f[2].to_owned())
            })
            .collect();
        let want = [
            ("/usr/lib/a/x.js", "8"),
            ("/usr/lib/a/x.js", "0"),
            ("/usr/lib/b/x.js", "0"),
        ];
        assert_eq!(lines, want.map(|(a, b)| (a.to_owned(), b.to_owned())));
        let h = hook(&["/usr/lib/b/x.js", "/usr/lib/a/x.js", "/usr/lib/a/x.js"]);
        assert!(
            h.contains("Target = usr/lib/a/x.js\nTarget = usr/lib/b/x.js\n"),
            "{h}"
        );
        assert!(h.ends_with("Exec = /usr/lib/wolos/webpolicy\n"));
    }

    #[test]
    fn flag_rows_follow_the_policy() {
        let none = Policy::default();
        let all = Policy {
            heap_mb: Some(65536),
            background_throttle: true,
            ..Policy::default()
        };
        let small = Policy {
            heap_mb: Some(512),
            ..Policy::default()
        };
        for row in ROWS {
            if row.status == webruntime::Status::Baseline {
                assert!(allows(&none, row), "{}", row.id);
            }
            if let Some(mb) = heap_mb(row) {
                assert!(!allows(&none, row) && allows(&all, row), "{}", row.id);
                assert_eq!(allows(&small, row), mb <= 512, "{}", row.id);
            }
            if matches!(row.knob, Knob::Feature(_)) {
                assert!(!allows(&none, row) && allows(&all, row), "{}", row.id);
            }
        }
        assert!(
            ROWS.iter().any(|r| heap_mb(r).is_some()),
            "the heap rows parse"
        );
    }

    #[test]
    fn state_lines_round_trip_what_the_helper_writes() {
        let h = "a".repeat(64);
        let text = format!(
            "patched /usr/lib/a.js {h}\n\
                refused /usr/lib/b.js 1762180000 {h} {} not the version the patch was written for\n\
                refused /etc/x 1 {h} - not under /usr/lib or /opt\ngarbage\npatched /x short\n",
            "b".repeat(64)
        );
        let s = parse_state(&text);
        assert_eq!(s.len(), 3, "{s:?}");
        assert_eq!(
            s[0],
            StateLine::Patched {
                file: "/usr/lib/a.js".into(),
                sha256: h.clone()
            }
        );
        assert!(
            matches!(&s[1], StateLine::Refused { time: 1762180000, reason, found: Some(_), .. }
            if reason == "not the version the patch was written for")
        );
        assert!(matches!(&s[2], StateLine::Refused { found: None, .. }));
    }

    /// The browser's managed-policy rows come from helium.json alone: throttling only with
    /// `background_throttle`, and a shipped row the policy refuses never reaches the file.
    #[test]
    fn browser_rows_come_from_the_browser_policy() {
        let p = parse(HELIUM_JSON.as_bytes()).unwrap();
        assert_eq!(
            browser_policies(&p),
            webruntime::browser_policies(&|row| allows(&p, row))
        );
        let off = Policy::default();
        for row in ROWS {
            if let Knob::Policy(k, _) = row.knob
                && k.contains("WakeUp")
            {
                assert!(!allows(&off, row), "{}", row.id);
            }
        }
        assert!(crate::theme::helium::policies().contains("MetricsReportingEnabled"));
    }

    #[test]
    fn ids_are_file_stems() {
        for ok in ["code", "electron", "org.wolos.notes", "a_b-1"] {
            assert!(valid_id(ok), "{ok}");
        }
        for bad in ["", ".hidden", "../x", "A", "a/b", &"x".repeat(65)] {
            assert!(!valid_id(bad), "{bad}");
        }
        let d = std::env::temp_dir().join(format!("wol-webpolicy-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        assert!(load(&d, "nothing").is_none());
        assert_eq!(load(&d, "../x").map(|r| r.is_err()), Some(true));
        std::fs::write(d.join("x.json"), "{}").unwrap();
        assert_eq!(load(&d, "x"), Some(Ok(Policy::default())));
        std::fs::remove_dir_all(&d).ok();
    }

    /// Every shipped policy parses, has its plan next to it (`WOL_REGEN=1` rewrites the plans),
    /// and the three role policies exist.
    #[test]
    fn shipped_policies_parse_and_ship_their_plans() {
        let dir = shipped("");
        let mut ids = Vec::new();
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if let Some(id) = n.strip_suffix(".json") {
                assert!(valid_id(id), "{n}");
                let p = load(&dir, id)
                    .unwrap()
                    .unwrap_or_else(|e| panic!("{n}: {e}"));
                let plan_path = dir.join(format!("{id}.plan"));
                if std::env::var_os("WOL_REGEN").is_some() {
                    std::fs::write(&plan_path, plan(&p)).unwrap();
                }
                assert_eq!(
                    std::fs::read_to_string(&plan_path).ok(),
                    Some(plan(&p)),
                    "{id}.plan"
                );
                ids.push(id.to_owned());
            } else {
                assert!(
                    n.ends_with(".plan") && dir.join(n.replace(".plan", ".json")).exists(),
                    "{n}"
                );
            }
        }
        for r in Role::ALL {
            assert!(ids.iter().any(|i| i == role_id(r)), "{r:?}");
        }
        // The hook names every patched file.
        let pols: Vec<Policy> = ids
            .iter()
            .map(|i| load(&dir, i).unwrap().unwrap())
            .collect();
        let targets: Vec<&str> = pols
            .iter()
            .flat_map(|p| p.patches.iter().map(|x| x.file.as_str()))
            .collect();
        let hook_path = shipped("../../libalpm/hooks/wolos-webpolicy.hook");
        if std::env::var_os("WOL_REGEN").is_some() {
            std::fs::write(&hook_path, hook(&targets)).unwrap();
        }
        assert_eq!(
            std::fs::read_to_string(&hook_path).ok(),
            Some(hook(&targets))
        );
        // The session files are the role policies applied to the table.
        for r in Role::ALL {
            let p = load(&dir, role_id(r)).unwrap().unwrap();
            for prof in Profile::ALL {
                for g in Gpu::ALL {
                    for l in webruntime::flags_with(prof, r, g, &|row| allows(&p, row)) {
                        assert!(webruntime::flags(prof, r, g).contains(&l), "{l}");
                    }
                }
            }
        }
    }
}
