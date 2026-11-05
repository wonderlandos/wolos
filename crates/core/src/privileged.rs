// SPDX-License-Identifier: BSD-2-Clause
//! Root-side process launch (docs/STANDARDS.md section 7, ADR 0040; audit A22/B12, R2C-16). Every
//! program a root process of WolOS starts (wold, wol-admind, wol-login, the xguard daemon) is
//! built by [`privileged_command`]: a cleared environment, a fixed `PATH`, `LC_ALL=C`, stdin from
//! /dev/null, typed argv and never a shell. The invariant: nothing an app or a user put in the
//! daemon's environment (`LD_PRELOAD`, `PATH`, `PACMAN_*`) reaches a root child. Rust opens every
//! descriptor close-on-exec, so only 0-2 are inherited. A caller that needs one more variable (a
//! secret homectl reads from its environment, the login window's `LANG`) adds it to the returned
//! `Command` and says why at that line. `api::privileged` re-exports these for wold's code.
use std::process::{Command, Stdio};
use std::time::Duration;

/// The only search path a root child gets.
pub const PATH: &str = "/usr/bin:/usr/sbin";

/// A `Command` for `program` with a cleared environment, `PATH` = [`PATH`], `LC_ALL=C` (stable
/// output to parse) and stdin from /dev/null. The caller adds typed arguments; there is no shell.
pub fn privileged_command(program: &str) -> Command {
    let mut c = Command::new(program);
    c.env_clear()
        .env("PATH", PATH)
        .env("LC_ALL", "C")
        .current_dir("/")
        .stdin(Stdio::null());
    c
}

/// The most of a failed command's stderr kept (the end of it: the error is last).
pub(crate) const STDERR_TAIL: usize = 2048;
/// The longest a failed command's stderr is waited for after it ended.
const STDERR_WAIT: Duration = Duration::from_millis(300);

thread_local! {
    static LAST_STDERR: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

/// What the last [`capture`] on this thread that failed said on stderr (its last lines, at most
/// [`STDERR_TAIL`] bytes), taken: empty when it succeeded or said nothing. The daemon logs it, so
/// "the update failed" in the journal comes with pacman's own reason (live-4 F1: the stderr of
/// a root child used to go to /dev/null).
pub fn take_last_stderr() -> String {
    LAST_STDERR.with(|s| std::mem::take(&mut *s.borrow_mut()))
}

/// Leaves `output` (the end of a run that was read by someone else, `progress::run_tracked_cmd`)
/// for [`take_last_stderr`] as a failed [`capture`] would; an empty `output` clears it.
pub(crate) fn keep_stderr(output: &[u8]) {
    let said = tail_of(output);
    LAST_STDERR.with(|s| *s.borrow_mut() = said);
}

/// The last [`STDERR_TAIL`] bytes of `bytes`, from a whole line, as text.
fn tail_of(bytes: &[u8]) -> String {
    let start = bytes.len().saturating_sub(STDERR_TAIL);
    let text = String::from_utf8_lossy(&bytes[start..]);
    let text = if start > 0 {
        text.split_once('\n').map_or("", |(_, rest)| rest)
    } else {
        &text
    };
    text.trim().to_owned()
}

/// `argv` as [`privileged_command`]: stdout when it exits 0 within `timeout`, None otherwise.
/// `cat`/`ls` probes are answered without a fork (`crate::system::builtin`). A failure leaves
/// the child's stderr for [`take_last_stderr`].
pub fn capture(argv: &[&str], timeout: Duration) -> Option<String> {
    capture_grace(argv, timeout, crate::system::KILL_GRACE)
}

/// [`capture`] where a child past `timeout` is interrupted (its whole process group), given `grace`
/// to stop, then killed: a package transaction gets time to finish its package (QA-A9-CORE-3).
pub fn capture_grace(argv: &[&str], timeout: Duration, grace: Duration) -> Option<String> {
    use std::io::Read;
    use std::os::unix::process::CommandExt;
    if let Some(out) = crate::system::builtin(argv) {
        // No child ran: nothing of an earlier failure is left to be mistaken for this one's.
        let _ = take_last_stderr();
        return out;
    }
    let (file, args) = argv.split_first()?;
    let mut child = privileged_command(file)
        .args(args)
        .process_group(0)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    // Drained as it comes (a full pipe would stop the child); only the end is kept.
    let (tx, rx) = std::sync::mpsc::channel();
    if let Some(mut err) = child.stderr.take() {
        let drain = std::thread::Builder::new().name("privileged-stderr".into());
        let _ = drain.spawn(move || {
            let mut all = Vec::new();
            let mut chunk = [0u8; 4096];
            while let Ok(n) = err.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                all.extend_from_slice(&chunk[..n]);
                if all.len() > 4 * STDERR_TAIL {
                    all.drain(..all.len() - STDERR_TAIL);
                }
            }
            let _ = tx.send(all);
        });
    }
    let out = crate::system::wait_output_grace(child, timeout, grace);
    // Only a failure is explained, and only for a moment: a grandchild may hold the pipe open
    // (a success never waits for it).
    let said = if out.is_none() {
        rx.recv_timeout(STDERR_WAIT)
            .map(|b| tail_of(&b))
            .unwrap_or_default()
    } else {
        String::new()
    };
    LAST_STDERR.with(|s| *s.borrow_mut() = said);
    out
}

