// SPDX-License-Identifier: BSD-2-Clause
//! Processes for the Processes page: the system API's [`ProcessSampler`] (struct of arrays,
//! CPU% per process from runtime deltas, names cached) over a [`ProcSource`] that reads
//! `/proc/[pid]/stat` under a [`Root`], so fixture trees work as `/proc` does.
use super::Root;
use crate::system::proc::{ProcSource, ProcessSampler, ProcessState, SystemTotals, table};
use std::collections::HashMap;

/// One row of the Processes page.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Proc {
    pub pid: i32,
    pub ppid: i32,
    pub name: String,
    pub user: String,
    /// Percent of one core since the previous sample.
    pub cpu: f32,
    /// Resident memory, bytes.
    pub rss: u64,
    pub threads: i32,
    pub state: ProcessState,
    /// Microseconds after boot it started: with the pid, names this process and no later one.
    pub start: i64,
    /// In a frozen cgroup (Memory Saver paused it): it cannot answer a polite signal.
    pub frozen: bool,
}

/// A process a person chose to signal, as the confirmation saw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Target {
    pub pid: i32,
    pub start: i64,
}

/// When `pid` started (microseconds after boot, as [`Proc::start`]); None when it is gone.
pub fn start_of(root: &Root, pid: i32) -> Option<i64> {
    let t = root.read_bytes(&format!("/proc/{pid}/stat"))?;
    table::parse_stat(&t, 0, crate::system::proc::clock_ticks()).map(|r| r.row.start_us)
}

/// Owner of `pid` (its `/proc` entry).
pub fn owner(root: &Root, pid: i32) -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(root.path(&format!("/proc/{pid}")))
        .ok()
        .map(|m| m.uid())
}

/// The targets that may still be signalled now: not init or the kernel (pid > 1), owned by
/// `uid`, and the same process the person saw (same start time: a pid reused since is not
/// it). Returns those pids and how many were dropped.
pub fn still_ours(root: &Root, uid: u32, targets: &[Target]) -> (Vec<i32>, usize) {
    let ok: Vec<i32> = targets
        .iter()
        .filter(|t| {
            t.pid > 1 && owner(root, t.pid) == Some(uid) && start_of(root, t.pid) == Some(t.start)
        })
        .map(|t| t.pid)
        .collect();
    let dropped = targets.len() - ok.len();
    (ok, dropped)
}

/// `pid` sits in a frozen cgroup (its `/proc/<pid>/cgroup` v2 path has `frozen 1`).
pub fn in_frozen_cgroup(root: &Root, pid: i32) -> bool {
    root.read(&format!("/proc/{pid}/cgroup"))
        .as_deref()
        .and_then(|t| t.lines().find_map(|l| l.strip_prefix("0::")))
        .and_then(|cg| root.read(&format!("/sys/fs/cgroup/{}/cgroup.events", cg.trim())))
        .as_deref()
        .is_some_and(super::apps::parse_frozen)
}

/// `/proc` under a root; the clock is the sampler's (set before each sample), so a test decides
/// how much time passed.
pub struct RootProcs {
    pub root: Root,
    tck: i64,
    page: i64,
    pub now_us: i64,
    passwd: Option<HashMap<i32, String>>,
}

impl RootProcs {
    /// A process table that reads `/proc` under `root`.
    pub fn new(root: Root) -> Self {
        RootProcs {
            root,
            tck: crate::system::proc::clock_ticks(),
            page: crate::system::proc::page_size(),
            now_us: 0,
            passwd: None,
        }
    }

    /// Reads the next samples from `root` instead.
    pub fn set_root(&mut self, root: Root) {
        self.root = root;
        self.passwd = None;
    }
}

impl ProcSource for RootProcs {
    fn procs(&mut self, buf: &mut Vec<table::Rec>) -> Option<usize> {
        use std::os::unix::fs::MetadataExt;
        let mut n = 0;
        for pid in self.root.names("/proc") {
            if !pid.bytes().all(|c| c.is_ascii_digit()) {
                continue;
            }
            let dir = self.root.path(&format!("/proc/{pid}"));
            let uid = std::fs::metadata(&dir)
                .ok()
                .and_then(|m| i32::try_from(m.uid()).ok())
                .unwrap_or(-1);
            let Some(rec) = self
                .root
                .read_bytes(&format!("/proc/{pid}/stat"))
                .and_then(|t| table::parse_stat(&t, uid, self.tck))
            else {
                continue;
            };
            if n == buf.len() {
                buf.resize(64.max(n + n / 4), table::Rec::default());
            }
            buf[n] = rec;
            n += 1;
        }
        Some(n)
    }
    fn now_us(&self) -> i64 {
        self.now_us
    }
    fn page_size(&self) -> i64 {
        self.page
    }
    /// The sampler reads the totals itself (per core); nothing to add here.
    fn totals(&mut self, _t: &mut SystemTotals) {}
    fn user_name(&mut self, uid: i32) -> Option<String> {
        let root = &self.root;
        let p = self.passwd.get_or_insert_with(|| {
            root.read("/etc/passwd")
                .unwrap_or_default()
                .lines()
                .filter_map(|l| {
                    let f: Vec<&str> = l.split(':').collect();
                    Some((f.get(2)?.parse().ok()?, f[0].to_owned()))
                })
                .collect()
        });
        p.get(&uid).cloned()
    }
}

/// The sampler's table as rows, by pid. `frozen`: pids of the frozen app scopes; a stopped or
/// waiting process outside them is looked up in its own cgroup.
pub fn rows(
    s: &mut ProcessSampler<RootProcs>,
    frozen: &std::collections::HashSet<i32>,
) -> Vec<Proc> {
    let mut v = Vec::with_capacity(s.count());
    for i in 0..s.count() {
        let (pid, stat) = (s.pid[i], s.stat[i]);
        let frozen = frozen.contains(&pid)
            || (matches!(stat, b'T' | b't' | b'D') && in_frozen_cgroup(&s.source().root, pid));
        v.push(Proc {
            pid,
            ppid: s.ppid[i],
            name: s.name[i].clone(),
            user: s.user(i).to_owned(),
            cpu: (s.cpu_percent[i] * 10.0).round() / 10.0,
            rss: u64::try_from(s.rss_bytes[i]).unwrap_or(0),
            threads: s.threads[i],
            state: s.state(i),
            start: s.start_us[i],
            frozen,
        });
    }
    v.sort_by_key(|p| p.pid);
    v
}
