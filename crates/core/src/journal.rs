//! Settings journal: every user settings change records the files' text before and after, so any
//! change can be undone (`wol undo`, `wol history`, Settings > Restore). Writers: Settings,
//! Control Center, `wol`, helpers (`wol mcp`). One JSON object per line in
//! `$XDG_STATE_HOME/wolos/journal.jsonl`, newest last, at most [`MAX_ENTRIES`].
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

/// Relative to the state home.
pub const FILE: &str = "wolos/journal.jsonl";
pub const MAX_ENTRIES: usize = 200;
/// Larger files are written but not journaled (settings files are a few hundred bytes).
pub const MAX_TEXT: usize = 64 * 1024;
/// `what` of helper permission changes: helpers can never undo these.
pub const HELPERS: &str = "helpers";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Change {
    /// Settings file relative to the config home; `None` = no file.
    File {
        path: String,
        before: Option<String>,
        after: Option<String>,
    },
    /// User file moved (Downloads tidy): absolute paths.
    Move { from: String, to: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: u64,
    /// Unix seconds.
    pub t: u64,
    /// Actor: `settings`, `control-center`, `cli`, `script:<name>`, `mcp:<client>`.
    pub by: String,
    /// Topic: `theme`, `wallpaper`, `focus`, `shell`, `keyboard`, `displays`, `idle`, `helpers`, `files`.
    pub what: String,
    pub changes: Vec<Change>,
    /// Set on undo entries: the entry this one reverted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undoes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UndoError {
    /// No change left to undo.
    Nothing,
    NotFound(u64),
    /// This file or move target changed after the entry; undo the newer change first.
    Changed(String),
    Refused(&'static str),
    Io(String),
}

impl std::fmt::Display for UndoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Nothing => write!(f, "nothing to undo"),
            Self::NotFound(id) => write!(f, "no change #{id}"),
            Self::Changed(p) => write!(f, "{p} changed since; undo the newer change first"),
            Self::Refused(why) => write!(f, "{why}"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

/// Helper actors (`wol mcp` clients) are bound by the helper policy.
pub fn is_helper(by: &str) -> bool {
    by.starts_with("mcp:")
}

/// `$XDG_STATE_HOME`, else `~/.local/state`.
pub fn state_home() -> PathBuf {
    match std::env::var_os("XDG_STATE_HOME") {
        Some(x) if !x.is_empty() => x.into(),
        _ => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state"),
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// What a running session reloads after a change was undone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reload {
    /// Theme id now active (None = theme file removed: follow appearance).
    pub theme: Option<Option<String>>,
    /// wol-shell rereads shell.json / wallpaper (`reload` on wol-shell.sock).
    pub shell: bool,
    /// Hyprland fragments changed (`hyprctl reload`).
    pub hyprland: bool,
}

pub fn reload_for(e: &Entry) -> Reload {
    let mut r = Reload::default();
    for c in &e.changes {
        let Change::File { path, after, .. } = c else {
            continue;
        };
        if path == shared::theme::ACTIVE_FILE {
            r.theme = Some(after.as_deref().map(|t| t.trim().to_owned()));
        } else if path.starts_with("hypr/") {
            r.hyprland = true;
        } else {
            r.shell = true;
        }
    }
    r
}

fn valid_rel(p: &str) -> bool {
    let p = Path::new(p);
    !p.as_os_str().is_empty()
        && p.is_relative()
        && p.components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension(format!("wol-tmp-{}", std::process::id()));
    std::fs::write(&tmp, text)?;
    std::fs::rename(tmp, path)
}

fn put(path: &Path, text: Option<&str>) -> io::Result<()> {
    match text {
        Some(t) => write_atomic(path, t),
        None => match std::fs::remove_file(path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
    }
}

/// Move without ever replacing an existing file (hard link + unlink; same file system).
pub fn move_no_clobber(from: &Path, to: &Path) -> io::Result<()> {
    if let Some(d) = to.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::hard_link(from, to)?;
    std::fs::remove_file(from)
}

/// Opens (creating, mode 0600) `path` for read + append and takes an exclusive flock, released
/// when the file closes. Shared by the journal and the helper audit log.
pub(crate) fn open_locked(path: &Path) -> io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::AsRawFd;
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let f = std::fs::OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .mode(0o600)
        .open(path)?;
    // SAFETY: flock on an fd this function owns; the lock is released when `f` closes.
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(f)
}

/// The journal file plus the config home its relative paths resolve against.
#[derive(Clone, Debug)]
pub struct Journal {
    file: PathBuf,
    config_home: PathBuf,
}

/// Exclusive lock on the journal file for one read-modify-write (flock; released on drop).
struct Locked(std::fs::File);

impl Journal {
    pub fn new(state_home: &Path, config_home: &Path) -> Self {
        Self {
            file: state_home.join(FILE),
            config_home: config_home.to_owned(),
        }
    }

    /// The session user's journal.
    pub fn user() -> Self {
        Self::new(&state_home(), &crate::theme::config_home())
    }

    /// Journal for settings under `config_home`: the user's own for the user's config home, else
    /// `<config_home>/.wol-state` (a temporary home in a test never writes the user's journal).
    pub fn for_config(config_home: &Path) -> Self {
        if config_home == crate::theme::config_home() {
            Self::user()
        } else {
            Self::new(&config_home.join(".wol-state"), config_home)
        }
    }

    pub fn config_home(&self) -> &Path {
        &self.config_home
    }

    fn lock(&self) -> io::Result<Locked> {
        open_locked(&self.file).map(Locked)
    }

    /// Every entry, oldest first; malformed lines skipped.
    pub fn entries(&self) -> Vec<Entry> {
        parse(&std::fs::read_to_string(&self.file).unwrap_or_default())
    }

    fn append(&self, l: &Locked, mut e: Entry) -> io::Result<u64> {
        use std::io::{Read, Seek, Write};
        let mut f = &l.0;
        let mut text = String::new();
        f.seek(io::SeekFrom::Start(0))?;
        f.read_to_string(&mut text)?;
        let mut all = parse(&text);
        e.id = all.last().map_or(1, |x| x.id + 1);
        let id = e.id;
        all.push(e);
        let keep = all.len().saturating_sub(MAX_ENTRIES);
        let lines: String = all[keep..].iter().map(line).collect();
        if keep == 0 {
            f.write_all(line(all.last().expect("pushed")).as_bytes())?;
        } else {
            l.0.set_len(0)?;
            f.write_all(lines.as_bytes())?;
        }
        Ok(id)
    }

    /// Writes settings files (`Some` = new text, `None` = remove) and records one entry.
    /// Paths are relative to the config home. Unchanged files = no entry (`Ok(None)`).
    pub fn save(
        &self,
        what: &str,
        by: &str,
        files: &[(&str, Option<&str>)],
        now: u64,
    ) -> io::Result<Option<u64>> {
        if let Some((p, _)) = files.iter().find(|(p, _)| !valid_rel(p)) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("not a settings path: {p}"),
            ));
        }
        let l = self.lock()?;
        let mut changes = Vec::new();
        let mut journaled = true;
        for (rel, after) in files {
            let path = self.config_home.join(rel);
            let before = std::fs::read_to_string(&path).ok();
            if before.as_deref() == *after {
                continue;
            }
            put(&path, *after)?;
            journaled &= before.as_ref().map_or(0, String::len) <= MAX_TEXT
                && after.map_or(0, str::len) <= MAX_TEXT;
            changes.push(Change::File {
                path: (*rel).to_owned(),
                before,
                after: after.map(str::to_owned),
            });
        }
        if changes.is_empty() || !journaled {
            return Ok(None);
        }
        self.append(&l, entry(what, by, changes, now, None))
            .map(Some)
    }

    /// Like [`Journal::save`] for one absolute path, which must be inside the config home.
    pub fn save_path(
        &self,
        what: &str,
        by: &str,
        path: &Path,
        text: Option<&str>,
        now: u64,
    ) -> io::Result<Option<u64>> {
        let rel = path
            .strip_prefix(&self.config_home)
            .ok()
            .and_then(Path::to_str)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "{} is not under {}",
                        path.display(),
                        self.config_home.display()
                    ),
                )
            })?;
        self.save(what, by, &[(rel, text)], now)
    }

    /// Records moves the caller already made (absolute paths), so they can be moved back.
    pub fn record_moves(
        &self,
        what: &str,
        by: &str,
        moves: &[(PathBuf, PathBuf)],
        now: u64,
    ) -> io::Result<Option<u64>> {
        if moves.is_empty() {
            return Ok(None);
        }
        let changes = moves
            .iter()
            .map(|(f, t)| Change::Move {
                from: f.to_string_lossy().into_owned(),
                to: t.to_string_lossy().into_owned(),
            })
            .collect();
        let l = self.lock()?;
        self.append(&l, entry(what, by, changes, now, None))
            .map(Some)
    }

    /// What `wol undo` without an id reverts: the newest change not undone yet.
    pub fn undo_target(&self) -> Option<Entry> {
        latest_undoable(&self.entries()).cloned()
    }

    /// Reverts entry `id` (None = [`Journal::undo_target`]) and records the revert as a new entry.
    /// Undoing an undo entry redoes the change. Nothing is touched when any file changed since.
    pub fn undo(&self, id: Option<u64>, by: &str, now: u64) -> Result<Entry, UndoError> {
        let io = |e: io::Error| UndoError::Io(e.to_string());
        let l = self.lock().map_err(io)?;
        let all = self.entries();
        let target = match id {
            None => latest_undoable(&all).ok_or(UndoError::Nothing)?,
            Some(id) => all
                .iter()
                .find(|e| e.id == id)
                .ok_or(UndoError::NotFound(id))?,
        };
        if is_helper(by) && target.what == HELPERS {
            return Err(UndoError::Refused(
                "helpers cannot change their own permissions",
            ));
        }
        if id.is_some() && is_undone(&all, target.id) {
            return Err(UndoError::Refused("this change is already undone"));
        }
        let inverse: Vec<Change> = target.changes.iter().rev().map(invert).collect();
        for c in &inverse {
            self.check(c)?;
        }
        for c in &inverse {
            self.apply(c).map_err(io)?;
        }
        let e = entry(&target.what, by, inverse, now, Some(target.id));
        let id = self.append(&l, e.clone()).map_err(io)?;
        Ok(Entry { id, ..e })
    }

    /// Precondition for applying `c`: the file still holds `before` / the move source exists and
    /// the destination does not.
    fn check(&self, c: &Change) -> Result<(), UndoError> {
        match c {
            Change::File { path, before, .. } => {
                let cur = std::fs::read_to_string(self.config_home.join(path)).ok();
                (cur == *before)
                    .then_some(())
                    .ok_or_else(|| UndoError::Changed(path.clone()))
            }
            Change::Move { from, to } => {
                let ok = std::fs::symlink_metadata(from).is_ok_and(|m| m.is_file())
                    && std::fs::symlink_metadata(to).is_err();
                ok.then_some(())
                    .ok_or_else(|| UndoError::Changed(from.clone()))
            }
        }
    }

    fn apply(&self, c: &Change) -> io::Result<()> {
        match c {
            Change::File { path, after, .. } => put(&self.config_home.join(path), after.as_deref()),
            Change::Move { from, to } => move_no_clobber(Path::new(from), Path::new(to)),
        }
    }
}

