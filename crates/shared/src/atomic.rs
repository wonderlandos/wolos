// SPDX-License-Identifier: BSD-2-Clause
//! One atomic file writer for every state file WolOS writes (audit S4-10, R2C-23; STANDARDS 7.1
//! "TOCTOU"). A reader sees the old file or the new one, never a truncated mix, and two writers
//! never share a temporary name.
//!
//! Invariant: the temporary is `<name>.<pid>-<nonce>.wol-tmp` next to the target, created with
//! `O_CREAT | O_EXCL | O_NOFOLLOW` (a planted file or link makes the write fail, it is never
//! followed), given its mode (and owner) on the descriptor before any byte is written, flushed
//! with `fsync`, then renamed over the target. On any error the temporary is removed.
use std::io::Write;
use std::path::{Path, PathBuf};

/// Suffix of every temporary this module creates.
pub const TMP_SUFFIX: &str = ".wol-tmp";

/// The temporary for `path`: `<name>.<pid>-<nonce>.wol-tmp` in the same directory. Err when the
/// path has no file name or the kernel's random source is unreadable.
pub fn temp_name(path: &Path) -> std::io::Result<PathBuf> {
    let Some(name) = path.file_name() else {
        return Err(std::io::Error::other("the path names no file"));
    };
    let nonce = crate::jobs::new_nonce()?;
    let mut t = name.to_os_string();
    t.push(format!(".{}-{nonce}{TMP_SUFFIX}", std::process::id()));
    Ok(path.with_file_name(t))
}

/// Replace `path` with `data`, mode `mode` (exactly, whatever the umask). The parent directory
/// must exist. Err (and nothing changed at `path`) when any step fails.
pub fn atomic_write(path: &Path, mode: u32, data: &[u8]) -> std::io::Result<()> {
    write_as(path, mode, None, data)
}

