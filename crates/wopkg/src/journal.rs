// SPDX-License-Identifier: GPL-2.0-or-later
//! History: one JSON line per change in `<state>/history.jsonl`, written before the change is
//! reported done. Undo is either a file swap libwopkg does itself (AppImages) or a command the
//! caller runs (Flatpak commit, system restore slot).
use crate::context::Context;
use crate::ffi::{Error, Result};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Undo {
    /// Put `from` back at `to` (the previous file).
    Restore { from: PathBuf, to: PathBuf },
    /// Delete what was installed.
    Remove { path: PathBuf },
    /// Run this as the person (e.g. `flatpak update --user --commit=<old> <id>`).
    Run { argv: Vec<String> },
    /// A system change: undone by booting the restore slot (`wol update rollback`, through wold).
    System { note: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub time: u64,
    /// `appimage`, `flatpak`, `pacman`, `aur`, `recipe`, `wol`, `system`.
    pub source: String,
    /// `install`, `remove`, `update`, `rollback`.
    pub action: String,
    pub name: String,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub undo: Option<Undo>,
    #[serde(default)]
    pub undone: bool,
}

impl Entry {
    pub fn new(
        source: &str,
        action: &str,
        name: &str,
        from: Option<String>,
        to: Option<String>,
        undo: Option<Undo>,
    ) -> Entry {
        Entry {
            id: String::new(),
            time: 0,
            source: source.into(),
            action: action.into(),
            name: name.into(),
            from,
            to,
            undo,
            undone: false,
        }
    }
}

/// Write by temp file + fsync + rename.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    // The disk being full is named; no path, no system text (QA-STORE-24).
    let bad = |e: std::io::Error| {
        crate::fetch::write_error(&e, "The package manager's records cannot be written.")
    };
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(bad)?;
    }
    shared::atomic::atomic_write(path, 0o644, bytes).map_err(bad)
}

