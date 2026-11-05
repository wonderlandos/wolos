// SPDX-License-Identifier: BSD-2-Clause
//! The one sampler of the System Monitor: every second the app's sampler thread calls
//! [`Sampler::sample`] and sends the [`Sample`] to the window. It keeps the previous counters
//! (CPU ticks, bytes per interface and disk, CPU time per app) and turns them into rates over
//! the time the caller says passed. What no page shows is not read ([`Want`]); the costly reads
//! are spaced out: PSS every [`PSS_EVERY`] s, disk space (a `df`) every [`DISKS_EVERY`] s.
use super::{Root, apps, cpu, disk, gpu, net, procs, rate, temps};
use crate::system::proc::ProcessSampler;
use crate::system::stats::MemoryStats;
use crate::system::{Host, Load};
use std::collections::HashMap;

/// Seconds between two PSS reads of one app (one `smaps_rollup` per process).
pub const PSS_EVERY: f64 = 5.0;
/// Seconds between two disk space reads.
pub const DISKS_EVERY: f64 = 10.0;

/// What the open pages need beyond the always-on numbers (CPU, memory, network, disks I/O,
/// GPU, temperatures, apps).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Want {
    /// The process table (Processes page).
    pub procs: bool,
    /// Space per disk (Disks page).
    pub disks: bool,
    /// PSS per app (Apps page).
    pub pss: bool,
}

/// Everything one tick read.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct Sample {
    /// Caller's clock, seconds.
    pub at: f64,
    /// Percent busy: the total first, then one per core; all 0 on the first sample.
    pub cpu: Vec<f64>,
    pub memory: MemoryStats,
    pub load: Option<Load>,
    pub net: Vec<net::Interface>,
    pub disk_io: Vec<disk::Io>,
    /// Only when [`Want::disks`].
    pub mounts: Option<Vec<disk::DiskUsage>>,
    pub gpus: Vec<gpu::Gpu>,
    pub temps: Vec<temps::Temperature>,
    pub apps: Vec<apps::AppUse>,
    /// Only when [`Want::procs`].
    pub procs: Option<Vec<procs::Proc>>,
    /// Memory Saver is on.
    pub saver_on: bool,
}

/// Reads the machine's figures and keeps the previous reading to turn counters into rates.
pub struct Sampler {
    root: Root,
    uid: Option<u32>,
    prev_at: Option<f64>,
    cpu: Vec<cpu::CpuTimes>,
    net: HashMap<String, (u64, u64)>,
    disk: HashMap<String, (u64, u64)>,
    app_cpu: HashMap<String, u64>,
    /// unit -> (when, PSS).
    pss: HashMap<String, (f64, Option<u64>)>,
    mounts: Option<(f64, Vec<disk::DiskUsage>)>,
    procs: ProcessSampler<procs::RootProcs>,
}

impl Sampler {
    /// Apps of `uid` only (the session's user), or of everyone.
    pub fn new(root: Root, uid: Option<u32>) -> Self {
        Sampler {
            procs: ProcessSampler::new(procs::RootProcs::new(root.clone())),
            root,
            uid,
            prev_at: None,
            cpu: Vec::new(),
            net: HashMap::new(),
            disk: HashMap::new(),
            app_cpu: HashMap::new(),
            pss: HashMap::new(),
            mounts: None,
        }
    }

    /// Read another tree from the next sample on, keeping the counters (tests: t0 then t1).
    pub fn set_root(&mut self, root: Root) {
        self.procs.source().set_root(root.clone());
        self.root = root;
    }