/// [`atomic_write`], and the file belongs to account `owner` (uid; group unchanged) before it
/// appears at `path`, so it is never readable by anyone else, not even for a moment. Changing the
/// owner needs root (or the same uid); Err otherwise, and nothing changed at `path`.
pub fn write_as(path: &Path, mode: u32, owner: Option<u32>, data: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let tmp = temp_name(path)?;
    let done = (|| {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&tmp)?;
        // On the descriptor, not the name: the umask may have taken bits off the open's mode.
        f.set_permissions(std::fs::Permissions::from_mode(mode))?;
        if let Some(uid) = owner {
            std::os::unix::fs::fchown(&f, Some(uid), None)?;
        }
        f.write_all(data)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if done.is_err() {
        let _ = std::fs::remove_file(&tmp);
        return done;
    }
    sync_parent(path)
}

/// Flush the directory entry the rename made (audit C6-4), so the new file survives a power
/// loss. Filesystems that cannot sync a directory (EINVAL) are not an error.
fn sync_parent(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let d = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC)
        .open(parent)?;
    match d.sync_all() {
        Err(e) if e.raw_os_error() == Some(libc::EINVAL) => Ok(()),
        r => r,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wol-atomic-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn mode(p: &Path) -> u32 {
        std::fs::metadata(p).unwrap().permissions().mode() & 0o7777
    }

    #[test]
    fn writes_replace_and_keep_the_mode_asked_for() {
        let d = dir("mode");
        let p = d.join("a.conf");
        atomic_write(&p, 0o600, b"one").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"one");
        assert_eq!(mode(&p), 0o600);
        atomic_write(&p, 0o600, b"two").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"two");
        assert_eq!(mode(&p), 0o600, "a 0600 file keeps 0600");
        // The umask (022 or 077) never decides: 0644 is 0644.
        atomic_write(&p, 0o644, b"three").unwrap();
        assert_eq!(mode(&p), 0o644);
        let left: Vec<_> = std::fs::read_dir(&d)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(TMP_SUFFIX))
            .collect();
        assert!(left.is_empty(), "no temporary left behind");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// `foo.conf` and `foo.key` used to share `foo.wol-tmp` (S4-10): each now ends with its own text.
    #[test]
    fn two_names_with_one_stem_written_at_once_keep_their_own_content() {
        let d = dir("stem");
        let (a, b) = (d.join("a.conf"), d.join("a.key"));
        let hs: Vec<_> = [
            (a.clone(), b"conf".as_slice()),
            (b.clone(), b"key".as_slice()),
        ]
        .into_iter()
        .map(|(p, text)| {
            std::thread::spawn(move || {
                for _ in 0..200 {
                    atomic_write(&p, 0o600, text).unwrap();
                }
            })
        })
        .collect();
        for h in hs {
            h.join().unwrap();
        }
        assert_eq!(std::fs::read(&a).unwrap(), b"conf");
        assert_eq!(std::fs::read(&b).unwrap(), b"key");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Many writers of one file and a reader: the reader always sees one whole version.
    #[test]
    fn a_reader_never_sees_a_torn_file() {
        let d = dir("torn");
        let p = d.join("status.json");
        atomic_write(&p, 0o644, b"{\"n\":0}").unwrap();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reader = {
            let (p, stop) = (p.clone(), stop.clone());
            std::thread::spawn(move || {
                let mut reads = 0;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    let t = std::fs::read(&p).unwrap();
                    let v: serde_json::Value = serde_json::from_slice(&t).unwrap();
                    assert!(v.get("n").is_some());
                    reads += 1;
                }
                reads
            })
        };
        let writers: Vec<_> = (0..8)
            .map(|i| {
                let p = p.clone();
                std::thread::spawn(move || {
                    for n in 0..100 {
                        let body = format!("{{\"n\":{n},\"w\":{i},\"pad\":\"{}\"}}", "x".repeat(n));
                        atomic_write(&p, 0o644, body.as_bytes()).unwrap();
                    }
                })
            })
            .collect();
        for w in writers {
            w.join().unwrap();
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(reader.join().unwrap() > 0);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A temporary name somebody planted (as a link) is never followed: the write fails or uses a
    /// fresh name, and the link's target is untouched.
    #[test]
    fn a_planted_link_is_not_followed_and_failures_leave_nothing() {
        let d = dir("link");
        let victim = d.join("victim");
        std::fs::write(&victim, b"keep").unwrap();
        let p = d.join("missing-dir").join("x");
        assert!(atomic_write(&p, 0o600, b"x").is_err(), "no parent: Err");
        let t = temp_name(&d.join("x")).unwrap();
        assert!(t.to_string_lossy().ends_with(TMP_SUFFIX));
        assert_ne!(t, temp_name(&d.join("x")).unwrap(), "unique per call");
        std::os::unix::fs::symlink(&victim, &t).unwrap();
        // create_new on an existing name fails: exercise it through the same open flags.
        use std::os::unix::fs::OpenOptionsExt;
        assert!(
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&t)
                .is_err()
        );
        atomic_write(&d.join("x"), 0o600, b"new").unwrap();
        assert_eq!(std::fs::read(&victim).unwrap(), b"keep");
        assert!(temp_name(Path::new("/")).is_err(), "no file name");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// C6-4: the directory sync runs for a nested path and a bare relative name.
    #[test]
    fn the_parent_directory_is_synced_after_the_rename() {
        let d = dir("dirsync");
        let p = d.join("x");
        atomic_write(&p, 0o644, b"a").unwrap();
        sync_parent(&p).unwrap();
        sync_parent(Path::new("bare-name")).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"a");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn write_as_sets_the_owner_before_the_file_appears() {
        let d = dir("owner");
        let p = d.join("findings.json");
        // Our own uid is always allowed; another uid needs root.
        let me = crate::sys::euid();
        write_as(&p, 0o600, Some(me), b"[]").unwrap();
        use std::os::unix::fs::MetadataExt;
        assert_eq!(std::fs::metadata(&p).unwrap().uid(), me);
        assert_eq!(mode(&p), 0o600);
        if me != 0 {
            assert!(write_as(&p, 0o600, Some(me + 1), b"x").is_err());
            assert_eq!(
                std::fs::read(&p).unwrap(),
                b"[]",
                "a failed write changes nothing"
            );
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}
