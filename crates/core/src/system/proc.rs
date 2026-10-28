//! Process table as struct-of-arrays from Linux `/proc/[pid]/stat` (no ps fork), refreshed in place.
//! Steady-state sampling allocates nothing in the sampler: arrays grow only with the process count, names
//! are cached per (pid, start time), users per uid. CPU% = runtime delta / wall delta per process (100 = one core).
//! Sequential on purpose: IaraOS measured a fork/join of table || totals slower (totals cost ~7 us).
use std::collections::HashMap;

/// One process record, decoded from `/proc/[pid]/stat`.
pub mod table {
    /// Linux TASK_COMM_LEN.
    pub const COMM_LEN: usize = 16;

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct Row {
        pub pid: i32,
        pub ppid: i32,
        pub uid: i32,
        /// Linux state letter (R S D T t Z I X).
        pub stat: u8,
        pub rss_pages: i64,
        pub vsize: i64,
        pub runtime_us: i64,
        /// Since boot; identifies a pid across reuse.
        pub start_us: i64,
        pub nice: i8,
        pub threads: i32,
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct Rec {
        pub row: Row,
        comm: [u8; COMM_LEN],
        comm_len: u8,
    }

    impl Rec {
        pub fn new(row: Row, comm: &[u8]) -> Self {
            let mut r = Self {
                row,
                ..Default::default()
            };
            let n = comm.len().min(COMM_LEN);
            r.comm[..n].copy_from_slice(&comm[..n]);
            r.comm_len = n as u8;
            r
        }
        pub fn comm(&self) -> &[u8] {
            &self.comm[..self.comm_len as usize]
        }
    }

    fn num(f: &[u8]) -> Option<i64> {
        std::str::from_utf8(f).ok()?.parse().ok()
    }

    /// `/proc/[pid]/stat`; `tck` = clock ticks per second. Allocation-free. None when malformed.
    pub fn parse_stat(text: &[u8], uid: i32, tck: i64) -> Option<Rec> {
        let open = text.iter().position(|c| *c == b'(')?;
        let close = text.iter().rposition(|c| *c == b')')?;
        let pid = num(text[..open].trim_ascii())?;
        let comm = text.get(open + 1..close)?;
        // Fields after ") ": state(3) ppid(4) ... utime(14) stime(15) .. nice(19) threads(20) .. start(22) vsize(23) rss(24).
        let mut f = [0i64; 22];
        let mut state = b'?';
        for (i, w) in text
            .get(close + 1..)?
            .split(|c| c.is_ascii_whitespace())
            .filter(|w| !w.is_empty())
            .take(22)
            .enumerate()
        {
            if i == 0 {
                state = w[0];
            } else {
                f[i] = num(w)?;
            }
        }
        let tck = tck.max(1);
        let us = |t: i64| t * 1_000_000 / tck;
        Some(Rec::new(
            Row {
                pid: pid as i32,
                ppid: f[1] as i32,
                uid,
                stat: state,
                rss_pages: f[21],
                vsize: f[20],
                runtime_us: us(f[11] + f[12]),
                start_us: us(f[19]),
                nice: f[16] as i8,
                threads: f[17] as i32,
            },
            comm,
        ))
    }
}

/// System-wide numbers sampled with the table. Bytes, cumulative ticks.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SystemTotals {
    pub cpu_busy: i64,
    pub cpu_total: i64,
    pub mem_total: i64,
    pub mem_used: i64,
}

