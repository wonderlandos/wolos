// SPDX-License-Identifier: BSD-2-Clause
//! Joining untrusted path text under a trusted root. [`safe_join`] rejects absolute paths, `..`,
//! NUL bytes and any result that resolves (through symlinks) outside the root. [`Untrusted`]
//! marks text that came from a peer so it cannot be used as a path without going through here.

use std::fmt;
use std::io;
use std::path::{Component, Path, PathBuf};

/// Text received from an untrusted peer (IPC, package metadata, plugin manifest).
/// The only way to turn it into a filesystem path is [`safe_join`].
pub struct Untrusted<T: ?Sized>(Box<T>);

impl Clone for Untrusted<str> {
    fn clone(&self) -> Self {
        Untrusted(self.0.clone())
    }
}

impl PartialEq for Untrusted<str> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for Untrusted<str> {}

impl Untrusted<str> {
    /// Wraps peer-supplied text.
    pub fn new(s: impl Into<String>) -> Self {
        Untrusted(s.into().into_boxed_str())
    }

    /// The raw text, for logging or comparison only. Do not build paths from it.
    pub fn as_unchecked_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Untrusted<str> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Untrusted({:?})", self.0)
    }
}

/// Why a join was refused.
#[derive(Debug)]
pub enum SafePathError {
    Empty,
    Nul,
    Absolute,
    ParentDir,
    Escapes,
    Io(io::Error),
}

impl fmt::Display for SafePathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SafePathError::Empty => f.write_str("path is empty"),
            SafePathError::Nul => f.write_str("path contains a NUL byte"),
            SafePathError::Absolute => f.write_str("path must be relative"),
            SafePathError::ParentDir => f.write_str("path must not contain '..'"),
            SafePathError::Escapes => f.write_str("path resolves outside its root"),
            SafePathError::Io(e) => write!(f, "cannot resolve path: {e}"),
        }
    }
}

impl std::error::Error for SafePathError {}

/// Checks the text alone (no filesystem access): non-empty, no NUL, relative, no `..`, no
/// prefix/root components. Returns the normalized relative path.
pub fn lexical(untrusted: &str) -> Result<PathBuf, SafePathError> {
    if untrusted.is_empty() {
        return Err(SafePathError::Empty);
    }
    if untrusted.contains('\0') {
        return Err(SafePathError::Nul);
    }
    let p = Path::new(untrusted);
    if p.is_absolute() || untrusted.starts_with('/') || untrusted.starts_with('\\') {
        return Err(SafePathError::Absolute);
    }
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::Normal(n) => out.push(n),
            Component::CurDir => {}
            Component::ParentDir => return Err(SafePathError::ParentDir),
            Component::RootDir | Component::Prefix(_) => return Err(SafePathError::Absolute),
        }
    }
    if out.as_os_str().is_empty() {
        return Err(SafePathError::Empty);
    }
    Ok(out)
}

/// Joins `untrusted` under `root`. The root is canonicalized; the deepest existing ancestor of
/// the joined path is canonicalized too and must stay under the root, so a symlink inside the
/// root that points out is refused. Non-existing tail components are allowed (for creating
/// files), they cannot be symlinks yet. The caller should still open with `O_NOFOLLOW` where a
/// race with a writer of `root` matters.
pub fn safe_join(root: &Path, untrusted: &Untrusted<str>) -> Result<PathBuf, SafePathError> {
    safe_join_str(root, untrusted.as_unchecked_str())
}

/// [`safe_join`] for a plain `&str` (same checks).
pub fn safe_join_str(root: &Path, untrusted: &str) -> Result<PathBuf, SafePathError> {
    let rel = lexical(untrusted)?;
    let root = root.canonicalize().map_err(SafePathError::Io)?;
    let joined = root.join(&rel);
    // Walk up to the deepest component that exists and resolve it.
    let mut existing = joined.clone();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    loop {
        match std::fs::symlink_metadata(&existing) {
            Ok(_) => break,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let Some(name) = existing.file_name().map(|n| n.to_os_string()) else {
                    return Err(SafePathError::Io(e));
                };
                tail.push(name);
                if !existing.pop() {
                    return Err(SafePathError::Io(e));
                }
            }
            Err(e) => return Err(SafePathError::Io(e)),
        }
    }
    let mut resolved = existing.canonicalize().map_err(SafePathError::Io)?;
    if !resolved.starts_with(&root) {
        return Err(SafePathError::Escapes);
    }
    for name in tail.into_iter().rev() {
        resolved.push(name);
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wolos-safe-path-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).ok();
        d
    }

    #[test]
    fn rejects_lexical_attacks() {
        assert!(matches!(lexical(""), Err(SafePathError::Empty)));
        assert!(matches!(lexical("."), Err(SafePathError::Empty)));
        assert!(matches!(
            lexical("/etc/passwd"),
            Err(SafePathError::Absolute)
        ));
        assert!(matches!(
            lexical("a/../../b"),
            Err(SafePathError::ParentDir)
        ));
        assert!(matches!(lexical(".."), Err(SafePathError::ParentDir)));
        assert!(matches!(lexical("a\0b"), Err(SafePathError::Nul)));
        assert_eq!(lexical("./a/./b").ok(), Some(PathBuf::from("a/b")));
    }

    #[test]
    fn joins_inside_root() {
        let root = tmp("inside");
        std::fs::create_dir_all(root.join("sub")).ok();
        let p = safe_join(&root, &Untrusted::new("sub/new.txt"));
        let canon = root.canonicalize().ok();
        assert!(p.is_ok(), "{p:?}");
        assert!(canon.is_some_and(|c| p.is_ok_and(|p| p.starts_with(c))));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape() {
        let root = tmp("escape");
        let outside = tmp("escape-outside");
        assert!(std::os::unix::fs::symlink(&outside, root.join("link")).is_ok());
        let r = safe_join(&root, &Untrusted::new("link/x"));
        assert!(matches!(r, Err(SafePathError::Escapes)), "{r:?}");
        // A symlink that stays inside is fine.
        std::fs::create_dir_all(root.join("real")).ok();
        assert!(std::os::unix::fs::symlink(root.join("real"), root.join("ok")).is_ok());
        assert!(safe_join(&root, &Untrusted::new("ok/y")).is_ok());
    }

    #[test]
    fn debug_marks_untrusted() {
        assert_eq!(format!("{:?}", Untrusted::new("a")), "Untrusted(\"a\")");
    }
}
