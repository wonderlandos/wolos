// SPDX-License-Identifier: BSD-2-Clause
//! System Monitor data (docs/modules/monitor.md): what `wol-monitor` shows, read from /proc, /sys
//! and cgroup v2 only. Every topic is a pure parser over the text of one file plus a read through
//! a [`Root`], so the tests run on fixture trees (`crates/core/tests/fixtures/monitor`). The
//! [`sampler::Sampler`] keeps the previous counters and turns them into rates; it is the only
//! thing that touches the files, and the app runs one, on one thread.
pub mod apps;
pub mod cpu;
pub mod disk;
pub mod gpu;
pub mod net;
pub mod procs;
pub mod sampler;
pub mod startup;
pub mod temps;

use std::path::{Path, PathBuf};

/// Most bytes [`Root::read`] takes from one file.
pub const MAX_READ: u64 = 1 << 20;

/// Where /proc, /sys and /var/run are: `/` on a running system, a fixture directory in tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Root(PathBuf);

impl Root {
    /// The running system.
    pub fn system() -> Self {
        Root(PathBuf::from("/"))
    }

    /// A tree laid out like `/` (tests).
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Root(dir.into())
    }

    pub fn is_system(&self) -> bool {
        self.0 == Path::new("/")
    }

    /// `rel` (written absolute, `/proc/stat`) under the root.
    pub fn path(&self, rel: &str) -> PathBuf {
        self.0.join(rel.trim_start_matches('/'))
    }

    /// The file's text, at most [`MAX_READ`] bytes of it (a /proc or /sys file is small; a
    /// fixture or a bind mount that is not must not fill memory).
    pub fn read(&self, rel: &str) -> Option<String> {
        String::from_utf8(self.read_bytes(rel)?).ok()
    }

    /// [`Root::read`] as bytes.
    pub fn read_bytes(&self, rel: &str) -> Option<Vec<u8>> {
        use std::io::Read;
        let mut v = Vec::new();
        std::fs::File::open(self.path(rel))
            .ok()?
            .take(MAX_READ)
            .read_to_end(&mut v)
            .ok()?;
        Some(v)
    }

    pub fn exists(&self, rel: &str) -> bool {
        self.path(rel).exists()
    }

    /// Entry names of a directory, sorted; empty when it is missing.
    pub fn names(&self, rel: &str) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.path(rel))
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| e.file_name().into_string().ok())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }

    /// Directories (not symlinks to them) among [`Root::names`].
    pub fn dirs(&self, rel: &str) -> Vec<String> {
        self.names(rel)
            .into_iter()
            .filter(|n| {
                std::fs::symlink_metadata(self.path(&format!("{rel}/{n}")))
                    .is_ok_and(|m| m.is_dir())
            })
            .collect()
    }
}

/// The system API's file reads on the same root, so its parsers (temperatures, disks) run here
/// too. Commands run only on the system root; a fixture answers `df` from `run/df`.
impl crate::system::Host for Root {
    fn now(&self) -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64())
    }
    fn read_text(&self, path: &str) -> Option<String> {
        self.read(path)
    }
    fn run(&self, argv: &[&str]) -> Option<String> {
        if self.is_system() {
            crate::system::run_with_timeout(argv, crate::system::RUN_TIMEOUT)
        } else {
            self.read(&format!("run/{}", argv.first()?))
        }
    }
    fn load_average(&self) -> Option<crate::system::Load> {
        let t = self.read("/proc/loadavg")?;
        let mut f = t.split_whitespace().map(|v| v.parse::<f64>().ok());
        Some(crate::system::Load {
            one: f.next()??,
            five: f.next()??,
            fifteen: f.next()??,
        })
    }
    fn list_dir(&self, dir: &str) -> Vec<String> {
        let dir = dir.trim_end_matches('/');
        self.names(dir)
            .into_iter()
            .map(|n| format!("{dir}/{n}"))
            .collect()
    }
}

/// Bytes per second from two readings of a counter `secs` apart. A counter that went back (a
/// device came again, a wrap) reads 0, never a negative or huge rate.
pub fn rate(prev: u64, cur: u64, secs: f64) -> f64 {
    if secs <= 0.0 || cur < prev {
        0.0
    } else {
        (cur - prev) as f64 / secs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_never_go_negative() {
        assert_eq!(rate(100, 300, 2.0), 100.0);
        assert_eq!(rate(300, 100, 1.0), 0.0, "counter reset");
        assert_eq!(rate(1, 2, 0.0), 0.0, "no time passed");
    }

    #[test]
    fn root_paths_are_under_the_root() {
        let r = Root::at("/tmp/x");
        assert_eq!(r.path("/proc/stat"), Path::new("/tmp/x/proc/stat"));
        assert!(Root::system().is_system() && !r.is_system());
    }
}