    /// One reading at time `at` (seconds): only the parts in `want`, rates from the previous reading.
    pub fn sample(&mut self, at: f64, want: Want) -> Sample {
        let dt = self.prev_at.map_or(0.0, |p| at - p);
        self.prev_at = Some(at);
        let r = &self.root;

        let cores = cpu::parse(&r.read("/proc/stat").unwrap_or_default());
        let cpu = if self.cpu.is_empty() {
            vec![0.0; cores.len()]
        } else {
            cpu::usage(&self.cpu, &cores)
        };
        self.cpu = cores;

        let mut seen = HashMap::new();
        let net = net::parse_net_dev(&r.read("/proc/net/dev").unwrap_or_default())
            .into_iter()
            .map(|c| {
                let (prx, ptx) = self.net.get(&c.name).copied().unwrap_or((c.rx, c.tx));
                seen.insert(c.name.clone(), (c.rx, c.tx));
                net::Interface {
                    kind: net::kind(r, &c.name),
                    up: net::is_up(r, &c.name),
                    rx_rate: rate(prx, c.rx, dt),
                    tx_rate: rate(ptx, c.tx, dt),
                    rx_total: c.rx,
                    tx_total: c.tx,
                    name: c.name,
                }
            })
            .collect();
        self.net = seen;

        let mut seen = HashMap::new();
        let disk_io = disk::parse_diskstats(&r.read("/proc/diskstats").unwrap_or_default())
            .into_iter()
            .filter(|c| disk::is_disk(r, &c.name))
            .map(|c| {
                let (pr, pw) = self
                    .disk
                    .get(&c.name)
                    .copied()
                    .unwrap_or((c.read, c.written));
                seen.insert(c.name.clone(), (c.read, c.written));
                disk::Io {
                    read_rate: rate(pr, c.read, dt),
                    write_rate: rate(pw, c.written, dt),
                    name: c.name,
                }
            })
            .collect();
        self.disk = seen;

        let mounts = want.disks.then(|| match &self.mounts {
            Some((when, m)) if at - when < DISKS_EVERY && at >= *when => m.clone(),
            _ => {
                let m = disk::mounts(r);
                self.mounts = Some((at, m.clone()));
                m
            }
        });

        let status = apps::saver(r);
        let mut frozen_pids = std::collections::HashSet::new();
        let mut seen = HashMap::new();
        let apps = apps::scopes(r, self.uid)
            .into_iter()
            .map(|s| {
                let prev = self.app_cpu.get(&s.unit).copied().unwrap_or(s.cpu_usec);
                seen.insert(s.unit.clone(), s.cpu_usec);
                let pss = want.pss.then(|| match self.pss.get(&s.unit) {
                    Some((when, v)) if at - when < PSS_EVERY && at >= *when => *v,
                    _ => {
                        let v = apps::pss(r, &s.pids);
                        self.pss.insert(s.unit.clone(), (at, v));
                        v
                    }
                });
                if s.frozen {
                    frozen_pids.extend(s.pids.iter().copied());
                }
                let targets = s
                    .pids
                    .iter()
                    .filter_map(|&pid| {
                        Some(procs::Target {
                            pid,
                            start: procs::start_of(r, pid)?,
                        })
                    })
                    .collect();
                apps::AppUse {
                    targets,
                    // usec of CPU per second -> percent of one core.
                    cpu: (rate(prev, s.cpu_usec, dt) / 1e4 * 10.0).round() / 10.0,
                    rest: apps::rest(apps::saver_state(status.as_ref(), &s.app), s.frozen),
                    pss: pss.flatten(),
                    unit: s.unit,
                    app: s.app,
                    pids: s.pids,
                    memory: s.memory,
                    swap: s.swap,
                    frozen: s.frozen,
                }
            })
            .collect();
        self.pss.retain(|u, _| seen.contains_key(u));
        self.app_cpu = seen;

        let procs = want.procs.then(|| {
            self.procs.source().now_us = (at * 1e6) as i64;
            self.procs.sample();
            procs::rows(&mut self.procs, &frozen_pids)
        });

        let r = &self.root;
        Sample {
            at,
            cpu,
            memory: MemoryStats::parse_meminfo(&r.read("/proc/meminfo").unwrap_or_default()),
            load: r.load_average(),
            net,
            disk_io,
            mounts,
            gpus: gpu::read(r),
            temps: temps::read(r),
            apps,
            procs,
            saver_on: status.is_some_and(|s| s.enabled),
        }
    }
}