fn path(ctx: &Context) -> PathBuf {
    ctx.state.join("history.jsonl")
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn list(ctx: &Context) -> Vec<Entry> {
    std::fs::read_to_string(path(ctx))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn save(ctx: &Context, all: &[Entry]) -> Result<()> {
    let text: String = all
        .iter()
        .map(|e| serde_json::to_string(e).unwrap_or_default() + "\n")
        .collect();
    write_atomic(&path(ctx), text.as_bytes())
}

/// One above the highest id in the history: a torn or dropped line never makes two entries
/// share an id, so a rollback reaches the entry asked for (QA-STORE-18).
fn next_id(all: &[Entry]) -> String {
    let max = all
        .iter()
        .filter_map(|e| e.id.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    (max + 1).to_string()
}

/// Append `e`; returns its id.
pub fn record(ctx: &Context, mut e: Entry) -> Result<String> {
    let all = list(ctx);
    e.time = now();
    e.id = next_id(&all);
    // A last line a crash cut short gets its newline first, so this entry starts a line of its
    // own and is read back (QA-STORE-18).
    let torn = std::fs::read(path(ctx))
        .ok()
        .is_some_and(|b| b.last().is_some_and(|c| *c != b'\n'));
    let line = if torn { "\n" } else { "" }.to_owned()
        + &serde_json::to_string(&e)
            .map_err(|_| Error::internal("The history entry cannot be written."))?
        + "\n";
    std::fs::create_dir_all(&ctx.state)
        .map_err(|_| Error::unavailable("The history cannot be written."))?;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path(ctx))
        .map_err(|_| Error::unavailable("The history cannot be written."))?;
    f.write_all(line.as_bytes())
        .and_then(|_| f.sync_data())
        .map_err(|_| Error::unavailable("The history cannot be written."))?;
    Ok(e.id)
}

/// Undo entry `id`: file undos are done here; `Run`/`System` are returned for the caller.
pub fn rollback(ctx: &Context, id: &str) -> Result<Entry> {
    let mut all = list(ctx);
    let e = all.iter_mut().find(|e| e.id == id).ok_or_else(|| {
        Error::new(
            crate::ffi::Status::NotFound,
            format!("There is no change {id} in the history."),
        )
    })?;
    if e.undone {
        return Err(Error::invalid(format!("Change {id} was already undone.")));
    }
    match &e.undo {
        Some(Undo::Restore { from, to }) => {
            std::fs::rename(from, to)
                .map_err(|_| Error::unavailable("The previous version is no longer there."))?;
            if e.source == "appimage" {
                let mut reg = crate::appimage::Registry::load(ctx);
                if let Some(a) = reg.apps.iter_mut().find(|a| &a.path == to)
                    && let Some(f) = &e.from
                {
                    a.sha256 = f.clone();
                }
                reg.save(ctx)?;
            }
        }
        Some(Undo::Remove { path }) => {
            let _ = std::fs::remove_file(path);
            if e.source == "appimage" {
                let mut reg = crate::appimage::Registry::load(ctx);
                reg.apps.retain(|a| &a.path != path);
                reg.save(ctx)?;
            }
        }
        Some(Undo::Run { .. }) | Some(Undo::System { .. }) => {}
        None => return Err(Error::invalid(format!("Change {id} cannot be undone."))),
    }
    e.undone = true;
    let out = e.clone();
    save(ctx, &all)?;
    record(
        ctx,
        Entry::new(
            &out.source,
            "rollback",
            &out.name,
            out.to.clone(),
            out.from.clone(),
            None,
        ),
    )?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// QA-STORE-18: a crash mid-write left a last line without its newline; the next entry is
    /// still read back, and ids never repeat (the torn one's number is not handed out again).
    #[test]
    fn a_torn_tail_never_swallows_the_next_entry() {
        let d = crate::testutil::temp("journal-torn");
        let c = crate::appimage::tests::ctx(&d);
        let e = || Entry::new("flatpak", "install", "org.x", None, None, None);
        assert_eq!(record(&c, e()).unwrap(), "1");
        assert_eq!(record(&c, e()).unwrap(), "2");
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(path(&c))
            .unwrap();
        f.write_all(br#"{"id":"3","kind":"flat"#).unwrap();
        drop(f);
        let id = record(&c, e()).unwrap();
        assert_eq!(id, "3");
        let all = list(&c);
        assert_eq!(all.len(), 3, "{all:?}");
        assert_eq!(all.last().unwrap().id, "3");
        // An entry dropped from the middle does not make the next id collide with the last.
        let text = std::fs::read_to_string(path(&c)).unwrap();
        let kept: String = text
            .lines()
            .filter(|l| !l.contains("\"id\":\"1\""))
            .map(|l| format!("{l}\n"))
            .collect();
        std::fs::write(path(&c), kept).unwrap();
        assert_eq!(record(&c, e()).unwrap(), "4");
    }

    #[test]
    fn record_list_rollback() {
        let d = crate::testutil::temp("journal");
        let c = crate::appimage::tests::ctx(&d);
        std::fs::write(d.join("cur"), b"new").unwrap();
        std::fs::write(d.join("prev"), b"old").unwrap();
        let id = record(
            &c,
            Entry::new(
                "appimage",
                "update",
                "x",
                Some("a".into()),
                Some("b".into()),
                Some(Undo::Restore {
                    from: d.join("prev"),
                    to: d.join("cur"),
                }),
            ),
        )
        .unwrap();
        let id2 = record(
            &c,
            Entry::new(
                "flatpak",
                "update",
                "org.x",
                None,
                None,
                Some(Undo::Run {
                    argv: vec!["flatpak".into()],
                }),
            ),
        )
        .unwrap();
        assert_eq!((id.as_str(), id2.as_str()), ("1", "2"));
        rollback(&c, &id).unwrap();
        assert_eq!(std::fs::read(d.join("cur")).unwrap(), b"old");
        assert!(rollback(&c, &id).is_err(), "only once");
        let e = rollback(&c, &id2).unwrap();
        assert!(
            matches!(e.undo, Some(Undo::Run { .. })),
            "the caller runs it"
        );
        let all = list(&c);
        assert_eq!(all.len(), 4);
        assert_eq!(all[3].action, "rollback");
        assert!(rollback(&c, "99").is_err());
    }
}
