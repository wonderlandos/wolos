// SPDX-License-Identifier: BSD-2-Clause
//! Why an app or package change failed, in words (QA-STORE-59). pacman and flatpak say it on
//! stderr in C locale; the root daemon and the Store read the end of that and name the cause, so
//! the App Store's status sheet tells the person the true reason instead of one guess for every
//! failure ("Check that you’re online" for a full disk). Pure: the caller keeps the output.

/// What stopped a change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    /// The mirror or Flathub could not be reached, or the file did not come down.
    Download,
    /// The disk has no room for it.
    NoRoom,
    /// A package failed its signature or checksum: damaged, or changed on the way.
    Damaged,
    /// A slot build, `wol update` or another package change holds the update lock.
    UpdateRunning,
}

/// The cause the end of a failed change's output names; None when it names none of them.
pub fn cause(output: &str) -> Option<Cause> {
    let t = output.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| t.contains(w));
    // In this order: the lock sentence is the whole output; a full disk can also stop a download
    // ("failed retrieving file ... write error"), and a damaged package came down fine.
    if t.contains(&crate::update::BUSY.to_lowercase()) {
        Some(Cause::UpdateRunning)
    } else if has(&["not enough free disk space", "too full", "no space left on device"])
        // R2-1: the kernel's mount-count limit says "No space left on device" too, and no disk
        // space frees it.
        && !t.contains("move_mount")
    {
        Some(Cause::NoRoom)
    } else if has(&[
        "invalid or corrupted package",
        "pgp signature",
        "signature from",
        "checksum mismatch",
        "unknown trust",
    ]) {
        Some(Cause::Damaged)
    } else if has(&[
        "failed retrieving file",
        "failed to retrieve some files",
        "failed to synchronize",
        "could not resolve host",
        "could not connect",
        "failed to connect",
        "unable to connect",
        "name resolution",
        "network is unreachable",
        "connection timed out",
        "operation too slow",
    ]) {
        Some(Cause::Download)
    } else {
        None
    }
}

/// The sentence for `cause`; `installing` picks "installed" or "removed". No app name: the
/// sentence travels in wol-admind's `failed <message>` line, 200 bytes at most.
pub fn sentence(cause: Cause, installing: bool) -> String {
    let why = match cause {
        Cause::Download => "the download failed. Check that you’re online, then try again.",
        Cause::NoRoom => {
            "there isn’t enough free space on the disk. Free some in Settings > Storage, then \
             try again."
        }
        Cause::Damaged => {
            "the download didn’t pass its safety check, so it may be damaged or changed on the \
             way. Try again later."
        }
        Cause::UpdateRunning => {
            "another update or app change is running. Wait for it to finish, then try again."
        }
    };
    let verb = if installing { "installed" } else { "removed" };
    format!("Nothing was {verb}: {why}")
}

/// [`sentence`] for the cause `output` names, if any.
pub fn explain(output: &str, installing: bool) -> Option<String> {
    cause(output).map(|c| sentence(c, installing))
}

/// What is said when the output names no cause: still true (the change was not made) and still
/// something to do, and it blames neither the network nor the person.
pub fn unknown(installing: bool) -> String {
    let verb = if installing { "installed" } else { "removed" };
    format!("Nothing was {verb}: the change didn’t finish. Restart the computer, then try again.")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// pacman's own words for each failure, as `LC_ALL=C pacman -S` prints them, then `wol pkg`'s
    /// closing line (which used to be the only line the daemon kept).
    const CLOSING: &str = "wol: Nothing was changed. Try again, or run `wol update` first.\n";

    fn run(lines: &str) -> String {
        format!("resolving dependencies...\nlooking for conflicting packages...\n{lines}{CLOSING}")
    }

    /// QA-STORE-59, the five kinds of the 0.12.1 VM run: offline, disk full, tampered, update
    /// lock held (a slot build), and the mount-limit failure of R2-1 (which says "No space left
    /// on device" and is not a full disk).
    #[test]
    fn each_failure_kind_is_named() {
        let offline = run(
            "error: failed retrieving file 'bashtop-0.9.25-1-any.pkg.tar.zst' \
            from mirror.wolos.org : Could not resolve host: mirror.wolos.org\n\
            error: failed to commit transaction (failed to retrieve some files)\n",
        );
        assert_eq!(cause(&offline), Some(Cause::Download));
        let full = run(
            "error: Partition / too full: 3208 MiB needed, 410 MiB free\n\
            error: not enough free disk space\n\
            error: failed to commit transaction (not enough free disk space)\n",
        );
        assert_eq!(cause(&full), Some(Cause::NoRoom));
        let tampered = run("error: botsay: signature from \"x\" is invalid\n\
            error: failed to commit transaction (invalid or corrupted package (PGP signature))\n\
            Errors occurred, no packages were upgraded.\n");
        assert_eq!(cause(&tampered), Some(Cause::Damaged));
        let busy = "wol: Another system update or package change is running. Try again when it \
            has finished.\n";
        assert_eq!(cause(busy), Some(Cause::UpdateRunning));
        let leak = run(
            "mount: /run/wolos/layer-root: move_mount() failed: No space left on \
            device.\n",
        );
        assert_eq!(cause(&leak), None, "the mount limit is not a full disk");
        assert_eq!(cause(""), None);
        assert_eq!(cause("error: target not found: nosuchpkg\n"), None);
    }

    /// Flatpak says it differently.
    #[test]
    fn flatpak_failures_are_named() {
        assert_eq!(
            cause(
                "error: Unable to connect to dl.flathub.org: Temporary failure in name resolution\n"
            ),
            Some(Cause::Download)
        );
        assert_eq!(
            cause(
                "error: Failed to install org.x.App: Error writing to file: No space left on device\n"
            ),
            Some(Cause::NoRoom)
        );
        assert_eq!(
            cause("error: Failed to install org.x.App: Checksum mismatch for 'abc'\n"),
            Some(Cause::Damaged)
        );
    }

    /// Every sentence is whole, in one reply line, and says what was and was not done.
    #[test]
    fn the_sentences_are_whole_and_fit_one_reply_line() {
        for installing in [true, false] {
            for c in [
                Cause::Download,
                Cause::NoRoom,
                Cause::Damaged,
                Cause::UpdateRunning,
            ] {
                let s = sentence(c, installing);
                assert!(s.starts_with("Nothing was "), "{s}");
                assert!(s.ends_with('.'), "{s}");
                assert!(
                    s.len() <= shared::admin::MAX_FAILURE_MESSAGE,
                    "{} bytes: {s}",
                    s.len()
                );
                assert!(!s.contains("error:"), "{s}");
            }
        }
        assert!(sentence(Cause::NoRoom, true).contains("installed"));
        assert!(sentence(Cause::NoRoom, false).contains("removed"));
        assert_eq!(explain("", true), None);
        for installing in [true, false] {
            let s = unknown(installing);
            assert!(s.starts_with("Nothing was ") && s.len() <= 200, "{s}");
            assert!(!s.to_lowercase().contains("online"), "{s}");
        }
    }
}
