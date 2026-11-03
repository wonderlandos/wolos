// SPDX-License-Identifier: GPL-2.0-or-later
//! Root-side process launch (docs/STANDARDS.md section 7, ADR 0040; audit A22/B12). Every program
//! wold or wol-admind starts as root is built by [`privileged_command`]: a cleared environment, a
//! fixed `PATH`, stdin from /dev/null, typed argv and never a shell. The invariant: nothing an
//! app or a user put in the daemon's environment (`LD_PRELOAD`, `PATH`, `PACMAN_*`) reaches a
//! root child. Rust opens every descriptor close-on-exec, so only 0-2 are inherited.
//! `crates/api/tests/privileged.rs` fails when a root-side `Command::new` bypasses this file. The
//! builder and the capture helpers live in `wol_core::privileged` (so wol-login and xguard use the
//! same ones, R2C-16) and are re-exported here.
use std::time::Duration;
pub use wol_core::privileged::{PATH, capture, capture_input, privileged_command};

/// wold's [`wol_core::system::Host`] (audit S14): reads files as `wol_core::system::PosixHost`
/// does, and runs every command through [`capture`], so facts and device probes get the cleared
/// environment too.
pub struct PrivilegedHost;

impl wol_core::system::Host for PrivilegedHost {
    fn now(&self) -> f64 {
        wol_core::system::PosixHost.now()
    }
    fn read_text(&self, path: &str) -> Option<String> {
        wol_core::system::PosixHost.read_text(path)
    }
    fn read_head(&self, path: &str, buf: &mut [u8]) -> Option<usize> {
        wol_core::system::PosixHost.read_head(path, buf)
    }
    fn run(&self, argv: &[&str]) -> Option<String> {
        capture(argv, wol_core::system::RUN_TIMEOUT)
    }
    fn run_slow(&self, argv: &[&str]) -> Option<String> {
        capture(argv, Duration::from_secs(120))
    }
    fn read_link(&self, path: &str) -> Option<String> {
        wol_core::system::PosixHost.read_link(path)
    }
    fn load_average(&self) -> Option<wol_core::system::Load> {
        wol_core::system::PosixHost.load_average()
    }
    fn list_dir(&self, dir: &str) -> Vec<String> {
        wol_core::system::PosixHost.list_dir(dir)
    }
    fn is_root(&self) -> bool {
        wol_core::system::PosixHost.is_root()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_environment_is_cleared_and_the_path_fixed() {
        let c = privileged_command("env");
        let envs: Vec<(String, Option<String>)> = c
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();
        assert!(envs.contains(&("PATH".into(), Some(PATH.into()))));
        assert!(envs.iter().all(|(k, _)| k == "PATH" || k == "LC_ALL"));
        // The reaper tests in this binary reap any child (waitpid(-1)), so a run may lose its
        // exit status; a few tries make that race irrelevant.
        let retry = |f: &dyn Fn() -> Option<String>| (0..5).find_map(|_| f());
        // The test runner's own environment (HOME, CARGO_*, ...) never reaches the child.
        let out = retry(&|| capture(&["env"], Duration::from_secs(5))).unwrap_or_default();
        let mut keys: Vec<&str> = out.lines().filter_map(|l| l.split('=').next()).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["LC_ALL", "PATH"], "{out}");
        assert_eq!(
            retry(&|| capture_input(&["head", "-c", "6"], "secret", Duration::from_secs(5)))
                .as_deref(),
            Some("secret")
        );
        // wold's Host runs commands the same way.
        use wol_core::system::Host;
        let out = retry(&|| PrivilegedHost.run(&["env"])).unwrap_or_default();
        let mut keys: Vec<&str> = out.lines().filter_map(|l| l.split('=').next()).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["LC_ALL", "PATH"], "{out}");
    }
}