/// Where samples come from. Must not allocate in steady state.
pub trait ProcSource {
    /// Process records into `buf` (grow only when too small); returns count.
    fn procs(&mut self, buf: &mut Vec<table::Rec>) -> Option<usize>;
    /// Monotonic microseconds.
    fn now_us(&self) -> i64;
    fn page_size(&self) -> i64;
    fn totals(&mut self, t: &mut SystemTotals);
    /// Called once per uid; cached by the sampler.
    fn user_name(&mut self, uid: i32) -> Option<String>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum ProcessState {
    Unknown,
    Running,
    Sleeping,
    Idle,
    Waiting,
    Stopped,
    Zombie,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    Pid,
    Name,
    User,
    Cpu,
    Memory,
    Threads,
    Runtime,
}

struct Prev {
    start: i64,
    runtime: i64,
    name: String,
}

/// Struct-of-arrays process table. Not thread-safe: one sampler per consumer.
pub struct ProcessSampler<S: ProcSource> {
    src: S,
    table: Vec<table::Rec>,
    pub pid: Vec<i32>,
    pub ppid: Vec<i32>,
    pub uid: Vec<i32>,
    pub threads: Vec<i32>,
    pub rss_bytes: Vec<i64>,
    pub vsize: Vec<i64>,
    pub runtime_us: Vec<i64>,
    pub start_us: Vec<i64>,
    pub stat: Vec<u8>,
    pub nice: Vec<i8>,
    pub cpu_percent: Vec<f32>,
    pub name: Vec<String>,
    prev: HashMap<i32, Prev>,
    cur: HashMap<i32, Prev>,
    users: HashMap<i32, String>,
    prev_now: i64,
    prev_totals: SystemTotals,
    pub totals: SystemTotals,
    pub cpu_total_percent: f32,
    count: usize,
}

impl<S: ProcSource> ProcessSampler<S> {
    pub fn new(src: S) -> Self {
        Self {
            src,
            table: Vec::new(),
            pid: vec![],
            ppid: vec![],
            uid: vec![],
            threads: vec![],
            rss_bytes: vec![],
            vsize: vec![],
            runtime_us: vec![],
            start_us: vec![],
            stat: vec![],
            nice: vec![],
            cpu_percent: vec![],
            name: vec![],
            prev: HashMap::new(),
            cur: HashMap::new(),
            users: HashMap::new(),
            prev_now: 0,
            prev_totals: SystemTotals::default(),
            totals: SystemTotals::default(),
            cpu_total_percent: 0.0,
            count: 0,
        }
    }

    pub fn source(&mut self) -> &mut S {
        &mut self.src
    }

    pub fn count(&self) -> usize {
        self.count
    }

    /// Linux state letter: R running, S sleeping, D disk wait, T/t stopped, Z zombie, I idle.
    pub fn state(&self, i: usize) -> ProcessState {
        match self.stat[i] {
            b'R' => ProcessState::Running,
            b'S' => ProcessState::Sleeping,
            b'D' => ProcessState::Waiting,
            b'T' | b't' => ProcessState::Stopped,
            b'Z' | b'X' => ProcessState::Zombie,
            b'I' => ProcessState::Idle,
            _ => ProcessState::Unknown,
        }
    }

    /// User name of row `i`; numeric uid when unknown. Cached per uid.
    pub fn user(&mut self, i: usize) -> &str {
        let u = self.uid[i];
        if !self.users.contains_key(&u) {
            let n = self.src.user_name(u).unwrap_or_else(|| u.to_string());
            self.users.insert(u, n);
        }
        &self.users[&u]
    }

    fn grow(&mut self, n: usize) {
        if self.pid.len() >= n {
            return;
        }
        let cap = 64.max(n + n / 4);
        self.pid.resize(cap, 0);
        self.ppid.resize(cap, 0);
        self.uid.resize(cap, 0);
        self.threads.resize(cap, 0);
        self.rss_bytes.resize(cap, 0);
        self.vsize.resize(cap, 0);
        self.runtime_us.resize(cap, 0);
        self.start_us.resize(cap, 0);
        self.stat.resize(cap, 0);
        self.nice.resize(cap, 0);
        self.cpu_percent.resize(cap, 0.0);
        self.name.resize(cap, String::new());
        self.prev.reserve(cap);
        self.cur.reserve(cap);
    }

