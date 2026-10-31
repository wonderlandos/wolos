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
    let bad = |_| Error::unavailable(format!("{} cannot be written.", path.display()));
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(bad)?;
    }
    let tmp = PathBuf::from(format!("{}.tmp-{}", path.display(), std::process::id()));
    let mut f = std::fs::File::create(&tmp).map_err(bad)?;
    f.write_all(bytes).and_then(|_| f.sync_all()).map_err(bad)?;
    std::fs::rename(&tmp, path).map_err(bad)
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

/// Append `e`; returns its id.
pub fn record(ctx: &Context, mut e: Entry) -> Result<String> {
    let all = list(ctx);
    let n = all.len() + 1;
    e.time = now();
    e.id = format!("{n}");
    let line = serde_json::to_string(&e)
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
