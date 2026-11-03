// SPDX-License-Identifier: BSD-2-Clause
//! Root-side process launch (docs/STANDARDS.md section 7, ADR 0015; audit A22/B12, R2C-16). Every
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

/// `argv` as [`privileged_command`]: stdout when it exits 0 within `timeout`, None otherwise.
/// `cat`/`ls` probes are answered without a fork (`crate::system::builtin`).
pub fn capture(argv: &[&str], timeout: Duration) -> Option<String> {
    if let Some(out) = crate::system::builtin(argv) {
        return out;
    }
    let (file, args) = argv.split_first()?;
    let child = privileged_command(file)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    crate::system::wait_output(child, timeout)
}

/// `argv` with `input` on stdin (secrets never in argv); stdout as [`capture`].
pub fn capture_input(argv: &[&str], input: &str, timeout: Duration) -> Option<String> {
    let (file, args) = argv.split_first()?;
    let mut cmd = privileged_command(file);
    cmd.args(args);
    crate::system::run_input(cmd, input, timeout)
}