/// `argv` with `input` on stdin (secrets never in argv); stdout as [`capture`].
pub fn capture_input(argv: &[&str], input: &str, timeout: Duration) -> Option<String> {
    // Its stderr is not kept: what an earlier `capture` left must not be read as this one's.
    let _ = take_last_stderr();
    let (file, args) = argv.split_first()?;
    let mut cmd = privileged_command(file);
    cmd.args(args);
    crate::system::run_input(cmd, input, timeout)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live-4 F1: a root child that fails says why, and the daemon can log it. Before, the
    /// stderr went to /dev/null and "pkg-upgrade: failed" was all the journal had.
    #[test]
    fn a_failed_child_leaves_its_stderr() {
        let _ = take_last_stderr();
        let out = capture(
            &[
                "sh",
                "-c",
                "echo 'error: unable to lock database' >&2; exit 1",
            ],
            Duration::from_secs(5),
        );
        assert_eq!(out, None);
        assert_eq!(take_last_stderr(), "error: unable to lock database");
        assert_eq!(take_last_stderr(), "", "taken once");
        // A success clears it; a loud success is not an error.
        assert!(
            capture(
                &["sh", "-c", "echo hi; echo warn >&2"],
                Duration::from_secs(5)
            )
            .is_some()
        );
        assert_eq!(take_last_stderr(), "");
        // Not stale after a command that never ran or whose stderr is not kept.
        assert_eq!(
            capture(
                &["sh", "-c", "echo old >&2; exit 1"],
                Duration::from_secs(5)
            ),
            None
        );
        let _ = capture_input(&["cat"], "x", Duration::from_secs(5));
        assert_eq!(
            take_last_stderr(),
            "",
            "capture_input leaves nothing from the earlier failure"
        );
        assert_eq!(
            capture(
                &["sh", "-c", "echo old >&2; exit 1"],
                Duration::from_secs(5)
            ),
            None
        );
        let _ = capture(&["cat", "/proc/cmdline"], Duration::from_secs(5));
        assert_eq!(take_last_stderr(), "", "a builtin probe clears it");
        // A grandchild that keeps stderr open does not hold up a failure for long.
        let t = std::time::Instant::now();
        assert_eq!(
            capture(
                &["sh", "-c", "(sleep 3 >/dev/null &); exit 1"],
                Duration::from_secs(5)
            ),
            None
        );
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    }

    #[test]
    fn only_the_end_of_a_long_stderr_is_kept() {
        let long = format!("{}\nlast line\n", "x".repeat(10_000));
        let t = tail_of(long.as_bytes());
        assert!(
            t.len() <= STDERR_TAIL && t.ends_with("last line"),
            "{}",
            t.len()
        );
        let script = "i=0; while [ $i -lt 3000 ]; do echo 0123456789012345678901234567890123456789 >&2; i=$((i+1)); done; echo end >&2; exit 3";
        assert_eq!(
            capture(&["sh", "-c", script], Duration::from_secs(10)),
            None
        );
        let said = take_last_stderr();
        assert!(
            said.ends_with("end") && said.len() <= STDERR_TAIL,
            "{}",
            said.len()
        );
    }
}
