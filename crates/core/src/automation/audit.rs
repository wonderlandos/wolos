//! Helper audit log: one JSON object per action in `$XDG_STATE_HOME/wolos/actions.jsonl`, newest
//! last. Secrets (passwords) are never recorded. Bounded: past [`MAX_BYTES`] the newest half stays.
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Relative to the state home.
pub const FILE: &str = "wolos/actions.jsonl";
pub const MAX_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    Ok,
    Failed,
    /// Turned away by the helper policy (or a dry-run rule).
    Refused,
    /// Sent to the screen for the user's administrator password; nothing changed yet.
    ApprovalNeeded,
    Approved,
    Declined,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub t: u64,
    /// MCP client name, `script:<name>`, or `cli`.
    pub helper: String,
    /// `mcp`, `cli` or `shell` (approval sheet).
    pub via: String,
    /// Catalogue verb, e.g. `appearance set`.
    pub tool: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    pub result: Outcome,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

fn line(r: &Record) -> String {
    let mut s = serde_json::to_string(r).unwrap_or_default();
    s.push('\n');
    s
}

/// Appends one record under the file lock; trims to the newest half past [`MAX_BYTES`].
pub fn append(state_home: &Path, r: &Record) -> io::Result<()> {
    use std::io::{Read, Seek, Write};
    let path = state_home.join(FILE);
    let mut f = crate::journal::open_locked(&path)?;
    let l = line(r);
    let len = f.metadata()?.len() as usize;
    if len + l.len() <= MAX_BYTES {
        return f.write_all(l.as_bytes());
    }
    let mut text = String::new();
    f.seek(io::SeekFrom::Start(0))?;
    f.read_to_string(&mut text)?;
    let cut = text.len().saturating_sub(MAX_BYTES / 2);
    let start = text[cut..].find('\n').map_or(text.len(), |i| cut + i + 1);
    let kept = format!("{}{l}", &text[start..]);
    f.set_len(0)?;
    f.write_all(kept.as_bytes())
}

/// Every readable record, oldest first.
pub fn read(state_home: &Path) -> Vec<Record> {
    std::fs::read_to_string(state_home.join(FILE))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// One helper as Settings lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelperSeen {
    pub name: String,
    pub via: String,
    pub first: u64,
    pub last: u64,
    pub actions: u32,
    pub refused: u32,
}

/// Helpers in `records`, most recently seen first.
pub fn helpers(records: &[Record]) -> Vec<HelperSeen> {
    let mut v: Vec<HelperSeen> = Vec::new();
    for r in records {
        let h = match v.iter_mut().find(|h| h.name == r.helper) {
            Some(h) => h,
            None => {
                v.push(HelperSeen {
                    name: r.helper.clone(),
                    via: r.via.clone(),
                    first: r.t,
                    last: r.t,
                    actions: 0,
                    refused: 0,
                });
                v.last_mut().expect("pushed")
            }
        };
        h.first = h.first.min(r.t);
        h.last = h.last.max(r.t);
        h.actions += 1;
        if matches!(r.result, Outcome::Refused | Outcome::Declined) {
            h.refused += 1;
        }
    }
    v.sort_by(|a, b| b.last.cmp(&a.last).then(a.name.cmp(&b.name)));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(t: u64, helper: &str, result: Outcome) -> Record {
        Record {
            t,
            helper: helper.into(),
            via: "mcp".into(),
            tool: "theme get".into(),
            args: vec![],
            result,
            detail: String::new(),
        }
    }

    #[test]
    fn appends_reads_and_summarises_helpers() {
        let d = std::env::temp_dir().join(format!("wol-audit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        append(&d, &rec(1, "Claude", Outcome::Ok)).unwrap();
        append(&d, &rec(2, "llama", Outcome::Refused)).unwrap();
        append(&d, &rec(3, "Claude", Outcome::ApprovalNeeded)).unwrap();
        let r = read(&d);
        assert_eq!(r.len(), 3);
        assert_eq!(r[2].result, Outcome::ApprovalNeeded);
        let h = helpers(&r);
        assert_eq!(
            h.iter()
                .map(|h| (h.name.as_str(), h.first, h.last, h.actions, h.refused))
                .collect::<Vec<_>>(),
            [("Claude", 1, 3, 2, 0), ("llama", 2, 2, 1, 1)]
        );
        let text = std::fs::read_to_string(d.join(FILE)).unwrap();
        assert!(text.contains(r#""result":"approval-needed""#), "{text}");
    }

    #[test]
    fn bounded_keeps_the_newest_records() {
        let d = std::env::temp_dir().join(format!("wol-audit-big-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let mut r = rec(0, "Claude", Outcome::Ok);
        r.detail = "x".repeat(1000);
        let n = MAX_BYTES / 1000 + 50;
        for t in 0..n as u64 {
            r.t = t;
            append(&d, &r).unwrap();
        }
        let size = std::fs::metadata(d.join(FILE)).unwrap().len() as usize;
        assert!(size <= MAX_BYTES, "{size}");
        let all = read(&d);
        assert_eq!(all.last().unwrap().t, n as u64 - 1);
        assert!(all.len() > 50 && all.windows(2).all(|w| w[0].t + 1 == w[1].t));
    }
}