fn entry(what: &str, by: &str, changes: Vec<Change>, t: u64, undoes: Option<u64>) -> Entry {
    Entry {
        id: 0,
        t,
        by: by.to_owned(),
        what: what.to_owned(),
        changes,
        undoes,
    }
}

fn line(e: &Entry) -> String {
    let mut s = serde_json::to_string(e).unwrap_or_default();
    s.push('\n');
    s
}

fn parse(text: &str) -> Vec<Entry> {
    text.lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// The change that applies `c` in reverse.
fn invert(c: &Change) -> Change {
    match c {
        Change::File {
            path,
            before,
            after,
        } => Change::File {
            path: path.clone(),
            before: after.clone(),
            after: before.clone(),
        },
        Change::Move { from, to } => Change::Move {
            from: to.clone(),
            to: from.clone(),
        },
    }
}

/// Undone = reverted by an undo entry that is itself not undone (redo).
pub fn is_undone(all: &[Entry], id: u64) -> bool {
    all.iter()
        .filter(|e| e.undoes == Some(id))
        .any(|e| !is_undone(all, e.id))
}

fn latest_undoable(all: &[Entry]) -> Option<&Entry> {
    all.iter()
        .rev()
        .find(|e| e.undoes.is_none() && !is_undone(all, e.id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> (PathBuf, Journal) {
        let d = std::env::temp_dir().join(format!("wol-journal-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let j = Journal::new(&d.join("state"), &d.join("config"));
        (d, j)
    }

    fn read(j: &Journal, rel: &str) -> Option<String> {
        std::fs::read_to_string(j.config_home().join(rel)).ok()
    }

    #[test]
    fn save_records_before_and_after_and_skips_no_ops() {
        let (_d, j) = temp("save");
        assert_eq!(
            j.save("theme", "cli", &[("wolos/theme", Some("wol-dark\n"))], 10)
                .unwrap(),
            Some(1)
        );
        assert_eq!(
            j.save("theme", "cli", &[("wolos/theme", Some("wol-dark\n"))], 11)
                .unwrap(),
            None,
            "unchanged file: no entry"
        );
        j.save("theme", "settings", &[("wolos/theme", Some("nord\n"))], 12)
            .unwrap();
        let e = j.entries();
        assert_eq!(e.len(), 2);
        assert_eq!(
            e[1].changes,
            [Change::File {
                path: "wolos/theme".into(),
                before: Some("wol-dark\n".into()),
                after: Some("nord\n".into())
            }]
        );
        assert_eq!((e[1].by.as_str(), e[1].t), ("settings", 12));
        assert_eq!(read(&j, "wolos/theme").as_deref(), Some("nord\n"));
    }

    #[test]
    fn undo_reverts_newest_then_older_and_undo_of_undo_redoes() {
        let (_d, j) = temp("undo");
        j.save("theme", "cli", &[("wolos/theme", Some("a\n"))], 1)
            .unwrap();
        j.save("focus", "cli", &[("wol/dnd", Some(""))], 2).unwrap();
        let u = j.undo(None, "cli", 3).unwrap();
        assert_eq!((u.id, u.undoes, u.what.as_str()), (3, Some(2), "focus"));
        assert_eq!(read(&j, "wol/dnd"), None, "undo removes a created file");
        assert_eq!(j.undo_target().map(|e| e.id), Some(1));
        j.undo(None, "cli", 4).unwrap();
        assert_eq!(read(&j, "wolos/theme"), None);
        assert_eq!(j.undo(None, "cli", 5), Err(UndoError::Nothing));
        // Redo #2 by undoing its undo (#3).
        j.undo(Some(3), "cli", 6).unwrap();
        assert_eq!(read(&j, "wol/dnd").as_deref(), Some(""));
        assert_eq!(
            j.undo(Some(3), "cli", 7),
            Err(UndoError::Refused("this change is already undone"))
        );
        assert_eq!(j.undo(Some(99), "cli", 7), Err(UndoError::NotFound(99)));
    }

    #[test]
    fn undo_never_overwrites_a_newer_change() {
        let (_d, j) = temp("conflict");
        j.save("theme", "cli", &[("wolos/theme", Some("a\n"))], 1)
            .unwrap();
        j.save("theme", "cli", &[("wolos/theme", Some("b\n"))], 2)
            .unwrap();
        assert_eq!(
            j.undo(Some(1), "cli", 3),
            Err(UndoError::Changed("wolos/theme".into()))
        );
        assert_eq!(read(&j, "wolos/theme").as_deref(), Some("b\n"));
        std::fs::write(j.config_home().join("wolos/theme"), "edited\n").unwrap();
        assert!(matches!(j.undo(None, "cli", 4), Err(UndoError::Changed(_))));
        assert_eq!(j.entries().len(), 2, "a refused undo records nothing");
    }

    #[test]
    fn helpers_cannot_undo_their_permission_changes() {
        let (_d, j) = temp("helpers");
        j.save(HELPERS, "cli", &[("wolos/helpers.json", Some("{}"))], 1)
            .unwrap();
        assert!(matches!(
            j.undo(None, "mcp:Claude", 2),
            Err(UndoError::Refused(_))
        ));
        assert!(j.undo(None, "cli", 2).is_ok());
    }

    #[test]
    fn bounded_to_max_entries_with_increasing_ids() {
        let (_d, j) = temp("bounded");
        for i in 0..MAX_ENTRIES + 5 {
            let v = format!("{i}\n");
            j.save("theme", "cli", &[("wolos/theme", Some(&v))], i as u64)
                .unwrap();
        }
        let e = j.entries();
        assert_eq!(e.len(), MAX_ENTRIES);
        assert_eq!(e.first().unwrap().id, 6);
        assert_eq!(e.last().unwrap().id, (MAX_ENTRIES + 5) as u64);
    }

    #[test]
    fn rejects_paths_outside_the_config_home() {
        let (d, j) = temp("paths");
        for bad in ["", "/etc/passwd", "../x", "wolos/../../x"] {
            assert!(j.save("x", "cli", &[(bad, Some("x"))], 1).is_err(), "{bad}");
        }
        assert!(
            j.save_path("x", "cli", Path::new("/etc/x"), Some("x"), 1)
                .is_err()
        );
        let p = d.join("config/wolos/wallpaper");
        assert_eq!(
            j.save_path("wallpaper", "cli", &p, Some("/w.png\n"), 1)
                .unwrap(),
            Some(1)
        );
    }

    #[test]
    fn moves_undo_without_clobbering() {
        let (d, j) = temp("moves");
        let (a, b) = (d.join("dl/a.png"), d.join("dl/Images/a.png"));
        std::fs::create_dir_all(d.join("dl")).unwrap();
        std::fs::write(&a, "png").unwrap();
        move_no_clobber(&a, &b).unwrap();
        j.record_moves("files", "cli", &[(a.clone(), b.clone())], 1)
            .unwrap();
        std::fs::write(&a, "new file with the old name").unwrap();
        assert!(matches!(j.undo(None, "cli", 2), Err(UndoError::Changed(_))));
        std::fs::remove_file(&a).unwrap();
        j.undo(None, "cli", 3).unwrap();
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "png");
        assert!(!b.exists());
        std::fs::write(&b, "x").unwrap();
        assert!(move_no_clobber(&a, &b).is_err(), "never replaces a file");
    }

    #[test]
    fn reload_hints_per_file() {
        let f = |p: &str, after: Option<&str>| Change::File {
            path: p.into(),
            before: None,
            after: after.map(String::from),
        };
        let e = |c: Vec<Change>| entry("x", "cli", c, 0, None);
        assert_eq!(
            reload_for(&e(vec![f("wolos/theme", Some("nord\n"))])).theme,
            Some(Some("nord".into()))
        );
        assert_eq!(
            reload_for(&e(vec![f("wolos/theme", None)])).theme,
            Some(None)
        );
        let r = reload_for(&e(vec![
            f("hypr/wolos/keyboard.conf", Some("")),
            f("wolos/shell.json", Some("{}")),
        ]));
        assert!(r.hyprland && r.shell && r.theme.is_none());
        assert!(is_helper("mcp:Claude") && !is_helper("cli"));
    }
}
