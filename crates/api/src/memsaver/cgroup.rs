//! cgroup v2 files Memory Saver reads and writes. Every function takes the cgroup directory, so
//! tests run against a plain temp directory.
use std::io::Write;
use std::path::Path;

pub const ROOT: &str = "/sys/fs/cgroup";

fn read_u64(dir: &Path, file: &str) -> Option<u64> {
    std::fs::read_to_string(dir.join(file))
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Write one value in one write (kernfs takes O_TRUNC, as systemd's writes do).
pub fn write(dir: &Path, file: &str, value: &str) -> std::io::Result<()> {
    std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(dir.join(file))?
        .write_all(value.as_bytes())
}

pub fn current(dir: &Path) -> u64 {
    read_u64(dir, "memory.current").unwrap_or(0)
}

/// `usage_usec` from cpu.stat (always present, no cpu controller needed).
pub fn cpu_usec(dir: &Path) -> u64 {
    std::fs::read_to_string(dir.join("cpu.stat"))
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("usage_usec ")?.trim().parse().ok())
        })
        .unwrap_or(0)
}

/// Forks the pids limit refused so far (`max` in pids.events).
pub fn refused(dir: &Path) -> u64 {
    std::fs::read_to_string(dir.join("pids.events"))
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("max ")?.trim().parse().ok())
        })
        .unwrap_or(0)
}

/// Kill everything in the cgroup, descendants included, in one step.
pub fn kill(dir: &Path) -> bool {
    write(dir, "cgroup.kill", "1").is_ok()
}

pub fn procs(dir: &Path) -> Vec<i32> {
    std::fs::read_to_string(dir.join("cgroup.procs"))
        .map(|s| s.lines().filter_map(|l| l.trim().parse().ok()).collect())
        .unwrap_or_default()
}

/// memory.reclaim `bytes`, anon (to zram) first. Kernels without `swappiness=max` (anon only,
/// 6.15+) get `swappiness=200`, older ones the plain request. A partial reclaim (EAGAIN) is fine.
/// Returns the argument the kernel took, if any.
pub fn reclaim(dir: &Path, bytes: u64) -> Option<&'static str> {
    if bytes == 0 {
        return None;
    }
    for arg in [" swappiness=max", " swappiness=200", ""] {
        match write(dir, "memory.reclaim", &format!("{bytes}{arg}")) {
            Ok(()) => return Some(arg.trim()),
            Err(e) if e.raw_os_error() == Some(libc::EAGAIN) => return Some(arg.trim()),
            Err(e) if e.raw_os_error() == Some(libc::EINVAL) => continue,
            Err(_) => return None,
        }
    }
    None
}

pub fn freeze(dir: &Path, frozen: bool) -> bool {
    write(dir, "cgroup.freeze", if frozen { "1" } else { "0" }).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("wol-cg-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn reads_and_writes_cgroup_files() {
        let d = dir("rw");
        for (f, v) in [
            ("memory.current", "4096\n"),
            ("cpu.stat", "usage_usec 777\nuser_usec 700\n"),
            ("cgroup.procs", "10\n11\n"),
            ("cgroup.freeze", ""),
        ] {
            std::fs::write(d.join(f), v).unwrap();
        }
        assert_eq!((current(&d), cpu_usec(&d)), (4096, 777));
        assert_eq!(procs(&d), vec![10, 11]);
        assert!(freeze(&d, true));
        assert_eq!(
            std::fs::read_to_string(d.join("cgroup.freeze")).unwrap(),
            "1"
        );
        // Missing files read as zero and refuse writes.
        let none = d.join("absent");
        assert_eq!(current(&none), 0);
        assert!(!freeze(&none, true));
        assert_eq!(reclaim(&none, 1), None);
        assert_eq!(reclaim(&d, 0), None, "nothing to reclaim: no write");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn reclaim_asks_for_anon_first() {
        let d = dir("reclaim");
        std::fs::write(d.join("memory.reclaim"), "").unwrap();
        assert_eq!(reclaim(&d, 1 << 20), Some("swappiness=max"));
        assert_eq!(
            std::fs::read_to_string(d.join("memory.reclaim")).unwrap(),
            "1048576 swappiness=max"
        );
        std::fs::remove_dir_all(&d).ok();
    }
}
