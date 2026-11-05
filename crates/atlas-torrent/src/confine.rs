// SPDX-License-Identifier: BSD-2-Clause
//! Path confinement: every name a torrent carries becomes a path under its job folder and nowhere
//! else. Checked twice: on the names ([`component`], [`relative`]) before the engine starts, and on
//! the disk ([`under`]) before a job folder is used: no part of it may be a symbolic link and it must
//! stay below the downloads folder. The helper's service unit is the third fence (only the
//! downloads folder and its own state folder are writable: [`crate::sandbox`]).
use std::path::{Component, Path, PathBuf};

/// Longest name of one file or folder (bytes, as most Linux file systems allow).
pub const MAX_NAME: usize = 255;
/// Deepest folder nesting inside a torrent.
pub const MAX_DEPTH: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathError {
    Empty,
    Dot,
    Separator,
    Nul,
    TooLong,
    TooDeep,
    NotText,
    Outside,
    Link,
    /// A job folder name starting with `.` (a hidden file or folder: `.ssh`, `.bash_profile`).
    Hidden,
}

/// Unicode format characters that change how a name reads (right-to-left override makes
/// `exe.fdp` show as `pdf.exe`; zero-width spaces hide letters): never in a name (DL-SEC-12).
fn is_format_char(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{061C}' | '\u{180E}'
        | '\u{200B}'..='\u{200F}'
        | '\u{202A}'..='\u{202E}'
        | '\u{2060}'..='\u{2064}'
        | '\u{2066}'..='\u{2069}'
        | '\u{FEFF}'
        | '\u{FFF9}'..='\u{FFFB}')
}

/// One path component from a torrent: UTF-8, not empty, not `.` or `..`, no `/`, `\` or NUL, at
/// most [`MAX_NAME`] bytes, no control characters and no Unicode format characters.
pub fn component(b: &[u8]) -> Result<String, PathError> {
    if b.is_empty() {
        return Err(PathError::Empty);
    }
    if b.len() > MAX_NAME {
        return Err(PathError::TooLong);
    }
    if b.contains(&0) {
        return Err(PathError::Nul);
    }
    if b.contains(&b'/') || b.contains(&b'\\') {
        return Err(PathError::Separator);
    }
    if b == b"." || b == b".." {
        return Err(PathError::Dot);
    }
    let s = std::str::from_utf8(b).map_err(|_| PathError::NotText)?;
    if s.chars().any(|c| c.is_control() || is_format_char(c)) {
        return Err(PathError::NotText);
    }
    Ok(s.to_string())
}

/// A file's path inside a torrent (a list of components).
pub fn relative(comps: &[&[u8]]) -> Result<Vec<String>, PathError> {
    if comps.is_empty() {
        return Err(PathError::Empty);
    }
    if comps.len() > MAX_DEPTH {
        return Err(PathError::TooDeep);
    }
    comps.iter().map(|c| component(c)).collect()
}

/// `root/rel...` when it stays under `root`, and no existing part of it below `root` is a
/// symbolic link (checked with `symlink_metadata`, so a link is seen, not followed).
pub fn under(root: &Path, rel: &[String]) -> Result<PathBuf, PathError> {
    if !root.is_absolute() {
        return Err(PathError::Outside);
    }
    let mut p = root.to_path_buf();
    for c in rel {
        component(c.as_bytes())?;
        p.push(c);
        if let Ok(m) = std::fs::symlink_metadata(&p)
            && m.file_type().is_symlink()
        {
            return Err(PathError::Link);
        }
    }
    if p.components()
        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        || !p.starts_with(root)
    {
        return Err(PathError::Outside);
    }
    Ok(p)
}

/// A folder name for a new job under `root`: the torrent's name, or `name (2)`, `name (3)`... when
/// that is taken, so a torrent never writes into a folder or over a file that was already there.
/// A name starting with `.` is refused: the job folder is always visible, so a torrent cannot
/// plant `.ssh` or `.bash_profile` when the downloads folder is a home folder (DL-SEC-05).
pub fn fresh_folder(root: &Path, name: &str) -> Result<PathBuf, PathError> {
    let name = component(name.as_bytes())?;
    if name.starts_with('.') {
        return Err(PathError::Hidden);
    }
    for n in 1..1000u32 {
        let cand = if n == 1 {
            name.clone()
        } else {
            format!("{name} ({n})")
        };
        let cand = if cand.len() > MAX_NAME {
            format!("{} ({n})", &name[..name.floor_char_boundary(MAX_NAME - 8)])
        } else {
            cand
        };
        let p = under(root, &[cand])?;
        if std::fs::symlink_metadata(&p).is_err() {
            return Ok(p);
        }
    }
    Err(PathError::TooLong)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("atlas-torrent-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn every_component_is_checked_on_its_own() {
        assert_eq!(component(b"a b.txt").unwrap(), "a b.txt");
        assert_eq!(component(b""), Err(PathError::Empty));
        assert_eq!(component(b".."), Err(PathError::Dot));
        assert_eq!(component(b"."), Err(PathError::Dot));
        assert_eq!(component(b"a/b"), Err(PathError::Separator));
        assert_eq!(component(b"a\\b"), Err(PathError::Separator));
        assert_eq!(component(b"a\0b"), Err(PathError::Nul));
        assert_eq!(component(b"a\nb"), Err(PathError::NotText));
        assert_eq!(component(&[0xff, 0xfe]), Err(PathError::NotText));
        assert_eq!(component(&[b'a'; 256]), Err(PathError::TooLong));
        assert!(component(&[b'a'; 255]).is_ok());
        // DL-SEC-12: names that read differently than they are
        assert_eq!(component("a\u{202E}b".as_bytes()), Err(PathError::NotText));
        assert_eq!(
            component("exe\u{200B}.pdf".as_bytes()),
            Err(PathError::NotText)
        );
        assert_eq!(component("\u{FEFF}x".as_bytes()), Err(PathError::NotText));
        assert!(component("café ünïcode 日本".as_bytes()).is_ok());
    }

    #[test]
    fn depth_is_bounded() {
        let c: Vec<&[u8]> = vec![b"a"; MAX_DEPTH + 1];
        assert_eq!(relative(&c), Err(PathError::TooDeep));
    }

    #[test]
    fn a_link_inside_the_folder_is_refused() {
        let d = tmp("link");
        std::os::unix::fs::symlink("/etc", d.join("evil")).unwrap();
        assert_eq!(
            under(&d, &["evil".into(), "passwd".into()]),
            Err(PathError::Link)
        );
        assert!(under(&d, &["fine".into(), "x".into()]).is_ok());
        assert_eq!(
            under(Path::new("rel"), &["x".into()]),
            Err(PathError::Outside)
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn fresh_folders_never_reuse_a_name() {
        let d = tmp("fresh");
        assert_eq!(fresh_folder(&d, "t").unwrap(), d.join("t"));
        std::fs::write(d.join("t"), b"mine").unwrap();
        assert_eq!(fresh_folder(&d, "t").unwrap(), d.join("t (2)"));
        std::fs::create_dir(d.join("t (2)")).unwrap();
        assert_eq!(fresh_folder(&d, "t").unwrap(), d.join("t (3)"));
        assert!(fresh_folder(&d, "..").is_err());
        // DL-SEC-05: never a hidden job folder
        assert_eq!(fresh_folder(&d, ".ssh"), Err(PathError::Hidden));
        assert_eq!(fresh_folder(&d, ".bash_profile"), Err(PathError::Hidden));
        let _ = std::fs::remove_dir_all(&d);
    }
}