    /// One refresh. False (count 0) when the table cannot be read or has an unknown layout.
    pub fn sample(&mut self) -> bool {
        let len = self.src.procs(&mut self.table);
        let now = self.src.now_us();
        self.src.totals(&mut self.totals);
        let dt = self.totals.cpu_total - self.prev_totals.cpu_total;
        self.cpu_total_percent = if dt > 0 {
            100.0 * (self.totals.cpu_busy - self.prev_totals.cpu_busy) as f32 / dt as f32
        } else {
            0.0
        };
        self.prev_totals = self.totals;
        let Some(n) = len.filter(|l| *l <= self.table.len()) else {
            self.count = 0;
            return false;
        };
        self.grow(n);
        let wall = now - self.prev_now;
        let page = self.src.page_size();
        self.cur.clear();
        for i in 0..n {
            let rec = self.table[i];
            let r = rec.row;
            self.pid[i] = r.pid;
            self.ppid[i] = r.ppid;
            self.uid[i] = r.uid;
            self.threads[i] = r.threads;
            self.rss_bytes[i] = r.rss_pages * page;
            self.vsize[i] = r.vsize;
            self.runtime_us[i] = r.runtime_us;
            self.start_us[i] = r.start_us;
            self.stat[i] = r.stat;
            self.nice[i] = r.nice;
            let comm = rec.comm();
            let prev = self.prev.remove(&r.pid).filter(|p| p.start == r.start_us);
            self.cpu_percent[i] = match &prev {
                Some(p) if wall > 0 => {
                    (100.0 * (r.runtime_us - p.runtime) as f32 / wall as f32).max(0.0)
                }
                _ => 0.0,
            };
            // Reuse the row's String buffer: no allocation when the name fits.
            let mut nm = prev
                .map(|p| p.name)
                .unwrap_or_else(|| std::mem::take(&mut self.name[i]));
            if nm.as_bytes() != comm {
                nm.clear();
                nm.push_str(&String::from_utf8_lossy(comm));
            }
            self.name[i].clone_from(&nm);
            self.cur.insert(
                r.pid,
                Prev {
                    start: r.start_us,
                    runtime: r.runtime_us,
                    name: nm,
                },
            );
        }
        std::mem::swap(&mut self.prev, &mut self.cur);
        self.prev_now = now;
        self.count = n;
        true
    }

    /// Fill `index` with rows ordered by `col`, ties by pid. Unstable sort: no allocation; pid tie-break keeps it deterministic.
    pub fn sort(&mut self, index: &mut [usize], col: Column, descending: bool) {
        let len = index.len().min(self.count);
        let ix = &mut index[..len];
        for (i, v) in ix.iter_mut().enumerate() {
            *v = i;
        }
        if col == Column::User {
            for i in 0..len {
                let _ = self.user(i);
            }
        }
        let s = &*self;
        ix.sort_unstable_by(|&a, &b| {
            let c = match col {
                Column::Pid => std::cmp::Ordering::Equal,
                Column::Name => s.name[a]
                    .bytes()
                    .map(|c| c.to_ascii_lowercase())
                    .cmp(s.name[b].bytes().map(|c| c.to_ascii_lowercase())),
                Column::User => s.users[&s.uid[a]].cmp(&s.users[&s.uid[b]]),
                Column::Cpu => s.cpu_percent[a].total_cmp(&s.cpu_percent[b]),
                Column::Memory => s.rss_bytes[a].cmp(&s.rss_bytes[b]),
                Column::Threads => s.threads[a].cmp(&s.threads[b]),
                Column::Runtime => s.runtime_us[a].cmp(&s.runtime_us[b]),
            };
            let c = if descending { c.reverse() } else { c };
            c.then(s.pid[a].cmp(&s.pid[b]))
        });
    }
}

/// Product source: `/proc/[pid]/stat`, `/proc/stat`, `/proc/meminfo`.
pub struct LinuxProcSource {
    tck: i64,
    page: i64,
    passwd: Option<HashMap<i32, String>>,
    start: std::time::Instant,
    path: String,
    text: Vec<u8>,
}

impl Default for LinuxProcSource {
    fn default() -> Self {
        // SAFETY: sysconf has no preconditions.
        let (tck, page) = unsafe {
            (
                libc::sysconf(libc::_SC_CLK_TCK),
                libc::sysconf(libc::_SC_PAGESIZE),
            )
        };
        Self {
            tck: if tck > 0 { tck } else { 100 },
            page: if page > 0 { page } else { 4096 },
            passwd: None,
            start: std::time::Instant::now(),
            path: String::with_capacity(32),
            text: Vec::with_capacity(1024),
        }
    }
}

impl LinuxProcSource {
    fn read(&mut self, path: &str) -> bool {
        use std::io::Read;
        self.text.clear();
        std::fs::File::open(path)
            .and_then(|mut f| f.read_to_end(&mut self.text))
            .is_ok()
    }
}

impl ProcSource for LinuxProcSource {
    fn procs(&mut self, buf: &mut Vec<table::Rec>) -> Option<usize> {
        use std::fmt::Write;
        use std::os::unix::fs::MetadataExt;
        let mut n = 0;
        for e in std::fs::read_dir("/proc").ok()?.flatten() {
            let name = e.file_name();
            let Some(pid) = name
                .to_str()
                .filter(|s| s.bytes().all(|c| c.is_ascii_digit()))
            else {
                continue;
            };
            let uid = e.metadata().map_or(-1, |m| m.uid() as i32);
            self.path.clear();
            let _ = write!(self.path, "/proc/{pid}/stat");
            let path = std::mem::take(&mut self.path);
            let ok = self.read(&path);
            self.path = path;
            let Some(rec) = ok
                .then(|| table::parse_stat(&self.text, uid, self.tck))
                .flatten()
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
        self.start.elapsed().as_micros() as i64
    }
    fn page_size(&self) -> i64 {
        self.page
    }
    fn totals(&mut self, t: &mut SystemTotals) {
        if self.read("/proc/stat")
            && let Some(c) = std::str::from_utf8(&self.text)
                .ok()
                .and_then(super::stats::CpuTimes::parse_proc_stat)
        {
            t.cpu_busy = c.busy;
            t.cpu_total = c.total;
        }
        if self.read("/proc/meminfo") {
            let m = super::stats::MemoryStats::parse_meminfo(&String::from_utf8_lossy(&self.text));
            t.mem_total = m.total;
            t.mem_used = m.used;
        }
    }
    fn user_name(&mut self, uid: i32) -> Option<String> {
        let p = self.passwd.get_or_insert_with(|| {
            std::fs::read_to_string("/etc/passwd")
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

/// Scripted process table for tests.
#[derive(Default)]
pub struct FakeProcSource {
    pub table: Vec<table::Rec>,
    pub now: i64,
    pub sys: SystemTotals,
    pub user_lookups: usize,
    /// /proc unreadable.
    pub broken: bool,
}

/// (pid, runtime_us, start_sec, comm, uid, rss_pages, stat letter)
pub type FakeProc<'a> = (i32, i64, i64, &'a str, i32, i64, u8);

impl FakeProcSource {
    pub fn set(&mut self, rows: &[FakeProc]) {
        self.table.clear();
        for &(pid, runtime_us, start, comm, uid, rss_pages, stat) in rows {
            let r = table::Row {
                pid,
                runtime_us,
                start_us: start * 1_000_000,
                uid,
                rss_pages,
                stat,
                ..Default::default()
            };
            self.table.push(table::Rec::new(r, comm.as_bytes()));
        }
    }
}

impl ProcSource for FakeProcSource {
    fn procs(&mut self, buf: &mut Vec<table::Rec>) -> Option<usize> {
        if self.broken {
            return None;
        }
        if buf.len() < self.table.len() {
            buf.resize(self.table.len(), table::Rec::default());
        }
        buf[..self.table.len()].copy_from_slice(&self.table);
        Some(self.table.len())
    }
    fn now_us(&self) -> i64 {
        self.now
    }
    fn page_size(&self) -> i64 {
        4096
    }
    fn totals(&mut self, t: &mut SystemTotals) {
        *t = self.sys;
    }
    fn user_name(&mut self, uid: i32) -> Option<String> {
        self.user_lookups += 1;
        match uid {
            0 => Some("root".into()),
            1001 => Some("klee".into()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: i32, rt: i64) -> FakeProc<'static> {
        (pid, rt, 100, "sh", 0, 1, b'S')
    }
    fn mk() -> ProcessSampler<FakeProcSource> {
        ProcessSampler::new(FakeProcSource::default())
    }
    fn idx(s: &ProcessSampler<FakeProcSource>, pid: i32) -> usize {
        s.pid[..s.count()].iter().position(|x| *x == pid).unwrap()
    }

    #[test]
    fn parses_proc_pid_stat_fixture() {
        // Real /proc/[pid]/stat line (Arch, kernel 6.x); comm may hold spaces and parens.
        let line = b"1234 (Web Content (x)) S 1 1234 1234 0 -1 4194560 5000 0 10 0 250 50 0 0 20 0 27 0 9000 3221225472 51200 18446744073709551615 1 1 0 0 0 0 0 4096 0 0 0 0 17 3 0 0 0 0 0\n";
        let r = table::parse_stat(line, 1001, 100).unwrap();
        assert_eq!(r.comm(), b"Web Content (x)");
        assert_eq!(
            r.row,
            table::Row {
                pid: 1234,
                ppid: 1,
                uid: 1001,
                stat: b'S',
                rss_pages: 51200,
                vsize: 3_221_225_472,
                runtime_us: 3_000_000,
                start_us: 90_000_000,
                nice: 0,
                threads: 27,
            }
        );
        assert!(table::parse_stat(b"garbage", 0, 100).is_none());
        assert!(table::parse_stat(b"12 (x) S 1 x", 0, 100).is_none());
    }

    #[test]
    fn real_proc_table_has_this_process() {
        let mut s = ProcessSampler::new(LinuxProcSource::default());
        assert!(s.sample());
        let me = std::process::id() as i32;
        let i = s.pid[..s.count()].iter().position(|x| *x == me).unwrap();
        assert!(s.rss_bytes[i] > 0 && s.threads[i] >= 1);
        assert!(s.totals.mem_total > 0 && s.totals.cpu_total > 0);
    }

    #[test]
    fn first_sample_decodes_rows_with_zero_cpu() {
        let mut s = mk();
        s.source().set(&[
            (1, 5_000_000, 100, "init", 0, 3, b'R'),
            (42, 0, 100, "fish", 1002, 1, b'S'),
        ]);
        assert!(s.sample());
        assert_eq!(s.count(), 2);
        let i = idx(&s, 1);
        assert_eq!(
            (
                s.name[i].as_str(),
                s.rss_bytes[i],
                s.state(i),
                s.cpu_percent[i]
            ),
            ("init", 3 * 4096, ProcessState::Running, 0.0)
        );
        assert_eq!(s.user(i), "root");
        let j = idx(&s, 42);
        assert_eq!(s.user(j), "1002");
    }

    #[test]
    fn cpu_percent_is_runtime_delta_over_wall() {
        let mut s = mk();
        for (rt, now, want) in [
            (1_000_000, 10_000_000, 0.0),
            (1_500_000, 11_000_000, 50.0),
            (3_500_000, 12_000_000, 200.0),
        ] {
            s.source().set(&[p(7, rt)]);
            s.source().now = now;
            s.sample();
            assert!((s.cpu_percent[0] - want).abs() < 0.01, "{want}");
        }
    }

    #[test]
    fn reused_pid_with_new_start_is_new_process_and_exec_renames() {
        let mut s = mk();
        s.source().set(&[(7, 9_000_000, 100, "old", 0, 1, b'S')]);
        s.source().now = 1_000_000;
        s.sample();
        s.source().set(&[(7, 100_000, 200, "new", 0, 1, b'S')]);
        s.source().now = 2_000_000;
        s.sample();
        assert_eq!((s.cpu_percent[0], s.name[0].as_str()), (0.0, "new"));
        s.source().set(&[(7, 100_000, 200, "vim", 0, 1, b'S')]);
        s.sample();
        assert_eq!(s.name[0], "vim");
    }

    #[test]
    fn unreadable_table_fails_closed() {
        let mut s = mk();
        s.source().broken = true;
        s.source().set(&[p(7, 0)]);
        assert!(!s.sample());
        assert_eq!(s.count(), 0);
    }

    #[test]
    fn totals_give_cpu_usage() {
        let mut s = mk();
        s.source().set(&[p(1, 0)]);
        s.source().sys = SystemTotals {
            cpu_busy: 100,
            cpu_total: 1000,
            mem_total: 8,
            mem_used: 3,
        };
        s.sample();
        s.source().sys = SystemTotals {
            cpu_busy: 175,
            cpu_total: 1100,
            mem_total: 8,
            mem_used: 4,
        };
        s.sample();
        assert_eq!((s.cpu_total_percent, s.totals.mem_used), (75.0, 4));
    }

    #[test]
    fn sort_orders_index_without_moving_rows() {
        let mut s = mk();
        s.source().set(&[
            (1, 0, 100, "b", 0, 5, b'S'),
            (2, 0, 100, "c", 0, 9, b'S'),
            (3, 0, 100, "a", 0, 1, b'S'),
        ]);
        s.sample();
        let mut ix = vec![0; 3];
        s.sort(&mut ix, Column::Memory, true);
        assert_eq!(ix.iter().map(|i| s.pid[*i]).collect::<Vec<_>>(), [2, 1, 3]);
        s.sort(&mut ix, Column::Name, false);
        assert_eq!(ix.iter().map(|i| s.pid[*i]).collect::<Vec<_>>(), [3, 1, 2]);
    }
}
